# Step 03b: shared request correlation

ScT's storage-backed `application()` uses simple-server's optional correlation
scope and response propagation, pinned in `simple-server.rev` to
`2740d5c81184c381d3178aa9688dfcb675ce2308`. The adapter selects a fresh UUID v4
encoded as 32 lowercase hexadecimal characters; caller headers are never trusted.
Application errors, response normalization, and structured request logs read the
same scoped ID. Error messages, retry guidance, cache/security headers, metrics,
and streaming bodies remain application-owned.

The no-storage `router()` keeps its original narrower behavior: API fallback
errors receive generated IDs, while health and static responses have no ID
header. Error conversion outside a request scope also keeps a local UUID fallback.
The selected-ID API retains UUID variant/version bits and does not replace the
existing format with the library's default random hexadecimal generator.

Verification on the isolated migration worktree:

- Baseline workspace all-feature tests passed, with database/S3 qualifications
  explicitly ignored by their existing configuration.
- Baseline disposable-PostgreSQL foundation and observability HTTP tests passed.
- Final server tests retain body/header equality, untrusted-input rejection,
  UUID v4 formatting, and no-storage health/static header absence.
- Full workspace tests, formatting, and strict Clippy were checked; final results
  and integration commit are recorded in simple-server's central migration tracker.
- Browser, S3, transfer-scale, and complete Docker qualification were not rerun.

Work started from local `master` at `117734c30dad697548d81a47913fecd43bf76142`
in a dedicated `migration/step03b-correlation` worktree. Integration rebases master
onto the tested migration branch and removes that worktree/branch. Existing
untracked M4 recovery work and the earlier step-01 worktree are unrelated.
No deployment or push is part of this change.
