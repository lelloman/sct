# Durable optical archive catalog

Migration 20 adds account quotas, collections, append sets/jobs, source selections,
disk-backed capture/index tables, logical discs, payload placements, immutable ISO
records, physical copies and verification observations. These are separate from
online file/version lifetime: archived sources retain opaque root/version identities
without foreign keys to online retention tables. Account identity remains explicit.

Core collection/append creation authenticates the current account and reads every
selected root's current grant. Request replay uses the existing principal-scoped,
transactionally serialized idempotency store. The collection row serializes append
creation; a partial unique index independently rejects a second active append.
Number reservation locks the collection and set, advances a permanent high-water
mark, and returns the same interval on replay. Cancelled intervals leave gaps and
are never recycled. Display numbers/set sequences are bounded by format 1.

SQL guards reject decreasing number counters, changing identities, deleting
historical collections/sets/discs/copies/reports, rewriting published ISO hashes,
and modifying a frozen source/index/placement graph. Foreign keys keep each
placement within its collection, enforce object lengths and reject absent graph
references. Placements cannot introduce a dependency on a future set. Publication
requires the full reserved disc interval and available artifacts for every disc.
Cache expiry changes artifact availability without rewriting set history.

A logical disc can have many physical copies. Verification reports are immutable;
current copy health may later become damaged or missing. No observation deletes
old dependencies or grants permission to retire a disc. The initial account cache
ceiling is 100 GiB; worker reservation/physical-capacity enforcement and management
of these limits are implemented by the artifact-lifecycle task.

Migration 21 adds stable capture pins and persistent graph-enumeration queues.
Append creation accepts explicit committed version IDs, locks their roots against
retirement, and pins their manifests in the same transaction as the source list.
An atomic directory preserves its original tree snapshot. A selection spanning
files or roots records those individual versions; it does not promise an
application-wide atomic instant. Later uploads, renames and retention cannot alter
the selected graph. The browser must resolve a current view to explicit versions
before submitting it; unavailable versions fail instead of silently advancing.

Capture processes one native metadata node per transaction, at most 256 KiB and
1,024 children. Persistent frontier/page rows and the server writer fence allow
restart without a long-lived database snapshot. Directory structure, empty files,
empty directories, root-scoped shared chunks and exact extent offsets survive.
Hashes, heights and parent totals are checked during enumeration. The transfer
worker advances at most four sets and 32 nodes per set per tick, rotating their
next-attempt times so large captures do not monopolize work.

Initial per-account limits are four active sets, 1 TiB of pinned logical source
bytes, 100 million capture records per set and a 24-hour deadline. Logical bytes
are charged conservatively per selected version, without a deduplication discount;
record limits also account for enumeration queues. Administrator core methods
`archive_capture_limits` and `configure_archive_capture_limits` provide revision-
checked configuration (maximum 64 active sets and seven-day deadline). HTTP/UI
configuration belongs to the artifact quota task. Limit failures return
`archive_resource_limit`; the active-set ceiling returns `archive_busy`.

Owned graph pins deliberately have no independent GC expiry. The worker first
fences cancellation, expiry, failure or revoked source authority, then releases
at most 64 owned pins per cleanup transaction. Account charges are released once
the last pin is gone, including for zero-byte selections. This avoids GC racing
an active batch. The fixed deadline spans capture, planning and export; it cannot
be extended by polling. Completed archive history remains intact after cleanup.
During service downtime pins remain protective; cleanup resumes with the writer.

Service checkpoint format remains schema-specific, unlike portable optical format
1. The current service checkpoint reader uses schema 23. Restore a schema-19/20/21/22 service
checkpoint with its matching release, then migrate; it is not silently reinterpreted.
Trusted catalog import temporarily permits inserting already-frozen optical rows,
then immediately reinstates guards. Imported collections require reconciliation:
an older checkpoint cannot establish that no later discs were burned. Their saved
high-water marks and history survive, but normal append creation is blocked until
the dedicated archive-recovery flow resolves the uncertainty.
