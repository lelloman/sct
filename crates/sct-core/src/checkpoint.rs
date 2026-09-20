//! Durable checkpoint barrier and exact-generation deletion protection.
use crate::{
    auth::{self, Actor},
    error::{Error, Result, id, require},
    lease::Writer,
    management::audit,
    paging,
    roots::{json, revision},
};
use sct_protocol::*;
use sqlx::{Row, postgres::PgRow};

fn inspect(row: &PgRow) -> Result<CheckpointInspection> {
    Ok(CheckpointInspection {
        id: row.get::<String, _>("id").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        state: serde_json::from_value(json(&row.get::<String, _>("state"))?)?,
        generation_watermark: (row.get::<i64, _>("generation_watermark") as u64).try_into()?,
        pinned_bytes: (row.get::<i64, _>("pinned_bytes") as u64).try_into()?,
        failure: row
            .get::<Option<String>, _>("failure")
            .map(|s| serde_json::from_value(serde_json::Value::String(s)))
            .transpose()?,
        created_at_ms: (row.get::<i64, _>("created_ms") as u64).try_into()?,
    })
}
impl Writer {
    pub async fn create_checkpoint(
        &self,
        actor: &Actor,
        request: &CreateCheckpointRequest,
    ) -> Result<CheckpointInspection> {
        let mut tx = self.control_transaction().await?;
        auth::admin(&mut tx, actor).await?;
        // Serialize checkpoint creation/release without blocking ordinary root work.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext(current_schema()),73628494)")
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT *, (extract(epoch FROM created_at)*1000)::bigint created_ms FROM checkpoints WHERE principal_id=$1 AND key=$2").bind(&actor.principal).bind(request.idempotency_key.as_str()).fetch_optional(&mut *tx).await? { return inspect(&row); }
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM checkpoint_barrier)")
            .fetch_one(&mut *tx)
            .await?;
        require(!busy, ErrorCode::RootBusy)?;
        let checkpoint = id();
        let row=sqlx::query("INSERT INTO checkpoints(id,principal_id,credential_id,key,revision) VALUES($1,$2,$3,$4,$5) RETURNING *, (extract(epoch FROM created_at)*1000)::bigint created_ms").bind(&checkpoint).bind(&actor.principal).bind(&actor.credential).bind(request.idempotency_key.as_str()).bind(id()).fetch_one(&mut *tx).await?;
        sqlx::query("INSERT INTO checkpoint_barrier(checkpoint_id,server_epoch) VALUES($1,$2)")
            .bind(&checkpoint)
            .bind(self.epoch)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            actor,
            None,
            "checkpoint.created",
            serde_json::json!({"checkpoint_id":checkpoint}),
        )
        .await?;
        tx.commit().await?;
        inspect(&row)
    }
    pub async fn checkpoint(
        &self,
        actor: &Actor,
        checkpoint: &OperationId,
    ) -> Result<CheckpointInspection> {
        let mut tx = self.control_transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row=sqlx::query("SELECT *, (extract(epoch FROM created_at)*1000)::bigint created_ms FROM checkpoints WHERE id=$1").bind(checkpoint.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        inspect(&row)
    }
    pub async fn list_checkpoints(
        &self,
        actor: &Actor,
        request: &ListRequest,
    ) -> Result<CheckpointPage> {
        let mut tx = self.control_transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let after: String = match &request.cursor {
            Some(c) => {
                paging::consume(&mut tx, actor, "checkpoints", c, &json(&request.limit)?).await?
            }
            None => String::new(),
        };
        let rows=sqlx::query("SELECT *, (extract(epoch FROM created_at)*1000)::bigint created_ms FROM checkpoints WHERE id>$1 ORDER BY id LIMIT $2").bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let limit = request.limit.get() as usize;
        let next_cursor = if rows.len() > limit {
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    "checkpoints",
                    json(&request.limit)?,
                    json(&rows[limit - 1].get::<String, _>("id"))?,
                )
                .await?,
            )
        } else {
            None
        };
        let checkpoints = rows
            .iter()
            .take(limit)
            .map(inspect)
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(CheckpointPage {
            checkpoints,
            next_cursor,
        })
    }
    pub async fn release_checkpoint(
        &self,
        actor: &Actor,
        checkpoint: &OperationId,
        request: &ReleaseCheckpointRequest,
    ) -> Result<CheckpointInspection> {
        request.validate()?;
        let mut tx = self.control_transaction().await?;
        auth::admin(&mut tx, actor).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext(current_schema()),73628494)")
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT * FROM checkpoints WHERE id=$1 FOR UPDATE")
            .bind(checkpoint.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        revision(&row, &request.expected_revision)?;
        let row=sqlx::query("UPDATE checkpoints SET state='released',revision=$2 WHERE id=$1 RETURNING *, (extract(epoch FROM created_at)*1000)::bigint created_ms").bind(checkpoint.as_str()).bind(id()).fetch_one(&mut *tx).await?;
        sqlx::query("DELETE FROM checkpoint_barrier WHERE checkpoint_id=$1")
            .bind(checkpoint.as_str())
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            actor,
            None,
            "checkpoint.released",
            serde_json::json!({"checkpoint_id":checkpoint}),
        )
        .await?;
        tx.commit().await?;
        inspect(&row)
    }
    /// One bounded pinning batch. The caller drains old delete intents first.
    /// Ordinary transactions share this advisory fence, including publication.
    pub async fn advance_checkpoint(&self) -> Result<()> {
        let mut tx = self.control_transaction().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext(current_schema()),73628494)")
            .execute(&mut *tx)
            .await?;
        let locked: bool = sqlx::query_scalar(
            "SELECT pg_try_advisory_xact_lock(hashtext(current_schema()),73628493)",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !locked {
            return Ok(());
        }
        let Some(barrier)=sqlx::query("SELECT b.*,c.principal_id,c.credential_id,c.state,c.inventory_cursor,c.generation_watermark FROM checkpoint_barrier b JOIN checkpoints c ON c.id=b.checkpoint_id FOR UPDATE OF b,c").fetch_optional(&mut *tx).await? else {return Ok(());};
        let checkpoint: String = barrier.get("checkpoint_id");
        let actor = Actor {
            principal: barrier.get("principal_id"),
            credential: barrier
                .get::<Option<String>, _>("credential_id")
                .unwrap_or_default(),
        };
        if let Err(error) = auth::admin(&mut tx, &actor).await {
            sqlx::query("UPDATE checkpoints SET state='failed',failure=$2,revision=$3 WHERE id=$1")
                .bind(&checkpoint)
                .bind(serde_json::to_value(error.0)?.as_str().unwrap())
                .bind(id())
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM checkpoint_barrier WHERE checkpoint_id=$1")
                .bind(&checkpoint)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(());
        }
        if barrier.get::<String, _>("phase") == "draining" {
            let pending: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM object_copies WHERE state='deleting')",
            )
            .fetch_one(&mut *tx)
            .await?;
            if pending {
                return Ok(());
            }
            sqlx::query("UPDATE checkpoint_barrier SET phase='capturing',server_epoch=$1")
                .bind(self.epoch)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE checkpoints SET generation_watermark=(SELECT COALESCE(max(created_sequence),0) FROM object_copies),revision=$2 WHERE id=$1").bind(&checkpoint).bind(id()).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(());
        }
        if barrier.get::<String, _>("state") != "fencing" {
            return Ok(());
        }
        let rows=sqlx::query("SELECT generation,created_sequence,bytes FROM object_copies WHERE state='available' AND created_sequence>$1 AND created_sequence<=$2 ORDER BY created_sequence LIMIT 256").bind(barrier.get::<i64,_>("inventory_cursor")).bind(barrier.get::<i64,_>("generation_watermark")).fetch_all(&mut *tx).await?;
        let mut bytes = 0i64;
        for row in &rows {
            sqlx::query("INSERT INTO checkpoint_objects(checkpoint_id,generation) VALUES($1,$2)")
                .bind(&checkpoint)
                .bind(row.get::<String, _>("generation"))
                .execute(&mut *tx)
                .await?;
            bytes += row.get::<i64, _>("bytes");
        }
        let cursor = rows
            .last()
            .map_or(barrier.get::<i64, _>("inventory_cursor"), |r| {
                r.get("created_sequence")
            });
        sqlx::query("UPDATE checkpoints SET inventory_cursor=$2,pinned_bytes=pinned_bytes+$3,revision=$4,state=$5 WHERE id=$1").bind(&checkpoint).bind(cursor).bind(bytes).bind(id()).bind(if rows.len()<256 {"exporting"}else{"fencing"}).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
/// Both queued cleanup and new deletion decisions must consult durable pins.
pub(crate) async fn protected(db: &mut sqlx::PgConnection, generation: &str) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM checkpoint_objects o JOIN checkpoints c ON c.id=o.checkpoint_id WHERE o.generation=$1 AND c.state<>'released')").bind(generation).fetch_one(db).await?)
}

impl crate::transfer_io::TransferEngine {
    /// Drain only already durable deletion intents. Exact generations are never
    /// reused, so retrying an old delete cannot remove a replacement generation.
    /// The state transition releases charges at most once even if an older worker
    /// also receives its delete acknowledgement during this pass.
    pub(crate) async fn checkpoint_tick(&self) -> Result<bool> {
        let phase: Option<String> = {
            let mut tx = self.writer.control_transaction().await?;
            sqlx::query_scalar("SELECT phase FROM checkpoint_barrier")
                .fetch_optional(&mut *tx)
                .await?
        };
        let Some(phase) = phase else {
            return Ok(false);
        };
        if phase == "draining" {
            let rows = {
                let mut tx = self.writer.transaction().await?;
                sqlx::query("SELECT root_id,generation,digest,backend_id FROM object_copies WHERE state='deleting' ORDER BY created_sequence LIMIT 8").fetch_all(&mut *tx).await?
            };
            for row in rows {
                let generation: String = row.get("generation");
                let root: String = row.get("root_id");
                let key = crate::payload::ObjectKey {
                    root: root.parse()?,
                    digest: row.get::<String, _>("digest").parse()?,
                    generation: generation.parse()?,
                };
                let store = self.store(&row.get::<String, _>("backend_id"))?;
                let result =
                    tokio::time::timeout(std::time::Duration::from_secs(2), store.delete(&key))
                        .await;
                if !matches!(result, Ok(Ok(crate::payload::DeleteOutcome::Confirmed))) {
                    continue;
                }
                let mut tx = self.writer.transaction().await?;
                crate::content_catalog::lock_content_root(&mut tx, &root).await?;
                let changed=sqlx::query("UPDATE object_copies SET state='deleted',uncertain=false,pending_charge=false WHERE generation=$1 AND state='deleting' RETURNING allocation_id,operation_id,bytes").bind(&generation).fetch_optional(&mut *tx).await?;
                if let Some(copy) = changed {
                    let allocation: String = copy.get("allocation_id");
                    let operation: String = copy.get("operation_id");
                    let bytes: i64 = copy.get("bytes");
                    let refund:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transfer_sessions s JOIN allocations a ON a.id=$2 WHERE s.operation_id=$1 AND s.state IN ('open','ready') AND s.admission='admitted' AND a.root_id IS NULL)").bind(&operation).bind(&allocation).fetch_one(&mut *tx).await?;
                    sqlx::query("UPDATE allocations SET occupied=occupied-$2,pending_cleanup=pending_cleanup-$2,reserved=reserved+$3 WHERE id=$1").bind(&allocation).bind(bytes).bind(if refund{bytes}else{0}).execute(&mut *tx).await?;
                    if refund {
                        sqlx::query("UPDATE reservations SET bytes=bytes+$3 WHERE allocation_id=$1 AND operation_id=$2").bind(&allocation).bind(&operation).bind(bytes).execute(&mut *tx).await?;
                    }
                    sqlx::query("UPDATE operations SET uncertain_io=EXISTS(SELECT 1 FROM object_copies WHERE operation_id=$1 AND (uncertain OR state IN ('writing','deleting'))) WHERE id=$1").bind(operation).execute(&mut *tx).await?;
                }
                tx.commit().await?;
            }
        }
        self.writer.advance_checkpoint().await?;
        Ok(true)
    }
}
