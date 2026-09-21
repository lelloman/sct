//! Portable optical archive v1. Independent of service catalog migrations.
use crate::*;
use serde::{Deserialize, Serialize};

pub const ARCHIVE_FORMAT: u32 = 1;
pub const ARCHIVE_RECORD_BYTES: usize = 64 * 1024;
pub const ARCHIVE_MAX_RECORDS: u64 = 100_000_000;
pub const ARCHIVE_MAX_INDEX_BYTES: u64 = 1 << 40;
pub const ARCHIVE_MAX_DISCS: u64 = 1_000_000;
pub const ARCHIVE_MAX_SOURCES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveDiscDescriptor {
    pub format: u32,
    pub collection_id: String,
    pub set_id: String,
    pub set_sequence: u64,
    pub disc_id: String,
    pub number: u64,
    pub first_disc: u64,
    pub last_disc: u64,
    pub index_sha256: ChunkDigest,
    pub index_bytes: u64,
    pub index_records: u64,
    pub bundle_sha256: ChunkDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveObject {
    pub root_id: RootId,
    pub digest: ChunkDigest,
    pub length: ByteCount,
}
impl ArchiveObject {
    /// Root identity is hashed for an unambiguous, portable filesystem component.
    pub fn relative_path(&self) -> String {
        format!(
            "objects/{}/{}",
            digest(self.root_id.as_str().as_bytes()),
            self.digest.as_str()
        )
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        archive_require((1..=CHUNK_MAX as u64).contains(&self.length.get()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveSource {
    pub root_id: RootId,
    pub version_id: VersionId,
    pub path: RootPath,
}

/// Canonical JSONL: one bounded record per line, never an unbounded file chunk list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchiveRecord {
    Snapshot {
        format: u32,
        collection_id: String,
        set_id: String,
        set_sequence: u64,
        first_disc: u64,
        last_disc: u64,
        sources: Vec<ArchiveSource>,
    },
    Disc {
        disc_id: String,
        number: u64,
        set_sequence: u64,
    },
    Directory {
        path: RootPath,
    },
    File {
        path: RootPath,
        root_id: RootId,
        version_id: VersionId,
        length: ByteCount,
    },
    Extent {
        path: RootPath,
        offset: ByteCount,
        object: ArchiveObject,
    },
    Placement {
        object: ArchiveObject,
        disc_id: String,
    },
}

pub fn archive_require(ok: bool) -> Result<(), ValidationError> {
    if ok {
        Ok(())
    } else {
        Err(ValidationError("invalid archive"))
    }
}
pub fn archive_id(value: &str) -> Result<(), ValidationError> {
    archive_require(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
    )
}
pub fn archive_path(path: &RootPath) -> Result<(), ValidationError> {
    // Linux v1 paths are UTF-8, relative to the selected destination after dropping
    // exactly the leading slash. Never normalize user-controlled path components.
    archive_require(path.as_str().len() <= 4096 && path.as_str().split('/').count() <= 129)
}
impl ArchiveDiscDescriptor {
    pub fn validate(&self) -> Result<(), ValidationError> {
        archive_id(&self.collection_id)?;
        archive_id(&self.set_id)?;
        archive_id(&self.disc_id)?;
        archive_require(
            self.format == ARCHIVE_FORMAT
                && self.set_sequence > 0
                && self.set_sequence <= ARCHIVE_MAX_DISCS
                && self.first_disc > 0
                && self.first_disc <= self.number
                && self.number <= self.last_disc
                && self.last_disc <= ARCHIVE_MAX_DISCS
                && self.index_bytes > 0
                && self.index_bytes <= ARCHIVE_MAX_INDEX_BYTES
                && self.index_records > 0
                && self.index_records <= ARCHIVE_MAX_RECORDS,
        )
    }
}
impl ArchiveRecord {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Snapshot {
                format,
                collection_id,
                set_id,
                set_sequence,
                first_disc,
                last_disc,
                sources,
            } => {
                archive_id(collection_id)?;
                archive_id(set_id)?;
                archive_require(
                    *format == ARCHIVE_FORMAT
                        && *set_sequence > 0
                        && *set_sequence <= ARCHIVE_MAX_DISCS
                        && *first_disc > 0
                        && *first_disc <= *last_disc
                        && *last_disc <= ARCHIVE_MAX_DISCS
                        && !sources.is_empty()
                        && sources.len() <= ARCHIVE_MAX_SOURCES,
                )?;
                for source in sources {
                    archive_path(&source.path)?;
                }
            }
            Self::Disc {
                disc_id,
                number,
                set_sequence,
            } => {
                archive_id(disc_id)?;
                archive_require(
                    *number > 0
                        && *number <= ARCHIVE_MAX_DISCS
                        && *set_sequence > 0
                        && *set_sequence <= ARCHIVE_MAX_DISCS,
                )?;
            }
            Self::Directory { path } => archive_path(path)?,
            Self::File { path, .. } => {
                archive_path(path)?;
                archive_require(path.as_str() != "/")?;
            }
            Self::Extent {
                path,
                offset,
                object,
            } => {
                archive_path(path)?;
                object.validate()?;
                archive_require(
                    path.as_str() != "/"
                        && offset
                            .get()
                            .checked_add(object.length.get())
                            .is_some_and(|end| end <= MAX_SAFE_INTEGER),
                )?;
            }
            Self::Placement { object, disc_id } => {
                object.validate()?;
                archive_id(disc_id)?;
            }
        }
        archive_require(canonical(self)?.len() <= ARCHIVE_RECORD_BYTES)
    }
}

pub fn archive_record(bytes: &[u8]) -> Result<ArchiveRecord, ValidationError> {
    archive_require(bytes.len() <= ARCHIVE_RECORD_BYTES)?;
    let record: ArchiveRecord = parse_canonical(bytes)?;
    record.validate()?;
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveBundleFile {
    pub path: String,
    pub length: ByteCount,
    pub sha256: ChunkDigest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveBundle {
    pub format: u32,
    pub version: String,
    pub platform: String,
    pub files: Vec<ArchiveBundleFile>,
}
impl ArchiveBundle {
    pub fn validate(&self) -> Result<(), ValidationError> {
        archive_id(&self.version)?;
        archive_id(&self.platform)?;
        archive_require(
            self.format == ARCHIVE_FORMAT && !self.files.is_empty() && self.files.len() <= 64,
        )?;
        let mut previous = "";
        for f in &self.files {
            let path: RootPath = format!("/{}", f.path).parse()?;
            archive_path(&path)?;
            archive_require(
                f.path.as_str() > previous
                    && f.path != "recovery/bundle.json"
                    && (f.path == "START-HERE.txt" || f.path.starts_with("recovery/"))
                    && f.length.get() <= 512 * 1024 * 1024,
            )?;
            previous = &f.path;
        }
        archive_require(canonical(self)?.len() <= ARCHIVE_RECORD_BYTES)
    }
}
