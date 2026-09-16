# ScT v1 protocol foundations

Rust definitions in `crates/sct-protocol` own these contracts. The optional
`codegen` feature derives JSON Schema with Schemars and TypeScript with ts-rs;
the normal client/protocol build does not depend on either generator.

```sh
./scripts/generate-contracts        # write all generated artifacts
./scripts/generate-contracts check  # compare without modifying files
./scripts/check                    # includes generation, fixtures and HTTP checks
```

Generated artifacts are `openapi.json`, `schema.json`, `operations.md`, and
`web/src/generated/api.ts`. They contain no timestamps or filesystem paths.
The check works in an exported checkout without Git. Tests exercise repeated
generation in separate processes and deliberately corrupt/remove outputs.

## What is implemented

This delivery provides DTOs, scalar parsing, self-contained validation, generation,
fixtures and the HTTP error envelope. It does **not** implement the planned root,
namespace, policy, upload or administration operations.

`openapi.json` is OpenAPI 3.1 with an empty `paths` object. Its schemas describe
the foundation DTOs; `x-sct-planned-operations` contains the complete operation
map with milestone and implementation status. `operations.md` is generated from
that same Rust map. A null request/response schema means its shape is reserved;
it does not authorize accepting arbitrary JSON. GET request DTOs describe query
parameters, not GET bodies. Future handlers must promote operations into `paths`
only when implemented, register their request/response schemas, and expose them
in capability discovery.

The scaffold returns `404 unsupported_operation` for all `/api` and `/api/*`
methods. Errors have JSON content, a server-generated `X-Request-Id` matching
the envelope, and `Cache-Control: no-store`. Health endpoints remain outside v1.
No capabilities endpoint is advertised as working.

## Wire conventions

- JSON uses snake_case fields and enum variants. Tagged unions use `kind`.
- IDs and revisions are distinct Rust types and nominal TypeScript string types.
  Their contents are opaque; clients must not parse ordering or provider location
  from them. ID/revision/idempotency tokens allow 1–128 ASCII letters, digits,
  underscores or hyphens. Cursor tokens use the same alphabet, up to 2,048 characters.
- Manifest IDs and chunk digests are separate types containing 64 lowercase hex
  SHA-256 characters. A manifest digest is not the reconstructed file digest.
- Names are case-sensitive UTF-8, 1–255 Unicode scalar values, without `/`, NUL,
  `.` or `..`. Logical paths are absolute within a root, up to 4,096 scalar values.
  `/` denotes that root. Empty components and trailing slashes are rejected.
  Names are not normalized or mapped onto host paths.
- Byte counts, commit sequences and millisecond timestamps are integers in
  `0..=9007199254740991`, preserving exact JavaScript values. Timestamps are Unix
  epoch milliseconds; revisions remain opaque strings. Unsafe JSON numbers,
  fractions, negatives and overflows are rejected rather than rounded.
- Page size is 1–1,000; responses contain at most 1,000 items. Version counts are
  1–1,000,000 and include the current content version. Timeout/progress windows are
  1–86,400,000 milliseconds. Positive throughput limits use exact safe integers.
  These are protocol ceilings; server capabilities and administrator policy may
  lower operational limits. Concrete default timeout policy belongs to M2 wiring.
- Unknown DTO fields and variants are rejected. Nullable fields serialize as
  explicit `null`; Serde also accepts omission for optional inputs. Generated
  TypeScript describes normalized serialized values. These pre-release contracts
  are not a promise of compatibility with an already released v1.

Parse with Serde and call `validate()` where provided. These methods enforce
self-contained invariants for placement, root requests, policy apply, pages and
upload status. Schema validation handles structural constraints; it cannot replace
the cross-field validation methods or core's authorization/catalog checks.

## Revisions, listing and retries

Mutations require the relevant expected revision. Creates carry idempotency keys
and directory creation also checks the parent revision. Renames check the entry,
source parent and destination parent revisions. Foundation rename rejects crossing
effective-policy or atomic-versioning boundaries. A future explicit policy-effect
flow can extend that request; there is no silent retention change.

Directory cursors contain the directory ID and observed revision plus an opaque
token. The server must authenticate the token and bind it to the principal, root,
directory, revision, filter and limit. `validate_cursor` compares the authorized
directory snapshot; a mismatch produces `cursor_refresh_required` with an explicit
refresh instruction. A page validates that its continuation belongs to its snapshot.

Errors use stable codes, safe fixed messages, request IDs, and `retry` instructions:
`never`, `after`, `refresh`, or `reconcile_upload`. `root_busy` and policy transitions
are 409 with a retry delay; callers must preserve idempotency when retrying. Codes
distinguish root quota, server staging and destination capacity (507), destination
unavailability (503), expiry (410) and integrity failure (502). Core may attach only
authorized typed details. Never copy backend diagnostics, credentials or host paths
into a public error. Future retryable handlers should also send `Retry-After` for
the envelope's delay. Generic HTTP success headers will use the same request ID.

Policy apply references an expiring preview and both the policy and evaluated scope
revisions. Core must reject stale previews, reauthorize the operation and verify the
acknowledged effects before scheduling a transition.

## Placement, capacity and transfer extension points

A placement is a single backend ID or exactly two distinct mirror backend IDs.
Read candidates must contain each member exactly once; their order is explicit and
independent of member ordering, adapter kind or hints. Connection, initial response,
idle, throughput-window and startup budgets are distinct. Hints may be unknown and
are separate from observed health. Alias detection and provider validation require
the core registry; the wire schema does not claim failure-domain independence.

Backend inspection and adapter behavior documentation expose no destination paths,
credentials or provider receipts. Root allocation requests refer to capacity pool
IDs separately from backend IDs. Core resolves pool membership and accounts for
each physical copy, including two copies in the same pool.

Allocation inspection distinguishes user roots from `server_staging`. Reserved
bytes describe unused reservations; occupied bytes describe physically occupied
capacity, including the reported pending-cleanup subset. These fields must not be
added blindly or presented as available capacity. Pool reports may have unknown
available space; capacity recovery may be confirmed, delayed or unknown. Exact
atomic ledger transitions remain M2/M3 design work.

Operation inspection exposes root ownership, recovery/cleanup state, progress,
retry guidance and safe failure codes. It does not expose worker epochs, leases
or physical generations. Terminal upload state does not imply cleanup capacity is
free; inspect the owning operation and allocation reports as well.

Upload status distinguishes per-destination acknowledgements and the committed
receipt. Ready/committed views require all reported required copies acknowledged;
only committed views carry receipts. Actual catalog publication still requires the
M2 transaction and placement checks. Acknowledgement is not certified durability.

Directory draft references explicitly identify complete replacement versus patch,
the expected base, and server-issued new versus existing entry identities. Core
must check identity continuity, root/draft ownership and uniqueness. A deleted path
recreated later gets a new entry ID. Stable IDs, paths, version IDs and content
digests are independent. Directory deletion/purge semantics remain unresolved;
these references do not choose that behavior.

Storage format version 1 is **reserved**, not supported yet. Canonical manifest
encoding, page structure, exact chunk-profile fixtures, source consistency, upload
body/session contracts, read leases and checkpoint recovery remain later work.
No storage format or chunk profile should appear in capabilities until implemented.

## Shared fixture checks

`crates/sct-protocol/tests/fixtures/wire.json` supplies positive and negative cases
to Rust/Serde and TypeScript/Ajv. Both check Unicode paths, numeric bounds, cursor
envelopes, strict fields, placement unions, draft identity, retry envelopes and
server staging. TypeScript also consumes validated fixtures through generated types
and has compile-only failures for exchanged identities, missing revisions and
incorrect mirror cardinality. Rust separately tests cross-field invariants.

Generator references: [Schemars](https://docs.rs/schemars/1.2.2/schemars/)
and [ts-rs](https://docs.rs/ts-rs/12.0.1/ts_rs/trait.TS.html).
