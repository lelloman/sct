ALTER TABLE archive_sets ADD COLUMN archive_run_after timestamptz NOT NULL DEFAULT clock_timestamp();
CREATE TABLE archive_storage (
 singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
 directory text NOT NULL, quota_bytes bigint NOT NULL CHECK(quota_bytes BETWEEN 1 AND 9007199254740991),
 cache_bytes bigint NOT NULL DEFAULT 0 CHECK(cache_bytes>=0),
 reserved_bytes bigint NOT NULL DEFAULT 0 CHECK(reserved_bytes>=0),
 CHECK(cache_bytes+reserved_bytes<=quota_bytes)
);
CREATE TABLE archive_exports (
 set_id text PRIMARY KEY REFERENCES archive_sets(id),
 reserved_bytes bigint NOT NULL CHECK(reserved_bytes>=0),
 worker_epoch bigint NOT NULL DEFAULT 0,
 server_epoch bigint, worker_expires_at timestamptz,
 finished boolean NOT NULL DEFAULT false,
 writer_version text NOT NULL, writer_options jsonb NOT NULL
);
CREATE TABLE archive_builds (
 disc_id text PRIMARY KEY REFERENCES archive_discs(id),
 artifact_id text NOT NULL UNIQUE REFERENCES archive_artifacts(id),
 attempt text NOT NULL, stage text NOT NULL CHECK(stage IN ('staging','verified','available','failed')),
 relative_path text NOT NULL,
 server_epoch bigint NOT NULL, worker_epoch bigint NOT NULL,
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX archive_artifact_expiry ON archive_artifacts(expires_at,id) WHERE state='available';
