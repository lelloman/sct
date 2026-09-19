# Independent-file transfers with the Rust SDK

`sct-client` depends only on public protocol and transport libraries. Run
`cargo run -p sct-client --example upload -- ROOT SOURCE NEW_JOB_DIRECTORY`
with `SCT_ORIGIN` and `SCT_TOKEN` set. Repeat with the same job directory to resume
or recover a lost commit receipt. Keep tokens out of job metadata.

Capture a **stable, caller-owned file** before `UploadJob::prepare`. For a live
SQLite database, use SQLite's backup API or another consistent snapshot mechanism;
copying an actively written database file is not a consistent capture. The SDK
neither captures SQLite nor freezes a filesystem. Keep the source unchanged until
commit. A source mismatch returns `SourceChanged`; abort the old upload and prepare
a new job with a new idempotency key. Replacing an existing file uses
`FileTarget::Replace` with its current entry revision and effective policy revision.

Planning performs one sequential chunking pass and writes the immutable request,
chunk index and canonical pages to the caller's job directory. Metadata grows on
disk, not in memory. It holds at most a chunker buffer, a chunk and 1024 page
references. Run preparation on a blocking thread when sharing an async runtime.
The transfer pass validates every source chunk, including reusable chunks, before
commit. Metadata is submitted before admission; missing payload is sent only after
all destinations are reserved. The durable job supplies the same idempotency key
across process restarts. Dropping an upload future disconnects; `abort_upload` is
explicit. Pending admission/sealing is polled with a delay. Transport errors return
to the caller, who controls retry/backoff and can safely resume the job.

`file_history` uses opaque cursors. `download_to` supports whole files and half-open
ranges, checks response status, range headers and exact advertised length, and
surfaces stream failures. Write downloads to a temporary destination and rename
only after success; a failed download can leave a prefix in that destination.
Server chunk verification protects bytes before release; use HTTPS to protect
transport in production. The SDK disables redirects and applies connection/idle
transport timeouts rather than a short total transfer timeout.

Job directories contain metadata and missing-chunk markers, not captured payload.
Keep the job and source until the receipt is durably recorded, then remove them
according to the application's lifecycle. They are trusted local state, not an
untrusted interchange format. Preparing into an existing directory is rejected;
a failed preparation leaves an incomplete directory for caller cleanup.

## Trees and lifecycle

The standalone client exposes tree draft create/status/advance, identity allocation,
mutation pages, file manifest pages, freeze/admit/commit and explicit abort.
It also exposes immutable snapshot listing/path lookup/range download, hide,
policy preview/effect paging/apply, purge and job status/advance/resume.

For a durable replacement or patch:

1. Capture stable source files yourself. Construct and persist a
   `CreateTreeDraftRequest` with the exact directory/base/policy revisions and
   a stable request key. Choose `CompleteReplacement` or `Patch` explicitly.
2. Call `create_tree_draft`, then `allocate_tree_identities` with stable allocation
   keys for new entries. Existing snapshot identities need no allocation.
3. On a blocking thread, call `TreeJob::prepare(directory, request, pages)`.
   Supply an iterator of bounded pages of `TreeMutation` (file, directory,
   removal or an existing manifest). One source file per identity is supported.
   Mutation order, manifests and chunk indexes are persisted; the caller retains
   stable source files. Preparation never publishes and an incomplete preparation
   has no usable job receipt.
4. Call `client.upload_tree(&job)`. After disconnect or process restart, reopen
   with `TreeJob::open` and call again. Exact mutation keys and server receipts
   make replay safe. Payload starts only after whole-tree admission. Every source
   is rehashed, including reusable chunks; drift returns `SourceChanged`.
5. Keep the job directory until the commit receipt is known. Dropping a future
   disconnects only; `abort_tree_draft` is explicit.

Memory is bounded by one mutation page, one manifest page and the current chunk.
Tree/file counts use disk-backed plans, directory iteration and server pagination.
Snapshot downloads stream into caller-owned temporary output; promote only after
success. Patch omission preserves entries; replacement omission removes them from
the new snapshot. Older retained snapshots remain addressable by version.

Destructive lifecycle calls never auto-approve effects: the caller constructs
`ApplyChangeRequest` with explicit acknowledgment or an explicit `PurgeRequest`.
Persist these request keys to recover an uncertain response. Inspect and advance
the returned job; blocked jobs require explicit authorized resume. No rollback
or cancel promise is exposed after apply.
