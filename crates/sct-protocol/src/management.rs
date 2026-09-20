//! Management and operator recovery contracts. Content writers remain SDK/API clients.
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
    pub enum MaintenanceKind { IntegrityScan, MirrorRepair }
    pub enum MaintenanceState { Queued, Running, RetryWait, Blocked, Completed, Failed }
    pub enum IntegrityFindingKind { Missing, Corrupt, Inaccessible, CatalogInconsistent }
    pub enum CheckpointState { Fencing, Exporting, Exported, Failed, Released }
    pub enum RecoveryPackageState { CatalogOnly, PayloadVerified }
}
dto! {
    pub struct AccountInspection {
        pub principal_id: PrincipalId,
        pub administrator: bool,
    }
    pub struct PrincipalInspection {
        pub id: PrincipalId,
        pub administrator: bool,
        pub disabled: bool,
        pub revision: Revision,
    }
    pub struct PrincipalPage {
        pub principals: Vec<PrincipalInspection>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct CreatePrincipalRequest { pub id: PrincipalId }
    pub struct UpdatePrincipalRequest {
        pub expected_revision: Revision,
        pub disabled: bool,
        pub administrator: bool,
    }
    pub struct CredentialInspection {
        pub id: CredentialId,
        pub principal_id: PrincipalId,
        pub kind: String,
        pub revoked: bool,
        pub expires_at_ms: Option<TimestampMs>,
    }
    pub struct CredentialPage {
        pub credentials: Vec<CredentialInspection>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct RootManagement {
        pub root_id: RootId,
        pub revision: Revision,
        pub quota_bytes: ByteCount,
        pub quota_ceiling_bytes: ByteCount,
        pub expected_upload_interval_seconds: Option<ByteCount>,
        pub last_commit_at_ms: Option<TimestampMs>,
        pub capabilities: Vec<Capability>,
    }
    pub struct UpdateRootManagementRequest {
        pub expected_revision: Revision,
        pub display_name: EntryName,
        pub quota_bytes: ByteCount,
        pub expected_upload_interval_seconds: Option<ByteCount>,
    }
    pub struct UpdateQuotaCeilingRequest {
        pub expected_revision: Revision,
        pub quota_ceiling_bytes: ByteCount,
    }
    pub struct CreatePoolRequest { pub id: PoolId, pub capacity_bytes: ByteCount, pub staging_bytes: ByteCount }
    pub struct CreateAllocationRequest { pub pool_id: PoolId, pub root_id: RootId, pub capacity_bytes: ByteCount }
    pub struct AllocationManagement { pub inspection: AllocationInspection, pub revision: Revision }
    pub struct UpdatePoolRequest {
        pub expected_revision: Revision,
        pub capacity_bytes: ByteCount,
    }
    pub struct ResizeAllocationRequest {
        pub expected_revision: Revision,
        pub capacity_bytes: ByteCount,
    }
    pub struct ConfigureBackendRequest {
        pub expected_revision: Option<Revision>,
        pub inspection: BackendInspection,
        pub pool_id: PoolId,
        #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
        pub settings: serde_json::Value,
    }
    pub struct BackendConfiguration {
        pub inspection: BackendInspection,
        pub pool_id: PoolId,
        pub revision: Revision,
        #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
        pub settings: serde_json::Value,
    }
    pub struct BackendPage {
        pub backends: Vec<BackendConfiguration>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct PoolPage {
        pub pools: Vec<PoolInspection>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct TransferActivity {
        pub id: UploadId,
        pub root_id: RootId,
        pub principal_id: PrincipalId,
        pub kind: String,
        pub state: SessionState,
        pub phase: String,
        pub admission: Option<AdmissionState>,
        pub expires_at_ms: TimestampMs,
        pub failure: Option<ErrorCode>,
    }
    pub struct TransferActivityPage { pub sessions: Vec<TransferActivity>, pub next_cursor: Option<CursorToken> }
    pub struct JobActivity { pub operation: OperationInspection, pub phase: String, pub lifecycle: Option<LifecycleJob> }
    pub struct JobActivityPage { pub jobs: Vec<JobActivity>, pub next_cursor: Option<CursorToken> }
    pub struct RootCapacityUsage {
        pub root_id: RootId,
        pub logical_retained_bytes: ByteCount,
        pub unique_retained_bytes: ByteCount,
        pub physical_occupied_bytes: ByteCount,
        pub staging_occupied_bytes: ByteCount,
        pub reserved_bytes: ByteCount,
        pub pending_cleanup_bytes: ByteCount,
    }
    pub struct AuditEvent {
        pub id: String,
        pub root_id: Option<RootId>,
        pub principal_id: Option<PrincipalId>,
        pub event: String,
        #[cfg_attr(feature = "codegen", ts(type = "unknown"))]
        pub details: serde_json::Value,
        pub created_at_ms: TimestampMs,
    }
    pub struct AuditPage { pub events: Vec<AuditEvent>, pub next_cursor: Option<CursorToken> }
    pub struct StartMaintenanceRequest {
        pub idempotency_key: IdempotencyKey,
        pub kind: MaintenanceKind,
        pub root_id: RootId,
    }
    pub struct MaintenanceJob {
        pub id: OperationId,
        pub root_id: RootId,
        pub kind: MaintenanceKind,
        pub state: MaintenanceState,
        pub processed: ByteCount,
        pub attempts: u32,
        pub next_attempt_at_ms: Option<TimestampMs>,
        pub failure: Option<ErrorCode>,
    }
    pub struct MaintenancePage {
        pub jobs: Vec<MaintenanceJob>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct IntegrityFinding {
        pub id: String,
        pub root_id: RootId,
        pub backend_id: Option<BackendId>,
        pub digest: ChunkDigest,
        pub generation: Option<String>,
        pub kind: IntegrityFindingKind,
        pub repaired: bool,
    }
    pub struct IntegrityFindingPage {
        pub findings: Vec<IntegrityFinding>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct CreateCheckpointRequest { pub idempotency_key: IdempotencyKey }
    pub struct ReleaseCheckpointRequest {
        pub expected_revision: Revision,
        pub acknowledge_release: bool,
    }
    pub struct CheckpointInspection {
        pub id: OperationId,
        pub revision: Revision,
        pub state: CheckpointState,
        pub generation_watermark: ByteCount,
        pub pinned_bytes: ByteCount,
        pub failure: Option<ErrorCode>,
        pub created_at_ms: TimestampMs,
    }
    pub struct CheckpointPage {
        pub checkpoints: Vec<CheckpointInspection>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct CheckpointObject {
        pub root_id: RootId,
        pub backend_id: BackendId,
        pub digest: ChunkDigest,
        pub generation: String,
        pub bytes: ByteCount,
    }
    pub struct CheckpointInventoryPage {
        pub objects: Vec<CheckpointObject>,
        pub next_cursor: Option<CursorToken>,
    }
    pub struct CheckpointDescriptor {
        pub format: u32,
        pub checkpoint_id: OperationId,
        pub generation_watermark: ByteCount,
        pub catalog_digest: ChunkDigest,
        pub inventory_digest: ChunkDigest,
        pub package_state: RecoveryPackageState,
        pub storage_format_version: u32,
    }
}
impl UpdateRootManagementRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self
            .expected_upload_interval_seconds
            .is_some_and(|n| !(60..=31_536_000).contains(&n.get()))
        {
            return Err(ValidationError("invalid expected upload interval"));
        }
        Ok(())
    }
}
impl ReleaseCheckpointRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.acknowledge_release {
            return Err(ValidationError(
                "checkpoint release requires acknowledgement",
            ));
        }
        Ok(())
    }
}
impl CheckpointDescriptor {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.format != 1 || self.storage_format_version != 1 {
            return Err(ValidationError("unsupported recovery format"));
        }
        Ok(())
    }
}
