use super::*;
use sct_core::{
    maintenance::MaintenanceExecutor,
    payload::{Fault, MemoryStore, ObjectKey, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;
async fn finish(w: &Writer, e: &TransferEngine, a: &Actor, job: &OperationId) -> MaintenanceJob {
    for _ in 0..100 {
        e.tick().await.unwrap();
        let status = w.maintenance_job(a, job).await.unwrap();
        if status.state == MaintenanceState::Completed {
            return status;
        }
    }
    panic!("scan did not finish");
}
async fn start(w: &Writer, a: &Actor, root: &RootId, key: &str) -> MaintenanceJob {
    w.start_maintenance(
        a,
        &StartMaintenanceRequest {
            idempotency_key: key.parse().unwrap(),
            kind: MaintenanceKind::IntegrityScan,
            root_id: root.clone(),
        },
    )
    .await
    .unwrap()
}
async fn kinds(w: &Writer, a: &Actor, r: &RootId, job: &OperationId) -> Vec<IntegrityFindingKind> {
    w.list_integrity_findings(
        a,
        r,
        &ListRequest {
            limit: 100.try_into().unwrap(),
            cursor: None,
        },
    )
    .await
    .unwrap()
    .findings
    .into_iter()
    .filter(|f| &f.job_id == job)
    .map(|f| f.kind)
    .collect()
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn integrity_scan_classifies_missing_corrupt_inaccessible_and_catalog_findings() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/integrity", 4000).await;
    let memory = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        w.clone(),
        BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]),
        2,
        3600,
    )
    .unwrap();
    let upload = ready_bytes(&engine, &w, &a, &r.id, "file", b"abc")
        .await
        .unwrap();
    engine.commit_upload(&a, &r.id, &upload).await.unwrap();
    engine.tick().await.unwrap();
    let healthy = start(&w, &a, &r.id, "healthy").await;
    finish(&w, &engine, &a, &healthy.id).await;
    assert!(kinds(&w, &a, &r.id, &healthy.id).await.is_empty());
    let generation: String = sqlx::query_scalar(
        "SELECT generation FROM object_copies WHERE root_id=$1 AND digest=$2 AND state='available'",
    )
    .bind(r.id.as_str())
    .bind(digest(b"abc"))
    .fetch_one(&db)
    .await
    .unwrap();
    memory
        .delete(&ObjectKey {
            root: r.id.clone(),
            digest: digest(b"abc").parse().unwrap(),
            generation: generation.parse().unwrap(),
        })
        .await
        .unwrap();
    let missing = start(&w, &a, &r.id, "missing").await;
    finish(&w, &engine, &a, &missing.id).await;
    assert!(
        kinds(&w, &a, &r.id, &missing.id)
            .await
            .contains(&IntegrityFindingKind::Missing)
    );
    memory.fault(Fault::CorruptRead).await;
    let corrupt = start(&w, &a, &r.id, "corrupt").await;
    finish(&w, &engine, &a, &corrupt.id).await;
    assert!(
        kinds(&w, &a, &r.id, &corrupt.id)
            .await
            .contains(&IntegrityFindingKind::Corrupt)
    );
    memory.fault(Fault::Offline).await;
    let offline = start(&w, &a, &r.id, "offline").await;
    let claim = w.claim_maintenance(&offline.id).await.unwrap().unwrap();
    let result = engine.advance(&claim).await.unwrap();
    let count = kinds(&w, &a, &r.id, &offline.id).await.len();
    assert!(count > 0);
    let _ = engine.advance(&claim).await.unwrap(); // Lost progress response rechecks safely.
    assert_eq!(kinds(&w, &a, &r.id, &offline.id).await.len(), count);
    w.finish_maintenance_batch(&claim, Ok(result))
        .await
        .unwrap();
    finish(&w, &engine, &a, &offline.id).await;
    assert!(
        kinds(&w, &a, &r.id, &offline.id)
            .await
            .iter()
            .all(|k| *k == IntegrityFindingKind::Inaccessible)
    );
    memory.fault(Fault::None).await;
    sqlx::query("DELETE FROM content_edges WHERE root_id=$1")
        .bind(r.id.as_str())
        .execute(&db)
        .await
        .unwrap();
    let catalog = start(&w, &a, &r.id, "catalog").await;
    finish(&w, &engine, &a, &catalog.id).await;
    assert!(
        kinds(&w, &a, &r.id, &catalog.id)
            .await
            .contains(&IntegrityFindingKind::CatalogInconsistent)
    );
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn integrity_cursor_survives_writer_replacement_and_bounds_pages() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/scan-progress", 4000).await;
    sqlx::query("UPDATE pools SET capacity=200000")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE allocations SET capacity=90000")
        .execute(&db)
        .await
        .unwrap();
    let memory = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([("one".into(), memory as Arc<dyn ObjectStore>)]);
    let engine = TransferEngine::new(w.clone(), stores.clone(), 2, 3600).unwrap();
    for n in 0..20 {
        w.renew().await.unwrap();
        let upload = ready_bytes(
            &engine,
            &w,
            &a,
            &r.id,
            &format!("file-{n}"),
            format!("value-{n}").as_bytes(),
        )
        .await
        .unwrap();
        engine.commit_upload(&a, &r.id, &upload).await.unwrap();
        engine.tick().await.unwrap();
    }
    let job = start(&w, &a, &r.id, "restart").await;
    w.run_maintenance(&engine).await.unwrap();
    let progress = w.maintenance_job(&a, &job.id).await.unwrap();
    assert_eq!(progress.processed.get(), 16);
    assert_eq!(progress.state, MaintenanceState::Running);
    w.release().await.unwrap();
    let replacement = w.catalog().acquire_writer().await.unwrap();
    let resumed = TransferEngine::new(replacement.clone(), stores, 2, 3600).unwrap();
    let complete = finish(&replacement, &resumed, &a, &job.id).await;
    let objects: i64 = sqlx::query_scalar("SELECT count(*) FROM content_objects WHERE root_id=$1")
        .bind(r.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(complete.processed.get(), objects as u64);
    assert!(kinds(&replacement, &a, &r.id, &job.id).await.is_empty());
}
