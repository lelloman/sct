# M4 qualification

Approved scope: SCT-42–61, twenty tasks covering management contracts/schema,
account and storage APIs, durable maintenance, integrity/repair, checkpoint export
and isolated recovery, browser management, observability and the standalone SDK.
M5 scale and release qualification remains pending.

## Reproduce

Run `./scripts/check` for the complete required regression suite. It includes Rust
formatting/Clippy/unit and HTTP tests, generated contracts, frontend checks/build,
standalone packaged SDK compilation, PostgreSQL integrations, real MinIO adapter
conformance and Chromium acceptance against the actual Rust service.

Run `./scripts/test-m4` for the focused independent recovery drill. It starts a
disposable PostgreSQL 17 container unless `SCT_TEST_DATABASE_URL` points to a
dedicated test database. Browser M4 acceptance runs the operations workflow and
this drill; `./scripts/test-e2e --grep 'M4:'` selects that gate. The full acceptance
command still fails intentionally on the single pending M5 scenario.

## Evidence and boundaries

Browser scenarios exercise cookie/CSRF sessions, scoped navigation, own/admin
credentials, disabled principals, delegated root limits/grants, acknowledged
policy retirement, retained snapshot browsing/downloads, pool/backend/root
provisioning, allocation resizing, rejected capacity shrink, read-policy editing,
adapter documentation, configured expectations, activity, integrity scans and
checkpoint artifact downloads/release. Program fixtures produce content through
the API. Browser controls never offer backup content mutations.

PostgreSQL tests cover revision/permission guards, durable maintenance claims and
restart/retry fencing, corruption/missing/inaccessible findings, verified alternate
replica repair, conservative capacity accounting, checkpoint barriers and pins,
failed/restarted export handling, artifact integrity and isolated catalog recovery.
Existing local/S3/mirror transfer and lifecycle fault suites remain mandatory.
HTTP and SDK tests cover structured errors, management workflows, verified
checkpoint streams, bounded metric labels, readiness and successful-commit
freshness. SDK packaging is checked outside the workspace.

The final recovery drill creates independent file and atomic-tree backups via the
Rust SDK against a real HTTP service. It also leaves an unfinished upload, exports
and verifies the catalog artifacts, and invokes the actual operator CLI to copy
payloads into an independent directory. It stops the source server and **removes
the original payload directory** before verification and recovery. The CLI rejects
missing isolated-recovery acknowledgment, restores a separate empty catalog and
rewrites storage paths. The recovered service rejects old credentials, accepts a
new recovery credential, serves byte-exact file/tree downloads and expires the
unfinished upload without publishing its path. CLI execution uses a nonexistent
production config to verify recovery does not depend on loading that config.

This is a functional recovery drill on small local fixtures, not a claim of
production recovery time or large-archive throughput. Independent physical failure
domains, operational backup scheduling and application source consistency remain
operator/consumer responsibilities. M5 must qualify broader scale and release
readiness before the complete product acceptance gate can pass.

Operational instructions: [browser management](browser-management.md),
[checkpoint recovery](checkpoint-recovery.md), [observability](observability.md),
[SDK integration](rust-sdk.md) and [M4 contracts](m4-contracts.md).

## Final results

`./scripts/check` exited 0: all required Rust, contract, frontend, packaged SDK,
PostgreSQL and real MinIO checks passed. Chromium reported **12 passed, 1 skipped**
(the deliberately pending M5 gate), including active M2/M3/M4 acceptance. The
isolated recovery test passed both in the integration suite and through the M4
browser gate. The separate focused M4/storage-admin run passed both cases.
