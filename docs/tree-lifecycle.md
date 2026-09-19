# Atomic trees and lifecycle contract v1 (SCT-28)

This freezes the approved M3 behavior. M2 file/chunk objects retain their exact
encoding. M3 routes are advertised only as their implementation lands. The HTTP
and standalone SDK are in scope; browser controls and checkpoint export/restore
remain M4. Populated directories never change versioning mode in v1.

## Immutable directory representation

A DirectoryManifest has format 1 and kind `directory`, immediate child count,
recursive logical byte/chunk totals, a page height and optional page digest.
Empty directories have no page and all summaries/height zero. An empty directory
is a real node, not an omitted path. Canonical JSON, SHA-256, 256 KiB object limits,
1024-reference fanout and maximum page height 8 use the M2 canonical rules.

DirectoryPage has a disjoint kind (`directory_entries` or `directory_branch`).
Leaf entries are strictly ordered by UTF-8 name bytes and contain stable entry ID,
name, file/directory kind, child manifest digest and logical byte/chunk totals.
A file child references an unchanged M2 FileManifest. A directory child references
another DirectoryManifest. Branches contain ordered, nonoverlapping first/last
name ranges, immediate entry counts, recursive totals and child page digests.
Each branch height is one greater than its children's height. Summaries and ranges
are verified against actual children, never trusted as authority for accounting.

A directory's stable entry identity belongs to its parent's entry record, not its
reusable content manifest. Two versions can share file manifests, directory nodes
and chunks. Duplicate entry IDs anywhere in a resulting snapshot are invalid,
even if names differ. Validate cycles, root/identity continuity and the existing
4096-character path bound. Atomic scopes cannot nest or span roots. A commit
switches one directory version pointer; immutable namespace indexes reference
shared pages, without copying a million descendant rows into each version.

## Draft protocol and ownership

CreateTreeDraftRequest binds root/directory, exact base version and entry/policy
revisions, draft mode and principal-scoped idempotency key. The draft owns the
root through build/admission/payload/publication or abort/expiry cleanup. Child
file metadata/chunks use this owner, never nested independent upload sessions.
Inactivity defaults to the existing configured 24 hours. Worker lease loss means
recovery of the same operation, not expiry or release of its root.

Modes are explicit: `complete_replacement` begins an empty resulting tree; omitted
base entries are removals. `patch` begins from the base and removes only explicit
Remove mutations. Both produce complete independently readable immutable trees,
not delta chains. Base inspection/materialization and building are bounded,
resumable work; shared unchanged nodes remain reusable.

AllocateDraftIdentitiesRequest mints up to 1000 server-owned draft identities with
an idempotency key. Upsert names an Existing identity from the base or a New
identity allocated by that draft, its parent ID, name, kind and (for files) a
FileManifest. Existing IDs express explicit renames; path/digest equality never
creates continuity. A recreated path uses a fresh ID. Removes address identities,
not a digest. A directory removal removes its descendants from the resulting tree.
Mutation pages are bounded to 1000 operations and 256 KiB, replay identically by
key, and reject changed reuse of a key. Final validation rejects missing parents,
cycles, duplicate names, dangling references and kind changes of existing entries.

Initialization -> editing -> building -> planning -> staging -> ready -> committed
are durable draft phases. Build freezes mutation pages; source changes require a
new draft. Metadata pages may be supplied before payload admission. Admit validates
the full graph, pins reuse and atomically reserves every destination pool for the
whole draft. Existing M2 strict/relaxed/staging rules apply; snapshot pruning is
irreversible after an aborted upload. Seal requires all physical copies, including
tree metadata. Commit rechecks base/policy/auth and atomically switches the snapshot,
rotates keep-N, transfers accounting and records a permanent replayable receipt.

Write authorizes creation/replacement; removal also requires Delete, including
omissions in complete replacement. Recheck removal permission at admission and
commit. Child policies/history are never independently configurable. Whole/root
namespace reads use a pinned version and bounded cursors, so publication cannot
mix two snapshots in one traversal. Whole/range child file reads verify M2 chunks
and hold the containing directory version pin.

## Hiding and purge

Ordinary Delete hides an independent entry/subtree or whole atomic directory.
Ancestor visibility makes hiding atomic without descendant-wide synchronous
updates. Explicit deleted-entry browsing and retained downloads remain available;
latest hidden independent-file/directory content remains protected and charged.
A namespace tombstone does not consume a content-version slot. In an atomic tree,
removing a descendant instead publishes a new tree; older snapshots retain that
entry only for the lifetime of those snapshots, with no separate permanent pin.

Purge requires Purge capability, an exact revision and an idempotent explicit target:
HistoricalVersion (never a current version) or HiddenEntry (all retained histories
in that hidden subtree). No live/current shortcut and no individual atomic child
purge. At the logical retirement boundary reject new pins; already acquired live
pins may finish and remain charged. Shared objects survive while any reference
or pin protects them. Actual cleanup is bounded, generation-fenced and restartable;
unknown backend outcomes never release occupied bytes. Audit intent, logical
retirement, cleanup progress and completion. Root mount/allocation destruction and
undelete/consumer restoration are not added by these endpoints.

## Preview and apply

PolicyChange supports Set, Inherit (remove an independent override) and Move
(with explicit source and destination revisions/name). Preserve administrator
bounds and atomic policy boundaries. A root policy cannot inherit. A move must be
within independent namespace scopes in one root; crossing into/out of an atomic
scope remains invalid. Moving through a different inherited policy requires this
preview/apply flow even if the values happen to be equal.

Previews run in bounded batches over a captured root mutation generation. Readers
need not block writers: if relevant scope/content/policy/constraint mutations
invalidate that generation, the preview/apply is rejected and must be recreated.
Store the evaluated policy, exact affected versions, expiring preview identity,
requester and relevant revisions. Recheck grants and administrator constraints at
apply. One-hour preview lifetime matches existing listing-token operational bounds.
Expose a paginated effect list: retirement and loss of strict protection are
separate flags. Byte totals are estimates, not promises of physically free space.

Apply requires explicit acknowledgment, a fresh preview and an idempotency key.
Acquire durable root ownership and freeze the work set. The approved new policy
becomes effective at apply; stricter settings protect currently available history
immediately. Bounded retirement follows; reads remain available and other root
mutations return root_busy. Never fabricate pruned versions. Job states distinguish
planning, ready, applying, cleaning_up, blocked and completed. Report processed
counts, failure code and phase. There is no destructive-job cancel/rollback API.

Revalidate actor authority before each destructive batch. Revocation blocks further
logical effects; an appropriately authorized explicit resume can continue forward.
Already-issued physical I/O and safety cleanup must still be reconciled. Blocked
jobs retain root ownership/charges, distinguish backend failure from authorization,
and never automatically broaden authority. Recovery renews/claims the same fence.
Cross-policy moves validate both namespace endpoints again at apply and publish the
move atomically before bounded policy reconciliation, without partial subtree moves.

## Qualification boundaries

Reclamation uses one reusable root/object mark table and a persisted scan cursor.
Each pass seeds at most 64 roots from each reference class, expands at most 64
objects with bounded adjacency, and examines at most 64 physical copies. Catalog
changes invalidate the scan generation; writer epoch changes preserve its progress.
The worker rechecks the operation fence and persists deletion intent under the same
root lock as the reachability proof. Only confirmed backend deletion releases
occupied/pending bytes. File and tree versions, readers, active transfers/drafts
and internal graph pins participate; `checkpoint` graph pins reserve the M4
extension point without exposing checkpoint creation. A completed sweep starts
fresh on the next pass so expired readers do not protect content indefinitely.

Draft metadata API (all under `/api/v1/roots/{root}/tree-drafts`): `POST /`
creates or replays a draft; `GET /{draft}` inspects it; `POST /{draft}/advance`
imports one base page or validates 64 entries; `POST /{draft}/identities` mints
an idempotent batch of IDs; `POST /{draft}/mutations` accepts a bounded mutation
page; `PUT /{draft}/pages/{digest}` stages canonical file chunk-index pages.
`POST /{draft}/freeze` stops editing and starts validation. Invalid frozen trees
can be aborted; they cannot publish. `POST /{draft}/abort` releases a metadata-only
owner, or defers payload cleanup to the transfer worker when one exists.
Draft status is durable across writer epochs; inactivity expiry releases abandoned
metadata-only owners through the recovery worker. `POST /{draft}/admit` advances
the frozen tree builder (128 entries per leaf, 64 references per branch), then
uses the existing root operation to admit the complete content graph. Poll until
admitted before using the ordinary upload chunk/seal routes with the same ID.
Whole-tree unique payload quota and every destination's metadata/payload staging
obligation are checked before bytes; deduplication spans all files. Terminal draft
working rows are removed in bounded recovery batches, preserving the small draft
receipt/status record. Publication follows under SCT-33.

Migration 5 extends the existing version/pin identities with a content kind; old
file rows default to `file`. Directory versions point at immutable manifests and
do not create descendant rows in `entries`. Draft working rows, traversal frontiers,
preview effects and lifecycle job cursors are root-scoped with foreign keys.
Namespace/version mutations advance a separate generation for preview invalidation;
this does not change M1 revision tokens returned during root creation. Snapshot
lookup checks authorization, retained-version membership, canonical hashes and
bounded page height at every path component. Runtime workers and HTTP lifecycle
routes are supplied by the subsequent implementation tickets.

Each implementation task carries focused tests and its ticket-linked commit.
M3 qualification includes real local/S3/mirrored operations, late generation deletion,
both pin/retirement orderings, crash/replay, protected hidden history, policy-inherit
boundaries, stale previews, revocation, and million-entry bounded/resumable work.
Generalized graph roots reserve a checkpoint pin kind; M4 supplies actual checkpoint
creation/export/restore. M1/M2 checks remain executable and M4/M5 gates stay pending.
