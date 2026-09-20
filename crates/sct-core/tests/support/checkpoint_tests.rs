use super::*;
use sct_core::{
    payload::{Fault, MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn checkpoint_capture_fences_publication_and_failed_restart_keeps_pins() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/checkpoint", 4000).await;
    let memory = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]);
    let engine = TransferEngine::new(w.clone(), stores.clone(), 2, 3600).unwrap();
    let first = ready_bytes(&engine, &w, &a, &r.id, "file", b"first")
        .await
        .unwrap();
    engine.commit_upload(&a, &r.id, &first).await.unwrap();
    engine.tick().await.unwrap();
    let second = ready_bytes(&engine, &w, &a, &r.id, "file", b"second")
        .await
        .unwrap();
    let request = CreateCheckpointRequest {
        idempotency_key: "checkpoint".parse().unwrap(),
    };
    let checkpoint = w.create_checkpoint(&a, &request).await.unwrap();
    assert_eq!(w.create_checkpoint(&a, &request).await.unwrap(), checkpoint);
    engine.tick().await.unwrap(); // Drain boundary; subsequent publication is fenced.
    w.renew().await.unwrap();
    engine.heartbeat().await.unwrap();
    assert_eq!(
        engine
            .commit_upload(&a, &r.id, &second)
            .await
            .unwrap_err()
            .0,
        ErrorCode::RootBusy
    );
    for _ in 0..10 {
        engine.tick().await.unwrap();
        if w.checkpoint(&a, &checkpoint.id).await.unwrap().state == CheckpointState::Exporting {
            break;
        }
    }
    let captured = w.checkpoint(&a, &checkpoint.id).await.unwrap();
    assert_eq!(captured.state, CheckpointState::Exporting);
    assert!(captured.pinned_bytes.get() > 0);
    let pinned: i64 =
        sqlx::query_scalar("SELECT count(*) FROM checkpoint_objects WHERE checkpoint_id=$1")
            .bind(checkpoint.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(pinned >= 2);
    assert_eq!(
        w.release_checkpoint(
            &a,
            &checkpoint.id,
            &ReleaseCheckpointRequest {
                expected_revision: captured.revision.clone(),
                acknowledge_release: false
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::InvalidRequest
    );
    w.release().await.unwrap();
    let resumed = w.catalog().acquire_writer().await.unwrap();
    let engine = TransferEngine::new(resumed.clone(), stores, 2, 3600).unwrap();
    let failed = resumed.checkpoint(&a, &checkpoint.id).await.unwrap();
    assert_eq!(failed.state, CheckpointState::Failed);
    assert_eq!(failed.pinned_bytes, captured.pinned_bytes);
    engine.tick().await.unwrap(); // Recover the upload's old worker epoch.
    engine.commit_upload(&a, &r.id, &second).await.unwrap();
    engine.tick().await.unwrap();
    let deleted:i64=sqlx::query_scalar("SELECT count(*) FROM checkpoint_objects p JOIN object_copies c ON c.generation=p.generation WHERE p.checkpoint_id=$1 AND c.state='deleted'").bind(checkpoint.id.as_str()).fetch_one(&db).await.unwrap();
    assert_eq!(deleted, 0);
    assert_eq!(
        resumed
            .release_checkpoint(
                &a,
                &checkpoint.id,
                &ReleaseCheckpointRequest {
                    expected_revision: captured.revision,
                    acknowledge_release: true
                }
            )
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
    resumed
        .release_checkpoint(
            &a,
            &checkpoint.id,
            &ReleaseCheckpointRequest {
                expected_revision: failed.revision,
                acknowledge_release: true,
            },
        )
        .await
        .unwrap();
    for _ in 0..5 {
        engine.tick().await.unwrap();
    }
    let deleted:i64=sqlx::query_scalar("SELECT count(*) FROM checkpoint_objects p JOIN object_copies c ON c.generation=p.generation WHERE p.checkpoint_id=$1 AND c.state='deleted'").bind(checkpoint.id.as_str()).fetch_one(&db).await.unwrap();
    assert!(deleted > 0);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn checkpoint_drains_queued_uncertain_deletes_before_capturing() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/drain", 4000).await;
    let memory = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        w.clone(),
        BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]),
        2,
        3600,
    )
    .unwrap();
    let first = ready_bytes(&engine, &w, &a, &r.id, "file", b"first")
        .await
        .unwrap();
    engine.commit_upload(&a, &r.id, &first).await.unwrap();
    engine.tick().await.unwrap();
    let second = ready_bytes(&engine, &w, &a, &r.id, "file", b"second")
        .await
        .unwrap();
    memory.fault(Fault::UnknownDelete).await;
    engine.commit_upload(&a, &r.id, &second).await.unwrap();
    engine.tick().await.unwrap();
    let deleting: i64 =
        sqlx::query_scalar("SELECT count(*) FROM object_copies WHERE state='deleting'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(deleting > 0);
    let checkpoint = w
        .create_checkpoint(
            &a,
            &CreateCheckpointRequest {
                idempotency_key: "drain".parse().unwrap(),
            },
        )
        .await
        .unwrap();
    engine.tick().await.unwrap();
    assert_eq!(
        w.checkpoint(&a, &checkpoint.id).await.unwrap().state,
        CheckpointState::Fencing
    );
    assert_eq!(
        w.checkpoint(&a, &checkpoint.id)
            .await
            .unwrap()
            .pinned_bytes
            .get(),
        0
    );
    memory.fault(Fault::None).await;
    for _ in 0..10 {
        engine.tick().await.unwrap();
        if w.checkpoint(&a, &checkpoint.id).await.unwrap().state == CheckpointState::Exporting {
            break;
        }
    }
    assert_eq!(
        w.checkpoint(&a, &checkpoint.id).await.unwrap().state,
        CheckpointState::Exporting
    );
    let deleting: i64 =
        sqlx::query_scalar("SELECT count(*) FROM object_copies WHERE state='deleting'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(deleting, 0);
    let invalid:i64=sqlx::query_scalar("SELECT count(*) FROM checkpoint_objects p JOIN object_copies c ON c.generation=p.generation WHERE c.state<>'available'").fetch_one(&db).await.unwrap();
    assert_eq!(invalid, 0);
}
