# Incremental optical planning

Migration 22 persists planner cursors, provisional packing and canonical index
records. `advance_archive_plan` processes at most 128 rows per ordinary step and
commits its cursor with those rows. It consumes an already captured graph and a
fixed canonical recovery-bundle manifest. Changing the bundle on resume fails.
The export worker must verify the installed bundle's actual bytes before planning
and building. Worker/configuration wiring and actual ISO writer checks are part
of the ISO task, SCT-72.

Entries and extents are serialized in UTF-8 byte order. Objects receive a stable
first-use ordinal while walking file paths and offsets: shared chunks are stored
once per root and a file's new chunks stay adjacent where capacity permits. A
file larger than a disc spans discs at chunk boundaries. Empty files/directories
remain in the index; an empty or fully reused snapshot still produces one new
recovery/index disc.

Reuse requires the same collection, root, digest and length, an earlier verified
completed set, and a currently read-back-verified physical copy of the original
payload disc. Original-disc source permissions are checked in full, including
roots absent from the new selection. Unverified exports and unreadable copies
cause new payload allocation. Preview completion and confirmation recheck reused
media and source access; changed media returns `archive_media_unavailable` rather
than pretending the old bytes remain accessible. Publication must repeat this
check. No old index or physical observation is changed by planning.

The budget includes every repeated bundle file and manifest, complete index/map,
sector padding, directory entries and a 2 MiB fixed writer reserve. Each object
reserves its sector-rounded length plus 8 KiB for filesystem structures. Bundle
paths reserve their directory components separately. Snapshot-header and new-disc
records are measured using maximum-width numbers; payload placement identities
have a fixed-width numeric suffix. Increasing disc count therefore increases
repeated metadata explicitly. Packing repeats with a monotonic count bound until
stable (maximum 32 passes); oversized metadata, an indivisible chunk that cannot
fit, or nonconvergence fails with `archive_metadata_too_large`. Budgets are
conservative and do not promise the nominal capacity is all usable payload.
The ISO worker must additionally reject any actual image exceeding its reserved
budget or selected media capacity before publishing it.

Numbers are reserved only after packing converges. Cancellation after reservation
leaves permanent gaps. Every new disc includes the same complete index, including
all required earlier disc identities. SHA-256 is computed by paging the persisted
canonical lines outside any long-lived row-locked transaction. A crash may repeat
that hash scan, but does not restart graph enumeration or packing; memory remains
bounded. Concurrent hash completion checks the persistent cursor before committing
so a stale worker cannot rewind progress. Per-disc payload inventory hashes use
canonical `ArchiveObject` JSON followed by LF, ordered by root ID and digest.

Confirmation freezes index records and planner state through SQL guards, together
with the existing source, extent and placement guards. Service checkpoints are
schema-specific; this release reads schema 23. Optical format 1 remains unchanged.

Tests include independent recovery-reader acceptance from every planned disc,
Unicode order, files spanning discs, shared chunks, rename reuse, unrelated-root
isolation, mixed-source authorization, exact budget boundaries, oversized repeated
metadata, empty snapshots, restart and permanent cancelled number reservations.
