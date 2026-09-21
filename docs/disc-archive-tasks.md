# Approved implementation tickets: append-only disc archives

Parent: [LLPR/SCT-62](https://crumbles.lelloman.com/w/LLPR/SCT/62)

Approved and created: SCT-63–79 with all 21 blocking edges, assigned to Codex. Delivery is in progress under SCT-62. Crumbles is authoritative for current ticket status.

All 17 direct children are **Task** (schema type 3), initial **Open**, assignee **Codex (user 2)**, supervisor **lelloman (user 1)**, priority **Medium**, labels **disc-archive**. Keys D01–D17 below map in order to SCT-63–79. Each repository-changing implementation task produces its own ticket-linked commit.

## Agreed invariants

Every disc is a recovery entry point: software, instructions, full target index and content-to-disc map are repeated on every image. Missing final/middle discs limit available data, never access to the index or recovery tool. Recover complete files, report missing chunks precisely, and never label a partial snapshot complete. Disc collections and historical recovery points are append-only and independent of online retention. The browser manages exports without mutating source snapshots.

## Approved v1 defaults

- Collections belong to ScT user accounts; restore runs locally without ScT.
- Capture explicit committed file/tree versions or a frozen current-view selection; keep root identities and do not claim application-wide atomicity across roots.
- Ship and test Linux x86_64 recovery software first, plus source and format documentation. Other platforms need separate executable qualification.
- Media capacity is an explicit byte budget including filesystem, index and tool overhead. Tiny budgets are supported for tests.
- One active append per collection. Pin deadlines and account/server staging ceilings are explicit configuration; expired/cancelled work is visibly incomplete, never silently complete.
- No direct burner control, bootable operating system, parity repair, disc retirement or broad platform-compatibility claim.
- Existing M5 release work remains separate.

## Approved breakdown

### D01 / SCT-63 — Specify the portable archive format and compatibility fixtures

Type: Task. Depends on: none.

Define a versioned, database-independent archive format: collection/set/disc identities; root-scoped object identities; complete snapshot index; complete content-to-disc map; local inventories; archive-prefix identity; checksums; reader capabilities and compatibility rules. Every disc includes the complete target index and recovery instructions, with no privileged final disc. Archive histories refer only to their own set or earlier sets.

Acceptance criteria:

- Canonical fixtures represent empty directories, renames, deletions, shared chunks, files spanning discs, multiple sets and arbitrary surviving subsets.
- A reader can enumerate the full target snapshot from any disc's metadata without loading another index; descriptor hashes have no whole-image self-reference.
- Specify all size/count/version limits, safe path encoding, schema-independent compatibility and explicit unsupported-format errors; complete metadata/software must fit every disc.
- Commit the agreed design and format specification to the repository with fixture checksums and no implicit dependency on SQL migration versions.

### D02 / SCT-64 — Define archive API, SDK contracts and lifecycle states

Type: Task. Depends on: D01.

Define public DTOs, IDs, errors and generated OpenAPI/TypeScript contracts for collections, captures, plans, jobs, logical discs, artifacts, physical copies and verification reports. Distinguish planning/build/ready/downloaded/reported-burned/read-back-verified observations instead of one misleading linear state. Model current media health separately from historical completion.

Acceptance criteria:

- Document allowed transitions, expected revisions, idempotency, pagination and bounded progress/cancellation endpoints.
- Preview inputs include selected committed versions and explicit media byte capacity; output includes new/reused bytes, repeated metadata/tool overhead and required discs.
- Schemas distinguish complete snapshot recovery from complete-file partial recovery, missing objects and unavailable artifacts.
- Standalone client can compile against the public contracts without core/server dependencies; generated contracts remain reproducible.

### D03 / SCT-65 — Persist account-owned collections and append-only disc history

Type: Task. Depends on: D02.

Add catalog schema and core operations for collections, recovery points, frozen plans, jobs, logical discs, immutable image records and physical copies. Collections belong to accounts; source root IDs remain explicit. Use permanent UUIDs plus monotonic display numbers, one active append per collection and preserved aborted/unfinished records.

Acceptance criteria:

- Restart preserves identities, inventories and history; concurrent append requests serialize without reusing a published disc number.
- One logical disc supports multiple physical copies; lost/damaged observations never delete history or silently invalidate dependencies.
- Freeze prevents content/assignment edits after publication; revisions reject stale edits and replay keys return stable results.
- Online retention does not delete archive history; migrations and integrity constraints are covered by real PostgreSQL tests.

### D04 / SCT-66 — Enforce archive authorization and expose collection management

Type: Task. Depends on: D03.

Implement account-isolated collection/history/disc APIs with explicit administrator access and source read/export authorization. Collection ownership alone never grants root access. Recheck authority for async work and artifact downloads; specify revocation behavior and audit management actions.

Acceptance criteria:

- An unrelated user cannot list, inspect, plan against or download another account's archive by guessing any ID.
- Users can archive only currently authorized source content; mixed-source artifacts require authority for all included sources.
- Revocation denies new reads/downloads and suspends or terminates affected work according to documented state transitions; it cannot revoke offline copies already obtained.
- Browser sessions may manage archives and download exports while existing bans on source snapshot upload/edit/delete/restore remain enforced.

### D05 / SCT-67 — Capture stable source versions and manage bounded export pins

Type: Task. Depends on: D04.

Create durable, resumable captures of explicitly selected committed trees/files or a frozen current-view selection. Capture version identities consistently before graph enumeration, protect required content against retention/deletion races and preserve bounded work. Configure per-account resource ceilings and a documented pin deadline/cancellation policy.

Acceptance criteria:

- Concurrent uploads, renames, purges and retention cannot change a frozen recovery point or reclaim content the active exporter still needs.
- An atomic tree retains its native snapshot semantics; a multi-root file selection reports its captured version IDs without claiming application-wide atomicity.
- Crash/restart resumes bounded enumeration with generation/fence checks, and no long-lived database snapshot or unbounded HTTP operation is required.
- Cancellation, expiry and revocation stop workers before releasing owned pins; abandoned work cannot reserve bytes indefinitely. Expiry is visible and never edits completed optical history.

### D06 / SCT-68 — Plan incremental reuse and capacity-safe disc layouts

Type: Task. Depends on: D05.

Compute required object closure, reuse content from completed verified earlier sets within the same collection/root scope, and assign new objects to discs. Include the complete index, map, software and filesystem overhead on every disc. Freeze the whole set's identities/assignments before any image is published.

Acceptance criteria:

- Identical content is reused without introducing cross-root/account deduplication; incomplete or known-unreadable earlier archives are not silently assumed available.
- Deterministic previews explain which old discs remain required and which new bytes go on each new disc; later sets never modify earlier indexes.
- Serialized metadata, mapping growth and exact writer size checks converge within configured capacity; report a clear failure if the repeated recovery bundle cannot fit.
- Prefer file locality where possible; test boundary sizes, files larger than a disc, shared chunks, empty snapshots and interrupted planning.

### D07 / SCT-69 — Build a standalone archive reader and media verifier

Type: Task. Depends on: D01.

Add a standalone recovery package that reads the archive format from mounted/extracted disc roots, inspects snapshots/dependencies and verifies media/object identities. It must not link to ScT server/database state. Maintain a disk-backed index and stream payload validation.

Acceptance criteria:

- Start from every disc in a fixture set and obtain the same full target index and required-media map offline.
- Accept discs in any order and duplicate physical copies; reject wrong collection/disc identity, contradictory metadata and corrupted objects.
- Only successfully verified surviving copies satisfy object availability; return bounded machine-readable and human-readable missing/corrupt reports.
- Test untrusted path metadata, excessive lengths/counts, malformed formats and bounded memory; unsupported formats fail before recovery writes.

### D08 / SCT-70 — Implement resumable complete and degraded offline restoration

Type: Task. Depends on: D07.

Restore into an explicit local destination using the reader's verified availability map. Support all required media for a complete snapshot, or an arbitrary surviving subset for complete-file partial recovery. Preserve stored directory structure/metadata and record resumable progress.

Acceptance criteria:

- With disc 10 absent, start from disc 9 and recover every complete file supported by discs 1–9 and available earlier sets; no server, database, network or missing index is needed.
- Report exact incomplete files and missing chunks, including shared/spanning chunks; never report partial output as a complete snapshot or silently use an older file version.
- Resume after interruption without trusting unverified destination bytes; protect against path traversal, symlink substitution and unintended overwrites.
- Recovered complete files match independently computed hashes; preserve empty directories; clearly distinguish snapshot completeness from application consistency.

### D09 / SCT-71 — Package an offline recovery bundle for inclusion on every disc

Type: Task. Depends on: D08.

Produce versioned recovery bundles with a tested Linux x86_64 executable, readable instructions, archive-format documentation, source and licensing notices. Record bundle size, version and hashes as fixed planner inputs. The included executable must run without a compiler, service installation or dependency download.

Acceptance criteria:

- The exact bundle included on a disc runs in a clean supported runtime with networking disabled.
- Inspect, verify, complete restore and degraded restore are exercised using the packaged executable rather than cargo-run or a developer installation.
- Document runtime/OS requirements and artifact provenance; unsupported platforms are explicit rather than implied by shipping source.
- Bundle identity is fixed in frozen plans and every disc contains it; a subsequent tool release never mutates an existing ISO.

### D10 / SCT-72 — Generate verified immutable ISO images with resumable workers

Type: Task. Depends on: D06, D09.

Implement a bounded image-writer adapter and durable export worker using frozen plans and verified source reads. Stage output, construct images, reopen/inspect actual image contents, verify checksums/layout, enforce exact byte capacity and publish atomically. Writer version/options are captured with build metadata.

Acceptance criteria:

- Every emitted ISO contains recovery software plus the complete snapshot index/map, not only its own payload inventory.
- Published image bytes/hash never change; download-ready status cannot reference a partial or oversized image.
- Crashes, full staging storage, source failures and writer failures are recoverable or explicitly failed without leaking pins/reservations or falsely completing the archive.
- Tests independently extract/read generated images, validate path/identity mappings and prove every image respects the selected capacity.

### D11 / SCT-73 — Deliver authorized ISO downloads and manage export resources

Type: Task. Depends on: D10.

Expose durable export progress, cancellation and immutable artifact downloads through API/SDK, with bounded staging/cache budgets. Implement source-pin release and artifact-cache cleanup separately from permanent archive history; quotas must be enforced before starting costly work.

Acceptance criteria:

- Interrupted and range downloads return bytes from the same verified immutable image; authorization is rechecked and generated artifacts are never publicly exposed.
- Concurrent jobs cannot exceed configured account/server staging ceilings; cancellation and cleanup serialize with active builders/downloads.
- Source pins release only at the documented safe transition or explicit cancellation/expiry; removal of cached ISOs never deletes disc/recovery-point records.
- UI/API accurately report cached-image availability and source availability; never promise regeneration after needed bytes or build inputs are unavailable.

### D12 / SCT-74 — Track physical copies and ingest bound read-back verification reports

Type: Task. Depends on: D04, D07.

Add physical-copy labels/locations, reported-burn observations, damage/loss observations and local read-back reports. Extend the standalone verifier to scan actual mounted media and emit reports bound to collection/disc/content identities. Reports are owner-supplied observations, not cryptographic proof of physical possession.

Acceptance criteria:

- ISO creation/download alone cannot mark a burn verified; reported burn, successful read-back and later damage remain distinguishable.
- Reject reports for the wrong frozen inventory; report partial burns/corruption; idempotent imports do not duplicate copies or observations.
- A set becomes historically completed only when required new discs and dependencies satisfy the defined verification policy; later damage updates availability without erasing that event.
- Duplicate physical copies can satisfy the same disc requirement; never offer automatic retirement of a disc because the latest snapshot does not use it.

### D13 / SCT-75 — Preserve archive catalogs through service and offline recovery

Type: Task. Depends on: D11, D12.

Integrate new archive records and active-work recovery rules with administrator checkpoint export/recovery, without putting administrative data into user ISOs. Add standalone metadata inventory/export from scanned discs so collection identities, recovery points and content placement can be reconstructed independently of the service database.

Acceptance criteria:

- Service checkpoint recovery preserves completed archive history and resumes/fails active jobs safely without stale pins or writer ownership.
- Scanning any disc from a set reconstructs that set's complete target index and disc map; scanning representatives of historical sets preserves their separate recovery points.
- Never fabricate unknown physical-copy observations, unread scanned media or historical snapshot indexes absent from supplied metadata; report incomplete catalog coverage.
- Service migration/schema changes do not prevent the bundled archive reader restoring old fixture images; restored catalog identities never collide or silently rebind source accounts.

### D14 / SCT-76 — Add collection, snapshot-history and disc-inventory web views

Type: Task. Depends on: D12.

Add account archive navigation, collection creation/settings, recovery-point timeline, searchable/paginated disc inventories and physical-copy details. Show archived content separately from current online availability and expose required-disc relationships.

Acceptance criteria:

- Users can answer which disc number comes next, what each disc contains and which historical snapshots require it.
- Show latest completed recovery point, unfinished sets, source selection, missing/damaged copies and current artifact availability without conflating them.
- History survives online deletion and no retire/delete-snapshot action appears; administrators and regular users see only authorized controls.
- Browser E2E covers account isolation, pagination, historical dependencies and a disc unused by the newest snapshot but required by an older one.

### D15 / SCT-77 — Add next-set planning, ISO export and verification web flows

Type: Task. Depends on: D11, D14.

Connect source selection, next-set preview, media byte budget, new/reused data, per-disc assignments, generation progress, ISO downloads and verification-report recording. Explain what a selected surviving prefix/subset can restore.

Acceptance criteria:

- Preview displays every disc's repeated index/software overhead and required earlier discs before a plan is frozen.
- Revision conflicts, failed builds, cancellation, quota exhaustion and resumable downloads have usable states; no completed-burn claim follows a download.
- Users can label copies, record reported burns and submit verifier reports without uploading backup payloads or mutating source snapshots.
- End-to-end browser flow produces/downloads an image and records correctly bound verification observations against the actual backend.

### D16 / SCT-78 — Qualify complete, historical and missing-disc recovery in Docker

Type: Task. Depends on: D13, D15.

Extend the isolated qualification harness with real generated/extracted ISOs, the packaged recovery executable, service/database isolation and fault injection. Use tiny configurable media budgets for ten-disc fixtures plus larger scale fixtures for bounded catalog and payload behavior.

Acceptance criteria:

- Create archives A/B/C with edits, renames and deletions, then disable original service/database/storage; restore each historical snapshot starting independently from EVERY disc in its own set.
- For a ten-disc set remove disc 10 and start from 9; repeat with missing middle media, arbitrary order, duplicate copies, corrupt bytes, shared chunks and spanning files. Compare exact recovered file sets/bytes and missing reports.
- Interrupt burning/export after disc 9 and recover useful partial contents even though the set was never marked complete; test restarting workers during concurrent online retention and grant revocation.
- Exercise exact image capacity, absent tools, clean-runtime offline execution, resource ceilings and large indexes; retain logs/artifacts and never count skipped hardware checks as passes.

### D17 / SCT-79 — Document operations and record real optical-media qualification

Type: Task. Depends on: D16.

Publish collection planning, labels, export, external burning, read-back verification, partial recovery, catalog recovery and archive-format compatibility documentation. Establish release artifacts and a reproducible physical-media drill separate from automated ISO qualification.

Acceptance criteria:

- A clean supported machine can follow the included instructions and restore without ScT or internet; all examples match the shipped CLI/UI.
- Record an actual burn/read-back/restoration drill with available drive/media and tested software versions, including starting recovery from a non-final disc.
- If hardware is unavailable, report that precise remaining qualification blocker and do not claim this task or hardware support complete; ISO-only coverage remains separately recorded.
- Document observed resource limits, metadata-capacity failure behavior, pin/cache policies, and limitations: no retirement, automatic burner control, bootable OS or parity repair in this scope.

## Exact dependency graph

Each line means **left blocks right**. These are the approved typed dependency edges; each task is also a direct child of SCT-62.

```text
D01 -> D02
D02 -> D03
D03 -> D04
D04 -> D05
D05 -> D06
D01 -> D07
D07 -> D08
D08 -> D09
D06 -> D10
D09 -> D10
D10 -> D11
D04 -> D12
D07 -> D12
D11 -> D13
D12 -> D13
D12 -> D14
D11 -> D15
D14 -> D15
D13 -> D16
D15 -> D16
D16 -> D17
```

## Suggested execution order

1. D01.
2. D02–D06 for catalog/source/planning; D07–D09 for standalone recovery and its bundle can proceed independently after D01.
3. D10–D11 after planning and bundle; D12 after authorization and reader; D14 after physical-copy APIs.
4. D13 and D15 once their dependencies are complete.
5. D16 automated qualification, then D17 documentation and actual hardware qualification.

The user approved this plan, assigned every task to Codex and authorized implementation. No parallel-agent delegation was requested.
