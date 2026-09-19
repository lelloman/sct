CREATE TABLE content_objects (
 root_id text NOT NULL REFERENCES roots(id), digest text NOT NULL CHECK(digest ~ '^[0-9a-f]{64}$'),
 kind text NOT NULL CHECK(kind IN ('chunk','page','file')), length bigint NOT NULL CHECK(length BETWEEN 0 AND 9007199254740991),
 body bytea CHECK(octet_length(body)<=262144), height integer CHECK(height BETWEEN 0 AND 8),
 PRIMARY KEY(root_id,digest), CHECK(kind<>'chunk' OR (body IS NULL AND length BETWEEN 1 AND 4194304))
);
CREATE TABLE content_edges (
 root_id text NOT NULL, parent text NOT NULL, ordinal integer NOT NULL CHECK(ordinal BETWEEN 0 AND 1023),
 child text NOT NULL, logical_length bigint NOT NULL CHECK(logical_length BETWEEN 0 AND 9007199254740991),
 chunk_count bigint NOT NULL CHECK(chunk_count BETWEEN 0 AND 9007199254740991),
 PRIMARY KEY(root_id,parent,ordinal),
 FOREIGN KEY(root_id,parent) REFERENCES content_objects(root_id,digest),
 FOREIGN KEY(root_id,child) REFERENCES content_objects(root_id,digest), CHECK(parent<>child)
);
CREATE INDEX content_incoming ON content_edges(root_id,child);
CREATE TABLE file_versions (
 id text PRIMARY KEY, root_id text NOT NULL, entry_id text NOT NULL, manifest text NOT NULL,
 sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE CHECK(sequence<=9007199254740991),
 length bigint NOT NULL CHECK(length BETWEEN 0 AND 9007199254740991),
 retired boolean NOT NULL DEFAULT false,
 UNIQUE(root_id,id), FOREIGN KEY(root_id,entry_id) REFERENCES entries(root_id,id),
 FOREIGN KEY(root_id,manifest) REFERENCES content_objects(root_id,digest)
);
CREATE INDEX file_history ON file_versions(root_id,entry_id,sequence DESC) WHERE NOT retired;
ALTER TABLE entries ADD CONSTRAINT current_version_root FOREIGN KEY(root_id,current_version_id) REFERENCES file_versions(root_id,id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE transfer_sessions (
 id text PRIMARY KEY, root_id text NOT NULL, operation_id text NOT NULL UNIQUE,
 principal_id text NOT NULL REFERENCES principals(id), key text NOT NULL, request jsonb NOT NULL,
 manifest text NOT NULL, state text NOT NULL DEFAULT 'open' CHECK(state IN ('open','ready','committed','aborted','expired')),
 admission text NOT NULL DEFAULT 'planning' CHECK(admission IN ('planning','pending','admitted','rejected')),
 expires_at timestamptz NOT NULL DEFAULT(clock_timestamp()+interval '24 hours'),
 failure text, receipt jsonb, cursor text,
 UNIQUE(root_id,id), UNIQUE(principal_id,root_id,key),
 FOREIGN KEY(root_id,operation_id) REFERENCES operations(root_id,id),
 FOREIGN KEY(root_id,manifest) REFERENCES content_objects(root_id,digest),
 CHECK((state='committed')=(receipt IS NOT NULL))
);
CREATE INDEX session_expiry ON transfer_sessions(expires_at) WHERE state IN ('open','ready');
-- One session set, not one flattened set for each retained version. Rebuilt by
-- bounded graph traversal and pinned until session terminal cleanup completes.
CREATE TABLE session_objects (
 root_id text NOT NULL, session_id text NOT NULL, digest text NOT NULL,
 PRIMARY KEY(root_id,session_id,digest),
 FOREIGN KEY(root_id,session_id) REFERENCES transfer_sessions(root_id,id),
 FOREIGN KEY(root_id,digest) REFERENCES content_objects(root_id,digest)
);
CREATE TABLE read_pins (
 id text PRIMARY KEY, root_id text NOT NULL, version_id text NOT NULL,
 principal_id text NOT NULL REFERENCES principals(id), expires_at timestamptz NOT NULL,
 FOREIGN KEY(root_id,version_id) REFERENCES file_versions(root_id,id)
);
CREATE INDEX pins_version ON read_pins(root_id,version_id,expires_at);
CREATE TABLE object_copies (
 generation text PRIMARY KEY, root_id text NOT NULL, digest text NOT NULL,
 backend_id text NOT NULL, allocation_id text NOT NULL REFERENCES allocations(id),
 operation_id text NOT NULL, bytes bigint NOT NULL CHECK(bytes BETWEEN 0 AND 9007199254740991),
 state text NOT NULL CHECK(state IN ('writing','available','deleting','deleted')),
 worker_epoch bigint NOT NULL, server_epoch bigint NOT NULL,
 -- An unresolved issued call must never be mistaken for unused reservation.
 uncertain boolean NOT NULL DEFAULT true,
 FOREIGN KEY(root_id,digest) REFERENCES content_objects(root_id,digest),
 FOREIGN KEY(root_id,backend_id) REFERENCES root_backends(root_id,backend_id),
 FOREIGN KEY(root_id,operation_id) REFERENCES operations(root_id,id)
);
CREATE UNIQUE INDEX copy_current ON object_copies(root_id,digest,backend_id) WHERE state IN ('writing','available');
CREATE INDEX copy_cleanup ON object_copies(operation_id,state,generation);
CREATE TABLE transfer_audit (
 sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 root_id text NOT NULL REFERENCES roots(id), operation_id text NOT NULL,
 event text NOT NULL, details jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(root_id,operation_id) REFERENCES operations(root_id,id)
);
-- Copy generations and immutable graph metadata are append-only identities.
CREATE FUNCTION protect_content_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.root_id<>OLD.root_id OR NEW.digest<>OLD.digest OR NEW.kind<>OLD.kind OR NEW.length<>OLD.length
 OR (OLD.body IS NOT NULL AND NEW.body IS DISTINCT FROM OLD.body) OR NEW.height IS DISTINCT FROM OLD.height THEN
 RAISE EXCEPTION 'immutable content identity' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_content BEFORE UPDATE ON content_objects FOR EACH ROW EXECUTE FUNCTION protect_content_identity();
CREATE FUNCTION check_copy_allocation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM allocations a JOIN backends b ON b.pool_id=a.pool_id WHERE a.id=NEW.allocation_id AND b.id=NEW.backend_id AND (a.root_id IS NULL OR a.root_id=NEW.root_id)) THEN
 RAISE EXCEPTION 'copy allocation mismatch' USING ERRCODE='23514'; END IF;
 IF TG_OP='UPDATE' AND (NEW.generation<>OLD.generation OR NEW.root_id<>OLD.root_id OR NEW.digest<>OLD.digest OR NEW.backend_id<>OLD.backend_id OR NEW.bytes<>OLD.bytes OR (OLD.state='deleted' AND NEW.state<>'deleted')) THEN
 RAISE EXCEPTION 'copy generation is immutable' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER copy_allocation BEFORE INSERT OR UPDATE ON object_copies FOR EACH ROW EXECUTE FUNCTION check_copy_allocation();
