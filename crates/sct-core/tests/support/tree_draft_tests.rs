use super::*;
use sct_core::{payload::Stores, transfer_io::TransferEngine};
use simple_server::database::postgres as db;

pub(super) async fn draft_request(
    w: &Writer,
    a: &Actor,
    r: &RootId,
    d: &EntryId,
    mode: DraftMode,
) -> CreateTreeDraftRequest {
    let entry = w.inspect_entry(a, d).await.unwrap();
    let policy = w
        .effective_policy(
            a,
            &PolicyScope::Entry {
                root_id: r.clone(),
                entry_id: d.clone(),
            },
        )
        .await
        .unwrap();
    CreateTreeDraftRequest {
        idempotency_key: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
        base: DirectoryDraftBase {
            root_id: r.clone(),
            directory_id: d.clone(),
            expected_revision: entry.revision,
            base_version_id: entry.current_version_id,
            mode,
        },
        expected_policy_revision: policy.revision,
    }
}
pub(super) fn mutation(mutations: Vec<DraftMutation>) -> DraftMutationPage {
    DraftMutationPage {
        idempotency_key: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
        mutations,
    }
}
fn directory(identity: DraftEntryIdentity, parent_id: EntryId, name: &str) -> DraftMutation {
    DraftMutation::Upsert {
        identity,
        parent_id,
        name: name.parse().unwrap(),
        entry_kind: EntryKind::Directory,
        file: None,
    }
}
pub(super) async fn import_base(e: &TransferEngine, a: &Actor, r: &RootId, d: &UploadId) {
    for _ in 0..20 {
        if e.tree_draft_status(a, r, d).await.unwrap().phase != TreeDraftPhase::Initializing {
            return;
        }
        e.advance_tree_draft(a, r, d).await.unwrap();
    }
    panic!("base import did not finish");
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn drafts_identity_validation_restart_abort_and_expiry() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/draft", 4000).await;
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
    let e = TransferEngine::new(w.clone(), Stores::new(), 1, 60).unwrap();
    let request = draft_request(&w, &a, &r.id, &d.id, DraftMode::Patch).await;
    let draft = e.create_tree_draft(&a, &r.id, &request).await.unwrap();
    assert_eq!(
        e.create_tree_draft(&a, &r.id, &request).await.unwrap().id,
        draft.id
    );
    assert_eq!(
        e.create_tree_draft(
            &a,
            &r.id,
            &draft_request(&w, &a, &r.id, &d.id, DraftMode::Patch).await
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::RootBusy
    );
    let allocation = AllocateDraftIdentitiesRequest {
        idempotency_key: "allocate".parse().unwrap(),
        count: 2.try_into().unwrap(),
    };
    let ids = e
        .allocate_draft_identities(&a, &r.id, &draft.id, &allocation)
        .await
        .unwrap();
    assert_eq!(
        ids,
        e.allocate_draft_identities(&a, &r.id, &draft.id, &allocation)
            .await
            .unwrap()
    );
    let new = |i: usize| DraftEntryIdentity::New {
        entry_id: ids.entry_ids[i].clone(),
    };
    assert_eq!(
        e.mutate_tree_draft(
            &a,
            &r.id,
            &draft.id,
            &mutation(vec![directory(
                DraftEntryIdentity::Existing {
                    entry_id: ids.entry_ids[0].clone()
                },
                d.id.clone(),
                "bad"
            )])
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::InvalidRequest
    );
    let changes = mutation(vec![
        directory(new(0), d.id.clone(), &"é".repeat(255)),
        directory(new(1), ids.entry_ids[0].clone(), "nested"),
    ]);
    e.mutate_tree_draft(&a, &r.id, &draft.id, &changes)
        .await
        .unwrap();
    e.mutate_tree_draft(&a, &r.id, &draft.id, &changes)
        .await
        .unwrap();
    // A new writer resumes the same draft and durable root owner.
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let e = TransferEngine::new(w.clone(), Stores::new(), 1, 60).unwrap();
    assert_eq!(
        e.tree_draft_status(&a, &r.id, &draft.id).await.unwrap().id,
        draft.id
    );
    e.mutate_tree_draft(
        &a,
        &r.id,
        &draft.id,
        &mutation(vec![directory(new(0), ids.entry_ids[1].clone(), "cycle")]),
    )
    .await
    .unwrap();
    assert_eq!(
        e.freeze_tree_draft(&a, &r.id, &draft.id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::InvalidManifest
    );
    assert_eq!(
        e.abort_tree_draft(&a, &r.id, &draft.id)
            .await
            .unwrap()
            .state,
        SessionState::Aborted
    );
    let fresh = e
        .create_tree_draft(
            &a,
            &r.id,
            &draft_request(&w, &a, &r.id, &d.id, DraftMode::Patch).await,
        )
        .await
        .unwrap();
    assert_eq!(
        e.mutate_tree_draft(
            &a,
            &r.id,
            &fresh.id,
            &mutation(vec![directory(new(0), d.id.clone(), "stolen")])
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::InvalidRequest
    );
    db::query(
        "UPDATE tree_drafts SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(fresh.id.as_str())
    .execute(&db)
    .await
    .unwrap();
    e.tick().await.unwrap();
    assert_eq!(
        e.tree_draft_status(&a, &r.id, &fresh.id)
            .await
            .unwrap()
            .state,
        SessionState::Expired
    );
    assert_eq!(
        db::query_scalar::<_, i64>(
            "SELECT count(*) FROM operations WHERE state IN ('active','recovering','cleaning_up')"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
}

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn patch_preserves_omission_complete_replacement_requires_delete() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/base", 4000).await;
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
    let empty = DirectoryManifest {
        format: 1,
        kind: DirectoryObjectKind::Directory,
        entries: 0.try_into().unwrap(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
        height: 0,
        tree: None,
    };
    let body = canonical(&empty).unwrap();
    let empty_id = digest(&body);
    let child: EntryId = "base-child".parse().unwrap();
    let page = canonical(&DirectoryPage::DirectoryEntries {
        format: 1,
        entries: vec![DirectoryEntry {
            id: child.clone(),
            name: "old".parse().unwrap(),
            kind: EntryKind::Directory,
            manifest: empty_id.parse().unwrap(),
            length: 0.try_into().unwrap(),
            chunks: 0.try_into().unwrap(),
        }],
    })
    .unwrap();
    let page_id = digest(&page);
    let manifest = canonical(&DirectoryManifest {
        entries: 1.try_into().unwrap(),
        tree: Some(page_id.parse().unwrap()),
        ..empty
    })
    .unwrap();
    let manifest_id = digest(&manifest);
    for (id, kind, bytes) in [
        (&empty_id, "directory", &body),
        (&page_id, "directory_page", &page),
        (&manifest_id, "directory", &manifest),
    ] {
        db::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,$3,0,0,$4)").bind(r.id.as_str()).bind(id).bind(kind).bind(bytes).execute(&db).await.unwrap();
    }
    db::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length,kind) VALUES('base-version',$1,$2,$3,0,'directory')").bind(r.id.as_str()).bind(d.id.as_str()).bind(&manifest_id).execute(&db).await.unwrap();
    db::query("UPDATE entries SET current_version_id='base-version' WHERE id=$1")
        .bind(d.id.as_str())
        .execute(&db)
        .await
        .unwrap();
    w.create_principal(&a, &"writer".parse().unwrap())
        .await
        .unwrap();
    let token = w.issue_token(&a, &"writer".parse().unwrap()).await.unwrap();
    let actor = w.authenticate(&token.secret, false, None).await.unwrap();
    w.create_grant(
        &a,
        &CreateGrantRequest {
            root_id: r.id.clone(),
            principal_id: "writer".parse().unwrap(),
            capabilities: vec![Capability::Write, Capability::ListRead],
        },
    )
    .await
    .unwrap();
    let e = TransferEngine::new(w.clone(), Stores::new(), 1, 60).unwrap();
    let request = draft_request(&w, &actor, &r.id, &d.id, DraftMode::Patch).await;
    let mut stale = request.clone();
    stale.base.base_version_id = None;
    assert_eq!(
        e.create_tree_draft(&actor, &r.id, &stale)
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
    let patch = e.create_tree_draft(&actor, &r.id, &request).await.unwrap();
    import_base(&e, &actor, &r.id, &patch.id).await;
    assert!(
        !db::query_scalar::<_, bool>(
            "SELECT removed FROM draft_entries WHERE draft_id=$1 AND id='base-child'"
        )
        .bind(patch.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap()
    );
    assert_eq!(
        e.mutate_tree_draft(
            &actor,
            &r.id,
            &patch.id,
            &mutation(vec![DraftMutation::Remove {
                entry_id: child.clone()
            }])
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    e.mutate_tree_draft(
        &actor,
        &r.id,
        &patch.id,
        &mutation(vec![directory(
            DraftEntryIdentity::Existing {
                entry_id: child.clone(),
            },
            d.id.clone(),
            "renamed",
        )]),
    )
    .await
    .unwrap();
    e.freeze_tree_draft(&actor, &r.id, &patch.id).await.unwrap();
    e.abort_tree_draft(&actor, &r.id, &patch.id).await.unwrap();
    let complete = e
        .create_tree_draft(
            &actor,
            &r.id,
            &draft_request(&w, &actor, &r.id, &d.id, DraftMode::CompleteReplacement).await,
        )
        .await
        .unwrap();
    import_base(&e, &actor, &r.id, &complete.id).await;
    assert!(
        db::query_scalar::<_, bool>(
            "SELECT removed FROM draft_entries WHERE draft_id=$1 AND id='base-child'"
        )
        .bind(complete.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap()
    );
    assert_eq!(
        e.freeze_tree_draft(&actor, &r.id, &complete.id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    e.abort_tree_draft(&actor, &r.id, &complete.id)
        .await
        .unwrap();
}
