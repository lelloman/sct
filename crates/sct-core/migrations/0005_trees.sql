-- The existing version table becomes a typed content-root table. Its identity,
-- pin and current-pointer foreign keys survive the upgrade unchanged.
ALTER TABLE content_objects DROP CONSTRAINT content_objects_kind_check;
ALTER TABLE content_objects ADD CHECK(kind IN ('chunk','page','file','directory','directory_page'));
ALTER TABLE file_versions ADD COLUMN kind text NOT NULL DEFAULT 'file' CHECK(kind IN ('file','directory'));
ALTER TABLE roots ADD COLUMN mutation_serial bigint NOT NULL DEFAULT 0 CHECK(mutation_serial>=0);
CREATE FUNCTION bump_tree_generation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 UPDATE roots SET mutation_serial=mutation_serial+1 WHERE id=COALESCE(NEW.root_id,OLD.root_id);
 RETURN NULL;
END $$;
CREATE TRIGGER namespace_generation AFTER INSERT OR UPDATE OR DELETE ON entries FOR EACH ROW EXECUTE FUNCTION bump_tree_generation();
CREATE TRIGGER version_generation AFTER INSERT OR UPDATE OR DELETE ON file_versions FOR EACH ROW EXECUTE FUNCTION bump_tree_generation();
CREATE FUNCTION check_version_kind() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM entries e JOIN content_objects o ON o.root_id=e.root_id
 AND o.digest=NEW.manifest WHERE e.root_id=NEW.root_id AND e.id=NEW.entry_id
 AND e.kind=NEW.kind AND o.kind=NEW.kind) THEN
 RAISE EXCEPTION 'version kind mismatch' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER version_kind BEFORE INSERT OR UPDATE ON file_versions FOR EACH ROW EXECUTE FUNCTION check_version_kind();

CREATE TABLE tree_drafts (
 id text PRIMARY KEY, root_id text NOT NULL, directory_id text NOT NULL,
 operation_id text NOT NULL UNIQUE, principal_id text NOT NULL REFERENCES principals(id),
 key text NOT NULL, request jsonb NOT NULL, base_version_id text, manifest text,
 phase text NOT NULL DEFAULT 'initializing' CHECK(phase IN ('initializing','editing','building','planning','staging','ready','committed','cleaning_up','terminal')),
 state text NOT NULL DEFAULT 'open' CHECK(state IN ('open','ready','committed','aborted','expired')),
 processed bigint NOT NULL DEFAULT 0 CHECK(processed>=0),
 expires_at timestamptz NOT NULL DEFAULT(clock_timestamp()+interval '24 hours'),
 requires_delete boolean NOT NULL DEFAULT false, receipt jsonb,
 UNIQUE(root_id,id), UNIQUE(principal_id,root_id,key),
 FOREIGN KEY(root_id,directory_id) REFERENCES entries(root_id,id),
 FOREIGN KEY(root_id,operation_id) REFERENCES operations(root_id,id),
 FOREIGN KEY(root_id,base_version_id) REFERENCES file_versions(root_id,id),
 FOREIGN KEY(root_id,manifest) REFERENCES content_objects(root_id,digest),
 CHECK((state='committed')=(receipt IS NOT NULL))
);
CREATE INDEX tree_draft_expiry ON tree_drafts(expires_at) WHERE state IN ('open','ready');
-- Working namespace exists only once per draft, never once per retained version.
-- Parents may arrive later; connectivity/cycles are validated at freeze time.
CREATE TABLE draft_entries (
 root_id text NOT NULL, draft_id text NOT NULL, id text NOT NULL,
 parent_id text, name text COLLATE "C" NOT NULL, kind text NOT NULL CHECK(kind IN ('file','directory')),
 manifest text, origin text NOT NULL CHECK(origin IN ('base','new')),
 removed boolean NOT NULL DEFAULT false, built boolean NOT NULL DEFAULT false,
 PRIMARY KEY(root_id,draft_id,id),
 FOREIGN KEY(root_id,draft_id) REFERENCES tree_drafts(root_id,id),
 FOREIGN KEY(root_id,manifest) REFERENCES content_objects(root_id,digest),
 CHECK(parent_id IS DISTINCT FROM id),
 CHECK(octet_length(name) BETWEEN 1 AND 255 AND name NOT IN ('.','..') AND position('/' in name)=0)
);
CREATE UNIQUE INDEX draft_names ON draft_entries(root_id,draft_id,parent_id,name) WHERE NOT removed;
CREATE INDEX draft_parents ON draft_entries(root_id,draft_id,parent_id,id);
CREATE INDEX draft_build ON draft_entries(root_id,draft_id,built,id) WHERE NOT removed;
CREATE TABLE draft_identities (
 root_id text NOT NULL, draft_id text NOT NULL, id text NOT NULL,
 PRIMARY KEY(root_id,draft_id,id), UNIQUE(id),
 FOREIGN KEY(root_id,draft_id) REFERENCES tree_drafts(root_id,id)
);
CREATE TABLE tree_work (
 root_id text NOT NULL, draft_id text NOT NULL, sequence bigint GENERATED ALWAYS AS IDENTITY,
 kind text NOT NULL, entry_id text, manifest text, position jsonb NOT NULL DEFAULT '{}',
 PRIMARY KEY(root_id,draft_id,sequence),
 FOREIGN KEY(root_id,draft_id) REFERENCES tree_drafts(root_id,id),
 FOREIGN KEY(root_id,manifest) REFERENCES content_objects(root_id,digest)
);
CREATE TABLE change_previews (
 id text PRIMARY KEY, root_id text NOT NULL REFERENCES roots(id),
 principal_id text NOT NULL REFERENCES principals(id), request jsonb NOT NULL,
 root_revision text NOT NULL, mutation_serial bigint NOT NULL,
 state text NOT NULL DEFAULT 'planning' CHECK(state IN ('planning','ready','applying','completed','blocked')),
 expires_at timestamptz NOT NULL DEFAULT(clock_timestamp()+interval '1 hour'),
 cursor text, processed bigint NOT NULL DEFAULT 0 CHECK(processed>=0),
 UNIQUE(root_id,id)
);
CREATE TABLE preview_versions (
 root_id text NOT NULL, preview_id text NOT NULL, version_id text NOT NULL,
 retire boolean NOT NULL, loses_protection boolean NOT NULL,
 PRIMARY KEY(root_id,preview_id,version_id),
 FOREIGN KEY(root_id,preview_id) REFERENCES change_previews(root_id,id),
 FOREIGN KEY(root_id,version_id) REFERENCES file_versions(root_id,id)
);
CREATE TABLE lifecycle_jobs (
 operation_id text PRIMARY KEY, root_id text NOT NULL,
 principal_id text NOT NULL REFERENCES principals(id), credential_id text NOT NULL REFERENCES credentials(id),
 key text NOT NULL, request jsonb NOT NULL, preview_id text,
 state text NOT NULL CHECK(state IN ('planning','ready','applying','cleaning_up','blocked','completed')),
 phase text NOT NULL, cursor text, processed bigint NOT NULL DEFAULT 0 CHECK(processed>=0), failure text,
 UNIQUE(root_id,operation_id), UNIQUE(principal_id,root_id,key),
 FOREIGN KEY(root_id,operation_id) REFERENCES operations(root_id,id),
 FOREIGN KEY(root_id,preview_id) REFERENCES change_previews(root_id,id)
);
-- A durable frontier supports bounded work, including after a new writer epoch.
CREATE TABLE lifecycle_work (
 root_id text NOT NULL, operation_id text NOT NULL, sequence bigint GENERATED ALWAYS AS IDENTITY,
 entry_id text, version_id text, position jsonb NOT NULL DEFAULT '{}',
 PRIMARY KEY(root_id,operation_id,sequence),
 FOREIGN KEY(root_id,operation_id) REFERENCES lifecycle_jobs(root_id,operation_id),
 FOREIGN KEY(root_id,entry_id) REFERENCES entries(root_id,id),
 FOREIGN KEY(root_id,version_id) REFERENCES file_versions(root_id,id)
);
