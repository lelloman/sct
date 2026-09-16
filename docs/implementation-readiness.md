# Implementation readiness — 2026-09-16

SCT-6 can proceed without another product decision. Its scope is development wiring,
component boundaries and an executable test harness. The full product is not yet
implementation-complete or acceptance-ready.

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

M2 requires real staging reservations, physical capacity checks and basic publication
accounting. These cannot all wait for M3 despite the short milestone summary. M3
extends them with full retention, atomic trees and reclamation. The shared-staging
amendments in TICKET_PLAN.md already establish this dependency.

Remote ticket descriptions may lag local amendments. The SCT-6 implementation includes
the early acceptance harness required by the local delivery decision. It does not
implement the subsequent protocol, registry, ownership or allocation models.
