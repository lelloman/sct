# Initial Crumbles ticket migration

Implementation update, 2026-09-19: M1 and the approved M2 independent-file pipeline
are implemented. SCT-14–27 have scoped local commits and are handed off for review;
this does not mark the milestone parent complete or change older M1 ticket states.
See `WORK_LOG.md` and `docs/m2-qualification.md`. M3–M5 require decomposition.

Refinement update, 2026-09-19: M2 now includes independent-file history, keep-N
rotation and admission-time relaxed reclamation before payloads. See
[the approved M2 plan](docs/m2-implementation-plan.md) for 14 Tasks
(SCT-14–27), their acceptance criteria and 17 dependency edges. The user approved
the complete breakdown; all children and edges were created and verified. This supersedes older M3-only assignments of basic file
rotation, publication accounting and admission reclamation below.

Source: `ARCHITECTURE_PLAN.md`, dated 2026-09-14. Crumbles project: https://crumbles.lelloman.com/w/LLPR/SCT

Review amendment, 2026-09-15: all five milestone epics and eight implementation tasks were read from Crumbles and reviewed against the provider-independent adapter trust model in architecture §8. The revised task bodies below and epic amendments at the end are the local proposed ticket updates. Remote descriptions remain unchanged: the connected Crumbles tools do not expose an existing-ticket description editor. All 13 tickets were Open at resource version 1 when reviewed; all 18 dependency edges remain applicable. No new tickets or dependency changes are needed for this decision.

Created milestone epics:

| Ticket | Milestone |
|---|---|
| [LLPR/SCT-1](https://crumbles.lelloman.com/w/LLPR/SCT/1) | Contracts and domain foundation |
| [LLPR/SCT-2](https://crumbles.lelloman.com/w/LLPR/SCT/2) | Immutable transfer and storage backends |
| [LLPR/SCT-3](https://crumbles.lelloman.com/w/LLPR/SCT/3) | Versioned trees, retention, and safe reclamation |
| [LLPR/SCT-4](https://crumbles.lelloman.com/w/LLPR/SCT/4) | Web control surface and storage operations |
| [LLPR/SCT-5](https://crumbles.lelloman.com/w/LLPR/SCT/5) | Scale and release qualification |

The following eight direct children of LLPR/SCT-1 were approved and created on 2026-09-14. All are type **Task**, initially **Open**, unassigned, with labels `architecture-plan` and `milestone-1`. All 18 approved dependency links were created and verified. The architecture document remains the detailed contract; no implementation is included in this migration.

| Plan key | Created ticket | Title |
|---|---|---|
| A | [LLPR/SCT-6](https://crumbles.lelloman.com/w/LLPR/SCT/6) | Bootstrap the Rust workspace and repeatable development checks |
| B | [LLPR/SCT-7](https://crumbles.lelloman.com/w/LLPR/SCT/7) | Define v1 protocol contracts and reproducible API generation |
| C | [LLPR/SCT-8](https://crumbles.lelloman.com/w/LLPR/SCT/8) | Establish the PostgreSQL catalog and migration harness |
| D | [LLPR/SCT-9](https://crumbles.lelloman.com/w/LLPR/SCT/9) | Validate configuration and enforce a single active server |
| E | [LLPR/SCT-10](https://crumbles.lelloman.com/w/LLPR/SCT/10) | Implement authentication and root-scoped authorization foundations |
| F | [LLPR/SCT-11](https://crumbles.lelloman.com/w/LLPR/SCT/11) | Implement storage-root provisioning and placement invariants |
| G | [LLPR/SCT-12](https://crumbles.lelloman.com/w/LLPR/SCT/12) | Implement the logical namespace and safe in-root rename |
| H | [LLPR/SCT-13](https://crumbles.lelloman.com/w/LLPR/SCT/13) | Model effective retention and atomic-versioning constraints |

## A — Bootstrap the Rust workspace and repeatable development checks

Source: §§1–2, 11.2, 15 M1.

Create `sct-protocol`, `sct-client`, `sct-core`, and `sct-server` with the prescribed component boundaries; establish Tokio/Axum and SQLx/PostgreSQL wiring, development documentation, and repeatable formatting, lint, build, and test commands. Establish a minimal Vue 3/TypeScript build scaffold for future generated contracts and same-origin serving. Domain behavior belongs in core; object-store adapters handle provider operations and a separate placement coordinator composes them. Runtime wiring selects registered adapter implementations. The client depends on protocol without depending on core/server or another consumer repository.

Acceptance:
- A fresh checkout builds the Rust workspace and web scaffold using documented commands.
- Formatting, lint, and baseline tests run through one documented check entry point.
- An external sample crate can build against the client and protocol without any consumer repository or server dependency.
- No application integration or storage behavior is introduced in the scaffold.
- Establish unit, integration, and E2E entry points, including browser checks against the real server and initial explicitly pending milestone scenarios. Pending scenarios never count as passing acceptance; the full-acceptance command must fail while any remain pending.

Dependencies: none.

## B — Define v1 protocol contracts and reproducible API generation

Source: §§5.1, 7, 11.

Define Rust-owned opaque identifiers, revisions, capabilities/limits, bounded cursor envelopes, errors, and foundation request/response DTOs. Establish OpenAPI and TypeScript generation from those definitions. Document the complete v1 operation map and extension points for later transfer/manifests; unimplemented operations must not appear to be working endpoints.

Acceptance:
- Repeated generation yields identical OpenAPI and TypeScript artifacts, with a check detecting drift.
- Foundation operations use `/api/v1`, safe error messages, request IDs, expected revisions, and explicit retry guidance.
- The error taxonomy covers authorization, paths/manifests, revision/idempotency conflicts, policy transitions, quota/staging/destination failures, expiry, and integrity.
- Entry, version, manifest, and backend identities are distinct; no client DTO exposes backend secrets or physical paths.
- Define retryable `root_busy` and authorized operation/recovery status. Draft contracts preserve entry IDs across snapshots and explicit renames within one storage root; delete-and-create receives a new identity. Physical generations are internal and do not change logical content digests.
- Root placement references backend instance IDs and read preference without hard-coding provider combinations. Transfer status represents adapter acknowledgements, not certified durability; capabilities describe operational limits rather than guarantee negotiation.
- Expose adapter contract documentation and optional instance speed/availability hints, including unknown values, separately from observed health. Define explicit root read order, connection/initial-response timeouts, and transfer progress limits without provider-specific content manifests or a mandatory three-score profile.
- Listing contracts include bounded cursors and namespace revision refresh semantics; Rust/TypeScript fixture tests agree.

Dependencies: A.

## C — Establish the PostgreSQL catalog and migration harness

Source: §§2.2, 3–4, 6, 12.

Add forward-only migrations and persistence for M1 principals, credential/grant metadata, backend destination references, roots, stable namespace entries, and policies/revisions. Document the schema extension points for content graphs, sessions/reservations, jobs/audit, read leases, and checkpoints from §6; their full behavior belongs to later milestones. Build isolated real-PostgreSQL integration fixtures and transaction helpers.

Acceptance:
- Migrations initialize an empty PostgreSQL database and reject unsupported schema states predictably.
- Foreign keys and uniqueness constraints enforce ownership/root boundaries and parent/name identity.
- Indexed parent/name and root queries support bounded keyset pagination.
- Integration tests prove transaction rollback, stable identities, and constraint enforcement against PostgreSQL.
- Payload-store listings never define catalog state; secrets are represented by references or hashes as appropriate.
- Backend instance identity, adapter kind, and root placement membership remain distinct; mirrors are placements rather than provider kinds. Future object-copy records retain opaque adapter receipts separately from integrity verification and deletion/capacity state. Adding a provider does not require provider-specific columns in namespace or retention tables.
- Persist optional instance hints and explicit root read policy independently of content identity; observed health is distinct from expected availability. Adaptive ranking and cross-request cooldown machinery are deferred.
- Model durable root operation ownership, phase, fencing epoch, worker lease, and recovery state. Define extension points for exact physical incarnation records and never-reused generations; delayed writes/deletes must remain attributable after restart. Ownership acquisition and catalog fencing use short transactions, not a database lock held across backend I/O.

Dependencies: A, B.

## D — Validate configuration and enforce a single active server

Source: §§2.2, 12, 14.1.

Implement typed TOML settings with documented explicit environment overrides, startup validation, database connectivity, Axum runtime wiring, same-origin static assets, and an exclusive server-instance lease. Include liveness/readiness foundations and graceful shutdown. Validate infrastructure settings for public URL, OIDC, secret references, destinations, workers, expiry, staging budgets, and headroom; user-facing roots/grants/policies/quotas remain catalog data.

Acceptance:
- Invalid settings fail startup with actionable redacted errors rather than silent defaults.
- Establish registration/configuration-validation extension points with test implementations; real providers follow in M2. Adapter-specific settings and secret resolution stay behind registration. Operational probes report connectivity and I/O problems; there is no universal durability-certification gate.
- Validate explicit read order, connection/initial-response timeouts, progress limits, and optional overall deadlines; absent or unknown instance hints remain valid. Test implementations exercise contract documentation and instance configuration independently.
- Two processes against the same catalog cannot both become writers; lease loss prevents further writes.
- Clean shutdown and process failure permit safe lease recovery, verified with integration tests.
- Distinguish the process-instance lease from durable per-root operation ownership and session lifetime. Establish recovery wiring and epoch validation; a lost worker lease does not silently clear an active root operation. Full upload/job recovery follows in M2/M3.
- Readiness reflects database/lease availability, and private configuration never appears in responses or logs.
- Documented configuration starts the server and serves the web scaffold from the same origin.

Dependencies: A, C.

## E — Implement authentication and root-scoped authorization foundations

Source: §§7.2, 12, 16 security.

Add OIDC login compatible with LelloAuth using issuer+subject identities, secure HttpOnly browser sessions and mutation CSRF protection. Add independently revocable opaque application tokens hashed at rest, root-scoped capability checks, and a local administrator bootstrap/recovery command. Keep authorization in reusable core operations. Full administration UI follows in M4.

Acceptance:
- OIDC identity tests distinguish issuer/subject and do not bind identity to mutable email.
- Browser mutation requests enforce CSRF protection and production cookie security.
- Revoked tokens fail the next request; raw tokens and backend credentials are absent from stored hashes, logs, errors, and API responses.
- List/read, write, policy management, ordinary delete, and purge capabilities remain separate; grants cannot exceed the grantor's authority.
- Negative tests cover cross-user/root identifiers and unauthorized operations; core operations require authorization context.
- Administrator bootstrap and credential recovery work while OIDC is unavailable.

Dependencies: B, C, D.

## F — Implement storage-root provisioning and placement invariants

Source: §§3.1, 8, 12.

Implement authorized root inspection and administrator provisioning of owners, mounts, placement, quotas, defaults, grants, and operational state. Roots reference backend instance IDs; adapter registration owns provider-specific configuration validation and server-side secret references. Establish a backend validation/capability interface using test adapters; real adapters and their operational probes are completed in M2. Administrators choose adapters based on documented guarantees. Roots start disabled and can be administratively enabled with a registered adapter and valid configuration; M1 does not provide production payload transfer.

Acceptance:
- Overlapping logical roots are rejected, and root listings reveal only authorized roots.
- Root identity independently bounds placement, quota, snapshots, and future deduplication/reclamation.
- Single-backend and exactly-two-destination synchronous mirrored placements validate structurally through instance IDs, independently of provider kind; known destination aliases are rejected and read preference is explicit.
- Local and S3 are the first production kinds, delivered in M2. A test registration proves that a new kind needs no changes to root placement or domain policy models; durability guarantees are documented by the adapter and assessed by the administrator.
- Populated roots reject destination changes; credential rotation for the same destination is allowed.
- Test adapters exercise administrative enablement and operational probe results. No certification of durability, consistency, or independent failure domains is required for enablement; invalid configuration and unregistered kinds still fail validation.
- Root inspection exposes adapter documentation, optional instance speed/availability hints, and explicit read policy. Administrators can prefer a fast/intermittent copy with short connection/initial-response timeouts and suitable transfer progress limits before a slower/normally-available mirror. Hints do not automatically reorder reads, and preference does not relax the requirement for both write acknowledgements.
- Expected-revision tests reject stale root/grant mutations, and secret values are never returned.
- Provide the root mutation ownership primitive and authorized busy/recovery inspection using test operations. Competing same-root mutations return `root_busy`; unrelated roots progress. Abort/expiry recovery must act on the owning operation without acquiring a competing lock; payload lifecycle integration follows in M2/M3.

Dependencies: B, C, E.

## G — Implement the logical namespace and safe in-root rename

Source: §§3.2, 4.2, 11.1.

Implement core and foundation HTTP operations for directory creation, authorized path resolution, metadata, paginated listing, and in-root rename using stable entry identities. Preserve case-sensitive UTF-8 names and revision checks. Introduce policy/versioning-boundary guards; content upload, hiding/purge, and populated atomic-tree mutation are delivered in later milestones.

Acceptance:
- Reject empty components, dot/dot-dot, NUL, traversal, unsupported entry kinds, and cross-root moves.
- Rename preserves identity/history references and rejects cycles, collisions, and stale revisions.
- Entry IDs persist across snapshots within one root; delete-and-create at the same path obtains a new ID. Define draft validation extension points for explicit identity continuity; paged snapshot execution follows in M3. Namespace mutations acquire root operation ownership and reject competing mutations with `root_busy`.
- Cursors are bounded and return an explicit refresh requirement after conflicting directory mutation.
- Renames cannot silently change effective retention: differing policy boundaries require explicit resulting policy and acknowledged effects, or are rejected.
- Ordinary rename cannot move entries into/out of atomic-versioning scopes.
- Integration tests cover authorization, case sensitivity, malformed paths, pagination, and concurrent rename conflicts without payload-store scans.

Dependencies: B, C, F, H.

## H — Model effective retention and atomic-versioning constraints

Source: §§4, 9.2–9.3, 16 policies.

Implement reusable policy value types, persistence/resolution, expected policy revisions, administrator constraints, and core validation for independent-file versus atomic-directory scopes. Define strict/relaxed eligibility and policy-transition guards as domain rules. This task establishes the model; destructive policy preview/apply jobs, version rotation, quota admission, and reclamation execution follow in M3.

Acceptance:
- Default policy is strict keep-one; keep-N includes current content, and retention order is server commit order.
- Nearest explicit directory policy applies, with a file override taking precedence; atomic descendants cannot have independent policies/history.
- Atomic scopes cannot nest, overlap, cross roots, or change mode when populated.
- Tests cover keep-one/keep-three, fewer initial versions, inheritance/override, stale revisions, and administrator constraints.
- Latest content remains protected in strict and relaxed modes even when hidden; tombstones do not consume content-version slots.
- Policy transitions retain root mutation ownership and reject competing same-root mutations while reads and unrelated roots remain usable. Increasing counts never invents pruned history; finer scope-level mutation concurrency is deferred.

Dependencies: B, C, F.

## Milestone epic amendments

These are exact proposed edits to the existing epic descriptions, reviewed at resource version 1. Preserve their titles, scope outside these edits, labels, hierarchy, workflow state, and sequence. The task descriptions above provide the revised bodies for A–D and F–H; E needs no description change. Preserve existing implementation-delivery instructions when applying those bodies.

### LLPR/SCT-1 — Contracts and domain foundation

Append to outcome and scope:

> Model roots through backend instance IDs and provider-independent placement. Establish adapter registration/configuration extension points with test implementations; real local and S3 adapters follow in M2. Keep provider-specific settings out of namespace, retention, and client content contracts.

> Separate documented adapter guarantees, optional instance speed/availability hints, and explicit root read order/timeouts. Provide safe inspection DTOs and configuration; distinguish expectations from observed health. No mandatory three-score profile or adaptive selection system is required.

Append to exit criteria:

> Configuration and provisioning validate operational requirements without a universal durability-certification gate. Administrators choose adapters according to their documented guarantees.

> Establish root mutation ownership/recovery metadata, epoch validation, `root_busy` contracts, and stable entry identity across snapshots/renames. Test same-root exclusion and independent-root progress with test operations. Backend I/O never holds a long-running database transaction.

### LLPR/SCT-2 — Immutable transfer and storage backends

Replace `local and S3 adapters, synchronous local+S3 mirrors` with:

> the common object-store interface, backend registry implementations, initial local and S3 adapters, and synchronous two-destination mirrors independent of provider kind

Replace `required-copy verification before publication` with:

> byte-integrity verification and required adapter acknowledgements before publication

Replace the first exit-criteria sentence with:

> Single-local, single-S3, and mirrored placements pass shared operational transfer tests against real filesystem/S3-compatible storage. Test adapters prove provider extension without domain changes, same-kind mirror composition, and publication ordering under differing documented acknowledgement semantics. Each production adapter documents and tests its own behavior; these checks do not certify provider durability.

Append to outcome and scope:

> Implement bounded sequential read fallback in explicitly configured order. Bound connection/initial-response delay separately from transfer progress, using idle limits and a minimum-throughput window with startup allowance; a healthy slow transfer must not inherit a short connection timeout as a complete-chunk deadline. Respect cancellation and any overall deadline. Verify chunks before release and retry the same immutable chunk on fallback; late results cannot corrupt output. After failure, retain fallback selection for that read rather than retrying the failed candidate on every chunk. Offline observations alone do not imply lost content or trigger repair. Both write acknowledgements remain required. Adaptive scoring and cross-request cooldown machinery are deferred.

Append to exit criteria:

> Deterministic test adapters exercise fast success, offline/stalled/trickling candidates, a healthy slow transfer, corruption, slow cancellation, discarded late results, fallback success, and all-copy failure. Assert configured order even with absent or inaccurate hints, no repeated failed attempt per chunk, bounded failure handling and memory, exact downloaded bytes, and no publication when either required write acknowledgement is missing.

### LLPR/SCT-3 — Versioned trees, retention, and safe reclamation

Append to outcome and scope:

> Snapshots are complete logical trees with shared immutable structure, not replayable delta chains. Drafts explicitly preserve stable entry IDs across versions and renames within one storage root; new entries at deleted paths get new IDs. Validate base identity continuity and duplicate/cross-root IDs. Serialize retirement, purge, policy transitions, reclamation, physical deletion, and repairs through durable root mutation ownership. Read pins remain usable and serialize with version retirement in short metadata transactions.

Append to exit criteria:

> Test both orderings of read-pin acquisition versus retirement, same-root upload/cleanup exclusion, independent-root progress, abort/expiry recovery without deadlock, and stale-worker fencing. Deliberately finish a deletion of an old generation after a replacement upload and prove the replacement remains readable. Test crash/retry boundaries and charge unresolved physical deletion until adapter-reported capacity recovery.

Replace `Strict rotation never deletes protected history before durable replacement.` with:

> Strict rotation never deletes protected history before all required adapters acknowledge the replacement and publication commits.

Replace `physical usage remains charged until deletion is confirmed` with:

> physical usage remains charged until the adapter reports capacity release according to its documented semantics; logical deletion, delayed recovery, and unknown provider capacity remain distinguishable

In SCT-2, also append to outcome and scope:

> Upload sessions hold durable exclusive root mutation ownership from creation through commit, abort, or expiry, including backend I/O and resumable interruptions. Persist operation phase/epoch and exact physical incarnation before issuing backend work. Recreated or replacement objects use never-reused generation-specific addresses; stale worker receipts cannot update current catalog state. Recovery reclaims the owning operation rather than treating lease expiry as an idle root. Implement durable expiry/reconciliation foundations here because safe transfer already depends on them; later milestones extend the job framework.

In SCT-2, also append to exit criteria:

> Root-busy responses are retryable; interrupted owners remain resumable; abort and expiry release ownership through recovery; unrelated roots and pinned committed reads continue. Fault-injection tests cover uncertain writes and a late old-generation delete after replacement creation. Concurrent chunk transfers within the owning session remain bounded.

### LLPR/SCT-4 — Web control surface and storage operations

Append to outcome and scope:

> Backend administration uses registered provider kinds and instance-based placement, including same-kind mirrors and read preference. Make adapter documentation available when selecting destinations, including acknowledgement, consistency, durability, and deletion behavior.

> Display optional instance speed and availability hints separately from observed health. Expose explicit read order and connection/initial-response/progress settings, and show fallback outcomes without presenting expected availability as a guarantee or automatically deriving read order from hints.

Append to exit criteria:

> The UI distinguishes per-adapter acknowledgement from integrity verification and does not label publication as independently certified durability. Operational probes do not create a durability-approval workflow. Unknown or delayed provider capacity recovery is visible.

> Show the current root mutation owner, busy/recovering state, and retry guidance. Repair jobs acquire the same root ownership as uploads and cleanup. Extend the durable worker infrastructure introduced for safe transfer; do not defer its foundations until M4.

### LLPR/SCT-5 — Scale and release qualification

Replace `real-backend qualification` with `real-backend integration tests and adapter behavior documentation`.

Replace `qualify AWS S3 before claiming AWS support` with:

> test AWS S3 before claiming AWS integration support, recording tested endpoints and adapter limitations

Append to exit criteria:

> Shared suites verify common operations and adapter-specific tests cover documented acknowledgement, consistency, retry, and deletion behavior. They establish tested compatibility rather than certifying provider durability or imposing runtime enablement gates. Administrator documentation explains the adapter trust model.

> Measure fast/intermittent-to-slow/available read fallback under injected latency, stalls, and outages. Record configured hints/budgets and measured outcomes separately; verify bounded fallback and document the latency/availability tradeoff.

## Review coverage and unresolved design work

| Tickets | Review result |
|---|---|
| SCT-1–5 | Epic amendments above cover foundation, transfer, reclamation, administration, and release wording |
| SCT-6 / A | Separate adapter operations, placement coordination, and runtime registration in component boundaries |
| SCT-7 / B | Provider-independent placement DTOs and acknowledgement semantics |
| SCT-8 / C | Backend instances and placement membership; opaque copy receipts and accounting extension points |
| SCT-9 / D | Registration/configuration extension points and operational probes without durability certification |
| SCT-10 / E | Reviewed; authorization and secret-handling requirements already apply independently of provider |
| SCT-11 / F | Replace fixed local-plus-S3 placement and qualification gates with configurable instances and administrator-selected adapters |
| SCT-12 / G | Stable IDs across snapshots/renames and root ownership for namespace mutations |
| SCT-13 / H | Policy transitions serialize whole-root mutations; reads and other roots remain usable |

Decisions now include stable entry IDs across snapshots within a root, durable root-level mutation serialization, and generation-specific physical addressing for late backend operations. Define detailed crash/recovery state transitions and prove them with unit, integration, and E2E tests during implementation. Durable expiry/reconciliation foundations belong with M2 transfer; M3/M4 extend that infrastructure. Dropbox or another additional production adapter remains future work, not a new delivery commitment.

Capacity decision: administrators define storage pools, allocate roots from them, and assign roots to users. Each pool has a separate logical staging allocation owned by the server and shared by users; optional fairness caps do not create user-owned staging roots. For 10 GB of allocatable capacity, two User A roots may receive 5 GB and 4 GB, leaving 1 GB explicitly reserved for staging. Safety overhead is excluded before allocating. No separate physical partition or staging-copy path is required; the default stages directly into required destinations under temporary session ownership.

Staging reservations cover new unique bytes for the complete upload. Every required mirror destination needs sufficient pool capacity; shared staging does not enable cross-root deduplication or access. Publication transfers retained-byte accounting to the root, but outgoing copies, pinned history, abandoned uploads, and uncertain I/O keep temporary capacity occupied until cleanup releases it. Abort/expiry release unused reservations, not bytes still physically occupied. Coarse root locking does not replace atomic shared-pool reservations. Exact shared-graph accounting and ledger implementation remain to be designed.

### Shared staging ticket amendments

These additional local amendments apply to the existing tickets; remote descriptions remain unchanged.

- **SCT-1 / M1:** establish pool/allocation models and shared staging contracts with test reservations.
- **SCT-7 / B:** expose authorized pool/root/staging allocation and reservation status; distinguish retained-root quota from shared staging exhaustion and destination capacity. Staging is an internal allocation, not a browsable root.
- **SCT-8 / C:** persist pools, root allocations, server staging allocations, and reservation-ledger extension points; enforce non-overcommit and protect outstanding obligations on resize. Backend instance identity remains distinct from capacity-pool identity.
- **SCT-9 / D:** validate pool membership, capacity budgets, overhead, and staging allocation configuration; optional user/root fairness caps limit shared access rather than assign staging ownership.
- **SCT-11 / F:** provision allocated roots and assign them to users; expose server-owned staging separately and reject over-allocation. Test the 5/4/1 GB example on a 10 GB allocatable pool.
- **SCT-2 / M2:** reserve shared staging atomically across roots/users and all required destination pools; enforce session isolation and the complete-upload staging allowance. Persist reservations through restart. Abort/expiry retain charges for uploaded or uncertain bytes pending cleanup.
- **SCT-3 / M3:** commit/retirement accounting transfers new retained bytes without inventing free capacity; outgoing or pinned copies remain charged until capacity recovery. Test concurrent roots, delayed cleanup, rollback, and read/checkpoint pins. Detailed shared-graph calculations still require design.
- **SCT-4 / M4:** administer and display pool allocation totals, root assignments, shared staging reservations and cleanup obligations, and blocked uploads with appropriate permissions.
- **SCT-5 / M5:** include E2E shared staging exhaustion/recovery, mirror pool exhaustion, concurrent users, crash recovery, and allocation-resize validation in full acceptance.

Decision: every configured mirrored destination is a permanent required replica. An unavailable destination prevents successful publication, while an upload may retain resumable progress. A fast cache would be a separate future optimization; it does not replace required storage. Permanent archival/cold-storage intent does not itself select asynchronous archive-tier retrieval.

Delivery decision: build the entire planned system with unit, integration, and E2E tests, using end-to-end acceptance scenarios to drive implementation. All five milestones remain required. Amend task A/SCT-6 and epic SCT-1 to establish test entry points, the E2E harness, and the initial executable acceptance scenarios; later milestones extend them against real implementations. Pending scenarios must not count as passing acceptance. Apply the unit/integration/E2E acceptance strategy across all milestone epics; no implementation is complete solely because its unit tests pass.

## Approved dependency graph and execution order

These `blocks` edges are recorded in Crumbles (arrows point from prerequisite to dependent):

```text
A -> B, C, D
B -> C, E, F, G, H
C -> D, E, F, G, H
D -> E
E -> F
F -> G, H
H -> G
```

Suggested execution order: A, B, C, D, E, F, H, G. Creation order A–H is only for stable mapping. No other typed relations or assignments are proposed in this batch. Milestones 2–5 remain roadmap epics for later refinement.

For each implementation task, inherited LLPR instructions require one scoped commit with a Markdown ticket link in the commit body. Implementation acceptance includes relevant checks and that repository artifact; creating these tickets does not mark implementation complete.
