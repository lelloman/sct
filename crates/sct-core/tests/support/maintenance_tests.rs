use super::*;
use sct_core::{
    error::{Error, Result},
    maintenance::{MaintenanceClaim, MaintenanceExecutor, MaintenanceProgress},
};
struct Steps;
impl MaintenanceExecutor for Steps {
    async fn advance(&self, claim: &MaintenanceClaim) -> Result<MaintenanceProgress> {
        let step = claim
            .cursor()
            .get("step")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            + 1;
        Ok(MaintenanceProgress {
            cursor: json!({"step":step}),
            processed: 1,
            complete: step == 3,
        })
    }
}
fn request_job(root: &RootId, key: &str) -> StartMaintenanceRequest {
    StartMaintenanceRequest {
        idempotency_key: key.parse().unwrap(),
        kind: MaintenanceKind::IntegrityScan,
        root_id: root.clone(),
    }
}
fn done() -> Result<MaintenanceProgress> {
    Ok(MaintenanceProgress {
        cursor: json!({}),
        processed: 1,
        complete: true,
    })
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn maintenance_queue_progress_restart_and_stale_worker_fencing() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/maintenance", 4000).await;
    let upload = w
        .begin_operation(&a, &r.id, OperationKind::Upload)
        .await
        .unwrap();
    let request = request_job(&r.id, "scan");
    let job = w.start_maintenance(&a, &request).await.unwrap();
    assert_eq!(w.start_maintenance(&a, &request).await.unwrap().id, job.id);
    assert_eq!(
        w.start_maintenance(
            &a,
            &StartMaintenanceRequest {
                kind: MaintenanceKind::MirrorRepair,
                ..request
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::IdempotencyConflict
    );
    w.run_maintenance(&Steps).await.unwrap();
    assert_eq!(
        w.maintenance_job(&a, &job.id).await.unwrap().state,
        MaintenanceState::Queued
    );
    w.finish_operation(&upload, true).await.unwrap();
    w.run_maintenance(&Steps).await.unwrap();
    assert_eq!(
        w.maintenance_job(&a, &job.id)
            .await
            .unwrap()
            .processed
            .get(),
        1
    );
    let stale = w.claim_maintenance(&job.id).await.unwrap().unwrap();
    assert!(w.claim_maintenance(&job.id).await.unwrap().is_none());
    sqlx::query("UPDATE maintenance_jobs SET worker_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(job.id.as_str()).execute(&db).await.unwrap();
    let current = w.claim_maintenance(&job.id).await.unwrap().unwrap();
    assert_eq!(
        w.finish_maintenance_batch(&stale, done())
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
    w.finish_maintenance_batch(&current, Steps.advance(&current).await)
        .await
        .unwrap();
    assert_eq!(
        w.maintenance_job(&a, &job.id)
            .await
            .unwrap()
            .processed
            .get(),
        2
    );
    let before = w.claim_maintenance(&job.id).await.unwrap().unwrap();
    w.release().await.unwrap();
    let replacement = w.catalog().acquire_writer().await.unwrap();
    assert!(w.finish_maintenance_batch(&before, done()).await.is_err());
    replacement.run_maintenance(&Steps).await.unwrap();
    let completed = replacement.maintenance_job(&a, &job.id).await.unwrap();
    assert_eq!(completed.state, MaintenanceState::Completed);
    assert_eq!(completed.processed.get(), 3);
    assert!(replacement.inspect_operation(&a, &r.id).await.is_err());
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn maintenance_retries_authorization_resume_and_capacity_obligations() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/retry", 4000).await;
    let job = w
        .start_maintenance(&a, &request_job(&r.id, "retry"))
        .await
        .unwrap();
    for attempt in 1..=5 {
        let claim = w.claim_maintenance(&job.id).await.unwrap().unwrap();
        w.finish_maintenance_batch(&claim, Err(Error(ErrorCode::DestinationUnavailable)))
            .await
            .unwrap();
        let status = w.maintenance_job(&a, &job.id).await.unwrap();
        assert_eq!(status.attempts, attempt);
        assert_eq!(
            status.state,
            if attempt < 5 {
                MaintenanceState::RetryWait
            } else {
                MaintenanceState::Failed
            }
        );
        assert!(w.claim_maintenance(&job.id).await.unwrap().is_none());
        sqlx::query("UPDATE maintenance_jobs SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(job.id.as_str()).execute(&db).await.unwrap();
    }
    assert_eq!(
        w.begin_operation(&a, &r.id, OperationKind::Upload)
            .await
            .err()
            .unwrap()
            .0,
        ErrorCode::RootBusy
    );
    let failed = w.maintenance_job(&a, &job.id).await.unwrap();
    assert!(
        w.resume_maintenance(&a, &job.id, &job.revision)
            .await
            .is_err()
    );
    w.resume_maintenance(&a, &job.id, &failed.revision)
        .await
        .unwrap();
    w.create_principal(&a, &"rescuer".parse().unwrap())
        .await
        .unwrap();
    let p = w
        .inspect_principal(&a, &"rescuer".parse().unwrap())
        .await
        .unwrap();
    w.update_principal(
        &a,
        &p.id,
        &UpdatePrincipalRequest {
            expected_revision: p.revision,
            disabled: false,
            administrator: true,
        },
    )
    .await
    .unwrap();
    let token = w.issue_token(&a, &p.id).await.unwrap();
    let rescuer = w.authenticate(&token.secret, false, None).await.unwrap();
    let old: String = sqlx::query_scalar(
        "SELECT id FROM credentials WHERE principal_id='local_admin' AND NOT revoked",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    w.revoke_credential(&rescuer, &old).await.unwrap();
    assert!(w.claim_maintenance(&job.id).await.unwrap().is_none());
    let blocked = w.maintenance_job(&rescuer, &job.id).await.unwrap();
    assert_eq!(blocked.state, MaintenanceState::Blocked);
    w.resume_maintenance(&rescuer, &job.id, &blocked.revision)
        .await
        .unwrap();
    let claim = w.claim_maintenance(&job.id).await.unwrap().unwrap();
    w.reserve_staging(
        claim.operation(),
        &"pool".parse().unwrap(),
        10.try_into().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        w.finish_maintenance_batch(&claim, done())
            .await
            .unwrap_err()
            .0,
        ErrorCode::RootBusy
    );
    assert_eq!(
        w.maintenance_job(&rescuer, &job.id).await.unwrap().state,
        MaintenanceState::Running
    );
    w.release_unused_reservations(claim.operation())
        .await
        .unwrap();
    w.finish_maintenance_batch(&claim, done()).await.unwrap();
    let identities: (String, String) =
        sqlx::query_as("SELECT principal_id,run_principal_id FROM maintenance_jobs WHERE id=$1")
            .bind(job.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(identities, ("local_admin".into(), "rescuer".into()));
}
