# M2 qualification — 2026-09-19

M2 implements independent-file upload/replacement, retained history, verified
full/range reads and the standalone Rust SDK over local, S3 and mirrored storage.
The approved plan contains **14 tasks (SCT-14–27) and 17 dependency edges**. Scoped
implementation commits carry ticket links; tasks are handed off for review.
M3–M5 remain explicit pending product acceptance gates. No browser upload or
administration control surface is claimed.

## Bounded-memory measurement

`./scripts/measure-transfer-memory` runs a release-built SDK and HTTP server in
one Linux process with two backend worker slots, real loopback HTTP, PostgreSQL 17,
and the local filesystem adapter. Each workload runs in a fresh process. Source
bytes use seeded xorshift64 generation in a 1 MiB buffer; planning is disk-backed;
downloads stream to a sink. The peak is Linux `/proc/self/status` VmHWM for the
**combined SDK/server process**. PostgreSQL, filesystem cache, Docker and build
processes are excluded. This is a bounded-memory check, not release-scale/load or
provider throughput qualification.

| Source | Chunks | Tree height | Peak combined RSS | Ceiling | Plan/upload/download time |
| --- | ---: | ---: | ---: | ---: | ---: |
| 16 MiB (16,777,216 bytes) | 15 | 0 | 33,708 KiB | 262,144 KiB | 1.41 s |
| 1.5 GiB (1,610,612,736 bytes) | 1,256 | 1 | 43,860 KiB | 262,144 KiB | 97.73 s |

The 96-fold source-size increase added about 10 MiB peak RSS; both runs stayed
under 43 MiB total measured RSS. The larger case exercised multiple leaf pages,
a branch page, missing-chunk pagination and bounded admission traversal. Times
exclude source generation and compilation. Metadata and staging disk use grow
with content; metadata page/worker/chunk bounds limit application RAM. Larger
concurrency, giant trees and long-lived catalog growth remain M5 qualification.

## Executable evidence

- `./scripts/check`: formatting, Clippy, Rust unit/contracts/builds, generated
  contract drift, 49 TypeScript contract cases, frontend checks/build, packaged
  SDK isolation, PostgreSQL/HTTP/OIDC/process integration, real MinIO adapter
  conformance, and Chromium/M1 plus real HTTP/SDK/M2 acceptance.
- `./scripts/test-transfers`: five real placements, lost write acknowledgment and
  durable SDK resume, receipt replay, full/range reads, pinned replacement,
  same-pool mirror charges, changed-source rejection, real local corruption,
  relaxed pruning before new payload, idle expiry and no-space rejection.
- `./scripts/test-integration`: 19 PostgreSQL-backed cases, including actual server
  process kill/restart with an incomplete stored upload and resumed SDK job;
  core cases cover shared references, atomic reservations, authorization,
  protected history, unknown deletion, corrupt/truncated mirrors and expired pins.
- `./scripts/test-s3`: pinned MinIO conformance, conditional writes, verified reads,
  empty tombstone fencing, bad credentials and versioned-bucket rejection.
- `./scripts/measure-transfer-memory`: both workloads above enforce the ceiling
  and require a branched plan for the large file.

The full-product `npm --prefix web run test:acceptance` is expected to exit nonzero
until the three M3/M4/M5 scenarios are implemented. Pending gates never count as
passing tests. The M2 acceptance scenario runs the real HTTP/SDK placement harness;
it does not pretend there is a browser file uploader.

## Operational scope

See [transfer operations](transfers.md), [SDK source capture](rust-sdk.md),
[immutable format](immutable-format.md), [capacity/recovery](capacity-recovery.md)
and [adapter guarantees](storage-adapters.md). Pruned relaxed history remains
retired after failed/aborted uploads. Staging is separately allocated; physical
charges remain until exact-generation cleanup is confirmed. Read pins can prolong
outgoing staging occupation and root ownership. Credentials and grants remain
revocable while a transfer owns the root.

S3 results qualify pinned MinIO (`RELEASE.2025-04-22T22-12-26Z`), not AWS infrastructure
or arbitrary compatible providers. Local durability depends on fsync/atomic
filesystem semantics. Versioned/locked buckets are rejected. Zero-byte generation
tombstones and catalog records consume overhead and are not payload-capacity
credits; provision metadata/physical-block headroom. Session/receipt/catalog
metadata compaction and full release-scale performance remain later operational
work. The repository has local commits; no deployment or push is part of this work.
