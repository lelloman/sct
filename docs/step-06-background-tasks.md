# Step 06 background work

Starting master `2651b53`; reviewed simple-server source
`06c7531dfcd4149a0947b4b74a79e96bf245639c` (simple-server.rev).

06a adopts TaskSet ownership for the two fixed archive/recovery workers. Each
bounded owner retains the existing ordered explicit abort and join before writer
lease release and catalog closure. Drop still detaches unfinished work; no second
shutdown budget is installed. HTTP body producers, read-pin release and archive
lease cleanup remain application-owned; this is scoped server-worker adoption.

06b adopts shared `run_poll_worker`/`PollCadence` for the production archive
and recovery loops, with the same immediate first cycle and two-second delay
after each completed cycle, including errors. This is scoped worker scheduling:
PostgreSQL still owns due-time selection, SKIP LOCKED claims, clocks, worker
epochs and root obligations. Existing TaskSet abort/join order still owns shutdown;
the driver adds neither a second deadline nor in-memory durable claims.
06c adopts RetryPolicy
for maintenance delay calculations only, preserving 2/4/8/16-second retries and
the existing 32-second calculation cap. Attempt exhaustion/classification and
clock_timestamp-based eligibility remain in the same transaction. Archive's fixed
poll/recheck intervals and durable recovery state remain application-owned.
The core dependency disables shared HTTP features.

Baseline ordinary affected tests: 15 pass, 91 database/qualification tests ignored;
two maintenance tests separately pass against disposable PostgreSQL. Final ordinary
tests: 18 pass, the same 91 ignored; both PostgreSQL maintenance tests pass with
new assertions bounding persisted due times by database timestamps and checking
no deadline after exhaustion. New worker tests prove cancellation joins before
resource release and tolerate completed/panicked workers. Strict affected all-target
Clippy, changed-file formatting and diff checks pass. Both server/recovery binaries
build. Five real-process cases pass: SIGINT/SIGTERM with active HTTP body drain,
database-backed writer release/immediate restart under both signals, and lease
loss exiting unsuccessfully. Disposable containers are removed after each run.

The process harness now honors CARGO_TARGET_DIR, checks the actual JSON shutdown
event, supplies the current recovery-binary fixture requirement and uses its own
static page instead of requiring an unrelated frontend build. It had stale logging
and fixture assumptions before this migration. Full archive/media/S3/browser suites
were not rerun; local validation files and existing CORS notes are unrelated work.

06b completion from master `6a999be`, shared source `429b782` (2026-09-23):
baseline/final ordinary core/server suites each pass 18 tests with the same 91
opt-in database/qualification tests ignored. Strict affected all-target Clippy,
changed-file rustfmt and diff checks pass. Server and recovery binaries build.
Five real-process checks pass against disposable PostgreSQL: SIGINT/SIGTERM
with HTTP drain, writer release/immediate restart under both signals, and lease
loss exiting nonzero. Containers are removed by the harness. Full archive/media,
S3, browser and ignored qualification suites were not rerun. Existing untracked
validation work and CORS notes remain unrelated and preserved.
