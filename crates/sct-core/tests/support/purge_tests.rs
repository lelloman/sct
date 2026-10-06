use super::*;
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn purge_is_explicit_idempotent_shared_and_pin_safe() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/purge", 4000).await;
    db::query("UPDATE roots SET default_policy=jsonb_set(default_policy,'{keep_versions}','200')")
        .execute(&db)
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
    let memory = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([("one".into(), memory as Arc<dyn ObjectStore>)]);
    let e = TransferEngine::new(w.clone(), stores, 2, 60).unwrap();
    let u = ready_bytes(&e, &w, &a, &r.id, "file", b"shared")
        .await
        .unwrap();
    let old = e.commit_upload(&a, &r.id, &u).await.unwrap();
    let u = ready_bytes(&e, &w, &a, &r.id, "file", b"shared")
        .await
        .unwrap();
    let current = e.commit_upload(&a, &r.id, &u).await.unwrap();
    let historical = |revision, version| PurgeRequest {
        idempotency_key: key(),
        expected_revision: revision,
        target: PurgeTarget::HistoricalVersion {
            version_id: version,
        },
    };
    let revision = w.inspect_root(&a, &r.id).await.unwrap().revision;
    assert_eq!(
        e.purge(
            &a,
            &r.id,
            &historical(revision.clone(), current.version_id.clone())
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::InvalidRequest
    );
    let pin = w.pin_version(&a, &r.id, &old.version_id).await.unwrap();
    let request = historical(revision, old.version_id.clone());
    let job = e.purge(&a, &r.id, &request).await.unwrap();
    assert_eq!(e.purge(&a, &r.id, &request).await.unwrap().id, job.id);
    assert_eq!(
        w.pin_version(&a, &r.id, &old.version_id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    for _ in 0..30 {
        e.tick().await.unwrap();
    }
    assert_ne!(
        e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state,
        LifecycleState::Completed
    );
    w.renew_read_pin(&a, &r.id, &pin).await.unwrap();
    w.release_read_pin(&a, &pin).await.unwrap();
    finish(&e, &a, &r.id, &job.id).await;
    let mut read = e
        .download(&a, &r.id, &current.version_id, None)
        .await
        .unwrap();
    assert_eq!(read.next().await.unwrap().unwrap(), b"shared".as_slice());
    assert!(read.next().await.unwrap().is_none());
    drop(read);
    let file = w
        .resolve_path(&a, &r.id, &"/file".parse().unwrap())
        .await
        .unwrap();
    let hidden_request = |revision| PurgeRequest {
        idempotency_key: key(),
        expected_revision: revision,
        target: PurgeTarget::HiddenEntry {
            entry_id: file.id.clone(),
        },
    };
    assert_eq!(
        e.purge(&a, &r.id, &hidden_request(file.revision.clone()))
            .await
            .unwrap_err()
            .0,
        ErrorCode::InvalidRequest
    );
    let hidden = w
        .hide_entry(
            &a,
            &r.id,
            &file.id,
            &HideEntryRequest {
                idempotency_key: key(),
                expected_revision: file.revision,
            },
        )
        .await
        .unwrap();
    w.create_principal(&a, &"operator".parse().unwrap())
        .await
        .unwrap();
    let token = w
        .issue_token(&a, &"operator".parse().unwrap())
        .await
        .unwrap();
    let operator = w.authenticate(&token.secret, false, None).await.unwrap();
    w.create_grant(
        &a,
        &CreateGrantRequest {
            root_id: r.id.clone(),
            principal_id: "operator".parse().unwrap(),
            capabilities: vec![
                Capability::Write,
                Capability::Delete,
                Capability::ManagePolicy,
                Capability::ListRead,
            ],
        },
    )
    .await
    .unwrap();
    let request = hidden_request(hidden.revision);
    assert_eq!(
        e.purge(&operator, &r.id, &request).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    let pin = w.pin_version(&a, &r.id, &current.version_id).await.unwrap();
    let job = e.purge(&a, &r.id, &request).await.unwrap();
    assert_eq!(
        w.inspect_entry(&a, &file.id).await.unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        w.pin_version(&a, &r.id, &current.version_id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(
        e.file_manifest(&a, &r.id, &current.version_id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    w.release_read_pin(&a, &pin).await.unwrap();
    finish(&e, &a, &r.id, &job.id).await;
    assert_eq!(
        e.purge(&a, &r.id, &request).await.unwrap().state,
        LifecycleState::Completed
    );
    let available: i64 = db::query_scalar(
        "SELECT count(*) FROM object_copies WHERE root_id=$1 AND state<>'deleted'",
    )
    .bind(r.id.as_str())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(available, 0);
}
async fn finish(e: &TransferEngine, a: &Actor, r: &RootId, j: &OperationId) {
    for _ in 0..150 {
        e.tick().await.unwrap();
        if e.lifecycle_job(a, r, j).await.unwrap().state == LifecycleState::Completed {
            return;
        }
    }
    panic!(
        "purge did not finish: {:?}",
        e.lifecycle_job(a, r, j).await.unwrap()
    )
}
fn key() -> IdempotencyKey {
    uuid::Uuid::new_v4().to_string().parse().unwrap()
}

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn purge_subtree_resumes_with_atomic_snapshots_and_rechecks_authority() {
    use super::tree_build_tests::{admitted, stage_files};
    use super::tree_publication_tests::seal;
    use sct_core::payload::Fault;
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/purge-tree", 4000).await;
    db::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    db::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let memory = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]);
    let e = TransferEngine::new(w.clone(), stores.clone(), 2, 60).unwrap();
    let root_entry = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let folder = mkdir(
        &w,
        &a,
        &r.id,
        &root_entry.id,
        "folder",
        VersioningMode::IndependentFiles,
    )
    .await;
    let atomic = mkdir(
        &w,
        &a,
        &r.id,
        &folder.id,
        "atomic",
        VersioningMode::AtomicDirectory,
    )
    .await;
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
    let snap = e.commit_upload(&a, &r.id, &draft).await.unwrap();
    let child = w
        .snapshot_entry(&a, &r.id, &snap.version_id, &["file-0".parse().unwrap()])
        .await
        .unwrap();
    assert_eq!(
        e.purge(
            &a,
            &r.id,
            &PurgeRequest {
                idempotency_key: key(),
                expected_revision: atomic.revision.clone(),
                target: PurgeTarget::HiddenEntry { entry_id: child.id }
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::NotFound
    );
    db::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length,kind) SELECT 'purge-copy-'||n,v.root_id,v.entry_id,v.manifest,v.length,v.kind FROM file_versions v CROSS JOIN generate_series(1,130) n WHERE v.id=$1").bind(snap.version_id.as_str()).execute(&db).await.unwrap();
    let folder = w.inspect_entry(&a, &folder.id).await.unwrap();
    let hidden = w
        .hide_entry(
            &a,
            &r.id,
            &folder.id,
            &HideEntryRequest {
                idempotency_key: key(),
                expected_revision: folder.revision,
            },
        )
        .await
        .unwrap();
    w.create_principal(&a, &"purger".parse().unwrap())
        .await
        .unwrap();
    let token = w.issue_token(&a, &"purger".parse().unwrap()).await.unwrap();
    let operator = w.authenticate(&token.secret, false, None).await.unwrap();
    let grant = w
        .create_grant(
            &a,
            &CreateGrantRequest {
                root_id: r.id.clone(),
                principal_id: "purger".parse().unwrap(),
                capabilities: vec![Capability::Purge],
            },
        )
        .await
        .unwrap();
    let mut reader = e
        .download_snapshot_file(
            &a,
            &r.id,
            &snap.version_id,
            &"/file-0".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let job = e
        .purge(
            &operator,
            &r.id,
            &PurgeRequest {
                idempotency_key: key(),
                expected_revision: hidden.revision,
                target: PurgeTarget::HiddenEntry {
                    entry_id: folder.id.clone(),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        w.snapshot_entry(&a, &r.id, &snap.version_id, &["file-0".parse().unwrap()])
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    e.tick().await.unwrap(); // enqueue atomic child
    e.tick().await.unwrap(); // first 64 snapshots
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &job.id)
            .await
            .unwrap()
            .processed
            .get(),
        64
    );
    w.update_grant(
        &a,
        &grant.id,
        &UpdateGrantRequest {
            expected_revision: grant.revision,
            capabilities: vec![Capability::ListRead],
        },
    )
    .await
    .unwrap();
    e.tick().await.unwrap();
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state,
        LifecycleState::Blocked
    );
    assert_eq!(reader.next().await.unwrap().unwrap(), b"tree".as_slice());
    drop(reader);
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let e = TransferEngine::new(w.clone(), stores, 2, 60).unwrap();
    e.tick().await.unwrap();
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &job.id)
            .await
            .unwrap()
            .processed
            .get(),
        64
    );
    assert!(
        e.resume_lifecycle_job(&operator, &r.id, &job.id)
            .await
            .is_err()
    );
    e.resume_lifecycle_job(&a, &r.id, &job.id).await.unwrap();
    db::query("UPDATE read_pins SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&db)
        .await
        .unwrap();
    memory.fault(Fault::UnknownDelete).await;
    for _ in 0..15 {
        e.tick().await.unwrap();
    }
    assert_ne!(
        e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state,
        LifecycleState::Completed
    );
    let charged: i64 = db::query_scalar("SELECT sum(occupied)::bigint FROM allocations")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(charged > 0);
    memory.fault(Fault::None).await;
    finish(&e, &a, &r.id, &job.id).await;
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &job.id)
            .await
            .unwrap()
            .processed
            .get(),
        131
    );
}
