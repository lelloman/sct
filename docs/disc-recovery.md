# Offline archive inspection and recovery

`sct-recover` depends on the public protocol and a bundled SQLite work index, not
ScT server/core, PostgreSQL or network services. The executable is built with
`cargo build --locked -p sct-archive --bin sct-recover`. Production bundles are built with `scripts/build-recovery-bundle`; these commands operate
on mounted or extracted disc directories using archive format 1.

Use a private working directory on writable local storage. The database path must
be new on first use; a failed initial import leaves an unusable work file, which
can be discarded or replaced with a different filename. It contains snapshot paths
and should be protected like backup metadata. Indexing uses an 8 MiB SQLite cache,
file-backed temporary storage, bounded 64 KiB records and bounded query pages.

```sh
sct-recover inspect /media/disc /private/recovery.sqlite
sct-recover verify /media/disc /private/recovery.sqlite > read-back.jsonl
```

Start from ANY disc in a set. `inspect` emits the descriptor, required disc list
and snapshot entries as JSONL. A reused work index must belong to the same target
snapshot. `verify` emits one bounded observation per expected object and a final
summary; stderr prints a human-readable count. Exit 0 means the selected disc's
required payloads passed, exit 2 means missing/unreadable/corrupt payloads, exit 1
means invalid metadata or an operational failure, and exit 64 means invalid usage.
A successful verification of one disc does not mean the entire set is present.

Indexes must match their descriptors' lengths, counts and SHA-256 values. Before
an index is committed, the reader validates canonical encoding, ordering, parent
directories, unique disc identities, source versions, contiguous file extents,
object lengths, complete placements and absence of future-set dependencies. It
also checks the recovery bundle manifest and all declared bundle files.

Media reads are descriptor-relative and refuse symlinks and non-regular files.
Chunk bytes are hashed in bounded blocks. Verification reports distinguish missing,
unreadable, corrupt and verified objects; an unavailable object never counts as
verified. Earlier discs used by later snapshots retain their own indexes: the
library verifies collection/disc identity and hashes the needed objects against
the target snapshot's map. No old disc needs to be rewritten.

The six canonical fixture discs are format-only fixtures, explicitly lacking an
executable recovery bundle. They test the reader, not software packaging or actual
optical hardware. Hardware burn/read-back qualification is a separate task.

## Restoring files across discs

Create a private cache directory and a destination directory on writable storage.
The cache holds one verified copy of each required chunk; allow space for those
chunks, the work index, and restored files. Keep the cache dedicated to this work
index and out of the restore destination. Begin from any surviving disc in the
chosen set, then insert required earlier discs in any order:

```sh
mkdir -m 700 /private/cache /private/restored
sct-recover ingest /media/disc /private/recovery.sqlite /private/cache
# Repeat ingest after each disc swap. Restarting the process is safe.
sct-recover restore /private/recovery.sqlite /private/cache /private/restored
# If some discs cannot be obtained:
sct-recover restore /private/recovery.sqlite /private/cache /private/restored --partial
```

The default restore checks every required chunk before writing anything. Partial
mode restores only complete files, emits an observation for each missing file
extent (path, offset, root, digest, length), and exits 2. An object shared by several
files makes every affected file unavailable. No previous version is substituted.
This reports file recovery, not application-level consistency of a partial snapshot.

Resume by ingesting additional discs and repeating restore. Cache bytes and existing
files are rehashed every time. A corrupt regular cache file can be repaired by
ingesting a good copy; a destination conflict is reported and never overwritten.
Symlinks and non-regular files are refused. Empty files/directories are restored.
Directory permissions are 0700 and new file permissions 0600; the archive does not
claim to preserve POSIX ownership, executable bits or timestamps.

Files are staged under a random `.sct-recovery-` name in the destination parent,
flushed, and published atomically without replacing any existing path. A killed
process may leave such an incomplete temporary file; it is never considered a
restored file or cache object. Resume creates a new temporary file. After ensuring
no recovery is running, these orphan temporary files can be inspected and removed
manually to reclaim space. Do not remove unrelated destination files. Only one
process may hold a work index at a time; its operating-system lock releases on exit.

## Full physical-copy read-back reports

After burning, mount the actual disc read-only and run:

```sh
sct-recover verify-report /media/disc /private/disc-original.sqlite > disc-report.json
```

Use a private work index for that disc's **original snapshot**, not a newer
snapshot that reuses only some of its payload. The command checks the original
inventory and emits one JSON observation on stdout; per-object details stream to
stderr. The observation binds collection/disc identities, original index and bundle
hashes, and SHA-256 of canonical object records with LF separators in root/digest
order. Exit 0 means metadata and all original payload objects verified, exit 2
means an incomplete read-back, and exit 1 means identity/metadata could not be
established or another operational failure. A cached original work index can still
report a later metadata read failure; unusable metadata on first import fails
without producing a valid report. None of these failures permits a verified claim.

Register a physical copy with its label, location and optional reported burn time,
then submit this observation with a new request ID and the copy's current revision
to `/api/v1/archives/{collection}/copies/{copy}/reports`. The browser provides this
management surface; the standalone recovery program never needs credentials or a
network connection. The server rejects a report for a different frozen inventory.
These are attributed owner-supplied observations, not cryptographic evidence that
a physical medium was burned: running against an extracted ISO also reads those
bytes, so operators must select the actual mounted media for physical verification.

ISO creation and download leave physical-copy health Unknown. Only a successful
bound read-back observation sets ReadBackVerified; an incomplete report sets
Damaged. A later Missing/Damaged/Unknown observation changes current availability
without deleting earlier reports. Label/location updates can omit `health` to
preserve it. Events and reports are immutable and paged separately; retries with
the same report request ID never duplicate observations or undo newer health.

A recovery point receives a permanent historical completion timestamp only when
every new disc (including metadata-only discs) and every earlier dependency has at
least one currently verified healthy copy. Large collections finish this accounting
in durable batches of 32 pending sets; the normal worker advances the queue.
Multiple physical copies can satisfy the same logical disc. Later loss or damage
does not erase the earlier completion event, and no disc is retired automatically.
