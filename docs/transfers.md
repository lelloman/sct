# Operating independent-file transfers (M2)

The configured service supports immutable file creation/replacement, retained
history, verified whole/range reads and resumable Rust SDK uploads. Atomic tree
publication, ordinary delete/purge, populated policy changes, checkpoints and the
web administration/upload surface remain later milestones.

## Configure real storage

Start with `config.example.toml` for metadata-only development, or
`config.local.example.toml` for payload storage. Create the configured private local
directory before startup. Pool capacities include separately allocated staging;
subtract adequate overhead for PostgreSQL/catalog metadata, directories, immutable
zero-byte deletion tombstones, filesystem allocation units and provider overhead.
Payload accounting tracks exact object bytes, not a filesystem's allocated blocks.
Backends on the same physical volume must share a capacity pool. Both copies of a
mirror count even when they share that pool.

Use [adapter settings and guarantees](storage-adapters.md) for S3 and local storage.
S3 credentials are environment references, never returned in API inspection.
Never-versioned buckets without Object Lock are required; no outside writer or
lifecycle rule may mutate managed keys. Local durability relies on atomic links,
renames and fsync. The adapter's verified acknowledgment is not independent
certification of hardware/provider durability.

## Admission, publication and cleanup

Submit the immutable file manifest and bounded canonical metadata pages first.
Call `/api/v1/roots/{root}/uploads/{upload}/admit` until admission is `admitted`.
`202` means pending work, not payload permission. Only then send missing chunks.
The server checks chunk permission before reading its body. It cannot prevent a
proxy or network stack from buffering bytes sent prematurely.

Admission compares root unique-chunk quota, root retained physical allocations,
shared server staging and observed destination free space. Reservations across
all destination pools are atomic. Capacity failures distinguish:

- `quota_exhausted`: the projected retained unique chunks exceed the root quota.
- `staging_exhausted`: the separately allocated staging space cannot hold incoming
  copies/outgoing rotation debt; deleting retained history cannot enlarge it.
- `destination_capacity`: catalog physical capacity or observed local space is
  insufficient. `destination_unavailable` denotes provider/access/timeout failure.

If needed, admission can retire eligible older relaxed versions in the same root,
oldest first. It protects latest versions, strict history and live read pins.
Shared chunks may prevent retirement from freeing bytes. Capacity is released
only after exact-generation physical cleanup is confirmed. Irreducible limits
are rejected before retirement. **Pruning is irreversible even if the upload is
later aborted or fails.** It never prunes unrelated roots.

Publication requires all immutable objects to have verified required copies and
rechecks authority, target and policy revisions. It atomically creates the new
version, moves physical charges and applies keep-N rotation. Read-pinned outgoing
copies can occupy staging until readers finish. The previous latest version stays
published on pre-commit failure. Commit receipts survive pruning and support exact
idempotent retries without resurrecting old versions.

## Resume and diagnosis

Use the [SDK saved-job example](rust-sdk.md). A key is permanently bound to its
principal, root and original request. Replaying the exact creation request recovers
its session; replaying commit recovers its receipt. Changing source bytes requires
new metadata and a new key. Browser upload controls are not provided yet.

`GET .../uploads/{upload}` returns session/admission state and copy progress.
`GET /api/v1/roots/{root}/operation` exposes the durable owner, phase and recovery
information; allocation inspection shows reserved, occupied and pending cleanup.
`GET .../entries/{entry}/versions` lists retained versions. Download full bytes or
one HTTP byte range from `.../versions/{version}/content`. Late storage failure
aborts the stream; consumers must check exact length before accepting a file.

Idle sessions expire after `session_expiry_seconds` (default 86400). Worker/server
leases are separate and do not erase sessions. Background recovery runs every two
seconds, renews worker ownership, verifies uncertain writes and cleans bounded
batches. An unavailable backend can keep a root busy and bytes charged until it
recovers. Never manually delete ledger rows to force the root free. Revocation is
checked on every action and on read-pin renewal. Abort returns durable session
state; it does not promise that physical cleanup has already finished.

Limits: 4 MiB maximum chunks; 256 KiB canonical objects; 1024 references per page;
height at most 8; missing/history pages at most 1000; 64-object recovery/planning
batches; metadata sealing at most 8 objects per request. Chunk/page body ingestion
has a 60-second deadline. `workers` bounds active payload buffers and backend work.
Slow readers use configured candidate connection/initial-response envelopes,
idle/progress budgets and optional overall deadline. Failed candidates are skipped
for the rest of that download; bytes are verified before release.
