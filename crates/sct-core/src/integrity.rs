//! Incremental verification of catalog graph edges and exact physical generations.
use crate::{
    auth::{self, Actor},
    content_catalog::{self, lock_content_root},
    error::{Error, Result, id, require},
    lease::Writer,
    maintenance::{MaintenanceClaim, MaintenanceExecutor, MaintenanceProgress},
    operations::check_fence,
    paging,
    payload::{ObjectKey, verified},
    roots::json,
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use sqlx::{Row, postgres::PgRow};
type Edges = Vec<(String, u64, u64)>;
fn shape(row: &PgRow) -> Result<Edges> {
    let kind: String = row.get("kind");
    let length = row.get::<i64, _>("length") as u64;
    let height: Option<i32> = row.get("height");
    let body: Option<Vec<u8>> = row.get("body");
    if kind == "chunk" {
        require(
            body.is_none() && height.is_none() && (1..=CHUNK_MAX as u64).contains(&length),
            ErrorCode::IntegrityFailure,
        )?;
        return Ok(Vec::new());
    }
    let body = body.ok_or(Error(ErrorCode::IntegrityFailure))?;
    require(
        digest(&body) == row.get::<String, _>("digest"),
        ErrorCode::IntegrityFailure,
    )?;
    let (expected_length, expected_height, edges): (u64, u32, Edges) = match kind.as_str() {
        "file" => {
            let m: FileManifest = parse_canonical(&body)?;
            m.validate()?;
            (
                m.length.get(),
                m.height,
                m.tree
                    .into_iter()
                    .map(|d| (d.as_str().into(), m.length.get(), m.chunks.get()))
                    .collect(),
            )
        }
        "directory" => {
            let m: DirectoryManifest = parse_canonical(&body)?;
            m.validate()?;
            (
                m.length.get(),
                m.height,
                m.tree
                    .into_iter()
                    .map(|d| (d.as_str().into(), m.length.get(), m.chunks.get()))
                    .collect(),
            )
        }
        "page" => {
            let page: ManifestPage = parse_canonical(&body)?;
            page.validate()?;
            match page {
                ManifestPage::Chunks { chunks, .. } => (
                    chunks.iter().map(|c| c.length.get()).sum(),
                    0,
                    chunks
                        .into_iter()
                        .map(|c| (c.digest.as_str().into(), c.length.get(), 1))
                        .collect(),
                ),
                ManifestPage::Branch { height, pages, .. } => (
                    pages.iter().map(|p| p.length.get()).sum(),
                    height,
                    pages
                        .into_iter()
                        .map(|p| (p.digest.as_str().into(), p.length.get(), p.chunks.get()))
                        .collect(),
                ),
            }
        }
        "directory_page" => {
            let page: DirectoryPage = parse_canonical(&body)?;
            page.validate()?;
            match page {
                DirectoryPage::DirectoryEntries { entries, .. } => (
                    entries.iter().map(|c| c.length.get()).sum(),
                    0,
                    entries
                        .into_iter()
                        .map(|c| (c.manifest.as_str().into(), c.length.get(), c.chunks.get()))
                        .collect(),
                ),
                DirectoryPage::DirectoryBranch { height, pages, .. } => (
                    pages.iter().map(|p| p.length.get()).sum(),
                    height,
                    pages
                        .into_iter()
                        .map(|p| (p.digest.as_str().into(), p.length.get(), p.chunks.get()))
                        .collect(),
                ),
            }
        }
        _ => return Err(Error(ErrorCode::IntegrityFailure)),
    };
    require(
        length == expected_length && height == Some(expected_height as i32),
        ErrorCode::IntegrityFailure,
    )?;
    Ok(edges)
}
impl TransferEngine {
    async fn finding(
        &self,
        claim: &MaintenanceClaim,
        digest: &str,
        backend: Option<&str>,
        generation: Option<&str>,
        kind: IntegrityFindingKind,
    ) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &claim.fence.root).await?;
        check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
        sqlx::query("INSERT INTO integrity_findings(id,root_id,job_id,backend_id,digest,generation,kind) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING").bind(id()).bind(&claim.fence.root).bind(&claim.job).bind(backend).bind(digest).bind(generation).bind(json(&kind)?.as_str().unwrap()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn integrity_batch(
        &self,
        claim: &MaintenanceClaim,
    ) -> Result<MaintenanceProgress> {
        let (rows, scan, backends) = {
            let mut tx = self.writer.transaction().await?;
            lock_content_root(&mut tx, &claim.fence.root).await?;
            check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
            if !crate::graph_work::advance(&mut tx, &claim.fence).await? {
                tx.commit().await?;
                return Ok(MaintenanceProgress {
                    cursor: serde_json::json!({}),
                    processed: 0,
                    complete: false,
                });
            }
            let scan: String =
                sqlx::query_scalar("SELECT scan_id FROM graph_scans WHERE root_id=$1")
                    .bind(&claim.fence.root)
                    .fetch_one(&mut *tx)
                    .await?;
            let after = if claim.cursor.get("scan").and_then(|v| v.as_str()) == Some(&scan) {
                claim
                    .cursor
                    .get("after")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            } else {
                ""
            };
            let rows=sqlx::query("SELECT o.* FROM graph_marks m JOIN content_objects o ON o.root_id=m.root_id AND o.digest=m.digest WHERE m.root_id=$1 AND m.scan_id=$2 AND m.digest>$3 ORDER BY m.digest LIMIT 16").bind(&claim.fence.root).bind(&scan).bind(after).fetch_all(&mut *tx).await?;
            let backends: Vec<String> = sqlx::query_scalar(
                "SELECT backend_id FROM root_backends WHERE root_id=$1 ORDER BY backend_id",
            )
            .bind(&claim.fence.root)
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            (rows, scan, backends)
        };
        let started = std::time::Instant::now();
        let mut processed = 0u64;
        let mut last = String::new();
        for row in &rows {
            let digest: String = row.get("digest");
            let catalog_valid = {
                let mut tx = self.writer.transaction().await?;
                check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
                let actual = content_catalog::children(&mut tx, &claim.fence.root, &digest).await?;
                let ordinals:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM (SELECT ordinal,row_number() OVER(ORDER BY ordinal)-1 expected FROM content_edges WHERE root_id=$1 AND parent=$2) e WHERE ordinal<>expected)").bind(&claim.fence.root).bind(&digest).fetch_one(&mut *tx).await?;
                shape(row).is_ok_and(|expected| expected == actual) && ordinals
            };
            if !catalog_valid {
                self.finding(
                    claim,
                    &digest,
                    None,
                    None,
                    IntegrityFindingKind::CatalogInconsistent,
                )
                .await?;
            }
            let expected_bytes = if row.get::<String, _>("kind") == "chunk" {
                row.get::<i64, _>("length") as u64
            } else {
                row.get::<Option<Vec<u8>>, _>("body")
                    .map_or(0, |b| b.len() as u64)
            };
            for backend in &backends {
                let copy = {
                    let mut tx = self.writer.transaction().await?;
                    check_fence(&mut tx, &claim.fence, self.writer.epoch).await?;
                    sqlx::query("SELECT generation,bytes FROM object_copies WHERE root_id=$1 AND digest=$2 AND backend_id=$3 AND state='available'").bind(&claim.fence.root).bind(&digest).bind(backend).fetch_optional(&mut *tx).await?
                };
                let Some(copy) = copy else {
                    self.finding(
                        claim,
                        &digest,
                        Some(backend),
                        None,
                        IntegrityFindingKind::Missing,
                    )
                    .await?;
                    continue;
                };
                let generation: String = copy.get("generation");
                if copy.get::<i64, _>("bytes") as u64 != expected_bytes
                    || !(1..=CHUNK_MAX as u64).contains(&expected_bytes)
                {
                    self.finding(
                        claim,
                        &digest,
                        Some(backend),
                        Some(&generation),
                        IntegrityFindingKind::CatalogInconsistent,
                    )
                    .await?;
                    continue;
                }
                let key = ObjectKey {
                    root: claim.fence.root.parse()?,
                    digest: digest.parse()?,
                    generation: generation.parse()?,
                };
                let read = async {
                    let _slot = self
                        .slots
                        .acquire()
                        .await
                        .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
                    let store = self.store(backend)?;
                    verified(store.read(&key).await?, &key, expected_bytes)
                        .await
                        .map(|_| ())
                };
                let result = tokio::time::timeout(std::time::Duration::from_secs(10), read)
                    .await
                    .unwrap_or(Err(Error(ErrorCode::DestinationUnavailable)));
                let kind = match result {
                    Ok(()) => None,
                    Err(Error(ErrorCode::NotFound)) => Some(IntegrityFindingKind::Missing),
                    Err(Error(ErrorCode::IntegrityFailure)) => Some(IntegrityFindingKind::Corrupt),
                    Err(_) => Some(IntegrityFindingKind::Inaccessible),
                };
                if let Some(kind) = kind {
                    self.finding(claim, &digest, Some(backend), Some(&generation), kind)
                        .await?;
                }
            }
            processed += 1;
            last = digest;
            if started.elapsed() >= std::time::Duration::from_secs(15) {
                break;
            }
        }
        Ok(MaintenanceProgress {
            cursor: serde_json::json!({"scan":scan,"after":last}),
            processed,
            complete: rows.len() < 16 && processed == rows.len() as u64,
        })
    }
}
impl MaintenanceExecutor for TransferEngine {
    async fn advance(&self, claim: &MaintenanceClaim) -> Result<MaintenanceProgress> {
        match claim.kind {
            MaintenanceKind::IntegrityScan => self.integrity_batch(claim).await,
            MaintenanceKind::MirrorRepair => Err(Error(ErrorCode::UnsupportedOperation)),
        }
    }
}
impl Writer {
    pub async fn list_integrity_findings(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<IntegrityFindingPage> {
        let mut tx = self.transaction().await?;
        auth::authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let scope = format!("integrity-findings:{}", root.as_str());
        let binding = json(&request.limit)?;
        let after: String = match &request.cursor {
            Some(token) => paging::consume(&mut tx, actor, &scope, token, &binding).await?,
            None => String::new(),
        };
        let rows = sqlx::query(
            "SELECT * FROM integrity_findings WHERE root_id=$1 AND id>$2 ORDER BY id LIMIT $3",
        )
        .bind(root.as_str())
        .bind(after)
        .bind(request.limit.get() as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
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
        let findings = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|r| {
                Ok(IntegrityFinding {
                    id: r.get("id"),
                    job_id: r.get::<String, _>("job_id").parse()?,
                    root_id: root.clone(),
                    backend_id: r
                        .get::<Option<String>, _>("backend_id")
                        .map(|s| s.parse())
                        .transpose()?,
                    digest: r.get::<String, _>("digest").parse()?,
                    generation: r.get("generation"),
                    kind: serde_json::from_value(json(&r.get::<String, _>("kind"))?)?,
                    repaired: r.get("repaired"),
                })
            })
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(IntegrityFindingPage {
            findings,
            next_cursor,
        })
    }
}
