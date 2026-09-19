use super::*;
use super::{
    tree_build_tests::{admitted, stage_files},
    tree_draft_tests::{draft_request, import_base, mutation},
};
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;

async fn seal(e: &TransferEngine, a: &Actor, r: &RootId, draft: &UploadId) {
    for _ in 0..30 {
        if e.seal_upload(a, r, draft).await.unwrap().upload.state == SessionState::Ready {
            return;
        }
    }
    panic!("seal did not finish");
}
async fn patch(
    e: &TransferEngine,
    w: &Writer,
    a: &Actor,
    r: &RootId,
    d: &EntryId,
    remove: Option<EntryId>,
) -> UploadId {
    let draft = e
        .create_tree_draft(a, r, &draft_request(w, a, r, d, DraftMode::Patch).await)
        .await
        .unwrap();
    import_base(e, a, r, &draft.id).await;
    if let Some(entry_id) = remove {
        e.mutate_tree_draft(
            a,
            r,
            &draft.id,
            &mutation(vec![DraftMutation::Remove { entry_id }]),
        )
        .await
        .unwrap();
    }
    e.freeze_tree_draft(a, r, &draft.id).await.unwrap();
    assert_eq!(
        admitted(e, a, r, &draft.id).await.admission,
        AdmissionState::Admitted
    );
    draft.id
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn directory_publication_is_atomic_replays_receipts_and_rotates_snapshots() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/tree-publish", 4000).await;
    sqlx::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let parent = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let d = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "atomic",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let scope = PolicyScope::Entry {
        root_id: r.id.clone(),
        entry_id: d.id.clone(),
    };
    let policy = w.effective_policy(&a, &scope).await.unwrap();
    w.set_policy(
        &a,
        &SetPolicyRequest {
            scope,
            expected_revision: policy.revision,
            expected_scope_revision: d.revision.clone(),
            policy: RetentionPolicy {
                mode: RetentionMode::Strict,
                keep_versions: 2.try_into().unwrap(),
            },
            versioning: VersioningMode::AtomicDirectory,
        },
    )
    .await
    .unwrap();
    let e = TransferEngine::new(
        w.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        60,
    )
    .unwrap();
    let files = vec![vec![1; 100], vec![2; 100]];
    let first = stage_files(&e, &w, &a, &r.id, &d.id, &files, 1).await;
    admitted(&e, &a, &r.id, &first).await;
    for bytes in &files {
        e.upload_chunk(
            &a,
            &r.id,
            &first,
            &digest(bytes).parse().unwrap(),
            bytes.clone(),
        )
        .await
        .unwrap();
    }
    assert!(
        w.inspect_entry(&a, &d.id)
            .await
            .unwrap()
            .current_version_id
            .is_none()
    );
    assert_eq!(
        e.commit_upload(&a, &r.id, &first).await.unwrap_err().0,
        ErrorCode::InvalidRequest
    );
    seal(&e, &a, &r.id, &first).await;
    let one = e.commit_upload(&a, &r.id, &first).await.unwrap();
    assert_eq!(e.commit_upload(&a, &r.id, &first).await.unwrap(), one);
    assert_eq!(
        e.tree_draft_status(&a, &r.id, &first).await.unwrap().result,
        Some(one.clone())
    );
    let removed = w
        .snapshot_entry(&a, &r.id, &one.version_id, &["file-0".parse().unwrap()])
        .await
        .unwrap();
    let kept = w
        .snapshot_entry(&a, &r.id, &one.version_id, &["file-1".parse().unwrap()])
        .await
        .unwrap();
    let empty = w
        .snapshot_entry(&a, &r.id, &one.version_id, &["empty-0000".parse().unwrap()])
        .await
        .unwrap();
    let second = patch(&e, &w, &a, &r.id, &d.id, Some(removed.id.clone())).await;
    seal(&e, &a, &r.id, &second).await;
    assert_eq!(
        w.inspect_entry(&a, &d.id).await.unwrap().current_version_id,
        Some(one.version_id.clone())
    );
    let two = e.commit_upload(&a, &r.id, &second).await.unwrap();
    assert_eq!(
        w.inspect_entry(&a, &d.id).await.unwrap().current_version_id,
        Some(two.version_id.clone())
    );
    assert_eq!(
        w.snapshot_entry(&a, &r.id, &one.version_id, &["file-0".parse().unwrap()])
            .await
            .unwrap(),
        removed
    );
    assert_eq!(
        w.snapshot_entry(&a, &r.id, &two.version_id, &["file-0".parse().unwrap()])
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(
        w.snapshot_entry(&a, &r.id, &two.version_id, &["file-1".parse().unwrap()])
            .await
            .unwrap(),
        kept
    );
    assert_eq!(
        w.snapshot_entry(&a, &r.id, &two.version_id, &["empty-0000".parse().unwrap()])
            .await
            .unwrap(),
        empty
    );
    // Atomic descendants never acquire independent version/current-entry rows.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM entries WHERE root_id=$1")
            .bind(r.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap(),
        2
    );
    let pin = w.pin_version(&a, &r.id, &one.version_id).await.unwrap();
    let third = patch(&e, &w, &a, &r.id, &d.id, None).await;
    seal(&e, &a, &r.id, &third).await;
    let three = e.commit_upload(&a, &r.id, &third).await.unwrap();
    assert_eq!(
        three.manifest_id, two.manifest_id,
        "unchanged whole trees reuse their canonical root"
    );
    assert_eq!(
        e.file_history(&a, &r.id, &d.id, None)
            .await
            .unwrap()
            .versions
            .len(),
        2
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT retired FROM file_versions WHERE id=$1")
            .bind(one.version_id.as_str())
            .fetch_one(&db)
            .await
            .unwrap()
    );
    for _ in 0..3 {
        e.tick().await.unwrap();
    }
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&db)
            .await
            .unwrap()
            > 0
    );
    w.renew_read_pin(&a, &r.id, &pin).await.unwrap();
    w.release_read_pin(&a, &pin).await.unwrap();
    for _ in 0..8 {
        e.tick().await.unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT occupied+reserved FROM allocations WHERE root_id IS NULL"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    let failed = patch(&e, &w, &a, &r.id, &d.id, Some(kept.id)).await;
    assert_eq!(
        e.commit_upload(&a, &r.id, &failed).await.unwrap_err().0,
        ErrorCode::InvalidRequest
    );
    e.abort_tree_draft(&a, &r.id, &failed).await.unwrap();
    assert_eq!(
        w.inspect_entry(&a, &d.id).await.unwrap().current_version_id,
        Some(three.version_id.clone())
    );
    assert_eq!(e.commit_upload(&a, &r.id, &third).await.unwrap(), three);
}
