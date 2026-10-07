use crate::{
    auth::{Actor, authorize},
    error::{Error, Result, id, require},
    lease::Writer,
    roots::{json, lock_root},
};
use sct_protocol::*;
use sqlx::{PgConnection, Row};

/// Private worker fence; never serialized into API responses.
#[derive(Clone)]
pub struct OperationFence {
    pub(crate) id: String,
    pub(crate) root: String,
    pub(crate) epoch: i64,
}
impl OperationFence {
    pub fn id(&self) -> OperationId {
        self.id.parse().expect("generated operation ID")
    }
}
pub(crate) async fn check_fence(
    db: &mut PgConnection,
    fence: &OperationFence,
    server_epoch: i64,
) -> Result<()> {
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM operations WHERE id=$1 AND root_id=$2 AND epoch=$3 AND server_epoch=$4 AND state IN ('active','recovering','cleaning_up') AND worker_expires_at>clock_timestamp() FOR UPDATE)")
      .bind(&fence.id).bind(&fence.root).bind(fence.epoch).bind(server_epoch).fetch_one(db).await?;
    require(valid, ErrorCode::RevisionConflict)
}
impl Writer {
    /// Foundation operation primitive. Production payload lifecycle follows in M2.
    pub async fn begin_operation(
        &self,
        actor: &Actor,
        root: &RootId,
        kind: OperationKind,
    ) -> Result<OperationFence> {
        let mut tx = self.transaction().await?;
        let required = match kind {
            OperationKind::PolicyTransition => Capability::ManagePolicy,
            OperationKind::Purge => Capability::Purge,
            _ => Capability::Write,
        };
        authorize(&mut tx, actor, root.as_str(), required).await?;
        lock_root(&mut tx, root.as_str()).await?;
        let operation = id();
        sqlx::query("INSERT INTO operations(id,root_id,principal_id,kind,state,server_epoch,worker_expires_at) VALUES($1,$2,$3,$4,'active',$5,clock_timestamp()+interval '30 seconds')")
          .bind(&operation).bind(root.as_str()).bind(&actor.principal).bind(json(&kind)?.as_str().unwrap()).bind(self.epoch).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(OperationFence {
            id: operation,
            root: root.as_str().into(),
            epoch: 1,
        })
    }
    /// Recovery claims the existing owner, never attempts to acquire a competing operation.
    pub async fn recover_operation(&self, operation: &OperationId) -> Result<OperationFence> {
        let mut tx = self.transaction().await?;
        let row=sqlx::query("UPDATE operations SET epoch=epoch+1,server_epoch=$2,state='recovering',worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1 AND state IN ('active','recovering','cleaning_up') AND (worker_expires_at IS NULL OR worker_expires_at<=clock_timestamp() OR server_epoch<>$2) RETURNING root_id,epoch")
          .bind(operation.as_str()).bind(self.epoch).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::RootBusy))?;
        tx.commit().await?;
        Ok(OperationFence {
            id: operation.as_str().into(),
            root: row.get("root_id"),
            epoch: row.get("epoch"),
        })
    }
    pub async fn renew_operation(&self, fence: &OperationFence) -> Result<()> {
        let mut tx = self.transaction().await?;
        check_fence(&mut tx, fence, self.epoch).await?;
        sqlx::query("UPDATE operations SET worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1").bind(&fence.id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Metadata-only completion cannot clear uncertain physical work or reservations.
    pub async fn finish_operation(&self, fence: &OperationFence, aborted: bool) -> Result<()> {
        let mut tx = self.transaction().await?;
        check_fence(&mut tx, fence, self.epoch).await?;
        let obligations:bool=sqlx::query_scalar("SELECT uncertain_io OR EXISTS(SELECT 1 FROM reservations WHERE operation_id=$1) OR EXISTS(SELECT 1 FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE c.operation_id=$1 AND c.state<>'deleted' AND (a.root_id IS NULL OR c.state IN ('writing','deleting'))) FROM operations WHERE id=$1").bind(&fence.id).fetch_one(&mut *tx).await?;
        require(!obligations, ErrorCode::RootBusy)?;
        sqlx::query("UPDATE operations SET state=$2,worker_expires_at=NULL WHERE id=$1")
            .bind(&fence.id)
            .bind(if aborted { "aborted" } else { "completed" })
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    /// M1 test-reservation primitive. No physical bytes are created or freed here.
    pub async fn reserve_staging(
        &self,
        fence: &OperationFence,
        pool: &PoolId,
        bytes: ByteCount,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        check_fence(&mut tx, fence, self.epoch).await?;
        let belongs:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM root_backends r JOIN backends b ON b.id=r.backend_id WHERE r.root_id=$1 AND b.pool_id=$2)").bind(&fence.root).bind(pool.as_str()).fetch_one(&mut *tx).await?;
        require(belongs, ErrorCode::InvalidRequest)?;

        let row = sqlx::query(
            "SELECT id FROM allocations WHERE pool_id=$1 AND root_id IS NULL FOR UPDATE",
        )
        .bind(pool.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
        let allocation: String = row.get("id");
        let changed=sqlx::query("UPDATE allocations SET reserved=reserved+$2 WHERE id=$1 AND capacity-reserved-occupied >= $2").bind(&allocation).bind(bytes.get() as i64).execute(&mut *tx).await?.rows_affected();
        require(changed == 1, ErrorCode::StagingExhausted)?;
        sqlx::query("INSERT INTO reservations(id,allocation_id,operation_id,bytes) VALUES($1,$2,$3,$4) ON CONFLICT(allocation_id,operation_id) DO UPDATE SET bytes=reservations.bytes+EXCLUDED.bytes").bind(id()).bind(allocation).bind(&fence.id).bind(bytes.get() as i64).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn release_unused_reservations(&self, fence: &OperationFence) -> Result<()> {
        let mut tx = self.transaction().await?;
        check_fence(&mut tx, fence, self.epoch).await?;
        let rows=sqlx::query("SELECT allocation_id,bytes FROM reservations WHERE operation_id=$1 ORDER BY allocation_id").bind(&fence.id).fetch_all(&mut *tx).await?;
        for row in rows {
            sqlx::query("UPDATE allocations SET reserved=reserved-$2 WHERE id=$1")
                .bind(row.get::<String, _>("allocation_id"))
                .bind(row.get::<i64, _>("bytes"))
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM reservations WHERE operation_id=$1")
            .bind(&fence.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn inspect_operation(
        &self,
        actor: &Actor,
        root: &RootId,
    ) -> Result<OperationInspection> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let row=sqlx::query("SELECT * FROM operations WHERE root_id=$1 AND state IN ('active','recovering','cleaning_up')").bind(root.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        Ok(OperationInspection {
            id: row.get::<String, _>("id").parse()?,
            root_id: root.clone(),
            owner_principal_id: row
                .get::<Option<String>, _>("principal_id")
                .map(|p| p.parse())
                .transpose()?,
            kind: serde_json::from_value(serde_json::json!(row.get::<String, _>("kind")))?,
            state: serde_json::from_value(serde_json::json!(row.get::<String, _>("state")))?,
            upload_id: None,
            completed_units: 0.try_into()?,
            total_units: None,
            retry: RetryGuidance::After {
                delay_ms: 1000.try_into()?,
            },
            failure_code: None,
        })
    }
}
