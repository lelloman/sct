# Immutable file format and transfer contract v1

SCT-14 freezes these rules. Directory snapshots remain reserved for M3.

## Encoding and identity

Canonical JSON is compact UTF-8 with lexicographically sorted ASCII field names,
no whitespace, and serde_json string escaping (no optional escapes). Values must
round-trip to exactly the submitted bytes. Unknown or duplicate fields, floats,
negative lengths, integers above 9007199254740991 and trailing bytes are invalid.
Digests are lowercase SHA-256 hex of the exact raw chunk/canonical object bytes.
A file manifest digest is **not** the digest of concatenated file bytes.

FastCDC uses fastcdc 3.2.1 v2020, normalization level 1, seed 0, min 262144,
target 1048576 and max 4194304. The profile identifier is
`fastcdc-v2020-3.2.1-n1-s0-256k-1m-4m`. The checked-in chunk-profile fixture
contains lengths and SHA-256 digests for the explicitly seeded xorshift64 input
in the Rust fixture generator. Client implementations must match it exactly.
Transport read fragmentation must not change boundaries. Empty files have no chunks.

## Bounded graph

Rust's FileManifest and ManifestPage are the normative fields. Every object has
format 1. A file root records profile, total length, total chunk count, tree digest
and tree height. The empty file uses length/chunks/height 0 and tree null.
Nonempty trees start with height 0 chunk pages; branch height is one greater than
each child. Each page holds 1–1024 ordered references, each canonical object is
at most 262144 bytes and tree height is at most 8. Branch references record each
child's total file length and chunk count. Server graph validation checks these
summaries, depth, every digest, chunk length and root totals. A tree cannot cycle
because child heights strictly decrease. Repeated references are meaningful and
preserve file order; physical chunk storage deduplicates within a root only.

Chunk lengths are 1–4194304 bytes. Small final chunks are legal. The server
verifies chunk digest/length, not whether a client used optimal CDC boundaries.
Catalog graph edges are shared; traversals and missing/history results are paged
(maximum 1000 results) and never materialize an entire file graph in RAM.
Physical keys use opaque root ID, object digest and unique copy generation, never
user names. Manifest objects follow the same required-copy rules as payloads.

## Implemented HTTP workflow

All routes are under `/api/v1/roots/{root}`. Generated capabilities advertise these M2 routes; later milestone operations
remain disabled.

| Request | Behavior |
| --- | --- |
| POST uploads | CreateUploadRequest: bind principal, root, target/revisions, immutable file manifest and idempotency key; return TransferStatus |
| PUT uploads/{id}/pages/{digest} | Canonical page body; bounded metadata; identical replay succeeds, changed bytes conflict |
| POST uploads/{id}/admit | Freeze complete graph, pin reuse, check limits, reclaim if allowed and reserve all destination pools |
| GET uploads/{id} | TransferStatus, authenticated status/resume and expiry renewal |
| GET uploads/{id}/chunks | MissingChunkPage with opaque cursor, at most 1000 references |
| PUT uploads/{id}/chunks/{digest} | Admitted only; exact planned length/digest; maximum 4 MiB; acknowledged copies survive retries |
| POST uploads/{id}/seal | Validate full graph and all required acknowledgments; set session ready |
| POST uploads/{id}/commit | Atomically publish; return original CommitReceipt on exact retry |
| POST uploads/{id}/abort | Durable abort and cleanup; never release uncertain physical charges |
| GET entries/{entry}/versions | Paginated retained FileVersion history |
| GET versions/{version}/manifest | Canonical file manifest |
| GET versions/{version}/content | Verified bytes; single HTTP byte range supported |

Admission planning/pending/admitted/rejected is distinct from session
open/ready/committed/aborted/expired. A pending operation returns 202 and retry
information; payload permission requires admitted. A completed admission returns
200. Guard chunk bodies before consuming them; a client must wait for admission.
Proxies may already have buffered bytes. Metadata submission is not payload upload.

Chunk/page bodies have a 60-second ingestion deadline; retry the same bounded
request on transport failure. Inactivity expires after 24 hours by default. Authenticated successful session actions renew
expiry; worker leases are separate. Credential revocation is immediately effective.
Every action reauthorizes; commit also rechecks expected target and policy
revisions. Planning binds content immutably; changed source requires a new session.

Capacity errors distinguish quota_exhausted, staging_exhausted and
destination_capacity. Revisions produce revision_conflict, changed idempotent
requests idempotency_conflict, bad content integrity_failure/invalid_manifest,
expired sessions upload_expired, backend faults destination_unavailable. No
private backend configuration appears in error responses. The M1 envelope applies.

A terminal key binds principal/root/operation/request hash permanently. A receipt
can outlive retired content; exact retries return the original publication result.
If full receipt retention later expires, retain a non-reusable key tombstone.

Reads pin content before retirement can remove it, renew leases and verify whole
chunks before releasing any bytes, including ranges. Late failures abort the
stream; clients validate total returned length and never report partial success.
