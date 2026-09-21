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
reserved to cache usage. The initial account limit is 100 GiB; quota management
and ordinary cache expiry/download lifetimes are provided by SCT-73.

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
