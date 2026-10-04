//! Offline, append-only metadata catalog. No accounts or physical-copy claims.
use crate::sqlite_connection::connect_archive;
use crate::*;
use sqlx::{Connection, Row, SqliteConnection};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader},
    os::unix::fs::OpenOptionsExt,
};

pub struct ArchiveCatalog {
    connection: SqliteConnection,
    _lock: std::fs::File,
}
fn emit(output: &mut impl Write, value: &serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}
impl ArchiveCatalog {
    pub async fn open(database: &Path) -> Result<Self> {
        let (lock, new) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(database)
        {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(
                        rustix::fs::OFlags::NOFOLLOW.bits() as i32
                            | rustix::fs::OFlags::NONBLOCK.bits() as i32,
                    )
                    .open(database)?,
                false,
            ),
            Err(e) => return Err(e.into()),
        };
        ensure(
            lock.metadata()?.is_file(),
            "catalog must be a regular private file",
        )?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
        let mut connection = connect_archive(database).await?;
        if new {
            let mut tx = connection.begin().await?;
            for statement in crate::schema::catalog_header_plan()?.statements {
                sqlx::raw_sql(&statement).execute(&mut *tx).await?;
            }
            sqlx::query("INSERT INTO catalog_meta VALUES(1)")
                .execute(&mut *tx)
                .await?;
            for statement in crate::schema::catalog_body_plan()?.statements {
                sqlx::raw_sql(&statement).execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        let format: i64 = sqlx::query_scalar("SELECT format FROM catalog_meta")
            .fetch_one(&mut connection)
            .await?;
        ensure(format == 1, "unsupported offline catalog format")?;
        Ok(Self {
            connection,
            _lock: lock,
        })
    }
    /// Requires a validated original index. Rechecks the scanned metadata while
    /// copying so media changes cannot commit an unvalidated catalog.
    pub async fn scan(&mut self, media: &Path, index: &ArchiveIndex) -> Result<()> {
        let dir = SafeDir::open(media)?;
        let desc = descriptor(&dir)?;
        ensure(
            desc.collection_id == index.descriptor.collection_id
                && desc.set_id == index.descriptor.set_id
                && desc.set_sequence == index.descriptor.set_sequence
                && desc.index_sha256 == index.descriptor.index_sha256
                && desc.bundle_sha256 == index.descriptor.bundle_sha256
                && desc.first_disc == index.descriptor.first_disc
                && desc.last_disc == index.descriptor.last_disc
                && desc.index_bytes == index.descriptor.index_bytes
                && desc.index_records == index.descriptor.index_records,
            "catalog scan requires the disc's original validated index",
        )?;
        verify_bundle(&dir, &desc)?;
        let mut tx = self.connection.begin().await?;
        let old =
            sqlx::query("SELECT * FROM catalog_sets WHERE collection=? AND (id=? OR sequence=?)")
                .bind(&desc.collection_id)
                .bind(&desc.set_id)
                .bind(desc.set_sequence as i64)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(old) = &old {
            ensure(
                old.get::<String, _>("id") == desc.set_id
                    && old.get::<i64, _>("sequence") as u64 == desc.set_sequence
                    && old.get::<String, _>("index_hash") == desc.index_sha256.as_str()
                    && old.get::<String, _>("bundle_hash") == desc.bundle_sha256.as_str(),
                "conflicting recovery-point identity",
            )?;
            let known: ArchiveDiscDescriptor =
                serde_json::from_str(&old.get::<String, _>("descriptor"))?;
            ensure(
                known.first_disc == desc.first_disc
                    && known.last_disc == desc.last_disc
                    && known.index_bytes == desc.index_bytes
                    && known.index_records == desc.index_records,
                "conflicting recovery-point bounds",
            )?;
        } else {
            sqlx::query("INSERT INTO catalog_sets VALUES(?,?,?,?,?,?)")
                .bind(&desc.collection_id)
                .bind(&desc.set_id)
                .bind(desc.set_sequence as i64)
                .bind(desc.index_sha256.as_str())
                .bind(desc.bundle_sha256.as_str())
                .bind(serde_json::to_string(&desc)?)
                .execute(&mut *tx)
                .await?;
        }
        let mut input = BufReader::new(dir.file("snapshot.jsonl")?);
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut records = 0u64;
        loop {
            let mut line = Vec::new();
            (&mut input)
                .take(ARCHIVE_RECORD_BYTES as u64 + 2)
                .read_until(b'\n', &mut line)?;
            if line.is_empty() {
                break;
            }
            ensure(
                line.last() == Some(&b'\n') && line.len() <= ARCHIVE_RECORD_BYTES + 1,
                "invalid catalog index line",
            )?;
            bytes += line.len() as u64;
            records += 1;
            ensure(
                bytes <= desc.index_bytes && records <= desc.index_records,
                "catalog index exceeds declared bounds",
            )?;
            hash.update(&line);
            let record = archive_record(&line[..line.len() - 1])?;
            match record {
                ArchiveRecord::Disc {
                    disc_id,
                    number,
                    set_sequence,
                } => {
                    let prior=sqlx::query("SELECT id,number,set_sequence FROM catalog_discs WHERE collection=? AND (id=? OR number=?)").bind(&desc.collection_id).bind(&disc_id).bind(number as i64).fetch_optional(&mut *tx).await?;
                    if let Some(prior) = prior {
                        ensure(
                            prior.get::<String, _>(0) == disc_id
                                && prior.get::<i64, _>(1) as u64 == number
                                && prior.get::<i64, _>(2) as u64 == set_sequence,
                            "conflicting logical-disc identity",
                        )?;
                    } else {
                        sqlx::query("INSERT INTO catalog_discs VALUES(?,?,?,?)")
                            .bind(&desc.collection_id)
                            .bind(disc_id)
                            .bind(number as i64)
                            .bind(set_sequence as i64)
                            .execute(&mut *tx)
                            .await?;
                    }
                }
                ArchiveRecord::Placement { object, disc_id } if old.is_none() => {
                    let prior:Option<i64>=sqlx::query_scalar("SELECT length FROM catalog_placements WHERE collection=? AND root=? AND digest=? LIMIT 1").bind(&desc.collection_id).bind(object.root_id.as_str()).bind(object.digest.as_str()).fetch_optional(&mut *tx).await?;
                    ensure(
                        prior.is_none_or(|length| length as u64 == object.length.get()),
                        "conflicting root-scoped object length",
                    )?;
                    sqlx::query("INSERT INTO catalog_placements VALUES(?,?,?,?,?,?)")
                        .bind(&desc.collection_id)
                        .bind(&desc.set_id)
                        .bind(disc_id)
                        .bind(object.root_id.as_str())
                        .bind(object.digest.as_str())
                        .bind(object.length.get() as i64)
                        .execute(&mut *tx)
                        .await?;
                }
                _ => {}
            }
            if old.is_none() {
                sqlx::query("INSERT INTO catalog_records VALUES(?,?,?,?)")
                    .bind(&desc.collection_id)
                    .bind(&desc.set_id)
                    .bind(records as i64)
                    .bind(line)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        ensure(
            bytes == desc.index_bytes
                && records == desc.index_records
                && format!("{:x}", hash.finalize()) == desc.index_sha256.as_str(),
            "catalog index checksum/count mismatch",
        )?;
        let member:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM catalog_discs WHERE collection=? AND id=? AND number=? AND set_sequence=?)").bind(&desc.collection_id).bind(&desc.disc_id).bind(desc.number as i64).bind(desc.set_sequence as i64).fetch_one(&mut *tx).await?;
        ensure(member, "scanned disc is not in its original index")?;
        // Once an original set's index is known, later references must agree
        // with its complete payload inventory. Unknown earlier indexes stay unknown.
        let conflict:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM catalog_placements p JOIN catalog_discs d ON d.collection=p.collection AND d.id=p.disc JOIN catalog_sets original ON original.collection=d.collection AND original.sequence=d.set_sequence WHERE p.collection=? AND (p.set_id=? OR d.set_sequence=?) AND NOT EXISTS(SELECT 1 FROM catalog_placements own WHERE own.collection=p.collection AND own.set_id=original.id AND own.disc=p.disc AND own.root=p.root AND own.digest=p.digest AND own.length=p.length))").bind(&desc.collection_id).bind(&desc.set_id).bind(desc.set_sequence as i64).fetch_one(&mut *tx).await?;
        ensure(
            !conflict,
            "snapshot placement contradicts an original disc inventory",
        )?;
        let encoded = serde_json::to_string(&desc)?;
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT descriptor FROM catalog_scans WHERE collection=? AND disc=?",
        )
        .bind(&desc.collection_id)
        .bind(&desc.disc_id)
        .fetch_optional(&mut *tx)
        .await?;
        ensure(
            previous.as_ref().is_none_or(|v| v == &encoded),
            "scanned disc metadata changed",
        )?;
        sqlx::query("INSERT INTO catalog_scans VALUES(?,?,?) ON CONFLICT DO NOTHING")
            .bind(&desc.collection_id)
            .bind(&desc.disc_id)
            .bind(encoded)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn export(&mut self, output: &mut impl Write, include_records: bool) -> Result<()> {
        emit(
            output,
            &serde_json::json!({"kind":"catalog","format":"sct-offline-catalog-v1","coverage":"observed_metadata_only","unknown_later_sets":true,"physical_copy_observations_available":false,"payload_verification":"not_performed"}),
        )?;
        let mut after = (String::new(), 0i64);
        loop {
            let rows=sqlx::query("SELECT * FROM catalog_sets WHERE (collection,sequence)>(?,?) ORDER BY collection,sequence LIMIT 128").bind(&after.0).bind(after.1).fetch_all(&mut self.connection).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let collection: String = row.get("collection");
                let set: String = row.get("id");
                let sequence: i64 = row.get("sequence");
                let desc: ArchiveDiscDescriptor =
                    serde_json::from_str(&row.get::<String, _>("descriptor"))?;
                emit(
                    output,
                    &serde_json::json!({"kind":"recovery_point","collection_id":collection,"set_id":set,"sequence":sequence,"index_sha256":row.get::<String,_>("index_hash"),"representative_descriptor":desc}),
                )?;
                if include_records {
                    let mut position = 0i64;
                    loop {
                        let records=sqlx::query("SELECT position,line FROM catalog_records WHERE collection=? AND set_id=? AND position>? ORDER BY position LIMIT 128").bind(&collection).bind(&set).bind(position).fetch_all(&mut self.connection).await?;
                        if records.is_empty() {
                            break;
                        }
                        for record in records {
                            position = record.get(0);
                            let line: Vec<u8> = record.get(1);
                            let value: serde_json::Value = serde_json::from_slice(&line)?;
                            emit(
                                output,
                                &serde_json::json!({"kind":"index_record","collection_id":collection,"set_id":set,"position":position,"record":value}),
                            )?;
                        }
                    }
                }
                after = (collection, sequence);
            }
        }
        let mut after = (String::new(), 0i64);
        loop {
            let rows=sqlx::query("SELECT d.*,EXISTS(SELECT 1 FROM catalog_scans s WHERE s.collection=d.collection AND s.disc=d.id) AS scanned,EXISTS(SELECT 1 FROM catalog_sets s WHERE s.collection=d.collection AND s.sequence=d.set_sequence) AS original_known FROM catalog_discs d WHERE (d.collection,d.number)>(?,?) ORDER BY d.collection,d.number LIMIT 128").bind(&after.0).bind(after.1).fetch_all(&mut self.connection).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let collection: String = row.get("collection");
                let number: i64 = row.get("number");
                emit(
                    output,
                    &serde_json::json!({"kind":"logical_disc","collection_id":collection,"disc_id":row.get::<String,_>("id"),"number":number,"set_sequence":row.get::<i64,_>("set_sequence"),"metadata_scanned":row.get::<bool,_>("scanned"),"original_snapshot_index_known":row.get::<bool,_>("original_known"),"physical_copy_health":"unknown"}),
                )?;
                after = (collection, number);
            }
        }
        Ok(())
    }
}
