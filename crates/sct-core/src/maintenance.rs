//! Durable scheduling independent of concrete scan/repair executors.
use crate::{
    auth::{self, Actor},
    content_catalog::lock_content_root,
    error::{Error, Result, id, require},
    lease::Writer,
    management::audit,
    operations::{OperationFence, check_fence},
    paging,
    roots::{json, revision},
};
use sct_protocol::*;
use sqlx::{Row, postgres::PgRow};

/// Opaque worker ownership; never sent over HTTP. Executors must do bounded work
/// and fence every catalog mutation before persisting physical I/O decisions.
pub struct MaintenanceClaim {
    pub(crate) job: String,
    pub(crate) fence: OperationFence,
    pub(crate) worker_epoch: i64,
    pub(crate) kind: MaintenanceKind,
    pub(crate) cursor: serde_json::Value,
}
impl MaintenanceClaim {
    pub fn kind(&self) -> &MaintenanceKind {
        &self.kind
    }
    pub fn cursor(&self) -> &serde_json::Value {
        &self.cursor
    }
    pub fn operation(&self) -> &OperationFence {
        &self.fence
    }
}
pub struct MaintenanceProgress {
    pub cursor: serde_json::Value,
    pub processed: u64,
    pub complete: bool,
}
pub trait MaintenanceExecutor: Send + Sync {
    fn advance(
        &self,
        claim: &MaintenanceClaim,
    ) -> impl std::future::Future<Output = Result<MaintenanceProgress>> + Send;
}
fn inspect(row: &PgRow) -> Result<MaintenanceJob> {
    Ok(MaintenanceJob {
        id: row.get::<String, _>("id").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        root_id: row.get::<String, _>("root_id").parse()?,
        kind: serde_json::from_value(json(&row.get::<String, _>("kind"))?)?,
        state: serde_json::from_value(json(&row.get::<String, _>("state"))?)?,
        processed: (row.get::<i64, _>("processed") as u64).try_into()?,
        attempts: row.get::<i32, _>("attempts") as u32,
        next_attempt_at_ms: row
            .get::<Option<i64>, _>("next_ms")
            .map(|n| (n as u64).try_into())
            .transpose()?,
        failure: row
            .get::<Option<String>, _>("failure")
            .map(|v| serde_json::from_value(serde_json::json!(v)))
            .transpose()?,
    })
}
fn actor(row: &PgRow) -> Actor {
    Actor {
        principal: row.get("run_principal_id"),
        credential: row
            .get::<Option<String>, _>("credential_id")
            .unwrap_or_default(),
    }
}
const INSPECT: &str = "SELECT *, (extract(epoch FROM next_attempt_at)*1000)::bigint AS next_ms FROM maintenance_jobs WHERE id=$1";
impl Writer {
    pub async fn start_maintenance(
        &self,
        actor: &Actor,
        request: &StartMaintenanceRequest,
    ) -> Result<MaintenanceJob> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roots WHERE id=$1)")
            .bind(request.root_id.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(exists, ErrorCode::NotFound)?;
        let job = id();
        let inserted=sqlx::query("INSERT INTO maintenance_jobs(id,root_id,principal_id,run_principal_id,credential_id,key,request,kind,revision) VALUES($1,$2,$3,$3,$4,$5,$6,$7,$8) ON CONFLICT(principal_id,key) DO NOTHING").bind(&job).bind(request.root_id.as_str()).bind(actor.principal_id()).bind(&actor.credential).bind(request.idempotency_key.as_str()).bind(json(request)?).bind(json(&request.kind)?.as_str().unwrap()).bind(id()).execute(&mut *tx).await?.rows_affected();
        let row=sqlx::query("SELECT *, (extract(epoch FROM next_attempt_at)*1000)::bigint AS next_ms FROM maintenance_jobs WHERE principal_id=$1 AND key=$2").bind(actor.principal_id()).bind(request.idempotency_key.as_str()).fetch_one(&mut *tx).await?;
        require(
            row.get::<serde_json::Value, _>("request") == json(request)?,
            ErrorCode::IdempotencyConflict,
        )?;
        if inserted == 1 {
            if request.kind == MaintenanceKind::MirrorRepair {
                let placement: serde_json::Value =
                    sqlx::query_scalar("SELECT placement FROM roots WHERE id=$1")
                        .bind(request.root_id.as_str())
                        .fetch_one(&mut *tx)
                        .await?;
                require(
                    matches!(
                        serde_json::from_value::<Placement>(placement)?,
                        Placement::Mirror { .. }
                    ),
                    ErrorCode::UnsupportedOperation,
                )?;
            }
            audit(
                &mut tx,
                actor,
                Some(request.root_id.as_str()),
                "maintenance.queued",
                serde_json::json!({"job_id":job,"kind":request.kind}),
            )
            .await?;
        }
        tx.commit().await?;
        inspect(&row)
    }
    pub async fn maintenance_job(
        &self,
        actor: &Actor,
        job: &OperationId,
    ) -> Result<MaintenanceJob> {
        let mut tx = self.transaction().await?;
        let row = sqlx::query(INSPECT)
            .bind(job.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        auth::authorize(
            &mut tx,
            actor,
            &row.get::<String, _>("root_id"),
            Capability::ListRead,
        )
        .await?;
        inspect(&row)
    }
    pub async fn list_maintenance(
        &self,
        actor: &Actor,
        root: Option<&RootId>,
        request: &ListRequest,
    ) -> Result<MaintenancePage> {
        let mut tx = self.transaction().await?;
        match root {
            Some(root) => {
                auth::authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?
            }
            None => auth::admin(&mut tx, actor).await?,
        };
        let scope = format!("maintenance:{}", root.map_or("*", RootId::as_str));
        let binding = json(&request.limit)?;
        let after: String = match &request.cursor {
            Some(c) => paging::consume(&mut tx, actor, &scope, c, &binding).await?,
            None => String::new(),
        };
        let rows=sqlx::query("SELECT *, (extract(epoch FROM next_attempt_at)*1000)::bigint AS next_ms FROM maintenance_jobs WHERE ($1::text IS NULL OR root_id=$1) AND id>$2 ORDER BY id LIMIT $3").bind(root.map(RootId::as_str)).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let next_cursor = if rows.len() > request.limit.get() as usize {
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    &scope,
                    binding,
                    json(&rows[request.limit.get() as usize - 1].get::<String, _>("id"))?,
                )
                .await?,
            )
        } else {
            None
        };
        let jobs = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(inspect)
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(MaintenancePage { jobs, next_cursor })
    }
    pub async fn resume_maintenance(
        &self,
        actor: &Actor,
        job: &OperationId,
        expected: &Revision,
    ) -> Result<MaintenanceJob> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row = sqlx::query("SELECT * FROM maintenance_jobs WHERE id=$1 FOR UPDATE")
            .bind(job.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        revision(&row, expected)?;
        require(
            matches!(
                row.get::<String, _>("state").as_str(),
                "blocked" | "failed" | "retry_wait"
            ),
            ErrorCode::InvalidRequest,
        )?;
        sqlx::query("UPDATE maintenance_jobs SET state='queued',run_principal_id=$2,credential_id=$3,attempts=0,next_attempt_at=NULL,failure=NULL,revision=$4,worker_expires_at=NULL WHERE id=$1").bind(job.as_str()).bind(actor.principal_id()).bind(&actor.credential).bind(id()).execute(&mut *tx).await?;
        audit(
            &mut tx,
            actor,
            Some(&row.get::<String, _>("root_id")),
            "maintenance.resumed",
            serde_json::json!({"job_id":job}),
        )
        .await?;
        let row = sqlx::query(INSPECT)
            .bind(job.as_str())
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        inspect(&row)
    }
    pub async fn claim_maintenance(&self, job: &OperationId) -> Result<Option<MaintenanceClaim>> {
        let mut tx = self.transaction().await?;
        let row=sqlx::query("SELECT * FROM maintenance_jobs WHERE id=$1 AND state IN ('queued','running','retry_wait') AND (next_attempt_at IS NULL OR next_attempt_at<=clock_timestamp()) AND (worker_expires_at IS NULL OR worker_expires_at<=clock_timestamp() OR server_epoch<>$2) FOR UPDATE SKIP LOCKED").bind(job.as_str()).bind(self.epoch).fetch_optional(&mut *tx).await?;
        let Some(row) = row else { return Ok(None) };
        let actor = actor(&row);
        if let Err(error) = auth::admin(&mut tx, &actor).await {
            sqlx::query("UPDATE maintenance_jobs SET state='blocked',failure=$2,revision=$3,worker_expires_at=NULL WHERE id=$1").bind(job.as_str()).bind(json(&error.0)?.as_str().unwrap()).bind(id()).execute(&mut *tx).await?;
            audit(
                &mut tx,
                &actor,
                Some(&row.get::<String, _>("root_id")),
                "maintenance.blocked",
                serde_json::json!({"job_id":job,"failure":error.0}),
            )
            .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let root: String = row.get("root_id");
        lock_content_root(&mut tx, &root).await?;
        let existing: Option<String> = row.get("operation_id");
        let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM operations WHERE root_id=$1 AND state IN ('active','recovering','cleaning_up') AND ($2::text IS NULL OR id<>$2))").bind(&root).bind(&existing).fetch_one(&mut *tx).await?;
        if busy {
            return Ok(None);
        };
        let operation = existing.unwrap_or_else(id);
        let kind: MaintenanceKind = serde_json::from_value(json(&row.get::<String, _>("kind"))?)?;
        let epoch:i64=sqlx::query_scalar("INSERT INTO operations(id,root_id,principal_id,kind,state,server_epoch,worker_expires_at) VALUES($1,$2,$3,$4,'active',$5,clock_timestamp()+interval '30 seconds') ON CONFLICT(id) DO UPDATE SET epoch=operations.epoch+1,server_epoch=EXCLUDED.server_epoch,state='active',worker_expires_at=EXCLUDED.worker_expires_at RETURNING epoch").bind(&operation).bind(&root).bind(actor.principal_id()).bind(if kind==MaintenanceKind::MirrorRepair{"repair"}else{"reclamation"}).bind(self.epoch).fetch_one(&mut *tx).await?;
        let worker_epoch:i64=sqlx::query_scalar("UPDATE maintenance_jobs SET operation_id=$2,state='running',worker_epoch=worker_epoch+1,server_epoch=$3,worker_expires_at=clock_timestamp()+interval '30 seconds',revision=$4 WHERE id=$1 RETURNING worker_epoch").bind(job.as_str()).bind(&operation).bind(self.epoch).bind(id()).fetch_one(&mut *tx).await?;
        let cursor = row.get("cursor");
        tx.commit().await?;
        Ok(Some(MaintenanceClaim {
            job: job.as_str().into(),
            fence: OperationFence {
                id: operation,
                root,
                epoch,
            },
            worker_epoch,
            kind,
            cursor,
        }))
    }
    pub async fn finish_maintenance_batch(
        &self,
        claim: &MaintenanceClaim,
        result: Result<MaintenanceProgress>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let row=sqlx::query("SELECT * FROM maintenance_jobs WHERE id=$1 AND state='running' AND worker_epoch=$2 AND server_epoch=$3 AND worker_expires_at>clock_timestamp() FOR UPDATE").bind(&claim.job).bind(claim.worker_epoch).bind(self.epoch).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::RevisionConflict))?;
        lock_content_root(&mut tx, &claim.fence.root).await?;
        check_fence(&mut tx, &claim.fence, self.epoch).await?;
        let run_actor = actor(&row);
        let result = match auth::admin(&mut tx, &run_actor).await {
            Ok(()) => result,
            Err(error) => Err(error),
        };
        match result {
            Ok(progress) => {
                require(
                    progress.processed <= MAX_SAFE_INTEGER,
                    ErrorCode::InvalidRequest,
                )?;
                if progress.complete {
                    let pending:bool=sqlx::query_scalar("SELECT uncertain_io OR EXISTS(SELECT 1 FROM reservations WHERE operation_id=$1) OR EXISTS(SELECT 1 FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE c.operation_id=$1 AND c.state<>'deleted' AND (a.root_id IS NULL OR c.state IN ('writing','deleting'))) FROM operations WHERE id=$1").bind(&claim.fence.id).fetch_one(&mut *tx).await?;
                    require(!pending, ErrorCode::RootBusy)?;
                    sqlx::query("UPDATE operations SET state='completed',worker_expires_at=NULL WHERE id=$1").bind(&claim.fence.id).execute(&mut *tx).await?;
                    audit(
                        &mut tx,
                        &run_actor,
                        Some(&claim.fence.root),
                        "maintenance.completed",
                        serde_json::json!({"job_id":claim.job}),
                    )
                    .await?;
                }
                sqlx::query("UPDATE maintenance_jobs SET state=$2,cursor=$3,processed=processed+$4,attempts=0,next_attempt_at=CASE WHEN $2='completed' THEN NULL ELSE clock_timestamp() END,failure=NULL,worker_expires_at=NULL,revision=$5 WHERE id=$1").bind(&claim.job).bind(if progress.complete{"completed"}else{"running"}).bind(progress.cursor).bind(progress.processed as i64).bind(id()).execute(&mut *tx).await?;
            }
            Err(error) => {
                let attempts = row.get::<i32, _>("attempts") + 1;
                let retry = matches!(
                    error.0,
                    ErrorCode::DestinationUnavailable
                        | ErrorCode::RateLimited
                        | ErrorCode::InternalError
                        | ErrorCode::RevisionConflict
                );
                let state = if retry && attempts < 5 {
                    "retry_wait"
                } else if retry {
                    "failed"
                } else {
                    "blocked"
                };
                sqlx::query("UPDATE maintenance_jobs SET state=$2,attempts=$3,next_attempt_at=CASE WHEN $2='retry_wait' THEN clock_timestamp()+make_interval(secs=>$4) ELSE NULL END,failure=$5,worker_expires_at=NULL,revision=$6 WHERE id=$1").bind(&claim.job).bind(state).bind(attempts.min(100)).bind(maintenance_retry_seconds(attempts)).bind(json(&error.0)?.as_str().unwrap()).bind(id()).execute(&mut *tx).await?;
                audit(&mut tx,&run_actor,Some(&claim.fence.root),"maintenance.failed_attempt",serde_json::json!({"job_id":claim.job,"state":state,"attempts":attempts,"failure":error.0})).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn renew_maintenance_claim(&self, claim: &MaintenanceClaim) -> Result<()> {
        let mut tx = self.transaction().await?;
        let row=sqlx::query("SELECT * FROM maintenance_jobs WHERE id=$1 AND state='running' AND worker_epoch=$2 AND server_epoch=$3 AND worker_expires_at>clock_timestamp() FOR UPDATE")
            .bind(&claim.job).bind(claim.worker_epoch).bind(self.epoch).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::RevisionConflict))?;
        auth::admin(&mut tx, &actor(&row)).await?;
        check_fence(&mut tx, &claim.fence, self.epoch).await?;
        sqlx::query("UPDATE operations SET worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1").bind(&claim.fence.id).execute(&mut *tx).await?;
        sqlx::query("UPDATE maintenance_jobs SET worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1").bind(&claim.job).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    /// At most eight jobs and one executor batch per job in a scheduling pass.
    pub async fn run_maintenance<E: MaintenanceExecutor>(&self, executor: &E) -> Result<()> {
        let jobs = {
            let mut tx = self.transaction().await?;
            sqlx::query_scalar::<_,String>("SELECT id FROM maintenance_jobs WHERE state IN ('queued','running','retry_wait') AND (next_attempt_at IS NULL OR next_attempt_at<=clock_timestamp()) AND (worker_expires_at IS NULL OR worker_expires_at<=clock_timestamp() OR server_epoch<>$1) AND NOT EXISTS(SELECT 1 FROM operations o WHERE o.root_id=maintenance_jobs.root_id AND o.state IN ('active','recovering','cleaning_up') AND (maintenance_jobs.operation_id IS NULL OR o.id<>maintenance_jobs.operation_id)) ORDER BY COALESCE(next_attempt_at,created_at),id LIMIT 8").bind(self.epoch).fetch_all(&mut *tx).await?
        };
        for job in jobs {
            if let Some(claim) = self.claim_maintenance(&job.parse()?).await? {
                let work = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    executor.advance(&claim),
                );
                tokio::pin!(work);
                let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(10));
                heartbeat.tick().await;
                let result = loop {
                    tokio::select! {
                        result=&mut work=>break result.unwrap_or(Err(Error(ErrorCode::DestinationUnavailable))),
                        _=heartbeat.tick()=>if let Err(error)=self.renew_maintenance_claim(&claim).await {break Err(error)},
                    }
                };
                self.finish_maintenance_batch(&claim, result).await?;
            }
        }
        Ok(())
    }
}

// Delay only: PostgreSQL remains authoritative for attempts, clocks and leases.
fn maintenance_retry_seconds(attempts: i32) -> f64 {
    let policy = simple_server::task_policies::RetryPolicy {
        max_attempts: std::num::NonZeroU32::new(7).unwrap(),
        initial_delay: std::time::Duration::from_secs(1),
        max_delay: std::time::Duration::from_secs(32),
        jitter: std::time::Duration::ZERO,
    };
    let attempt = u32::try_from(attempts.min(5)).expect("nonnegative persisted attempt count") + 1;
    policy
        .delay_after(attempt, 0.0)
        .expect("bounded retry delay")
        .as_secs_f64()
}

#[cfg(test)]
mod retry_policy_tests {
    #[test]
    fn delay_matches_existing_attempt_and_cap_boundaries() {
        for attempt in 0..=100 {
            assert_eq!(
                super::maintenance_retry_seconds(attempt),
                f64::from(1u32 << attempt.min(5))
            );
        }
    }
}
