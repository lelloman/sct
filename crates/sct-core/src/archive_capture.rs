//! Stable version pins and bounded immutable graph enumeration.
use crate::{
    archive_catalog::*,
    archive_management::source_access,
    auth::{self, Actor},
    content_catalog::lock_content_root,
    error::{Error, Result, id, require},
    lease::Writer,
    roots::json,
    tree_catalog::metadata,
};
use db::PgConnection;
use sct_protocol::*;
use simple_server::database::postgres as db;

/// Serialize account limits before any shared/exclusive root locks. Pin acquisition
/// upgrades no locks: retirement and capture use the same exclusive root boundary.
pub(crate) async fn lock_capture(
    db: &mut PgConnection,
    actor: &Actor,
    owner: &PrincipalId,
    selections: &[ArchiveSelection],
) -> Result<()> {
    db::query("SELECT principal_id FROM archive_accounts WHERE principal_id=$1 FOR UPDATE")
        .bind(owner.as_str())
        .fetch_one(&mut *db)
        .await?;
    let roots: std::collections::BTreeSet<_> =
        selections.iter().map(|s| s.root_id.as_str()).collect();
    for root in roots {
        lock_content_root(db, root).await?;
    }
    source_access(db, actor, selections).await
}
/// Called in the same transaction as set creation, before any response is returned.
pub(crate) async fn pin_capture(
    db: &mut PgConnection,
    owner: &PrincipalId,
    set: &ArchiveSetId,
    selections: &[ArchiveSelection],
) -> Result<()> {
    let limits=db::query("SELECT *, (SELECT count(*) FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE c.owner_id=$1 AND s.state IN ('capturing','planning','awaiting_confirmation','building')) AS active FROM archive_accounts WHERE principal_id=$1 FOR UPDATE").bind(owner.as_str()).fetch_one(&mut *db).await?;
    require(
        limits.get::<i64, _>("active") <= limits.get::<i32, _>("max_active") as i64,
        ErrorCode::ArchiveBusy,
    )?;
    let mut bytes = 0u64;
    let mut records = 0u64;
    for (ordinal, selection) in selections.iter().enumerate() {
        let version=db::query("SELECT manifest,kind,length FROM file_versions WHERE root_id=$1 AND id=$2 AND NOT retired AND NOT entry_purged(entry_id)").bind(selection.root_id.as_str()).bind(selection.version_id.as_str()).fetch_optional(&mut *db).await?.ok_or(Error(ErrorCode::NotFound))?;
        let length = version.get::<i64, _>("length");
        bytes = bytes
            .checked_add(length as u64)
            .ok_or(Error(ErrorCode::ArchiveResourceLimit))?;
        let pin = format!("archive_{}_{}", set.as_str(), ordinal);
        let manifest: String = version.get("manifest");
        let kind: String = version.get("kind");
        require(
            kind == "directory" || selection.path.as_str() != "/",
            ErrorCode::InvalidPath,
        )?;
        // A fixed set deadline is enforced by fenced cleanup, not GC wall-clock
        // expiry that could race an in-flight export batch.
        db::query("INSERT INTO graph_pins(root_id,id,kind,manifest,expires_at) VALUES($1,$2,'job',$3,NULL)").bind(selection.root_id.as_str()).bind(&pin).bind(&manifest).execute(&mut *db).await?;
        db::query("UPDATE archive_sources SET manifest=$3,kind=$4,pin_id=$5 WHERE set_id=$1 AND ordinal=$2").bind(set.as_str()).bind(ordinal as i32).bind(&manifest).bind(&kind).bind(pin).execute(&mut *db).await?;
        db::query("INSERT INTO archive_frontier(set_id,path,root_id,version_id,manifest,kind,expected_length) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(set.as_str()).bind(selection.path.as_str()).bind(selection.root_id.as_str()).bind(selection.version_id.as_str()).bind(manifest).bind(kind).bind(length).execute(&mut *db).await?;
        records += 1;
        let mut parent = selection.path.as_str();
        while parent != "/" {
            parent = parent
                .rsplit_once('/')
                .map(|(p, _)| if p.is_empty() { "/" } else { p })
                .ok_or(Error(ErrorCode::InvalidPath))?;
            records+=db::query("INSERT INTO archive_entries(set_id,path,kind,length) VALUES($1,$2,'directory',0) ON CONFLICT DO NOTHING").bind(set.as_str()).bind(parent).execute(&mut *db).await?.rows_affected();
        }
    }
    require(bytes <= MAX_SAFE_INTEGER, ErrorCode::ArchiveResourceLimit)?;
    require(
        records <= limits.get::<i64, _>("max_capture_records") as u64,
        ErrorCode::ArchiveResourceLimit,
    )?;
    let charged=db::query("UPDATE archive_accounts SET pinned_bytes=pinned_bytes+$2 WHERE principal_id=$1 AND pin_quota_bytes-pinned_bytes >= $2").bind(owner.as_str()).bind(bytes as i64).execute(&mut *db).await?.rows_affected();
    require(charged == 1, ErrorCode::ArchiveResourceLimit)?;
    db::query("UPDATE archive_sets SET pinned_bytes=$2,capture_records=$3,capture_record_limit=$4,expires_at=clock_timestamp()+make_interval(secs=>$5),processed_entries=(SELECT count(*) FROM archive_entries WHERE set_id=$1) WHERE id=$1")
        .bind(set.as_str()).bind(bytes as i64).bind(records as i64).bind(limits.get::<i64,_>("max_capture_records")).bind(limits.get::<i32,_>("pin_deadline_seconds") as f64).execute(&mut *db).await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CaptureLimits {
    pub revision: Revision,
    pub pin_quota_bytes: ByteCount,
    pub max_active: u32,
    pub max_records: ByteCount,
    pub deadline_seconds: u32,
}
impl Writer {
    pub async fn archive_capture_limits(
        &self,
        actor: &Actor,
        principal: &PrincipalId,
    ) -> Result<CaptureLimits> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row = db::query("SELECT * FROM archive_accounts WHERE principal_id=$1")
            .bind(principal.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        Ok(CaptureLimits {
            revision: row.get::<String, _>("revision").parse()?,
            pin_quota_bytes: (row.get::<i64, _>("pin_quota_bytes") as u64).try_into()?,
            max_active: row.get::<i32, _>("max_active") as u32,
            max_records: (row.get::<i64, _>("max_capture_records") as u64).try_into()?,
            deadline_seconds: row.get::<i32, _>("pin_deadline_seconds") as u32,
        })
    }
    pub async fn configure_archive_capture_limits(
        &self,
        actor: &Actor,
        principal: &PrincipalId,
        limits: &CaptureLimits,
    ) -> Result<()> {
        require(
            (1..=64).contains(&limits.max_active)
                && (1..=ARCHIVE_MAX_RECORDS).contains(&limits.max_records.get())
                && (60..=604800).contains(&limits.deadline_seconds),
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row = db::query(
            "SELECT revision,pinned_bytes FROM archive_accounts WHERE principal_id=$1 FOR UPDATE",
        )
        .bind(principal.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
        require(
            row.get::<String, _>("revision") == limits.revision.as_str(),
            ErrorCode::RevisionConflict,
        )?;
        require(
            row.get::<i64, _>("pinned_bytes") as u64 <= limits.pin_quota_bytes.get(),
            ErrorCode::ArchiveResourceLimit,
        )?;
        db::query("UPDATE archive_accounts SET revision=$2,pin_quota_bytes=$3,max_active=$4,max_capture_records=$5,pin_deadline_seconds=$6 WHERE principal_id=$1").bind(principal.as_str()).bind(id()).bind(limits.pin_quota_bytes.get() as i64).bind(limits.max_active as i32).bind(limits.max_records.get() as i64).bind(limits.deadline_seconds as i32).execute(&mut *tx).await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "archive.capture.limits",
            serde_json::json!({"principal_id":principal}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[derive(Default)]
struct Progress {
    entries: u64,
    bytes: u64,
    records: u64,
}
struct PageWork<'a> {
    set: &'a str,
    path: &'a str,
    root: &'a str,
    version: &'a str,
    manifest: &'a str,
    kind: &'a str,
    offset: u64,
    height: u32,
    length: u64,
    chunks: u64,
    entries: Option<u64>,
}
async fn queue_page(db: &mut PgConnection, p: PageWork<'_>) -> Result<()> {
    db::query("INSERT INTO archive_capture_pages(set_id,path,root_id,version_id,manifest,kind,offset_bytes,height,expected_length,expected_chunks,expected_entries) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(p.set).bind(p.path).bind(p.root).bind(p.version).bind(p.manifest).bind(p.kind).bind(p.offset as i64).bind(p.height as i32).bind(p.length as i64).bind(p.chunks as i64).bind(p.entries.map(|n|n as i64)).execute(db).await?;
    Ok(())
}
async fn expand_manifest(db: &mut PgConnection, set: &str, row: &db::PgRow) -> Result<Progress> {
    let path: String = row.get("path");
    let root: String = row.get("root_id");
    let version: String = row.get("version_id");
    let manifest: String = row.get("manifest");
    let kind: String = row.get("kind");
    let mut progress = Progress::default();
    let (length, chunks, height, tree, entries) = if kind == "file" {
        let value: FileManifest = metadata(db, &root, &manifest, "file").await?;
        value.validate()?;
        (
            value.length.get(),
            value.chunks.get(),
            value.height,
            value.tree,
            None,
        )
    } else {
        let value: DirectoryManifest = metadata(db, &root, &manifest, "directory").await?;
        value.validate()?;
        (
            value.length.get(),
            value.chunks.get(),
            value.height,
            value.tree,
            Some(value.entries.get()),
        )
    };
    require(
        length == row.get::<i64, _>("expected_length") as u64
            && row
                .get::<Option<i64>, _>("expected_chunks")
                .is_none_or(|n| n as u64 == chunks),
        ErrorCode::IntegrityFailure,
    )?;
    progress.entries=db::query("INSERT INTO archive_entries(set_id,path,kind,root_id,version_id,length) VALUES($1,$2,$3,$4,$5,$6)").bind(set).bind(&path).bind(&kind).bind(&root).bind(&version).bind(if kind=="file"{length as i64}else{0}).execute(&mut *db).await?.rows_affected();
    progress.records += progress.entries;
    if kind == "file" {
        progress.bytes = length;
    }
    if let Some(tree) = tree {
        queue_page(
            db,
            PageWork {
                set,
                path: &path,
                root: &root,
                version: &version,
                manifest: tree.as_str(),
                kind: if kind == "file" {
                    "page"
                } else {
                    "directory_page"
                },
                offset: 0,
                height,
                length,
                chunks,
                entries,
            },
        )
        .await?;
        progress.records += 1;
    }
    db::query("UPDATE archive_frontier SET expanded=true WHERE set_id=$1 AND path=$2")
        .bind(set)
        .bind(path)
        .execute(db)
        .await?;
    Ok(progress)
}
async fn expand_page(db: &mut PgConnection, set: &str, row: &db::PgRow) -> Result<Progress> {
    let path: String = row.get("path");
    let root: String = row.get("root_id");
    let version: String = row.get("version_id");
    let manifest: String = row.get("manifest");
    let kind: String = row.get("kind");
    let offset = row.get::<i64, _>("offset_bytes") as u64;
    let height = row.get::<i32, _>("height") as u32;
    let length = row.get::<i64, _>("expected_length") as u64;
    let chunks = row.get::<i64, _>("expected_chunks") as u64;
    let mut progress = Progress::default();
    let mut children = Vec::new();
    if kind == "page" {
        let page: ManifestPage = metadata(db, &root, &manifest, "page").await?;
        page.validate()?;
        match page {
            ManifestPage::Chunks { chunks: refs, .. } => {
                require(
                    height == 0
                        && refs.len() as u64 == chunks
                        && refs.iter().map(|r| r.length.get()).sum::<u64>() == length,
                    ErrorCode::IntegrityFailure,
                )?;
                let mut position = offset;
                let mut values = Vec::new();
                for object in refs {
                    values.push(serde_json::json!({"digest":object.digest,"length":object.length,"offset_bytes":position}));
                    position += object.length.get();
                }
                let values = json(&values)?;
                progress.records+=db::query("INSERT INTO archive_objects(set_id,root_id,digest,length) SELECT DISTINCT $1,$2,digest,length FROM jsonb_to_recordset($3) AS x(digest text,length bigint,offset_bytes bigint) ON CONFLICT DO NOTHING").bind(set).bind(&root).bind(&values).execute(&mut *db).await?.rows_affected();
                progress.records+=db::query("INSERT INTO archive_extents(set_id,path,offset_bytes,root_id,digest,length) SELECT $1,$2,offset_bytes,$3,digest,length FROM jsonb_to_recordset($4) AS x(digest text,length bigint,offset_bytes bigint)").bind(set).bind(&path).bind(&root).bind(values).execute(&mut *db).await?.rows_affected();
            }
            ManifestPage::Branch {
                height: actual,
                pages,
                ..
            } => {
                require(
                    actual == height
                        && height > 0
                        && pages.iter().map(|p| p.length.get()).sum::<u64>() == length
                        && pages.iter().map(|p| p.chunks.get()).sum::<u64>() == chunks,
                    ErrorCode::IntegrityFailure,
                )?;
                let mut position = offset;
                for page in pages {
                    children.push(serde_json::json!({"manifest":page.digest,"offset_bytes":position,"height":height-1,"length":page.length,"chunks":page.chunks,"entries":null}));
                    position += page.length.get();
                }
            }
        }
    } else {
        let expected = row.get::<i64, _>("expected_entries") as u64;
        let page: DirectoryPage = metadata(db, &root, &manifest, "directory_page").await?;
        page.validate()?;
        match page {
            DirectoryPage::DirectoryEntries { entries, .. } => {
                require(
                    height == 0
                        && entries.len() as u64 == expected
                        && entries.iter().map(|e| e.length.get()).sum::<u64>() == length
                        && entries.iter().map(|e| e.chunks.get()).sum::<u64>() == chunks,
                    ErrorCode::IntegrityFailure,
                )?;
                let mut values = Vec::new();
                for entry in entries {
                    let child: RootPath =
                        format!("{}/{}", path.trim_end_matches('/'), entry.name.as_str())
                            .parse()?;
                    archive_path(&child)?;
                    values.push(serde_json::json!({"path":child,"manifest":entry.manifest,"kind":entry.kind,"length":entry.length,"chunks":entry.chunks}));
                }
                progress.records+=db::query("INSERT INTO archive_frontier(set_id,path,root_id,version_id,manifest,kind,expected_length,expected_chunks) SELECT $1,path,$2,$3,manifest,kind,length,chunks FROM jsonb_to_recordset($4) AS x(path text,manifest text,kind text,length bigint,chunks bigint)").bind(set).bind(&root).bind(&version).bind(json(&values)?).execute(&mut *db).await?.rows_affected();
            }
            DirectoryPage::DirectoryBranch {
                height: actual,
                pages,
                ..
            } => {
                require(
                    actual == height
                        && height > 0
                        && pages.iter().map(|p| p.entries.get()).sum::<u64>() == expected
                        && pages.iter().map(|p| p.length.get()).sum::<u64>() == length
                        && pages.iter().map(|p| p.chunks.get()).sum::<u64>() == chunks,
                    ErrorCode::IntegrityFailure,
                )?;
                for page in pages {
                    children.push(serde_json::json!({"manifest":page.digest,"offset_bytes":0,"height":height-1,"length":page.length,"chunks":page.chunks,"entries":page.entries}));
                }
            }
        }
    }
    if !children.is_empty() {
        progress.records+=db::query("INSERT INTO archive_capture_pages(set_id,path,root_id,version_id,manifest,kind,offset_bytes,height,expected_length,expected_chunks,expected_entries) SELECT $1,$2,$3,$4,manifest,$5,offset_bytes,height,length,chunks,entries FROM jsonb_to_recordset($6) AS x(manifest text,offset_bytes bigint,height integer,length bigint,chunks bigint,entries bigint)").bind(set).bind(&path).bind(&root).bind(&version).bind(kind).bind(json(&children)?).execute(&mut *db).await?.rows_affected();
    }
    db::query("UPDATE archive_capture_pages SET expanded=true WHERE set_id=$1 AND path=$2 AND manifest=$3 AND offset_bytes=$4").bind(set).bind(path).bind(manifest).bind(offset as i64).execute(db).await?;
    Ok(progress)
}
impl Writer {
    /// One native metadata node (at most 256 KiB/1024 children), one short
    /// transaction. No network I/O or long-lived database snapshot is held.
    pub async fn advance_archive_capture(&self, set: &ArchiveSetId) -> Result<bool> {
        if !self.authorize_archive_work(set).await? {
            return Ok(false);
        }
        let mut tx = self.transaction().await?;
        let row=db::query("SELECT s.collection_id,c.owner_id FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE s.id=$1 FOR UPDATE OF c").bind(set.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        let collection: ArchiveCollectionId = row.get::<String, _>("collection_id").parse()?;
        let set_record=db::query("SELECT state,principal_id,credential_id,expires_at<=clock_timestamp() AS expired FROM archive_sets WHERE id=$1 FOR UPDATE").bind(set.as_str()).fetch_one(&mut *tx).await?;
        if set_record.get::<String, _>("state") != "capturing" {
            return Ok(false);
        }
        if set_record.get::<bool, _>("expired") {
            db::query("UPDATE archive_sets SET state='expired',failure='archive_expired',revision=$2 WHERE id=$1").bind(set.as_str()).bind(id()).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(false);
        }
        let actor = Actor {
            principal: set_record.get("principal_id"),
            credential: set_record.get("credential_id"),
        };
        let snapshot = load_set(&mut tx, &collection, set).await?;
        source_access(&mut tx, &actor, &snapshot.selections).await?;
        // Epochs survive process replacement; SQL row locks fence concurrent workers.
        db::query("UPDATE archive_jobs SET worker_epoch=worker_epoch+1,server_epoch=$2,worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE set_id=$1").bind(set.as_str()).bind(self.epoch).execute(&mut *tx).await?;
        let node = db::query(
            "SELECT * FROM archive_frontier WHERE set_id=$1 AND NOT expanded ORDER BY path LIMIT 1",
        )
        .bind(set.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        let progress = if let Some(node) = node {
            expand_manifest(&mut tx, set.as_str(), &node).await?
        } else {
            let node=db::query("SELECT * FROM archive_capture_pages WHERE set_id=$1 AND NOT expanded ORDER BY path,manifest,offset_bytes LIMIT 1").bind(set.as_str()).fetch_optional(&mut *tx).await?;
            if let Some(node) = node {
                expand_page(&mut tx, set.as_str(), &node).await?
            } else {
                db::query("UPDATE archive_sets SET state='planning',revision=$2 WHERE id=$1")
                    .bind(set.as_str())
                    .bind(id())
                    .execute(&mut *tx)
                    .await?;
                db::query(
                    "UPDATE archive_jobs SET phase='plan',worker_expires_at=NULL WHERE set_id=$1",
                )
                .bind(set.as_str())
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Ok(false);
            }
        };
        let changed=db::query("UPDATE archive_sets SET processed_entries=processed_entries+$2,processed_bytes=processed_bytes+$3,capture_records=capture_records+$4,revision=$5 WHERE id=$1 AND capture_record_limit-capture_records >= $4 AND processed_bytes <= 9007199254740991-$3").bind(set.as_str()).bind(progress.entries as i64).bind(progress.bytes as i64).bind(progress.records as i64).bind(id()).execute(&mut *tx).await?.rows_affected();
        require(changed == 1, ErrorCode::ArchiveResourceLimit)?;
        db::query("UPDATE archive_jobs SET worker_expires_at=NULL WHERE set_id=$1")
            .bind(set.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
    /// Expiry/revocation fences the set before owned pins are released. Cleanup is
    /// bounded to 64 pins and does not rewrite frozen optical source history.
    pub async fn cleanup_archive_capture(&self, set: &ArchiveSetId) -> Result<bool> {
        let mut tx = self.transaction().await?;
        let row=db::query("SELECT c.id,c.owner_id FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE s.id=$1 FOR UPDATE OF c").bind(set.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        let owner: String = row.get("owner_id");
        db::query("SELECT principal_id FROM archive_accounts WHERE principal_id=$1 FOR UPDATE")
            .bind(&owner)
            .fetch_one(&mut *tx)
            .await?;
        let row=db::query("SELECT state,pinned_bytes,expires_at<=clock_timestamp() AS expired FROM archive_sets WHERE id=$1 FOR UPDATE").bind(set.as_str()).fetch_one(&mut *tx).await?;
        let state: ArchiveJobState = serde_json::from_value(json(&row.get::<String, _>("state"))?)?;
        if !state.terminal() {
            if !row.get::<bool, _>("expired") {
                return Ok(false);
            }
            db::query("UPDATE archive_sets SET state='expired',failure='archive_expired',revision=$2 WHERE id=$1").bind(set.as_str()).bind(id()).execute(&mut *tx).await?;
        }
        db::query("UPDATE archive_jobs SET worker_epoch=worker_epoch+1,worker_expires_at=NULL WHERE set_id=$1").bind(set.as_str()).execute(&mut *tx).await?;
        let pins=db::query("SELECT s.root_id,s.pin_id FROM archive_sources s JOIN graph_pins p ON p.root_id=s.root_id AND p.id=s.pin_id WHERE s.set_id=$1 ORDER BY s.root_id,s.pin_id LIMIT 64").bind(set.as_str()).fetch_all(&mut *tx).await?;
        for pin in &pins {
            let root: String = pin.get("root_id");
            lock_content_root(&mut tx, &root).await?;
            db::query("DELETE FROM graph_pins WHERE root_id=$1 AND id=$2 AND kind='job'")
                .bind(root)
                .bind(pin.get::<String, _>("pin_id"))
                .execute(&mut *tx)
                .await?;
        }
        let remains:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM archive_sources s JOIN graph_pins p ON p.root_id=s.root_id AND p.id=s.pin_id WHERE s.set_id=$1)").bind(set.as_str()).fetch_one(&mut *tx).await?;
        if !remains {
            db::query(
                "UPDATE archive_accounts SET pinned_bytes=pinned_bytes-$2 WHERE principal_id=$1",
            )
            .bind(owner)
            .bind(row.get::<i64, _>("pinned_bytes"))
            .execute(&mut *tx)
            .await?;
            db::query("UPDATE archive_sets SET pinned_bytes=0 WHERE id=$1")
                .bind(set.as_str())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(!remains)
    }
    pub async fn archive_capture_tick(&self) -> Result<()> {
        let sets: Vec<String> = {
            let mut tx = self.transaction().await?;
            db::query_scalar("SELECT s.id FROM archive_sets s JOIN archive_jobs j ON j.set_id=s.id WHERE (j.next_attempt_at IS NULL OR j.next_attempt_at<=clock_timestamp()) AND (s.state IN ('capturing','planning','awaiting_confirmation','building') OR s.pinned_bytes>0 OR EXISTS(SELECT 1 FROM archive_sources src JOIN graph_pins p ON p.root_id=src.root_id AND p.id=src.pin_id WHERE src.set_id=s.id)) ORDER BY j.next_attempt_at NULLS FIRST,s.id LIMIT 4").fetch_all(&mut *tx).await?
        };
        for identity in sets {
            let set = identity.parse()?;
            let allowed = self.authorize_archive_work(&set).await?;
            if allowed {
                for _ in 0..32 {
                    match self.advance_archive_capture(&set).await {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) => {
                            let mut tx = self.transaction().await?;
                            let failure = if matches!(
                                error.0,
                                ErrorCode::Forbidden
                                    | ErrorCode::Unauthorized
                                    | ErrorCode::NotFound
                            ) {
                                "revoked"
                            } else {
                                "failed"
                            };
                            db::query("UPDATE archive_sets SET state=$2,failure=$3,revision=$4 WHERE id=$1 AND state='capturing'").bind(set.as_str()).bind(failure).bind(serde_json::to_value(&error.0)?.as_str().ok_or(Error(ErrorCode::InternalError))?).bind(id()).execute(&mut *tx).await?;
                            tx.commit().await?;
                            break;
                        }
                    }
                }
            }
            self.cleanup_archive_capture(&set).await?;
            let mut tx = self.transaction().await?;
            db::query("UPDATE archive_jobs SET next_attempt_at=clock_timestamp()+interval '1 second' WHERE set_id=$1").bind(set.as_str()).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        Ok(())
    }
}
