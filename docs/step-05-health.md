# Step 05 health and readiness

The production SCT server adopts `simple-server` health at reviewed revision
`ed245d2d46e9d29aeee7be5202f3a8b8113c9caf`. `/health/live` uses the shared
liveness endpoint; `/health/database` keeps its two-second database ping;
and `/health/ready` retains the writer readiness condition. Exact empty/text
bodies, 200/204/503 statuses, headers, unauthenticated access, middleware, and
GET/HEAD/405 behavior remain application-owned.

The `sct-client` only consumes these endpoints and serves none itself. Other
workspace binaries expose no production health routes.

The baseline live-server HTTP test compiled but the sandbox denied its local
socket bind (`Operation not permitted`). After migration,
`cargo check --locked -p sct-server` passed and the same HTTP test passed 1/1
when local socket binding was allowed. Changed-file Rust formatting passed.
