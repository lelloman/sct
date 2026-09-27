# Step 03c: HTTP tracing

Reviewed shared library: `adc1640bde4ac8f934ed454c8d6c5e264a6a2790`.
Migration starts from master `2d1a921` in `sct-step03c` on
`migration/step03c-http-tracing`.

Storage-backed `application()` uses `http_tracing::trace_with_observer` inside
its existing observe middleware. SCT implements the observer to retain the
unconditional single stderr JSON `http_response` event: event, server-generated
request_id, approved method, bounded router template, status, integer duration_ms,
and typed ErrorCode. The shared callback supplies header latency, while existing
metrics retain their route/method/status-class labels, buckets and header-time
active-request boundary. Response normalization happens inside the traced callback.
No global subscriber, default duplicate events, new body-completion logs or
per-client identifiers are introduced. The shared span/body lifecycle is installed
but observer completion callbacks remain silent, matching SCT's logging contract.

The no-storage `router()` remains untraced. Correlation generation and the separate
domain/startup/worker/CLI stderr and stdout events remain unchanged. `simple-server.rev`
is the authoritative build pin; tracing is a type-only dependency for the observer
signature, not a subscriber initialization.

The active service was subsequently migrated to the backend-independent owned
tracing API at reviewed shared-library revision
`ca98a4159e1cb0dd7b9db2faa9a076d198b7973b`. Production now calls
`web::tracing::trace_with_observer`; `HttpObserver` consumes read-only
`web::tracing::ResponseInfo`, including the application `ErrorCode` extension.
The obsolete `web-compat` feature was removed because the workspace has no other
compatibility API use. The event schema, request ID, bounded labels, histogram,
status class and header-time active gauge remain unchanged.

At that revision, the focused observability contract tests pass 3/3, including
the real loopback middleware test with no subscriber and an unfinished body.
The full locked workspace suite passes 54 tests with the same 91 PostgreSQL,
S3 and extended-qualification tests intentionally ignored as at baseline.
Workspace/all-feature/all-target strict Clippy, formatting and build checks pass;
the ignored external-service qualification was not rerun.

Baseline workspace/all-feature test run:31 passed,65 existing PostgreSQL/S3 ignores.
Final:33 passed,65 unchanged ignores. New tests verify exact JSON fields/error code
and integer header timing, then exercise the actual observe+correlation middleware
over real loopback HTTP with no subscriber: an unfinished streaming response has
already incremented header metrics and released the active gauge; supplied IDs are
ignored and 405 is counted. Workspace all-feature/all-target strict Clippy
(`-D warnings`), full fmt and diff checks pass. Database-backed production router
qualification, S3, browser tests and container process recovery were not rerun.
Concurrent user qualification fixtures and original worktree changes were not used
or modified by these tests.

Integrate by rebasing master onto the migration in a clean linked worktree,
preserving unrelated work, verify ancestry/tree/WIP, then remove the temporary
branch/worktree. Final integration evidence is recorded in the central tracker.
