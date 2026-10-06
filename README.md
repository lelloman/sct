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
The server links a separately built `libsimple_server_engine.so.1`. The matching
host library lives in the sibling `../simple-server` checkout at the commit in
`simple-server.rev`; the source backend is no longer compiled into ScT.
PostgreSQL/SQLx, OIDC/Reqwest and application Tokio remain on the application side.
The shared engine serves HTTP and static files; `sct-server::web` keeps static
route declarations concise and bridges graceful shutdown to the engine.

For local development, check out the recorded library revision, then build its
engine once and expose it to Cargo and the runtime loader:

```sh
cargo build --locked --manifest-path ../simple-server/crates/engine/Cargo.toml
export SIMPLE_SERVER_ENGINE_DIR="$(realpath ../simple-server/crates/engine/target/debug)"
export LD_LIBRARY_PATH="$SIMPLE_SERVER_ENGINE_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
ln -sf libsimple_server_engine.so "$SIMPLE_SERVER_ENGINE_DIR/libsimple_server_engine.so.1"
```

`./scripts/build-image` builds a reusable, revision-tagged engine image and a
local `sct-engine-canary:local` runtime image. It exports the exact library commit,
verifies the engine checksum, and copies the same `.so` into builder and runtime
images. The consumer builder does not compile the native engine. Repeated builds
reuse the engine image and BuildKit Cargo caches; nothing is pushed. `scripts/qualify` uses the same
pinned source and engine image. Production storage/configuration remains
application-owned and must be supplied when running the image.

The local canary on 6 October 2026 passed all 61 workspace tests (92 existing
ignores), all 12 database-backed server tests, and real-process HTTP drain,
SIGINT/SIGTERM, writer-release/restart and lease-loss checks. The release container
loads the engine and serves static/API/health responses before graceful shutdown.
A bounded-concurrency run also passed 65 core database contracts, explicitly
excluding the known failure below. The broader core suite has an existing archive migration-name mismatch
(`draft_names` versus `draft names`), reproduced on the unchanged source backend;
its unrestricted parallel run also exhibits database/timing failures. This is
not a claim that the full qualification matrix is green. No image was published
or deployed.

Three isolated debug-build trials (eight Cargo jobs, engine prebuild excluded)
gave median clean builds of **36.49s source / 33.77s engine** (about 7.5% faster).
Timestamp-only rebuilds were **1.13s / 1.15s**, effectively unchanged. ScT still
compiles its own database/client/runtime dependencies. See the
[raw measurements](docs/measurements/sct-engine-build-2026-10-06.json).
To reproduce, export baseline commit `9e118e6` into a separate directory and run:

```sh
./scripts/benchmark-engine-builds --baseline /path/to/baseline \
  --engine-dir "$SIMPLE_SERVER_ENGINE_DIR" --output /tmp/sct-builds.json
```

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
