# Foundation implementation

Scope: complete SCT-8, SCT-9, SCT-10, SCT-11, SCT-13, SCT-12 in dependency order. M2–M5 remain out of scope.

Each ticket receives a scoped commit with its Crumbles link and relevant verification. Acceptance is tracked here and in focused PostgreSQL, HTTP, and browser tests.

- [x] SCT-8: forward migrations, constrained catalog, isolated database tests, extension points.
- [x] SCT-9: validated private configuration, adapter registry, exclusive fenced server lease, readiness/shutdown.
- [x] SCT-10: issuer/subject OIDC, secure sessions/CSRF, hashed revocable tokens, reusable authorization, local recovery.
- [x] SCT-11: authorized root/grant provisioning, placement validation, pool allocations, root operation ownership.
- [x] SCT-13: effective policies, administrator bounds, atomic scopes and transition guards.
- [x] SCT-12: namespace operations, bounded authenticated pagination, identity/revision/policy-safe rename.
- [x] Final: generated contracts, SDK, executable M1 acceptance, full development checks and documentation.

Design choices and test results will be recorded as implementation proceeds.

SCT-8 verification: real PostgreSQL tests passed for repeat migration, checksum mismatch rejection, rollback, principal/root foreign keys, case-sensitive unique names, cross-root parent rejection, and the 10,000 = 5,000 + 4,000 + 1,000 allocation example. Each integration case creates an isolated schema in the disposable database.


## Final foundation result

- SCT-9: typed configuration and registered test adapters; exclusive 30-second server lease with 10-second renewal; transaction fencing, readiness, graceful shutdown and crash recovery. Real process tests reject a second writer and verify local recovery while OIDC is unavailable.
- SCT-10: issuer/subject identity, PKCE plus browser-bound one-use login state, signed OIDC verification, hashed/revocable credentials, secure production session cookies and Origin/CSRF checks. Core authorization is reused by HTTP and local administration. A signed mock provider tests invalid issuer, audience, expiry, nonce, signature and access-token hash.
- SCT-11: disabled-by-default roots, explicit enablement, owner/grant authority, revision checks, destination/pool distinction, same-pool mirror charging, bounded allocation inspection, durable operation ownership, recovery fencing and unused staging reservations. Concurrent cross-user allocation attempts cannot overcommit a pool.
- SCT-13: inherited policies, strict keep-one defaults, administrator bounds, metadata-only revision-checked policy edits, atomic scope restrictions and transition exclusion. Latest-content protection is independent of hidden namespace state. Content transitions remain M3.
- SCT-12: authorized resolution, metadata, directory creation, bounded keyset listing and stable-ID rename. Tests cover Unicode/case sensitivity, cross-root parents/moves, cycles/collisions, stale and concurrent revisions, cursor isolation/invalidation, policy boundaries and disabled roots.
- Integrated foundation: authenticated Rust SDK, generated OpenAPI/schema/TypeScript, fixed error envelopes (including framework rejections), configured HTTP runtime/local CLI, disposable PostgreSQL browser fixtures and executable M1 acceptance.

Verification: `./scripts/check` passed (formatting, Clippy, Rust tests/builds,
contract drift and 49 TypeScript fixtures, web checks/build, independent SDK
packaging, PostgreSQL/HTTP/OIDC/process integration, and Chromium). Final additional
concurrency/isolation regressions also passed: 11 database-backed tests in total.
The browser suite has three passing tests including the real M1 workflow.
`npm --prefix web run test:acceptance` exits 1 intentionally: M1 passes and the
four M2–M5 scenarios remain pending. No later milestone gate was removed.

The six implementation tickets have scoped local commits with ticket links.
Repository documentation is updated; remote ticket descriptions/statuses and
repository hosting were not modified. No deployment or push was performed.

Operational limits and M1/M2 boundaries are documented in `docs/foundation.md`.
The remaining readiness decisions in `docs/implementation-readiness.md` still apply
to their dependent milestones.
