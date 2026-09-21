# Step 04a: extractor body limits

Storage-backed application router retains its explicit 65,536-byte extractor limit. Existing problem response, correlation, auth, content-type and observability behavior remains local. No-storage router has no explicit limit to migrate.

Uses optional simple-server body-limit feature at reviewed revision
`0b945750b6b97a9e18c531d1cf1137d4ed4b69c9`. Raw readers and custom domain limits
are not replaced. Shared differential tests qualify JSON/bytes/multipart
rejections, unknown lengths, route overrides and lazy response pass-through.

## Verification

Baseline and final sct-server suite: five passed, eleven existing PostgreSQL/S3 integration tests ignored. No database-backed production HTTP qualification is claimed.

Primary command: `cargo test --locked -p sct-server` in the affected package/workspace. Runs use
two jobs, `/tmp/sct-03c-target` and dev/test debug=0. Lockfiles add only the
shared library's tower-layer dependency. Strict all-target/all-feature server Clippy passes. Changed-file rustfmt and
whitespace checks pass; repository-wide formatting is not claimed.

Database-backed HTTP, PostgreSQL/S3, browser and container qualification were not repeated. Active original qualification work/staging and pre-existing worktrees/stash are preserved.

## Integration

Started from active master `3343fbed`
in a dedicated `migration/step04a-body-limits` branch and sibling worktree.
Commit there, rebase the original development branch onto the migration, verify
ancestry/tested tree and preserve unrelated work before removing the temporary
worktree/branch. Exact integration revision and any concurrent changes are
recorded in the central simple-server trackers. No push or deployment.
