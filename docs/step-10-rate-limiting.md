# Step 10: rate limiting and admission assessment

Status: N/A for SCT's current request policy. SCT has no configured request-rate
budget, failure cooldown, or HTTP admission layer to migrate to
`simple_server::rate_limit`. Adding one would introduce a new policy.

Reviewed SCT `master` at `7a0c6bd5ad77f6f3c12bc18aaf8afeb9b53c357f` and
shared `simple-server` source at `15ad34b5ee7c2f36720a13467984ff63c042b326`.
The production HTTP entry point is `sct-server`; `sct-recover` is an offline
archive utility. The client and protocol crates do not install server policy.
The server config, startup, router, core admission paths, and active Cargo
features contain no per-request rate or failure-attempt budget.

Two core paths return `ErrorCode::RateLimited`, but each protects durable state
cardinality rather than counting requests over time:

- `sct-core/src/auth.rs::save_oidc_flow` deletes expired OIDC flows and refuses
  insertion when 10,000 live rows already exist globally. The flow is stored in
  PostgreSQL and later consumed by matching state and browser hashes.
- `sct-core/src/paging.rs::save` deletes expired cursors and refuses insertion
  when a principal has 10,000 live cursors. The tokens bind query parameters and
  survive server restarts.

Both failures map through SCT's existing 429 response, structured error body,
and one-second `Retry-After`. That response mapping alone does not make either
database row cap a Step 10 request quota. A process-local limiter would not
preserve the durable row invariant or the expiry and consumption semantics.

Other uses of "admission" are also outside this module: upload admission
reserves retained and staging storage in PostgreSQL; `workers` controls a
waiting payload-I/O semaphore and background execution; archive `max_active`
limits durable account records. Replacing those with request-rate checks would
change their ownership and behavior. SCT has no Step 10 production API call,
and this assessment makes no dependency, configuration, or runtime change.

Verification: searched production crates, HTTP routes, startup/configuration,
Cargo features, and the offline archive binary for rate, retry, concurrency,
and admission policies. The pre-edit worktree was clean and `git diff --check`
passed. This documentation-only assessment was validated with `git diff --check`;
no application build or runtime test was needed because no executable code
changed.
