# Portable disc archive format 1

This is an archive format, independent of the ScT SQL schema and service checkpoint
format. The format contract is implemented in `sct-protocol/src/archive_format.rs` and publicly
re-exported by `sct-protocol`. Reader, planner, ISO and UI delivery follow SCT-63 under SCT-62.

## Recovery contract

Every disc carries the SAME COMPLETE target snapshot index and content-to-disc map
as every other disc in its set, plus recovery software/instructions. There is no
privileged final disc. Starting from any survivor must allow enumerating the full
snapshot and discovering missing content without ScT, PostgreSQL or network access.
Objects on older sets may be reused. Old sets never refer to future sets. No disc
or historical recovery point is retired because a newer snapshot does not need it.

A prefix or arbitrary surviving subset supports **complete-file partial recovery**:
restore files whose every extent is verified, report all incomplete paths/chunks,
and never present a partial snapshot as complete. Shared objects on a lost disc
can affect several files. Never substitute an older file version silently.

## On-disc layout

- `disc.json`: canonical `ArchiveDiscDescriptor`, bounded to 64 KiB.
- `snapshot.jsonl`: complete canonical JSONL index, replicated on every disc.
- `START-HERE.txt`: human recovery instructions.
- `recovery/bundle.json`: canonical recovery bundle manifest, bounded to 64 KiB.
- `recovery/`: executable, source archive, format/license documentation.
- `objects/<SHA256(root ID UTF-8)>/<chunk digest>`: raw chunk bytes on this disc.

No credentials, SQL catalog, environment values or service configuration belong in
these images. Object identities remain root-scoped even for equal chunk hashes.
Whole-image checksums live in the service/artifact record, outside the image being
hashed. The descriptor hashes `snapshot.jsonl` and `recovery/bundle.json`; the bundle
manifest hashes its files and excludes itself. No checksum depends on itself.

Identifiers are 1–128 ASCII alphanumeric, hyphen or underscore bytes. IDs are never
rebound to new content. Display disc numbers are positive, monotonic and never reused
once published. A descriptor binds collection, set, set sequence, disc identity,
disc number, first/last number of this set, index digest/length/record count and bundle
digest. All set members share index and bundle digests. Descriptor format must be 1.

## Encoding and record types

Use ScT canonical JSON: compact UTF-8, lexicographic object field ordering, integers
only, no duplicate or unknown fields. JSONL is exactly one canonical record followed
by LF; no BOM, CRLF, blank lines or missing final LF. The hash covers these exact bytes.
A reader must reject noncanonical bytes rather than normalizing them before hashing.

Records appear in the following order (all sorting is bytewise UTF-8):

1. Exactly one `snapshot`: collection/set identities, set sequence, first/last disc
   numbers and source root/version/path selections. Selections are frozen committed
   versions, not changing `latest` pointers; multi-root selection does not assert
   application-wide atomicity.
2. `disc` records for every new disc and every referenced earlier disc, ascending by
   display number. Identities/numbers are unique. Prior discs have an earlier set
   sequence and number below this set's first number; current discs cover the full
   inclusive first–last interval. No references to future sets or disc numbers.
3. `directory` and `file` records in one path-sorted sequence. Include `/` as a
   directory; parents must exist as directories. Each path occurs exactly once.
   Files name a root/version and total length. Directories preserve empty entries.
4. `extent` records sorted by path then increasing byte offset. Each names a root,
   chunk digest and length. For each file, extents start at zero, are contiguous,
   stay in its root and sum exactly to its length. Empty files have no extents.
5. `placement` records sorted by root ID, object digest and disc ID. Each describes
   one whole-object copy, never a byte-range fragment. Every referenced object has
   at least one placement; no unused objects or contradictory lengths are accepted.
   A placement refers to a disc in this index. Multiple placements can provide
   alternatives, and multiple physical burns can provide the same logical disc.

The descriptor and snapshot header must agree. Cross-record uniqueness, ordering,
parent relationships, extent coverage and map completeness require a streaming,
disk-backed index validator; per-record validation alone does not prove a valid set.
Local inventory is derived from placements naming the inserted disc. A reused old
disc may have a different target index, but its collection/disc identity must match
and every consumed object must pass length and SHA-256 verification.

## Limits and compatibility

- Record: at most 65,536 bytes excluding LF; at most 100,000,000 records.
- Entire index: at most 1 TiB, and it plus the bundle and filesystem overhead must
  actually fit on **every** selected medium. Fail planning if it cannot fit.
- Disc count/display number and set sequence: at most 1,000,000.
- Source selections: 1–1,024, further constrained by the snapshot record byte limit.
- UTF-8 absolute logical paths: at most 4,096 bytes, at most 128 components; component
  names follow existing RootPath rules (no empty, `.` or `..`, NUL or `/` component).
- File sizes/offsets use ByteCount: integers 0–9,007,199,254,740,991.
- Objects: 1–4,194,304 bytes; raw SHA-256 as exactly 64 lowercase hexadecimal digits.
- Recovery bundle: 1–64 distinct path-sorted files, each at most 512 MiB, beneath
  `recovery/` or the exact `START-HERE.txt` path; manifest excluded from its own list.

Linux is the initial executable target. Logical leading `/` is removed exactly once
when restoring beneath the destination; it never names the host filesystem root.
Do not follow destination/media symlinks or normalize unsafe paths. Backslash is a
literal Linux filename byte; other platforms must explicitly qualify path behavior.
Readers reject unsupported versions before writing output. Future incompatible
formats need a new version and new compatibility vectors; database migrations alone
must not change the ability to read these bytes.

## Compatibility vectors

`contracts/fixtures/disc-archive` contains six format-only disc directories covering
three historical sets: shared content, a file spanning discs, rename/deletion, empty
directories, and reuse of older discs. Each member of a set contains an identical
complete index. SHA256SUMS fixes the fixture bytes. Small payloads are intentional;
the bundle is explicitly marked fixture-only and is not an executable release.

The protocol test verifies canonical records, descriptor/index/bundle hashes and
reconstructs file bytes from object placements starting with each disc's index.
Executable missing-disc recovery is covered by SCT-69/70; real ISO and ten-disc
adversarial qualification are SCT-78. Format fixtures do not claim hardware testing.
