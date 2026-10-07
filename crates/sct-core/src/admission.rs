use crate::{
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Error, Result, require},
    operations::check_fence,
    policy::effective,
    roots::json,
    transfer_io::TransferEngine,
    upload::Session,
};
use sct_protocol::*;
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;

/// PostgreSQL performs set-union graph accounting; the application receives only
/// aggregate pool rows and bounded candidate batches, not a flattened file graph.
pub(crate) fn graphs() -> String {
    "WITH RECURSIVE projected(digest) AS (SELECT v.manifest FROM file_versions v WHERE v.root_id=$1 AND NOT v.retired AND NOT EXISTS(SELECT 1 FROM transfer_retire r WHERE r.session_id=$2 AND r.version_id=v.id) UNION SELECT manifest FROM transfer_sessions WHERE id=$2 UNION SELECT manifest FROM graph_pins WHERE root_id=$1 AND (expires_at IS NULL OR expires_at>clock_timestamp()) UNION SELECT e.child FROM projected p JOIN content_edges e ON e.root_id=$1 AND e.parent=p.digest), prelive(digest) AS (SELECT v.manifest FROM file_versions v WHERE v.root_id=$1 AND NOT v.retired AND NOT EXISTS(SELECT 1 FROM transfer_retire r WHERE r.session_id=$2 AND r.version_id=v.id AND r.reason='admission') UNION SELECT manifest FROM transfer_sessions WHERE id=$2 UNION SELECT v.manifest FROM read_pins p JOIN file_versions v ON v.id=p.version_id WHERE p.root_id=$1 AND p.expires_at>clock_timestamp() UNION SELECT manifest FROM graph_pins WHERE root_id=$1 AND (expires_at IS NULL OR expires_at>clock_timestamp()) UNION SELECT e.child FROM prelive p JOIN content_edges e ON e.root_id=$1 AND e.parent=p.digest)".into()
}
#[derive(Debug)]
pub(crate) struct PoolDemand {
    pub pool: String,
    pub incoming: u64,
    pub outgoing: u64,
    pub reclaimable: u64,
    pub reserved: u64,
    pub staging_free: u64,
    pub retained_used: u64,
    pub retained_capacity: u64,
}
pub(crate) async fn inventory(
    db: &mut PgConnection,
    s: &Session,
) -> Result<(u64, Vec<PoolDemand>)> {
    let quota_sql = format!(
        "{} SELECT COALESCE(sum(o.length),0)::bigint FROM content_objects o JOIN projected p ON p.digest=o.digest WHERE o.root_id=$1 AND o.kind='chunk'",
        graphs()
    );
    let unique: i64 = sqlx::query_scalar(&quota_sql)
        .bind(&s.root)
        .bind(&s.id)
        .fetch_one(&mut *db)
        .await?;
    let sql = format!(
        "{}, incoming AS (SELECT b.pool_id,COALESCE(sum(CASE WHEN o.kind='chunk' THEN o.length ELSE octet_length(o.body) END),0)::bigint bytes FROM root_backends rb JOIN backends b ON b.id=rb.backend_id JOIN session_objects p ON p.root_id=rb.root_id AND p.session_id=$2 JOIN content_objects o ON o.root_id=p.root_id AND o.digest=p.digest WHERE rb.root_id=$1 AND NOT EXISTS(SELECT 1 FROM object_copies c WHERE c.root_id=o.root_id AND c.digest=o.digest AND c.backend_id=b.id AND c.state='available') GROUP BY b.pool_id), outgoing AS (SELECT b.pool_id,COALESCE(sum(c.bytes) FILTER(WHERE EXISTS(SELECT 1 FROM prelive p WHERE p.digest=c.digest) AND NOT EXISTS(SELECT 1 FROM projected p WHERE p.digest=c.digest)),0)::bigint bytes,COALESCE(sum(c.bytes) FILTER(WHERE NOT EXISTS(SELECT 1 FROM prelive p WHERE p.digest=c.digest)),0)::bigint reclaimable FROM object_copies c JOIN backends b ON b.id=c.backend_id JOIN allocations a ON a.id=c.allocation_id WHERE c.root_id=$1 AND a.root_id=$1 AND c.state='available' GROUP BY b.pool_id) SELECT a.pool_id,a.occupied retained_used,a.capacity retained_capacity,s.capacity-s.occupied-s.reserved staging_free,COALESCE(i.bytes,0) incoming,COALESCE(o.bytes,0) outgoing,COALESCE(o.reclaimable,0) reclaimable,(SELECT COALESCE(sum(x.reserved),0)::bigint FROM allocations x WHERE x.pool_id=a.pool_id) reserved FROM allocations a JOIN allocations s ON s.pool_id=a.pool_id AND s.root_id IS NULL LEFT JOIN incoming i ON i.pool_id=a.pool_id LEFT JOIN outgoing o ON o.pool_id=a.pool_id WHERE a.root_id=$1 ORDER BY a.pool_id",
        graphs()
    );
    let rows = sqlx::query(&sql)
        .bind(&s.root)
        .bind(&s.id)
        .fetch_all(db)
        .await?;
    Ok((
        unique as u64,
        rows.iter()
            .map(|r| PoolDemand {
                pool: r.get("pool_id"),
                incoming: r.get::<i64, _>("incoming") as u64,
                outgoing: r.get::<i64, _>("outgoing") as u64,
                reclaimable: r.get::<i64, _>("reclaimable") as u64,
                reserved: r.get::<i64, _>("reserved") as u64,
                staging_free: r.get::<i64, _>("staging_free") as u64,
                retained_used: r.get::<i64, _>("retained_used") as u64,
                retained_capacity: r.get::<i64, _>("retained_capacity") as u64,
            })
            .collect(),
    ))
}
fn fits(
    unique: u64,
    quota: u64,
    pools: &[PoolDemand],
    available: &BTreeMap<String, u64>,
) -> Result<()> {
    require(unique <= quota, ErrorCode::QuotaExhausted)?;
    for p in pools {
        require(
            p.incoming.max(p.outgoing) <= p.staging_free,
            ErrorCode::StagingExhausted,
        )?;
        require(
            p.retained_used
                .saturating_sub(p.reclaimable)
                .saturating_add(p.incoming)
                .saturating_sub(p.outgoing)
                <= p.retained_capacity,
            ErrorCode::DestinationCapacity,
        )?;
        if let Some(free) = available.get(&p.pool) {
            require(
                p.incoming
                    <= free
                        .saturating_sub(p.reserved)
                        .saturating_add(p.reclaimable),
                ErrorCode::DestinationCapacity,
            )?;
        }
    }
    Ok(())
}
impl TransferEngine {
    async fn probe_pools(&self, root: &str) -> Result<BTreeMap<String, u64>> {
        let rows = {
            let mut tx = self.writer.transaction().await?;
            sqlx::query("SELECT b.id,b.pool_id FROM root_backends r JOIN backends b ON b.id=r.backend_id WHERE r.root_id=$1 ORDER BY b.id").bind(root).fetch_all(&mut *tx).await?
        };
        let mut free = BTreeMap::new();
        for r in rows {
            if let Some(n) = self.store(&r.get::<String, _>("id"))?.available().await? {
                let pool: String = r.get("pool_id");
                free.entry(pool)
                    .and_modify(|v: &mut u64| *v = (*v).min(n))
                    .or_insert(n);
            }
        }
        Ok(free)
    }
    pub async fn admit_upload(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus> {
        let result = self.admit_inner(actor, root, upload).await;
        if let Err(Error(code)) = &result
            && matches!(
                code,
                ErrorCode::QuotaExhausted
                    | ErrorCode::StagingExhausted
                    | ErrorCode::DestinationCapacity
                    | ErrorCode::InvalidManifest
            )
        {
            let mut tx = self.writer.transaction().await?;
            authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
            lock_content_root(&mut tx, root.as_str()).await?;
            authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
            sqlx::query("UPDATE transfer_sessions SET admission='rejected',failure=$4 WHERE id=$1 AND root_id=$2 AND principal_id=$3 AND admission<>'admitted'").bind(upload.as_str()).bind(root.as_str()).bind(&actor.principal).bind(json(code)?.as_str().unwrap()).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        result
    }
    async fn admit_inner(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus> {
        let s = self.session(actor, root, upload).await?;
        if s.admission == AdmissionState::Admitted {
            return self.upload_status(actor, root, upload).await;
        }
        require(s.state == SessionState::Open, ErrorCode::UploadExpired)?;
        require(
            s.admission != AdmissionState::Rejected,
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        let policy = s.check_target(&mut tx, actor).await?;
        sqlx::query("UPDATE transfer_sessions SET admission='pending' WHERE id=$1")
            .bind(&s.id)
            .execute(&mut *tx)
            .await?;
        if s.admission == AdmissionState::Planning {
            if let Some(entry_id) = s.replaced_entry() {
                sqlx::query("INSERT INTO transfer_retire(root_id,session_id,version_id,reason) SELECT root_id,$2,id,'rotation' FROM file_versions WHERE root_id=$1 AND entry_id=$3 AND NOT retired ORDER BY sequence DESC OFFSET $4 ON CONFLICT DO NOTHING").bind(&s.root).bind(&s.id).bind(entry_id.as_str()).bind((policy.policy.keep_versions.get()-1) as i64).execute(&mut *tx).await?;
            }
            sqlx::query("UPDATE operations SET phase='admission' WHERE id=$1")
                .bind(&s.fence.id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        if !self.expand_plan(&s).await? {
            return self.upload_status(actor, root, upload).await;
        }
        if s.phase == "reclaiming" {
            return self.finish_reclamation(actor, root, upload, &s).await;
        }
        let available = self.probe_pools(&s.root).await?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(&mut tx, &s.root).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        let quota: i64 = sqlx::query_scalar("SELECT quota FROM roots WHERE id=$1")
            .bind(&s.root)
            .fetch_one(&mut *tx)
            .await?;
        let (mut unique, mut pools) = inventory(&mut tx, &s).await?;
        // History deletion cannot enlarge the dedicated staging allocation.
        for p in &pools {
            require(p.incoming <= p.staging_free, ErrorCode::StagingExhausted)?;
        }
        let mut fit = fits(unique, quota as u64, &pools, &available);
        if fit.is_err() {
            let cursor: Option<String> =
                sqlx::query_scalar("SELECT cursor FROM transfer_sessions WHERE id=$1")
                    .bind(&s.id)
                    .fetch_one(&mut *tx)
                    .await?;
            let after = cursor
                .as_deref()
                .unwrap_or("0")
                .parse::<i64>()
                .map_err(|_| Error(ErrorCode::InternalError))?;
            let candidates=sqlx::query("SELECT v.id,v.entry_id,v.sequence FROM file_versions v WHERE v.root_id=$1 AND NOT v.retired AND v.sequence>$2 AND EXISTS(SELECT 1 FROM file_versions newer WHERE newer.entry_id=v.entry_id AND NOT newer.retired AND newer.sequence>v.sequence) AND NOT EXISTS(SELECT 1 FROM read_pins p WHERE p.version_id=v.id AND p.expires_at>clock_timestamp()) ORDER BY v.sequence LIMIT 64").bind(&s.root).bind(after).fetch_all(&mut *tx).await?;
            if candidates.is_empty() {
                return Err(fit.unwrap_err());
            }
            for candidate in &candidates {
                let sequence: i64 = candidate.get("sequence");
                sqlx::query("UPDATE transfer_sessions SET cursor=$2 WHERE id=$1")
                    .bind(&s.id)
                    .bind(sequence.to_string())
                    .execute(&mut *tx)
                    .await?;
                if effective(
                    &mut tx,
                    &s.root,
                    Some(&candidate.get::<String, _>("entry_id")),
                )
                .await?
                .policy
                .mode
                    != RetentionMode::Relaxed
                {
                    continue;
                }
                sqlx::query("INSERT INTO transfer_retire(root_id,session_id,version_id,reason) VALUES($1,$2,$3,'admission') ON CONFLICT(root_id,session_id,version_id) DO UPDATE SET reason='admission'").bind(&s.root).bind(&s.id).bind(candidate.get::<String,_>("id")).execute(&mut *tx).await?;
                (unique, pools) = inventory(&mut tx, &s).await?;
                fit = fits(unique, quota as u64, &pools, &available);
                if fit.is_ok() {
                    break;
                }
            }
            if fit.is_err() {
                tx.commit().await?;
                return self.upload_status(actor, root, upload).await;
            }
        }
        sqlx::query("UPDATE operations SET phase='reclaiming' WHERE id=$1")
            .bind(&s.fence.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.finish_reclamation(actor, root, upload, &s).await
    }
    async fn finish_reclamation(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
        s: &Session,
    ) -> Result<TransferStatus> {
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(&mut tx, &s.root).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        s.check_target(&mut tx, actor).await?;
        let rows=sqlx::query("SELECT r.version_id,v.entry_id FROM transfer_retire r JOIN file_versions v ON v.id=r.version_id WHERE r.session_id=$1 AND r.reason='admission' AND NOT r.applied ORDER BY v.sequence LIMIT 64").bind(&s.id).fetch_all(&mut *tx).await?;
        for r in rows {
            let version: String = r.get("version_id");
            require(
                effective(&mut tx, &s.root, Some(&r.get::<String, _>("entry_id")))
                    .await?
                    .policy
                    .mode
                    == RetentionMode::Relaxed,
                ErrorCode::RevisionConflict,
            )?;
            let protected:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entries WHERE current_version_id=$1) OR EXISTS(SELECT 1 FROM read_pins WHERE version_id=$1 AND expires_at>clock_timestamp())").bind(&version).fetch_one(&mut *tx).await?;
            require(!protected, ErrorCode::DestinationCapacity)?;
            sqlx::query("UPDATE file_versions SET retired=true WHERE id=$1")
                .bind(&version)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE transfer_retire SET applied=true WHERE session_id=$1 AND version_id=$2",
            )
            .bind(&s.id)
            .bind(&version)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'admission_retirement',$3)").bind(&s.root).bind(&s.fence.id).bind(serde_json::json!({"version_id":version})).execute(&mut *tx).await?;
        }
        let more:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transfer_retire WHERE session_id=$1 AND reason='admission' AND NOT applied)").bind(&s.id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        let garbage = self.collect_unreferenced(&s.fence).await?;
        self.cleanup_copies(&s.fence).await?;
        if more || garbage == 64 {
            return self.upload_status(actor, root, upload).await;
        }
        let available = self.probe_pools(&s.root).await?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(&mut tx, &s.root).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM object_copies WHERE root_id=$1 AND state IN ('writing','deleting'))").bind(&s.root).fetch_one(&mut *tx).await?;
        if pending {
            return Err(Error(ErrorCode::DestinationUnavailable));
        }
        // Pool locks serialize physical probes/reservations with every other root.
        sqlx::query("SELECT id FROM pools WHERE id IN (SELECT pool_id FROM allocations WHERE root_id=$1) ORDER BY id FOR UPDATE").bind(&s.root).fetch_all(&mut *tx).await?;
        let quota: i64 = sqlx::query_scalar("SELECT quota FROM roots WHERE id=$1")
            .bind(&s.root)
            .fetch_one(&mut *tx)
            .await?;
        let (unique, pools) = inventory(&mut tx, s).await?;
        fits(unique, quota as u64, &pools, &available)?;
        for p in pools {
            let n = p.incoming.max(p.outgoing) as i64;
            let allocation:String=sqlx::query_scalar("UPDATE allocations SET reserved=reserved+$2 WHERE pool_id=$1 AND root_id IS NULL AND capacity-reserved-occupied >= $2 RETURNING id").bind(&p.pool).bind(n).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::StagingExhausted))?;
            sqlx::query(
                "INSERT INTO reservations(id,allocation_id,operation_id,bytes) VALUES($1,$2,$3,$4)",
            )
            .bind(crate::error::id())
            .bind(allocation)
            .bind(&s.fence.id)
            .bind(n)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE transfer_sessions SET admission='admitted' WHERE id=$1")
            .bind(&s.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE operations SET phase='staging' WHERE id=$1")
            .bind(&s.fence.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.upload_status(actor, root, upload).await
    }
}
