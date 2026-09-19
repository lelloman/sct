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
