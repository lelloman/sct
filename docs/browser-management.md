# Browser management

ScT stores backups produced by programs. Sign in using the configured identity
provider or exchange a program/recovery credential for a browser session. The
browser session uses an HttpOnly cookie and CSRF protection. Sign out when finished.

All users can inspect their account and issue or revoke their own credentials.
New credential secrets appear once; save them before dismissing the result.
Administrators can also create/disable principals, change roles and manage their
credentials from Accounts. Disabling a principal prevents authentication.

Open an assigned root to browse current entries, retained versions and atomic
snapshot descendants, then download the selected content. Hidden entries can be
included explicitly. The browser has no upload, edit, rename, hide, purge or file
deletion controls. Those operations belong to authorized API/SDK programs.

Root management tabs follow effective grants. Root managers can change permitted
settings and quotas within the administrator's ceiling. Policy managers preview
retention changes, inspect affected versions and acknowledge retirement before
applying. Large previews/jobs advance in bounded batches; continue or resume the
displayed operation when requested. Grant managers can delegate only capabilities
they hold. Revision conflicts require refreshing before submitting again.

Administrators use Storage to configure pools, backend destinations and explicit
root allocations, provision roots and set quota ceilings and policy bounds. A
new root starts disabled. Configure its allocations before enabling it. Backend
documentation describes acknowledgment, consistency and durability semantics;
configured performance expectations are shown separately from observed health.
Placement settings control ordered read candidates, timeouts and throughput
limits. Changing destinations requires an unpopulated root and valid allocations.
S3 configuration refers to server environment variable names, not secret values.

Activity separates logical/retained usage, staging, physical occupancy and pending
cleanup. Pending deletion is not free capacity. Integrity/Operations expose scans,
findings, mirror repair and resumable maintenance with explicit administrative
actions. Root freshness reflects successful server commits; expected intervals do
not schedule backups or prove consistent source capture.

Checkpoints exports the catalog and inventory, with a brief publication barrier
and generation pins. **An exported catalog is not an independent backup.** Follow
[checkpoint recovery](checkpoint-recovery.md) to copy and verify payloads in a
separate failure domain and recover into an empty isolated destination. Failed
exports retain pins until explicit release. Releasing pins is irreversible; first
confirm the independent package is complete. There is no browser application
restore or backup-now action.

See [observability](observability.md) for readiness, metrics and logs, and
[the SDK guide](rust-sdk.md) for program integration.
