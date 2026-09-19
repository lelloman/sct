//! Forward-only retirement jobs. Root ownership outlives requests and credentials.
use crate::{
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Error, Result, id, require},
    operations::{OperationFence, check_fence},
    roots::{json, lock_root},
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

pub(crate) fn inspect(row: &PgRow) -> Result<LifecycleJob> {
    Ok(LifecycleJob {
        id: row.get::<String, _>("operation_id").parse()?,
        root_id: row.get::<String, _>("root_id").parse()?,
        state: serde_json::from_value(json(&row.get::<String, _>("state"))?)?,
        phase: row.get("phase"),
        processed: (row.get::<i64, _>("processed") as u64).try_into()?,
        total: row
            .get::<Option<i64>, _>("total")
            .map(|n| (n as u64).try_into())
            .transpose()?,
        failure: row
            .get::<Option<String>, _>("failure")
            .map(|s| serde_json::from_value(serde_json::json!(s)))
            .transpose()?,
    })
}
pub(crate) fn required(kind: &str) -> Capability {
    if kind == "purge" {
        Capability::Purge
    } else {
        Capability::ManagePolicy
    }
}
async fn authorize_job(db: &mut PgConnection, actor: &Actor, row: &PgRow) -> Result<()> {
    let root: String = row.get("root_id");
    authorize(db, actor, &root, required(&row.get::<String, _>("kind"))).await?;
    if let Some(preview) = row.get::<Option<String>, _>("preview_id") {
        let request: serde_json::Value =
            sqlx::query_scalar("SELECT request FROM change_previews WHERE root_id=$1 AND id=$2")
                .bind(&root)
                .bind(preview)
                .fetch_one(&mut *db)
                .await?;
        let request: PreviewChangeRequest = serde_json::from_value(request)?;
        if matches!(request.change, PolicyChange::Move { .. }) {
            authorize(db, actor, &root, Capability::Write).await?;
        }
    }
    Ok(())
}
pub(crate) async fn apply_policy(
    db: &mut PgConnection,
    root: &str,
    change: &PolicyChange,
) -> Result<()> {
    match change {
        PolicyChange::Set {
            scope: PolicyScope::Root { .. },
            policy,
            ..
        } => {
            sqlx::query("UPDATE roots SET default_policy=$2,policy_revision=$3 WHERE id=$1")
                .bind(root)
                .bind(json(policy)?)
                .bind(id())
                .execute(&mut *db)
                .await?;
        }
        PolicyChange::Set {
            scope: PolicyScope::Entry { entry_id, .. },
            policy,
            ..
        } => {
            sqlx::query("UPDATE entries SET policy=$3,policy_id=COALESCE(policy_id,$4),policy_revision=$5,revision=$6 WHERE root_id=$1 AND id=$2")
                .bind(root).bind(entry_id.as_str()).bind(json(policy)?).bind(id()).bind(id()).bind(id()).execute(&mut *db).await?;
        }
        PolicyChange::Inherit {
            scope: PolicyScope::Entry { entry_id, .. },
            ..
        } => {
            sqlx::query("UPDATE entries SET policy=NULL,policy_id=NULL,policy_revision=NULL,revision=$3 WHERE root_id=$1 AND id=$2")
                .bind(root).bind(entry_id.as_str()).bind(id()).execute(&mut *db).await?;
        }
        PolicyChange::Move { .. } => crate::namespace_moves::apply(db, root, change).await?,
        _ => return Err(Error(ErrorCode::UnsupportedOperation)),
    }
    sqlx::query("UPDATE roots SET revision=$2 WHERE id=$1")
        .bind(root)
        .bind(id())
        .execute(db)
        .await?;
    Ok(())
}

impl TransferEngine {
    pub async fn apply_change(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &ApplyChangeRequest,
    ) -> Result<LifecycleJob> {
        request.validate()?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ManagePolicy).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ManagePolicy).await?;
        if let Some(row) = sqlx::query(
            "SELECT * FROM lifecycle_jobs WHERE root_id=$1 AND principal_id=$2 AND key=$3",
        )
        .bind(root.as_str())
        .bind(&actor.principal)
        .bind(request.idempotency_key.as_str())
        .fetch_optional(&mut *tx)
        .await?
        {
            require(
                row.get::<serde_json::Value, _>("request") == json(request)?,
                ErrorCode::IdempotencyConflict,
            )?;
            return inspect(&row);
        }
        lock_root(&mut tx, root.as_str()).await?;
        let preview =
            crate::policy_preview::load(&mut tx, root.as_str(), request.preview_id.as_str())
                .await?;
        let captured = crate::policy_preview::check_preview(&mut tx, actor, &preview).await?;
        require(
            preview.get::<String, _>("state") == "ready"
                && request.expected_root_revision.as_str()
                    == preview.get::<String, _>("root_revision"),
            ErrorCode::RevisionConflict,
        )?;
        let operation = id();
        sqlx::query("INSERT INTO operations(id,root_id,principal_id,kind,state,phase,server_epoch,worker_expires_at) VALUES($1,$2,$3,'policy_transition','active','retiring',$4,clock_timestamp()+interval '30 seconds')")
            .bind(&operation).bind(root.as_str()).bind(&actor.principal).bind(self.writer.epoch).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO lifecycle_jobs(operation_id,root_id,principal_id,run_principal_id,credential_id,key,request,preview_id,state,phase,total) VALUES($1,$2,$3,$3,$4,$5,$6,$7,'applying','retiring',$8)")
            .bind(&operation).bind(root.as_str()).bind(&actor.principal).bind(&actor.credential).bind(request.idempotency_key.as_str()).bind(json(request)?).bind(request.preview_id.as_str()).bind(preview.get::<i64,_>("retire_count")).execute(&mut *tx).await?;
        apply_policy(&mut tx, root.as_str(), &captured.change).await?;
        sqlx::query("UPDATE change_previews SET state='applying' WHERE id=$1")
            .bind(request.preview_id.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'policy_applied',$3)")
            .bind(root.as_str()).bind(&operation).bind(json(&captured.change)?).execute(&mut *tx).await?;
        let row = sqlx::query("SELECT * FROM lifecycle_jobs WHERE operation_id=$1")
            .bind(operation)
            .fetch_one(&mut *tx)
            .await?;
        let result = inspect(&row)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn lifecycle_job(
        &self,
        actor: &Actor,
        root: &RootId,
        operation: &OperationId,
    ) -> Result<LifecycleJob> {
        let mut tx = self.writer.transaction().await?;
        let row=sqlx::query("SELECT j.*,o.kind FROM lifecycle_jobs j JOIN operations o ON o.id=j.operation_id WHERE j.root_id=$1 AND j.operation_id=$2")
            .bind(root.as_str()).bind(operation.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        authorize(
            &mut tx,
            actor,
            root.as_str(),
            required(&row.get::<String, _>("kind")),
        )
        .await?;
        inspect(&row)
    }
    pub async fn advance_lifecycle_job(
        &self,
        actor: &Actor,
        root: &RootId,
        operation: &OperationId,
    ) -> Result<LifecycleJob> {
        self.lifecycle_job(actor, root, operation).await?;
        self.tick().await?;
        self.lifecycle_job(actor, root, operation).await
    }
    pub async fn resume_lifecycle_job(
        &self,
        actor: &Actor,
        root: &RootId,
        operation: &OperationId,
    ) -> Result<LifecycleJob> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        let row=sqlx::query("SELECT j.*,o.kind FROM lifecycle_jobs j JOIN operations o ON o.id=j.operation_id WHERE j.root_id=$1 AND j.operation_id=$2")
            .bind(root.as_str()).bind(operation.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        authorize(
            &mut tx,
            actor,
            root.as_str(),
            required(&row.get::<String, _>("kind")),
        )
        .await?;
        require(
            row.get::<String, _>("state") == "blocked",
            ErrorCode::InvalidRequest,
        )?;
        authorize_job(&mut tx, actor, &row).await?;
        sqlx::query("UPDATE lifecycle_jobs SET run_principal_id=$2,credential_id=$3,failure=NULL,state=CASE WHEN phase='cleanup' THEN 'cleaning_up' ELSE 'applying' END WHERE operation_id=$1")
            .bind(operation.as_str()).bind(&actor.principal).bind(&actor.credential).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'job_resumed',$3)")
            .bind(root.as_str()).bind(operation.as_str()).bind(serde_json::json!({"principal_id":actor.principal})).execute(&mut *tx).await?;
        tx.commit().await?;
        self.lifecycle_job(actor, root, operation).await
    }
    /// The stored run credential must still authorize each logical batch.
    pub(crate) async fn advance_lifecycle(&self, fence: &OperationFence) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &fence.root).await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        let row=sqlx::query("SELECT j.*,o.kind FROM lifecycle_jobs j JOIN operations o ON o.id=j.operation_id WHERE j.operation_id=$1")
            .bind(&fence.id).fetch_optional(&mut *tx).await?;
        let Some(row) = row else {
            return Ok(());
        };
        if row.get::<String, _>("state") != "applying" {
            return Ok(());
        }
        let actor = Actor {
            principal: row.get("run_principal_id"),
            credential: row.get("credential_id"),
        };
        if let Err(error) = authorize_job(&mut tx, &actor, &row).await {
            block(&mut tx, &fence.id, &error.0).await?;
            tx.commit().await?;
            return Ok(());
        }
        let preview: String = row
            .get::<Option<String>, _>("preview_id")
            .ok_or(Error(ErrorCode::InvalidRequest))?;
        let after = row.get::<Option<String>, _>("cursor").unwrap_or_default();
        let versions:Vec<String>=sqlx::query_scalar("SELECT version_id FROM preview_versions WHERE root_id=$1 AND preview_id=$2 AND retire AND version_id>$3 ORDER BY version_id LIMIT 64")
            .bind(&fence.root).bind(preview).bind(after).fetch_all(&mut *tx).await?;
        for version in &versions {
            let current: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM entries WHERE root_id=$1 AND current_version_id=$2)",
            )
            .bind(&fence.root)
            .bind(version)
            .fetch_one(&mut *tx)
            .await?;
            require(!current, ErrorCode::RevisionConflict)?;
            sqlx::query(
                "UPDATE file_versions SET retired=true WHERE root_id=$1 AND id=$2 AND NOT retired",
            )
            .bind(&fence.root)
            .bind(version)
            .execute(&mut *tx)
            .await?;
        }
        let done = versions.len() < 64;
        sqlx::query("UPDATE lifecycle_jobs SET processed=processed+$2,cursor=$3,state=$4,phase=$5 WHERE operation_id=$1")
            .bind(&fence.id).bind(versions.len() as i64).bind(versions.last()).bind(if done {"cleaning_up"}else{"applying"}).bind(if done {"cleanup"}else{"retiring"}).execute(&mut *tx).await?;
        sqlx::query("UPDATE operations SET phase=$2 WHERE id=$1")
            .bind(&fence.id)
            .bind(if done { "cleanup" } else { "retiring" })
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'policy_retirement_batch',$3)")
            .bind(&fence.root).bind(&fence.id).bind(serde_json::json!({"versions":versions})).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn lifecycle_failure(
        &self,
        fence: &OperationFence,
        code: &ErrorCode,
        logical: bool,
    ) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        // Physical cleanup can retry automatically. An authorization-blocked
        // logical phase is only resumed by an explicit authorized request.
        sqlx::query("UPDATE lifecycle_jobs SET state='blocked',failure=$2 WHERE operation_id=$1 AND (phase='cleanup' OR ($3 AND state='applying'))")
            .bind(&fence.id).bind(json(code)?.as_str().unwrap()).bind(logical).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn complete_lifecycle(&self, fence: &OperationFence) -> Result<bool> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &fence.root).await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        let job: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM lifecycle_jobs WHERE operation_id=$1)")
                .bind(&fence.id)
                .fetch_one(&mut *tx)
                .await?;
        if !job {
            return Ok(false);
        }
        let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM operations WHERE id=$1 AND uncertain_io) OR EXISTS(SELECT 1 FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE c.operation_id=$1 AND c.state<>'deleted' AND (a.root_id IS NULL OR c.state IN ('writing','deleting'))) OR EXISTS(SELECT 1 FROM reservations WHERE operation_id=$1) OR EXISTS(SELECT 1 FROM read_pins p JOIN file_versions v ON v.root_id=p.root_id AND v.id=p.version_id WHERE p.root_id=$2 AND v.retired AND p.expires_at>clock_timestamp())")
            .bind(&fence.id).bind(&fence.root).fetch_one(&mut *tx).await?;
        if pending {
            sqlx::query(
                "UPDATE lifecycle_jobs SET state='cleaning_up',failure=NULL WHERE operation_id=$1",
            )
            .bind(&fence.id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(true);
        }
        sqlx::query("UPDATE lifecycle_jobs SET state='completed',phase='completed',failure=NULL WHERE operation_id=$1").bind(&fence.id).execute(&mut *tx).await?;
        sqlx::query("UPDATE change_previews SET state='completed' WHERE id=(SELECT preview_id FROM lifecycle_jobs WHERE operation_id=$1)").bind(&fence.id).execute(&mut *tx).await?;
        sqlx::query("UPDATE operations SET state='completed',phase='completed',worker_expires_at=NULL WHERE id=$1").bind(&fence.id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'job_completed','{}')").bind(&fence.root).bind(&fence.id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
}
async fn block(db: &mut PgConnection, operation: &str, code: &ErrorCode) -> Result<()> {
    sqlx::query("UPDATE lifecycle_jobs SET state='blocked',failure=$2 WHERE operation_id=$1")
        .bind(operation)
        .bind(json(code)?.as_str().unwrap())
        .execute(db)
        .await?;
    Ok(())
}
