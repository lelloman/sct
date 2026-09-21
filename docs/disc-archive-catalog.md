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

Service checkpoint format remains schema-specific, unlike portable optical format
1. The current service checkpoint reader uses schema 20. Restore a schema-19 service
checkpoint with its matching release, then migrate; it is not silently reinterpreted.
Trusted catalog import temporarily permits inserting already-frozen optical rows,
then immediately reinstates guards. Imported collections require reconciliation:
an older checkpoint cannot establish that no later discs were burned. Their saved
high-water marks and history survive, but normal append creation is blocked until
the dedicated archive-recovery flow resolves the uncertainty.
