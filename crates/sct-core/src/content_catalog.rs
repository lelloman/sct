//! Transactional graph/ledger primitives. No backend I/O is performed here.
use crate::{
    auth::{Actor, authorize},
    error::{Error, Result, id, require},
    lease::Writer,
    operations::{OperationFence, check_fence},
};
use sct_protocol::*;
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;

/// Root-row serialization also orders pin acquisition against retirement.
pub(crate) async fn lock_content_root(db: &mut PgConnection, root: &str) -> Result<()> {
    sqlx::query("SELECT id FROM roots WHERE id=$1 FOR UPDATE")
        .bind(root)
        .fetch_optional(db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
    Ok(())
}
impl Writer {
    pub async fn reserve_transfer(
        &self,
        fence: &OperationFence,
        demand: &BTreeMap<PoolId, ByteCount>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        lock_content_root(&mut tx, &fence.root).await?;
        check_fence(&mut tx, fence, self.epoch).await?;
        for (pool, bytes) in demand {
            let belongs:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM root_backends r JOIN backends b ON b.id=r.backend_id WHERE r.root_id=$1 AND b.pool_id=$2)").bind(&fence.root).bind(pool.as_str()).fetch_one(&mut *tx).await?;
            require(belongs, ErrorCode::InvalidRequest)?;
            sqlx::query("SELECT id FROM pools WHERE id=$1 FOR UPDATE")
                .bind(pool.as_str())
                .fetch_one(&mut *tx)
                .await?;
            let allocation: String = sqlx::query_scalar(
                "SELECT id FROM allocations WHERE pool_id=$1 AND root_id IS NULL FOR UPDATE",
            )
            .bind(pool.as_str())
            .fetch_one(&mut *tx)
            .await?;
            let previous: Option<i64> = sqlx::query_scalar(
                "SELECT bytes FROM reservations WHERE allocation_id=$1 AND operation_id=$2",
            )
            .bind(&allocation)
            .bind(&fence.id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(previous) = previous {
                require(
                    previous == bytes.get() as i64,
                    ErrorCode::IdempotencyConflict,
                )?;
                continue;
            }
            let n=sqlx::query("UPDATE allocations SET reserved=reserved+$2 WHERE id=$1 AND capacity-reserved-occupied >= $2").bind(&allocation).bind(bytes.get() as i64).execute(&mut *tx).await?.rows_affected();
            require(n == 1, ErrorCode::StagingExhausted)?;
            sqlx::query(
                "INSERT INTO reservations(id,allocation_id,operation_id,bytes) VALUES($1,$2,$3,$4)",
            )
            .bind(id())
            .bind(allocation)
            .bind(&fence.id)
            .bind(bytes.get() as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn pin_version(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
    ) -> Result<String> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM file_versions WHERE id=$1 AND root_id=$2 AND NOT retired AND NOT entry_purged(entry_id))",
        )
        .bind(version.as_str())
        .bind(root.as_str())
        .fetch_one(&mut *tx)
        .await?;
        require(exists, ErrorCode::NotFound)?;
        let pin = id();
        sqlx::query("INSERT INTO read_pins(id,root_id,version_id,principal_id,expires_at) VALUES($1,$2,$3,$4,clock_timestamp()+interval '30 seconds')").bind(&pin).bind(root.as_str()).bind(version.as_str()).bind(&actor.principal).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(pin)
    }
    pub async fn renew_read_pin(&self, actor: &Actor, root: &RootId, pin: &str) -> Result<()> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let n=sqlx::query("UPDATE read_pins SET expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1 AND root_id=$2 AND principal_id=$3 AND expires_at>clock_timestamp()").bind(pin).bind(root.as_str()).bind(&actor.principal).execute(&mut *tx).await?.rows_affected();
        require(n == 1, ErrorCode::NotFound)?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn release_read_pin(&self, actor: &Actor, pin: &str) -> Result<()> {
        let mut tx = self.transaction().await?;
        sqlx::query("DELETE FROM read_pins WHERE id=$1 AND principal_id=$2")
            .bind(pin)
            .bind(&actor.principal)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
/// Bounded adjacency fetch: immutable pages have no more than 1024 children.
pub(crate) async fn children(
    db: &mut PgConnection,
    root: &str,
    parent: &str,
) -> Result<Vec<(String, u64, u64)>> {
    let rows=sqlx::query("SELECT child,logical_length,chunk_count FROM content_edges WHERE root_id=$1 AND parent=$2 ORDER BY ordinal LIMIT 1024").bind(root).bind(parent).fetch_all(db).await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get("child"),
                r.get::<i64, _>("logical_length") as u64,
                r.get::<i64, _>("chunk_count") as u64,
            )
        })
        .collect())
}

pub(crate) async fn object(
    db: &mut PgConnection,
    root: &str,
    digest: &str,
    kind: &str,
    length: u64,
    height: Option<u32>,
    body: Option<&[u8]>,
) -> Result<()> {
    sqlx::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(root_id,digest) DO NOTHING").bind(root).bind(digest).bind(kind).bind(length as i64).bind(height.map(|v|v as i32)).bind(body).execute(&mut *db).await?;
    let r=sqlx::query("SELECT kind,length,height,body FROM content_objects WHERE root_id=$1 AND digest=$2 FOR UPDATE").bind(root).bind(digest).fetch_one(&mut *db).await?;
    require(
        r.get::<String, _>("kind") == kind
            && r.get::<i64, _>("length") == length as i64
            && r.get::<Option<i32>, _>("height") == height.map(|v| v as i32),
        ErrorCode::InvalidManifest,
    )?;
    if let Some(body) = body {
        match r.get::<Option<Vec<u8>>, _>("body") {
            Some(old) => require(old == body, ErrorCode::InvalidManifest)?,
            None => {
                sqlx::query("UPDATE content_objects SET body=$3 WHERE root_id=$1 AND digest=$2")
                    .bind(root)
                    .bind(digest)
                    .bind(body)
                    .execute(&mut *db)
                    .await?;
            }
        }
    }
    Ok(())
}
pub(crate) async fn edge(
    db: &mut PgConnection,
    root: &str,
    parent: &str,
    ordinal: usize,
    child: &str,
    length: u64,
    count: u64,
) -> Result<()> {
    sqlx::query("INSERT INTO content_edges(root_id,parent,ordinal,child,logical_length,chunk_count) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING").bind(root).bind(parent).bind(ordinal as i32).bind(child).bind(length as i64).bind(count as i64).execute(db).await?;
    Ok(())
}
impl Writer {
    pub async fn store_manifest_page(
        &self,
        fence: &OperationFence,
        expected: &ManifestId,
        bytes: &[u8],
    ) -> Result<()> {
        let page: ManifestPage = parse_canonical(bytes)?;
        page.validate()?;
        require(
            digest(bytes) == expected.as_str(),
            ErrorCode::IntegrityFailure,
        )?;
        let (height, length) = match &page {
            ManifestPage::Chunks { chunks, .. } => (0, chunks.iter().map(|c| c.length.get()).sum()),
            ManifestPage::Branch { height, pages, .. } => {
                (*height, pages.iter().map(|p| p.length.get()).sum())
            }
        };
        let mut tx = self.transaction().await?;
        lock_content_root(&mut tx, &fence.root).await?;
        check_fence(&mut tx, fence, self.epoch).await?;
        let frozen:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transfer_sessions WHERE operation_id=$1 AND admission<>'planning') OR EXISTS(SELECT 1 FROM tree_drafts WHERE operation_id=$1 AND (phase<>'editing' OR state<>'open'))").bind(&fence.id).fetch_one(&mut *tx).await?;
        require(!frozen, ErrorCode::InvalidRequest)?;
        object(
            &mut tx,
            &fence.root,
            expected.as_str(),
            "page",
            length,
            Some(height),
            Some(bytes),
        )
        .await?;
        match page {
            ManifestPage::Chunks { chunks, .. } => {
                for (i, c) in chunks.iter().enumerate() {
                    object(
                        &mut tx,
                        &fence.root,
                        c.digest.as_str(),
                        "chunk",
                        c.length.get(),
                        None,
                        None,
                    )
                    .await?;
                    edge(
                        &mut tx,
                        &fence.root,
                        expected.as_str(),
                        i,
                        c.digest.as_str(),
                        c.length.get(),
                        1,
                    )
                    .await?;
                }
            }
            ManifestPage::Branch { height, pages, .. } => {
                for (i, p) in pages.iter().enumerate() {
                    object(
                        &mut tx,
                        &fence.root,
                        p.digest.as_str(),
                        "page",
                        p.length.get(),
                        Some(height - 1),
                        None,
                    )
                    .await?;
                    edge(
                        &mut tx,
                        &fence.root,
                        expected.as_str(),
                        i,
                        p.digest.as_str(),
                        p.length.get(),
                        p.chunks.get(),
                    )
                    .await?;
                }
            }
        }
        // Read back the bounded adjacency to ensure exact persisted fanout.
        require(
            children(&mut tx, &fence.root, expected.as_str())
                .await?
                .len()
                <= MANIFEST_FANOUT,
            ErrorCode::InvalidManifest,
        )?;
        tx.commit().await?;
        Ok(())
    }
}
