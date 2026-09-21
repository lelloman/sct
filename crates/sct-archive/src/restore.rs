use crate::*;
use sqlx::Row;
use std::io::Seek;

#[derive(Debug, Default, Serialize)]
pub struct RestoreSummary {
    pub complete: bool,
    pub restored_files: u64,
    pub existing_files: u64,
    pub unavailable_files: u64,
    pub conflicting_files: u64,
}
fn object(row: &sqlx::sqlite::SqliteRow) -> Result<ArchiveObject> {
    Ok(ArchiveObject {
        root_id: row.get::<String, _>(0).parse()?,
        digest: row.get::<String, _>(1).parse()?,
        length: (row.get::<i64, _>(2) as u64).try_into()?,
    })
}
fn available(dir: &SafeDir, object: &ArchiveObject) -> bool {
    dir.file(&object.relative_path())
        .and_then(|f| hash_reader(f, object.length.get()))
        .is_ok_and(|hash| hash == object.digest.as_str())
}
fn emit(out: &mut impl Write, value: serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *out, &value)?;
    out.write_all(b"\n")?;
    Ok(())
}
impl ArchiveIndex {
    /// Cache only authenticated target objects. The cache must be private and dedicated
    /// to this work index. Re-reading the bytes prevents stale availability flags.
    pub async fn ingest(
        &mut self,
        media: &Path,
        cache: &Path,
        report: &mut impl Write,
    ) -> Result<VerificationSummary> {
        let summary = self.verify_media(media, report).await?;
        let source = SafeDir::open(media)?;
        let destination = SafeDir::open(cache)?;
        let desc = descriptor(&source)?;
        let mut after = (String::new(), String::new());
        loop {
            let rows = sqlx::query("SELECT root,digest,length FROM placements WHERE disc=? AND (root,digest)>(?,?) ORDER BY root,digest LIMIT 128")
                .bind(&desc.disc_id).bind(&after.0).bind(&after.1).fetch_all(&mut self.connection).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let object = object(&row)?;
                after = (
                    object.root_id.as_str().to_owned(),
                    object.digest.as_str().to_owned(),
                );
                if available(&destination, &object) {
                    continue;
                }
                let Ok(mut input) = source.file(&object.relative_path()) else {
                    continue;
                };
                // A chunk is at most CHUNK_MAX bytes. Validate before any cache write.
                let mut bytes = Vec::new();
                (&mut input)
                    .take(object.length.get() + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 != object.length.get()
                    || digest(&bytes) != object.digest.as_str()
                {
                    continue;
                }
                let mut pending = destination.pending(&object.relative_path())?;
                pending.file.write_all(&bytes)?;
                destination.remove_cached_file(&object.relative_path())?;
                pending.publish()?;
            }
        }
        Ok(summary)
    }

    /// By default nothing is written until all objects are available. Partial mode
    /// emits complete files only, with exact missing extents streamed in JSONL.
    pub async fn restore_to(
        &mut self,
        cache: &Path,
        destination: &Path,
        partial: bool,
        report: &mut impl Write,
    ) -> Result<RestoreSummary> {
        let cache = SafeDir::open(cache)?;
        sqlx::raw_sql("CREATE TEMP TABLE IF NOT EXISTS available(root TEXT,digest TEXT,PRIMARY KEY(root,digest)); DELETE FROM available;")
            .execute(&mut self.connection).await?;
        let mut after = (String::new(), String::new());
        loop {
            let rows = sqlx::query("SELECT root,digest,length FROM objects WHERE (root,digest)>(?,?) ORDER BY root,digest LIMIT 128")
                .bind(&after.0).bind(&after.1).fetch_all(&mut self.connection).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let object = object(&row)?;
                after = (
                    object.root_id.as_str().to_owned(),
                    object.digest.as_str().to_owned(),
                );
                if available(&cache, &object) {
                    sqlx::query("INSERT INTO available VALUES(?,?)")
                        .bind(&after.0)
                        .bind(&after.1)
                        .execute(&mut self.connection)
                        .await?;
                }
            }
        }
        let missing: i64 = sqlx::query_scalar("SELECT count(*) FROM objects o WHERE NOT EXISTS(SELECT 1 FROM available a WHERE a.root=o.root AND a.digest=o.digest)")
            .fetch_one(&mut self.connection).await?;
        let output = if partial || missing == 0 {
            Some(SafeDir::open(destination)?)
        } else {
            None
        };
        let mut summary = RestoreSummary::default();
        let mut path = String::new();
        loop {
            let entries = sqlx::query(
                "SELECT path,kind,length FROM entries WHERE path>? ORDER BY path LIMIT 128",
            )
            .bind(&path)
            .fetch_all(&mut self.connection)
            .await?;
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                path = entry.get(0);
                let relative = path.trim_start_matches('/');
                if entry.get::<String, _>(1) == "directory" {
                    if let Some(output) = &output {
                        output.directory(relative)?;
                    }
                    continue;
                }
                let mut offset = -1i64;
                let mut unavailable = false;
                loop {
                    let rows = sqlx::query("SELECT root,digest,length,offset FROM extents e WHERE path=? AND offset>? AND NOT EXISTS(SELECT 1 FROM available a WHERE a.root=e.root AND a.digest=e.digest) ORDER BY offset LIMIT 128")
                        .bind(&path).bind(offset).fetch_all(&mut self.connection).await?;
                    if rows.is_empty() {
                        break;
                    }
                    unavailable = true;
                    for row in rows {
                        offset = row.get(3);
                        emit(
                            report,
                            serde_json::json!({"state":"missing_extent","path":path,"offset":offset,"object":object(&row)?}),
                        )?;
                    }
                }
                if unavailable {
                    summary.unavailable_files += 1;
                    continue;
                }
                let Some(output) = &output else {
                    continue;
                };
                let existing = output.file(relative);
                let absent = existing.as_ref().err().is_some_and(|e| {
                    e.downcast_ref::<rustix::io::Errno>() == Some(&rustix::io::Errno::NOENT)
                });
                if !absent && existing.is_err() {
                    summary.conflicting_files += 1;
                    emit(report, serde_json::json!({"state":"conflict","path":path}))?;
                    continue;
                }
                let mut existing = existing.ok();
                let mut pending = if absent {
                    Some(output.pending(relative)?)
                } else {
                    None
                };
                let mut good = true;
                let mut offset = -1i64;
                loop {
                    let rows = sqlx::query("SELECT root,digest,length,offset FROM extents WHERE path=? AND offset>? ORDER BY offset LIMIT 128")
                        .bind(&path).bind(offset).fetch_all(&mut self.connection).await?;
                    if rows.is_empty() {
                        break;
                    }
                    for row in rows {
                        let object = object(&row)?;
                        offset = row.get(3);
                        if let Some(file) = &mut existing {
                            good &=
                                hash_reader(file.take(object.length.get()), object.length.get())
                                    .is_ok_and(|hash| hash == object.digest.as_str());
                        } else if let Some(pending) = &mut pending {
                            let mut source = cache.file(&object.relative_path())?;
                            ensure(
                                hash_reader(&mut source, object.length.get())?
                                    == object.digest.as_str(),
                                "cache changed during restore",
                            )?;
                            source.rewind()?;
                            // Hash the actual bytes that are written too, closing the re-read race.
                            let mut bytes = Vec::new();
                            source
                                .take(object.length.get() + 1)
                                .read_to_end(&mut bytes)?;
                            ensure(
                                bytes.len() as u64 == object.length.get()
                                    && digest(&bytes) == object.digest.as_str(),
                                "cache changed during restore",
                            )?;
                            pending.file.write_all(&bytes)?;
                        }
                    }
                }
                if let Some(mut file) = existing {
                    let mut extra = [0];
                    good &= file.read(&mut extra)? == 0;
                    if good {
                        summary.existing_files += 1;
                    } else {
                        summary.conflicting_files += 1;
                    }
                } else if let Some(pending) = pending {
                    pending.publish()?;
                    summary.restored_files += 1;
                }
                emit(
                    report,
                    serde_json::json!({"path":path,"state":if good {"restored"} else {"conflict"}}),
                )?;
            }
        }
        summary.complete = missing == 0 && summary.conflicting_files == 0;
        emit(report, serde_json::json!({"summary":summary}))?;
        Ok(summary)
    }
}
