# Implementation readiness — 2026-09-16

Update, 2026-09-19: M1–M4 (SCT-6–61) are implemented with executable acceptance.
M2 freezes immutable format, physical ledger/recovery, metadata-first admission,
keep-N publication and pinned reads. See [qualification](m2-qualification.md).
M3 adds atomic snapshots, explicit purge and resumable preview/apply transitions; see
[M3 qualification](m3-qualification.md). M4 adds browser management, maintenance and isolated recovery; see [M4 qualification](m4-qualification.md). Remaining release qualification applies to M5. The full product is not yet implementation-complete or acceptance-ready.

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

Approved M3 deletion interpretation: removing an atomic descendant
removes it from the new snapshot; older directory snapshots preserve it only for their
retention lifetime. Reject individual descendant purge while snapshots reference it.
This avoids silently adding independent file retention to an atomic directory.
The approved purge and preview/apply rules are frozen in [tree lifecycle](tree-lifecycle.md).
M3 tasks SCT-28–41 and their 15 dependency edges are implemented through the API and
standalone SDK. M4 implements browser management and downloads; backup content mutations remain API/SDK operations.

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
qualification and the M2 acceptance scenario. Atomic deletion and forward-only lifecycle recovery are implemented and tested.
Checkpoint export, independent copying/verification and isolated recovery are implemented
and qualified. Full product acceptance still fails while M5 remains pending.
