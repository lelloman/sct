# Offline archive inspection and recovery

`sct-recover` depends on the public protocol and a bundled SQLite work index, not
ScT server/core, PostgreSQL or network services. The executable is built with
`cargo build --locked -p sct-archive --bin sct-recover`. Packaged recovery bundles follow in SCT-71; these commands operate
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
