-- Private resumable planning workspace; portable history remains in migration 20.
CREATE TABLE archive_plan_state (
 set_id text PRIMARY KEY REFERENCES archive_sets(id),
 bundle jsonb NOT NULL, cursor jsonb NOT NULL
);
CREATE TABLE archive_plan_objects (
 set_id text NOT NULL REFERENCES archive_sets(id), root_id text NOT NULL,
 digest text NOT NULL, length bigint NOT NULL,
 ordinal bigint NOT NULL CHECK(ordinal>=0), old_disc text REFERENCES archive_discs(id),
 new_disc bigint,
 PRIMARY KEY(set_id,root_id,digest), UNIQUE(set_id,ordinal),
 FOREIGN KEY(set_id,root_id,digest,length) REFERENCES archive_objects(set_id,root_id,digest,length)
);
CREATE TABLE archive_plan_discs (
 set_id text NOT NULL REFERENCES archive_sets(id), number bigint NOT NULL,
 payload_bytes bigint NOT NULL DEFAULT 0, object_count bigint NOT NULL DEFAULT 0,
 occupied_bytes bigint NOT NULL DEFAULT 0,
 PRIMARY KEY(set_id,number)
);
CREATE TABLE archive_index_records (
 set_id text NOT NULL REFERENCES archive_sets(id), phase integer NOT NULL CHECK(phase BETWEEN 0 AND 4),
 key1 text COLLATE "C" NOT NULL, key2 text COLLATE "C" NOT NULL DEFAULT '',
 position bigint NOT NULL DEFAULT 0, line bytea NOT NULL CHECK(octet_length(line) BETWEEN 2 AND 65537),
 PRIMARY KEY(set_id,phase,key1,key2,position)
);
CREATE TRIGGER archive_index_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_index_records FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_plan_state_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_plan_state FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_plan_object_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_plan_objects FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_plan_disc_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_plan_discs FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
