# Step 04b: response header policies

Reviewed shared source: `b88b908421db2552ff1e81966c56958925741e27`.
The optional response-headers feature is enabled only in the affected HTTP
package(s). Production calls use the shared header operations, preserving
existing values, placement, status/body/extension behavior and application policy.
No new global defaults or response buffering are introduced.

The storage-backed API response policy and ApiError use shared replacement for no-store, nosniff and no-referrer. Correlation, retry guidance, framework-error normalization, auth, status and body construction remain local. No-storage fallback and metrics response tuples remain local response construction; no new global policy is installed.

## Verification

Baseline and final: **6 passed, 0 failed, 15 ignored** across the
selected suites. A new real-HTTP regression exercises the production policy without storage: GET/HEAD, existing cache/referrer override, cookies, body/status, generated correlation and normalized early errors. It passes before and after migration alongside the server suite.

Strict all-target Clippy passes. Changed-file formatting and whitespace checks pass. The lockfile
changes add only the shared crate's direct http dependency, with no package
version changes. Applicable CI/readme/source pins reference the reviewed source.

Final test commands (baseline used --locked before the dependency-edge update;
final tests used --offline, then lint verified --locked):

- In `.`: `cargo test --offline -p sct-server -q -j 2 --target-dir /tmp/sct-03c-target --config profile.dev.debug=0 --config profile.test.debug=0`

Lint commands:

- In `.`: `cargo clippy --locked -p sct-server --all-targets -j 2 --target-dir /tmp/sct-03c-target --config profile.dev.debug=0 -- -D warnings`

Fifteen existing PostgreSQL/S3 qualification tests remain ignored. Database-backed production HTTP, browser and destructive container qualifications were not rerun.

## Integration

Started from active master `d3961d4c` in dedicated branch
`migration/step04b-response-headers` and a sibling worktree. Integration follows
the shared workflow: commit there, rebase the original development branch onto
the migration, verify ancestry/tested tree and preserve concurrent work, then
remove only the temporary migration worktree/branch. The central simple-server
trackers record the final commit, integration outcome and any concurrent changes.
No push or deployment.
