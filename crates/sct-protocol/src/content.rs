//! Immutable v1 content. Canonical bytes, rather than a decoded value, are hashed.
use crate::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

pub const CHUNK_MIN: u32 = 256 * 1024;
pub const CHUNK_TARGET: u32 = 1024 * 1024;
pub const CHUNK_MAX: u32 = 4 * 1024 * 1024;
pub const CHUNK_PROFILE: &str = "fastcdc-v2020-3.2.1-n1-s0-256k-1m-4m";
pub const MANIFEST_BYTES: usize = 256 * 1024;
pub const MANIFEST_FANOUT: usize = 1024;
pub const MANIFEST_HEIGHT: u32 = 8;

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
/// StreamCDC buffers at most CHUNK_MAX bytes plus one returned chunk.
pub fn chunker<R: std::io::Read>(reader: R) -> fastcdc::v2020::StreamCDC<R> {
    fastcdc::v2020::StreamCDC::with_level_and_seed(
        reader,
        CHUNK_MIN,
        CHUNK_TARGET,
        CHUNK_MAX,
        fastcdc::v2020::Normalization::Level1,
        0,
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ChunkReference {
    pub digest: ChunkDigest,
    pub length: ByteCount,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct PageReference {
    pub digest: ManifestId,
    pub length: ByteCount,
    pub chunks: ByteCount,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum ManifestPage {
    Chunks {
        format: u32,
        chunks: Vec<ChunkReference>,
    },
    Branch {
        format: u32,
        height: u32,
        pages: Vec<PageReference>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct FileManifest {
    pub format: u32,
    pub profile: String,
    pub length: ByteCount,
    pub chunks: ByteCount,
    pub height: u32,
    pub tree: Option<ManifestId>,
}
fn valid(ok: bool) -> Result<(), ValidationError> {
    if ok {
        Ok(())
    } else {
        Err(ValidationError("invalid manifest"))
    }
}
impl ManifestPage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Chunks { format, chunks } => {
                valid(*format == 1 && !chunks.is_empty() && chunks.len() <= MANIFEST_FANOUT)?;
                for c in chunks {
                    valid((1..=u64::from(CHUNK_MAX)).contains(&c.length.get()))?;
                }
            }
            Self::Branch {
                format,
                height,
                pages,
            } => {
                valid(
                    *format == 1
                        && (1..=MANIFEST_HEIGHT).contains(height)
                        && !pages.is_empty()
                        && pages.len() <= MANIFEST_FANOUT,
                )?;
                for p in pages {
                    valid(p.length.get() > 0 && p.chunks.get() > 0)?;
                }
                valid(
                    pages
                        .iter()
                        .map(|p| p.length.get())
                        .try_fold(0u64, |a, b| a.checked_add(b))
                        .is_some_and(|v| v <= MAX_SAFE_INTEGER),
                )?;
                valid(
                    pages
                        .iter()
                        .map(|p| p.chunks.get())
                        .try_fold(0u64, |a, b| a.checked_add(b))
                        .is_some_and(|v| v <= MAX_SAFE_INTEGER),
                )?;
            }
        }
        Ok(())
    }
}
impl FileManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        valid(self.format == 1 && self.profile == CHUNK_PROFILE && self.height <= MANIFEST_HEIGHT)?;
        if self.length.get() == 0 {
            valid(self.tree.is_none() && self.chunks.get() == 0 && self.height == 0)
        } else {
            valid(
                self.tree.is_some()
                    && self.chunks.get() > 0
                    && self.chunks.get() <= self.length.get(),
            )
        }
    }
}
/// Lexicographic ASCII field ordering, UTF-8, compact JSON, integer numbers only.
/// These types contain no arbitrary maps/floats. serde_json's ordered Value map
/// pins field ordering independently of declaration order.
pub fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, ValidationError> {
    let value = serde_json::to_value(value).map_err(|_| ValidationError("invalid manifest"))?;
    let bytes = serde_json::to_vec(&value).map_err(|_| ValidationError("invalid manifest"))?;
    valid(bytes.len() <= MANIFEST_BYTES)?;
    Ok(bytes)
}
pub fn parse_canonical<T: Serialize + DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, ValidationError> {
    valid(bytes.len() <= MANIFEST_BYTES)?;
    let value: T =
        serde_json::from_slice(bytes).map_err(|_| ValidationError("invalid manifest"))?;
    valid(canonical(&value)? == bytes)?;
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum FileTarget {
    Create {
        parent_id: EntryId,
        expected_parent_revision: Revision,
        name: EntryName,
    },
    Replace {
        entry_id: EntryId,
        expected_revision: Revision,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateUploadRequest {
    pub idempotency_key: IdempotencyKey,
    pub target: FileTarget,
    pub expected_policy_revision: Revision,
    pub manifest: FileManifest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum AdmissionState {
    Planning,
    Pending,
    Admitted,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct TransferStatus {
    pub upload: UploadInspection,
    pub admission: AdmissionState,
    pub failure: Option<ErrorCode>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct MissingChunkPage {
    pub chunks: Vec<ChunkReference>,
    pub next_cursor: Option<CursorToken>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct FileVersion {
    pub id: VersionId,
    pub entry_id: EntryId,
    pub manifest_id: ManifestId,
    pub length: ByteCount,
    pub commit_sequence: CommitSequence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct VersionPage {
    pub versions: Vec<FileVersion>,
    pub next_cursor: Option<CursorToken>,
}
