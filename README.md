<img src="web/public/brand.svg" width="72" height="72" alt="ScT chest icon" />

# ScT — Stipa ca Truova

Self-hosted versioned storage. **M1–M4 are implemented**: authenticated roots,
namespace/policies, metadata-first file upload admission, immutable local/S3/mirrored
storage, resumable Rust SDK jobs, keep-N history and verified full/range downloads.
Atomic snapshots, hide/purge and acknowledged policy transitions are available through
the HTTP API and standalone Rust SDK. The browser supports management, browsing and
downloads; programs own backup content changes. M4 adds maintenance, integrity,
checkpoint export and isolated recovery. Broader release qualification remains M5.

See [service setup](docs/foundation.md), [transfer operations](docs/transfers.md),
[the Rust SDK](docs/rust-sdk.md), [M2 qualification](docs/m2-qualification.md) and [M3 qualification](docs/m3-qualification.md),
[browser management](docs/browser-management.md) and [M4 qualification](docs/m4-qualification.md).

The primary repository is hosted on Fucina. The public
[GitHub mirror](https://github.com/lelloman/sct) updates automatically on pushes
and every ten minutes. Maintainers push to Fucina through `origin`.

## Development

Prerequisites: Rust 1.96.0 (installed by rustup from `rust-toolchain.toml`), Node.js
24.13+ (`nvm use` reads `.nvmrc`), npm, Bash, ripgrep, jq, xorriso, util-linux (`prlimit`), and Docker for disposable PostgreSQL
integration tests. Dependency versions are captured in `Cargo.lock` and
`web/package-lock.json`. The check scripts target Linux development/CI hosts.
`simple-server.rev` records the reviewed shared-library checkout, including the
auth feature used by the HTTP server and transaction-scoped core checks, plus
the shared routing, HTTP interfaces and backend-independent tracing observer.
See [Step 03c](docs/step-03c-http-tracing.md) and
[Step 11](docs/step-11-routing.md).

```sh
npm --prefix web ci
cd web
npx playwright install chromium
cd ..
./scripts/check
```

On a minimal Linux host, install the browser's system dependencies with Playwright's
documented `npx playwright install --with-deps chromium` setup command. The initial
dependency, browser and PostgreSQL image downloads require network access.

`scripts/check` runs Rust formatting, Clippy, unit/HTTP integration tests, builds,
web formatting/lint/type checks, independent client packaging/build, real PostgreSQL
integration, and Chromium tests against the actual Rust server. It exits on the first
failure. Pending milestone scenarios are printed separately and do not count as
passing acceptance. To require complete product acceptance, use:

```sh
./scripts/check
npm --prefix web run test:acceptance
```

The second command **currently fails intentionally** because the M5 milestone
acceptance scenario remains pending; M1–M4 browser and HTTP/SDK scenarios execute. These gates must be expanded into the detailed
workflows in [the acceptance matrix](ARCHITECTURE_PLAN.md#16-test-and-acceptance-matrix),
not simply removed to obtain a green result.

Run the frontend-only scaffold (for the database-backed service, follow
[the foundation setup](docs/foundation.md)):

```sh
npm --prefix web run build
cargo run --locked -p sct-server
```

Open <http://127.0.0.1:8080>. The server serves `web/dist` from the same origin.
For web hot reload, run `npm --prefix web run dev` alongside the Rust server;
Vite proxies `/api`, `/auth`, and `/health` to port 8080. When testing cookie
authentication through Vite, set `SCT_PUBLIC_URL` to its loopback origin.

| Environment variable | Scaffold behavior |
| --- | --- |
| `SCT_BIND` | Socket address; defaults to `127.0.0.1:8080` |
| `SCT_WEB_DIR` | Built assets directory relative to the process working directory; defaults to `web/dist` |
| `SCT_CONFIG` | TOML configuration path; required for the database-backed foundation service |
| `SCT_DATABASE_URL` | PostgreSQL secret referenced by the example configuration |
| `SCT_TEST_DATABASE_URL` | Optional **dedicated test database** for `scripts/test-integration`; otherwise the script owns and removes a disposable Docker container |
| `SCT_E2E_PORT` | Browser test server port; defaults to `18080` |

`GET /health/live` checks process liveness. `GET /health/database` returns 204
after a successful PostgreSQL query or 503 if unconfigured/unavailable. `GET /health/ready` additionally checks the writer lease in configured service mode.
None claims payload storage is ready. The configured service implements the M1/M2
[operation map](contracts/operations.md); the frontend-only scaffold keeps returning
structured 404 responses for application routes.

Useful focused checks:

```sh
cargo test --locked --workspace
./scripts/test-integration
./scripts/check-client
./scripts/test-s3
./scripts/test-transfers
./scripts/measure-transfer-memory
npm --prefix web run test:e2e   # disposable PostgreSQL + real server/Chromium; requires builds
```

For formatting, run `cargo fmt --all` and `npm --prefix web run format`.

## Extensive qualification

Run `./scripts/qualify` for the isolated Docker crash, network-fault, ENOSPC,
concurrency, large-payload, SQLite and catalog-scale campaign. It retains failure
artifacts and uses only disposable storage. See [the qualification guide](docs/extensive-qualification.md)
for prerequisites, resource limits and the exact scope.

## Component boundaries

| Component | Responsibility |
| --- | --- |
| `sct-protocol` | Public Rust-owned contracts with generated OpenAPI, JSON Schema and TypeScript |
| `sct-client` | Standalone authenticated foundation API client; transfer/chunking follows in M2; depends on protocol, never core/server |
| `sct-core` | Authorization, catalog, roots, policies, namespace and operation fencing; immutable payload coordination and recovery |
| `sct-server` | Shared HTTP routes, Tokio runtime, configuration and worker wiring |
| `web` | Vue 3/TypeScript UI; wire models are generated from Rust contracts |

The external sample is a template copied to a temporary directory by
`scripts/check-client`. It builds against packaged client/protocol crates with no
server/core checkout. It is not part of the root workspace. No other application
repository is needed.

## Plan and next steps

- [Architecture and product contract](ARCHITECTURE_PLAN.md)
- [Ticket plan and amendments](TICKET_PLAN.md)
- [Implementation prerequisites from the readiness review](docs/implementation-readiness.md)
- [Approved icon and shared palette](docs/branding.md)
- [Protocol contracts, generation and wire conventions](contracts/README.md)

Next: complete M5 release qualification. The [extensive test campaign](docs/extensive-qualification.md)
covers crashes, faults and scale; M1–M4 tasks are implemented;
see [M4 qualification](docs/m4-qualification.md) for current coverage and limits.

Technical references: [Axum serving](https://docs.rs/axum/0.8.9/axum/fn.serve.html),
[SQLx PostgreSQL wiring](https://docs.rs/crate/sqlx/0.8.6/source/README.md),
and [Vite development/build setup](https://vite.dev/guide/).

Lifecycle migration and sibling-library checkout instructions are in
[Step 02](docs/step-02-lifecycle.md). The migration remains on its isolated branch.
