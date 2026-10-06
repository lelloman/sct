use super::policy_preview_tests::complete_preview;
use super::*;
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;

async fn movement(
    w: &Writer,
    a: &Actor,
    source: &EntryId,
    destination: &EntryId,
    name: &str,
) -> PolicyChange {
    PolicyChange::Move {
        entry_id: source.clone(),
        expected_revision: w.inspect_entry(a, source).await.unwrap().revision,
        parent_id: destination.clone(),
        expected_parent_revision: w.inspect_entry(a, destination).await.unwrap().revision,
        name: name.parse().unwrap(),
    }
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn cross_policy_moves_require_preview_and_preserve_subtree_identity_on_recovery() {
    let (w, a, db) = setup().await;
    let mut req = request(a.principal_id(), "/moves", 4000);
    req.default_policy.keep_versions = 3.try_into().unwrap();
    let r = w.create_root(&a, &req).await.unwrap();
    let r = w
        .update_root(
            &a,
            &r.id,
            &UpdateRootRequest {
                expected_revision: r.revision.clone(),
                display_name: r.display_name.clone(),
                state: RootState::Enabled,
                placement: r.placement.clone(),
                quota_bytes: r.quota_bytes,
            },
        )
        .await
        .unwrap();
    db::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    db::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let parent = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let source = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "source",
        VersioningMode::IndependentFiles,
    )
    .await;
    let inside = mkdir(
        &w,
        &a,
        &r.id,
        &source.id,
        "inside",
        VersioningMode::IndependentFiles,
    )
    .await;
    let destination = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "destination",
        VersioningMode::IndependentFiles,
    )
    .await;
    let equal = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "equal",
        VersioningMode::IndependentFiles,
    )
    .await;
    let atomic = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "atomic",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let mixed = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "mixed",
        VersioningMode::IndependentFiles,
    )
    .await;
    mkdir(
        &w,
        &a,
        &r.id,
        &mixed.id,
        "atomic-child",
        VersioningMode::AtomicDirectory,
    )
    .await;
    for entry in [&destination, &equal] {
        let scope = PolicyScope::Entry {
            root_id: r.id.clone(),
            entry_id: entry.id.clone(),
        };
        let policy = w.effective_policy(&a, &scope).await.unwrap();
        w.set_policy(
            &a,
            &SetPolicyRequest {
                scope,
                expected_revision: policy.revision,
                expected_scope_revision: w.inspect_entry(&a, &entry.id).await.unwrap().revision,
                policy: RetentionPolicy {
                    mode: RetentionMode::Relaxed,
                    keep_versions: 1.try_into().unwrap(),
                },
                versioning: VersioningMode::IndependentFiles,
            },
        )
        .await
        .unwrap();
    }
    let stores = BTreeMap::from([(
        "one".into(),
        Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
    )]);
    let e = TransferEngine::new(w.clone(), stores.clone(), 2, 60).unwrap();
    for n in 0..3 {
        let upload = ready_bytes(&e, &w, &a, &r.id, "file", &[n; 3])
            .await
            .unwrap();
        e.commit_upload(&a, &r.id, &upload).await.unwrap();
    }
    let file = w
        .resolve_path(&a, &r.id, &"/file".parse().unwrap())
        .await
        .unwrap();
    w.rename_entry(
        &a,
        &file.id,
        &rename_request(&w, &a, &file, &source.id, "file").await,
    )
    .await
    .unwrap();
    assert_eq!(
        w.rename_entry(
            &a,
            &source.id,
            &rename_request(&w, &a, &source, &destination.id, "moved").await
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::InvalidRequest
    );
    for change in [
        movement(&w, &a, &source.id, &atomic.id, "no").await,
        movement(&w, &a, &atomic.id, &destination.id, "no").await,
        movement(&w, &a, &source.id, &inside.id, "cycle").await,
    ] {
        assert_eq!(
            w.preview_change(
                &a,
                &r.id,
                &PreviewChangeRequest {
                    expected_root_revision: w.inspect_root(&a, &r.id).await.unwrap().revision,
                    change
                }
            )
            .await
            .unwrap_err()
            .0,
            ErrorCode::InvalidRequest
        );
    }
    let mixed_preview = w
        .preview_change(
            &a,
            &r.id,
            &PreviewChangeRequest {
                expected_root_revision: w.inspect_root(&a, &r.id).await.unwrap().revision,
                change: movement(&w, &a, &mixed.id, &destination.id, "mixed").await,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        w.advance_preview(&a, &r.id, &mixed_preview.id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::InvalidRequest
    );
    let stale = complete_preview(
        &w,
        &a,
        &r.id,
        movement(&w, &a, &source.id, &destination.id, "moved").await,
    )
    .await;
    mkdir(
        &w,
        &a,
        &r.id,
        &destination.id,
        "new-child",
        VersioningMode::IndependentFiles,
    )
    .await;
    assert_eq!(
        e.apply_change(
            &a,
            &r.id,
            &ApplyChangeRequest {
                idempotency_key: "stale".parse().unwrap(),
                preview_id: stale.id,
                expected_root_revision: stale.root_revision,
                acknowledge_retention_effect: true
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::RevisionConflict
    );
    let preview = complete_preview(
        &w,
        &a,
        &r.id,
        movement(&w, &a, &source.id, &destination.id, "moved").await,
    )
    .await;
    assert_eq!(preview.versions_to_retire.get(), 2);
    let request = ApplyChangeRequest {
        idempotency_key: "move".parse().unwrap(),
        preview_id: preview.id,
        expected_root_revision: preview.root_revision,
        acknowledge_retention_effect: true,
    };
    let job = e.apply_change(&a, &r.id, &request).await.unwrap();
    assert_eq!(
        w.resolve_path(&a, &r.id, &"/destination/moved/file".parse().unwrap())
            .await
            .unwrap()
            .id,
        file.id
    );
    assert_eq!(
        w.resolve_path(&a, &r.id, &"/destination/moved".parse().unwrap())
            .await
            .unwrap()
            .id,
        source.id
    );
    assert!(
        w.resolve_path(&a, &r.id, &"/source/file".parse().unwrap())
            .await
            .is_err()
    );
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let e = TransferEngine::new(w.clone(), stores, 2, 60).unwrap();
    for _ in 0..12 {
        e.tick().await.unwrap();
        if e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state == LifecycleState::Completed {
            break;
        }
    }
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state,
        LifecycleState::Completed
    );
    assert_eq!(
        e.file_history(&a, &r.id, &file.id, None)
            .await
            .unwrap()
            .versions
            .len(),
        1
    );
    assert_eq!(
        e.apply_change(&a, &r.id, &request).await.unwrap().id,
        job.id
    );
    let moved = w.inspect_entry(&a, &source.id).await.unwrap();
    assert_eq!(
        w.rename_entry(
            &a,
            &source.id,
            &rename_request(&w, &a, &moved, &equal.id, "same-values").await
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::InvalidRequest
    );
    let same = complete_preview(
        &w,
        &a,
        &r.id,
        movement(&w, &a, &source.id, &equal.id, "same-values").await,
    )
    .await;
    assert_eq!(same.versions_to_retire.get(), 0);
    assert_eq!(same.versions_losing_protection.get(), 0);
}
