# Capacity and recovery contract

SCT-15. The pure accounting model is `sct_core::accounting`; the catalog must
persist these transitions under server, operation and generation fences.

## Obligations and locks

A root quota counts the union of reachable payload chunks once, independently of
mirror count. Logical file sizes include repeated references. A physical allocation
counts each required copy plus charged manifest/temporary bytes. Root retained
allocations and server staging allocations belong to pools. Pool capacity covers
all allocated budgets and excludes administrative metadata/safety overhead.

For each allocation: 0 <= pending <= occupied, reserved >= 0, and
reserved + occupied <= capacity. Pending is a subset, never an extra debit.
Before an I/O starts, its maximum bytes move reserved -> occupied. An ambiguous
write is occupied, even if the object might not exist. Available/acknowledged is
an integrity state, not an accounting transition. Unused reservation can be
released on abort; issued bytes cannot. Local temporary writes consume one
object's obligation; no-replace publication uses rename/link semantics on that
same volume, never a second payload copy. A provider needing extra physical
copies must include that demand before admission.

Lock order: server lease fence, root row/operation, sorted pool rows, sorted
allocation rows, object/copy rows. Pin acquisition also locks the root row before
checking live version and inserting the pin; retirement holds the same root lock.
No transaction performs backend I/O. Concurrent roots reserve all destination
pools in one transaction. A second reservation observes the first transaction's
committed obligations, never a cached free-space value. Reported physical free
space is an additional bound, not a replacement for reservations; account for
outstanding unissued reservations when interpreting a probe. Unknown remote
capacity uses the configured budget. Probe/write races with external writers
can still fail and must follow recoverable I/O transitions.

## Shared graph and retirement

Versions reference immutable file roots. Branches own ordered edges to pages;
leaf pages own ordered edges to chunks. Store these edges once per immutable
object; repeated edge positions preserve multiplicity. Live versions, upload
reuse pins and read pins are graph roots. A bounded mark/traversal can calculate
unreachable objects without storing one complete reference set per version.
Both pin-versus-retirement orders serialize: a pin acquired first protects its
graph; retirement first makes the version unavailable to new pins.

Projected retained quota is the union after publication and keep-N rotation.
Strict outgoing versions stay protected until successful publication. Admit
new unique bytes only if the complete upload and expected outgoing/pinned
obligations fit staging and every physical destination. Publication atomically
transfers incoming charge staging -> retained and outgoing charge retained ->
staging pending; it must fit BOTH allocations. The pure `publish` function
checks this vector without partial changes. Dedup/shared objects that remain
reachable never move to pending merely because one version was retired.

Relaxed admission: first reject impossible staging/maximum-object limits. Pin
planned reuse, enumerate same-root eligible non-latest relaxed versions in
commit-sequence/id order, and compute the union after successive retirements.
Skip a prefix with no useful marginal recovery unless later candidates jointly
release a shared object; never prune beyond the shortest useful eligible prefix
needed by the limiting resource. Bound work by persisted cursor and batch size;
if no eligible set can satisfy admission, reject without futile pruning.
Recheck pins/policies/revisions and demand before each destructive transaction.
Audit each retirement durably. Delete only unreferenced exact generations, and
wait for confirmed physical recovery before final admission. Latest, strict
protected and active read-pinned versions are never admission candidates. A
failed upload does not reverse already committed reclamation. Backend deletion
failure ends a bounded attempt with inspectable pending cleanup, not an infinite
request or a fictitious capacity credit.

## Durable transition table

| Phase | Transaction before I/O | I/O | Transaction after I/O / recovery |
| --- | --- | --- | --- |
| Planning | Acquire root owner; bind principal/key/request; persist bounded pages | None | Validate complete graph; freeze plan |
| Admission | Pin reusable graph; compute projected retained/staging vector | Capacity probes outside transaction | Atomically reserve all pools, or persist pending/rejected |
| Reclamation | Retire eligible prefix, audit, mark unreachable copies deleting | Delete exact generation | On confirmed absence/recovery release occupied/pending; otherwise keep charge |
| Writing | Persist fresh generation, state writing, charge occupied, worker fence | Conditional immutable write | Verify exact bytes/length; set available only under current fence |
| Sealing | Check complete graph, available required copies | Verify ambiguous copies if needed | Ready only when every required copy is verified |
| Publication | Reauthorize; compare target/policy revisions; rotate/refcount/ledger/receipt/jobs in one transaction | None | Committed receipt survives version retirement |
| Abort/expiry | Terminal session; release unused reserve; mark issued unreferenced copies deleting | Delete/reconcile | Keep root ownership until issued work and cleanup obligations resolve |
| Recovery | Claim existing operation with incremented epoch, keep all charges | Inspect exact generations; verify/retry/delete | Resume persisted phase, never create a competing root owner |

An operation epoch fences catalog changes but cannot cancel an already-issued
backend call. Every physical write uses a never-reused generation. A successor
must drain/resolve issued work before releasing its root owner. No generation
is reused following an ambiguous write or delete. Tombstones identifying issued
generations survive until no old worker/I/O can materialize them. Restart never
turns all reservations into free bytes. S3 request ambiguity may require retained
orphan reconciliation; a stat miss alone is not proof an in-flight write ended.

Read leases renew while reading; expired readers stop before emitting another
buffer. A failed renewal terminates the stream. Verified bytes buffered before
expiry do not permit continuing unboundedly after lease loss. Worker expiry and
24-hour session inactivity are different clocks. Authenticated session activity
renews inactivity; server workers renew only their operation lease. Revocation
must not wait for a long root operation; each API action revalidates credentials
and current grants. Future checkpoints must act as additional protected graph
roots and fence/drain issued I/O; checkpoint execution is M4.

Root ownership is released only when terminal publication/abort has no unused
reservations, uncertain writes, issued late calls or pending cleanup obligations.
A durable receipt may already report committed while cleanup still owns the root.
Other roots and authorized reads can continue. Do not equate a released SQL lock
or worker lease with completion of the durable root operation.
