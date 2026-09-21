//! Fenced optical exports; local staging and immutable images have separate lives.
use crate::{
    archive_iso::{self, InstalledArchiveBundle, IsoWriter},
    archive_management::source_access,
    archive_planning::{ArchiveIndexCursor, validate_reuse},
    auth::Actor,
    error::{Error, Result, id, require},
    payload::ObjectKey,
    roots::json,
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use serde::Deserialize;
use sqlx::{PgConnection, Row};
use std::{
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::AsyncWriteExt;
fn io(_: std::io::Error) -> Error {
    Error(ErrorCode::DestinationUnavailable)
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveExportConfig {
    pub directory: PathBuf,
    pub bundle_directory: PathBuf,
    pub quota_bytes: ByteCount,
}
pub(crate) struct ArchiveRuntime {
    pub directory: PathBuf,
    pub bundle: InstalledArchiveBundle,
    pub writer: IsoWriter,
    pub version: String,
    pub mutex: tokio::sync::Mutex<()>,
}
pub(crate) struct ExportFence {
    pub set: ArchiveSetId,
    pub epoch: i64,
}
const WRITER_OPTIONS: &[&str] = &[
    "mkisofs",
    "-iso-level",
    "3",
    "-r",
    "-uid",
    "0",
    "-gid",
    "0",
    "-V",
    "SCT_ARCHIVE_V1",
    "--modification-date=2026010100000000",
];
impl ArchiveRuntime {
    fn lock(&self) -> Result<std::fs::File> {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(self.directory.join(".writer-lock"))
            .map_err(io)?;
        rustix::fs::flock(&f, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Error(ErrorCode::ArchiveBusy))?;
        Ok(f)
    }
}
impl TransferEngine {
    pub async fn with_archive_exports(mut self, config: ArchiveExportConfig) -> Result<Self> {
        require(config.quota_bytes.get() > 0, ErrorCode::InvalidRequest)?;
        tokio::fs::create_dir_all(&config.directory)
            .await
            .map_err(io)?;
        require(
            !tokio::fs::symlink_metadata(&config.directory)
                .await
                .map_err(io)?
                .file_type()
                .is_symlink(),
            ErrorCode::InvalidPath,
        )?;
        tokio::fs::set_permissions(&config.directory, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(io)?;
        let directory = tokio::fs::canonicalize(&config.directory)
            .await
            .map_err(io)?;
        for name in ["work", "cache"] {
            let p = directory.join(name);
            match tokio::fs::symlink_metadata(&p).await {
                Ok(m) => require(
                    m.is_dir() && !m.file_type().is_symlink(),
                    ErrorCode::InvalidPath,
                )?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    archive_iso::private_directory(&p).await?
                }
                Err(e) => return Err(io(e)),
            }
        }
        let bundle = InstalledArchiveBundle::open(&config.bundle_directory).await?;
        let writer = IsoWriter::default();
        let version = writer.version().await?;
        let mut tx = self.writer.transaction().await?;
        sqlx::query("INSERT INTO archive_storage(singleton,directory,quota_bytes) VALUES(true,$1,$2) ON CONFLICT DO NOTHING").bind(directory.to_str().ok_or(Error(ErrorCode::InvalidPath))?).bind(config.quota_bytes.get() as i64).execute(&mut *tx).await?;
        let row = sqlx::query("SELECT * FROM archive_storage WHERE singleton FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
        require(
            row.get::<String, _>("directory")
                == directory.to_str().ok_or(Error(ErrorCode::InvalidPath))?,
            ErrorCode::RevisionConflict,
        )?;
        require(
            (row.get::<i64, _>("cache_bytes") + row.get::<i64, _>("reserved_bytes")) as u64
                <= config.quota_bytes.get(),
            ErrorCode::ArchiveResourceLimit,
        )?;
        sqlx::query("UPDATE archive_storage SET quota_bytes=$1 WHERE singleton")
            .bind(config.quota_bytes.get() as i64)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.archive = Some(Arc::new(ArchiveRuntime {
            directory,
            bundle,
            writer,
            version,
            mutex: tokio::sync::Mutex::new(()),
        }));
        Ok(self)
    }
    pub async fn archive_tick(&self) -> Result<()> {
        let Some(runtime) = &self.archive else {
            return Ok(());
        };
        let Ok(_guard) = runtime.mutex.try_lock() else {
            return Ok(());
        };
        let disk_lock = runtime.lock()?;
        let mut iso_writer = runtime.writer.clone();
        iso_writer.inherited_lock = Some(Arc::new(disk_lock.try_clone().map_err(io)?));
        let mut tx = self.writer.transaction().await?;
        let sets:Vec<String>=sqlx::query_scalar("SELECT s.id FROM archive_sets s LEFT JOIN archive_exports e ON e.set_id=s.id WHERE (s.state IN ('planning','building') OR (e.set_id IS NOT NULL AND NOT e.finished)) ORDER BY s.archive_run_after,s.id LIMIT 4").fetch_all(&mut *tx).await?;
        drop(tx);
        for set in sets {
            let set: ArchiveSetId = set.parse()?;
            let result = async {
                for _ in 0..32 {
                    if !self
                        .writer
                        .advance_archive_plan(&set, &runtime.bundle.manifest)
                        .await?
                    {
                        break;
                    }
                }
                let mut tx = self.writer.transaction().await?;
                let state: String =
                    sqlx::query_scalar("SELECT state FROM archive_sets WHERE id=$1")
                        .bind(set.as_str())
                        .fetch_one(&mut *tx)
                        .await?;
                drop(tx);
                if state == "building" {
                    self.export_one_disc(runtime, &set, &iso_writer).await?;
                }
                self.cleanup_archive_export(runtime, &set).await
            }
            .await;
            if let Err(error) = result {
                if error.0 == ErrorCode::ArchiveBusy {
                    continue;
                }
                let mut tx = self.writer.transaction().await?;
                let state = match error.0 {
                    ErrorCode::Forbidden | ErrorCode::Unauthorized | ErrorCode::NotFound => {
                        "revoked"
                    }
                    ErrorCode::ArchiveExpired => "expired",
                    _ => "failed",
                };
                sqlx::query("UPDATE archive_sets SET state=$4,failure=$2,revision=$3 WHERE id=$1 AND state IN ('planning','building')").bind(set.as_str()).bind(serde_json::to_value(&error.0)?.as_str().ok_or(Error(ErrorCode::InternalError))?).bind(id()).bind(state).execute(&mut *tx).await?;
                tx.commit().await?;
                self.cleanup_archive_export(runtime, &set).await?;
                self.writer.cleanup_archive_capture(&set).await?;
            }
            let mut tx = self.writer.transaction().await?;
            sqlx::query("UPDATE archive_sets SET archive_run_after=clock_timestamp() WHERE id=$1")
                .bind(set.as_str())
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(())
    }
    async fn reserve_export(
        &self,
        runtime: &ArchiveRuntime,
        set: &ArchiveSetId,
    ) -> Result<ExportFence> {
        require(
            self.writer.authorize_archive_work(set).await?,
            ErrorCode::Forbidden,
        )?;
        let mut tx = self.writer.transaction().await?;
        let owner:String=sqlx::query_scalar("SELECT c.owner_id FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE s.id=$1 FOR UPDATE OF c").bind(set.as_str()).fetch_one(&mut *tx).await?;
        let host = sqlx::query("SELECT * FROM archive_storage WHERE singleton FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
        let account =
            sqlx::query("SELECT * FROM archive_accounts WHERE principal_id=$1 FOR UPDATE")
                .bind(&owner)
                .fetch_one(&mut *tx)
                .await?;
        let row=sqlx::query("SELECT state,plan,expires_at>clock_timestamp() AS live FROM archive_sets WHERE id=$1 FOR UPDATE").bind(set.as_str()).fetch_one(&mut *tx).await?;
        require(
            row.get::<String, _>("state") == "building" && row.get::<bool, _>("live"),
            ErrorCode::ArchiveExpired,
        )?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM archive_exports WHERE set_id=$1)")
                .bind(set.as_str())
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            let plan: ArchivePlanSummary = serde_json::from_value(row.get("plan"))?;
            require(
                plan.bundle_sha256.as_str() == digest(&canonical(&runtime.bundle.manifest)?),
                ErrorCode::IntegrityFailure,
            )?;
            let largest: i64 = sqlx::query_scalar(
                "SELECT max(planned_iso_bytes) FROM archive_discs WHERE set_id=$1",
            )
            .bind(set.as_str())
            .fetch_one(&mut *tx)
            .await?;
            let reserve = plan
                .total_iso_budget_bytes
                .get()
                .checked_add(2 * largest as u64)
                .ok_or(Error(ErrorCode::ArchiveResourceLimit))?;
            for budget in [&host, &account] {
                require(
                    reserve
                        <= budget.get::<i64, _>("quota_bytes") as u64
                            - (budget.get::<i64, _>("cache_bytes")
                                + budget.get::<i64, _>("reserved_bytes"))
                                as u64,
                    ErrorCode::ArchiveResourceLimit,
                )?;
            }
            let fs = rustix::fs::statvfs(&runtime.directory)
                .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
            require(
                fs.f_bavail.saturating_mul(fs.f_frsize)
                    >= reserve + host.get::<i64, _>("reserved_bytes") as u64,
                ErrorCode::DestinationCapacity,
            )?;
            sqlx::query(
                "UPDATE archive_storage SET reserved_bytes=reserved_bytes+$1 WHERE singleton",
            )
            .bind(reserve as i64)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE archive_accounts SET reserved_bytes=reserved_bytes+$2 WHERE principal_id=$1").bind(owner).bind(reserve as i64).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO archive_exports(set_id,reserved_bytes,writer_version,writer_options) VALUES($1,$2,$3,$4)").bind(set.as_str()).bind(reserve as i64).bind(&runtime.version).bind(json(&WRITER_OPTIONS)?).execute(&mut *tx).await?;
        }
        let epoch:Option<i64>=sqlx::query_scalar("UPDATE archive_exports SET worker_epoch=worker_epoch+1,server_epoch=$2,worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE set_id=$1 AND NOT finished AND (worker_expires_at IS NULL OR worker_expires_at<=clock_timestamp() OR server_epoch<>$2) RETURNING worker_epoch").bind(set.as_str()).bind(self.writer.epoch).fetch_optional(&mut *tx).await?;
        let fence = ExportFence {
            set: set.clone(),
            epoch: epoch.ok_or(Error(ErrorCode::ArchiveBusy))?,
        };
        tx.commit().await?;
        Ok(fence)
    }
    async fn export_fence(&self, fence: &ExportFence) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        tx.commit().await?;
        Ok(())
    }
    async fn guarded<T, F: std::future::Future<Output = Result<T>>>(
        &self,
        fence: &ExportFence,
        work: F,
    ) -> Result<T> {
        tokio::pin!(work);
        loop {
            tokio::select! {result=&mut work=>{let value=result?;self.export_fence(fence).await?;return Ok(value);},_=tokio::time::sleep(Duration::from_secs(5))=>self.export_fence(fence).await?}
        }
    }
}
async fn check_fence(db: &mut PgConnection, fence: &ExportFence, server: i64) -> Result<()> {
    let row=sqlx::query("SELECT state,principal_id,credential_id,selections,expires_at>clock_timestamp() AS live FROM archive_sets WHERE id=$1 FOR UPDATE").bind(fence.set.as_str()).fetch_one(&mut *db).await?;
    require(
        row.get::<String, _>("state") == "building" && row.get::<bool, _>("live"),
        ErrorCode::ArchiveExpired,
    )?;
    let changed=sqlx::query("UPDATE archive_exports SET worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE set_id=$1 AND worker_epoch=$2 AND server_epoch=$3 AND NOT finished AND worker_expires_at>clock_timestamp()").bind(fence.set.as_str()).bind(fence.epoch).bind(server).execute(&mut *db).await?.rows_affected();
    require(changed == 1, ErrorCode::ArchiveBusy)?;
    let actor = Actor {
        principal: row.get("principal_id"),
        credential: row.get("credential_id"),
    };
    source_access(
        db,
        &actor,
        &serde_json::from_value::<Vec<ArchiveSelection>>(row.get("selections"))?,
    )
    .await?;
    validate_reuse(db, &actor, &fence.set).await
}

impl TransferEngine {
    async fn export_one_disc(
        &self,
        runtime: &ArchiveRuntime,
        set: &ArchiveSetId,
        iso_writer: &IsoWriter,
    ) -> Result<()> {
        let fence = self.reserve_export(runtime, set).await?;
        self.export_fence(&fence).await?;
        // A crash after publication may leave the previous staging tree. Remove
        // it before spending the single-disc staging reservation again.
        let mut tx = self.writer.transaction().await?;
        let leftovers=sqlx::query("SELECT b.disc_id,b.relative_path FROM archive_builds b JOIN archive_discs d ON d.id=b.disc_id JOIN archive_artifacts a ON a.id=b.artifact_id WHERE d.set_id=$1 AND a.state='available' AND b.relative_path<>'' LIMIT 128").bind(set.as_str()).fetch_all(&mut *tx).await?;
        drop(tx);
        for old in leftovers {
            let relative: String = old.get("relative_path");
            self.guarded(&fence, remove_work(&work_path(runtime, &relative)?))
                .await?;
            let mut tx = self.writer.transaction().await?;
            check_fence(&mut tx, &fence, self.writer.epoch).await?;
            sqlx::query("UPDATE archive_builds SET relative_path='' WHERE disc_id=$1")
                .bind(old.get::<String, _>("disc_id"))
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }

        let mut tx = self.writer.transaction().await?;
        let row=sqlx::query("SELECT d.*,a.id AS artifact_id,a.sha256 AS artifact_hash,a.length AS artifact_length,b.relative_path AS work_path FROM archive_discs d LEFT JOIN archive_artifacts a ON a.disc_id=d.id LEFT JOIN archive_builds b ON b.disc_id=d.id WHERE d.set_id=$1 AND (a.id IS NULL OR a.state<>'available') ORDER BY d.number LIMIT 1").bind(set.as_str()).fetch_optional(&mut *tx).await?;
        drop(tx);
        let Some(row) = row else {
            let mut tx = self.writer.transaction().await?;
            check_fence(&mut tx, &fence, self.writer.epoch).await?;
            sqlx::query("UPDATE archive_sets SET state='ready',published_at=clock_timestamp(),revision=$2 WHERE id=$1").bind(set.as_str()).bind(id()).execute(&mut *tx).await?;
            sqlx::query("UPDATE archive_exports SET worker_expires_at=NULL WHERE set_id=$1")
                .bind(set.as_str())
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(());
        };
        let disc: ArchiveDiscId = row.get::<String, _>("id").parse()?;
        let artifact = row
            .get::<Option<String>, _>("artifact_id")
            .unwrap_or_else(id);
        let cache = runtime.directory.join(format!("cache/{artifact}.iso"));
        if let Some(hash) = row.get::<Option<String>, _>("artifact_hash") {
            let expected = row.get::<i64, _>("artifact_length") as u64;
            let work = row
                .get::<Option<String>, _>("work_path")
                .ok_or(Error(ErrorCode::IntegrityFailure))?;
            let candidate = if cache.try_exists().map_err(io)? {
                cache.clone()
            } else {
                work_path(runtime, &work)?.join("image.iso")
            };
            let (actual, length) = self
                .guarded(&fence, archive_iso::file_hash(&candidate, expected))
                .await?;
            require(
                actual.as_str() == hash && length == expected,
                ErrorCode::IntegrityFailure,
            )?;
            self.publish_image(
                runtime,
                &fence,
                VerifiedImage {
                    artifact: &artifact,
                    disc: &disc,
                    candidate: &candidate,
                    cache: &cache,
                    hash: &actual,
                    length,
                },
            )
            .await?;
            return Ok(());
        }
        if let Some(old) = row.get::<Option<String>, _>("work_path")
            && !old.is_empty()
        {
            remove_work(&work_path(runtime, &old)?).await?;
        }
        let attempt = id();
        let relative = format!("work/{attempt}");
        let work = work_path(runtime, &relative)?;
        let mut tx = self.writer.transaction().await?;
        check_fence(&mut tx, &fence, self.writer.epoch).await?;
        sqlx::query("INSERT INTO archive_artifacts(id,disc_id,state,expires_at) VALUES($1,$2,'building',clock_timestamp()+interval '7 days') ON CONFLICT(disc_id) DO NOTHING").bind(&artifact).bind(disc.as_str()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO archive_builds(disc_id,artifact_id,attempt,stage,relative_path,server_epoch,worker_epoch) VALUES($1,$2,$3,'staging',$4,$5,$6) ON CONFLICT(disc_id) DO UPDATE SET attempt=$3,stage='staging',relative_path=$4,server_epoch=$5,worker_epoch=$6,updated_at=clock_timestamp()").bind(disc.as_str()).bind(&artifact).bind(&attempt).bind(&relative).bind(self.writer.epoch).bind(fence.epoch).execute(&mut *tx).await?;
        tx.commit().await?;
        archive_iso::private_directory(&work).await?;
        let tree = work.join("tree");
        archive_iso::private_directory(&tree).await?;
        let descriptor = self.writer.archive_disc_descriptor(set, &disc).await?;
        self.guarded(&fence, runtime.bundle.copy_to(&tree)).await?;
        archive_iso::write_file(&tree.join("disc.json"), &canonical(&descriptor)?).await?;
        let mut index = archive_iso::new_file(&tree.join("snapshot.jsonl")).await?;
        let mut cursor = ArchiveIndexCursor::default();
        loop {
            self.export_fence(&fence).await?;
            let page = self.writer.archive_index_page(set, &cursor).await?;
            if page.is_empty() {
                break;
            }
            for (next, line) in page {
                index.write_all(&line).await.map_err(io)?;
                cursor = next;
            }
        }
        index.sync_all().await.map_err(io)?;
        drop(index);
        self.stage_objects(&fence, &disc, &tree).await?;
        let budget = row.get::<i64, _>("planned_iso_bytes") as u64;
        let image = work.join("image.iso");
        self.guarded(&fence, iso_writer.write(&tree, &image, budget))
            .await?;
        let extracted = work.join("verified");
        self.guarded(&fence, iso_writer.extract(&image, &extracted, budget))
            .await?;
        self.verify_image_tree(
            runtime,
            &fence,
            &disc,
            &descriptor,
            &extracted,
            row.get::<i64, _>("object_count") as u64,
        )
        .await?;
        let (hash, length) = self
            .guarded(&fence, archive_iso::file_hash(&image, budget))
            .await?;
        require(
            length > 0 && length <= budget,
            ErrorCode::ArchiveMetadataTooLarge,
        )?;
        sync_directory(&work).await?;
        let mut tx = self.writer.transaction().await?;
        check_fence(&mut tx, &fence, self.writer.epoch).await?;
        sqlx::query(
            "UPDATE archive_artifacts SET sha256=$2,length=$3 WHERE id=$1 AND state='building'",
        )
        .bind(&artifact)
        .bind(hash.as_str())
        .bind(length as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE archive_builds SET stage='verified',updated_at=clock_timestamp() WHERE disc_id=$1").bind(disc.as_str()).execute(&mut *tx).await?;
        tx.commit().await?;
        self.publish_image(
            runtime,
            &fence,
            VerifiedImage {
                artifact: &artifact,
                disc: &disc,
                candidate: &image,
                cache: &cache,
                hash: &hash,
                length,
            },
        )
        .await
    }
    async fn stage_objects(
        &self,
        fence: &ExportFence,
        disc: &ArchiveDiscId,
        tree: &Path,
    ) -> Result<()> {
        let mut cursor = (String::new(), String::new());
        loop {
            self.export_fence(fence).await?;
            let objects = self.disc_objects(&fence.set, disc, &cursor).await?;
            if objects.is_empty() {
                break;
            }
            for object in objects {
                let bytes = self
                    .guarded(fence, self.archive_object_bytes(&object))
                    .await?;
                let path = tree.join(object.relative_path());
                tokio::fs::create_dir_all(path.parent().ok_or(Error(ErrorCode::InternalError))?)
                    .await
                    .map_err(io)?;
                archive_iso::write_file(&path, &bytes).await?;
                cursor = (
                    object.root_id.as_str().into(),
                    object.digest.as_str().into(),
                );
            }
        }
        Ok(())
    }
    async fn disc_objects(
        &self,
        set: &ArchiveSetId,
        disc: &ArchiveDiscId,
        cursor: &(String, String),
    ) -> Result<Vec<ArchiveObject>> {
        let mut tx = self.writer.transaction().await?;
        let rows=sqlx::query("SELECT root_id,digest,length FROM archive_placements WHERE set_id=$1 AND disc_id=$2 AND (root_id COLLATE \"C\",digest COLLATE \"C\")>($3 COLLATE \"C\",$4 COLLATE \"C\") ORDER BY root_id COLLATE \"C\",digest COLLATE \"C\" LIMIT 128").bind(set.as_str()).bind(disc.as_str()).bind(&cursor.0).bind(&cursor.1).fetch_all(&mut *tx).await?;
        rows.into_iter()
            .map(|r| {
                Ok(ArchiveObject {
                    root_id: r.get::<String, _>("root_id").parse()?,
                    digest: r.get::<String, _>("digest").parse()?,
                    length: (r.get::<i64, _>("length") as u64).try_into()?,
                })
            })
            .collect()
    }
    async fn archive_object_bytes(&self, object: &ArchiveObject) -> Result<Vec<u8>> {
        let mut tx = self.writer.transaction().await?;
        let placement: serde_json::Value =
            sqlx::query_scalar("SELECT placement FROM roots WHERE id=$1")
                .bind(object.root_id.as_str())
                .fetch_one(&mut *tx)
                .await?;
        drop(tx);
        let policy = match serde_json::from_value::<Placement>(placement)? {
            Placement::Single { read_policy, .. } | Placement::Mirror { read_policy, .. } => {
                read_policy
            }
        };
        let deadline = policy
            .overall_deadline_ms
            .map(|n| tokio::time::Instant::now() + Duration::from_millis(n.get()));
        let mut failure = Error(ErrorCode::DestinationUnavailable);
        for candidate in &policy.candidates {
            let result=async {
                let mut tx=self.writer.transaction().await?;
                let generation:Option<String>=sqlx::query_scalar("SELECT generation FROM object_copies WHERE root_id=$1 AND digest=$2 AND backend_id=$3 AND state='available'").bind(object.root_id.as_str()).bind(object.digest.as_str()).bind(candidate.backend_id.as_str()).fetch_optional(&mut *tx).await?;drop(tx);
                let key=ObjectKey {root:object.root_id.clone(),digest:object.digest.clone(),generation:generation.ok_or(Error(ErrorCode::DestinationUnavailable))?.parse()?};let store=self.store(candidate.backend_id.as_str())?;
                let mut initial=Duration::from_millis(candidate.connection_timeout_ms.get()+candidate.initial_response_timeout_ms.get());if let Some(end)=deadline {initial=initial.min(end.saturating_duration_since(tokio::time::Instant::now()));}
                let reader=tokio::time::timeout(initial,store.read_with_policy(&key,candidate)).await.map_err(|_|Error(ErrorCode::DestinationUnavailable))??;
                let bytes=crate::download::read_progress(reader,object.length.get(),candidate,deadline).await?;require(bytes.len() as u64==object.length.get() && digest(&bytes)==object.digest.as_str(),ErrorCode::IntegrityFailure)?;Ok(bytes)
            }.await;
            match result {
                Ok(bytes) => return Ok(bytes),
                Err(e) => failure = e,
            }
        }
        Err(failure)
    }
    async fn verify_image_tree(
        &self,
        runtime: &ArchiveRuntime,
        fence: &ExportFence,
        disc: &ArchiveDiscId,
        descriptor: &ArchiveDiscDescriptor,
        tree: &Path,
        count: u64,
    ) -> Result<()> {
        let expected = canonical(descriptor)?;
        let (hash, length) =
            archive_iso::file_hash(&tree.join("disc.json"), expected.len() as u64).await?;
        require(
            hash.as_str() == digest(&expected) && length == expected.len() as u64,
            ErrorCode::IntegrityFailure,
        )?;
        let (hash, length) = self
            .guarded(
                fence,
                archive_iso::file_hash(&tree.join("snapshot.jsonl"), descriptor.index_bytes),
            )
            .await?;
        require(
            hash == descriptor.index_sha256 && length == descriptor.index_bytes,
            ErrorCode::IntegrityFailure,
        )?;
        let manifest = canonical(&runtime.bundle.manifest)?;
        let (hash, length) =
            archive_iso::file_hash(&tree.join("recovery/bundle.json"), manifest.len() as u64)
                .await?;
        require(
            hash == descriptor.bundle_sha256 && length == manifest.len() as u64,
            ErrorCode::IntegrityFailure,
        )?;
        for file in &runtime.bundle.manifest.files {
            let (hash, length) = self
                .guarded(
                    fence,
                    archive_iso::file_hash(&tree.join(&file.path), file.length.get()),
                )
                .await?;
            require(
                hash == file.sha256 && length == file.length.get(),
                ErrorCode::IntegrityFailure,
            )?;
        }
        require(
            tokio::fs::metadata(tree.join("recovery/sct-recover"))
                .await
                .map_err(io)?
                .permissions()
                .mode()
                & 0o111
                != 0,
            ErrorCode::IntegrityFailure,
        )?;
        let mut cursor = (String::new(), String::new());
        let mut verified = 0;
        loop {
            self.export_fence(fence).await?;
            let objects = self.disc_objects(&fence.set, disc, &cursor).await?;
            if objects.is_empty() {
                break;
            }
            for object in objects {
                let (hash, length) = self
                    .guarded(
                        fence,
                        archive_iso::file_hash(
                            &tree.join(object.relative_path()),
                            object.length.get(),
                        ),
                    )
                    .await?;
                require(
                    hash == object.digest && length == object.length.get(),
                    ErrorCode::IntegrityFailure,
                )?;
                verified += 1;
                cursor = (
                    object.root_id.as_str().into(),
                    object.digest.as_str().into(),
                );
            }
        }
        require(verified == count, ErrorCode::IntegrityFailure)?;
        let expected_files = count + runtime.bundle.manifest.files.len() as u64 + 3;
        let actual = self
            .guarded(fence, count_regular_files(tree, expected_files))
            .await?;
        require(actual == expected_files, ErrorCode::IntegrityFailure)
    }
    async fn publish_image(
        &self,
        runtime: &ArchiveRuntime,
        fence: &ExportFence,
        image: VerifiedImage<'_>,
    ) -> Result<()> {
        let VerifiedImage {
            artifact,
            disc,
            candidate,
            cache,
            hash,
            length,
        } = image;
        self.export_fence(fence).await?;
        if candidate != cache {
            match tokio::fs::hard_link(candidate, cache).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let (actual, size) = self
                        .guarded(fence, archive_iso::file_hash(cache, length))
                        .await?;
                    require(
                        actual == *hash && size == length,
                        ErrorCode::IntegrityFailure,
                    )?;
                }
                Err(e) => return Err(io(e)),
            }
        }
        sync_directory(&runtime.directory.join("cache")).await?;
        let mut tx = self.writer.transaction().await?;
        let owner:String=sqlx::query_scalar("SELECT c.owner_id FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE s.id=$1 FOR UPDATE OF c").bind(fence.set.as_str()).fetch_one(&mut *tx).await?;
        sqlx::query("SELECT singleton FROM archive_storage WHERE singleton FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("SELECT principal_id FROM archive_accounts WHERE principal_id=$1 FOR UPDATE")
            .bind(&owner)
            .fetch_one(&mut *tx)
            .await?;
        check_fence(&mut tx, fence, self.writer.epoch).await?;
        let changed=sqlx::query("UPDATE archive_artifacts SET state='available',relative_path=$2 WHERE id=$1 AND state='building' AND sha256=$3 AND length=$4").bind(artifact).bind(format!("cache/{artifact}.iso")).bind(hash.as_str()).bind(length as i64).execute(&mut *tx).await?.rows_affected();
        require(changed == 1, ErrorCode::RevisionConflict)?;
        sqlx::query("UPDATE archive_storage SET reserved_bytes=reserved_bytes-$1,cache_bytes=cache_bytes+$1 WHERE singleton").bind(length as i64).execute(&mut *tx).await?;
        sqlx::query("UPDATE archive_accounts SET reserved_bytes=reserved_bytes-$2,cache_bytes=cache_bytes+$2 WHERE principal_id=$1").bind(owner).bind(length as i64).execute(&mut *tx).await?;
        sqlx::query("UPDATE archive_exports SET reserved_bytes=reserved_bytes-$2,worker_expires_at=NULL WHERE set_id=$1").bind(fence.set.as_str()).bind(length as i64).execute(&mut *tx).await?;
        sqlx::query("UPDATE archive_builds SET stage='available',updated_at=clock_timestamp() WHERE disc_id=$1").bind(disc.as_str()).execute(&mut *tx).await?;
        tx.commit().await?;
        // Only one full staging/extraction tree is charged at a time.
        let mut tx = self.writer.transaction().await?;
        let relative: String =
            sqlx::query_scalar("SELECT relative_path FROM archive_builds WHERE disc_id=$1")
                .bind(disc.as_str())
                .fetch_one(&mut *tx)
                .await?;
        drop(tx);
        if !relative.is_empty() {
            remove_work(&work_path(runtime, &relative)?).await?;
            let mut tx = self.writer.transaction().await?;
            sqlx::query("UPDATE archive_builds SET relative_path='' WHERE disc_id=$1")
                .bind(disc.as_str())
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(())
    }
}
fn work_path(runtime: &ArchiveRuntime, relative: &str) -> Result<PathBuf> {
    let id = relative
        .strip_prefix("work/")
        .ok_or(Error(ErrorCode::InvalidPath))?;
    require(
        id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        ErrorCode::InvalidPath,
    )?;
    Ok(runtime.directory.join(relative))
}
async fn remove_work(path: &Path) -> Result<()> {
    // Rock Ridge restores read-only directory modes. Make owned staging
    // directories removable without following a symlink outside this attempt.
    let mut stack = vec![path.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let metadata = match tokio::fs::symlink_metadata(&directory).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(e)),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        tokio::fs::set_permissions(
            &directory,
            std::fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
        )
        .await
        .map_err(io)?;
        let mut entries = tokio::fs::read_dir(directory).await.map_err(io)?;
        while let Some(entry) = entries.next_entry().await.map_err(io)? {
            if entry.file_type().await.map_err(io)?.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
    }
}
async fn sync_directory(path: &Path) -> Result<()> {
    tokio::fs::File::open(path)
        .await
        .map_err(io)?
        .sync_all()
        .await
        .map_err(io)
}
async fn count_regular_files(root: &Path, maximum: u64) -> Result<u64> {
    let mut stack = vec![root.to_path_buf()];
    let mut count = 0;
    let mut directories = 0;
    while let Some(path) = stack.pop() {
        let mut entries = tokio::fs::read_dir(path).await.map_err(io)?;
        while let Some(entry) = entries.next_entry().await.map_err(io)? {
            let kind = entry.file_type().await.map_err(io)?;
            if kind.is_dir() {
                directories += 1;
                require(
                    directories <= ARCHIVE_MAX_SOURCES as u64 + 8195,
                    ErrorCode::IntegrityFailure,
                )?;
                stack.push(entry.path());
            } else {
                require(kind.is_file(), ErrorCode::IntegrityFailure)?;
                count += 1;
                require(count <= maximum, ErrorCode::IntegrityFailure)?;
            }
        }
    }
    Ok(count)
}

impl TransferEngine {
    async fn cleanup_archive_export(
        &self,
        runtime: &ArchiveRuntime,
        set: &ArchiveSetId,
    ) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        let state: String = sqlx::query_scalar("SELECT state FROM archive_sets WHERE id=$1")
            .bind(set.as_str())
            .fetch_one(&mut *tx)
            .await?;
        if matches!(
            state.as_str(),
            "capturing" | "planning" | "awaiting_confirmation" | "building"
        ) {
            return Ok(());
        }
        let export=sqlx::query("UPDATE archive_exports SET worker_epoch=worker_epoch+1,worker_expires_at=NULL WHERE set_id=$1 AND NOT finished RETURNING set_id").bind(set.as_str()).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        if export.is_none() {
            self.writer.cleanup_archive_capture(set).await?;
            return Ok(());
        }
        let ready = state == "ready";
        let mut tx = self.writer.transaction().await?;
        let rows=sqlx::query("SELECT b.disc_id,b.relative_path,b.artifact_id,a.state FROM archive_builds b JOIN archive_discs d ON d.id=b.disc_id JOIN archive_artifacts a ON a.id=b.artifact_id WHERE d.set_id=$1 AND (b.relative_path<>'' OR (NOT $2 AND a.state IN ('building','available'))) ORDER BY d.number LIMIT 128").bind(set.as_str()).bind(ready).fetch_all(&mut *tx).await?;
        drop(tx);
        for row in rows {
            let relative: String = row.get("relative_path");
            if !relative.is_empty() {
                remove_work(&work_path(runtime, &relative)?).await?;
            }
            let artifact: String = row.get("artifact_id");
            require(
                artifact.len() == 32 && artifact.bytes().all(|b| b.is_ascii_hexdigit()),
                ErrorCode::InvalidPath,
            )?;
            if !ready {
                match tokio::fs::remove_file(
                    runtime.directory.join(format!("cache/{artifact}.iso")),
                )
                .await
                {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io(e)),
                }
            }
            let mut tx = self.writer.transaction().await?;
            let owner:String=sqlx::query_scalar("SELECT c.owner_id FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE s.id=$1 FOR UPDATE OF c").bind(set.as_str()).fetch_one(&mut *tx).await?;
            sqlx::query("SELECT singleton FROM archive_storage WHERE singleton FOR UPDATE")
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query(
                "SELECT principal_id FROM archive_accounts WHERE principal_id=$1 FOR UPDATE",
            )
            .bind(&owner)
            .fetch_one(&mut *tx)
            .await?;
            if !ready {
                let released:Option<i64>=sqlx::query_scalar("UPDATE archive_artifacts SET state='expired',relative_path=NULL WHERE id=$1 AND state='available' RETURNING length").bind(&artifact).fetch_optional(&mut *tx).await?;
                if let Some(bytes) = released {
                    sqlx::query(
                        "UPDATE archive_storage SET cache_bytes=cache_bytes-$1 WHERE singleton",
                    )
                    .bind(bytes)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query("UPDATE archive_accounts SET cache_bytes=cache_bytes-$2 WHERE principal_id=$1").bind(&owner).bind(bytes).execute(&mut *tx).await?;
                }
                sqlx::query("UPDATE archive_artifacts SET state='failed',relative_path=NULL WHERE id=$1 AND state='building'").bind(&artifact).execute(&mut *tx).await?;
            }
            sqlx::query("UPDATE archive_builds SET relative_path='',stage=CASE WHEN $2 THEN stage ELSE 'failed' END WHERE disc_id=$1").bind(row.get::<String,_>("disc_id")).bind(ready).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        let mut tx = self.writer.transaction().await?;
        let owner:String=sqlx::query_scalar("SELECT c.owner_id FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE s.id=$1 FOR UPDATE OF c").bind(set.as_str()).fetch_one(&mut *tx).await?;
        sqlx::query("SELECT singleton FROM archive_storage WHERE singleton FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("SELECT principal_id FROM archive_accounts WHERE principal_id=$1 FOR UPDATE")
            .bind(&owner)
            .fetch_one(&mut *tx)
            .await?;
        let remaining:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM archive_builds b JOIN archive_discs d ON d.id=b.disc_id JOIN archive_artifacts a ON a.id=b.artifact_id WHERE d.set_id=$1 AND (b.relative_path<>'' OR (NOT $2 AND a.state IN ('building','available'))))").bind(set.as_str()).bind(ready).fetch_one(&mut *tx).await?;
        if remaining {
            return Ok(());
        }
        let released:Option<i64>=sqlx::query_scalar("SELECT reserved_bytes FROM archive_exports WHERE set_id=$1 AND NOT finished FOR UPDATE").bind(set.as_str()).fetch_optional(&mut *tx).await?;
        if let Some(bytes) = released {
            sqlx::query(
                "UPDATE archive_storage SET reserved_bytes=reserved_bytes-$1 WHERE singleton",
            )
            .bind(bytes)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE archive_accounts SET reserved_bytes=reserved_bytes-$2 WHERE principal_id=$1").bind(owner).bind(bytes).execute(&mut *tx).await?;
            sqlx::query(
                "UPDATE archive_exports SET reserved_bytes=0,finished=true WHERE set_id=$1",
            )
            .bind(set.as_str())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.writer.cleanup_archive_capture(set).await?;
        Ok(())
    }
}

struct VerifiedImage<'a> {
    artifact: &'a str,
    disc: &'a ArchiveDiscId,
    candidate: &'a Path,
    cache: &'a Path,
    hash: &'a ChunkDigest,
    length: u64,
}
