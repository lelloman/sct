CREATE TABLE archive_downloads (
 id text PRIMARY KEY, artifact_id text NOT NULL REFERENCES archive_artifacts(id),
 principal_id text NOT NULL REFERENCES principals(id), credential_id text NOT NULL,
 expires_at timestamptz NOT NULL,
 deadline timestamptz NOT NULL DEFAULT clock_timestamp()+interval '7 days'
);
CREATE INDEX archive_download_artifact ON archive_downloads(artifact_id,expires_at);
CREATE INDEX archive_download_expiry ON archive_downloads(expires_at,id);
ALTER TABLE archive_artifacts ADD COLUMN cleanup_pending boolean NOT NULL DEFAULT false;
ALTER TABLE archive_accounts ADD COLUMN cache_ttl_seconds integer NOT NULL DEFAULT 604800 CHECK(cache_ttl_seconds BETWEEN 3600 AND 2592000);
