//! Verified mirror repair, using existing exact-generation I/O intents and charges.
use crate::{
    content_catalog::{self, lock_content_root},
    error::{Error, Result, require},
    maintenance::{MaintenanceClaim, MaintenanceProgress},
    operations::check_fence,
    payload::{ObjectKey, verified},
    transfer_io::{TransferEngine, mark_copy_deleting},
};
use db::PgRow;
use sct_protocol::*;
use simple_server::database::postgres as db;
use std::collections::BTreeMap;
impl TransferEngine {
    async fn repair_read(
        &self,
        root: &str,
        digest: &str,
        backend: &str,
        generation: &str,
        length: u64,
    ) -> Result<Vec<u8>> {
        let key = ObjectKey {
            root: root.parse()?,
            digest: digest.parse()?,
            generation: generation.parse()?,
        };
        let read = async {
            let _slot = self.slots.acquire().await;
            let store = self.store(backend)?;
            verified(store.read(&key).await?, &key, length).await
        };
        simple_server::time::timeout(std::time::Duration::from_secs(10), read)
            .await
            .unwrap_or(Err(Error(ErrorCode::DestinationUnavailable)))
    }
    async fn repair_copy(
        &self,
        claim: &MaintenanceClaim,
        digest: &str,
        backend: &str,
    ) -> Result<Option<PgRow>> {
        let mut tx = self.writer.transaction().await?;
        check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
        Ok(db::query("SELECT c.*,a.root_id AS allocation_root FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE c.root_id=$1 AND c.digest=$2 AND c.backend_id=$3 AND c.state='available'").bind(&claim.fence.root).bind(digest).bind(backend).fetch_optional(&mut *tx).await?)
    }
    async fn repair_cleanup(&self, claim: &MaintenanceClaim) -> Result<()> {
        self.cleanup_copies(&claim.fence).await?;
        let mut tx = self.writer.transaction().await?;
        check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
        let pending:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM object_copies WHERE operation_id=$1 AND state IN ('writing','deleting'))").bind(&claim.fence.id).fetch_one(&mut *tx).await?;
        require(!pending, ErrorCode::DestinationUnavailable)
    }
    async fn complete_repair(
        &self,
        claim: &MaintenanceClaim,
        action: &PgRow,
        copy: &PgRow,
    ) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &claim.fence.root).await?;
        check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
        let generation: String = copy.get("generation");
        let row=db::query("SELECT c.*,a.root_id AS allocation_root,a.pool_id FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE c.generation=$1 AND c.root_id=$2 AND c.state='available' FOR UPDATE OF c").bind(&generation).bind(&claim.fence.root).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::RevisionConflict))?;
        if row.get::<Option<String>, _>("allocation_root").is_none() {
            require(
                row.get::<String, _>("operation_id") == claim.fence.id
                    && !row.get::<bool, _>("pending_charge"),
                ErrorCode::IntegrityFailure,
            )?;
            let pool: String = row.get("pool_id");
            let staging: String = row.get("allocation_id");
            let bytes: i64 = row.get("bytes");
            db::query("SELECT id FROM pools WHERE id=$1 FOR UPDATE")
                .bind(&pool)
                .fetch_one(&mut *tx)
                .await?;
            let retained: String =
                db::query_scalar("SELECT id FROM allocations WHERE pool_id=$1 AND root_id=$2")
                    .bind(&pool)
                    .bind(&claim.fence.root)
                    .fetch_one(&mut *tx)
                    .await?;
            db::query("SELECT id FROM allocations WHERE id=ANY($1) ORDER BY id FOR UPDATE")
                .bind(vec![&staging, &retained])
                .fetch_all(&mut *tx)
                .await?;
            let changed=db::query("UPDATE allocations SET occupied=occupied+$2 WHERE id=$1 AND capacity-occupied-reserved >= $2").bind(&retained).bind(bytes).execute(&mut *tx).await?.rows_affected();
            require(changed == 1, ErrorCode::QuotaExhausted)?;
            db::query("UPDATE allocations SET occupied=occupied-$2 WHERE id=$1")
                .bind(staging)
                .bind(bytes)
                .execute(&mut *tx)
                .await?;
            db::query("UPDATE object_copies SET allocation_id=$2 WHERE generation=$1")
                .bind(&generation)
                .bind(retained)
                .execute(&mut *tx)
                .await?;
        }
        let digest: String = action.get("digest");
        let backend: String = action.get("backend_id");
        db::query("UPDATE integrity_findings SET repaired=true WHERE root_id=$1 AND backend_id=$2 AND digest=$3 AND kind IN ('missing','corrupt','inaccessible')").bind(&claim.fence.root).bind(&backend).bind(&digest).execute(&mut *tx).await?;
        db::query("UPDATE mirror_repair_work SET state='completed',generation=$3 WHERE job_id=$1 AND id=$2").bind(&claim.job).bind(action.get::<String,_>("id")).bind(&generation).execute(&mut *tx).await?;
        db::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'mirror_repaired',$3)").bind(&claim.fence.root).bind(&claim.fence.id).bind(serde_json::json!({"backend_id":backend,"digest":digest,"generation":generation,"job_id":claim.job})).execute(&mut *tx).await?;
        tx.commit().await?;
        self.writer.release_unused_reservations(&claim.fence).await
    }
    pub(crate) async fn repair_batch(
        &self,
        claim: &MaintenanceClaim,
    ) -> Result<MaintenanceProgress> {
        let after = claim
            .cursor
            .get("after")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let action = {
            let mut tx = self.writer.transaction().await?;
            lock_content_root(&mut tx, &claim.fence.root).await?;
            check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
            let old = db::query(
                "SELECT * FROM mirror_repair_work WHERE job_id=$1 AND id>$2 ORDER BY id LIMIT 1",
            )
            .bind(&claim.job)
            .bind(after)
            .fetch_optional(&mut *tx)
            .await?;
            let row = if old.is_some() {
                old
            } else {
                db::query("INSERT INTO mirror_repair_work(job_id,id,root_id,backend_id,digest) SELECT $1,f.id,f.root_id,f.backend_id,f.digest FROM integrity_findings f WHERE f.root_id=$2 AND f.id>$3 AND NOT f.repaired AND f.backend_id IS NOT NULL AND f.kind IN ('missing','corrupt','inaccessible') ORDER BY f.id LIMIT 1 RETURNING *").bind(&claim.job).bind(&claim.fence.root).bind(after).fetch_optional(&mut *tx).await?
            };
            tx.commit().await?;
            row
        };
        let Some(action) = action else {
            self.writer
                .release_unused_reservations(&claim.fence)
                .await?;
            return Ok(MaintenanceProgress {
                cursor: claim.cursor.clone(),
                processed: 0,
                complete: true,
            });
        };
        let progress = MaintenanceProgress {
            cursor: serde_json::json!({"after":action.get::<String,_>("id")}),
            processed: 1,
            complete: false,
        };
        if action.get::<String, _>("state") == "completed" {
            self.writer
                .release_unused_reservations(&claim.fence)
                .await?;
            return Ok(progress);
        }
        let digest: String = action.get("digest");
        let target: String = action.get("backend_id");
        let (length, pool, candidates) = {
            let mut tx = self.writer.transaction().await?;
            check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
            let object = db::query("SELECT * FROM content_objects WHERE root_id=$1 AND digest=$2")
                .bind(&claim.fence.root)
                .bind(&digest)
                .fetch_one(&mut *tx)
                .await?;
            require(
                crate::integrity::shape(&object)?
                    == content_catalog::children(&mut tx, &claim.fence.root, &digest).await?,
                ErrorCode::IntegrityFailure,
            )?;
            let length = if object.get::<String, _>("kind") == "chunk" {
                object.get::<i64, _>("length") as u64
            } else {
                object
                    .get::<Option<Vec<u8>>, _>("body")
                    .ok_or(Error(ErrorCode::IntegrityFailure))?
                    .len() as u64
            };
            let pool: String = db::query_scalar("SELECT pool_id FROM backends WHERE id=$1")
                .bind(&target)
                .fetch_one(&mut *tx)
                .await?;
            let candidates:Vec<String>=db::query_scalar("SELECT backend_id FROM root_backends WHERE root_id=$1 AND backend_id<>$2 ORDER BY backend_id").bind(&claim.fence.root).bind(&target).fetch_all(&mut *tx).await?;
            (length, pool, candidates)
        };
        require(!candidates.is_empty(), ErrorCode::UnsupportedOperation)?;
        self.reconcile_writes(&claim.fence).await?;
        let current = self.repair_copy(claim, &digest, &target).await?;
        if let Some(copy) = &current
            && copy.get::<i64, _>("bytes") as u64 == length
            && self
                .repair_read(
                    &claim.fence.root,
                    &digest,
                    &target,
                    &copy.get::<String, _>("generation"),
                    length,
                )
                .await
                .is_ok()
        {
            self.repair_cleanup(claim).await?;
            self.complete_repair(claim, &action, copy).await?;
            return Ok(progress);
        }
        let mut source = None;
        for backend in candidates {
            if let Some(copy) = self.repair_copy(claim, &digest, &backend).await?
                && copy.get::<i64, _>("bytes") as u64 == length
                && let Ok(bytes) = self
                    .repair_read(
                        &claim.fence.root,
                        &digest,
                        &backend,
                        &copy.get::<String, _>("generation"),
                        length,
                    )
                    .await
            {
                source = Some((copy.get::<String, _>("generation"), bytes));
                break;
            }
        }
        let (source_generation, bytes) = source.ok_or(Error(ErrorCode::IntegrityFailure))?;
        // A verified alternate exists before any failed incarnation is deleted.
        self.repair_cleanup(claim).await?;
        self.writer
            .release_unused_reservations(&claim.fence)
            .await?;
        self.writer
            .reserve_transfer(
                &claim.fence,
                &BTreeMap::from([(pool.parse()?, length.try_into()?)]),
            )
            .await?;
        if let Some(copy) = current {
            let mut tx = self.writer.transaction().await?;
            lock_content_root(&mut tx, &claim.fence.root).await?;
            check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
            let valid:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM object_copies WHERE generation=$1 AND root_id=$2 AND digest=$3 AND backend_id<>$4 AND state='available')").bind(&source_generation).bind(&claim.fence.root).bind(&digest).bind(&target).fetch_one(&mut *tx).await?;
            require(valid, ErrorCode::RevisionConflict)?;
            mark_copy_deleting(&mut tx, &claim.fence, &copy.get::<String, _>("generation")).await?;
            tx.commit().await?;
        }
        self.write_copy(&claim.fence, &digest.parse()?, &target.parse()?, bytes)
            .await?;
        let replacement = self
            .repair_copy(claim, &digest, &target)
            .await?
            .ok_or(Error(ErrorCode::IntegrityFailure))?;
        self.repair_read(
            &claim.fence.root,
            &digest,
            &target,
            &replacement.get::<String, _>("generation"),
            length,
        )
        .await?;
        self.repair_cleanup(claim).await?;
        self.complete_repair(claim, &action, &replacement).await?;
        Ok(progress)
    }
}
