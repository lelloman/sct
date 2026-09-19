# Step 02: lifecycle and main

Originally implemented on `simple-server-step01`; local `master` was rebased
onto that branch on 2026-09-19, preserving subsequent storage and tree work.

The foundation service coordinates HTTP and writer-lease renewal with shared
`Signals`, `Lifecycle`, and HTTP adapters. SIGINT/SIGTERM stop accepting requests;
active HTTP requests drain while writer-lease renewal continues. Renewal stops
only once HTTP finishes, preserving ownership for writes during draining. Lease loss is a service
failure and produces a nonzero exit. Final cleanup attempts writer release and
closes the catalog pool, including after a service failure. One 30-second budget
covers drain and cleanup. The frontend-only scaffold uses the same HTTP lifecycle.
Local administration commands retain their existing setup and cleanup sequence.
Startup is outside the drain budget. The catalog's existing transaction fencing
still applies if renewal fails or the lease expires.
On failure or deadline expiry the application exits nonzero explicitly, rather
than waiting indefinitely for runtime teardown. The helper does not forcibly
terminate blocking jobs or supervise future storage workers.

Transfer heartbeats run alongside writer-lease renewal. The existing transfer
recovery task is aborted and joined during cleanup before writer release and
catalog closure; its retry policy remains unchanged. New transfer, tree and
policy routes use the shared Axum re-export.

The workspace uses sibling `../simple-server` with the `lifecycle` feature.
`simple-server.rev` records reviewed source commit
`c5359079ff4fad0b4b0359e8c88880dbbbc4eeb5`; run
`scripts/checkout-simple-server.sh` for a fresh checkout after that revision is
published. The helper refuses to replace an existing different/dirty checkout.
The coordinated local checkout can include later documentation commits with the
same source. No publication or deployment is part of this migration.

Original migration validation (2026-09-19, before the master rebase):

- Full `CARGO_BUILD_JOBS=2 SCT_E2E_PORT=19083 CARGO_PROFILE_DEV_DEBUG=0
  CARGO_PROFILE_TEST_DEBUG=0 scripts/check`: strict Clippy, formatting, Rust tests,
  PostgreSQL integration, workspace build, independent client packaging and
  compile, generated contracts, 49 JavaScript contract tests, frontend checks
  and production build, and three Chromium E2E scenarios pass. Four existing
  future-milestone scenarios remain explicitly pending.
- `scripts/test-lifecycle.py`: real binaries, both signals, an 8 MiB HTTP response
  completing during drain, foundation restart immediately after writer release,
  and writer-lease loss causing nonzero exit. Uses disposable Postgres and files.

Rebase validation (2026-09-19): workspace/all-feature tests passed (28 tests,
32 explicitly ignored), PostgreSQL integration passed (31 tests), strict
all-target/all-feature Clippy and workspace build passed, and the real-process
signal/drain/restart/lease-loss script passed. Concurrent namespace-move work
was present during integration validation and is excluded from this commit.
Browser, real-S3 and transfer-placement qualifications were not rerun for the rebase.
