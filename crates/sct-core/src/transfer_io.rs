//! Fenced durable physical-copy transitions. HTTP/session policy lives above this
//! layer; all I/O is outside SQL transactions and bounded by an object at a time.
use crate::{
    content_catalog::lock_content_root,
    error::{Error, Result, id, require},
    lease::Writer,
    operations::{OperationFence, check_fence},
    payload::*,
};
use sct_protocol::*;
use sqlx::Row;
use std::sync::Arc;

#[derive(Clone)]
pub struct TransferEngine {
    pub(crate) writer: Writer,
    pub(crate) stores: Arc<Stores>,
    pub(crate) ingress: Arc<tokio::sync::Semaphore>,
    pub(crate) slots: Arc<tokio::sync::Semaphore>,
    pub(crate) expiry_seconds: u32,
}
impl TransferEngine {
    pub fn new(writer: Writer, stores: Stores, workers: u16, expiry_seconds: u32) -> Result<Self> {
        require(
            workers > 0 && (60..=2592000).contains(&expiry_seconds),
            ErrorCode::InvalidRequest,
        )?;
        Ok(Self {
            writer,
            stores: Arc::new(stores),
            ingress: Arc::new(tokio::sync::Semaphore::new(workers as usize)),
            slots: Arc::new(tokio::sync::Semaphore::new(workers as usize)),
            expiry_seconds,
        })
    }
    pub fn writer(&self) -> &Writer {
        &self.writer
    }
    pub fn session_expiry_seconds(&self) -> u32 {
        self.expiry_seconds
    }
    pub(crate) fn store(&self, backend: &str) -> Result<Arc<dyn ObjectStore>> {
        self.stores
            .get(backend)
            .cloned()
            .ok_or(Error(ErrorCode::DestinationUnavailable))
    }
    /// Persist intent/charge before issuing I/O; replays reuse an available copy.
    pub async fn write_copy(
        &self,
        fence: &OperationFence,
        digest: &ChunkDigest,
        backend: &BackendId,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let engine = self.clone();
        let fence = fence.clone();
        let digest = digest.clone();
        let backend = backend.clone();
        tokio::spawn(async move {
            engine
                .write_copy_inner(&fence, &digest, &backend, bytes)
                .await
        })
        .await
        .map_err(|_| Error(ErrorCode::DestinationUnavailable))?
    }
    async fn write_copy_inner(
        &self,
        fence: &OperationFence,
        digest: &ChunkDigest,
        backend: &BackendId,
        bytes: Vec<u8>,
    ) -> Result<()> {
        require(
            sct_protocol::digest(&bytes) == digest.as_str()
                && !bytes.is_empty()
                && bytes.len() <= CHUNK_MAX as usize,
            ErrorCode::IntegrityFailure,
        )?;
        let _slot = self
            .slots
            .acquire()
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
        let store = self.store(backend.as_str())?;
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &fence.root).await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        let existing=sqlx::query("SELECT generation,state,bytes FROM object_copies WHERE root_id=$1 AND digest=$2 AND backend_id=$3 AND state IN ('writing','available') FOR UPDATE").bind(&fence.root).bind(digest.as_str()).bind(backend.as_str()).fetch_optional(&mut *tx).await?;
        let generation = if let Some(r) = existing {
            require(
                r.get::<i64, _>("bytes") == bytes.len() as i64,
                ErrorCode::IntegrityFailure,
            )?;
            if r.get::<String, _>("state") == "available" {
                return Ok(());
            }
            // A writing incarnation needs explicit reconciliation before retry.
            return Err(Error(ErrorCode::RootBusy));
        } else {
            let r=sqlx::query("SELECT a.id FROM allocations a JOIN backends b ON b.pool_id=a.pool_id JOIN root_backends rb ON rb.backend_id=b.id WHERE a.root_id IS NULL AND b.id=$1 AND rb.root_id=$2 FOR UPDATE OF a").bind(backend.as_str()).bind(&fence.root).fetch_one(&mut *tx).await?;
            let allocation: String = r.get("id");
            let size = bytes.len() as i64;
            let n=sqlx::query("UPDATE reservations SET bytes=bytes-$3 WHERE allocation_id=$1 AND operation_id=$2 AND bytes>=$3").bind(&allocation).bind(&fence.id).bind(size).execute(&mut *tx).await?.rows_affected();
            require(n == 1, ErrorCode::StagingExhausted)?;
            sqlx::query(
                "UPDATE allocations SET reserved=reserved-$2,occupied=occupied+$2 WHERE id=$1",
            )
            .bind(&allocation)
            .bind(size)
            .execute(&mut *tx)
            .await?;
            let generation = id();
            sqlx::query("INSERT INTO object_copies(generation,root_id,digest,backend_id,allocation_id,operation_id,bytes,state,worker_epoch,server_epoch) VALUES($1,$2,$3,$4,$5,$6,$7,'writing',$8,$9)").bind(&generation).bind(&fence.root).bind(digest.as_str()).bind(backend.as_str()).bind(allocation).bind(&fence.id).bind(size).bind(fence.epoch).bind(self.writer.epoch).execute(&mut *tx).await?;
            sqlx::query("UPDATE operations SET uncertain_io=true WHERE id=$1")
                .bind(&fence.id)
                .execute(&mut *tx)
                .await?;
            generation
        };
        tx.commit().await?;
        let key = ObjectKey {
            root: fence.root.parse()?,
            digest: digest.clone(),
            generation: generation.parse()?,
        };
        if let Err(error) = store.put(&key, bytes.len() as u64, reader(bytes)).await {
            let mut tx = self.writer.transaction().await?;
            check_fence(&mut tx, fence, self.writer.epoch).await?;
            sqlx::query(
                "UPDATE object_copies SET worker_epoch=0 WHERE generation=$1 AND state='writing'",
            )
            .bind(&generation)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Err(error);
        }
        self.acknowledge(fence, &generation).await
    }
    async fn acknowledge(&self, fence: &OperationFence, generation: &str) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &fence.root).await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        let n=sqlx::query("UPDATE object_copies SET state='available',uncertain=false WHERE generation=$1 AND operation_id=$2 AND state='writing' AND worker_epoch=$3 AND server_epoch=$4").bind(generation).bind(&fence.id).bind(fence.epoch).bind(self.writer.epoch).execute(&mut *tx).await?.rows_affected();
        require(n == 1, ErrorCode::RevisionConflict)?;
        refresh_uncertain(&mut tx, &fence.id).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Recover one persisted write. Positive verified data is safe to acknowledge.
    /// Missing/corrupt data remains charged until a generation-fenced deletion.
    pub async fn reconcile_writes(&self, fence: &OperationFence) -> Result<usize> {
        let rows = {
            let mut tx = self.writer.transaction().await?;
            check_fence(&mut tx, fence, self.writer.epoch).await?;
            sqlx::query("SELECT generation,digest,backend_id,bytes FROM object_copies WHERE operation_id=$1 AND state='writing' AND (worker_epoch<>$2 OR server_epoch<>$3) ORDER BY generation LIMIT 64").bind(&fence.id).bind(fence.epoch).bind(self.writer.epoch).fetch_all(&mut *tx).await?
        };
        let count = rows.len();
        for r in rows {
            let generation: String = r.get("generation");
            let key = ObjectKey {
                root: fence.root.parse()?,
                digest: r.get::<String, _>("digest").parse()?,
                generation: generation.parse()?,
            };
            let store = self.store(&r.get::<String, _>("backend_id"))?;
            let verified = match store.read(&key).await {
                Ok(reader) => {
                    crate::payload::verified(reader, &key, r.get::<i64, _>("bytes") as u64)
                        .await
                        .is_ok()
                }
                Err(_) => false,
            };
            if verified {
                let mut tx = self.writer.transaction().await?;
                check_fence(&mut tx, fence, self.writer.epoch).await?;
                sqlx::query("UPDATE object_copies SET worker_epoch=$2,server_epoch=$3 WHERE generation=$1 AND state='writing'").bind(&generation).bind(fence.epoch).bind(self.writer.epoch).execute(&mut *tx).await?;
                tx.commit().await?;
                self.acknowledge(fence, &generation).await?;
            } else {
                self.mark_deleting(fence, &generation).await?;
            }
        }
        Ok(count)
    }
    /// Caller must have proved this copy unreachable under the root lock. This
    /// primitive is crate-private: no API may delete arbitrary available content.
    pub(crate) async fn mark_deleting(
        &self,
        fence: &OperationFence,
        generation: &str,
    ) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &fence.root).await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        let r=sqlx::query("UPDATE object_copies SET state='deleting',operation_id=$3 WHERE generation=$1 AND root_id=$2 AND state IN ('writing','available') RETURNING allocation_id,bytes").bind(generation).bind(&fence.root).bind(&fence.id).fetch_optional(&mut *tx).await?;
        if let Some(r) = r {
            sqlx::query("UPDATE allocations SET pending_cleanup=pending_cleanup+$2 WHERE id=$1")
                .bind(r.get::<String, _>("allocation_id"))
                .bind(r.get::<i64, _>("bytes"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn cleanup_copies(&self, fence: &OperationFence) -> Result<usize> {
        let rows = {
            let mut tx = self.writer.transaction().await?;
            check_fence(&mut tx, fence, self.writer.epoch).await?;
            sqlx::query("SELECT generation,digest,backend_id FROM object_copies WHERE operation_id=$1 AND state='deleting' ORDER BY generation LIMIT 64").bind(&fence.id).fetch_all(&mut *tx).await?
        };
        let mut completed = 0;
        for row in rows {
            let generation: String = row.get("generation");
            let key = ObjectKey {
                root: fence.root.parse()?,
                digest: row.get::<String, _>("digest").parse()?,
                generation: generation.parse()?,
            };
            if self
                .store(&row.get::<String, _>("backend_id"))?
                .delete(&key)
                .await?
                != DeleteOutcome::Confirmed
            {
                continue;
            }
            let mut tx = self.writer.transaction().await?;
            lock_content_root(&mut tx, &fence.root).await?;
            check_fence(&mut tx, fence, self.writer.epoch).await?;
            let r=sqlx::query("UPDATE object_copies SET state='deleted',uncertain=false WHERE generation=$1 AND state='deleting' RETURNING allocation_id,bytes").bind(&generation).fetch_optional(&mut *tx).await?;
            if let Some(r) = r {
                let allocation: String = r.get("allocation_id");
                let bytes: i64 = r.get("bytes");
                let refund:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transfer_sessions s JOIN allocations a ON a.id=$2 WHERE s.operation_id=$1 AND s.state IN ('open','ready') AND s.admission='admitted' AND a.root_id IS NULL)").bind(&fence.id).bind(&allocation).fetch_one(&mut *tx).await?;
                sqlx::query("UPDATE allocations SET occupied=occupied-$2,pending_cleanup=pending_cleanup-$2,reserved=reserved+$3 WHERE id=$1").bind(&allocation).bind(bytes).bind(if refund{bytes}else{0}).execute(&mut *tx).await?;
                if refund {
                    sqlx::query("UPDATE reservations SET bytes=bytes+$3 WHERE allocation_id=$1 AND operation_id=$2").bind(allocation).bind(&fence.id).bind(bytes).execute(&mut *tx).await?;
                }
                completed += 1;
            }
            refresh_uncertain(&mut tx, &fence.id).await?;
            tx.commit().await?;
        }
        Ok(completed)
    }
}
async fn refresh_uncertain(db: &mut sqlx::PgConnection, operation: &str) -> Result<()> {
    sqlx::query("UPDATE operations SET uncertain_io=EXISTS(SELECT 1 FROM object_copies WHERE operation_id=$1 AND (uncertain OR state IN ('writing','deleting'))) WHERE id=$1").bind(operation).execute(db).await?;
    Ok(())
}

impl TransferEngine {
    /// Mark a bounded batch unreachable from live versions, read leases or active
    /// upload roots. Graph traversal stays in PostgreSQL; only 64 keys leave it.
    pub(crate) async fn collect_unreferenced(&self, fence: &OperationFence) -> Result<usize> {
        let rows = {
            let mut tx = self.writer.transaction().await?;
            lock_content_root(&mut tx, &fence.root).await?;
            check_fence(&mut tx, fence, self.writer.epoch).await?;
            sqlx::query("WITH RECURSIVE seeds(digest) AS (SELECT manifest FROM file_versions WHERE root_id=$1 AND NOT retired UNION SELECT v.manifest FROM read_pins p JOIN file_versions v ON v.id=p.version_id WHERE p.root_id=$1 AND p.expires_at>clock_timestamp() UNION SELECT manifest FROM transfer_sessions WHERE root_id=$1 AND state IN ('open','ready') AND admission<>'rejected'), live(digest) AS (SELECT digest FROM seeds UNION SELECT e.child FROM live l JOIN content_edges e ON e.parent=l.digest AND e.root_id=$1) SELECT generation FROM object_copies c WHERE c.root_id=$1 AND c.state='available' AND NOT EXISTS(SELECT 1 FROM live WHERE live.digest=c.digest) ORDER BY generation LIMIT 64").bind(&fence.root).fetch_all(&mut *tx).await?
        };
        for r in &rows {
            self.mark_deleting(fence, &r.get::<String, _>("generation"))
                .await?;
        }
        Ok(rows.len())
    }
    /// One bounded recovery pass. The runtime repeats this without extending user
    /// inactivity. An expired worker is reclaimed; an expired session is cleaned.
    pub async fn tick(&self) -> Result<()> {
        let rows = {
            let mut tx = self.writer.transaction().await?;
            sqlx::query("SELECT o.id,o.root_id,o.epoch,o.server_epoch,(o.worker_expires_at IS NULL OR o.worker_expires_at<=clock_timestamp()) expired FROM operations o WHERE o.state IN ('active','recovering','cleaning_up') AND (EXISTS(SELECT 1 FROM transfer_sessions s WHERE s.operation_id=o.id) OR EXISTS(SELECT 1 FROM object_copies c WHERE c.operation_id=o.id AND c.state IN ('writing','deleting'))) ORDER BY o.worker_expires_at NULLS FIRST,o.id LIMIT 64").fetch_all(&mut *tx).await?
        };
        for r in rows {
            let operation: OperationId = r.get::<String, _>("id").parse()?;
            let fence = if r.get::<bool, _>("expired")
                || r.get::<i64, _>("server_epoch") != self.writer.epoch
            {
                match self.writer.recover_operation(&operation).await {
                    Ok(f) => f,
                    Err(_) => continue,
                }
            } else {
                OperationFence {
                    id: operation.as_str().into(),
                    root: r.get("root_id"),
                    epoch: r.get("epoch"),
                }
            };
            self.writer.renew_operation(&fence).await?;
            let terminal = {
                let mut tx = self.writer.transaction().await?;
                lock_content_root(&mut tx, &fence.root).await?;
                check_fence(&mut tx, &fence, self.writer.epoch).await?;
                sqlx::query("UPDATE transfer_sessions SET state='expired' WHERE operation_id=$1 AND state IN ('open','ready') AND expires_at<=clock_timestamp()").bind(&fence.id).execute(&mut *tx).await?;
                let terminal:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transfer_sessions WHERE operation_id=$1 AND (state IN ('committed','aborted','expired') OR admission='rejected'))").bind(&fence.id).fetch_one(&mut *tx).await?;
                tx.commit().await?;
                terminal
            };
            // Failed provider I/O leaves durable charge for the next pass and does
            // not starve unrelated roots in this pass.
            if self.reconcile_writes(&fence).await.is_err() {
                continue;
            }
            if self.collect_unreferenced(&fence).await.is_err() {
                continue;
            }
            if self.cleanup_copies(&fence).await.is_err() {
                continue;
            }
            if terminal {
                self.writer.release_unused_reservations(&fence).await?;
                let _ = self.writer.finish_operation(&fence, false).await;
            }
        }
        Ok(())
    }
}
impl TransferEngine {
    pub async fn heartbeat(&self) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        sqlx::query("UPDATE operations SET worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE server_epoch=$1 AND worker_expires_at>clock_timestamp() AND state IN ('active','recovering','cleaning_up') AND kind='upload'").bind(self.writer.epoch).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
