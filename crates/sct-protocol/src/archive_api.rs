//! Account-owned optical archive contracts. ISO downloads are not burn evidence.
use crate::*;
use serde::{Deserialize, Serialize};
macro_rules! dto {
    ($($item:item)*) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[cfg_attr(feature="codegen",derive(schemars::JsonSchema,ts_rs::TS))]
        $item
    )*};
}
macro_rules! enumeration {
    ($($item:item)*) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all="snake_case")]
        #[cfg_attr(feature="codegen",derive(schemars::JsonSchema,ts_rs::TS))]
        $item
    )*};
}
enumeration! {
    pub enum ArchiveJobState { Capturing, Planning, AwaitingConfirmation, Building, Ready, Failed, Cancelled, Expired, Revoked }
    pub enum ArchiveArtifactState { Building, Available, Expired, Failed }
    pub enum ArchiveCopyHealth { Unknown, ReadBackVerified, Damaged, Missing }
    pub enum ArchiveRecoveryCompleteness { CompleteSnapshot, CompleteFilesOnly, Unavailable }
}
dto! {
    pub struct CreateArchiveCollectionRequest {
        pub request_id: RequestId,
        pub label: EntryName,
        pub media_capacity_bytes: ByteCount,
    }
    pub struct ArchiveCollection {
        pub id: ArchiveCollectionId,
        pub owner: PrincipalId,
        pub label: EntryName,
        pub revision: Revision,
        pub media_capacity_bytes: ByteCount,
        pub next_disc_number: ByteCount,
        pub last_completed_set_id: Option<ArchiveSetId>,
        // Restoring an old catalog must not reuse numbers possibly burned since then.
        pub reconciliation_required: bool,
        pub created_at_ms: TimestampMs,
    }
    pub struct ArchiveCollectionPage {pub collections: Vec<ArchiveCollection>, pub next_cursor: Option<CursorToken>}
    pub struct UpdateArchiveCollectionRequest {
        pub expected_revision: Revision,
        pub label: EntryName,
        pub media_capacity_bytes: ByteCount,
    }
    /// A committed file/tree version retained at its logical source path.
    pub struct ArchiveSelection {
        pub root_id: RootId,
        pub version_id: VersionId,
        pub path: RootPath,
    }
    pub struct CreateArchivePlanRequest {
        pub request_id: RequestId,
        pub expected_revision: Revision,
        #[cfg_attr(feature="codegen",schemars(extend("minItems"=1,"maxItems"=1024)))]
        pub selections: Vec<ArchiveSelection>,
        pub media_capacity_bytes: ByteCount,
    }
    pub struct ArchivePlanSummary {
        pub source_files: ByteCount,
        pub source_bytes: ByteCount,
        pub new_payload_bytes: ByteCount,
        pub reused_payload_bytes: ByteCount,
        pub new_disc_count: ByteCount,
        pub required_earlier_disc_count: ByteCount,
        pub index_bytes_per_disc: ByteCount,
        pub bundle_bytes_per_disc: ByteCount,
        pub filesystem_reserve_per_disc: ByteCount,
        pub total_iso_budget_bytes: ByteCount,
        pub bundle_sha256: ChunkDigest,
    }
    pub struct ArchiveSet {
        pub id: ArchiveSetId,
        pub collection_id: ArchiveCollectionId,
        pub sequence: ByteCount,
        pub revision: Revision,
        pub state: ArchiveJobState,
        pub selections: Vec<ArchiveSelection>,
        pub media_capacity_bytes: ByteCount,
        pub processed_entries: ByteCount,
        pub processed_bytes: ByteCount,
        pub plan: Option<ArchivePlanSummary>,
        pub failure: Option<ErrorCode>,
        pub created_at_ms: TimestampMs,
        pub expires_at_ms: TimestampMs,
        // Historical publication is permanent; cache availability is separate.
        pub published_at_ms: Option<TimestampMs>,
        pub verified_complete_at_ms: Option<TimestampMs>,
    }
    pub struct ArchiveSetPage {pub sets: Vec<ArchiveSet>, pub next_cursor: Option<CursorToken>}
    pub struct ArchiveSetActionRequest {pub expected_revision: Revision, pub request_id: RequestId}
    pub struct ArchiveLogicalDisc {
        pub id: ArchiveDiscId,
        pub collection_id: ArchiveCollectionId,
        pub set_id: ArchiveSetId,
        pub number: ByteCount,
        pub index_sha256: ChunkDigest,
        pub bundle_sha256: ChunkDigest,
        pub payload_bytes: ByteCount,
        pub object_count: ByteCount,
        pub payload_inventory_sha256: ChunkDigest,
        pub planned_iso_bytes: ByteCount,
        pub artifact: Option<ArchiveArtifact>,
    }
    pub struct ArchiveDiscPage {pub discs: Vec<ArchiveLogicalDisc>, pub next_cursor: Option<CursorToken>}
    pub struct ArchiveArtifact {
        pub id: ArchiveArtifactId,
        pub state: ArchiveArtifactState,
        pub sha256: Option<ChunkDigest>,
        pub length: Option<ByteCount>,
        pub expires_at_ms: TimestampMs,
        pub download_count: ByteCount,
    }
    pub struct RegisterArchiveCopyRequest {
        pub request_id: RequestId,
        pub label: EntryName,
        pub reported_burned_at_ms: Option<TimestampMs>,
    }
    pub struct ArchivePhysicalCopy {
        pub id: ArchiveCopyId,
        pub disc_id: ArchiveDiscId,
        pub label: EntryName,
        pub revision: Revision,
        pub reported_burned_at_ms: Option<TimestampMs>,
        pub health: ArchiveCopyHealth,
        pub latest_report_id: Option<ArchiveReportId>,
    }
    pub struct UpdateArchiveCopyRequest {
        pub expected_revision: Revision,
        pub label: EntryName,
        pub health: ArchiveCopyHealth,
    }
    pub struct ArchiveCopyPage {pub copies: Vec<ArchivePhysicalCopy>, pub next_cursor: Option<CursorToken>}
    pub struct SubmitArchiveVerificationRequest {
        pub request_id: RequestId,
        pub expected_revision: Revision,
        pub disc_id: ArchiveDiscId,
        pub index_sha256: ChunkDigest,
        pub bundle_sha256: ChunkDigest,
        pub payload_inventory_sha256: ChunkDigest,
        pub expected_objects: ByteCount,
        pub verified_objects: ByteCount,
        pub verified_bytes: ByteCount,
        pub metadata_verified: bool,
        pub verifier_version: EntryName,
    }
    pub struct ArchiveVerificationReport {
        pub id: ArchiveReportId,
        pub copy_id: ArchiveCopyId,
        pub received_at_ms: TimestampMs,
        pub observation: SubmitArchiveVerificationRequest,
        // An authenticated client observation, not proof that plastic was burned.
        pub complete_read_back: bool,
    }
    pub struct ArchiveReportPage {pub reports: Vec<ArchiveVerificationReport>, pub next_cursor: Option<CursorToken>}
    pub struct ArchiveEntry {
        pub path: RootPath,
        pub kind: EntryKind,
        pub root_id: Option<RootId>,
        pub version_id: Option<VersionId>,
        pub length: ByteCount,
    }
    pub struct ArchiveEntryPage { pub entries: Vec<ArchiveEntry>, pub next_cursor: Option<CursorToken> }
    pub struct ArchiveDiscObject { pub root_id: RootId, pub digest: ChunkDigest, pub length: ByteCount }
    pub struct ArchiveObjectPage { pub objects: Vec<ArchiveDiscObject>, pub next_cursor: Option<CursorToken> }
    pub struct ArchiveUsage {
        pub cache_bytes: ByteCount,
        pub reserved_bytes: ByteCount,
        pub quota_bytes: ByteCount,
        pub active_sets: ByteCount,
    }
    pub struct ArchiveMissingObject {
        pub path: RootPath,
        pub root_id: RootId,
        pub digest: ChunkDigest,
        pub length: ByteCount,
        pub candidate_discs: Vec<ArchiveDiscId>,
    }
    pub struct ArchiveRecoveryStatus {
        pub completeness: ArchiveRecoveryCompleteness,
        pub complete_files: ByteCount,
        pub incomplete_files: ByteCount,
        pub missing: Vec<ArchiveMissingObject>,
        pub next_cursor: Option<CursorToken>,
    }
}
impl CreateArchivePlanRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.selections.is_empty()
            || self.selections.len() > ARCHIVE_MAX_SOURCES
            || self.media_capacity_bytes.get() == 0
        {
            return Err(ValidationError("invalid archive capture bounds"));
        }
        let mut destinations = std::collections::BTreeSet::new();
        for selection in &self.selections {
            archive_path(&selection.path)?;
            if !destinations.insert(selection.path.as_str()) {
                return Err(ValidationError("duplicate archive destination"));
            }
        }
        // Prefix-overlapping selections would make snapshot composition ambiguous.
        for destination in &destinations {
            let mut parent = *destination;
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                parent = if prefix.is_empty() { "/" } else { prefix };
                if parent != *destination && destinations.contains(parent) {
                    return Err(ValidationError("overlapping archive destinations"));
                }
                if parent == "/" {
                    break;
                }
            }
        }
        Ok(())
    }
}

impl ArchiveJobState {
    pub fn can_transition_to(&self, next: &Self) -> bool {
        use ArchiveJobState::*;
        matches!(
            (self, next),
            (Capturing, Planning)
                | (Planning, AwaitingConfirmation)
                | (AwaitingConfirmation, Building)
                | (Building, Ready)
        ) || matches!(self, Capturing | Planning | AwaitingConfirmation | Building)
            && matches!(next, Failed | Cancelled | Expired | Revoked)
    }
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Failed | Self::Cancelled | Self::Expired | Self::Revoked
        )
    }
}
impl UpdateArchiveCopyRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.health == ArchiveCopyHealth::ReadBackVerified {
            return Err(ValidationError(
                "verification requires an immutable read-back report",
            ));
        }
        Ok(())
    }
}
