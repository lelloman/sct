//! Rust-owned public contracts, shared by clients and the server.
//! Parse requests with Serde, then call their `validate` method where provided.
//! Validation here covers wire shape and self-contained invariants. Authorization,
//! revision comparisons, policy inheritance and catalog identity checks live in core.
//!
//! Identities cannot be interchanged, even when their serialized strings match:
//! ```compile_fail
//! use sct_protocol::{RootId, EntryId};
//! let root: RootId = "root_a".parse().unwrap();
//! let entry: EntryId = root;
//! ```

mod tree;
pub use tree::*;
mod content;
pub use content::*;
mod scalars;
mod types;
pub use scalars::*;
pub use types::*;

#[cfg(feature = "codegen")]
pub mod codegen;

/// Reserved public API prefix. This does not advertise implemented operations.
pub const API_PREFIX: &str = "/api/v1";

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_PAGE_SIZE: u32 = 1000;
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// A fixed diagnostic for invalid wire values. Never echoes supplied secrets/data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationError(pub &'static str);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ValidationError {}

/// Implemented foundation application operations.
pub const FOUNDATION_OPERATIONS: &[&str] = &[
    "capabilities",
    "list_roots",
    "create_root",
    "inspect_root",
    "update_root",
    "list_entries",
    "resolve_path",
    "create_directory",
    "inspect_entry",
    "rename_entry",
    "effective_policy",
    "set_policy",
    "root_operation",
    "inspect_pool",
    "inspect_allocation",
    "inspect_backend",
    "adapter_documentation",
    "inspect_grant",
    "update_grant",
    "create_grant",
    "list_grants",
    "revoke_grant",
    "list_allocations",
    "policy_constraints",
];

/// Implemented independent-file transfer operations (M2).
pub const TRANSFER_OPERATIONS: &[&str] = &[
    "list_versions",
    "read_manifest",
    "download_content",
    "begin_upload",
    "inspect_upload",
    "abort_upload",
    "missing_chunks",
    "upload_chunk",
    "upload_page",
    "admit_upload",
    "seal_upload",
    "commit_upload",
];
