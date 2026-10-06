use super::*;
use super::{
    tree_build_tests::{admitted, stage_files},
    tree_publication_tests::{patch, seal},
};
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;

pub(super) async fn complete_preview(
    w: &Writer,
    a: &Actor,
    r: &RootId,
    change: PolicyChange,
) -> ChangePreview {
    let mut preview = w
        .preview_change(
            a,
            r,
            &PreviewChangeRequest {
                expected_root_revision: w.inspect_root(a, r).await.unwrap().revision,
                change,
            },
        )
        .await
        .unwrap();
    for _ in 0..100 {
        if preview.state == LifecycleState::Ready {
            return preview;
        }
        preview = w.advance_preview(a, r, &preview.id).await.unwrap();
    }
    panic!("preview did not finish");
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn policy_previews_page_hidden_history_respect_overrides_and_expire() {
    let (w, a, db) = setup().await;
    let mut req = request(a.principal_id(), "/preview", 4000);
    req.default_policy.keep_versions = 100.try_into().unwrap();
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
    let atomic = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "override",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let scope = PolicyScope::Entry {
        root_id: r.id.clone(),
        entry_id: atomic.id.clone(),
    };
    let inherited = w.effective_policy(&a, &scope).await.unwrap();
    w.set_policy(
        &a,
        &SetPolicyRequest {
            scope: scope.clone(),
            expected_revision: inherited.revision,
            expected_scope_revision: atomic.revision.clone(),
            policy: RetentionPolicy {
                mode: RetentionMode::Strict,
                keep_versions: 4.try_into().unwrap(),
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
    for i in 0..4 {
        let upload = ready_bytes(&e, &w, &a, &r.id, "file", &[i; 3])
            .await
            .unwrap();
        e.commit_upload(&a, &r.id, &upload).await.unwrap();
    }
    let file = w
        .resolve_path(&a, &r.id, &"/file".parse().unwrap())
        .await
        .unwrap();
    // Repeated identical commits are valid retained history and force a second
    // preview batch without creating redundant payload copies for this fixture.
    db::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length) SELECT 'history-'||n,v.root_id,v.entry_id,v.manifest,v.length FROM file_versions v CROSS JOIN generate_series(1,62) n WHERE v.id=$1")
        .bind(file.current_version_id.as_ref().unwrap().as_str()).execute(&db).await.unwrap();
    db::query("UPDATE entries SET current_version_id=(SELECT id FROM file_versions WHERE entry_id=$1 ORDER BY sequence DESC LIMIT 1) WHERE id=$1").bind(file.id.as_str()).execute(&db).await.unwrap();
    let draft = stage_files(&e, &w, &a, &r.id, &atomic.id, &[b"tree".to_vec()], 0).await;
    admitted(&e, &a, &r.id, &draft).await;
    e.upload_chunk(
        &a,
        &r.id,
        &draft,
        &digest(b"tree").parse().unwrap(),
        b"tree".to_vec(),
    )
    .await
    .unwrap();
    seal(&e, &a, &r.id, &draft).await;
    e.commit_upload(&a, &r.id, &draft).await.unwrap();
    for _ in 0..3 {
        let draft = patch(&e, &w, &a, &r.id, &atomic.id, None).await;
        seal(&e, &a, &r.id, &draft).await;
        e.commit_upload(&a, &r.id, &draft).await.unwrap();
    }
    w.hide_entry(
        &a,
        &r.id,
        &file.id,
        &HideEntryRequest {
            idempotency_key: "hide".parse().unwrap(),
            expected_revision: w.inspect_entry(&a, &file.id).await.unwrap().revision,
        },
    )
    .await
    .unwrap();
    let root_scope = PolicyScope::Root {
        root_id: r.id.clone(),
    };
    let policy = w.effective_policy(&a, &root_scope).await.unwrap();
    let change = PolicyChange::Set {
        scope: root_scope,
        expected_policy_revision: policy.revision,
        policy: RetentionPolicy {
            mode: RetentionMode::Strict,
            keep_versions: 2.try_into().unwrap(),
        },
    };
    let initial = w
        .preview_change(
            &a,
            &r.id,
            &PreviewChangeRequest {
                expected_root_revision: w.inspect_root(&a, &r.id).await.unwrap().revision,
                change: change.clone(),
            },
        )
        .await
        .unwrap();
    let partial = w.advance_preview(&a, &r.id, &initial.id).await.unwrap();
    assert_eq!(partial.state, LifecycleState::Planning);
    let complete = w.advance_preview(&a, &r.id, &initial.id).await.unwrap();
    assert_eq!(complete.state, LifecycleState::Ready);
    assert_eq!(complete.versions_to_retire.get(), 64);
    assert_eq!(complete.versions_losing_protection.get(), 0);
    assert_eq!(complete.estimated_payload_bytes.get(), 192);
    let page = w
        .preview_versions(
            &a,
            &r.id,
            &complete.id,
            &ListRequest {
                limit: 1.try_into().unwrap(),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert!(page.versions[0].retire);
    assert_eq!(page.versions[0].entry_id, file.id);
    let next = w
        .preview_versions(
            &a,
            &r.id,
            &complete.id,
            &ListRequest {
                limit: 1.try_into().unwrap(),
                cursor: page.next_cursor,
            },
        )
        .await
        .unwrap();
    assert_ne!(page.versions[0].version_id, next.versions[0].version_id);
    let override_policy = w.effective_policy(&a, &scope).await.unwrap();
    let relaxed = complete_preview(
        &w,
        &a,
        &r.id,
        PolicyChange::Set {
            scope: scope.clone(),
            expected_policy_revision: override_policy.revision.clone(),
            policy: RetentionPolicy {
                mode: RetentionMode::Relaxed,
                keep_versions: 4.try_into().unwrap(),
            },
        },
    )
    .await;
    assert_eq!(relaxed.versions_to_retire.get(), 0);
    assert_eq!(relaxed.versions_losing_protection.get(), 3);
    let inherit = complete_preview(
        &w,
        &a,
        &r.id,
        PolicyChange::Inherit {
            scope,
            expected_policy_revision: override_policy.revision,
        },
    )
    .await;
    assert_eq!(inherit.versions_to_retire.get(), 0);
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM file_versions WHERE NOT retired")
            .fetch_one(&db)
            .await
            .unwrap(),
        70,
        "preview is nondestructive"
    );
    db::query(
        "UPDATE change_previews SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(inherit.id.as_str())
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(
        w.inspect_preview(&a, &r.id, &inherit.id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
    w.create_principal(&a, &"planner".parse().unwrap())
        .await
        .unwrap();
    let token = w
        .issue_token(&a, &"planner".parse().unwrap())
        .await
        .unwrap();
    let planner = w.authenticate(&token.secret, false, None).await.unwrap();
    let grant = w
        .create_grant(
            &a,
            &CreateGrantRequest {
                root_id: r.id.clone(),
                principal_id: "planner".parse().unwrap(),
                capabilities: vec![Capability::ManagePolicy, Capability::ListRead],
            },
        )
        .await
        .unwrap();
    let owned = complete_preview(&w, &planner, &r.id, change.clone()).await;
    w.update_grant(
        &a,
        &grant.id,
        &UpdateGrantRequest {
            expected_revision: grant.revision,
            capabilities: vec![
                Capability::ManagePolicy,
                Capability::ListRead,
                Capability::Write,
            ],
        },
    )
    .await
    .unwrap();
    assert_eq!(
        w.inspect_preview(&planner, &r.id, &owned.id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
    mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "changed",
        VersioningMode::IndependentFiles,
    )
    .await;
    assert_eq!(
        w.inspect_preview(&a, &r.id, &complete.id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
    let constraints = complete_preview(&w, &a, &r.id, change).await;
    w.set_policy_constraints(
        &a,
        &r.id,
        &w.inspect_root(&a, &r.id).await.unwrap().revision,
        1.try_into().unwrap(),
        100.try_into().unwrap(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        w.inspect_preview(&a, &r.id, &constraints.id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
}
