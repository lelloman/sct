# ScT — Stipa ca Truova

Self-hosted versioned storage. Implementation has started with the SCT-6 development
scaffold. Storage operations, authentication, retention and administration are not
implemented yet; all five milestones remain required.

## Development

Prerequisites: Rust 1.96.0 (installed by rustup from `rust-toolchain.toml`), Node.js
24.13+ (`nvm use` reads `.nvmrc`), npm, Bash, ripgrep, and Docker for disposable PostgreSQL
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

The second command **currently fails intentionally** because the five milestone
acceptance scenarios are pending. These gates must be expanded into the detailed
workflows in [the acceptance matrix](ARCHITECTURE_PLAN.md#16-test-and-acceptance-matrix),
not simply removed to obtain a green result.

Run the scaffold:

```sh
npm --prefix web run build
cargo run --locked -p sct-server
```

Open <http://127.0.0.1:8080>. The server serves `web/dist` from the same origin.
For web hot reload, run `npm --prefix web run dev` alongside the Rust server;
Vite proxies `/api` and `/health` to port 8080.

| Environment variable | Scaffold behavior |
| --- | --- |
| `SCT_BIND` | Socket address; defaults to `127.0.0.1:8080` |
| `SCT_WEB_DIR` | Built assets directory relative to the process working directory; defaults to `web/dist` |
| `SCT_DATABASE_URL` | Optional PostgreSQL URL; absent means no database connection |
| `SCT_TEST_DATABASE_URL` | Optional **dedicated test database** for `scripts/test-integration`; otherwise the script owns and removes a disposable Docker container |
| `SCT_E2E_PORT` | Browser test server port; defaults to `18080` |

`GET /health/live` checks process liveness. `GET /health/database` returns 204
after a successful PostgreSQL query or 503 if unconfigured/unavailable. Neither
claims that storage is ready. `/api/v1/*` operations are not implemented. Full
validated TOML configuration, migrations and the single-writer lease follow in
SCT-8/SCT-9; this scaffold is not a production storage service.

Useful focused checks:

```sh
cargo test --locked --workspace
./scripts/test-integration
./scripts/check-client
npm --prefix web run test:e2e   # requires built server and web assets
```

For formatting, run `cargo fmt --all` and `npm --prefix web run format`.

## Component boundaries

| Component | Responsibility |
| --- | --- |
| `sct-protocol` | Public Rust-owned contracts; OpenAPI/TypeScript generation begins in SCT-7 |
| `sct-client` | Standalone HTTP transport and later transfer/chunking operations; depends on protocol, never core/server |
| `sct-core` | Application rules and catalog; later backend registration, byte adapters and a separate placement coordinator |
| `sct-server` | Axum routes, Tokio runtime, configuration and worker wiring |
| `web` | Vue 3/TypeScript UI; generated wire models will come from Rust contracts |

The external sample is a template copied to a temporary directory by
`scripts/check-client`. It builds against packaged client/protocol crates with no
server/core checkout. It is not part of the root workspace. No other application
repository is needed.

## Plan and next steps

- [Architecture and product contract](ARCHITECTURE_PLAN.md)
- [Ticket plan and amendments](TICKET_PLAN.md)
- [Implementation prerequisites from the readiness review](docs/implementation-readiness.md)

Next: SCT-7 protocol foundations, followed by catalog/configuration/authentication,
root provisioning, policies and namespace according to the recorded dependency graph.

Technical references: [Axum serving](https://docs.rs/axum/0.8.9/axum/fn.serve.html),
[SQLx PostgreSQL wiring](https://docs.rs/crate/sqlx/0.8.6/source/README.md),
and [Vite development/build setup](https://vite.dev/guide/).
