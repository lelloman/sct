# Append-only disc archives

Approved design under [SCT-62](https://crumbles.lelloman.com/w/LLPR/SCT/62).
Implementation tasks SCT-63–79 are assigned to Codex. Account-owned collections
and a Linux x86_64 standalone recovery bundle are the approved initial defaults.
Delivery is in progress; this document is a product contract, not a claim that all
interfaces below are implemented. See [format 1](disc-archive-format.md) and the [approved tasks](disc-archive-tasks.md).
Operators use the separate [physical optical-media drill](optical-media-drill.md)
to burn, read back and recover an exported set; its current qualification record
does not claim hardware support until an actual drive/media run succeeds.

## Product contract

- Each completed archive set preserves one explicitly selected, immutable recovery
  point. Later source changes cannot silently change an in-progress export.
- EVERY disc contains the recovery software/instructions, COMPLETE target snapshot
  index, and complete content-to-disc map for its frozen archive plan. Any surviving
  disc can bootstrap recovery; no final disc or earlier catalog is a metadata dependency.
- Every disc identifies the archive prefix ending at that disc and its available
  content. Recovery can use that prefix alone, or any surviving subset of discs.
  A prefix is not necessarily a complete snapshot: restore complete files whose
  required content survives and report incomplete files precisely.
- Data may live on earlier discs. Restoring an old recovery point never requires
  a later archive set. A multi-disc set becomes a recovery point only when complete.
- Never retire discs or invalidate historical recovery points. A disc unnecessary
  for the newest snapshot can remain necessary for an older one.
- Optical history survives online snapshot retention and loss of the ScT database.
- Exporting an ISO does not establish that a physical disc exists or is readable.

## 1. Account state, logical media, and physical media

| Entity | Meaning |
| --- | --- |
| Collection | Stable identity, owner, selected source scope, media byte budget, ordered archive history |
| Recovery point | Frozen source version identities, complete snapshot index, required content/discs, archive-format and recovery-tool versions |
| Export plan | Preview of existing content reuse, new bytes, metadata/tool overhead, disc assignments and estimated image sizes |
| Logical disc | Permanent identity and display number, set membership, content inventory, complete recovery bundle and prefix identity |
| Disc image | Immutable generated artifact, exact byte length and checksum, bound to a logical disc and plan revision |
| Physical copy | A particular burn of a logical disc; label, optional location, reported burn and verification history |
| Archive job | Resumable capture, packing, image generation and verification coordination |

Keep these separate: one ISO can have multiple physical copies; downloading an
image is not burning it. A failed burn can be retried from the same immutable image.
Damaged/lost copies receive observations, never removal of historical references.
Numbers and identities on published images are never reused for different content.

Collection views answer: latest completed recovery point; latest disc number;
which discs contain an object/path; which points require a disc; what is planned
for the next set; which images/copies remain unfinished. No 'safe to retire' action.

Mutable previews carry revisions. Freezing a plan fixes content and disc identities.
Use one active append job per collection initially. Incomplete runs remain visible;
later runs must not depend on them as though they were complete verified archives.

## 2. Standalone recovery and portable archive format

Create a separately versioned archive format and a small standalone recovery tool.
The current administrator checkpoint format exports the entire service catalog
and requires matching database migrations; it is not the user archive format.

Suggested image layout:

    START-HERE.txt
    disc.json
    recovery/                 # tested executable(s), source and format documentation
    objects/                  # immutable content addressed by identity
    snapshot/                 # complete index and required-disc map on EVERY disc

The recovery tool loads ANY disc, lists the snapshot and required disc IDs,
accepts discs in any order, verifies identities/lengths/hashes, and resumes copying
into an explicitly selected local destination. It must work without ScT, its
database, original backend credentials, login provider, compiler or network.
Ship executable builds for explicitly supported platforms and source for future
porting; source alone is not an offline executable. Start with a CLI, with commands
for inspect, verify and restore. A bootable operating-system image is separate scope.

Use a disk-backed work index for large snapshots and bounded streaming for payloads.
Preserve empty directories and the metadata ScT actually stores. Reject unsafe
paths, corrupt manifests and destination collisions instead of silently overwriting.
Record interrupted work and identify exactly which disc/content is missing.

For a ten-disc set with disc 10 lost, disc 9 (or any other survivor) must recover
all complete files whose required objects are present on discs 1–9 or available
earlier sets, without opening disc 10 or contacting ScT. Missing disc 10 must not
prevent enumerating the full snapshot or recovering unrelated files. Deduplicated
chunks on a missing disc can affect several files; a file spanning discs is not
complete merely because some of its bytes survive. Partial files, if explicitly
requested, are marked separately and never passed off as verified originals.
Do not silently substitute older file versions and call the result the same snapshot.
The report distinguishes a complete restore, verified complete files, and missing
files/chunks. Application consistency is not promised for a partial snapshot.

Reuse existing manifest/chunk contracts where appropriate, but isolate the reader
from service database migrations. Root-scoped identity remains explicit; do not
introduce cross-account or cross-root deduplication through archive planning.

## 3. Web surfaces and API

- Collections list: owner, source scope, latest completed recovery point, disc count
  and unfinished work.
- Collection timeline: every recovery point and its complete required-disc list.
- Disc detail: identity, contents, image download, physical copies, observations,
  and what is recoverable from its prefix or a selected surviving subset.
- Next-set preview: chosen source versions, reused/new bytes, disc count/capacity
  and software/index overhead, followed by generation progress and ISO downloads.
- Verification: distinguish user-reported burn status from a local verifier report.

These are archive management/export actions. Browser users still cannot upload,
edit, delete or restore content into ScT snapshots. Offline recovery writes to the
user-selected local destination. The browser does not need direct burner access.

Account ownership alone cannot grant source access: enforce source read/export
authority when capturing and authorizing artifact downloads. Define revocation
behavior for queued jobs and generated artifacts explicitly. Do not include the
administrator checkpoint catalog, credentials or grants in user discs.

## 4. ISO production and source lifetime

Capture selected committed versions and pin the required source content while
the exporter reads it. The source selection must be explicit: an atomic directory
snapshot is already immutable; independently versioned files require a frozen
selection of version IDs, not a walk through changing 'latest' pointers.

Plan reuse against completed, verified archive contents. Newly required data is
packed under the configured media byte budget, including filesystem overhead,
the complete index and recovery software on EVERY disc. Freeze the whole set's
layout before publishing its first image so all replicated indexes describe the
same assignments; revalidate capacity after serialization and packing stabilize.
Prefer keeping a file's newly required chunks together when capacity allows, to
reduce the number of incomplete files after one disc fails. Large files and shared
chunks may still span discs. Build to temporary storage, reopen
and verify the image, check its ACTUAL size, then publish it atomically for download.
Frozen images are never edited after publication. Repeated downloads return the
same bytes. Interrupted builds cannot appear as ready artifacts.

GNU xorriso is a candidate external ISO writer; it supports ISO 9660 images and
optical media. Keep the writer behind an adapter and qualify reading/export on
the supported platforms. Reference: https://www.gnu.org/software/xorriso/

Content stays in bounded objects rather than depending on original large-file
names/sizes in the ISO filesystem. Explicitly fail a plan if its complete index
and recovery bundle cannot fit each disc; do not silently replace the full
index with a chain of prior indexes.

Image checksum verification and physical read-back verification are separate.
The local tool reads the burned medium and produces a report bound to disc/content
identities; the server records that report as an observation, not remote proof of
hardware possession. Source-pin release, image cache cleanup and quotas need an
explicit policy so abandoned exports cannot pin source data forever. None of these
cleanup policies may erase completed optical history.

Avoid self-referential checksums: stable disc IDs and content-manifest hashes
identify disc contents; do not place an ISO's own whole-image hash inside that ISO.

## Implementation slices and acceptance

1. Specify archive contracts and fixtures; implement the standalone reader against
   synthetic multi-disc directories before integrating a server.
2. Add collection/recovery-point/disc/image/physical-copy catalog and authorization.
3. Add consistent source capture, bounded enumeration, pins and incremental planning.
4. Add resumable image generation, exact-capacity validation and artifact downloads.
5. Add standalone restoration, physical read-back verification and packaged software.
6. Add collection, timeline, plan, disc and download surfaces to the web application.
7. Add isolated end-to-end qualification and operational documentation.

The central E2E drill: archive A across several tiny test ISOs; change files,
rename and delete entries; archive B with content reuse; change again and archive C.
Remove access to ScT, PostgreSQL and original storage. Starting from EVERY ISO in
each set in turn, restore its target snapshot using its listed discs and compare
paths and bytes. Recovery entry points must be interchangeable within a set.
A disc unused by C but required by A must remain represented and usable.

Mandatory degraded-recovery drill: generate a ten-disc set, remove disc 10 and
start from disc 9. Recover exactly the complete files supported by surviving
content and report every incomplete file. Repeat with a missing middle disc and
starting from disc 1; test files spanning several discs and shared chunks. Compare
all recovered bytes and ensure the tool never labels a partial snapshot complete.
Also interrupt burning after disc 9: the already written prefix must be usable
for partial recovery even though the server never marked the set complete.

Also test full/boundary capacity including index overhead; interrupted/replayed
jobs; concurrent online retention; missing/corrupt/wrong/reordered discs; duplicate
physical copies; partial burns; grant revocation; restore interruption; unsafe
paths; and regeneration/download immutability. Real burner/media read-back is a
separate hardware qualification from ISO-only container tests.

## Configuration and implementation details

- Collections live in ScT user accounts; offline recovery has its own local work index.
- Initial source scope: one atomic snapshot per recovery point or a captured set
  of committed versions across selected paths/roots.
- Initially Linux x86_64; media budgets are explicit bytes including repeated recovery metadata.
- Source-pin/image-cache quotas and release behavior for unfinished burns.
