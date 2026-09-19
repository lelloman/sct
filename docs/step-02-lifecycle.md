# Step 02: lifecycle and main

Implemented on the isolated `simple-server-step01` branch in `sct-step01`.
The original `sct` checkout and ongoing storage work are untouched; this branch
must be reconciled with that work before merging.

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

The workspace uses sibling `../simple-server` with the `lifecycle` feature.
`simple-server.rev` records reviewed source commit
`c5359079ff4fad0b4b0359e8c88880dbbbc4eeb5`; run
`scripts/checkout-simple-server.sh` for a fresh checkout after that revision is
published. The helper refuses to replace an existing different/dirty checkout.
The coordinated local checkout can include later documentation commits with the
same source. No publication or deployment is part of this migration.

Validation (2026-09-19):

- Full `CARGO_BUILD_JOBS=2 SCT_E2E_PORT=19083 CARGO_PROFILE_DEV_DEBUG=0
  CARGO_PROFILE_TEST_DEBUG=0 scripts/check`: strict Clippy, formatting, Rust tests,
  PostgreSQL integration, workspace build, independent client packaging and
  compile, generated contracts, 49 JavaScript contract tests, frontend checks
  and production build, and three Chromium E2E scenarios pass. Four existing
  future-milestone scenarios remain explicitly pending.
- `scripts/test-lifecycle.py`: real binaries, both signals, an 8 MiB HTTP response
  completing during drain, foundation restart immediately after writer release,
  and writer-lease loss causing nonzero exit. Uses disposable Postgres and files.
