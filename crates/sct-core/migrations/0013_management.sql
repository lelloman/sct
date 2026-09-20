-- M4 catalog additions preserve all existing payload/operation/accounting rows.
ALTER TABLE principals ADD COLUMN revision text NOT NULL DEFAULT gen_random_uuid()::text;
ALTER TABLE allocations ADD COLUMN revision text NOT NULL DEFAULT gen_random_uuid()::text;
ALTER TABLE roots ADD COLUMN quota_ceiling bigint;
UPDATE roots SET quota_ceiling=quota;
ALTER TABLE roots ALTER COLUMN quota_ceiling SET NOT NULL;
ALTER TABLE roots ADD CONSTRAINT quota_ceiling_range CHECK(quota_ceiling BETWEEN 0 AND 9007199254740991);
ALTER TABLE roots ADD COLUMN expected_upload_interval_seconds bigint
 CHECK(expected_upload_interval_seconds BETWEEN 60 AND 31536000);
ALTER TABLE roots ADD COLUMN last_commit_at timestamptz;
-- Existing root creation paths initialize a ceiling without gaining authority to
-- change it on later updates. Management APIs enforce quota <= ceiling.
CREATE FUNCTION initialize_root_ceiling() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.quota_ceiling IS NULL THEN NEW.quota_ceiling := NEW.quota; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER root_ceiling_insert BEFORE INSERT ON roots FOR EACH ROW EXECUTE FUNCTION initialize_root_ceiling();

-- Absence means not yet imported, not an empty set of backend settings. Secrets
-- remain references resolved by the operator's runtime environment.
CREATE TABLE backend_configurations (
 backend_id text PRIMARY KEY REFERENCES backends(id), revision text NOT NULL,
 settings jsonb NOT NULL CHECK(jsonb_typeof(settings)='object'),
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE configuration_state (
 singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
 imported_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE management_audit (
 sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY CHECK(sequence<=9007199254740991),
 root_id text REFERENCES roots(id), principal_id text REFERENCES principals(id),
 event text NOT NULL, details jsonb NOT NULL CHECK(jsonb_typeof(details)='object'),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX management_audit_root ON management_audit(root_id,sequence);
CREATE INDEX management_audit_principal ON management_audit(principal_id,sequence);

-- A generation UUID has identity but no ordering. This independent sequence is
-- assigned to old and new rows and never changes when a copy is reconciled.
ALTER TABLE object_copies ADD COLUMN created_sequence bigint GENERATED ALWAYS AS IDENTITY;
ALTER TABLE object_copies ADD CONSTRAINT copy_sequence_range CHECK(created_sequence BETWEEN 1 AND 9007199254740991);
CREATE UNIQUE INDEX copy_creation_sequence ON object_copies(created_sequence);
CREATE INDEX copy_root_sequence ON object_copies(root_id,created_sequence);
CREATE FUNCTION protect_copy_sequence() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.created_sequence<>OLD.created_sequence THEN
 RAISE EXCEPTION 'copy creation sequence is immutable' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER copy_sequence_immutable BEFORE UPDATE ON object_copies FOR EACH ROW EXECUTE FUNCTION protect_copy_sequence();

CREATE TABLE maintenance_jobs (
 id text PRIMARY KEY, root_id text NOT NULL REFERENCES roots(id),
 principal_id text NOT NULL REFERENCES principals(id), credential_id text REFERENCES credentials(id),
 key text NOT NULL, request jsonb NOT NULL,
 kind text NOT NULL CHECK(kind IN ('integrity_scan','mirror_repair')),
 state text NOT NULL DEFAULT 'queued' CHECK(state IN ('queued','running','retry_wait','blocked','completed','failed')),
 revision text NOT NULL, cursor jsonb NOT NULL DEFAULT '{}',
 processed bigint NOT NULL DEFAULT 0 CHECK(processed BETWEEN 0 AND 9007199254740991),
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 100),
 next_attempt_at timestamptz, failure text, operation_id text,
 worker_epoch bigint NOT NULL DEFAULT 0 CHECK(worker_epoch>=0),
 server_epoch bigint NOT NULL DEFAULT 0 CHECK(server_epoch>=0), worker_expires_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(principal_id,key), UNIQUE(root_id,id),
 FOREIGN KEY(root_id,operation_id) REFERENCES operations(root_id,id)
);
CREATE INDEX maintenance_ready ON maintenance_jobs(state,next_attempt_at,id);
CREATE INDEX maintenance_root ON maintenance_jobs(root_id,id);
CREATE TABLE integrity_findings (
 id text PRIMARY KEY, root_id text NOT NULL REFERENCES roots(id), job_id text NOT NULL,
 backend_id text REFERENCES backends(id), digest text NOT NULL CHECK(digest ~ '^[0-9a-f]{64}$'),
 generation text, kind text NOT NULL CHECK(kind IN ('missing','corrupt','inaccessible','catalog_inconsistent')),
 repaired boolean NOT NULL DEFAULT false, created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(root_id,job_id) REFERENCES maintenance_jobs(root_id,id),
 FOREIGN KEY(root_id,backend_id) REFERENCES root_backends(root_id,backend_id),
 UNIQUE NULLS NOT DISTINCT(job_id,backend_id,digest,generation,kind)
);
CREATE INDEX integrity_findings_root ON integrity_findings(root_id,id);

CREATE TABLE checkpoints (
 id text PRIMARY KEY, principal_id text NOT NULL REFERENCES principals(id), key text NOT NULL,
 revision text NOT NULL,
 state text NOT NULL DEFAULT 'fencing' CHECK(state IN ('fencing','exporting','exported','failed','released')),
 generation_watermark bigint NOT NULL DEFAULT 0 CHECK(generation_watermark BETWEEN 0 AND 9007199254740991),
 pinned_bytes bigint NOT NULL DEFAULT 0 CHECK(pinned_bytes BETWEEN 0 AND 9007199254740991),
 failure text, descriptor jsonb, inventory_cursor bigint NOT NULL DEFAULT 0 CHECK(inventory_cursor>=0),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(), UNIQUE(principal_id,key)
);
-- Singleton durable barrier, activated only by the checkpoint executor after
-- checking leases and outstanding destructive I/O. No barrier exists on upgrade.
CREATE TABLE checkpoint_barrier (
 singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
 checkpoint_id text NOT NULL UNIQUE REFERENCES checkpoints(id),
 server_epoch bigint NOT NULL CHECK(server_epoch>0)
);
-- Exact generation inventory survives release for inspection. Active checkpoint
-- state decides whether these rows prohibit deletion; export failure retains pins.
CREATE TABLE checkpoint_objects (
 checkpoint_id text NOT NULL REFERENCES checkpoints(id),
 generation text NOT NULL REFERENCES object_copies(generation),
 PRIMARY KEY(checkpoint_id,generation)
);
CREATE INDEX checkpoint_generation ON checkpoint_objects(generation,checkpoint_id);
