use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum Capability {
    ListRead,
    Write,
    ManagePolicy,
    Delete,
    Purge,
    ManageGrants,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum ErrorCode {
    Unauthorized,
    Forbidden,
    NotFound,
    UnsupportedOperation,
    InvalidRequest,
    InvalidPath,
    InvalidManifest,
    RevisionConflict,
    IdempotencyConflict,
    CursorRefreshRequired,
    RootBusy,
    PolicyTransition,
    QuotaExhausted,
    StagingExhausted,
    DestinationCapacity,
    DestinationUnavailable,
    UploadExpired,
    IntegrityFailure,
    RateLimited,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RevisionScope {
    Entry,
    Directory,
    Root,
    Policy,
    Grant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RetryGuidance {
    Never,
    After { delay_ms: Milliseconds },
    Refresh { scope: RevisionScope },
    ReconcileUpload { upload_id: UploadId },
}

/// Include only details the caller is authorized to inspect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum ErrorDetails {
    Capacity {
        required_bytes: ByteCount,
        available_bytes: Option<ByteCount>,
    },
    Busy {
        root_id: RootId,
        operation_id: OperationId,
    },
    StaleRevision {
        scope: RevisionScope,
        current_revision: Revision,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ErrorResponse {
    pub request_id: RequestId,
    pub code: ErrorCode,
    pub message: String,
    pub retry: RetryGuidance,
    pub details: Option<ErrorDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestLimits {
    pub max_page_size: PageSize,
    pub max_request_bytes: ByteCount,
    pub max_metadata_bytes: ByteCount,
    pub max_manifest_page_bytes: ByteCount,
    pub max_manifest_page_entries: ByteCount,
    pub max_chunk_batch: PageSize,
}

/// Advertise only implemented operations/formats. Reserved formats are not capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct Capabilities {
    pub protocol_version: u32,
    pub supported_operations: Vec<String>,
    pub storage_format_versions: Vec<u32>,
    pub chunk_profiles: Vec<String>,
    pub limits: RequestLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ListRequest {
    pub limit: PageSize,
    pub cursor: Option<CursorToken>,
}

/// Opaque token binds principal/root/directory/revision/filter/limit; server authenticates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct DirectoryCursor {
    pub token: CursorToken,
    pub directory_id: EntryId,
    pub directory_revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ListEntriesRequest {
    pub limit: PageSize,
    pub cursor: Option<DirectoryCursor>,
    pub include_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RetentionMode {
    Strict,
    Relaxed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RetentionPolicy {
    pub mode: RetentionMode,
    pub keep_versions: VersionCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum VersioningMode {
    IndependentFiles,
    AtomicDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum PolicyScope {
    Root { root_id: RootId },
    Entry { root_id: RootId, entry_id: EntryId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct EffectivePolicy {
    pub id: PolicyId,
    pub revision: Revision,
    pub source: PolicyScope,
    pub inherited: bool,
    pub policy: RetentionPolicy,
    pub versioning: VersioningMode,
    pub transitioning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct PreviewPolicyRequest {
    pub scope: PolicyScope,
    pub expected_revision: Revision,
    pub proposed: RetentionPolicy,
}

/// Apply must reject changed policy or scope state; counts are previews, not capacity promises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct PolicyPreview {
    pub id: PreviewId,
    pub scope: PolicyScope,
    pub policy_revision: Revision,
    pub scope_revision: Revision,
    pub proposed: RetentionPolicy,
    pub versions_affected: CommitSequence,
    pub expires_at_ms: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ApplyPolicyRequest {
    pub preview_id: PreviewId,
    pub expected_revision: Revision,
    pub expected_scope_revision: Revision,
    pub acknowledge_retention_effect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum AvailabilityHint {
    Intermittent,
    NormallyAvailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum ObservedHealth {
    Unknown,
    Reachable,
    Unavailable,
    Degraded,
}

/// Null means unknown; these expectations never determine read order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct InstanceHints {
    pub expected_initial_response_ms: Option<Milliseconds>,
    pub expected_bytes_per_second: Option<BytesPerSecond>,
    pub expected_availability: Option<AvailabilityHint>,
}

/// Administrator-facing behavior documentation, not certified guarantees or provider settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct AdapterDocumentation {
    pub kind: String,
    pub display_name: String,
    pub acknowledgement: String,
    pub consistency: String,
    pub durability: String,
    pub deletion_and_capacity: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct BackendInspection {
    pub id: BackendId,
    pub display_name: String,
    pub adapter_kind: String,
    pub hints: InstanceHints,
    pub observed_health: ObservedHealth,
    pub last_observed_at_ms: Option<TimestampMs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ReadCandidate {
    pub backend_id: BackendId,
    pub connection_timeout_ms: Milliseconds,
    pub initial_response_timeout_ms: Milliseconds,
    pub idle_timeout_ms: Milliseconds,
    pub throughput_window_ms: Milliseconds,
    pub minimum_bytes_per_second: BytesPerSecond,
    pub startup_allowance_ms: Milliseconds,
}

/// Sequential order is authoritative. Transfer progress budgets are distinct from connection budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ReadPolicy {
    #[cfg_attr(feature = "codegen", schemars(length(min = 1, max = 2)))]
    pub candidates: Vec<ReadCandidate>,
    pub overall_deadline_ms: Option<Milliseconds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum Placement {
    Single {
        backend_id: BackendId,
        read_policy: ReadPolicy,
    },
    Mirror {
        backend_ids: [BackendId; 2],
        read_policy: ReadPolicy,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RootState {
    Disabled,
    Enabled,
    Recovering,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RootInspection {
    pub id: RootId,
    pub owner_id: PrincipalId,
    pub display_name: EntryName,
    pub mount_path: RootPath,
    pub revision: Revision,
    pub state: RootState,
    pub placement: Placement,
    pub quota_bytes: ByteCount,
    pub default_policy: RetentionPolicy,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RootPage {
    #[cfg_attr(feature = "codegen", schemars(length(max = 1000)))]
    pub items: Vec<RootInspection>,
    pub next_cursor: Option<CursorToken>,
}

/// Creates a disabled root. Administration must reject destination aliases and over-allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRootRequest {
    pub idempotency_key: IdempotencyKey,
    pub owner_id: PrincipalId,
    pub display_name: EntryName,
    pub mount_path: RootPath,
    pub placement: Placement,
    pub quota_bytes: ByteCount,
    pub default_policy: RetentionPolicy,
    pub allocations: Vec<RootAllocationRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct UpdateRootRequest {
    pub expected_revision: Revision,
    pub display_name: EntryName,
    pub state: RootState,
    pub placement: Placement,
    pub quota_bytes: ByteCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct GrantInspection {
    pub id: GrantId,
    pub root_id: RootId,
    pub principal_id: PrincipalId,
    pub revision: Revision,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct UpdateGrantRequest {
    pub expected_revision: Revision,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum EntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct EntryInspection {
    pub id: EntryId,
    pub root_id: RootId,
    pub parent_id: Option<EntryId>,
    pub name: EntryName,
    pub kind: EntryKind,
    pub revision: Revision,
    pub hidden: bool,
    pub current_version_id: Option<VersionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct EntryPage {
    pub directory_id: EntryId,
    pub directory_revision: Revision,
    #[cfg_attr(feature = "codegen", schemars(length(max = 1000)))]
    pub items: Vec<EntryInspection>,
    pub next_cursor: Option<DirectoryCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ResolvePathRequest {
    pub path: RootPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateDirectoryRequest {
    pub idempotency_key: IdempotencyKey,
    pub parent_id: EntryId,
    pub expected_parent_revision: Revision,
    pub name: EntryName,
    pub versioning: VersioningMode,
}

/// V1 foundation rejects moves across effective-policy or atomic-versioning boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RenameEntryRequest {
    pub expected_revision: Revision,
    pub expected_source_parent_revision: Revision,
    pub destination_parent_id: EntryId,
    pub expected_destination_parent_revision: Revision,
    pub name: EntryName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RootAllocationRequest {
    pub pool_id: PoolId,
    pub retained_capacity_bytes: ByteCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum AllocationOwner {
    Root { root_id: RootId },
    ServerStaging,
}

/// Pending cleanup is a subset of occupied bytes, not additional free capacity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct AllocationInspection {
    pub id: AllocationId,
    pub pool_id: PoolId,
    pub owner: AllocationOwner,
    pub capacity_bytes: ByteCount,
    pub reserved_bytes: ByteCount,
    pub occupied_bytes: ByteCount,
    pub pending_cleanup_bytes: ByteCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct PoolInspection {
    pub id: PoolId,
    pub revision: Revision,
    pub allocatable_bytes: ByteCount,
    pub allocated_bytes: ByteCount,
    pub reported_available_bytes: Option<ByteCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RootUsage {
    pub root_id: RootId,
    pub logical_bytes: ByteCount,
    pub unique_retained_bytes: ByteCount,
    pub reclaimable_bytes: ByteCount,
    pub physical_copies: Vec<DestinationUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct DestinationUsage {
    pub backend_id: BackendId,
    pub occupied_bytes: ByteCount,
    pub pending_cleanup_bytes: ByteCount,
    pub capacity_recovery: CapacityRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum CapacityRecovery {
    Confirmed,
    Delayed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum OperationKind {
    Upload,
    Namespace,
    PolicyTransition,
    Purge,
    Reclamation,
    Deletion,
    Repair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum OperationState {
    Active,
    Recovering,
    CleaningUp,
    Completed,
    Aborted,
    Failed,
}

/// Authorized operational view. Worker epochs, leases and physical incarnations stay private.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct OperationInspection {
    pub id: OperationId,
    pub root_id: RootId,
    pub owner_principal_id: Option<PrincipalId>,
    pub kind: OperationKind,
    pub state: OperationState,
    pub upload_id: Option<UploadId>,
    pub completed_units: CommitSequence,
    pub total_units: Option<CommitSequence>,
    pub retry: RetryGuidance,
    pub failure_code: Option<ErrorCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum SessionState {
    Open,
    Ready,
    Committed,
    Aborted,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum CopyAcknowledgement {
    Pending,
    Acknowledged,
    Unavailable,
    IntegrityFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CopyProgress {
    pub backend_id: BackendId,
    pub acknowledged_bytes: ByteCount,
    pub state: CopyAcknowledgement,
}

/// Retained for idempotency reconciliation even if its content version is later pruned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CommitReceipt {
    pub version_id: VersionId,
    pub manifest_id: ManifestId,
    pub commit_sequence: CommitSequence,
}

/// Extension point: acknowledgement is not independently certified durability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct UploadInspection {
    pub id: UploadId,
    pub root_id: RootId,
    pub operation_id: OperationId,
    pub state: SessionState,
    pub expires_at_ms: TimestampMs,
    pub copies: Vec<CopyProgress>,
    pub result: Option<CommitReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum DraftMode {
    CompleteReplacement,
    Patch,
}

/// New identities are server-issued and draft-scoped; never infer identity from a path or digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum DraftEntryIdentity {
    Existing { entry_id: EntryId },
    New { entry_id: EntryId },
}

/// Draft/page encoding and deletion/purge semantics are reserved for M3, not implemented here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct DirectoryDraftBase {
    pub root_id: RootId,
    pub directory_id: EntryId,
    pub expected_revision: Revision,
    pub base_version_id: Option<VersionId>,
    pub mode: DraftMode,
}

/// Validation must check base identity continuity, uniqueness, root and draft ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct DraftEntryReference {
    pub identity: DraftEntryIdentity,
    pub path: RootPath,
    pub kind: EntryKind,
    pub manifest_id: Option<ManifestId>,
}

impl ErrorCode {
    pub fn http_status(&self) -> u16 {
        match self {
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound | Self::UnsupportedOperation => 404,
            Self::InvalidRequest | Self::InvalidPath | Self::InvalidManifest => 400,
            Self::RevisionConflict
            | Self::IdempotencyConflict
            | Self::CursorRefreshRequired
            | Self::RootBusy
            | Self::PolicyTransition => 409,
            Self::QuotaExhausted | Self::StagingExhausted | Self::DestinationCapacity => 507,
            Self::DestinationUnavailable => 503,
            Self::UploadExpired => 410,
            Self::IntegrityFailure => 502,
            Self::RateLimited => 429,
            Self::InternalError => 500,
        }
    }
    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Unauthorized => "Authentication is required.",
            Self::Forbidden => "This operation is not permitted.",
            Self::NotFound => "The requested resource is not available.",
            Self::UnsupportedOperation => "This API operation is not implemented.",
            Self::InvalidRequest => "The request is invalid.",
            Self::InvalidPath => "The logical path is invalid.",
            Self::InvalidManifest => "The manifest is invalid.",
            Self::RevisionConflict => "Refresh the resource before retrying.",
            Self::IdempotencyConflict => "The idempotency key is already bound to another request.",
            Self::CursorRefreshRequired => "The directory changed; restart the listing.",
            Self::RootBusy => "Another operation owns this root; retry later.",
            Self::PolicyTransition => "A policy transition is in progress; retry later.",
            Self::QuotaExhausted => "The retained root quota is exhausted.",
            Self::StagingExhausted => "Shared staging capacity is exhausted.",
            Self::DestinationCapacity => "A required destination has insufficient capacity.",
            Self::DestinationUnavailable => "A required destination is unavailable.",
            Self::UploadExpired => "The upload session has expired.",
            Self::IntegrityFailure => "Content verification failed.",
            Self::RateLimited => "Too many requests; retry later.",
            Self::InternalError => "The operation failed.",
        }
    }
}
impl ErrorResponse {
    /// Error text is selected from fixed strings; never pass backend errors through.
    pub fn new(request_id: RequestId, code: ErrorCode) -> Self {
        let retry = match code {
            ErrorCode::RootBusy
            | ErrorCode::PolicyTransition
            | ErrorCode::DestinationUnavailable
            | ErrorCode::RateLimited => RetryGuidance::After {
                delay_ms: 1000.try_into().unwrap(),
            },
            ErrorCode::CursorRefreshRequired => RetryGuidance::Refresh {
                scope: RevisionScope::Directory,
            },
            _ => RetryGuidance::Never,
        };
        Self {
            request_id,
            message: code.safe_message().into(),
            code,
            retry,
            details: None,
        }
    }
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            mode: RetentionMode::Strict,
            keep_versions: 1.try_into().unwrap(),
        }
    }
}
impl Placement {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let (destinations, reads): (Vec<&BackendId>, &ReadPolicy) = match self {
            Self::Single {
                backend_id,
                read_policy,
            } => (vec![backend_id], read_policy),
            Self::Mirror {
                backend_ids,
                read_policy,
            } => {
                if backend_ids[0] == backend_ids[1] {
                    return Err(ValidationError("mirror destinations must be distinct"));
                }
                (backend_ids.iter().collect(), read_policy)
            }
        };
        if reads.candidates.len() != destinations.len() {
            return Err(ValidationError(
                "read order must contain every destination exactly once",
            ));
        }
        for destination in destinations {
            if reads
                .candidates
                .iter()
                .filter(|r| &r.backend_id == destination)
                .count()
                != 1
            {
                return Err(ValidationError(
                    "read order must contain every destination exactly once",
                ));
            }
        }
        Ok(())
    }
}
impl CreateRootRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.placement.validate()?;
        if self.allocations.is_empty() || self.allocations.len() > 2 {
            return Err(ValidationError("one or two pool allocations are required"));
        }
        if self.allocations.len() == 2 && self.allocations[0].pool_id == self.allocations[1].pool_id
        {
            return Err(ValidationError("pool allocations must be distinct"));
        }
        Ok(())
    }
}
impl UpdateRootRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.state == RootState::Recovering {
            return Err(ValidationError("recovering is a server-owned state"));
        }
        self.placement.validate()
    }
}
impl ApplyPolicyRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.acknowledge_retention_effect {
            return Err(ValidationError("retention acknowledgement is required"));
        }
        Ok(())
    }
}
impl EntryPage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.items.len() > MAX_PAGE_SIZE as usize {
            return Err(ValidationError("page exceeds maximum size"));
        }
        if let Some(cursor) = &self.next_cursor
            && (cursor.directory_id != self.directory_id
                || cursor.directory_revision != self.directory_revision)
        {
            return Err(ValidationError("cursor does not match directory snapshot"));
        }
        Ok(())
    }
}

impl RootPage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.items.len() > MAX_PAGE_SIZE as usize {
            return Err(ValidationError("page exceeds maximum size"));
        }
        Ok(())
    }
}

impl ListEntriesRequest {
    /// Use after authorization and authenticating the opaque cursor token.
    pub fn validate_cursor(
        &self,
        directory_id: &EntryId,
        revision: &Revision,
    ) -> Result<(), ErrorCode> {
        if let Some(cursor) = &self.cursor
            && (&cursor.directory_id != directory_id || &cursor.directory_revision != revision)
        {
            return Err(ErrorCode::CursorRefreshRequired);
        }
        Ok(())
    }
}

impl UploadInspection {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if (self.state == SessionState::Committed) != self.result.is_some() {
            return Err(ValidationError(
                "only committed sessions carry commit receipts",
            ));
        }
        if self.copies.is_empty() || self.copies.len() > 2 {
            return Err(ValidationError(
                "one or two required copy outcomes are required",
            ));
        }
        if self.copies.len() == 2 && self.copies[0].backend_id == self.copies[1].backend_id {
            return Err(ValidationError(
                "copy outcomes must reference distinct destinations",
            ));
        }
        if matches!(self.state, SessionState::Ready | SessionState::Committed)
            && self
                .copies
                .iter()
                .any(|copy| copy.state != CopyAcknowledgement::Acknowledged)
        {
            return Err(ValidationError(
                "ready or committed sessions require all acknowledgements",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateGrantRequest {
    pub root_id: RootId,
    pub principal_id: PrincipalId,
    pub capabilities: Vec<Capability>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct GrantPage {
    pub items: Vec<GrantInspection>,
    pub next_cursor: Option<CursorToken>,
}

/// Foundation policy edits are restricted to scopes with no published content.
/// Destructive previews and content transitions are M3 operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct SetPolicyRequest {
    pub scope: PolicyScope,
    pub expected_revision: Revision,
    pub expected_scope_revision: Revision,
    pub policy: RetentionPolicy,
    pub versioning: VersioningMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ListEntriesQuery {
    pub directory_id: EntryId,
    pub limit: PageSize,
    pub cursor: Option<CursorToken>,
    pub directory_revision: Option<Revision>,
    #[serde(default)]
    pub include_hidden: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ListGrantsQuery {
    pub root_id: RootId,
    pub limit: PageSize,
    pub cursor: Option<CursorToken>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ExpectedRevision {
    pub expected_revision: Revision,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct SessionInspection {
    pub principal_id: PrincipalId,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct IssuedToken {
    pub id: CredentialId,
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct AllocationPage {
    pub items: Vec<AllocationInspection>,
    pub next_cursor: Option<CursorToken>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct PolicyConstraintsRequest {
    pub expected_revision: Revision,
    pub min_versions: VersionCount,
    pub max_versions: VersionCount,
    pub allow_relaxed: bool,
}
