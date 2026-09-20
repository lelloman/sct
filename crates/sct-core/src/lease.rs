//! Short, fenced transactions. Root ownership survives worker/server lease expiry.
use crate::{
    catalog::Catalog,
    error::{Error, Result, id, require},
};
use sct_protocol::ErrorCode;
use sqlx::{Postgres, Transaction};

#[derive(Clone)]
pub struct Writer {
    pub(crate) catalog: Catalog,
    owner: String,
    pub(crate) epoch: i64,
}
impl Catalog {
    pub async fn acquire_writer(&self) -> Result<Writer> {
        let owner = id();
        let epoch: Option<i64> = sqlx::query_scalar(
            "INSERT INTO server_lease(singleton,owner,epoch,expires_at) VALUES(true,$1,1,clock_timestamp()+interval '30 seconds') ON CONFLICT(singleton) DO UPDATE SET owner=$1,epoch=server_lease.epoch+1,expires_at=clock_timestamp()+interval '30 seconds' WHERE server_lease.expires_at <= clock_timestamp() RETURNING epoch"
        ).bind(&owner).fetch_optional(&self.pool).await?;
        let writer = Writer {
            catalog: self.clone(),
            owner,
            epoch: epoch.ok_or(Error(ErrorCode::RootBusy))?,
        };
        let mut tx = writer.control_transaction().await?;
        // An exported PostgreSQL snapshot cannot survive its owning process.
        // Preserve every durable pin and make interruption explicit.
        sqlx::query("UPDATE checkpoints SET state='failed',failure='destination_unavailable',revision=$1 WHERE id IN (SELECT checkpoint_id FROM checkpoint_barrier WHERE phase='capturing' AND server_epoch<>$2)").bind(id()).bind(writer.epoch).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM checkpoint_barrier WHERE phase='capturing' AND server_epoch<>$1")
            .bind(writer.epoch)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE operations SET state='recovering',epoch=epoch+1,worker_expires_at=NULL WHERE state IN ('active','cleaning_up') AND server_epoch<>$1")
          .bind(writer.epoch).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(writer)
    }
}
impl Writer {
    /// The shared lease-row lock prevents takeover while this short metadata
    /// transaction commits. No backend/network I/O may occur inside it.
    pub(crate) async fn transaction(&self) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.control_transaction().await?;
        sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtext(current_schema()),73628493)")
            .execute(&mut *tx)
            .await?;
        let frozen: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM checkpoint_barrier WHERE phase='capturing')",
        )
        .fetch_one(&mut *tx)
        .await?;
        require(!frozen, ErrorCode::RootBusy)?;
        Ok(tx)
    }
    /// Checkpoint control, authentication and lease renewal only; callers must
    /// not mutate ordinary catalog content.
    pub(crate) async fn control_transaction(&self) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.catalog.pool.begin().await?;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM server_lease WHERE owner=$1 AND epoch=$2 AND expires_at>clock_timestamp() FOR SHARE)")
          .bind(&self.owner).bind(self.epoch).fetch_one(&mut *tx).await?;
        require(valid, ErrorCode::DestinationUnavailable)?;
        Ok(tx)
    }
    pub async fn renew(&self) -> Result<()> {
        let count = sqlx::query("UPDATE server_lease SET expires_at=clock_timestamp()+interval '30 seconds' WHERE owner=$1 AND epoch=$2 AND expires_at>clock_timestamp()")
          .bind(&self.owner).bind(self.epoch).execute(&self.catalog.pool).await?.rows_affected();
        require(count == 1, ErrorCode::DestinationUnavailable)
    }
    pub async fn ready(&self) -> bool {
        self.transaction().await.is_ok()
    }
    pub async fn release(&self) -> Result<()> {
        sqlx::query(
            "UPDATE server_lease SET expires_at=clock_timestamp() WHERE owner=$1 AND epoch=$2",
        )
        .bind(&self.owner)
        .bind(self.epoch)
        .execute(&self.catalog.pool)
        .await?;
        Ok(())
    }
}
impl Writer {
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
}
