CREATE TABLE draft_build_pages (
 root_id text NOT NULL, draft_id text NOT NULL, directory_id text NOT NULL,
 height integer NOT NULL CHECK(height BETWEEN 0 AND 8), first text COLLATE "C" NOT NULL,
 reference jsonb NOT NULL,
 PRIMARY KEY(root_id,draft_id,directory_id,height,first),
 FOREIGN KEY(root_id,draft_id,directory_id) REFERENCES draft_entries(root_id,draft_id,id)
);
CREATE TABLE draft_build_state (
 root_id text NOT NULL, draft_id text NOT NULL, directory_id text NOT NULL,
 height integer NOT NULL DEFAULT 0 CHECK(height BETWEEN 0 AND 8), after_name text COLLATE "C" NOT NULL DEFAULT '',
 PRIMARY KEY(root_id,draft_id),
 FOREIGN KEY(root_id,draft_id,directory_id) REFERENCES draft_entries(root_id,draft_id,id)
);
CREATE INDEX draft_ready_directories ON draft_entries(root_id,draft_id,id) WHERE kind='directory' AND NOT built AND NOT removed;
CREATE INDEX draft_unbuilt_children ON draft_entries(root_id,draft_id,parent_id,id) WHERE kind='directory' AND NOT built AND NOT removed;
-- Idempotent admission polls must not invalidate an in-progress graph scan.
CREATE OR REPLACE FUNCTION bump_graph_generation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='UPDATE' AND NEW IS NOT DISTINCT FROM OLD THEN RETURN NULL; END IF;
 IF TG_OP='UPDATE' THEN
  IF TG_TABLE_NAME='transfer_sessions' THEN
   IF (NEW.state,NEW.admission,NEW.manifest) IS NOT DISTINCT FROM (OLD.state,OLD.admission,OLD.manifest) THEN RETURN NULL; END IF;
  END IF;
  IF TG_TABLE_NAME='tree_drafts' THEN
   IF (NEW.state,NEW.phase,NEW.manifest) IS NOT DISTINCT FROM (OLD.state,OLD.phase,OLD.manifest) THEN RETURN NULL; END IF;
  END IF;
  IF TG_TABLE_NAME='draft_entries' THEN
   IF (NEW.manifest,NEW.removed) IS NOT DISTINCT FROM (OLD.manifest,OLD.removed) THEN RETURN NULL; END IF;
  END IF;
 END IF;
 UPDATE roots SET graph_serial=graph_serial+1 WHERE id=COALESCE(NEW.root_id,OLD.root_id);
 RETURN NULL;
END $$;
CREATE FUNCTION sync_tree_session() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 UPDATE tree_drafts SET state=CASE WHEN NEW.admission='rejected' THEN 'aborted' ELSE NEW.state END,receipt=NEW.receipt,expires_at=NEW.expires_at,
 phase=CASE WHEN NEW.state='committed' THEN 'committed'
 WHEN NEW.state IN ('aborted','expired') OR NEW.admission='rejected' THEN 'terminal'
 WHEN NEW.state='ready' THEN 'ready' WHEN NEW.admission='admitted' THEN 'staging' ELSE 'planning' END
 WHERE id=NEW.id;
 RETURN NULL;
END $$;
CREATE TRIGGER tree_session_state AFTER UPDATE ON transfer_sessions FOR EACH ROW EXECUTE FUNCTION sync_tree_session();
