# Checkpoint export and isolated recovery

A checkpoint initially contains a catalog and a generation inventory. It is
`catalog_only`: losing the original payload destinations still loses those bytes.
Keep its pins until an independent copy has been verified. Pin release is an
explicit administrator operation and cannot be undone.

## Export

Create a checkpoint with an administrator credential:

```http
POST /api/v1/admin/checkpoints
Content-Type: application/json

{"idempotency_key":"operator-checkpoint-2026-09"}
```

Poll `/api/v1/admin/checkpoints/{id}` until it is `exported`. Download
`catalog.jsonl`, `inventory.jsonl` and `descriptor.json` from
`/api/v1/admin/checkpoints/{id}/artifacts/{filename}` into a private package
directory. The service also stores them under `checkpoint_dir/{id}` (default
`checkpoints`). Keep that directory outside the web asset directory. Catalog
artifacts contain sensitive administrative data; their HTTP routes require an
administrator and disable caching.

The descriptor binds both JSONL files to SHA-256 digests, the checkpoint identity,
a monotonic generation watermark, and format versions. Catalog rows come from a
single PostgreSQL snapshot. The barrier stops new deletion work, drains issued
deletes, and freezes publication while the generation pins are recorded. Normal
catalog operations resume after the export snapshot starts. Export failure
retains pins and appears in checkpoint status. A restarted server marks an
interrupted export failed instead of pretending to resume its lost SQL snapshot.

## Copy and verify payloads

Payload copying is an explicit operator action. Create an empty directory on
storage independent of the production destinations. The operator is responsible
for selecting a separate failure domain; a different directory on the same disk
is insufficient protection against losing that disk.

Supply a source map with the original backend IDs and explicit adapter settings:

```json
{
  "local-primary": {
    "kind": "local",
    "settings": {"directory": "/srv/sct/payloads", "headroom_bytes": 0}
  }
}
```

The same map supports the documented `s3` adapter settings and credential
environment references. No source configuration is inferred from the exported
catalog. The copy command reads only the exact generations in the inventory,
checks length and digest, and writes each into independent local storage:

```sh
sct-server copy-checkpoint-payloads PACKAGE sources.json EMPTY_PAYLOAD_DIR --acknowledge-independent
sct-server verify-checkpoint PACKAGE PAYLOAD_DIR
```

Successful verification prints a `payload_verified` descriptor. The original
export descriptor stays `catalog_only`; copying does not release service pins.
Missing/corrupt generations, unsupported formats, checksum mismatches, symlink
substitution, and local destination overlap are rejected. Copying requires an
empty destination; failed copies can leave partial files and require a fresh
empty destination for another attempt. Payload layout is
`PAYLOAD_DIR/backend/root/digest-prefix/digest/generation`.

After preserving and verifying the independent package, an administrator may
release its service pins with the current checkpoint revision:

```http
POST /api/v1/admin/checkpoints/{id}/abandon
Content-Type: application/json

{"expected_revision":"CURRENT_REVISION","acknowledge_release":true}
```

## Recover in isolation

Use an explicitly selected empty PostgreSQL catalog and a separate empty local
payload destination. An empty already migrated ScT catalog is accepted; existing
application data or unrelated tables are refused. The supplied package must use
the current catalog schema version and migration checksums. Unsupported formats
are rejected before recovery writes. Recovery never opens the old catalog's
backend destinations or resolves its credential references.

```sh
# Set this variable securely to the isolated database connection string.
sct-server recover-checkpoint PACKAGE PAYLOAD_DIR EMPTY_RESTORED_DIR --database-url-env SCT_RECOVERY_DATABASE_URL --acknowledge-isolated
```

These commands run before normal `SCT_CONFIG` loading. Recovery verifies the
package and every payload, checks inventory identities against the captured
catalog, validates namespace and retained content graphs, and imports catalog
rows with normal foreign-key/check constraints enabled. Records are streamed in
bounded batches; a dedicated SQL transaction protects the isolated import.
Failure rolls back imported catalog data, though schema initialization and
partial destination files may remain. Retry using an empty destination.

All restored backends retain their IDs and pool assignments but point to new local
payload directories under `EMPTY_RESTORED_DIR`. Old credentials are revoked.
Runtime leases, reservations, read pins and checkpoint barriers are cleared;
unfinished uploads expire and never become committed backups. Maintenance work
that needs authority is blocked for explicit administrator review/resume. Identity
sequences continue above imported values. Committed entries, versions, policies,
grants and receipts remain intact.

Create a separate configuration for the recovered service with its isolated
`database_url_env`, web directory and bind/public URL. The recovered catalog
already has its independent backend configuration; startup does not reapply
production bootstrap destinations. If the IdP is unavailable, recover a local
administrator on the operator's terminal:

```sh
SCT_CONFIG=recovery.toml sct-server recover-admin
SCT_CONFIG=recovery.toml sct-server
```

The first command intentionally prints a new credential once. Use it to inspect
the recovered namespace and download committed file/tree versions through the
normal API/SDK. The recovery drill in the integration suite disables original
storage before doing these reads and also verifies subsequent writes. This is
service recovery; consumer programs still own application-level restore.
