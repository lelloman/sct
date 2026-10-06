use super::*;
use sct_core::{
    maintenance::MaintenanceExecutor,
    payload::{Fault, MemoryStore, ObjectKey, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;

async fn job(w: &Writer, a: &Actor, root: &RootId, kind: MaintenanceKind) -> MaintenanceJob {
    w.start_maintenance(
        a,
        &StartMaintenanceRequest {
            idempotency_key: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
            root_id: root.clone(),
            kind,
        },
    )
    .await
    .unwrap()
}
async fn complete(w: &Writer, e: &TransferEngine, a: &Actor, id: &OperationId) -> MaintenanceJob {
    for _ in 0..30 {
        e.tick().await.unwrap();
        let status = w.maintenance_job(a, id).await.unwrap();
        if status.state == MaintenanceState::Completed {
            return status;
        }
        assert!(
            !matches!(
                status.state,
                MaintenanceState::Blocked | MaintenanceState::Failed
            ),
            "{status:?}"
        );
    }
    panic!("maintenance did not complete");
}
async fn scenario(fault: Fault, healthy_source: bool, replay_progress: bool, no_capacity: bool) {
    let (w, a, db) = setup().await;
    let mut req = request(a.principal_id(), "/repair", 4000);
    let Placement::Single {
        mut read_policy, ..
    } = req.placement
    else {
        unreachable!()
    };
    let mut second = read_policy.candidates[0].clone();
    second.backend_id = "two".parse().unwrap();
    read_policy.candidates.push(second);
    req.placement = Placement::Mirror {
        backend_ids: ["one".parse().unwrap(), "two".parse().unwrap()],
        read_policy,
    };
    req.allocations[0].retained_capacity_bytes = 8000.try_into().unwrap();
    let r = w.create_root(&a, &req).await.unwrap();
    w.update_root(
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
    let source = Arc::new(MemoryStore::default());
    let target = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([
        ("one".into(), source.clone() as Arc<dyn ObjectStore>),
        ("two".into(), target.clone() as Arc<dyn ObjectStore>),
    ]);
    let engine = TransferEngine::new(w.clone(), stores.clone(), 2, 3600).unwrap();
    let upload = ready_bytes(&engine, &w, &a, &r.id, "file", b"abc")
        .await
        .unwrap();
    engine.commit_upload(&a, &r.id, &upload).await.unwrap();
    engine.tick().await.unwrap();
    let before = w.root_usage(&a, &r.id).await.unwrap();
    let old: String = db::query_scalar("SELECT generation FROM object_copies WHERE root_id=$1 AND backend_id='two' AND digest=$2 AND state='available'").bind(r.id.as_str()).bind(digest(b"abc")).fetch_one(&db).await.unwrap();
    target
        .delete(&ObjectKey {
            root: r.id.clone(),
            digest: digest(b"abc").parse().unwrap(),
            generation: old.parse().unwrap(),
        })
        .await
        .unwrap();
    let scan = job(&w, &a, &r.id, MaintenanceKind::IntegrityScan).await;
    complete(&w, &engine, &a, &scan.id).await;
    target.fault(fault).await;
    let staging: String = db::query_scalar("SELECT id FROM allocations WHERE root_id IS NULL")
        .fetch_one(&db)
        .await
        .unwrap();
    if no_capacity {
        let allocation = w
            .allocation_management(&a, &staging.parse().unwrap())
            .await
            .unwrap();
        w.resize_allocation(
            &a,
            &staging.parse().unwrap(),
            &ResizeAllocationRequest {
                expected_revision: allocation.revision,
                capacity_bytes: 0.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    }
    if !healthy_source {
        source.fault(Fault::CorruptRead).await;
    }
    let repair = job(&w, &a, &r.id, MaintenanceKind::MirrorRepair).await;
    if replay_progress {
        let claim = w.claim_maintenance(&repair.id).await.unwrap().unwrap();
        let first = engine.advance(&claim).await.unwrap();
        let replay = engine.advance(&claim).await.unwrap();
        assert_eq!(first.processed, replay.processed);
        assert_eq!(first.cursor, replay.cursor);
        w.finish_maintenance_batch(&claim, Ok(replay))
            .await
            .unwrap();
    } else {
        engine.tick().await.unwrap();
    }
    if !healthy_source || no_capacity {
        assert_eq!(
            w.maintenance_job(&a, &repair.id).await.unwrap().state,
            MaintenanceState::Blocked
        );
        assert_eq!(w.root_usage(&a, &r.id).await.unwrap(), before);
        let old_state: String =
            db::query_scalar("SELECT state FROM object_copies WHERE generation=$1")
                .bind(&old)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(old_state, "available");
        source.fault(Fault::None).await;
        if no_capacity {
            let allocation = w
                .allocation_management(&a, &staging.parse().unwrap())
                .await
                .unwrap();
            w.resize_allocation(
                &a,
                &staging.parse().unwrap(),
                &ResizeAllocationRequest {
                    expected_revision: allocation.revision,
                    capacity_bytes: 1000.try_into().unwrap(),
                },
            )
            .await
            .unwrap();
            let retained: String = db::query_scalar("SELECT id FROM allocations WHERE root_id=$1")
                .bind(r.id.as_str())
                .fetch_one(&db)
                .await
                .unwrap();
            let allocation = w
                .allocation_management(&a, &retained.parse().unwrap())
                .await
                .unwrap();
            w.resize_allocation(
                &a,
                &retained.parse().unwrap(),
                &ResizeAllocationRequest {
                    expected_revision: allocation.revision,
                    capacity_bytes: 8500.try_into().unwrap(),
                },
            )
            .await
            .unwrap();
        }
        let status = w.maintenance_job(&a, &repair.id).await.unwrap();
        w.resume_maintenance(&a, &repair.id, &status.revision)
            .await
            .unwrap();
    }
    target.fault(Fault::None).await;
    db::query("UPDATE maintenance_jobs SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(repair.id.as_str()).execute(&db).await.unwrap();
    w.release().await.unwrap();
    let resumed = w.catalog().acquire_writer().await.unwrap();
    let engine = TransferEngine::new(resumed.clone(), stores, 2, 3600).unwrap();
    let done = complete(&resumed, &engine, &a, &repair.id).await;
    assert_eq!(done.processed.get(), 1);
    assert_eq!(resumed.root_usage(&a, &r.id).await.unwrap(), before);
    let old_state: String = db::query_scalar("SELECT state FROM object_copies WHERE generation=$1")
        .bind(&old)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(old_state, "deleted");
    let findings = resumed
        .list_integrity_findings(
            &a,
            &r.id,
            &ListRequest {
                limit: 100.try_into().unwrap(),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(findings.findings.len(), 1);
    assert!(findings.findings[0].repaired);
    let scan = job(&resumed, &a, &r.id, MaintenanceKind::IntegrityScan).await;
    complete(&resumed, &engine, &a, &scan.id).await;
    assert_eq!(
        resumed
            .list_integrity_findings(
                &a,
                &r.id,
                &ListRequest {
                    limit: 100.try_into().unwrap(),
                    cursor: None
                }
            )
            .await
            .unwrap()
            .findings
            .len(),
        1
    );
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn mirror_repair_replays_completed_work_without_double_charging() {
    scenario(Fault::None, true, true, false).await;
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn mirror_repair_recovers_lost_write_acknowledgement() {
    scenario(Fault::LostWriteResponse, true, false, false).await;
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn mirror_repair_retains_uncertain_delete_charges() {
    scenario(Fault::UnknownDelete, true, false, false).await;
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn mirror_repair_requires_a_verified_alternate() {
    scenario(Fault::None, false, false, false).await;
}
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn mirror_repair_blocks_before_deletion_until_capacity_is_available() {
    scenario(Fault::None, true, false, true).await;
}
