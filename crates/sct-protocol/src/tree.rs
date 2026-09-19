//! Canonical directory objects and M3 lifecycle requests. See docs/tree-lifecycle.md.
use crate::*;
use serde::{Deserialize, Serialize};

macro_rules! dto {
    ($($item:item)*) => {$ (
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[cfg_attr(feature="codegen",derive(schemars::JsonSchema,ts_rs::TS))]
        $item
    )*};
}
dto! {
    pub struct DirectoryEntry {
        pub id: EntryId,
        pub name: EntryName,
        pub kind: EntryKind,
        pub manifest: ManifestId,
        pub length: ByteCount,
        pub chunks: ByteCount,
    }
    pub struct DirectoryPageReference {
        pub digest: ManifestId,
        pub first: EntryName,
        pub last: EntryName,
        pub entries: ByteCount,
        pub length: ByteCount,
        pub chunks: ByteCount,
    }
    pub struct DirectoryManifest {
        pub format: u32,
        pub kind: DirectoryObjectKind,
        pub entries: ByteCount,
        pub length: ByteCount,
        pub chunks: ByteCount,
        pub height: u32,
        pub tree: Option<ManifestId>,
    }
    pub struct CreateTreeDraftRequest {
        pub idempotency_key: IdempotencyKey,
        pub base: DirectoryDraftBase,
        pub expected_policy_revision: Revision,
    }
    pub struct DraftMutationPage {
        pub idempotency_key: IdempotencyKey,
        pub mutations: Vec<DraftMutation>,
    }
    pub struct AllocateDraftIdentitiesRequest {
        pub idempotency_key: IdempotencyKey,
        pub count: PageSize,
    }
    pub struct DraftIdentities { pub entry_ids: Vec<EntryId> }
    pub struct TreeDraftStatus {
        pub id: UploadId,
        pub root_id: RootId,
        pub directory_id: EntryId,
        pub operation_id: OperationId,
        pub state: SessionState,
        pub phase: TreeDraftPhase,
        pub admission: AdmissionState,
        pub processed_entries: ByteCount,
        pub expires_at_ms: TimestampMs,
        pub result: Option<CommitReceipt>,
    }
    pub struct SnapshotEntryPage {
        pub version_id: VersionId,
        pub entries: Vec<DirectoryEntry>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct SnapshotListQuery {
        pub path: RootPath,
        pub limit: PageSize,
        pub cursor: Option<CursorToken>,
    }
    pub struct SnapshotPathQuery { pub path: RootPath }
    pub struct HideEntryRequest {
        pub idempotency_key: IdempotencyKey,
        pub expected_revision: Revision,
    }
    pub struct PurgeRequest {
        pub idempotency_key: IdempotencyKey,
        pub expected_revision: Revision,
        pub target: PurgeTarget,
    }
    pub struct LifecycleJob {
        pub id: OperationId,
        pub root_id: RootId,
        pub state: LifecycleState,
        pub phase: String,
        pub processed: ByteCount,
        pub total: Option<ByteCount>,
        pub failure: Option<ErrorCode>,
    }
    pub struct PreviewChangeRequest {
        pub expected_root_revision: Revision,
        pub change: PolicyChange,
    }
    pub struct ChangePreview {
        pub id: PreviewId,
        pub root_id: RootId,
        pub root_revision: Revision,
        pub state: LifecycleState,
        pub versions_to_retire: ByteCount,
        pub versions_losing_protection: ByteCount,
        pub estimated_payload_bytes: ByteCount,
        pub expires_at_ms: TimestampMs,
        pub change: PolicyChange,
    }
    pub struct ApplyChangeRequest {
        pub idempotency_key: IdempotencyKey,
        pub preview_id: PreviewId,
        pub expected_root_revision: Revision,
        pub acknowledge_retention_effect: bool,
    }
    pub struct PreviewVersion {
        pub version_id: VersionId,
        pub entry_id: EntryId,
        pub retire: bool,
        pub loses_protection: bool,
    }
    pub struct PreviewVersionPage {
        pub versions: Vec<PreviewVersion>,
        pub next_cursor: Option<CursorToken>,
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum DirectoryObjectKind {
    Directory,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum DirectoryPage {
    DirectoryEntries {
        format: u32,
        entries: Vec<DirectoryEntry>,
    },
    DirectoryBranch {
        format: u32,
        height: u32,
        pages: Vec<DirectoryPageReference>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum DraftMutation {
    Upsert {
        identity: DraftEntryIdentity,
        parent_id: EntryId,
        name: EntryName,
        entry_kind: EntryKind,
        file: Option<FileManifest>,
    },
    Remove {
        entry_id: EntryId,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum TreeDraftPhase {
    Initializing,
    Editing,
    Building,
    Planning,
    Staging,
    Ready,
    Committed,
    CleaningUp,
    Terminal,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum PurgeTarget {
    HistoricalVersion { version_id: VersionId },
    HiddenEntry { entry_id: EntryId },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum LifecycleState {
    Planning,
    Ready,
    Applying,
    CleaningUp,
    Blocked,
    Completed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum PolicyChange {
    Set {
        scope: PolicyScope,
        expected_policy_revision: Revision,
        policy: RetentionPolicy,
    },
    Inherit {
        scope: PolicyScope,
        expected_policy_revision: Revision,
    },
    Move {
        entry_id: EntryId,
        expected_revision: Revision,
        parent_id: EntryId,
        expected_parent_revision: Revision,
        name: EntryName,
    },
}
fn valid(value: bool) -> Result<(), ValidationError> {
    if value {
        Ok(())
    } else {
        Err(ValidationError("invalid directory or lifecycle request"))
    }
}
fn totals(values: impl Iterator<Item = u64>) -> Result<(), ValidationError> {
    valid(values.try_fold_safe().is_some())
}
trait SafeSum: Iterator<Item = u64> + Sized {
    fn try_fold_safe(mut self) -> Option<u64> {
        self.try_fold(0u64, |a, b| {
            a.checked_add(b).filter(|n| *n <= MAX_SAFE_INTEGER)
        })
    }
}
impl<T: Iterator<Item = u64>> SafeSum for T {}
impl DirectoryManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        valid(self.format == 1 && self.height <= MANIFEST_HEIGHT)?;
        if self.entries.get() == 0 {
            valid(
                self.tree.is_none()
                    && self.height == 0
                    && self.length.get() == 0
                    && self.chunks.get() == 0,
            )
        } else {
            valid(self.tree.is_some())
        }
    }
}
impl DirectoryPage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::DirectoryEntries { format, entries } => {
                valid(*format == 1 && !entries.is_empty() && entries.len() <= MANIFEST_FANOUT)?;
                valid(
                    entries
                        .windows(2)
                        .all(|p| p[0].name.as_str() < p[1].name.as_str()),
                )?;
                let mut ids = std::collections::BTreeSet::new();
                valid(entries.iter().all(|e| ids.insert(e.id.as_str())))?;
                totals(entries.iter().map(|e| e.length.get()))?;
                totals(entries.iter().map(|e| e.chunks.get()))?;
            }
            Self::DirectoryBranch {
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
                valid(
                    pages
                        .iter()
                        .all(|p| p.entries.get() > 0 && p.first.as_str() <= p.last.as_str()),
                )?;
                valid(
                    pages
                        .windows(2)
                        .all(|p| p[0].last.as_str() < p[1].first.as_str()),
                )?;
                totals(pages.iter().map(|e| e.entries.get()))?;
                totals(pages.iter().map(|e| e.length.get()))?;
                totals(pages.iter().map(|e| e.chunks.get()))?;
            }
        }
        canonical(self)?;
        Ok(())
    }
}
impl DraftMutationPage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        valid(!self.mutations.is_empty() && self.mutations.len() <= MAX_PAGE_SIZE as usize)?;
        for mutation in &self.mutations {
            if let DraftMutation::Upsert {
                identity,
                parent_id,
                entry_kind,
                file,
                ..
            } = mutation
            {
                let id = match identity {
                    DraftEntryIdentity::Existing { entry_id }
                    | DraftEntryIdentity::New { entry_id } => entry_id,
                };
                valid(id != parent_id)?;
                match (entry_kind, file) {
                    (EntryKind::File, Some(m)) => m.validate()?,
                    (EntryKind::Directory, None) => {}
                    _ => return valid(false),
                }
            }
        }
        canonical(self)?;
        Ok(())
    }
}
impl ApplyChangeRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        valid(self.acknowledge_retention_effect)
    }
}
