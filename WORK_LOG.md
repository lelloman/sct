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
