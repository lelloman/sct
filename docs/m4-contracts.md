# Management and recovery contract v1 (SCT-42)

This document freezes the approved M4 behavior. Contracts are exported from
`sct-protocol`; routes enter the operation matrix when implemented. Existing
M1–M3 content encodings, authorization and SDK/API write operations remain valid.
M4 implementation and qualification are tracked by SCT-43 through SCT-61; M5
qualification remains separate.

## Permissions and browser surface

| Operation | Regular principal | Administrator | Browser |
| --- | --- | --- | --- |
| Browse roots, snapshot history, immutable trees, download | `list_read` on root | All roots | Yes |
| Issue/list/revoke own credentials | Own account | Any account | Yes |
| Manage principals or administrator status | No | Yes | Yes |
| Manage root grants | `manage_grants`, without privilege amplification | Yes | Yes |
| Edit root name, quota, expected upload interval | `manage_root`, within ceiling | Yes | Yes |
| Preview/apply retention policy | `manage_policy`; existing explicit acknowledgment | Yes | Yes |
| Change quota ceiling, placement, physical allocations or backend configuration | No | Yes | Yes |
| Inspect root usage, activity and findings | Authorized root scope | All roots | Yes |
| Start/resume maintenance, create/export/release checkpoints | No | Yes | Yes |
| Upload, file edit, rename, hide, purge | Existing content capabilities | Existing content capabilities | No |
| Source capture, scheduling, application restore | Consumer responsibility | Consumer responsibility | No |

`manage_root` is additive and does not imply `write`, `delete`, `purge`,
`manage_policy` or `manage_grants`. Existing delegated grants gain no new rights
on upgrade. Root owners and administrators have full root capabilities. Every
request rechecks active credentials, current grants and root scope. UI visibility
is not authorization. Revoked browser sessions clear cached private data.
Cookie-authenticated mutations retain CSRF protection. Credential secrets are
returned at issuance only and never appear in inspection, audit or logs.

The root quota is a logical retained-content limit; its administrator-owned
ceiling is a separate upper bound. Migration sets each existing ceiling to its
current quota. Changing a quota neither allocates storage nor rewrites history.
Lowering a quota below usage is permitted: subsequent admission must prune
according to policy or refuse before accepting payload. Raising a ceiling cannot
bypass physical allocation constraints. Concurrent edits require an exact
`expected_revision`; stale requests fail without partial changes.

## Configuration and accounting

Process configuration owns database/listen/OIDC/operator credentials and bootstrap
storage definitions. Once imported, catalog configuration owns administrator
changes to backend instances, pools and allocations across restart. Startup must
not silently overwrite persisted changes with old bootstrap values. Backend
settings contain provisioned secret references, never resolved secret values.
Inspection and audit must redact sensitive configuration. Adapter documentation
and optional expectations are separate from observed health and actual capacity.

Destination identity, aliases, backend membership and populated-root placement
changes must retain existing validation; unsupported transitions fail explicitly.
Configuration changes cannot orphan charged copies or active operations. Pool
capacity cannot drop below allocations, and allocations cannot drop below
occupied plus reserved bytes. Pending cleanup remains occupied until confirmed
deletion. Report logical retained, unique retained, physical occupied, staging,
reserved and pending cleanup separately; unknown provider capacity stays null.

All collections use bounded keyset pages with opaque, scope-bound cursors.
Administrative lists never disclose another root through a regular-user filter.
Audit records describe committed management and lifecycle transitions, omitting
secrets and content payloads; aborted changes must not create success records.

## Durable maintenance

Jobs carry kind, root, idempotency key, durable progress cursor, attempt count,
retry deadline and structured failure. Repeating the same principal/key/request
returns the same job; a changed request conflicts. States are `queued`, `running`,
`retry_wait`, `blocked`, `completed`, `failed`. Running work survives process loss
through lease fencing and durable cursors. Transient failures use bounded retries
and persisted backoff; authorization/configuration failures block; exhausted
retries fail visibly. Explicit resume revalidates authorization and prerequisites.

Integrity scans page through catalog objects and physical copies without loading
a root's graph into memory. Findings distinguish missing, corrupt, inaccessible
and catalog-inconsistent copies. Repair requires a source whose bytes match the
expected digest and length. Never promote an unchecked replica. Repairs reserve
capacity before writes and preserve exact-generation fencing, uncertain I/O
accounting and root ownership. A stale worker or lost response cannot replace a
new generation or release another worker's charges. Maintenance cannot steal an
active upload/lifecycle owner. Terminal status follows durable reconciliation.

## Checkpoints and recovery

Checkpoint states are `fencing`, `exporting`, `exported`, `failed`, `released`.
Creation uses an idempotency key. A persisted service barrier drains previously
issued destructive I/O and fences publication/lifecycle changes while capturing
a consistent catalog point. No long-running export may hold the writer-lease
row lock. Before releasing the barrier, persist deletion protection for every
required physical generation. Garbage collection and repair must respect these
pins across restart, including already queued deletion work.

A monotonically allocated catalog sequence is the generation watermark. Physical
generation identifiers are opaque strings, not ordered revisions. Inventory
records bind root, backend, digest, exact physical generation and byte length.
Inventory is deterministic and paged. The descriptor has format 1, storage
format version 1, checkpoint ID, watermark, SHA-256 catalog and inventory digests,
and package state. Unsupported formats are rejected before recovery writes.

`catalog_only` means an exported catalog and inventory, not an independent backup.
Payload copying is operator-controlled. `payload_verified` may be emitted only
by recovery tooling after every required generation has been copied and verified
against its digest and length in independent storage. Required service settings
and credential references must be supplied separately; secret values are not
embedded in the public descriptor. Failed exports remain inspectable and
conservatively pinned. Explicit release requires the current revision and an
acknowledgment that deletion protection is being removed. Export completion
alone never releases pins.

Recovery operates on an explicitly selected empty isolated catalog and storage,
validates formats/digests/inventory and namespaces before activation, and never
silently points at production destinations. Runtime leases, barriers and worker
ownership are reinitialized through the recovery procedure. Committed snapshots
at the capture point are promised; unfinished uploads are not committed backups.
The drill proves namespace and downloaded bytes after production is unavailable,
including an operator-controlled offline authentication recovery path when the
IdP is unavailable. This is service disaster recovery, not application restore.

## Visibility

Readiness reflects catalog/writer readiness; health and maintenance failures stay
visible. Prometheus labels are bounded categories, never root/principal/object
IDs or paths. Optional overdue indicators compare the configured expected
interval with the last successful snapshot commit, never session creation,
failed uploads or maintenance activity. They do not schedule backups.
