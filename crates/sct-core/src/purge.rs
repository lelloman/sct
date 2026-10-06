//! Explicit purge owns the root until its bounded retirement and physical cleanup finish.
use crate::{
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Error, Result, id, require},
    roots::{json, lock_root, revision, root_row},
    transfer_io::TransferEngine,
};
use db::PgConnection;
use sct_protocol::*;
use simple_server::database::postgres as db;

impl TransferEngine {
    pub async fn purge(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &PurgeRequest,
    ) -> Result<LifecycleJob> {
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Purge).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Purge).await?;
        if let Some(row) = db::query("SELECT j.*,o.kind FROM lifecycle_jobs j JOIN operations o ON o.id=j.operation_id WHERE j.root_id=$1 AND j.principal_id=$2 AND j.key=$3")
            .bind(root.as_str()).bind(&actor.principal).bind(request.idempotency_key.as_str()).fetch_optional(&mut *tx).await? {
            require(row.get::<String,_>("kind") == "purge" && row.get::<serde_json::Value,_>("request") == json(request)?, ErrorCode::IdempotencyConflict)?;
            return crate::lifecycle::inspect(&row);
        }
        lock_root(&mut tx, root.as_str()).await?;
        let root_row = root_row(&mut tx, root.as_str()).await?;
        require(
            root_row.get::<String, _>("state") == "enabled",
            ErrorCode::DestinationUnavailable,
        )?;
        let (entry, version) = match &request.target {
            PurgeTarget::HistoricalVersion { version_id } => {
                revision(&root_row, &request.expected_revision)?;
                let entry: String = db::query_scalar("SELECT entry_id FROM file_versions WHERE root_id=$1 AND id=$2 AND NOT retired AND NOT entry_purged(entry_id)")
                    .bind(root.as_str()).bind(version_id.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
                let current: bool = db::query_scalar("SELECT EXISTS(SELECT 1 FROM entries WHERE root_id=$1 AND current_version_id=$2)")
                    .bind(root.as_str()).bind(version_id.as_str()).fetch_one(&mut *tx).await?;
                require(!current, ErrorCode::InvalidRequest)?;
                // Retire in the acceptance transaction: new pins stop immediately.
                db::query("UPDATE file_versions SET retired=true WHERE root_id=$1 AND id=$2")
                    .bind(root.as_str())
                    .bind(version_id.as_str())
                    .execute(&mut *tx)
                    .await?;
                (None, Some((entry, version_id.as_str().to_owned())))
            }
            PurgeTarget::HiddenEntry { entry_id } => {
                let row = crate::namespace::row(&mut tx, entry_id.as_str()).await?;
                require(
                    row.get::<String, _>("root_id") == root.as_str(),
                    ErrorCode::NotFound,
                )?;
                revision(&row, &request.expected_revision)?;
                require(
                    row.get::<Option<String>, _>("parent_id").is_some()
                        && crate::namespace::hidden(&mut tx, entry_id.as_str()).await?,
                    ErrorCode::InvalidRequest,
                )?;
                db::query("UPDATE entries SET purged=true,hidden=true,revision=$3 WHERE root_id=$1 AND id=$2")
                    .bind(root.as_str()).bind(entry_id.as_str()).bind(id()).execute(&mut *tx).await?;
                db::query("UPDATE entries SET revision=$2 WHERE id=$1")
                    .bind(row.get::<Option<String>, _>("parent_id"))
                    .bind(id())
                    .execute(&mut *tx)
                    .await?;
                (Some(entry_id.as_str().to_owned()), None)
            }
        };
        let operation = id();
        db::query("INSERT INTO operations(id,root_id,principal_id,kind,state,phase,server_epoch,worker_expires_at) VALUES($1,$2,$3,'purge','active','purging',$4,clock_timestamp()+interval '30 seconds')")
            .bind(&operation).bind(root.as_str()).bind(&actor.principal).bind(self.writer.epoch).execute(&mut *tx).await?;
        db::query("INSERT INTO lifecycle_jobs(operation_id,root_id,principal_id,run_principal_id,credential_id,key,request,state,phase,processed) VALUES($1,$2,$3,$3,$4,$5,$6,'applying','purging',$7)")
            .bind(&operation).bind(root.as_str()).bind(&actor.principal).bind(&actor.credential).bind(request.idempotency_key.as_str()).bind(json(request)?).bind(if version.is_some(){1i64}else{0}).execute(&mut *tx).await?;
        if let Some(entry) = entry {
            db::query("INSERT INTO lifecycle_work(root_id,operation_id,entry_id) VALUES($1,$2,$3)")
                .bind(root.as_str())
                .bind(&operation)
                .bind(entry)
                .execute(&mut *tx)
                .await?;
        }
        db::query("UPDATE roots SET revision=$2 WHERE id=$1")
            .bind(root.as_str())
            .bind(id())
            .execute(&mut *tx)
            .await?;
        db::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'purge_applied',$3)")
            .bind(root.as_str()).bind(&operation).bind(json(request)?).execute(&mut *tx).await?;
        let row = db::query("SELECT * FROM lifecycle_jobs WHERE operation_id=$1")
            .bind(operation)
            .fetch_one(&mut *tx)
            .await?;
        let result = crate::lifecycle::inspect(&row)?;
        tx.commit().await?;
        Ok(result)
    }
}

/// At most 64 entries and 64 versions per pass; child expansion is capped at 128.
pub(crate) async fn advance(db: &mut PgConnection, root: &str, operation: &str) -> Result<()> {
    let mut retired = Vec::<String>::new();
    for _ in 0..64 {
        let row = db::query("SELECT * FROM lifecycle_work WHERE root_id=$1 AND operation_id=$2 ORDER BY sequence LIMIT 1")
            .bind(root).bind(operation).fetch_optional(&mut *db).await?;
        let Some(row) = row else { break };
        let entry: String = row.get("entry_id");
        let sequence: i64 = row.get("sequence");
        db::query("UPDATE entries SET purged=true,hidden=true,current_version_id=NULL WHERE root_id=$1 AND id=$2 AND (NOT purged OR NOT hidden OR current_version_id IS NOT NULL)")
            .bind(root).bind(&entry).execute(&mut *db).await?;
        let versions: Vec<String> = db::query_scalar("SELECT id FROM file_versions WHERE root_id=$1 AND entry_id=$2 AND NOT retired ORDER BY id LIMIT $3")
            .bind(root).bind(&entry).bind((64-retired.len()) as i64).fetch_all(&mut *db).await?;
        for version in &versions {
            db::query("UPDATE file_versions SET retired=true WHERE root_id=$1 AND id=$2")
                .bind(root)
                .bind(version)
                .execute(&mut *db)
                .await?;
        }
        retired.extend(versions);
        if retired.len() == 64 {
            break;
        }
        let position: serde_json::Value = row.get("position");
        let after = position["after"].as_str().unwrap_or("");
        let children: Vec<String> = db::query_scalar("SELECT id FROM entries WHERE root_id=$1 AND parent_id=$2 AND id>$3 AND NOT purged ORDER BY id LIMIT 128")
            .bind(root).bind(&entry).bind(after).fetch_all(&mut *db).await?;
        for child in &children {
            db::query("INSERT INTO lifecycle_work(root_id,operation_id,entry_id) VALUES($1,$2,$3)")
                .bind(root)
                .bind(operation)
                .bind(child)
                .execute(&mut *db)
                .await?;
        }
        if children.len() == 128 {
            db::query("UPDATE lifecycle_work SET position=$4 WHERE root_id=$1 AND operation_id=$2 AND sequence=$3")
                .bind(root).bind(operation).bind(sequence).bind(serde_json::json!({"after":children.last()})).execute(&mut *db).await?;
        } else {
            db::query(
                "DELETE FROM lifecycle_work WHERE root_id=$1 AND operation_id=$2 AND sequence=$3",
            )
            .bind(root)
            .bind(operation)
            .bind(sequence)
            .execute(&mut *db)
            .await?;
        }
        if !children.is_empty() {
            break;
        }
    }
    let done: bool = db::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM lifecycle_work WHERE root_id=$1 AND operation_id=$2)",
    )
    .bind(root)
    .bind(operation)
    .fetch_one(&mut *db)
    .await?;
    db::query("UPDATE lifecycle_jobs SET processed=processed+$2,state=$3,phase=$4,total=CASE WHEN $5 THEN processed+$2 ELSE NULL END WHERE operation_id=$1")
        .bind(operation).bind(retired.len() as i64).bind(if done{"cleaning_up"}else{"applying"}).bind(if done{"cleanup"}else{"purging"}).bind(done).execute(&mut *db).await?;
    if done {
        db::query("UPDATE operations SET phase='cleanup' WHERE id=$1")
            .bind(operation)
            .execute(&mut *db)
            .await?;
    }
    db::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'purge_retirement_batch',$3)")
        .bind(root).bind(operation).bind(serde_json::json!({"versions":retired})).execute(db).await?;
    Ok(())
}
