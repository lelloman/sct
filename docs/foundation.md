# Foundation service (M1)

SCT-8–13 implement the database-backed foundation. Payload transfers, real local/S3
adapters, content history, destructive policy jobs, deletion/purge, and the complete
administration UI remain M2–M5. The landing page is still a development page.

## Starting a development installation

Use a dedicated PostgreSQL 17+ database. Build the web assets and Rust server, set
`SCT_CONFIG` to `config.example.toml`, and set `SCT_DATABASE_URL` privately. The file
contains an explicitly enabled test adapter: it validates metadata configuration
and reports test connectivity, but cannot store bytes. Production configuration
must use HTTPS and cannot enable this adapter. No production payload adapter is
provided by M1.

With the server stopped, run:

```sh
export SCT_CONFIG="$PWD/config.example.toml"
# Set SCT_DATABASE_URL through your private environment/secrets mechanism.
npm --prefix web run build
cargo build --locked --workspace
target/debug/sct-server bootstrap-admin
```

The command prints one JSON object containing a credential ID and an opaque token.
Keep it private. `recover-admin` has the same effect: all previous local-admin
credentials are revoked and a replacement is printed. Both commands work without
contacting OIDC and refuse to run while another writer holds the lease.

Additional local commands use `SCT_ADMIN_TOKEN` and also require a stopped server:

```sh
target/debug/sct-server create-principal alice
target/debug/sct-server issue-token alice
target/debug/sct-server revoke-token CREDENTIAL_ID
target/debug/sct-server
```

OIDC identities are created on first successful login and keyed by exact issuer
and subject. They start without roots or administrator access. No email matching
or automatic administrator promotion occurs. Configure the callback URL as
`PUBLIC_ORIGIN/auth/callback` with the provider. Discovery occurs at startup;
restart after provider endpoint/signing-key configuration changes.

Without `SCT_CONFIG`, the server runs the frontend scaffold only and advertises no
application operations. A database URL without a configuration file is rejected.

## Configuration and authority

Unknown TOML fields, unsafe URLs, invalid worker/expiry budgets, duplicate pools,
destination aliases, unregistered adapters, and impossible capacity allocations
fail startup with fixed diagnostics. Only `SCT_BIND`, `SCT_WEB_DIR`, and
`SCT_PUBLIC_URL` override file values. The configured environment-variable names
supply database and OIDC client secrets; their values are never included in public
inspection models. `public_url` must be an origin without a path prefix.

Pool capacity is reduced by `overhead_bytes` before allocations are admitted.
Each pool has one server staging allocation. Root allocations plus staging cannot
exceed this capacity. Occupied and reserved bytes constrain allocation resize;
pending cleanup is a subset of occupied bytes. The M1 test-reservation primitive
moves only unused reservations, never claims to recover physical bytes.

Backend kind, destination identity, and pool membership are distinct. Destination
identity/pool reassignment through configuration is rejected; credential rotation
for the same destination is compatible with the registry interface. Referenced
backends cannot disappear on restart. A mirror has exactly two distinct instances,
requires explicit read order, and charges both copies even in a shared pool.

`workers` and `session_expiry_seconds` reserve validated settings for M2 transfer
workers/upload sessions. Browser sessions currently have a fixed 12-hour lifetime;
OIDC login attempts expire after 10 minutes and listing cursors after one hour.

## Authentication and API

The generated [operation map](../contracts/operations.md) and OpenAPI document list
implemented v1 routes. `GET /api/v1/capabilities` requires authentication and exposes
only those operations; supported storage formats and chunk profiles remain empty.

Application tokens use `Authorization: Bearer TOKEN`. Their authority comes from
the associated principal's current root grants. Tokens are independently revocable
and contain 244 bits of cryptographically generated randomness.
Only SHA-256 hashes are stored. Revocation/disabled-principal checks happen for each
request and again inside core operations; no cached authorization outlives a grant.

Browser login starts at `GET /auth/login`. Authorization-code exchange uses PKCE,
a one-use browser-bound state, nonce, issuer/audience/signature/expiry validation,
and access-token-hash verification when supplied. HTTPS sessions use
`__Host-sct_session` with Secure, HttpOnly, Path=/ and SameSite=Lax. Development HTTP
uses an unprefixed cookie only on loopback. Cookie-authenticated mutations require
the exact public origin and `X-CSRF-Token` from the corresponding `sct_csrf` cookie
(`__Host-sct_csrf` on HTTPS). Bearer tokens do not use browser CSRF checks.

- `GET /auth/session`: current principal ID.
- `POST /auth/tokens`: issue a token for the current principal, returned once.
- `DELETE /auth/tokens/{credential}`: revoke an owned credential (administrators may revoke any).
- `POST /auth/logout`: revoke the current browser session and clear its cookies.

All application responses use no-store and server-generated request IDs. Errors
have fixed safe messages; retryable busy/unavailable errors include Retry-After.
JSON bodies are limited to 64 KiB. Generated GET query models are flat; directory
listing takes `directory_id`, `limit`, optional `cursor` plus
`directory_revision`, and `include_hidden`. Root and grant listing use keyset
pagination. Server-held random cursors bind the principal, scope, revision (for
directories), filters and page size; changed directory revisions require refresh.

## Domain behavior and concurrency

Administrators create disabled roots and explicitly enable them. Namespace writes
require an enabled root. Mounts cannot overlap for one owner; separate owners can
use the same logical mount. Root owners have all root capabilities; delegated
grants keep list/read, write, policy management, ordinary delete, purge, and grant
management separate. Delegation cannot exceed the grantor's own capabilities.

Namespace operations preserve case-sensitive Unicode names and stable entry IDs,
reject cross-root parents/moves and cycles, and compare entry/parent revisions.
A rename across effective policy scopes or atomic scopes is rejected, including
moves between equal-valued but independently managed policy scopes. The foundation
has no hidden-entry mutation or content publication endpoint.

Policies default to strict keep-one. The nearest explicit policy applies, with file
overrides supported by the catalog/core model. Atomic directories have a single
policy and do not permit independent descendant policies or ordinary namespace
writes; populated mode changes and nested atomic scopes are rejected. M1 policy
editing is metadata-only and rejected once the root contains published content.
M3 will provide destructive previews and resumable content transitions. Increasing
a count changes the requirement, never manufactures historical versions.
Administrator bounds are set using the revision-checked policy-constraints endpoint.

The server lease lasts 30 seconds and renews every 10 seconds. Every authoritative
transaction checks owner/epoch/expiry while holding a shared lease-row lock;
takeover waits for these short transactions. Lease loss stops new mutations and
initiates shutdown. Graceful shutdown expires the lease; crashes require expiry.
`/health/live` is process liveness, `/health/database` checks a database query, and
`/health/ready` requires a valid writer lease. None promises payload availability.

Short metadata mutations serialize on a root row and commit atomically; they check
for an existing durable owner before changing anything. Operations spanning work
hold a durable root owner with a separate worker epoch/lease. Worker or server
expiry does not free the root. Recovery claims the same operation with a new epoch;
stale worker completion fails. Reservations and uncertain I/O prevent terminal
ownership release. Reads and operations on unrelated roots remain available.
M2 must extend these primitives with physical-incarnation records and explicit I/O
phase reconciliation before introducing backend work.

## Verification

`./scripts/check` runs formatting, Clippy, Rust tests/builds, contract drift and
cross-language fixtures, web checks/builds, independently packaged SDK compilation,
real PostgreSQL/HTTP/OIDC/process tests, and Chromium acceptance. Test databases are
disposable Docker containers by default. `SCT_TEST_DATABASE_URL` must point only to
a dedicated test database; Rust integration cases create isolated schemas there.
The browser fixture removes its schema after execution.

The M1 browser scenario provisions/enables a root, delegates/revokes a grant,
creates/lists/renames a directory, checks policy revisions and authorization, and
rejects traversal. HTTP tests additionally run a signed mock OIDC provider, invalid
claim cases, CSRF, revocation, safe error envelopes and SDK calls. Process tests
cover writer exclusion, SIGTERM release, crash fencing and local recovery with
an unavailable identity provider. M2–M5 remain pending and the full-product
`npm --prefix web run test:acceptance` command must continue to fail until they land.
