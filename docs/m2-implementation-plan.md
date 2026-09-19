# M2 implementation plan — 2026-09-19

Parent: [LLPR/SCT-2](https://crumbles.lelloman.com/w/LLPR/SCT/2).
Status: approved and recorded in Crumbles on 2026-09-19. All 14 Tasks and 17
`blocks` edges were created and read back successfully. Parent SCT-2 remains Open.
Plan letters map to the ticket links below. Implementation completed locally;
SCT-14–27 are handed off for review with scoped commits. Qualification and remaining
product gates are recorded in [M2 qualification](m2-qualification.md).

## Agreed product behavior

M2 delivers immutable independent-file upload, replacement, retained history and
verified downloads through the HTTP API and standalone Rust SDK, with local, S3
and mirrored placement. It includes keep-N rotation and admission-time relaxed
reclamation. M3 extends this with atomic directory snapshots, delete/purge and
broader retention/policy transitions. The web control surface remains M4.

Admission happens before file payload upload:

1. The client submits bounded metadata pages describing stable source bytes:
   file size, chunk hashes/lengths and canonical manifest structure.
2. The server validates authority, revisions and the plan, pins reusable content,
   and calculates projected root quota, staging and physical destination demand.
3. If necessary, it prunes eligible older relaxed versions in the same root,
   oldest first. Latest versions, strict-protected versions and active read pins
   remain protected. Reclamation is bounded and durable; failed uploads do not
   restore history already pruned.
4. After confirmed capacity recovery, it atomically reserves every required
   destination pool and authorizes missing payload chunks. Otherwise it returns
   a capacity error without accepting file payloads.

Pruning cannot enlarge the separately allocated staging allowance. Reject
irreducible limits before deleting history. Shared references can prevent a
retired version from freeing any physical bytes. Both mirror copies count when
they share a pool. Concurrent roots must not spend the same free capacity.
Admission cannot guarantee against later external disk consumption or backend
failure; those failures preserve the previous published version and follow the
same resumable/cleanup rules.

“Before bytes” means before application payload ingestion/storage. Metadata must
arrive first; clients wait for admission before sending chunk bodies. The server
guards chunk routes before consuming bodies. Network/proxy buffering is not a
zero-byte-on-the-wire guarantee.

## Approved execution contract

All children below are **Task** (live type ID 3), parent SCT-2, initially Open,
priority Medium, labels `architecture-plan`, `milestone-2`. Leave assignment and
estimates unset. Every repository implementation task produces one scoped commit
with its eventual ticket link, as required by the project's instructions.

The implementation baseline is completed local M1 code; this proposal does not
assert that remote M1 tickets are Done. A and B close explicit design gates with
normative documents and executable fixtures before dependent code begins.
Each implementation task includes its focused tests and protocol documentation;
N integrates and measures the milestone rather than deferring testing until then.

Approved API behavior: admission/cleanup can return a durable pending operation
that the client polls. Payload permission is explicit and only granted after
admission succeeds. A fixes exact wire states, bounds and error codes. Session
inactivity defaults to 24 hours; worker lease expiry alone does not expire a
session. File plans are immutable once admitted; changed source requires a new
plan. These details were adopted with approval of the complete breakdown.

## Complete child descriptions and acceptance criteria

### A — Freeze immutable file encoding and transfer protocol

Ticket: [LLPR/SCT-14](https://crumbles.lelloman.com/w/LLPR/SCT/14).

Define the versioned canonical JSON file/page encoding, SHA-256 digest inputs,
FastCDC v2020 profile (256 KiB / 1 MiB / 4 MiB), metadata bounds and independent-file
HTTP session/admission/history/read contracts. Generate shared wire types and
fixtures. Reserve directory format extension points without implementing snapshots.

Acceptance: deterministic chunk boundaries and byte-for-byte manifest vectors;
empty files and repeated chunks; rejection of noncanonical/oversized/unsafe integer
representations; paged plans and missing-chunk results; explicit pending/admitted
states, capacity failures, revision conflicts and idempotent retry semantics.
Document that manifest identity differs from reconstructed-file SHA-256.

Dependencies: none. Suggested order: 1.

### B — Specify capacity accounting and recoverable I/O transitions

Ticket: [LLPR/SCT-15](https://crumbles.lelloman.com/w/LLPR/SCT/15).

Define the normative ledger and transition table from plan through admission,
write, publication, retirement and confirmed deletion. Cover retained graph edges,
reuse/read pins, reservations, staged bytes, outgoing bytes, uncertain I/O and
temporary-file overhead. Specify server/operation fencing, never-reused physical
generations and exact root-ownership release conditions.

Acceptance: executable model cases conserve charges across rollback/retry/crash;
dedup counts once per root but each physical mirror copy per pool; multi-pool
reservation is all-or-nothing; strict rotation and pinned outgoing copies cannot
invent capacity; unknown/failed deletion never releases charges. Define bounded
oldest-first relaxed candidate selection, useful-recovery estimation and stopping
conditions, including shared-reference cases. No backend I/O inside SQL transactions.

Dependencies: none. Suggested order: 2.

### C — Persist content graphs, physical copies and capacity obligations

Ticket: [LLPR/SCT-16](https://crumbles.lelloman.com/w/LLPR/SCT/16).

Extend PostgreSQL migrations and core transactions with immutable manifests/pages,
versions, incoming graph references, sessions, copy incarnations, pins and the B
ledger. Replace the M1 unused-reservation primitive where transfer needs richer
accounting. Provide transactional reservation, pinning and reference operations.

Acceptance: migration/restart tests; constrained cross-root isolation; bounded
graph traversal; atomic concurrent multi-pool reservations; same-pool mirrors;
both pin-versus-retirement race orders; no flat per-snapshot duplication of the
entire graph. Stored states and charges enforce B's invariants.

Dependencies: A, B. Suggested order: 3.

### D — Implement the asynchronous payload adapter contract and fault harness

Ticket: [LLPR/SCT-17](https://crumbles.lelloman.com/w/LLPR/SCT/17).

Extend registration with immutable generation-specific writes, stat, streamed
read, idempotent delete and capacity inspection. Define acknowledgment guarantees,
verification, cancellation and ambiguous-result handling behind provider-neutral
interfaces. Add deterministic fault adapters and a reusable conformance suite.

Acceptance: delayed/partial/corrupt/lost-response operations are injectable;
metadata stat and S3-style ETags never substitute for byte integrity verification;
late completion cannot overwrite a newer generation; bounded buffering and
same-kind mirrors require no provider branching in core operations.

Dependencies: A, B. Suggested order: 4.

### E — Implement the local filesystem payload adapter

Ticket: [LLPR/SCT-18](https://crumbles.lelloman.com/w/LLPR/SCT/18).

Implement sharded opaque keys, exclusive temporary writes, verification, file
sync, atomic no-replace publication and directory sync, plus streamed reads,
deletion, capacity/headroom inspection and recoverable temporary artifacts.

Acceptance: D's conformance suite runs on real filesystem storage; interrupted
writes, collisions, disk exhaustion and restart leave no false acknowledgment;
temporary bytes are accounted; configured directories sharing a volume use their
shared pool; document filesystem assumptions and acknowledgment guarantees.

Dependencies: D. Suggested order: 5.

### F — Implement the S3 payload adapter

Ticket: [LLPR/SCT-19](https://crumbles.lelloman.com/w/LLPR/SCT/19).

Implement immutable conditional writes, streamed verification/read, idempotent
deletion and administrator-budgeted capacity using private registered settings.
Document supported unversioned bucket configuration and reject unsupported
versioning/Object Lock configurations rather than misaccounting retained copies.

Acceptance: D's suite runs against a real S3-compatible service; conditional
conflicts, ambiguous writes, authentication failures and interrupted transfers
are exercised; ETags are not treated as SHA-256; unknown free space respects
budget. Do not claim AWS qualification from an emulator-only test.

Dependencies: D. Suggested order: 6.

### G — Implement fenced recovery and physical cleanup workers

Ticket: [LLPR/SCT-20](https://crumbles.lelloman.com/w/LLPR/SCT/20).

Implement durable copy write/delete reconciliation, aborted/expired-session
cleanup, worker takeover and generation-specific orphan handling using C and D.
Retain operation ownership and capacity charges until outstanding I/O is resolved.

Acceptance: crash injection around every catalog/I/O boundary; obsolete workers
cannot publish or delete newer copies; ambiguous writes are verified or cleaned;
deletes are idempotent; retries remain bounded and resumable. Worker loss alone
does not expire sessions, and unresolved cleanup remains inspectable and charged.

Dependencies: C, D. Suggested order: 7.

### H — Implement metadata-first sessions and admission-time reclamation

Ticket: [LLPR/SCT-21](https://crumbles.lelloman.com/w/LLPR/SCT/21).

Expose authenticated session creation, paged plan submission, admission/status,
resume and abort. Hold root mutation ownership; pin reusable content before
reclamation. Evaluate all limits, retire eligible relaxed history as necessary,
drive G cleanup, and grant payload permission only after atomic reservations.

Acceptance: full disk with reclaimable history succeeds after confirmed cleanup;
no eligible history yields a capacity error before body consumption; insufficient
staging fails without futile pruning; latest/strict/read-pinned content survives;
cross-root content is never reclaimed. Test shared references, pool races, one
full mirror destination, deletion failure, restart during admission and abort.
Persist an audit of irreversible pruning even if the upload later fails.

Dependencies: G. Suggested order: 8.

### I — Implement verified resumable payload ingestion and mirror writes

Ticket: [LLPR/SCT-22](https://crumbles.lelloman.com/w/LLPR/SCT/22).

Expose admitted-session chunk transfer and sealing. Stream-check exact lengths
and hashes, persist per-copy progress and verify every required destination before
marking content usable. Reuse completed copies on retry and enforce plan bounds.

Acceptance: unadmitted, unauthorized, oversized and unplanned bodies are rejected;
corruption/truncation never becomes available content; lost responses, disconnects
and one failed mirror remain resumable without publication; retries do not double
charge; no cross-session/root reuse leaks. Sealing verifies the complete manifest
graph using bounded memory and refuses missing required copies.

Dependencies: H. Suggested order: 9.

### J — Publish independent-file versions with keep-N and durable receipts

Ticket: [LLPR/SCT-23](https://crumbles.lelloman.com/w/LLPR/SCT/23).

Expose commit and paginated file history. Atomically publish verified content,
update namespace/revisions/references, rotate keep-N history, transfer ledger
charges, enqueue cleanup and persist the authenticated idempotency receipt.
Revalidate authority, target revision and effective policy revision at commit.

Acceptance: first file creation and replacement; strict and relaxed keep-N;
projected retained quota with transient capacity; outgoing pinned copies remain
charged; failed/stale/revoked commit leaves previous latest intact. Retrying a lost
success response returns the original result, even after the version is pruned;
changed requests conflict and expired receipt keys cannot silently be reused.
Grant/token revocation remains effective while a root operation is active.

Dependencies: I. Suggested order: 10.

### K — Implement pinned verified whole-file and range downloads

Ticket: [LLPR/SCT-24](https://crumbles.lelloman.com/w/LLPR/SCT/24).

Expose authorized version reads backed by renewable pins and bounded chunk
verification. Honor configured mirror order, connection/initial-response and
progress limits, cancellation and sticky fallback for the rest of a read.

Acceptance: verify entire intersecting chunks before releasing range bytes;
corrupt/truncated primary falls back safely; slow healthy mirrors succeed; a
failed candidate is not retried on every chunk; exhausted candidates abort the
response. Concurrent retirement/deletion cannot invalidate a live pin, expired
readers stop, and authorization does not expose another root's versions.

Dependencies: J. Suggested order: 11.

### L — Deliver standalone Rust SDK upload, resume, history and download

Ticket: [LLPR/SCT-25](https://crumbles.lelloman.com/w/LLPR/SCT/25).

Implement stable-source planning/chunking, paged metadata submission, admission
polling, missing-chunk transfer, resume/abort, commit retries and verified download
length handling. Provide a small runnable example and explicit source-capture
requirements; caller-owned stable files support two-pass bounded-memory upload.

Acceptance: SDK alone performs first upload/replacement/history/range download;
disconnect/restart resumes with durable session identity; no payload is sent before
admission; changed source is detected and cannot publish the old plan; late stream
failure is surfaced, never reported as complete. Packaging builds independently
of server internals. Document that live SQLite capture is the consumer's job.

Dependencies: K. Suggested order: 12.

### M — Exercise transfer and recovery against all real placements

Ticket: [LLPR/SCT-26](https://crumbles.lelloman.com/w/LLPR/SCT/26).

Wire disposable PostgreSQL, filesystem and S3-compatible storage into a repeatable
integration harness for single-local, single-S3, local/S3 mirrored and same-kind
mirrored roots. Run the complete API/SDK workflow and recovery fault matrix.

Acceptance: admitted prune-then-upload and no-space rejection; replacement/read
races; same-pool accounting; failed mirror; process crash; response loss; session
expiry; corrupt/truncated download. Required tests fail when a configured backend
is unavailable rather than silently passing via mocks. Document how to run them.

Dependencies: E, F, L. Suggested order: 13.

### N — Close the M2 acceptance gate and document operational limits

Ticket: [LLPR/SCT-27](https://crumbles.lelloman.com/w/LLPR/SCT/27).

Measure bounded memory with large generated files and paged plans; activate the
M2 acceptance scenario using real HTTP and SDK behavior; update capabilities,
foundation limits, architecture/readiness documents and operator instructions.

Acceptance: record workload sizes and memory ceilings/results; M1 and M2 checks
pass together while unfinished M3–M5 gates stay explicit. Document capacity errors,
irreversible admission pruning, resume/cleanup inspection and adapter guarantees.
No upload/browser control-surface claim beyond the scope actually delivered.

Dependencies: M. Suggested order: 14.

## Approved dependency graph

Each row lists incoming `blocks` edges. For example, A blocks C and B blocks C.
These are the complete 17 child-to-child edges; do not add transitive
duplicates or change existing epic relations. All 14 tasks are direct SCT-2 children.

| Child | Blocked by |
| --- | --- |
| A | — |
| B | — |
| C | A, B |
| D | A, B |
| E | D |
| F | D |
| G | C, D |
| H | G |
| I | H |
| J | I |
| K | J |
| L | K |
| M | E, F, L |
| N | M |

Suggested serial order is A through N. After A/B, catalog and adapter work can
proceed independently; E/F can proceed alongside core work using fault adapters.
No implementation begins under this proposal merely because refinement is approved.

## Remaining gates

A must fix exact encodings, limits and wire details; B must fix the mathematical
ledger and crash transition table. Those deliverables are prerequisites, not
claims that the designs are already finished. M3 atomic-descendant deletion and
M4 checkpoint fencing remain explicitly outside this refinement.

Created and verified exactly these 14 Tasks and 17 dependency edges.
SCT-2's workflow state remains unchanged. Remote epic text still needs the agreed
M2/M3 scope amendment; this proposal does not authorize an unspecified mutation.
