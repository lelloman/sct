# Step 01: Axum centralization

The server and its tests use public `simple-server` Git revision
`46a36391ccca522f3ec9aa24206f2b231162dd96` and its transitional Axum re-export.
Axum remains exactly 0.8.9; the client/protocol crates retain their independent
distribution boundary.

The migration was developed on `simple-server-step01` from `5cffda9`.
Local `master` was rebased onto it on 2026-09-19, preserving subsequent storage
and tree development and adapting newly added Axum imports. The validation
below records the original migration before that integration.

Validation on 2026-09-19, Rust 1.96 and Node 24.13:

`SCT_E2E_PORT=19083 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 scripts/check`
passed before and after migration. This includes:

- 33 Rust tests including Postgres integration, plus one doctest.
- Strict all-target/all-feature Clippy, formatting and workspace builds.
- Generated-contract verification and all 49 JavaScript contract tests.
- Frontend formatting/lint/type checks and production build.
- Independent protocol/client packaging and standalone-client compilation.
- Three real-server Chromium E2E scenarios for the built UI, liveness/auth,
  root provisioning, grants, namespace and policy boundaries.

Four future milestone acceptance scenarios are explicitly marked pending and
skipped by the existing suite. This migration does not implement those milestones.
The tests used temporary Postgres containers and isolated fixture data, with
cleanup handled by the scripts. The dependency graph contains only central
Axum 0.8.9. No deployment or publication was performed.

Step 02 subsequently adopts shared lifecycle helpers and a reviewed sibling
checkout; see [the current lifecycle migration](step-02-lifecycle.md). The Git
pin above describes the historical Step 01 state.

## Owned static-file fallback — 2026-10-04

The production router now installs `simple_server::web::static_files::StaticDir`
instead of Tower HTTP `ServeDir`. Both the workspace and core use published
`lelloman-simple-server =0.1.6` (source
`58e9b7504176c82b538dca0cd3041468fb4a1ccd`, archive SHA256
`4f37d395af4d6ca8495ad190b8cecc5ab078ddfc92b8e9b3a41bc9eee80d3fce`).
Only the server enables the static-files feature. Direct Tower HTTP dependency
entries were removed; dependency metadata finds no direct normal Axum/Tower
HTTP/Hyper server dependencies in this workspace.

Defaults and route placement are unchanged: directory index lookup, GET/HEAD,
range/Last-Modified responses, directory redirects, missing-file 404 and
unsupported-method 405. No SPA fallback, cache policy, compression or startup
filesystem validation was added. Reserved `/api` and `/api/{*path}` routes keep
the structured unsupported-operation response instead of serving static content.
Existing health, correlation, auth, storage and worker behavior are preserved.

A new real-TCP test calls the production router and passes on the old and new
file service. It covers index/MIME/body, HEAD length/body, range/status/body,
modification conditionals, query-preserving redirect, absent files/traversal,
unsupported method, and API/static separation. Baseline workspace tests pass
58 tests with 92 infrastructure ignores; final tests pass **59 with the same
92 ignores**. Strict all-target/all-feature Clippy, workspace formatting and
diff checks pass. The initial debug-info-disabled baseline link hit an undefined
SQLite/Tokio monomorphization symbol; the successful baseline/final builds use
`CARGO_INCREMENTAL=0` and `--config 'profile.dev.package.sqlx-sqlite.codegen-units=1'`
with `CARGO_PROFILE_DEV_DEBUG=0`, isolated build output and two jobs. Repository
build policy was not changed. PostgreSQL/S3/destructive qualification and web
frontend suites were not rerun for this HTTP fallback migration.
