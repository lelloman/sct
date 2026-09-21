# Verified ISO exports

Install `xorriso` and util-linux (`/usr/bin/prlimit` and `/usr/bin/setpriv`) on the Linux service host.
Build the production recovery bundle using `scripts/build-recovery-bundle`, then
configure the optional `[archive]` section:

```toml
[archive]
directory = "/srv/sct/disc-exports"
bundle_directory = "/opt/sct/recovery-bundle"
quota_bytes = 1099511627776
```

Use a private service-owned directory. Its canonical identity is persisted; moving
an existing cache requires a deliberate migration, not changing this path while
workers are active. The directory contains a process lock, private staging trees
and an immutable ISO cache. Bundle startup validation checks the canonical
manifest, supported platform, required executable/source/instructions and every
file hash. The same manifest identity is fixed for each plan. A configured bundle
is administrator-trusted software; its checksums detect accidental changes and do
not authenticate an unknown software publisher.

A separate archive task advances bounded planner steps and one disc per turn,
while the normal transfer worker and server lease heartbeat continue. It fairly
rotates active sets. Confirmation freezes the plan; image construction requires
current source authority, valid source pins, readable reused media and the fixed
capture deadline. A database worker epoch and a local exclusive filesystem lock
prevent old workers from publishing or reclaiming a replacement worker's files.
Long I/O periodically rechecks authority, expiry and ownership.

Before staging, account and host quotas reserve the sum of all planned images plus
twice the largest disc budget. The latter covers one staged tree and one extracted
verification tree; only one disc uses that workspace at a time. Filesystem free
space is checked as an additional bound. Reservations remain charged until files
are removed, including after failure/restart. Published image bytes move from
reserved to cache usage. The initial account limit is 100 GiB. Administrators can revise account limits
through `/api/v1/admin/principals/{principal}/archive-limits`; reductions below
charged cache/reservation/pin usage or the number of active sets are rejected.
Users can inspect their limits through `/api/v1/archives/limits`.

Each image stages the complete identical index/map, descriptor, verified recovery
bundle and its assigned payload. Source chunks are read using the root's configured
read policy with timeouts, throughput checks, fallback and SHA-256 verification.
The writer uses ISO level 3 and rationalized Rock Ridge permissions, preserving the
recovery executable bit while making mounted files readable. Writer version and
options are recorded in the export catalog. `prlimit` imposes a hard file-size
limit on the writer, which also has a two-hour deadline and is killed when its
supervised operation is cancelled. The child inherits the filesystem lock and a
parent-death signal, so a killed server cannot leave an unfenced writer running
while a replacement reuses its staging reservation.

The service reopens and extracts the generated ISO into private verification
storage. It checks every descriptor/index/bundle/payload hash, expected length,
executable permissions and exact file count, rejecting symlinks and special files.
The actual ISO must fit its conservative planned budget (which itself fits selected
media capacity). No physical burning is performed or implied by this verification.

After verification, the service fsyncs the image, records its immutable hash and
length, publishes using a no-replace hard link, fsyncs the cache directory and only
then marks the artifact available. A restart reconciles the exact verified
candidate if it exists; a missing/corrupt candidate fails explicitly. It never
silently replaces bytes associated with a published hash. A pre-verification
interruption rebuilds the current disc after removing its old staging tree.
Completed prior discs remain unchanged. Source pins release after the complete set
is ready, or after cancellation/expiry/failure has fenced the worker. Permanent
logical disc/index history is independent of these temporary files.

Migration 23 stores host storage accounting and export/build ownership. Service
checkpoints remain schema-specific. Recovery from an older service checkpoint and
reconciliation of physical history are handled by the archive-catalog recovery
flow, SCT-75.

Integration tests generate and extract real ISO files, invoke the recovery
executable from each image and independently restore file bytes. They interrupt a
staging worker, replace its server lease, verify resumed publication and test
corrupt sources, quota denial, size-limited writer failure and no-overwrite behavior.
The integration bundle contains the real development recovery executable and an
explicit source placeholder; full vendored source and clean Debian runtime
qualification use the separate production bundle build/test scripts. These tests
do not qualify a physical burner or disc.

## Downloads and cache lifetime

`GET /api/v1/archives/status` reports whether export construction is configured and
which recovery bundle/platform is installed. A configured worker is necessary for
building or serving images. It does not imply that every historical image is cached.

`GET /api/v1/archives/{collection}/discs/{disc}/iso` serves only images from a Ready
set, with current authority over every source in that disc's original index.
The quoted SHA-256 ETag identifies the immutable image. Single byte ranges,
open-ended ranges and suffix ranges return 206; invalid/unsatisfiable ranges return
416. `If-Match` protects image identity; a mismatched `If-Range` returns the full
image. HEAD returns headers without starting/counting a transfer. Responses are
private and not cacheable. Download counts record attempts, not completed burns.

Each stream holds a durable 30-second read lease, renewed before and after each
bounded 1 MiB chunk, with an absolute seven-day deadline. Current account,
credential and complete source authority are rechecked at those boundaries.
A disconnected or stalled stream can resume using the immutable ETag and range;
a stream stalled longer than its lease must start a new request. Already delivered
bytes cannot be revoked. Full streams check the final SHA-256 before returning the
last chunk. Range clients must validate the assembled image's full hash.
The Rust SDK's `resume_archive_iso_to` hashes the existing temporary-file prefix,
checks the response ETag/range/length, and verifies the whole image before success.
Keep this output temporary until validation succeeds.

Account cache TTL defaults to seven days and can be configured between one and
30 days for future published sets. The download window starts when the complete
set becomes Ready. Existing published artifact deadlines remain unchanged.
Expiry stops new downloads; a live download lease temporarily holds the cached
file beyond TTL. Cleanup is serialized with builders, records durable deletion
intent, unlinks and fsyncs the cache directory, then releases the storage charge
once. Restart repeats incomplete cleanup safely. Stale download leases are reaped
in bounded batches. Expiry never removes logical discs, snapshot indexes, hashes,
physical-copy records or verification history. A successfully generated set remains
Ready even after its cached images expire; inspect each artifact's availability.

There is no promise to regenerate an expired image: source pins have been released,
source bytes may have been pruned, and the original bundle/build inputs may no
longer exist. Download and preserve the verified image before its deadline, burn
and read back physical copies, or create a new capture from available sources.
Migration 24 records download leases, cleanup intent and account cache TTL.
