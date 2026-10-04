//! Offline archive reader. Uses a private, disk-backed SQLite work index only.
mod catalog;
pub use catalog::ArchiveCatalog;
mod index;
mod restore;
mod verification;
pub use restore::RestoreSummary;
mod safe_fs;
mod schema;
mod sqlite_connection;
pub use index::ArchiveIndex;
pub use safe_fs::SafeDir;
use sct_protocol::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn ensure(ok: bool, message: &'static str) -> Result<()> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn small_file(dir: &SafeDir, path: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    dir.file(path)?
        .take(ARCHIVE_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure(
        bytes.len() <= ARCHIVE_RECORD_BYTES,
        "archive metadata exceeds limit",
    )?;
    Ok(bytes)
}
pub fn hash_reader(mut reader: impl Read, length: u64) -> Result<String> {
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut block = [0u8; 65536];
    loop {
        let n = reader.read(&mut block)?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or("size overflow")?;
        ensure(count <= length, "archive file longer than declared")?;
        hash.update(&block[..n]);
    }
    ensure(count == length, "archive file shorter than declared")?;
    Ok(format!("{:x}", hash.finalize()))
}
pub fn descriptor(dir: &SafeDir) -> Result<ArchiveDiscDescriptor> {
    let value: ArchiveDiscDescriptor = parse_canonical(&small_file(dir, "disc.json")?)?;
    value.validate()?;
    Ok(value)
}
pub fn verify_bundle(dir: &SafeDir, desc: &ArchiveDiscDescriptor) -> Result<()> {
    let bytes = small_file(dir, "recovery/bundle.json")?;
    ensure(
        digest(&bytes) == desc.bundle_sha256.as_str(),
        "bundle manifest checksum mismatch",
    )?;
    let bundle: ArchiveBundle = parse_canonical(&bytes)?;
    bundle.validate()?;
    for artifact in bundle.files {
        ensure(
            hash_reader(dir.file(&artifact.path)?, artifact.length.get())?
                == artifact.sha256.as_str(),
            "recovery bundle checksum mismatch",
        )?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct VerificationSummary {
    pub collection_id: String,
    pub disc_id: String,
    /// The target index being verified; can differ from an earlier disc's index.
    pub index_sha256: String,
    pub disc_index_sha256: String,
    pub expected_objects: u64,
    pub verified_objects: u64,
    pub unavailable_objects: u64,
    pub verified_bytes: u64,
}
impl VerificationSummary {
    pub fn complete(&self) -> bool {
        self.expected_objects == self.verified_objects
    }
}
impl ArchiveIndex {
    /// Stream object observations as JSONL rather than accumulating a large report.
    pub async fn verify_media(
        &mut self,
        media: &Path,
        report: &mut impl Write,
    ) -> Result<VerificationSummary> {
        use sqlx::Row;
        let dir = SafeDir::open(media)?;
        let desc = descriptor(&dir)?;
        ensure(
            desc.collection_id == self.descriptor.collection_id,
            "wrong disc collection",
        )?;
        let expected = sqlx::query("SELECT number,set_sequence FROM discs WHERE id=?")
            .bind(&desc.disc_id)
            .fetch_optional(&mut self.connection)
            .await?
            .ok_or("disc not required by this snapshot")?;
        ensure(
            expected.get::<i64, _>(0) as u64 == desc.number
                && expected.get::<i64, _>(1) as u64 == desc.set_sequence,
            "wrong disc identity",
        )?;
        if desc.set_sequence == self.descriptor.set_sequence {
            ensure(
                desc.index_sha256 == self.descriptor.index_sha256
                    && desc.bundle_sha256 == self.descriptor.bundle_sha256
                    && desc.set_id == self.descriptor.set_id,
                "contradictory current-set metadata",
            )?;
        }
        ensure(
            hash_reader(dir.file("snapshot.jsonl")?, desc.index_bytes)?
                == desc.index_sha256.as_str(),
            "media index checksum mismatch",
        )?;
        verify_bundle(&dir, &desc)?;
        let mut summary = VerificationSummary {
            collection_id: desc.collection_id,
            disc_id: desc.disc_id.clone(),
            index_sha256: self.descriptor.index_sha256.as_str().into(),
            disc_index_sha256: desc.index_sha256.as_str().into(),
            expected_objects: 0,
            verified_objects: 0,
            unavailable_objects: 0,
            verified_bytes: 0,
        };
        let mut after = (String::new(), String::new());
        loop {
            let rows=sqlx::query("SELECT root,digest,length FROM placements WHERE disc=? AND (root,digest)>(?,?) ORDER BY root,digest LIMIT 128").bind(&desc.disc_id).bind(&after.0).bind(&after.1).fetch_all(&mut self.connection).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let object = ArchiveObject {
                    root_id: row.get::<String, _>(0).parse()?,
                    digest: row.get::<String, _>(1).parse()?,
                    length: (row.get::<i64, _>(2) as u64).try_into()?,
                };
                after = (
                    object.root_id.as_str().into(),
                    object.digest.as_str().into(),
                );
                let state = match dir.file(&object.relative_path()) {
                    Err(error)
                        if error.downcast_ref::<rustix::io::Errno>()
                            == Some(&rustix::io::Errno::NOENT) =>
                    {
                        "missing"
                    }
                    Err(_) => "unreadable",
                    Ok(file) => match hash_reader(file, object.length.get()) {
                        Ok(hash) if hash == object.digest.as_str() => "verified",
                        _ => "corrupt",
                    },
                };
                let good = state == "verified";
                summary.expected_objects += 1;
                if good {
                    summary.verified_objects += 1;
                    summary.verified_bytes += object.length.get();
                } else {
                    summary.unavailable_objects += 1;
                }
                serde_json::to_writer(
                    &mut *report,
                    &serde_json::json!({"disc_id":desc.disc_id,"root_id":object.root_id,"digest":object.digest,"length":object.length,"verified":good,"state":state}),
                )?;
                report.write_all(b"\n")?;
            }
        }
        Ok(summary)
    }
}
