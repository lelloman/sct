# Foundation implementation

Scope: complete SCT-8, SCT-9, SCT-10, SCT-11, SCT-13, SCT-12 in dependency order. M2–M5 remain out of scope.

Each ticket receives a scoped commit with its Crumbles link and relevant verification. Acceptance is tracked here and in focused PostgreSQL, HTTP, and browser tests.

- [ ] SCT-8: forward migrations, constrained catalog, isolated database tests, extension points.
- [ ] SCT-9: validated private configuration, adapter registry, exclusive fenced server lease, readiness/shutdown.
- [ ] SCT-10: issuer/subject OIDC, secure sessions/CSRF, hashed revocable tokens, reusable authorization, local recovery.
- [ ] SCT-11: authorized root/grant provisioning, placement validation, pool allocations, root operation ownership.
- [ ] SCT-13: effective policies, administrator bounds, atomic scopes and transition guards.
- [ ] SCT-12: namespace operations, bounded authenticated pagination, identity/revision/policy-safe rename.
- [ ] Final: generated contracts, SDK, executable M1 acceptance, full development checks and documentation.

Design choices and test results will be recorded as implementation proceeds.

SCT-8 verification: real PostgreSQL tests passed for repeat migration, checksum mismatch rejection, rollback, principal/root foreign keys, case-sensitive unique names, cross-root parent rejection, and the 10,000 = 5,000 + 4,000 + 1,000 allocation example. Each integration case creates an isolated schema in the disposable database.
