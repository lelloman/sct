# Payload adapters

Local settings: `directory` (existing private service-owned directory) and optional
`headroom_bytes`. Publication verifies a bounded object, writes an exclusive
same-directory temporary file, fsyncs it, hard-links without replacement, removes
the temporary name, then fsyncs each directory up to the storage base. Retries
verify existing bytes. Correctness requires filesystem/hardware fsync guarantees
and no external path/symlink substitution. Capacity probes use statvfs available
bytes minus headroom in addition to catalog pool reservations. Map directories
on the same volume into one capacity pool.

S3 settings: `endpoint`, `bucket`, `region`, `access_key_env`, `secret_key_env` and
optional `session_token_env`. Secrets resolve privately at adapter opening.
Endpoints must use HTTPS except loopback HTTP for integration fixtures. Path-style
addressing uses generation-specific ASCII keys. Requests use SigV4 and conditional
If-None-Match PUT; a success/conflict is followed by verified GET before acknowledgment.
ETags never prove SHA-256. Redirects are disabled to keep signed credentials at the
configured endpoint. Configure a catalog capacity budget; remote free space is unknown.

S3 v1 supports only never-versioned buckets without Object Lock. First I/O checks
bucket configuration and fails closed if permissions prevent inspection. Suspended
versioning is also unsupported because old versions may still occupy storage.
Do not enable versioning/lock or external lifecycle rules while serving a root.
The provider must honor strong read-after-write/delete behavior. Provider durability
is documented, not certified by an operational probe. Objects are at most 4 MiB,
so multipart uploads and their separate abort/accounting semantics are unnecessary.

`./scripts/test-s3` runs conformance against disposable pinned MinIO; it also checks
corruption, bad credentials and versioned-bucket rejection. This is S3-compatible
qualification, not a claim of testing AWS infrastructure.

Protocol references: [AWS conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html)
and [SigV4 signing](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html).
