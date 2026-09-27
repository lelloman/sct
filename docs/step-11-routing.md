# Step 11: shared routing and HTTP core

Active branch: `master`, baseline `3b1144f`.
Reviewed shared source: `ca98a4159e1cb0dd7b9db2faa9a076d198b7973b`, pinned
in `simple-server.rev` for the existing reproducible checkout workflow.
Unrelated original untracked `.validation-work/` and `docs/step-04c-cors.md`
are preserved and excluded from this migration.

All SCT server route groups, ordinary handlers, custom authentication extraction,
request/response/body interfaces, middleware, main serving modes, HTTP test/mock
fixtures and the HTTP qualification example now use `simple_server::web`.
Static-file fallback, shared health endpoints, correlation, auth/cookies/CSRF,
response headers/body limits, worker ownership and transfer policies retain
application behavior.

JSON/query failures remain explicitly captured and mapped to SCT InvalidRequest
errors. ApiError renders one shared rejection envelope for extraction and ordinary
responses, preserving exact JSON bytes, UUID request ID/header, cache-control,
retry-after and ErrorCode response extensions used by telemetry. Shared extraction
supports standard HTTP Method for the archive ISO GET/HEAD handler; it preserves
HEAD and custom methods, and sees changes made by earlier head extractors.
Streamed ISO/checkpoint/transfer bodies and range/HEAD policies remain unchanged.

The tracing observer now receives backend-independent `web::tracing::ResponseInfo`
through `web::tracing::trace_with_observer`. There is no protocol adapter, backend
response type or backend router conversion in SCT's routing and observability code.

## Verification

- Baseline workspace: 55 passed, 91 explicitly ignored; final: 56 passed,
  same ignores. New envelope coverage verifies bytes/headers/retry metadata and
  ErrorCode extensions for both extraction rejection and handler response paths.
- Baseline/final dedicated PostgreSQL integration: 83 passed each. These run
  ignored database tests using a disposable container and isolated fixtures,
  including auth/OIDC/CSRF, safe errors, storage, trees, lifecycle, archives,
  checkpoint/recovery, observability, SDK and real process behavior.
- Shared all-feature suite: 261 passed. New contracts compare method extraction
  to the backend for GET, HEAD, POST/custom methods and verify extraction order.
- Consumer and shared strict all-target/all-feature Clippy and formatting pass.
  Workspace debug build and generated contract drift checks pass.
- Real-process Python lifecycle suite passes SIGINT/SIGTERM HTTP draining of an
  8 MiB streamed static file, writer lease release/restart, and lease loss with
  a nonzero process exit. Its PostgreSQL/static fixtures are disposable.
- Live S3/MinIO, heavy scale qualification, frontend/Chromium and release builds
  were not run. The repository's pinned MinIO image is unavailable locally.
  Existing test skip/qualification boundaries are retained; no production
  database or external identity provider was used.

Work was committed in isolated branches, integrated by rebasing development
branches onto them, then verified by ancestry/tree equality. Temporary worktrees,
branches, disposable containers, build outputs and scratch files were removed
following integration. No push or deployment is part of this work.
