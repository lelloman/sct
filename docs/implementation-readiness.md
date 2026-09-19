# Implementation readiness — 2026-09-16

Update, 2026-09-19: M1 and M2 (SCT-6–27) are implemented with executable acceptance.
M2 freezes immutable format, physical ledger/recovery, metadata-first admission,
keep-N publication and pinned reads. See [qualification](m2-qualification.md).
The remaining prerequisites apply to dependent M3/M4 features. The full product is not yet implementation-complete or acceptance-ready.

The following are design prerequisites for later features, not reasons to delay the
scaffold. Resolve them in the architecture and relevant task before implementing
the dependent behavior.

| Topic | Required decision or design | Deadline |
| --- | --- | --- |
| Capacity ledger | Atomic staging/retained/pending-deletion transitions; shared graphs; read/checkpoint pins; two physical copies sharing one pool; crash/retry invariants | M1 schema extension points, M2 admission/publication |
| Atomic descendant deletion | Reconcile directory snapshot retention with the generic latest-hidden-content protection; define permitted purge targets | Before final deletion contracts and M3 |
| Operation recovery | Durable phase/state transition table, transaction boundaries, fencing checks, uncertain I/O reconciliation and exact ownership-release conditions | M2 |
| Immutable format | Canonical JSON rules, bounds, page structure, digest inputs and exact chunk-profile fixtures | Reserve format versions in SCT-7; finish before M2 manifests |
| Checkpoints | Fencing resumable uploads, draining already-issued I/O, generation watermark and restore semantics | Compatible M1/M2 primitives; complete before M4 |

Proposed, **not yet adopted**, deletion interpretation: removing an atomic descendant
removes it from the new snapshot; older directory snapshots preserve it only for their
retention lifetime. Reject individual descendant purge while snapshots reference it.
This avoids silently adding independent file retention to an atomic directory.

M2 requires real staging reservations, physical capacity checks and publication
accounting. The agreed 2026-09-19 scope includes independent-file history, keep-N
rotation and admission-time relaxed reclamation before payload ingestion. M3
extends these with atomic trees, delete/purge and broader policy transitions.
The shared-staging amendments in TICKET_PLAN.md establish the accounting dependency.
The [approved M2 breakdown](m2-implementation-plan.md) assigns immutable encoding
and ledger/recovery design gates to A/B before dependent implementation. Its 14
children (SCT-14–27) and 17 dependency edges were created and verified in Crumbles.

The M2 capacity/recovery and immutable-format design gates are closed by
`capacity-recovery.md`, `immutable-format.md`, executable ledger tests, real adapter
qualification and the M2 acceptance scenario. Checkpoint and atomic deletion
semantics remain explicit prerequisites for M3/M4; full product acceptance still
fails while M3–M5 remain pending.
