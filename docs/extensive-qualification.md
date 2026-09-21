# Destructive Docker qualification

Run `./scripts/qualify` from the repository root. It builds a test image, starts
isolated PostgreSQL 17, pinned MinIO and a controllable TCP fault proxy, and runs
the actual Rust service, SDK and Chromium inside the test runner. Docker is the
only host runtime required besides Bash, Git, tar and ordinary core utilities.
The adjacent `simple-server` Git repository must contain the commit recorded in
`simple-server.rev`; the build exports that exact revision, regardless of its
current checkout. Source modifications in ScT are included deliberately.

The build downloads Rust/Node dependencies and Chromium; missing backend images
are pulled before startup. Cargo runs offline during the campaign. Runtime containers share
an internal Docker network with no published ports. The runner has a 6 GiB memory
limit and four CPUs; PostgreSQL has 2 GiB and MinIO 1 GiB. A separate 64 MiB tmpfs
at `/fault-disk` is the only filesystem deliberately filled to ENOSPC. No
production databases, buckets, directories, credentials or Docker socket are
mounted into the runner. Plaintext S3 is reached through local tunnels, preserving
ScT's loopback-only HTTP restriction. This tests transport failures, not TLS
certificate handling or AWS compatibility.

Every run creates uniquely named containers/network and removes only those
resources. Logs, source revision/diff summary and SHA-256 file inventory, tool versions, generated input
files, server logs and browser traces are retained under
`target/qualification/<run>/`, including on failure. The script propagates failures;
there is no automatic retry to turn a flaky run green. `SCT_QUALIFICATION_ARTIFACTS`
can select another artifact directory. Images/build caches are retained for reuse.

## Campaign

- **Crash boundaries:** 15 versions each on local and local/S3 storage, killing the
  service after admission, after a partial chunk transfer, after sealing, racing
  commit and after an acknowledged commit. Reopen saved SDK jobs, replay receipts,
  verify all three retained versions against independent SHA-256 values, and
  assert exact keep-three history. Only the isolated writer lease timestamp is
  advanced to avoid sleeping through thirty lease-expiry periods; work state is
  recovered by production code. This simulates process loss, not physical power
  failure or storage-controller cache loss.
- **Concurrency:** eight roots, eight rounds of simultaneous mirrored uploads and
  historical reads. Kill/restart between rounds, verify all latest bytes/history,
  reject cross-root version IDs and check capacity-ledger bounds.
- **Network faults:** real S3/S3 mirrors behind a TCP proxy. Disconnect, stall,
  truncate and delay the first candidate. Assert verified fallback within a bound;
  reject mirrored publication while one destination is offline, preserve the old
  version and resume the same upload when the destination returns.
- **Actual payload I/O:** stream a seeded 128 MiB source to a local/S3 mirror and
  verify with a constant-memory hash sink. Reupload unchanged content and assert
  zero new physical copies. Change 4 KiB, measure missing bytes and require reuse;
  verify old/new bytes after restart. `SCT_QUALIFY_MIB` increases this workload
  (minimum 32 MiB; larger values must fit the fixture's quotas).
- **SQLite:** take consumer-owned SQLite backups before/after updates, upload and
  recover each through the SDK, compare bytes, then run `integrity_check` and
  validate row counts. The service does not read live application databases.
- **Disk exhaustion:** preserve a committed backup, fill the dedicated tmpfs until
  the OS reports ENOSPC, reject a replacement, prove the old backup remains
  readable, release the filler, abort failed work and successfully upload again.
  Repeat exhaustion after admission and resume the same saved job after freeing
  space, checking the previous backup remains readable throughout.
- **Existing regression:** core/HTTP/process/maintenance/recovery suites, real S3
  conformance, all five local/S3/mirror file/tree placements, Chromium management
  flows and M4 independent recovery, and independently packaged SDK compilation.
- **Catalog scale:** the million-entry preview/apply test plus ten million
  canonical chunk-reference edges. The latter traverses production GC across a
  writer replacement, preserves a checkpoint-pinned generation and reclaims an
  orphan. Record batch counts, duration, worker RSS and edge-table/index size.

Scale fixtures directly seed metadata, with generation triggers suspended only
while loading and a single generation change afterward; foreign-key checks remain
active. They do not simulate millions of physical payload objects. The reference
fixture intentionally includes repeated references to shared chunks. Payload I/O
is measured separately. Worker RSS excludes PostgreSQL, MinIO and the page cache;
it is sampled, not a guaranteed peak. Runs share the host with unrelated workloads
and must not be treated as isolated hardware benchmarks.

## Release status

This campaign extends M5 testing. Passing it alone does not certify arbitrary S3
providers, physical power-loss durability, multi-day stability or upgrades from a
released format. M5 remains explicitly pending until its full release checklist
and baseline artifacts are complete. No deployment is performed by these tests.

## Findings and recorded checks (2026-09-21)

The concurrent workload reproduced a PostgreSQL deadlock between a pool lock and
its shared staging allocation. Ordinary counter updates invoked a capacity
trigger that acquired the pool after the allocation; publication acquired them in
the opposite order. Migration 19 limits that trigger to allocation creation,
capacity edits and identity edits. Row CHECK constraints still protect counters,
and capacity changes still serialize on the pool. A deterministic regression
holds the pool lock while updating staging counters and verifies budget rejection.
The eight-root workload subsequently completed all 64 mirrored commits and eight
restarts without a revision-conflict failure.

The reference preflight exposed one SQL request per graph edge. GC now marks one
bounded adjacency per statement, deduplicating repeated children before UPSERT.
The same 10,000-reference fixture's traversal dropped from 1,726 ms to 215 ms.
A ten-million-reference standalone run then passed: 637,810 ms fixture load,
74,505 ms GC, 306 passes including writer replacement, sampled worker RSS
19,972–20,072 KiB, 4,676,501,504 bytes for the edge relation and indexes, and
19,544 unique marks. It preserved the pinned copy and reclaimed the orphan.
These are observed development-host measurements, not isolated benchmarks.

The destructive workload passed 30 crash-boundary cases, 64 concurrent mirrored
commits, verified S3 fallback for all injected faults, three SQLite integrity
checks, and filesystem exhaustion both before and after admission. Post-admission
exhaustion resumed the same saved SDK job after freeing space. One recorded
128 MiB mirrored upload/verification took 49,759 ms with sampled server peak RSS
64,948 KiB. A 4 KiB edit required 1,188,380 missing bytes; an unchanged upload
created zero additional physical copies.

The TCP fault harness initially mishandled half-closed connections; actual S3
idempotent-write conformance exposed it. Correct TCP half-close propagation fixed
that harness failure without changing the S3 adapter. Full repository regression
(`./scripts/check`) passed after the product fixes: 12 Chromium/acceptance passes
and the one deliberately pending M5 gate. Container campaign results follow below.

Runner validation also caught a missing `rg` binary: an `if rg ...` condition in
the standalone SDK check could silently skip the forbidden-dependency assertion.
The image now includes ripgrep and `scripts/check-client` requires it before doing
any work. Offline packaging and the dependency-boundary check passed in the
corrected fresh image (`sct-qualification-20260921-093652-714070`).

Migration 19 is forward-only. Checkpoint recovery still requires the current
exact schema/checksum set; export new checkpoints with this build. This change
does not establish backward recovery compatibility with pre-release schema-18
packages.

## Final validation artifacts

Validation completed against the telemetry integration at `3343fbe` plus this
qualification change. `./scripts/check` passed again: formatting, Clippy, Rust,
contracts, frontend, PostgreSQL, MinIO, standalone SDK and 12 browser/acceptance
cases. The single M5 release gate remains intentionally pending.

The full Docker campaign exited 0 in
`target/qualification/sct-qualification-20260921-092210-610157/`.
Its SDK dependency assertion encountered the missing tool described above; that
stage was subsequently rerun successfully in the corrected fresh image under
`target/qualification/sct-qualification-20260921-093652-714070/`.
The updated bootstrap and source inventory also passed the focused S3 stage in
`target/qualification/sct-qualification-20260921-092620-646627/`.
These artifacts are local and ignored by Git; the scripts reproduce the tests.

| Check | Recorded result |
| --- | --- |
| Destructive workload | All 4 groups passed in 249.57 s; 30 crash boundaries, 64 concurrent mirrored commits, transport faults, reuse, SQLite and both ENOSPC timings |
| Streamed mirrored backup | 134,217,728 bytes; ingest plus verification 53,547 ms; sampled server peak RSS 63,460 KiB |
| Small edit | 4,096 changed bytes required 1,188,380 missing bytes; unchanged upload created no new copies |
| Existing integration | 48 core foundation cases plus HTTP/process/maintenance/recovery and real S3 conformance passed |
| Storage placements | All 5 local/S3/mirror file and tree placements passed |
| Namespace scale | 1,000,000 entries; seed 53,510 ms; preview 66,716 ms in 7,814 batches and final apply passed; sampled worker RSS 17,244–17,320 KiB |
| Container browser | All 10 selected management/recovery cases passed in 20.7 s; transfer placements exercised separately |
| Reference scale | 10,000,000 chunk references; seed 663,192 ms; GC 76,654 ms in 306 batches; sampled worker RSS 19,468–19,604 KiB |
| Reference integrity | 19,544 unique marks; pinned copy preserved and orphan reclaimed after writer replacement; edge relation/indexes 4,676,493,312 bytes |
| Independent SDK | Packaged public crates and standalone consumer compiled offline; dependency-boundary assertion passed in corrected image |
