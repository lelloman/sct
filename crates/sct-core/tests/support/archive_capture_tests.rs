use super::*;
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;
async fn collection(w: &Writer, a: &Actor) -> ArchiveCollection {
    w.create_archive_collection(
        a,
        &CreateArchiveCollectionRequest {
            request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
            label: "Capture test".parse().unwrap(),
            media_capacity_bytes: 25_000_000_000.try_into().unwrap(),
        },
    )
    .await
    .unwrap()
}
fn request(
    c: &ArchiveCollection,
    r: &RootId,
    v: &VersionId,
    path: &str,
) -> CreateArchivePlanRequest {
    CreateArchivePlanRequest {
        request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
        expected_revision: c.revision.clone(),
        selections: vec![ArchiveSelection {
            root_id: r.clone(),
            version_id: v.clone(),
            path: path.parse().unwrap(),
        }],
        media_capacity_bytes: c.media_capacity_bytes,
    }
}
fn engine(w: &Writer, store: &Arc<MemoryStore>) -> TransferEngine {
    TransferEngine::new(
        w.clone(),
        BTreeMap::from([("one".into(), store.clone() as Arc<dyn ObjectStore>)]),
        2,
        3600,
    )
    .unwrap()
}
async fn finish(w: &Writer, set: &ArchiveSetId) {
    for i in 0..1000 {
        if i % 16 == 0 {
            w.renew().await.unwrap();
        }
        if !w.advance_archive_capture(set).await.unwrap() {
            return;
        }
    }
    panic!("capture failed to converge");
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires PostgreSQL"]
async fn archive_capture_pins_old_bytes_through_retention_restart_and_hide() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/source", 4000).await;
    let store = Arc::new(MemoryStore::default());
    let e = engine(&w, &store);
    let upload = ready_bytes(&e, &w, &a, &r.id, "file", b"older")
        .await
        .unwrap();
    let old = e.commit_upload(&a, &r.id, &upload).await.unwrap();
    e.tick().await.unwrap();
    let c = collection(&w, &a).await;
    let set = w
        .create_archive_plan(&a, &c.id, &request(&c, &r.id, &old.version_id, "/file"))
        .await
        .unwrap();
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins WHERE expires_at IS NULL")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert!(w.advance_archive_capture(&set.id).await.unwrap());
    w.release().await.unwrap();
    let next = w.catalog().acquire_writer().await.unwrap();
    assert!(w.advance_archive_capture(&set.id).await.is_err());
    let e = engine(&next, &store);
    let upload = ready_bytes(&e, &next, &a, &r.id, "file", b"newer")
        .await
        .unwrap();
    e.commit_upload(&a, &r.id, &upload).await.unwrap();
    e.tick().await.unwrap();
    let entry = next
        .resolve_path(&a, &r.id, &"/file".parse().unwrap())
        .await
        .unwrap();
    next.hide_entry(
        &a,
        &r.id,
        &entry.id,
        &HideEntryRequest {
            idempotency_key: "hide".parse().unwrap(),
            expected_revision: entry.revision,
        },
    )
    .await
    .unwrap();
    finish(&next, &set.id).await;
    assert_eq!(
        next.inspect_archive_set(&a, &c.id, &set.id)
            .await
            .unwrap()
            .state,
        ArchiveJobState::Planning
    );
    let hashes: Vec<String> =
        db::query_scalar("SELECT digest FROM archive_objects WHERE set_id=$1")
            .bind(set.id.as_str())
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(hashes, vec![digest(b"older")]);
    assert_eq!(
        db::query_scalar::<_, String>(
            "SELECT version_id FROM archive_entries WHERE set_id=$1 AND path='/file'"
        )
        .bind(set.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap(),
        old.version_id.as_str()
    );
    assert!(db::query_scalar::<_,i64>("SELECT count(*) FROM object_copies WHERE root_id=$1 AND digest=$2 AND state='available'").bind(r.id.as_str()).bind(digest(b"older")).fetch_one(&db).await.unwrap()>0);
    let current = next.inspect_archive_set(&a, &c.id, &set.id).await.unwrap();
    next.cancel_archive_set(
        &a,
        &c.id,
        &set.id,
        &ArchiveSetActionRequest {
            expected_revision: current.revision,
            request_id: "cancel".parse().unwrap(),
        },
    )
    .await
    .unwrap();
    assert!(next.cleanup_archive_capture(&set.id).await.unwrap());
    assert!(next.cleanup_archive_capture(&set.id).await.unwrap());
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT pinned_bytes FROM archive_accounts")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires PostgreSQL"]
async fn archive_capture_preserves_atomic_tree_shared_objects_and_empty_directories() {
    use super::{
        tree_build_tests::{admitted, stage_files},
        tree_draft_tests::{draft_request, mutation},
        tree_publication_tests::seal,
    };
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/tree", 4000).await;
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
    let d = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "atomic",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let store = Arc::new(MemoryStore::default());
    let e = engine(&w, &store);
    let files = vec![b"same".to_vec(), b"same".to_vec()];
    let draft = stage_files(&e, &w, &a, &r.id, &d.id, &files, 130).await;
    admitted(&e, &a, &r.id, &draft).await;
    e.upload_chunk(
        &a,
        &r.id,
        &draft,
        &digest(b"same").parse().unwrap(),
        b"same".to_vec(),
    )
    .await
    .unwrap();
    seal(&e, &a, &r.id, &draft).await;
    let tree = e.commit_upload(&a, &r.id, &draft).await.unwrap();
    e.tick().await.unwrap();
    let c = collection(&w, &a).await;
    let set = w
        .create_archive_plan(&a, &c.id, &request(&c, &r.id, &tree.version_id, "/atomic"))
        .await
        .unwrap();
    for _ in 0..3 {
        assert!(w.advance_archive_capture(&set.id).await.unwrap());
    }
    let before = w.inspect_archive_set(&a, &c.id, &set.id).await.unwrap();
    assert_eq!(before.state, ArchiveJobState::Capturing);
    assert!(before.processed_entries.get() < 134);
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let e = engine(&w, &store);
    let removed = w
        .snapshot_entry(&a, &r.id, &tree.version_id, &["file-0".parse().unwrap()])
        .await
        .unwrap();
    let draft = e
        .create_tree_draft(
            &a,
            &r.id,
            &draft_request(&w, &a, &r.id, &d.id, DraftMode::Patch).await,
        )
        .await
        .unwrap()
        .id;
    for i in 0..1000 {
        if i % 16 == 0 {
            w.renew().await.unwrap();
        }
        if e.tree_draft_status(&a, &r.id, &draft).await.unwrap().phase
            != TreeDraftPhase::Initializing
        {
            break;
        }
        e.advance_tree_draft(&a, &r.id, &draft).await.unwrap();
    }
    assert_ne!(
        e.tree_draft_status(&a, &r.id, &draft).await.unwrap().phase,
        TreeDraftPhase::Initializing
    );
    e.mutate_tree_draft(
        &a,
        &r.id,
        &draft,
        &mutation(vec![DraftMutation::Remove {
            entry_id: removed.id,
        }]),
    )
    .await
    .unwrap();
    e.freeze_tree_draft(&a, &r.id, &draft).await.unwrap();
    admitted(&e, &a, &r.id, &draft).await;
    seal(&e, &a, &r.id, &draft).await;
    e.commit_upload(&a, &r.id, &draft).await.unwrap();
    e.tick().await.unwrap();
    finish(&w, &set.id).await;
    let result = w.inspect_archive_set(&a, &c.id, &set.id).await.unwrap();
    assert_eq!(result.state, ArchiveJobState::Planning);
    assert_eq!(result.processed_entries.get(), 134);
    assert_eq!(result.processed_bytes.get(), 8);
    assert_eq!(
        db::query_scalar::<_, i64>(
            "SELECT count(*) FROM archive_entries WHERE set_id=$1 AND kind='directory'"
        )
        .bind(set.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap(),
        132
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM archive_objects WHERE set_id=$1")
            .bind(set.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM archive_extents WHERE set_id=$1")
            .bind(set.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        db::query_scalar::<_, String>(
            "SELECT version_id FROM archive_entries WHERE set_id=$1 AND path='/atomic/file-0'"
        )
        .bind(set.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap(),
        tree.version_id.as_str()
    );
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires PostgreSQL"]
async fn archive_capture_limits_expiry_and_failed_work_release_owned_pins() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/limits", 500).await;
    let store = Arc::new(MemoryStore::default());
    let e = engine(&w, &store);
    let upload = ready_bytes(&e, &w, &a, &r.id, "file", b"data")
        .await
        .unwrap();
    let version = e
        .commit_upload(&a, &r.id, &upload)
        .await
        .unwrap()
        .version_id;
    e.tick().await.unwrap();
    let c = collection(&w, &a).await;
    let other = collection(&w, &a).await;
    let principal = a.principal_id().parse().unwrap();
    let mut limits = w.archive_capture_limits(&a, &principal).await.unwrap();
    limits.max_active = 1;
    limits.pin_quota_bytes = 8.try_into().unwrap();
    w.configure_archive_capture_limits(&a, &principal, &limits)
        .await
        .unwrap();
    let set = w
        .create_archive_plan(&a, &c.id, &request(&c, &r.id, &version, "/file"))
        .await
        .unwrap();
    assert_eq!(
        w.create_archive_plan(&a, &other.id, &request(&other, &r.id, &version, "/file"))
            .await
            .unwrap_err()
            .0,
        ErrorCode::ArchiveBusy
    );
    w.cancel_archive_set(
        &a,
        &c.id,
        &set.id,
        &ArchiveSetActionRequest {
            expected_revision: set.revision,
            request_id: "cancel".parse().unwrap(),
        },
    )
    .await
    .unwrap();
    w.cleanup_archive_capture(&set.id).await.unwrap();
    let mut limits = w.archive_capture_limits(&a, &principal).await.unwrap();
    limits.pin_quota_bytes = 3.try_into().unwrap();
    w.configure_archive_capture_limits(&a, &principal, &limits)
        .await
        .unwrap();
    assert_eq!(
        w.create_archive_plan(&a, &other.id, &request(&other, &r.id, &version, "/file"))
            .await
            .unwrap_err()
            .0,
        ErrorCode::ArchiveResourceLimit
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    let mut limits = w.archive_capture_limits(&a, &principal).await.unwrap();
    limits.pin_quota_bytes = 100.try_into().unwrap();
    limits.max_records = 2.try_into().unwrap();
    w.configure_archive_capture_limits(&a, &principal, &limits)
        .await
        .unwrap();
    let limited = w
        .create_archive_plan(&a, &other.id, &request(&other, &r.id, &version, "/file"))
        .await
        .unwrap();
    w.archive_capture_tick().await.unwrap();
    assert_eq!(
        w.inspect_archive_set(&a, &other.id, &limited.id)
            .await
            .unwrap()
            .state,
        ArchiveJobState::Failed
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT pinned_bytes FROM archive_accounts")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    let mut limits = w.archive_capture_limits(&a, &principal).await.unwrap();
    limits.max_records = 1000.try_into().unwrap();
    w.configure_archive_capture_limits(&a, &principal, &limits)
        .await
        .unwrap();
    let c = collection(&w, &a).await;
    let expiring = w
        .create_archive_plan(&a, &c.id, &request(&c, &r.id, &version, "/file"))
        .await
        .unwrap();
    db::query(
        "UPDATE archive_sets SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(expiring.id.as_str())
    .execute(&db)
    .await
    .unwrap();
    w.archive_capture_tick().await.unwrap();
    assert_eq!(
        w.inspect_archive_set(&a, &c.id, &expiring.id)
            .await
            .unwrap()
            .state,
        ArchiveJobState::Expired
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    let c = collection(&w, &a).await;
    let mut invalid = request(&c, &r.id, &version, "/file");
    invalid.selections[0].version_id = "not-committed".parse().unwrap();
    assert_eq!(
        w.create_archive_plan(&a, &c.id, &invalid)
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
}
