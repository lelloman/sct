# Service observability

`GET /health/live` is process liveness. `GET /health/ready` requires a valid writer
lease and an available catalog write gate; an expired writer or checkpoint capture
fence returns 503. Database and readiness probes have a two-second deadline.
`GET /health/database` checks catalog connectivity only. A single unavailable
storage destination does not take otherwise usable roots out of service.

`GET /metrics` exposes Prometheus text format to administrators, using the normal
Bearer credential or browser-session authentication. Configure the monitoring
system's authorization secret separately; do not put credentials in a URL. Scrapes
have a five-second aggregate-query deadline, return `Cache-Control: no-store`, and
remain inspectable during checkpoint capture. Failed authentication returns the
same structured errors as the API.

Metrics include HTTP responses and response-header latency histograms, active
requests, roots, operations, maintenance jobs, checkpoints, observed backend
health, backup freshness, and aggregate allocation capacity/reserved/occupied/
pending-cleanup bytes. Pending cleanup is a subset of occupied bytes; do not add
it again. State labels have fixed enumerated values. HTTP labels use matched
route templates, normalized methods and status classes, never raw paths, query
strings, account/root IDs, object digests or credential values. Response latency
ends when headers are produced; it is not whole-download duration. HTTP counters
reset on process restart; catalog gauges survive it.

Request logs are JSON lines with a server-generated correlation ID, route template,
method, status, duration and structured error code when available. The same ID is
returned in `x-request-id` and structured error responses. Incoming request IDs
are not trusted. Bodies, query strings, authentication headers, provider settings
and resolved secrets are omitted. Runtime events identify startup, shutdown,
lease loss, worker heartbeat failure and recovery retries without provider error
bodies. Local operator commands still emit their documented result JSON, including
one-time credentials when explicitly issuing them.

An optional expected backup interval belongs to each root. Root management
inspection reports `not_configured`, `never_committed`, `on_time` or `overdue`,
the last successful publication time and the next expected time. Only the atomic
publication of a committed file or directory snapshot refreshes it. Planning,
admission, payload writes, failed/aborted sessions, and replaying a prior commit
receipt do not. The browser displays this status on root pages. Migration 18
backfills historical successful publications from their durable audit records.
This measures backup freshness; programs remain responsible for scheduling,
consistent source capture and application restore.
