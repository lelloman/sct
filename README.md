<img src="web/public/brand.svg" width="72" height="72" alt="ScT chest icon" />

# ScT — Stipa ca Truova

Self-hosted versioned storage. **Milestone 1 (foundation) is implemented**: PostgreSQL
catalog/migrations, fenced configuration/runtime, OIDC and application credentials,
roots/grants/capacity allocations, policies, and directory namespace operations.
Uploads/downloads, real payload adapters, retention execution, and the complete
administration UI remain milestones 2–5.

See [foundation setup, API behavior and verification](docs/foundation.md).

The primary repository is hosted on Fucina. The public
[GitHub mirror](https://github.com/lelloman/sct) updates automatically on pushes
and every ten minutes. Maintainers push to Fucina through `origin`.

## Development

Prerequisites: Rust 1.96.0 (installed by rustup from `rust-toolchain.toml`), Node.js
24.13+ (`nvm use` reads `.nvmrc`), npm, Bash, ripgrep, jq, and Docker for disposable PostgreSQL
integration tests. Dependency versions are captured in `Cargo.lock` and
`web/package-lock.json`. The check scripts target Linux development/CI hosts.

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

The second command **currently fails intentionally** because the four M2–M5 milestone
acceptance scenarios are pending; the M1 browser scenario is executable. These gates must be expanded into the detailed
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
None claims payload storage is ready. The configured service implements the M1
[operation map](contracts/operations.md); the frontend-only scaffold keeps returning
structured 404 responses for application routes.

Useful focused checks:

```sh
cargo test --locked --workspace
./scripts/test-integration
./scripts/check-client
npm --prefix web run test:e2e   # disposable PostgreSQL + real server/Chromium; requires builds
```

For formatting, run `cargo fmt --all` and `npm --prefix web run format`.

## Component boundaries

| Component | Responsibility |
| --- | --- |
| `sct-protocol` | Public Rust-owned contracts with generated OpenAPI, JSON Schema and TypeScript |
| `sct-client` | Standalone authenticated foundation API client; transfer/chunking follows in M2; depends on protocol, never core/server |
| `sct-core` | Authorization, catalog, roots, policies, namespace and operation fencing; payload coordination follows in M2 |
| `sct-server` | Axum routes, Tokio runtime, configuration and worker wiring |
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

Next: refine and implement M2 immutable transfer and real storage adapters, resolving
its capacity-ledger, immutable-format and crash-recovery design prerequisites.
M2–M5 still need implementation-ticket decomposition beyond their existing epics.

Technical references: [Axum serving](https://docs.rs/axum/0.8.9/axum/fn.serve.html),
[SQLx PostgreSQL wiring](https://docs.rs/crate/sqlx/0.8.6/source/README.md),
and [Vite development/build setup](https://vite.dev/guide/).

Lifecycle migration and sibling-library checkout instructions are in
[Step 02](docs/step-02-lifecycle.md). The migration remains on its isolated branch.
