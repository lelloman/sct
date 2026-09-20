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

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn checkpoint_exports_consistent_catalog_inventory_and_verified_hashes() {
    use sct_core::checkpoint_export::{CatalogHeader, CatalogRecord};
    use sha2::{Digest, Sha256};
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/export", 4000).await;
    let memory = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        w.clone(),
        BTreeMap::from([("one".into(), memory as Arc<dyn ObjectStore>)]),
        2,
        3600,
    )
    .unwrap();
    let first = ready_bytes(&engine, &w, &a, &r.id, "file", b"first")
        .await
        .unwrap();
    let receipt = engine.commit_upload(&a, &r.id, &first).await.unwrap();
    engine.tick().await.unwrap();
    let second = ready_bytes(&engine, &w, &a, &r.id, "file", b"second")
        .await
        .unwrap();
    let checkpoint = w
        .create_checkpoint(
            &a,
            &CreateCheckpointRequest {
                idempotency_key: "export".parse().unwrap(),
            },
        )
        .await
        .unwrap();
    for _ in 0..10 {
        engine.tick().await.unwrap();
        if w.checkpoint(&a, &checkpoint.id).await.unwrap().state == CheckpointState::Exporting {
            break;
        }
    }
    let directory = std::env::temp_dir().join(format!("sct-checkpoint-{}", uuid::Uuid::new_v4()));
    let exporter = engine.clone().with_checkpoint_directory(directory.clone());
    let task = tokio::spawn(async move { exporter.tick().await });
    for _ in 0..1000 {
        let frozen: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM checkpoint_barrier)")
            .fetch_one(&db)
            .await
            .unwrap();
        if !frozen {
            break;
        }
        tokio::task::yield_now().await;
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), w.renew())
        .await
        .unwrap()
        .unwrap();
    engine.commit_upload(&a, &r.id, &second).await.unwrap();
    task.await.unwrap().unwrap();
    let status = w.checkpoint(&a, &checkpoint.id).await.unwrap();
    assert_eq!(status.state, CheckpointState::Exported);
    let descriptor = w.checkpoint_descriptor(&a, &checkpoint.id).await.unwrap();
    assert_eq!(descriptor.package_state, RecoveryPackageState::CatalogOnly);
    descriptor.validate().unwrap();
    let artifact = directory.join(checkpoint.id.as_str());
    let catalog = tokio::fs::read(artifact.join("catalog.jsonl"))
        .await
        .unwrap();
    let inventory = tokio::fs::read(artifact.join("inventory.jsonl"))
        .await
        .unwrap();
    assert_eq!(
        descriptor.catalog_digest.as_str(),
        format!("{:x}", Sha256::digest(&catalog))
    );
    assert_eq!(
        descriptor.inventory_digest.as_str(),
        format!("{:x}", Sha256::digest(&inventory))
    );
    let text = String::from_utf8(catalog).unwrap();
    let mut lines = text.lines();
    let header: CatalogHeader = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(header.checkpoint_id, checkpoint.id);
    let mut current = Vec::new();
    for line in lines {
        let record: CatalogRecord = serde_json::from_str(line).unwrap();
        if record.table == "entries" && record.row["name"] == "file" {
            current.push(
                record.row["current_version_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
    }
    assert_eq!(current, vec![receipt.version_id.as_str()]);
    let mut cursor = None;
    let mut objects = Vec::new();
    loop {
        let page = w
            .checkpoint_inventory(
                &a,
                &checkpoint.id,
                &ListRequest {
                    limit: 1.try_into().unwrap(),
                    cursor,
                },
            )
            .await
            .unwrap();
        objects.extend(page.objects);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let from_file: Vec<CheckpointObject> = String::from_utf8(inventory)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(objects, from_file);
    assert_eq!(
        objects.iter().map(|o| o.bytes.get()).sum::<u64>(),
        status.pinned_bytes.get()
    );
    let exported = engine.with_checkpoint_directory(directory.clone());
    assert!(
        exported
            .checkpoint_artifact(&a, &checkpoint.id, "../catalog.jsonl")
            .await
            .is_err()
    );
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn checkpoint_export_failure_retains_pins_and_releases_service_barrier() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/failed-export", 4000).await;
    let directory =
        std::env::temp_dir().join(format!("sct-checkpoint-failure-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&directory, b"not a directory")
        .await
        .unwrap();
    let engine = TransferEngine::new(
        w.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        3600,
    )
    .unwrap()
    .with_checkpoint_directory(directory.clone());
    let upload = ready_bytes(&engine, &w, &a, &r.id, "file", b"bytes")
        .await
        .unwrap();
    engine.commit_upload(&a, &r.id, &upload).await.unwrap();
    engine.tick().await.unwrap();
    let checkpoint = w
        .create_checkpoint(
            &a,
            &CreateCheckpointRequest {
                idempotency_key: "failed".parse().unwrap(),
            },
        )
        .await
        .unwrap();
    for _ in 0..10 {
        let _ = engine.tick().await;
        if w.checkpoint(&a, &checkpoint.id).await.unwrap().state == CheckpointState::Failed {
            break;
        }
    }
    let failed = w.checkpoint(&a, &checkpoint.id).await.unwrap();
    assert_eq!(failed.state, CheckpointState::Failed);
    assert!(failed.pinned_bytes.get() > 0);
    assert_eq!(failed.failure, Some(ErrorCode::DestinationUnavailable));
    let barriers: i64 = sqlx::query_scalar("SELECT count(*) FROM checkpoint_barrier")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(barriers, 0);
    assert!(w.ready().await);
    assert!(w.checkpoint_descriptor(&a, &checkpoint.id).await.is_err());
    tokio::fs::remove_file(directory).await.unwrap();
}
