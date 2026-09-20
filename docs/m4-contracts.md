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

## Catalog migration (SCT-43)

Migration 13 adds the management catalog without rewriting prior migrations.
Existing roots receive their current quota as their ceiling; old root creation
paths initialize a ceiling at insert. Existing grants are unchanged. Principal
and allocation revisions, backend configuration import state, audit records,
maintenance progress/findings, checkpoints, their barrier and exact-generation
inventory have durable tables. No checkpoint barrier or pin is activated during
upgrade. Missing expected intervals and last-commit timestamps remain unknown.
Copy creation sequences are backfilled and immutable; physical generation IDs,
root owners, reservations, occupied bytes and pending cleanup are preserved.
This migration supplies storage for later executors; it does not activate jobs
or silently change running configuration.

## Account and root APIs (SCT-44)

`GET /api/v1/account` identifies the current principal and administrator status.
`/api/v1/principals` supports administrator listing and creation. Inspecting a
principal is allowed to that account or an administrator; revision-checked PATCH
is administrator-only. Disabling or demoting the last enabled administrator is
rejected. Disabled accounts cannot authenticate or receive new credentials.

`/api/v1/principals/{principal}/credentials` lists metadata or issues a token for
that account, subject to self/admin authorization. Listing never returns hashes,
CSRF values or secrets. `DELETE /api/v1/credentials/{credential}` revokes a token
or session immediately; existing `/auth/tokens` routes remain available. Token
issuance returns its secret exactly once: clients must not automatically retry
an ambiguous response; inspect/revoke the orphaned credential before replacement.
All responses use `Cache-Control: no-store`; cookie mutations require origin and
CSRF validation. Opaque pagination cursors are bound to principal, scope and limit.

`GET/PATCH /api/v1/roots/{root}/management` exposes permitted settings and requires
`manage_root` for changes. Changes serialize against root work and recheck grants;
quota cannot exceed either the ceiling or current physical allocation. The
administrator-only `PATCH /api/v1/roots/{root}/quota-ceiling` cannot lower a ceiling
below the current quota. The older administrator-only root PATCH remains backward
compatible: a higher quota also raises its ceiling in the same transaction.
Existing grant routes prevent delegation of capabilities the caller does not
hold, including after waiting for the root lock. Committed account, credential,
grant and delegated-setting mutations record audit metadata without secrets.

## Storage administration (SCT-45)

Administrator routes now list/create pools, resize pools and allocations, create
root allocations, and list/configure backend destinations. Allocation management
inspection carries its revision separately from the original accounting DTO.
Only committed changes enter the audit log; settings and resolved credentials
are omitted from audit. Adapter validation rejects raw credential fields and
credential-bearing endpoint URLs. S3 configuration contains environment references;
local destinations require an existing private service-owned directory.

Bootstrap TOML infrastructure is imported once, atomically with a durable marker.
Subsequent restarts load catalog backend settings and retain administrator changes
to budgets, hints and credential references. TOML still controls process and
identity-provider configuration. An M3 upgrade imports its existing destinations
on first M4 startup; conflicting identities or insufficient budgets fail that
import without partial changes. Edit storage through administration after import.

Backend updates require the current revision. Kind, destination identity and pool
membership are immutable; unsupported relocation is rejected. A successfully
opened new runtime store is published after the catalog commit. Concurrent runtime
changes serialize; in-flight I/O keeps its handle to the same destination. Startup
reopens committed settings after a crash between commit and runtime publication.
An instance's observed health is derived by its adapter, never accepted from the
request's claimed health. Provider availability may still be unknown.

Pool capacity cannot undercut allocations. Allocation shrink cannot undercut
reserved plus occupied bytes (including pending cleanup), or the root quota's
required placement copies. Root allocation shrink waits for exclusive root work;
administrators can increase capacity while a repair is blocked for space.
Staging resize uses the existing pool/allocation accounting locks. A root may be
allocated space in another pool before an allowed empty-root placement change.
Populated roots keep their membership; updating read order or other settings
preserves backend references to existing copy generations.

## Inspection and audit (SCT-46)

`GET /roots/{root}/usage` returns one catalog snapshot. Logical retained bytes
sum retained version lengths, unique retained bytes count distinct reachable
payload chunks, and physical occupied bytes include metadata and every charged
copy generation. Staging occupied and pending cleanup are subsets of physical
occupied, not extra usage to add again. Reservations are separate commitments
owned by that root, including its use of shared staging. Retired versions held
by readers remain physically charged even though excluded from retained usage.
PostgreSQL computes graph unions and aggregates; application memory receives one
summary, never a flattened content graph.

Root-scoped `/sessions`, `/jobs` and `/audit` GET routes require `list_read`.
Session activity includes directory drafts before payload admission and never
renews expiry, claims an operation or exposes write access. Job pages include
terminal operations and lifecycle progress without disclosing worker leases or
credentials. Administrator `/admin/jobs` and `/admin/audit` pages cover all roots.
Audit merges management and transfer events in descending timestamp order, with
source/sequence tie breakers so equal timestamps cannot lose or duplicate rows.
Cursors bind the principal, root/filter and page size and recheck authorization
on every page. Audit details contain metadata, never credential secrets or bytes.

## Maintenance execution framework (SCT-47)

Jobs persist their original idempotency owner separately from the administrator
currently authorizing execution. Reusing a key with a different request conflicts.
The scheduler selects at most eight eligible jobs per pass and one bounded batch
per job. Busy roots remain queued without stealing their owner; rescheduled work
rotates behind older eligible jobs. Each claim increments independent worker and
root-operation fences. A new server can reclaim an old claim immediately after
writer takeover; late progress from the old server/worker is rejected.

Executors return a durable cursor, processed delta and completion flag. Progress
and terminal state commit together. Root ownership cannot complete while physical
I/O, staging charges or reservations remain unresolved. The coordinator renews
its lease every ten seconds while awaiting a batch and caps a batch at 120 seconds.
Cancellation stops renewal so another worker can recover after lease expiry.
Provider executors must preserve exact-generation I/O fencing independently of
this metadata coordinator; scan/repair integrations follow in SCT-48/SCT-49.
No scan/repair operation is advertised before those integrations land.

Transient failures persist retry deadlines with exponential backoff and fail
visibly after five consecutive attempts. Success clears the consecutive-attempt
counter. Authorization and configuration/capacity failures block. Blocked/failed
jobs retain ownership conservatively. Revision-checked explicit resume resets
backoff and binds the current administrator's credential, leaving original
idempotency ownership intact. Every claim, renewal and progress commit rechecks
authority. Failed attempts and blocked/resumed/completed transitions are audited.

## Integrity scanning (SCT-48)

Administrators queue scans through `POST /admin/maintenance` with kind
`integrity_scan`. The runtime executes them automatically on idle roots. Root
readers can inspect `/roots/{root}/maintenance`, an individual job beneath that
path, and `/roots/{root}/integrity-findings`; service-wide listing and explicit
resume remain administrator-only. Findings carry their scan job ID so historical
reports remain distinguishable. Repair remains a separate capability.

Scans reuse the durable graph frontier, then inspect at most sixteen reachable
catalog objects per batch. A batch buffers one physical object at a time, verifies
SHA-256 and length for each configured replica, and compares canonical metadata
with catalog kind, height, length and ordered edges. Missing catalog copies and
missing provider objects are reported separately from corrupt bytes, inaccessible
providers and inconsistent catalog metadata. Reads have a ten-second budget;
batches yield after fifteen seconds between objects, preserving their cursor.
A timeout is an accessibility finding, never evidence of corruption.

Progress survives writer replacement. If graph generation changes, traversal is
rebuilt conservatively. Findings are idempotent per job/object/generation/type;
a crash before progress commit may repeat reads but cannot duplicate findings.
Neither successful reads nor findings promote copies, delete bytes, or release
charges. Completed means the scan finished; health requires reviewing its findings.
Ordinary transfer cleanup cannot take over a maintenance-owned operation.

### Verified mirror repair (SCT-49)

Administrators can POST `StartMaintenanceRequest` with kind `mirror_repair` to
`/api/v1/admin/roots/{root}/repair`. Only mirrored roots support repair. The
maintenance coordinator retains exclusive root ownership, and existing status,
audit and resume routes expose its progress. Work consumes outstanding physical
integrity findings; catalog inconsistencies require operator investigation.

Each repair verifies the canonical object shape and a healthy alternate's full
length and digest before retiring a damaged generation. It reserves staging
capacity, writes a fresh generation, verifies it, confirms deletion of the old
generation, and transfers the charge into the retained allocation. Unknown
writes and deletes remain charged and are reconciled on retry. A durable target
ledger makes replay after a lost progress acknowledgement idempotent. A missing
healthy source blocks repair without retiring the original copy. Administrators
can correct availability or capacity and explicitly resume blocked work.

### Checkpoint fencing and pins (SCT-50)

The core checkpoint coordinator persists a draining barrier before capture. New
deletion intents wait; already issued exact-generation deletes are retried in
bounded batches and retain their charges until confirmed. Capture starts only
when those intents have drained and existing catalog transactions have finished.
A shared transaction fence covers ordinary catalog writes, including publication
and lifecycle changes. Capture then freezes those changes while pinning available
physical generations in batches of 256, ordered by their monotonic sequence.
The capture watermark never orders opaque generation identifiers.

Checkpoint inspection and authenticated explicit release remain available during
capture. A server replacement marks an interrupted capture failed, removes its
service barrier and retains all recorded generation pins. Garbage collection and
mirror repair cannot retire a pinned generation. Release requires an exact
revision and explicit acknowledgement. This step provides the core coordinator;
the export artifact and HTTP workflow follow in SCT-51.

### Checkpoint export (SCT-51)

`POST /api/v1/admin/checkpoints` accepts an idempotency key and returns a queued
checkpoint. The runtime drains, captures and exports it automatically; callers
poll the inspection route instead of declaring an export finished. The earlier
placeholder `/finish` operation is replaced by artifact and descriptor reads.
The administrator-only collection, inspection, inventory, descriptor and artifact
routes use the same current-credential authorization as other management APIs.
`POST /admin/checkpoints/{checkpoint}/abandon` explicitly releases protection with
the current revision and `acknowledge_release: true`.

`checkpoint_dir` defaults to `checkpoints` and must be private storage outside the
web asset directory. Each checkpoint gets a private directory containing
`catalog.jsonl`, `inventory.jsonl` and `descriptor.json`. Files are created without
overwriting existing artifacts, flushed and synced before export is reported as
complete. Only an exported checkpoint exposes artifact downloads. Partial files
from failed exports remain private for operator inspection.

The exporter establishes a PostgreSQL repeatable-read snapshot while the catalog
capture barrier is active. It then releases that barrier and streams bounded
catalog batches without holding the writer-lease row lock. The JSONL catalog
header identifies its format, schema migration version/checksum, checkpoint and
watermark; subsequent records bind table names to complete catalog rows. Runtime
ownership is included for explicit reinitialization during isolated recovery.
The deterministic inventory lists exact physical generations in catalog sequence
order. Both files have SHA-256 digests in the versioned descriptor.

The descriptor always reports `catalog_only`. This export contains no payload
copy and is not an independent backup. It can contain sensitive administrative
catalog data and is served only to administrators. Export failure or server
interruption preserves the recorded pins; success also retains them until an
explicit release. Operator-controlled payload copying and independent validation
are the recovery workflow in SCT-52.

### Isolated recovery (SCT-52)

The operator CLI can explicitly copy inventoried generations from supplied local
or S3 adapter settings, verify an independent payload directory, and recover into
an empty isolated catalog and payload destination. These commands bypass normal
production configuration loading. Recovery validates hashes, format/schema,
namespace and retained graph coverage before activating the imported catalog;
normal SQL constraints remain enabled. All destination backends are rewritten to
new local storage. Old credentials are revoked, leases and temporary pins are
cleared, unfinished uploads expire, and privileged maintenance requires explicit
review/resume. Offline `recover-admin` provides access without the IdP.

The drill restores committed file and tree bytes with original storage offline,
rejects missing/corrupt payloads and populated targets, and writes a fresh backup
after recovery. [The operator procedure](checkpoint-recovery.md) documents the
commands, independent-storage responsibility and partial-failure behavior.

### Browser foundation (SCT-53)

The browser shell loads the current account and assigned roots, pages root lists,
and separates regular-account navigation from administrator areas. It uses
same-origin session cookies and CSRF headers; an expired session clears visible
account data. Identity-provider sign-in is offered only when configured. An
explicit credential sign-in exchanges a valid bearer credential at
`POST /auth/session` for the same 12-hour HttpOnly session/CSRF cookies used by
OIDC. The exchange requires the exact service origin; browser code never persists
the bearer credential. Logout revokes the session and clears both cookies.

The responsive shell includes loading, empty, permission and request-error states.
Program file mutations remain API/SDK operations. Browser snapshot controls in
the following tasks are limited to browsing, history and downloading.

### Snapshot browser (SCT-54)

Assigned roots now expose paged namespace browsing, retained file history and
atomic directory snapshot traversal. Native authenticated download links stream
current and historical file bytes without buffering backups in browser memory.
The browser has no upload, rename, hide, purge, file-delete or application-restore
controls, including for administrators. A real program-API fixture verifies the
current file, a retained older version and a descendant of an atomic snapshot.

### Accounts and credentials (SCT-55)

The account page lists credentials and sessions, issues program credentials with
one-time secret display, and confirms revocation. Secrets never enter browser
storage. Administrator account management supports creation, revision-checked
role/disabled changes with acknowledgment, and credentials for other accounts.
Regular accounts cannot navigate to administrative account management. Real
browser qualification exercises issuance, authentication, revocation and account
disabling against the server rather than mocked responses.

### Delegated root management (SCT-56)

Root pages expose settings, retention and grants only for the corresponding
capabilities. Quota changes retain exact revisions and enforce administrator
ceilings; the optional expected interval configures monitoring, never scheduling.
Policy changes use bounded server previews, paged affected-version inspection,
explicit retention acknowledgment and durable lifecycle jobs. Both root defaults
and resolved entry scopes support this flow, including parent-policy inheritance.
Grants expose only delegable capabilities and confirm revocation. Browser tests
exercise a real two-version retirement and verify a delegated reader has no
management controls. Program file-mutation permissions remain distinct from the
browser's management surface.

### Storage administration (SCT-57)

Administrators can provision and resize physical pools and root/staging
allocations, configure local or S3 backend instances, provision/enable roots,
and set delegated quota ceilings and retention constraints. Lists are paged;
updates use inspected revisions and expose server capacity/conflict failures.
Backend destination identity remains immutable; S3 credential fields accept
server environment-variable references only. Root-management inspection now
includes current retention bounds so forms can edit actual values. Real browser
qualification provisions a private local backend and root, changes limits and
allocations, and verifies that shrinking a pool below obligations is rejected.

### Operations browser (SCT-58)

Assigned roots expose logical/unique/physical/staging/reserved/pending-cleanup
usage, transfer sessions, jobs, audit events and integrity findings. Administrators
can start integrity scans or verified mirror repairs with acknowledgment, inspect
failure/retry states, and explicitly resume maintenance after remediation.
Service-wide operations include checkpoint creation, export progress, inventory
paging, native artifact downloads and revision-checked acknowledged pin release.
The UI labels exports catalog-only and explains the separate operator payload
copy/verification and isolated recovery steps; it never claims export success
means recovery qualification. Browser tests verify scan completion, downloaded
artifact hashes and the release guard. Shared-fixture browser tests run serially
because checkpoint catalog fences deliberately block concurrent mutations.

### Observability and backup freshness (SCT-59)

Administrator-only Prometheus metrics expose bounded route/state labels and
service-wide capacity/job/freshness gauges. JSON request logs correlate responses
without logging credentials, payloads, query strings or raw paths. Readiness
requires live writer ownership and an open catalog gate with bounded probes.
Root inspection now reports optional backup freshness and due time, updated only
inside successful file/tree publication, never by commit receipt replay or failed
attempts. Migration 18 backfills existing durable publication audit timestamps;
checkpoint recovery requires the matching version-18 catalog schema/checksums.
See [observability.md](observability.md) for scrape, log and monitoring semantics.

### Standalone management SDK (SCT-60)

The public Rust client now covers account credentials, principals, assigned-root
settings, grants and policy constraints, backend/pool/allocation administration,
usage/activity/audit, integrity/repair maintenance and checkpoint export workflows.
Paginated calls preserve opaque cursor/limit bindings; revisions, acknowledgments
and retry decisions remain explicit. Checkpoint artifacts stream to caller-owned
output and verify descriptor hashes before reporting success. The management HTTP
qualification exercises permissions, pagination, stale revisions, 204 revocation,
checkpoint release acknowledgment and corrupt-artifact rejection. Packaging checks
compile management usage outside the workspace without core/server dependencies.
