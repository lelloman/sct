ALTER TABLE entries ADD COLUMN purged boolean NOT NULL DEFAULT false;
CREATE INDEX purge_versions ON file_versions(root_id,entry_id,id) WHERE NOT retired;
CREATE FUNCTION entry_purged(target text) RETURNS boolean LANGUAGE sql STABLE AS $$
 WITH RECURSIVE a AS (
 SELECT id,parent_id,purged FROM entries WHERE id=target
 UNION ALL SELECT e.id,e.parent_id,e.purged FROM entries e JOIN a ON e.id=a.parent_id
 ) SELECT COALESCE(bool_or(purged),true) FROM a
$$;
