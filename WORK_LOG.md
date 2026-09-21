# Foundation implementation

Scope: complete SCT-8, SCT-9, SCT-10, SCT-11, SCT-13, SCT-12 in dependency order. M2–M5 remain out of scope.

Each ticket receives a scoped commit with its Crumbles link and relevant verification. Acceptance is tracked here and in focused PostgreSQL, HTTP, and browser tests.

- [x] SCT-8: forward migrations, constrained catalog, isolated database tests, extension points.
- [x] SCT-9: validated private configuration, adapter registry, exclusive fenced server lease, readiness/shutdown.
- [x] SCT-10: issuer/subject OIDC, secure sessions/CSRF, hashed revocable tokens, reusable authorization, local recovery.
- [x] SCT-11: authorized root/grant provisioning, placement validation, pool allocations, root operation ownership.
- [x] SCT-13: effective policies, administrator bounds, atomic scopes and transition guards.
- [x] SCT-12: namespace operations, bounded authenticated pagination, identity/revision/policy-safe rename.
- [x] Final: generated contracts, SDK, executable M1 acceptance, full development checks and documentation.

Design choices and test results will be recorded as implementation proceeds.

SCT-8 verification: real PostgreSQL tests passed for repeat migration, checksum mismatch rejection, rollback, principal/root foreign keys, case-sensitive unique names, cross-root parent rejection, and the 10,000 = 5,000 + 4,000 + 1,000 allocation example. Each integration case creates an isolated schema in the disposable database.


## Final foundation result

- SCT-9: typed configuration and registered test adapters; exclusive 30-second server lease with 10-second renewal; transaction fencing, readiness, graceful shutdown and crash recovery. Real process tests reject a second writer and verify local recovery while OIDC is unavailable.
- SCT-10: issuer/subject identity, PKCE plus browser-bound one-use login state, signed OIDC verification, hashed/revocable credentials, secure production session cookies and Origin/CSRF checks. Core authorization is reused by HTTP and local administration. A signed mock provider tests invalid issuer, audience, expiry, nonce, signature and access-token hash.
- SCT-11: disabled-by-default roots, explicit enablement, owner/grant authority, revision checks, destination/pool distinction, same-pool mirror charging, bounded allocation inspection, durable operation ownership, recovery fencing and unused staging reservations. Concurrent cross-user allocation attempts cannot overcommit a pool.
- SCT-13: inherited policies, strict keep-one defaults, administrator bounds, metadata-only revision-checked policy edits, atomic scope restrictions and transition exclusion. Latest-content protection is independent of hidden namespace state. Content transitions remain M3.
- SCT-12: authorized resolution, metadata, directory creation, bounded keyset listing and stable-ID rename. Tests cover Unicode/case sensitivity, cross-root parents/moves, cycles/collisions, stale and concurrent revisions, cursor isolation/invalidation, policy boundaries and disabled roots.
- Integrated foundation: authenticated Rust SDK, generated OpenAPI/schema/TypeScript, fixed error envelopes (including framework rejections), configured HTTP runtime/local CLI, disposable PostgreSQL browser fixtures and executable M1 acceptance.

Verification: `./scripts/check` passed (formatting, Clippy, Rust tests/builds,
contract drift and 49 TypeScript fixtures, web checks/build, independent SDK
packaging, PostgreSQL/HTTP/OIDC/process integration, and Chromium). Final additional
concurrency/isolation regressions also passed: 11 database-backed tests in total.
The browser suite has three passing tests including the real M1 workflow.
`npm --prefix web run test:acceptance` exits 1 intentionally: M1 passes and the
four M2–M5 scenarios remain pending. No later milestone gate was removed.

The six implementation tickets have scoped local commits with ticket links.
Repository documentation is updated; remote ticket descriptions/statuses and
repository hosting were not modified. No deployment or push was performed.

Operational limits and M1/M2 boundaries are documented in `docs/foundation.md`.
The remaining readiness decisions in `docs/implementation-readiness.md` still apply
to their dependent milestones.

## M2 refinement — 2026-09-19

Agreed metadata-first admission with same-root relaxed-history reclamation before
payloads; latest/strict/read-pinned versions remain protected. Reclamation does
not expand staging allocation, and pruned history remains deleted if upload fails.
M2 includes basic file history and keep-N; M3 extends this with atomic trees,
delete/purge and broader policy transitions. Updated local scope/readiness notes.

Drafted `docs/m2-implementation-plan.md`: 14 proposed Tasks, full acceptance
criteria and 17 proposed dependency edges. Crumbles refinement inspection found
no existing M2 children or relations. The complete plan awaits user approval
before remote creation. No application code, remote tickets or workflow states
changed during this refinement.


M2 plan approval recorded, 2026-09-19: created exactly 14 direct Task children
SCT-14–27 and 17 approved `blocks` edges. Read back every child's relations and
the parent child list; both match the approved plan. All children are Open with
Medium priority and the approved labels; assignment/estimates remain unset.
SCT-2 remains Open. Added ticket links and refreshed local planning status.
No M2 implementation was started.

## M2 implementation in progress

Scoped commits: SCT-14 a7e5f10 (format/protocol), SCT-15 aba3f7e (ledger model),
SCT-16 d930075 (catalog), SCT-17 7520b7c (adapter/fault contract), SCT-18 5cffda9
(local), SCT-19 80aecc9 (S3), SCT-20 0611df2 (recovery). These tickets are in Review.
Correction: the SCT-16 remote handoff accidentally contained a placeholder commit
reference before a sandbox-blocked Git write was retried; d930075 is its actual
verified commit. No code was lost. The later full integration rerun passed.

Protocol fixtures, Clippy, core units, PostgreSQL/HTTP/process integration and
real pinned MinIO conformance pass at this point. Existing M1 bad-signature test
is flaky because it mutates the final base64url character (sometimes only unused
bits); fix during final acceptance. M2 application flow, SDK and acceptance are
still pending. SCT-21 is now In Progress. Pre-existing refinement docs remain
uncommitted until final M2 documentation reconciliation.

## M2 implementation result — 2026-09-19

The approved work is 14 tasks, SCT-14–27 (17 refers to dependency edges). The user
assigned the whole set and authorized implementation. Continued through all tasks:

- SCT-21 e2c6726: metadata-first admission, root ownership, bounded irreversible
  same-root relaxed pruning, physically confirmed cleanup and atomic reservations.
- SCT-22 1f65f5f: admitted verified chunks, mirror acknowledgment, bounded sealing.
- SCT-23 f477c59: atomic independent-file publication, keep-N/history, durable
  receipts and outgoing pinned-copy staging accounting. Also fixed the M1
  signature-test fixture to mutate meaningful signature bits.
- SCT-24 a229d37: authorized renewable pins, verified whole/range downloads,
  ordered sticky fallback, progress budgets and stream failure propagation.
- SCT-25 35dd20d: standalone SDK saved jobs, disk-backed plans, metadata admission,
  second-pass source verification, resume/abort/history/download and example.
- SCT-26 898acaf: five real placement workflows and actual process crash/resume.
  Integration fixed SDK height-0 leaf encoding and rejected-session inspection
  after durable ownership was released.
- SCT-27: capabilities/generated routes, executable M2 acceptance, large-file
  measurement, final cleanup/authorization hardening and reconciled operator docs.
  Its scoped commit carries the ticket link.

Memory qualification passed at 16 MiB and 1.5 GiB (15 / 1,256 chunks, tree heights
0 / 1): combined HTTP server/SDK peak RSS 33,708 / 43,860 KiB under a 262,144 KiB
ceiling. Plan/upload/download time 1.41 / 97.73 s. See docs/m2-qualification.md for
methodology, exact scope and reproducible commands. The larger plan exercises
branch pages, missing-chunk pagination and bounded traversal.

The final suite includes standalone SDK packaging, real local/MinIO placements,
PostgreSQL/HTTP/OIDC/process recovery and M1/M2 acceptance. M3 atomic trees and
broader delete/policy work, M4 control UI/checkpoints and M5 release qualification
remain pending; the full-product acceptance command must still report three
pending scenarios. No push or deployment was performed. All completed M2 tasks
are handed off to the user in Review, not marked Done or used to close the epic.

Final validation: `./scripts/check` exited 0: Rust/Clippy/contracts/web/packaged SDK,
19 PostgreSQL cases, real MinIO conformance, and 4 passing Chromium/acceptance cases
with 3 explicit M3–M5 pending gates. Both memory workloads passed. A final bounded
S3 write-timeout adjustment passed focused real MinIO conformance and workspace
Clippy. `git diff --check` is clean. Integration output is recorded locally in
`/tmp/sct-m2-final-check.log`; durable methodology/results are in the qualification
and test guides.

# M3 implementation

Approved scope: SCT-28–41 (14 tasks, 15 dependency edges), delivered through HTTP
and the standalone Rust SDK. Existing M1/M2 behavior remains covered. Local history
was rebased onto the shared server lifecycle work during implementation; current
M3 commits are:

- SCT-28 `fc30767`: canonical trees and lifecycle contracts.
- SCT-29 `4638d72`: snapshot and durable work catalog.
- SCT-30 `6f3a98b`: durable draft edits and identity validation.
- SCT-31 `5f9ae10`: shared, bounded graph reclamation.
- SCT-32 `bc6b448`: complete-tree building and pre-payload admission.
- SCT-33 `625ff42`: atomic publication and snapshot rotation.
- SCT-34 `641c983`: historical traversal and pinned descendant reads.
- SCT-35 `a851d96`: inherited namespace hiding and retained history.
- SCT-36 `61b2d04`: bounded destructive policy previews.
- SCT-37 `9ccc6a1`: forward-only, authorized lifecycle recovery.
- SCT-38 `9bbf979`: acknowledged independent-policy moves.
- SCT-39 `ce7b5c9`: separately authorized purge and pin-safe cleanup.
- SCT-40 `8ea27d0`: standalone SDK tree plans and lifecycle calls.
- SCT-41: real-placement and million-entry qualification, active M3 acceptance,
  advertised contracts and operational documentation; its scoped commit carries
  the ticket link.

The million-entry metadata preview completed in 7,814 bounded batches, including
a writer restart after batch 25. Measured preview time was 84.808 seconds, with
14,792 KiB baseline and 14,936 KiB peak sampled worker RSS. The entire preview was
then applied. See `docs/m3-qualification.md` for fixture and measurement limits.
M4 UI/checkpoints and M5 release qualification remain pending. Task handoffs use
Review; this does not close the parent epic or imply deployment.

Final M3 validation: `./scripts/check` exited 0 with 5 passing acceptance cases
and exactly 2 pending M4/M5 gates. All five M3 placements also assert pre-payload
capacity rejection creates zero physical copies. The separate million-entry
qualification passed. `git diff --check` is clean. Logs are local artifacts at
`/tmp/sct-m3-final-check.log` and `/tmp/sct-m3-scale.log`; durable results and
reproduction instructions are in `docs/m3-qualification.md`.

# M4 implementation

Approved scope: SCT-42–61, twenty tasks. Programs retain all authorized content
mutation APIs/SDK operations; the browser provides downloads, inspection and
management only. Regular users operate within grants and administrator ceilings;
administrators manage service-wide storage, maintenance and checkpoints.

Scoped commits:

- SCT-42 `63d2195`: management/recovery contracts.
- SCT-43 `407bed2`: catalog schema.
- SCT-44 `3a8684c`: account/delegated-root APIs.
- SCT-45 `b81c2a0`: persisted administrator storage management.
- SCT-46 `b9a21f1`: usage/activity/audit inspection.
- SCT-47 `3158799`: durable maintenance scheduling and retries.
- SCT-48 `53935b8`: integrity scans/findings.
- SCT-49 `b5b70c8`: verified mirror repair.
- SCT-50 `ba50fbd`: checkpoint barriers and generation pins.
- SCT-51 `1d67616`: consistent catalog/inventory exports.
- SCT-52 `1655eb2`: independent isolated recovery.
- SCT-53 `ecd36d5`: browser sessions/navigation.
- SCT-54 `eef58bc`: retained snapshot browsing/downloads.
- SCT-55 `d28a102`: credentials/principals UI.
- SCT-56 `b8102c2`: delegated root settings/policies/grants UI.
- SCT-57 `f1d26b8`: storage administration UI.
- SCT-58 `fd9296c`: activity/maintenance/checkpoints UI.
- SCT-59 `d5a586d`: metrics, readiness, logs and commit freshness.
- SCT-60 `117734c`: complete standalone management SDK.
- SCT-61: final qualification/docs, executable M4 acceptance and storage UI
  acceptance gaps (adapter documentation/hints and placement/read-policy editing);
  its scoped commit carries the ticket link.

The independent recovery drill passed with the original payload directory removed:
file and atomic-tree bytes recovered through the SDK, old credentials rejected,
and unfinished upload expired. Focused M4 browser/recovery and storage-administration
checks passed (2 cases). Durable reproduction and limits are documented in
`docs/m4-qualification.md`; operator instructions are in
`docs/browser-management.md`, `docs/checkpoint-recovery.md` and
`docs/observability.md`. Final suite results are recorded below.

Final validation: `./scripts/check` exited 0. Rust formatting, workspace Clippy,
unit/HTTP tests, generated contracts, frontend checks/build, standalone SDK package
checks, PostgreSQL integrations (including 47 core foundation cases and isolated
recovery), real MinIO conformance and Chromium all passed. Browser/acceptance:
12 passed, exactly 1 pending (M5). `git diff --check` is clean. Local full-suite log:
`/tmp/sct-m4-final-check.log`. No push or deployment was requested.

# M5 extensive qualification campaign

User authorized extensive testing after M4. Added `scripts/qualify`, a
self-contained Docker test image, an internal PostgreSQL/MinIO/proxy network,
retained failure artifacts, actual server process crashes, eight-root concurrent
mirrored workloads, verified network fallback, streamed 128 MiB backups, SQLite
capture/recovery and real ENOSPC on a dedicated tmpfs. `scripts/qualify-references`
adds the ten-million-reference metadata/GC fixture. Normal integration explicitly
excludes these destructive/resource-intensive qualification cases.

Found and fixed a real pool/allocation deadlock: byte-counter updates acquired a
pool lock through the capacity trigger after holding an allocation lock, reversing
publication's order. Migration 19 applies the pool-capacity guard only to
allocation creation/identity/capacity changes; row constraints still enforce ledger
bounds. The deterministic lock-order regression and 64 concurrent mirrored commits
then passed. Current checkpoint schema/checksum expectations advance to 19.

The reference workload exposed per-edge SQL round trips in garbage collection.
Each bounded adjacency is now marked in one deduplicated INSERT. The 10,000-edge
preflight improved from 1,726 to 215 ms. Ten million canonical references passed
in 306 GC batches, including a writer restart, at 74,505 ms traversal and sampled
worker RSS 19,972–20,072 KiB. Its edge table/indexes occupied 4,676,501,504 bytes.
Loading took 637,810 ms; this was metadata-only and did not manufacture a claim
of ten million real payload objects.

The fault campaign also exposed and corrected the test proxy's TCP half-close
handling: an ordinary repeated S3 write failed conformance through the old tunnel.
No S3 adapter change was needed. Disk-full behavior passed both before and after
admission; the latter resumed the same saved SDK job after freeing space.

`./scripts/check` passed after the product fixes, including 48 core foundation
integration cases, independent recovery and 12 browser/acceptance passes. Only M5
remains pending. See `docs/extensive-qualification.md` for detailed scope, measured
results and limitations. The final container-campaign outcome is recorded below.

Final validation on top of the independently committed telemetry integration
`3343fbe`: `./scripts/check` passed again (12 browser/acceptance passes, one pending
M5 gate). The Docker campaign exited 0; all destructive, integration, five-placement,
million-entry, browser and ten-million-reference stages passed. Final reference
measurement: 663,192 ms load, 76,654 ms GC, 306 batches, sampled worker RSS
19,468–19,604 KiB, pinned copy preserved and orphan reclaimed. Full artifacts:
`target/qualification/sct-qualification-20260921-092210-610157/`.

The container SDK check exposed a missing ripgrep dependency that an `if` could
silently skip. Added it to the image and made `scripts/check-client` fail early
without it. Packaging plus dependency isolation passed again, offline, in the
corrected fresh image: `sct-qualification-20260921-093652-714070`. Bootstrap/source
fingerprint capture passed separately in `sct-qualification-20260921-092620-646627`.
M5 stays In Progress: the complete release baseline, broader resource/performance
matrix and release gate remain outstanding. No push or deployment performed.

# Disc archives — SCT-62

Approved SCT-63–79 and 21 dependency edges; all assigned to Codex. SCT-63
adds portable format-1 DTOs and bounded validation, canonical historical fixtures
with SHA-256 inventory, and the approved design/format specifications. Every disc
contains its set’s complete target index; there is no final-disc bootstrap dependency.
No optical drive is exposed at /dev/sr* on this host; physical-media qualification
will be recorded separately from software/ISO tests.

SCT-63 validation: canonical archive tests pass; `./scripts/check` passed with
12 browser/acceptance cases and the existing pending M5 gate. Initial sandboxed
transport test could not bind a local socket; the authorized networking/Docker run
passed. Full log: `/tmp/sct-disc-d01-check-unrestricted.log`.

SCT-69 adds the standalone `sct-archive` crate and `sct-recover inspect/verify`.
A disk-backed, bounded SQLite index validates complete graph relationships before
commit; media access refuses symlinks/non-regular files and reports missing,
unreadable/corrupt chunks. Tests bootstrap all six fixture discs, verify earlier
and duplicate media, reject malformed graphs and bound metadata reads. A clean CLI
process indexed 50,005 entries in about 3.2 seconds with sampled peak RSS 11,708 KiB.

SCT-69 validation: five adversarial/scale reader tests, Clippy and full
`./scripts/check` pass (12 browser/acceptance passes; existing M5 gate pending).
Log: `/tmp/sct-disc-d07-check.log`. Recovery crate has no core/server dependency;
SQLite is bundled so the reader does not require a database server installation.

SCT-70 adds verified chunk ingestion and complete/partial offline restoration.
Ten-disc tests bootstrap from disc 9 with disc 10 missing, identify shared and
spanning-file losses, resume across CLI processes, reject destination conflicts
and symlinks, and detect corrupt cache bytes. A killed 256 MiB restore leaves no
partial final file; a fresh process completes it and independent chunk hashes match.
Private SQLite work files have process locks; destination publication never replaces
an existing path. Crash-orphan temporary files are documented for manual cleanup.

SCT-70 validation: eight reader/recovery tests, focused Clippy and full
`./scripts/check` pass (12 browser/acceptance passes; existing M5 gate pending).
Full log: `/tmp/sct-disc-d08-check.log`.

SCT-71 packages the Linux x86_64/glibc 2.36 recovery executable, format and recovery
instructions, normalized vendored source archive, dependency license metadata and
build provenance. The independent source is compiled in the resolved Rust Docker
image with networking disabled. Existing bundle output is never replaced; the
manifest is published last and provides fixed hashes/byte counts for planning.

SCT-71 validation: the exact 41,719,669-byte bundle passed inspect, verify, partial,
resumed complete and later-set recovery in a stock Debian 12 container with no
network, read-only root, non-root user and dropped capabilities. No development
compiler or service was available in that runtime. Final bundle manifest SHA-256:
`39820936b48637b428db0986975989c215eb3e7560f736398078190439b951ba`.
Build/runtime logs: `/tmp/sct-disc-d09-final-build.log`,
`/tmp/sct-disc-d09-final-runtime.log`. Full `./scripts/check` passed (12 browser
passes, existing M5 gate pending): `/tmp/sct-disc-d09-check.log`.

SCT-64 defines account archive DTOs, branded identities, explicit job/artifact/copy
states, immutable verification observations, bounded capture selections and generated
Rust/OpenAPI/TypeScript contracts. Planned routes are not advertised as implemented.
The standalone SDK includes typed metadata operations and streaming ISO download
with exact size/hash checks; the extracted package example compiles archive calls.
The API lifecycle documents revisions/idempotency, cancellation, metadata capacity
failure, preserved completion versus current media health, and recovered numbering.

SCT-64 validation: Rust protocol/lifecycle fixtures, 55 TypeScript/schema fixtures,
real HTTP SDK corruption/truncation cases, standalone packaged-client build and
workspace Clippy pass. `./scripts/check` passed with 12 browser passes and the
existing pending M5 gate (`/tmp/sct-disc-d02-check.log`). Final SDK/schema additions
were checked again with focused Rust, Clippy, contract and extracted-client checks
(`/tmp/sct-disc-d02-protocol.log`, `/tmp/sct-disc-d02-client.log`).

SCT-65 adds migration 20 and account-owned optical catalog operations. PostgreSQL
serializes idempotent append creation and permanent number reservations, preserves
cancelled intervals across writer replacement, freezes index/content assignments,
and retains immutable ISO identities and copy/report history independently of
online roots/versions. Service checkpoint schema support advances to 20; importing
an older catalog cannot silently reuse disc numbers because recovered collections
require reconciliation. Published sets require all reserved disc artifacts.

SCT-65 validation: three new real-PostgreSQL migration/concurrency/history tests
pass, and independent service recovery now verifies retained optical high-water
marks and blocks an unsafe append. Full `./scripts/check` passed (12 browser passes;
existing pending M5 gate). Logs: `/tmp/sct-disc-d03-integration.log` and
`/tmp/sct-disc-d03-check.log`. Schema-19 service checkpoints still require their
matching release for restore before migration; portable optical format stays v1.

SCT-66 exposes scoped collection/capture/history/disc/index management, bounded
account-bound pagination, cancellation/confirmation and audit events. Every source
in a disc's original index must remain authorized, including sources absent from
later snapshots. Captured-credential worker checks terminate revoked work. The
HTTP tests exposed that existing UI restrictions were not enforced for cookie
sessions: content mutation routes now reject them early, and core Write/Delete/
Purge checks require program credentials even for administrators. Move/purge job
checks retain this boundary; policy-only browser management remains permitted.

SCT-66 validation: PostgreSQL tests cover unrelated accounts, mixed-source grant
loss, credential revocation, cursor isolation, revisions and cancellation replay.
HTTP tests cover browser CSRF, archive management, source mutation denial and admin
access; a core administrator-session test verifies upload/hide denial. Initial HTTP
fixture exposed the missing session boundary; the corrected integration run and
full `./scripts/check` pass (12 browser cases; existing M5 gate pending).
Logs: `/tmp/sct-disc-d04-integration-final.log`, `/tmp/sct-disc-d04-check.log`.
