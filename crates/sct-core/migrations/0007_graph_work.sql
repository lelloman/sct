ALTER TABLE roots ADD COLUMN graph_serial bigint NOT NULL DEFAULT 0 CHECK(graph_serial>=0);
-- Internal extension point: M4 can add checkpoint roots without teaching the
-- collector a second graph representation. No checkpoint API is exposed here.
CREATE TABLE graph_pins (
 root_id text NOT NULL, id text NOT NULL, kind text NOT NULL CHECK(kind IN ('checkpoint','job')),
 manifest text NOT NULL, expires_at timestamptz,
 PRIMARY KEY(root_id,id), FOREIGN KEY(root_id,manifest) REFERENCES content_objects(root_id,digest)
);
CREATE FUNCTION bump_graph_generation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 UPDATE roots SET graph_serial=graph_serial+1 WHERE id=COALESCE(NEW.root_id,OLD.root_id);
 RETURN NULL;
END $$;
CREATE TRIGGER edge_generation AFTER INSERT OR UPDATE OR DELETE ON content_edges FOR EACH ROW EXECUTE FUNCTION bump_graph_generation();
CREATE TRIGGER session_generation AFTER INSERT OR UPDATE OF state,admission,manifest OR DELETE ON transfer_sessions FOR EACH ROW EXECUTE FUNCTION bump_graph_generation();
CREATE TRIGGER draft_generation AFTER INSERT OR UPDATE OF state,phase,manifest OR DELETE ON tree_drafts FOR EACH ROW EXECUTE FUNCTION bump_graph_generation();
CREATE TRIGGER draft_entry_generation AFTER INSERT OR UPDATE OF manifest,removed OR DELETE ON draft_entries FOR EACH ROW EXECUTE FUNCTION bump_graph_generation();
CREATE TRIGGER pin_generation AFTER INSERT OR UPDATE OR DELETE ON graph_pins FOR EACH ROW EXECUTE FUNCTION bump_graph_generation();
CREATE TABLE graph_scans (
 root_id text PRIMARY KEY REFERENCES roots(id), operation_id text NOT NULL, scan_id text NOT NULL,
 mutation_serial bigint NOT NULL, graph_serial bigint NOT NULL,
 source integer NOT NULL DEFAULT 0 CHECK(source BETWEEN 0 AND 6), after_key text NOT NULL DEFAULT '',
 phase text NOT NULL DEFAULT 'seeds' CHECK(phase IN ('seeds','expand','sweep')),
 FOREIGN KEY(root_id,operation_id) REFERENCES operations(root_id,id)
);
-- One reusable reachability mark per root/object, not a flattened set per version.
CREATE TABLE graph_marks (
 root_id text NOT NULL, digest text NOT NULL, scan_id text NOT NULL, expanded boolean NOT NULL DEFAULT false,
 PRIMARY KEY(root_id,digest), FOREIGN KEY(root_id,digest) REFERENCES content_objects(root_id,digest)
);
CREATE INDEX graph_frontier ON graph_marks(root_id,scan_id,digest) WHERE NOT expanded;
CREATE INDEX graph_scan_members ON graph_marks(root_id,scan_id,digest);
CREATE INDEX copy_sweep ON object_copies(root_id,state,generation);
