# Transfer integration qualification

Run these from the repository root with Docker available:

- `./scripts/test-integration`: isolated PostgreSQL schemas; authorization,
  ledgers, admission/pruning, shared chunks, pins, recovery, and actual server
  process kill/restart with an incomplete local upload and durable SDK job.
- `./scripts/test-s3`: real SigV4/conditional-write/read/tombstone conformance,
  corruption, wrong credentials, and rejection of versioned buckets.
- `./scripts/test-transfers`: disposable PostgreSQL and pinned MinIO, real local
  directories, and the public HTTP/Rust SDK workflow on local, S3, local/S3,
  local/local and S3/S3 placements. Both mirrors deliberately share a pool.

The real-placement suite transfers a seeded 6 MiB multi-chunk file, injects a lost
response **after an actual backend write**, resumes a saved SDK job, replays the
receipt, downloads full/range bytes, replaces while a reader pins the old version,
checks physical-copy accounting, and rejects changed sources. Local payloads are
actually corrupted to test single-copy failure and verified mirror fallback.
Small-quota roots exercise oldest relaxed pruning before new payload, idle expiry,
irreversible retirement, and no-space rejection. Missing/unavailable configured
services fail the suite; there is no fallback to memory payload stores.

Core fault tests separately exercise truncation, unknown deletion outcomes,
uncertain-write reconciliation, expired pins, slow progress and stalled readers.
Each test runner removes its disposable services on exit. Supplying
`SCT_TEST_DATABASE_URL` to the PostgreSQL-only runner requires a dedicated test
instance; tests create isolated schemas. MinIO integration qualifies this pinned
S3-compatible implementation, not every S3 provider or AWS deployment. Local
persistence depends on the filesystem honoring fsync and atomic same-filesystem
links/renames.
