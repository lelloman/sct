//! Authorized immutable ISO streams and durable cache read leases.
use crate::{
    archive_iso,
    archive_management::disc_access,
    auth,
    auth::Actor,
    error::{Error, Result, id, require},
    lease::Writer,
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use sha2::{Digest, Sha256};
use simple_server::database::postgres as db;
use simple_server::io::{AsyncReadExt, AsyncSeekExt};
#[derive(Debug, Clone)]
pub struct ArchiveImageInfo {
    pub artifact: ArchiveArtifact,
    pub disc_number: u64,
}
pub struct ArchiveDownload {
    writer: Writer,
    actor: Actor,
    collection: ArchiveCollectionId,
    disc: ArchiveDiscId,
    lease: String,
    file: simple_server::fs::File,
    position: u64,
    hash: Option<Sha256>,
    released: bool,
    pub length: u64,
    pub start: u64,
    pub end: u64,
    pub sha256: ChunkDigest,
}
impl TransferEngine {
    pub fn archive_service_status(&self) -> ArchiveServiceStatus {
        ArchiveServiceStatus {
            configured: self.archive.is_some(),
            bundle_version: self
                .archive
                .as_ref()
                .map(|r| r.bundle.manifest.version.clone()),
            platform: self
                .archive
                .as_ref()
                .map(|r| r.bundle.manifest.platform.clone()),
        }
    }

    /// Expire disposable images while retaining immutable logical history.
    pub async fn cleanup_archive_cache(&self) -> Result<()> {
        let Some(runtime) = &self.archive else {
            return Ok(());
        };
        let _guard = runtime.mutex.lock().await;
        let _disk = runtime.lock()?;
        self.cleanup_archive_cache_locked().await
    }
    pub(crate) async fn cleanup_archive_cache_locked(&self) -> Result<()> {
        let Some(runtime) = &self.archive else {
            return Ok(());
        };
        let mut tx = self.writer.transaction().await?;
        db::query("DELETE FROM archive_downloads WHERE id IN (SELECT id FROM archive_downloads WHERE expires_at<=clock_timestamp() ORDER BY expires_at,id LIMIT 128 FOR UPDATE SKIP LOCKED)").execute(&mut *tx).await?;
        // Locking the artifact serializes this intent with new download leases.
        // Existing streams renew independently; a live lease prevents intent.
        let rows = db::query("SELECT a.id FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id JOIN archive_sets s ON s.id=d.set_id WHERE s.state='ready' AND a.state='available' AND (a.cleanup_pending OR a.expires_at<=clock_timestamp()) AND NOT EXISTS(SELECT 1 FROM archive_downloads l WHERE l.artifact_id=a.id AND l.expires_at>clock_timestamp()) ORDER BY a.expires_at,a.id LIMIT 16 FOR UPDATE OF a SKIP LOCKED").fetch_all(&mut *tx).await?;
        let mut artifacts = Vec::new();
        for row in rows {
            let artifact: String = row.get("id");
            // A separate statement gets a fresh READ COMMITTED snapshot after
            // acquiring the row lock; a concurrent new lease cannot be missed.
            let marked = db::query("UPDATE archive_artifacts SET cleanup_pending=true WHERE id=$1 AND NOT EXISTS(SELECT 1 FROM archive_downloads WHERE artifact_id=$1 AND expires_at>clock_timestamp())")
                .bind(&artifact).execute(&mut *tx).await?.rows_affected();
            if marked == 1 {
                artifacts.push(artifact);
            }
        }
        tx.commit().await?;
        for artifact in artifacts {
            require(
                artifact.len() == 32 && artifact.bytes().all(|b| b.is_ascii_hexdigit()),
                ErrorCode::IntegrityFailure,
            )?;
            match simple_server::fs::remove_file(
                runtime.directory.join(format!("cache/{artifact}.iso")),
            )
            .await
            {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error(ErrorCode::DestinationUnavailable)),
            }
            crate::archive_export::sync_directory(&runtime.directory.join("cache")).await?;
            // Durable intent survives a crash on either side of unlink. Debit
            // once, only after unlink succeeded or the file is already absent.
            let mut tx = self.writer.transaction().await?;
            let owner: String = db::query_scalar("SELECT c.owner_id FROM archive_collections c JOIN archive_discs d ON d.collection_id=c.id JOIN archive_artifacts a ON a.disc_id=d.id WHERE a.id=$1 FOR UPDATE OF c").bind(&artifact).fetch_one(&mut *tx).await?;
            db::query("SELECT singleton FROM archive_storage WHERE singleton FOR UPDATE")
                .fetch_one(&mut *tx)
                .await?;
            db::query("SELECT principal_id FROM archive_accounts WHERE principal_id=$1 FOR UPDATE")
                .bind(&owner)
                .fetch_one(&mut *tx)
                .await?;
            let released: Option<i64> = db::query_scalar("UPDATE archive_artifacts SET state='expired',relative_path=NULL,cleanup_pending=false WHERE id=$1 AND state='available' AND cleanup_pending RETURNING length").bind(&artifact).fetch_optional(&mut *tx).await?;
            if let Some(bytes) = released {
                db::query("UPDATE archive_storage SET cache_bytes=cache_bytes-$1 WHERE singleton")
                    .bind(bytes)
                    .execute(&mut *tx)
                    .await?;
                db::query(
                    "UPDATE archive_accounts SET cache_bytes=cache_bytes-$2 WHERE principal_id=$1",
                )
                .bind(&owner)
                .bind(bytes)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
        Ok(())
    }
    pub async fn archive_image_info(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
    ) -> Result<ArchiveImageInfo> {
        require(
            self.archive.is_some(),
            ErrorCode::ArchiveArtifactUnavailable,
        )?;
        let mut tx = self.writer.transaction().await?;
        let set = disc_access(&mut tx, actor, collection, disc).await?;
        require(
            set.state == ArchiveJobState::Ready,
            ErrorCode::ArchiveArtifactUnavailable,
        )?;
        let row=db::query("SELECT a.*,d.number,(extract(epoch FROM a.expires_at)*1000)::bigint AS expires_ms FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id WHERE a.disc_id=$1 AND a.state='available' AND NOT a.cleanup_pending AND a.expires_at>clock_timestamp()").bind(disc.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::ArchiveArtifactUnavailable))?;
        let info = ArchiveImageInfo {
            artifact: ArchiveArtifact {
                id: row.get::<String, _>("id").parse()?,
                state: ArchiveArtifactState::Available,
                sha256: Some(row.get::<String, _>("sha256").parse()?),
                length: Some((row.get::<i64, _>("length") as u64).try_into()?),
                expires_at_ms: (row.get::<i64, _>("expires_ms") as u64).try_into()?,
                download_count: (row.get::<i64, _>("download_count") as u64).try_into()?,
            },
            disc_number: row.get::<i64, _>("number") as u64,
        };
        drop(tx);
        let runtime = self
            .archive
            .as_ref()
            .ok_or(Error(ErrorCode::ArchiveArtifactUnavailable))?;
        let artifact = info.artifact.id.as_str();
        require(
            artifact.len() == 32 && artifact.bytes().all(|b| b.is_ascii_hexdigit()),
            ErrorCode::IntegrityFailure,
        )?;
        let path = runtime.directory.join(format!("cache/{artifact}.iso"));
        if archive_iso::regular(&path).is_err() {
            let mut tx = self.writer.transaction().await?;
            db::query("UPDATE archive_artifacts SET expires_at=least(expires_at,clock_timestamp()) WHERE id=$1 AND state='available'").bind(artifact).execute(&mut *tx).await?;
            tx.commit().await?;
            return Err(Error(ErrorCode::ArchiveArtifactUnavailable));
        }
        Ok(info)
    }
    pub async fn download_archive_iso(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        range: Option<(u64, u64)>,
    ) -> Result<ArchiveDownload> {
        let runtime = self
            .archive
            .as_ref()
            .ok_or(Error(ErrorCode::ArchiveArtifactUnavailable))?;
        let mut tx = self.writer.transaction().await?;
        let set = disc_access(&mut tx, actor, collection, disc).await?;
        require(
            set.state == ArchiveJobState::Ready,
            ErrorCode::ArchiveArtifactUnavailable,
        )?;
        let row=db::query("SELECT * FROM archive_artifacts WHERE disc_id=$1 AND state='available' AND NOT cleanup_pending AND expires_at>clock_timestamp() FOR UPDATE").bind(disc.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::ArchiveArtifactUnavailable))?;
        let artifact: String = row.get("id");
        require(
            artifact.len() == 32 && artifact.bytes().all(|b| b.is_ascii_hexdigit()),
            ErrorCode::IntegrityFailure,
        )?;
        let relative: String = row.get("relative_path");
        require(
            relative == format!("cache/{artifact}.iso"),
            ErrorCode::IntegrityFailure,
        )?;
        let length = row.get::<i64, _>("length") as u64;
        let (start, end) = range.unwrap_or((0, length));
        require(start < end && end <= length, ErrorCode::InvalidRequest)?;
        let lease = id();
        db::query("INSERT INTO archive_downloads(id,artifact_id,principal_id,credential_id,expires_at) VALUES($1,$2,$3,$4,clock_timestamp()+interval '30 seconds')").bind(&lease).bind(&artifact).bind(actor.principal_id()).bind(&actor.credential).execute(&mut *tx).await?;
        db::query("UPDATE archive_artifacts SET download_count=download_count+1 WHERE id=$1")
            .bind(&artifact)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let opened = async {
            let mut file = archive_iso::regular(&runtime.directory.join(&relative))
                .map_err(|_| Error(ErrorCode::ArchiveArtifactUnavailable))?;
            require(
                file.metadata()
                    .await
                    .map_err(|_| Error(ErrorCode::ArchiveArtifactUnavailable))?
                    .len()
                    == length,
                ErrorCode::IntegrityFailure,
            )?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| Error(ErrorCode::ArchiveArtifactUnavailable))?;
            Ok::<_, Error>(file)
        }
        .await;
        let file = match opened {
            Ok(file) => file,
            Err(error) => {
                let mut tx = self.writer.transaction().await?;
                db::query("DELETE FROM archive_downloads WHERE id=$1")
                    .bind(&lease)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                return Err(error);
            }
        };
        Ok(ArchiveDownload {
            writer: self.writer.clone(),
            actor: actor.clone(),
            collection: collection.clone(),
            disc: disc.clone(),
            lease,
            file,
            position: start,
            hash: if start == 0 && end == length {
                Some(Sha256::new())
            } else {
                None
            },
            released: false,
            length,
            start,
            end,
            sha256: row.get::<String, _>("sha256").parse()?,
        })
    }
}
impl ArchiveDownload {
    async fn renew(&self) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        disc_access(&mut tx, &self.actor, &self.collection, &self.disc).await?;
        let n=db::query("UPDATE archive_downloads l SET expires_at=least(clock_timestamp()+interval '30 seconds',deadline) FROM archive_artifacts a WHERE l.id=$1 AND l.principal_id=$2 AND l.credential_id=$3 AND l.expires_at>clock_timestamp() AND l.deadline>clock_timestamp() AND a.id=l.artifact_id AND a.state='available' AND NOT a.cleanup_pending").bind(&self.lease).bind(self.actor.principal_id()).bind(&self.actor.credential).execute(&mut *tx).await?.rows_affected();
        require(n == 1, ErrorCode::ArchiveExpired)?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>> {
        if self.position == self.end {
            self.release().await?;
            return Ok(None);
        }
        self.renew().await?;
        let size = (self.end - self.position).min(1024 * 1024) as usize;
        let mut bytes = vec![0; size];
        self.file
            .read_exact(&mut bytes)
            .await
            .map_err(|_| Error(ErrorCode::IntegrityFailure))?;
        self.renew().await?;
        self.position += size as u64;
        if let Some(hash) = &mut self.hash {
            hash.update(&bytes);
            if self.position == self.end {
                require(
                    format!("{:x}", hash.clone().finalize()) == self.sha256.as_str(),
                    ErrorCode::IntegrityFailure,
                )?;
            }
        }
        Ok(Some(bytes))
    }
    pub async fn release(&mut self) -> Result<()> {
        if !self.released {
            let mut tx = self.writer.transaction().await?;
            db::query("DELETE FROM archive_downloads WHERE id=$1")
                .bind(&self.lease)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            self.released = true;
        }
        Ok(())
    }
}
impl Drop for ArchiveDownload {
    fn drop(&mut self) {
        if !self.released {
            let writer = self.writer.clone();
            let lease = self.lease.clone();
            if let Ok(runtime) = simple_server::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if let Ok(mut tx) = writer.transaction().await {
                        let _ = db::query("DELETE FROM archive_downloads WHERE id=$1")
                            .bind(lease)
                            .execute(&mut *tx)
                            .await;
                        let _ = tx.commit().await;
                    }
                });
            }
        }
    }
}

fn account_limits(row: &db::PgRow) -> Result<ArchiveAccountLimits> {
    Ok(ArchiveAccountLimits {
        principal_id: row.get::<String, _>("principal_id").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        quota_bytes: (row.get::<i64, _>("quota_bytes") as u64).try_into()?,
        pin_quota_bytes: (row.get::<i64, _>("pin_quota_bytes") as u64).try_into()?,
        max_active: row.get::<i32, _>("max_active") as u32,
        max_capture_records: (row.get::<i64, _>("max_capture_records") as u64).try_into()?,
        pin_deadline_seconds: row.get::<i32, _>("pin_deadline_seconds") as u32,
        cache_ttl_seconds: row.get::<i32, _>("cache_ttl_seconds") as u32,
    })
}
impl Writer {
    pub async fn archive_account_limits(
        &self,
        actor: &Actor,
        principal: &PrincipalId,
    ) -> Result<ArchiveAccountLimits> {
        let mut tx = self.transaction().await?;
        require(
            auth::administrator(&mut tx, actor).await?
                || actor.principal_id() == principal.as_str(),
            ErrorCode::Forbidden,
        )?;
        let exists: bool = db::query_scalar("SELECT EXISTS(SELECT 1 FROM principals WHERE id=$1)")
            .bind(principal.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(exists, ErrorCode::NotFound)?;
        db::query("INSERT INTO archive_accounts(principal_id) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(principal.as_str())
            .execute(&mut *tx)
            .await?;
        let row = db::query("SELECT * FROM archive_accounts WHERE principal_id=$1")
            .bind(principal.as_str())
            .fetch_one(&mut *tx)
            .await?;
        let result = account_limits(&row)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn update_archive_account_limits(
        &self,
        actor: &Actor,
        principal: &PrincipalId,
        request: &UpdateArchiveAccountLimitsRequest,
    ) -> Result<ArchiveAccountLimits> {
        require(
            (1..=64).contains(&request.max_active)
                && (1..=ARCHIVE_MAX_RECORDS).contains(&request.max_capture_records.get())
                && (60..=604800).contains(&request.pin_deadline_seconds)
                && (3600..=2592000).contains(&request.cache_ttl_seconds),
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row = db::query("SELECT * FROM archive_accounts WHERE principal_id=$1 FOR UPDATE")
            .bind(principal.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        require(
            row.get::<String, _>("revision") == request.expected_revision.as_str(),
            ErrorCode::RevisionConflict,
        )?;
        require(
            (row.get::<i64, _>("cache_bytes") as u64)
                + (row.get::<i64, _>("reserved_bytes") as u64)
                <= request.quota_bytes.get()
                && row.get::<i64, _>("pinned_bytes") as u64 <= request.pin_quota_bytes.get(),
            ErrorCode::ArchiveResourceLimit,
        )?;
        let active: i64 = db::query_scalar("SELECT count(*) FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE c.owner_id=$1 AND s.state IN ('capturing','planning','awaiting_confirmation','building')").bind(principal.as_str()).fetch_one(&mut *tx).await?;
        require(
            active <= request.max_active as i64,
            ErrorCode::ArchiveResourceLimit,
        )?;
        let row = db::query("UPDATE archive_accounts SET revision=$2,quota_bytes=$3,pin_quota_bytes=$4,max_active=$5,max_capture_records=$6,pin_deadline_seconds=$7,cache_ttl_seconds=$8 WHERE principal_id=$1 RETURNING *").bind(principal.as_str()).bind(id()).bind(request.quota_bytes.get() as i64).bind(request.pin_quota_bytes.get() as i64).bind(request.max_active as i32).bind(request.max_capture_records.get() as i64).bind(request.pin_deadline_seconds as i32).bind(request.cache_ttl_seconds as i32).fetch_one(&mut *tx).await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "archive.account.limits",
            serde_json::json!({"principal_id":principal}),
        )
        .await?;
        let result = account_limits(&row)?;
        tx.commit().await?;
        Ok(result)
    }
}
