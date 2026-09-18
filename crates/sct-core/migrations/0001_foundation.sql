-- Catalog metadata only. Payload graphs, copies, read pins and checkpoints extend
-- these root/operation identities in M2–M4; object listings are never authoritative.
CREATE TABLE principals (
 id text PRIMARY KEY, issuer text, subject text, administrator boolean NOT NULL DEFAULT false,
 disabled boolean NOT NULL DEFAULT false,
 CHECK ((issuer IS NULL) = (subject IS NULL)), UNIQUE(issuer, subject)
);
CREATE TABLE credentials (
 id text PRIMARY KEY, principal_id text NOT NULL REFERENCES principals(id),
 secret_hash text NOT NULL UNIQUE, kind text NOT NULL CHECK(kind IN ('token','session')),
 csrf_hash text, expires_at timestamptz, revoked boolean NOT NULL DEFAULT false,
 CHECK ((kind = 'session') = (csrf_hash IS NOT NULL))
);
CREATE TABLE server_lease (
 singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), owner text NOT NULL,
 epoch bigint NOT NULL CHECK(epoch > 0), expires_at timestamptz NOT NULL
);
CREATE TABLE pools (
 id text PRIMARY KEY, revision text NOT NULL, capacity bigint NOT NULL CHECK(capacity BETWEEN 0 AND 9007199254740991)
);
CREATE TABLE backends (
 id text PRIMARY KEY, pool_id text NOT NULL REFERENCES pools(id), kind text NOT NULL,
 destination_identity text NOT NULL UNIQUE, inspection jsonb NOT NULL
);
CREATE TABLE roots (
 id text PRIMARY KEY, owner_id text NOT NULL REFERENCES principals(id), display_name text NOT NULL,
 mount_path text NOT NULL, revision text NOT NULL, state text NOT NULL CHECK(state IN ('disabled','enabled','recovering')),
 placement jsonb NOT NULL, quota bigint NOT NULL CHECK(quota BETWEEN 0 AND 9007199254740991),
 default_policy jsonb NOT NULL, policy_id text NOT NULL UNIQUE, policy_revision text NOT NULL,
 min_versions bigint NOT NULL DEFAULT 1 CHECK(min_versions BETWEEN 1 AND 1000000),
 max_versions bigint NOT NULL DEFAULT 1000000 CHECK(max_versions BETWEEN min_versions AND 1000000),
 allow_relaxed boolean NOT NULL DEFAULT true, populated boolean NOT NULL DEFAULT false,
 UNIQUE(owner_id,mount_path)
);
CREATE TABLE root_backends (
 root_id text NOT NULL REFERENCES roots(id), backend_id text NOT NULL REFERENCES backends(id),
 PRIMARY KEY(root_id, backend_id)
);
CREATE TABLE grants (
 id text PRIMARY KEY, root_id text NOT NULL REFERENCES roots(id), principal_id text NOT NULL REFERENCES principals(id),
 revision text NOT NULL, capabilities jsonb NOT NULL CHECK(jsonb_typeof(capabilities) = 'array'), UNIQUE(root_id,principal_id)
);
CREATE INDEX grants_principal_root ON grants(principal_id,root_id);
CREATE TABLE allocations (
 id text PRIMARY KEY, pool_id text NOT NULL REFERENCES pools(id), root_id text REFERENCES roots(id),
 capacity bigint NOT NULL CHECK(capacity BETWEEN 0 AND 9007199254740991),
 reserved bigint NOT NULL DEFAULT 0 CHECK(reserved >= 0), occupied bigint NOT NULL DEFAULT 0 CHECK(occupied >= 0),
 pending_cleanup bigint NOT NULL DEFAULT 0 CHECK(pending_cleanup BETWEEN 0 AND occupied),
 CHECK(reserved + occupied <= capacity), UNIQUE NULLS NOT DISTINCT(pool_id,root_id)
);
-- Serialize all allocation/resize changes on the parent pool, including direct SQL.
CREATE FUNCTION check_allocation_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE available bigint; allocated numeric;
BEGIN
 IF TG_OP = 'UPDATE' AND (NEW.pool_id <> OLD.pool_id OR NEW.root_id IS DISTINCT FROM OLD.root_id) THEN
   RAISE EXCEPTION 'allocation identity is immutable' USING ERRCODE='23514';
 END IF;
 SELECT capacity INTO available FROM pools WHERE id=NEW.pool_id FOR UPDATE;
 SELECT COALESCE(sum(capacity),0) INTO allocated FROM allocations WHERE pool_id=NEW.pool_id AND id<>NEW.id;
 IF allocated + NEW.capacity > available THEN RAISE EXCEPTION 'pool over-allocation' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER allocation_capacity BEFORE INSERT OR UPDATE ON allocations FOR EACH ROW EXECUTE FUNCTION check_allocation_capacity();
CREATE FUNCTION check_pool_resize() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.capacity < (SELECT COALESCE(sum(capacity),0) FROM allocations WHERE pool_id=NEW.id) THEN
 RAISE EXCEPTION 'pool resize violates allocations' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER pool_resize BEFORE UPDATE ON pools FOR EACH ROW EXECUTE FUNCTION check_pool_resize();
CREATE TABLE entries (
 id text PRIMARY KEY, root_id text NOT NULL REFERENCES roots(id), parent_id text,
 name text COLLATE "C" NOT NULL CHECK(char_length(name) BETWEEN 1 AND 255 AND name NOT IN ('.','..') AND position('/' in name)=0),
 kind text NOT NULL CHECK(kind IN ('directory','file')), revision text NOT NULL,
 hidden boolean NOT NULL DEFAULT false, current_version_id text,
 policy_id text UNIQUE, policy_revision text, policy jsonb,
 versioning text NOT NULL DEFAULT 'independent_files' CHECK(versioning IN ('independent_files','atomic_directory')),
 UNIQUE(root_id,id), FOREIGN KEY(root_id,parent_id) REFERENCES entries(root_id,id),
 CHECK((policy IS NULL) = (policy_id IS NULL)), CHECK((policy IS NULL) = (policy_revision IS NULL)), CHECK(parent_id IS DISTINCT FROM id)
);
CREATE UNIQUE INDEX entry_live_names ON entries(root_id,parent_id,name) WHERE NOT hidden AND parent_id IS NOT NULL;
CREATE UNIQUE INDEX entry_root_directory ON entries(root_id) WHERE parent_id IS NULL;
CREATE INDEX entry_listing ON entries(root_id,parent_id,name,id);
CREATE TABLE operations (
 id text PRIMARY KEY, root_id text NOT NULL REFERENCES roots(id), principal_id text REFERENCES principals(id),
 kind text NOT NULL, state text NOT NULL CHECK(state IN ('active','recovering','cleaning_up','completed','aborted','failed')),
 phase text NOT NULL DEFAULT 'metadata', epoch bigint NOT NULL DEFAULT 1 CHECK(epoch>0),
 worker_expires_at timestamptz, server_epoch bigint NOT NULL,
 -- Later physical I/O must persist never-reused incarnation IDs before issuing I/O.
 uncertain_io boolean NOT NULL DEFAULT false, UNIQUE(root_id,id)
);
CREATE UNIQUE INDEX operation_root_owner ON operations(root_id) WHERE state IN ('active','recovering','cleaning_up');
CREATE TABLE reservations (
 id text PRIMARY KEY, allocation_id text NOT NULL REFERENCES allocations(id), operation_id text NOT NULL REFERENCES operations(id),
 bytes bigint NOT NULL CHECK(bytes BETWEEN 0 AND 9007199254740991), UNIQUE(allocation_id,operation_id)
);
CREATE TABLE idempotency (
 principal_id text NOT NULL REFERENCES principals(id), scope text NOT NULL, key text NOT NULL,
 request jsonb NOT NULL, response jsonb NOT NULL, PRIMARY KEY(principal_id,scope,key)
);
CREATE TABLE cursors (
 token_hash text PRIMARY KEY, principal_id text NOT NULL REFERENCES principals(id),
 scope text NOT NULL, binding jsonb NOT NULL, position jsonb NOT NULL,
 expires_at timestamptz NOT NULL DEFAULT (now() + interval '1 hour')
);
CREATE INDEX cursor_expiry ON cursors(expires_at);
CREATE TABLE oidc_flows (
 state_hash text PRIMARY KEY, browser_hash text NOT NULL, nonce text NOT NULL, pkce_verifier text NOT NULL,
 expires_at timestamptz NOT NULL DEFAULT (now()+interval '10 minutes')
);
