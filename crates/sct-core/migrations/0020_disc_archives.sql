-- Optical archive history has no cascade/FK dependency on retained online versions.
CREATE TABLE archive_accounts (
 principal_id text PRIMARY KEY REFERENCES principals(id),
 quota_bytes bigint NOT NULL DEFAULT 107374182400 CHECK(quota_bytes BETWEEN 0 AND 9007199254740991),
 cache_bytes bigint NOT NULL DEFAULT 0 CHECK(cache_bytes>=0),
 reserved_bytes bigint NOT NULL DEFAULT 0 CHECK(reserved_bytes>=0),
 CHECK(cache_bytes+reserved_bytes<=quota_bytes)
);
CREATE TABLE archive_collections (
 id text PRIMARY KEY, owner_id text NOT NULL REFERENCES principals(id),
 label text NOT NULL, revision text NOT NULL,
 media_capacity bigint NOT NULL CHECK(media_capacity BETWEEN 1 AND 9007199254740991),
 next_disc bigint NOT NULL DEFAULT 1 CHECK(next_disc BETWEEN 1 AND 1000001),
 next_set bigint NOT NULL DEFAULT 1 CHECK(next_set BETWEEN 1 AND 1000001),
 reconciliation_required boolean NOT NULL DEFAULT false,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX archive_collection_owner ON archive_collections(owner_id,id);
CREATE TABLE archive_sets (
 id text PRIMARY KEY, collection_id text NOT NULL REFERENCES archive_collections(id),
 sequence bigint NOT NULL CHECK(sequence BETWEEN 1 AND 1000000), revision text NOT NULL,
 state text NOT NULL CHECK(state IN ('capturing','planning','awaiting_confirmation','building','ready','failed','cancelled','expired','revoked')),
 principal_id text NOT NULL REFERENCES principals(id), credential_id text NOT NULL,
 selections jsonb NOT NULL CHECK(jsonb_typeof(selections)='array' AND jsonb_array_length(selections) BETWEEN 1 AND 1024),
 media_capacity bigint NOT NULL CHECK(media_capacity BETWEEN 1 AND 9007199254740991),
 processed_entries bigint NOT NULL DEFAULT 0 CHECK(processed_entries>=0),
 processed_bytes bigint NOT NULL DEFAULT 0 CHECK(processed_bytes>=0),
 plan jsonb, failure text, first_disc bigint, last_disc bigint,
 frozen_at timestamptz, published_at timestamptz, verified_complete_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 expires_at timestamptz NOT NULL DEFAULT clock_timestamp()+interval '24 hours',
 CHECK((first_disc IS NULL)=(last_disc IS NULL)),
 CHECK(first_disc BETWEEN 1 AND 1000000 AND last_disc BETWEEN first_disc AND 1000000),
 CHECK((state='ready')=(published_at IS NOT NULL)),
 CHECK(published_at IS NULL OR frozen_at IS NOT NULL),
 CHECK(frozen_at IS NULL OR (plan IS NOT NULL AND first_disc IS NOT NULL)),
 CHECK(verified_complete_at IS NULL OR state='ready'),
 UNIQUE(collection_id,sequence), UNIQUE(collection_id,id)
);
CREATE UNIQUE INDEX archive_single_append ON archive_sets(collection_id)
 WHERE state IN ('capturing','planning','awaiting_confirmation','building');
CREATE TABLE archive_sources (
 set_id text NOT NULL REFERENCES archive_sets(id), ordinal integer NOT NULL CHECK(ordinal BETWEEN 0 AND 1023),
 root_id text NOT NULL, version_id text NOT NULL, path text NOT NULL,
 manifest text, kind text CHECK(kind IN ('file','directory')), pin_id text,
 PRIMARY KEY(set_id,ordinal)
);
CREATE TABLE archive_jobs (
 set_id text PRIMARY KEY REFERENCES archive_sets(id),
 phase text NOT NULL DEFAULT 'capture', cursor jsonb NOT NULL DEFAULT '{}',
 worker_epoch bigint NOT NULL DEFAULT 0 CHECK(worker_epoch>=0),
 server_epoch bigint NOT NULL DEFAULT 0 CHECK(server_epoch>=0),
 worker_expires_at timestamptz, next_attempt_at timestamptz,
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts>=0)
);
CREATE TABLE archive_frontier (
 set_id text NOT NULL REFERENCES archive_sets(id), path text NOT NULL,
 root_id text NOT NULL, version_id text NOT NULL, manifest text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('file','directory')), expanded boolean NOT NULL DEFAULT false,
 PRIMARY KEY(set_id,path)
);
CREATE INDEX archive_frontier_pending ON archive_frontier(set_id,path) WHERE NOT expanded;
CREATE TABLE archive_entries (
 set_id text NOT NULL REFERENCES archive_sets(id), path text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('file','directory')), root_id text, version_id text,
 length bigint NOT NULL CHECK(length BETWEEN 0 AND 9007199254740991),
 PRIMARY KEY(set_id,path),
 CHECK(kind='directory' OR (root_id IS NOT NULL AND version_id IS NOT NULL)),
 CHECK(kind='file' OR length=0)
);
CREATE TABLE archive_objects (
 set_id text NOT NULL REFERENCES archive_sets(id), root_id text NOT NULL,
 digest text NOT NULL CHECK(digest ~ '^[0-9a-f]{64}$'),
 length bigint NOT NULL CHECK(length BETWEEN 1 AND 4194304),
 PRIMARY KEY(set_id,root_id,digest), UNIQUE(set_id,root_id,digest,length)
);
CREATE TABLE archive_extents (
 set_id text NOT NULL, path text NOT NULL, offset_bytes bigint NOT NULL CHECK(offset_bytes>=0),
 root_id text NOT NULL, digest text NOT NULL, length bigint NOT NULL,
 PRIMARY KEY(set_id,path,offset_bytes),
 FOREIGN KEY(set_id,path) REFERENCES archive_entries(set_id,path),
 FOREIGN KEY(set_id,root_id,digest,length) REFERENCES archive_objects(set_id,root_id,digest,length)
);
CREATE TABLE archive_discs (
 id text PRIMARY KEY, collection_id text NOT NULL, set_id text NOT NULL,
 number bigint NOT NULL CHECK(number BETWEEN 1 AND 1000000),
 index_sha256 text NOT NULL CHECK(index_sha256 ~ '^[0-9a-f]{64}$'),
 bundle_sha256 text NOT NULL CHECK(bundle_sha256 ~ '^[0-9a-f]{64}$'),
 inventory_sha256 text NOT NULL CHECK(inventory_sha256 ~ '^[0-9a-f]{64}$'),
 payload_bytes bigint NOT NULL CHECK(payload_bytes BETWEEN 0 AND 9007199254740991),
 object_count bigint NOT NULL CHECK(object_count>=0),
 planned_iso_bytes bigint NOT NULL CHECK(planned_iso_bytes BETWEEN 1 AND 9007199254740991),
 UNIQUE(collection_id,number), UNIQUE(collection_id,id),
 FOREIGN KEY(collection_id,set_id) REFERENCES archive_sets(collection_id,id)
);
CREATE TABLE archive_placements (
 set_id text NOT NULL, collection_id text NOT NULL, root_id text NOT NULL,
 digest text NOT NULL, length bigint NOT NULL, disc_id text NOT NULL,
 PRIMARY KEY(set_id,root_id,digest,disc_id),
 FOREIGN KEY(set_id,root_id,digest,length) REFERENCES archive_objects(set_id,root_id,digest,length),
 FOREIGN KEY(collection_id,set_id) REFERENCES archive_sets(collection_id,id),
 FOREIGN KEY(collection_id,disc_id) REFERENCES archive_discs(collection_id,id)
);
CREATE INDEX archive_placement_disc ON archive_placements(disc_id,set_id,root_id,digest);
CREATE TABLE archive_artifacts (
 id text PRIMARY KEY, disc_id text NOT NULL UNIQUE REFERENCES archive_discs(id),
 state text NOT NULL CHECK(state IN ('building','available','expired','failed')),
 sha256 text CHECK(sha256 ~ '^[0-9a-f]{64}$'), length bigint CHECK(length BETWEEN 1 AND 9007199254740991),
 relative_path text, expires_at timestamptz NOT NULL,
 download_count bigint NOT NULL DEFAULT 0 CHECK(download_count>=0),
 CHECK((sha256 IS NULL)=(length IS NULL)),
 CHECK(state<>'available' OR (sha256 IS NOT NULL AND relative_path IS NOT NULL))
);
CREATE TABLE archive_copies (
 id text PRIMARY KEY, disc_id text NOT NULL REFERENCES archive_discs(id),
 label text NOT NULL, revision text NOT NULL, reported_burned_at timestamptz,
 health text NOT NULL DEFAULT 'unknown' CHECK(health IN ('unknown','read_back_verified','damaged','missing')),
 latest_report_id text, created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX archive_copy_disc ON archive_copies(disc_id,id);
CREATE TABLE archive_reports (
 id text PRIMARY KEY, copy_id text NOT NULL REFERENCES archive_copies(id),
 principal_id text NOT NULL REFERENCES principals(id), observation jsonb NOT NULL,
 complete_read_back boolean NOT NULL,
 received_at timestamptz NOT NULL DEFAULT clock_timestamp(), UNIQUE(copy_id,id)
);
ALTER TABLE archive_copies ADD CONSTRAINT archive_latest_report
 FOREIGN KEY(id,latest_report_id) REFERENCES archive_reports(copy_id,id) DEFERRABLE INITIALLY DEFERRED;
CREATE INDEX archive_report_copy ON archive_reports(copy_id,id);

CREATE FUNCTION archive_immutable_history() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 RAISE EXCEPTION 'archive history is immutable' USING ERRCODE='23514';
END $$;
CREATE TRIGGER archive_reports_immutable BEFORE UPDATE OR DELETE ON archive_reports FOR EACH ROW EXECUTE FUNCTION archive_immutable_history();
CREATE TRIGGER archive_collections_preserved BEFORE DELETE ON archive_collections FOR EACH ROW EXECUTE FUNCTION archive_immutable_history();
CREATE TRIGGER archive_sets_preserved BEFORE DELETE ON archive_sets FOR EACH ROW EXECUTE FUNCTION archive_immutable_history();
CREATE TRIGGER archive_discs_preserved BEFORE DELETE ON archive_discs FOR EACH ROW EXECUTE FUNCTION archive_immutable_history();
CREATE TRIGGER archive_copies_preserved BEFORE DELETE ON archive_copies FOR EACH ROW EXECUTE FUNCTION archive_immutable_history();
CREATE TRIGGER archive_artifacts_preserved BEFORE DELETE ON archive_artifacts FOR EACH ROW EXECUTE FUNCTION archive_immutable_history();

CREATE FUNCTION archive_guard_set() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.collection_id,NEW.sequence,NEW.principal_id,NEW.credential_id,NEW.selections,NEW.media_capacity,NEW.created_at)
    IS DISTINCT FROM (OLD.id,OLD.collection_id,OLD.sequence,OLD.principal_id,OLD.credential_id,OLD.selections,OLD.media_capacity,OLD.created_at)
 OR (OLD.frozen_at IS NOT NULL AND (NEW.plan,NEW.first_disc,NEW.last_disc,NEW.frozen_at) IS DISTINCT FROM (OLD.plan,OLD.first_disc,OLD.last_disc,OLD.frozen_at))
 OR (OLD.published_at IS NOT NULL AND NEW.published_at IS DISTINCT FROM OLD.published_at)
 OR (OLD.verified_complete_at IS NOT NULL AND NEW.verified_complete_at IS DISTINCT FROM OLD.verified_complete_at)
 OR (NEW.state<>OLD.state AND NOT (
    (OLD.state='capturing' AND NEW.state='planning') OR
    (OLD.state='planning' AND NEW.state='awaiting_confirmation') OR
    (OLD.state='awaiting_confirmation' AND NEW.state='building') OR
    (OLD.state='building' AND NEW.state='ready') OR
    (OLD.state IN ('capturing','planning','awaiting_confirmation','building') AND NEW.state IN ('failed','cancelled','expired','revoked'))))
 THEN RAISE EXCEPTION 'immutable archive set or invalid transition' USING ERRCODE='23514'; END IF;
 IF NEW.state='ready' AND OLD.state<>'ready' THEN
   IF (SELECT count(*) FROM archive_discs WHERE set_id=NEW.id)<>NEW.last_disc-NEW.first_disc+1
      OR EXISTS(SELECT 1 FROM archive_discs d LEFT JOIN archive_artifacts a ON a.disc_id=d.id WHERE d.set_id=NEW.id AND (a.id IS NULL OR a.state<>'available'))
   THEN RAISE EXCEPTION 'incomplete archive publication' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER archive_set_guard BEFORE UPDATE ON archive_sets FOR EACH ROW EXECUTE FUNCTION archive_guard_set();

CREATE FUNCTION archive_guard_content() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
 IF current_setting('sct.archive_import',true)='on' THEN RETURN NEW; END IF;
 target=CASE WHEN TG_OP='DELETE' THEN OLD.set_id ELSE NEW.set_id END;
 IF TG_OP='UPDATE' AND NEW.set_id<>OLD.set_id THEN
   RAISE EXCEPTION 'cannot move archive content' USING ERRCODE='23514';
 END IF;
 PERFORM 1 FROM archive_sets WHERE id=target AND frozen_at IS NULL FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'frozen archive content' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='archive_discs' THEN
   PERFORM 1 FROM archive_sets WHERE id=NEW.set_id AND NEW.number BETWEEN first_disc AND last_disc;
   IF NOT FOUND THEN RAISE EXCEPTION 'disc outside reserved interval' USING ERRCODE='23514'; END IF;
 END IF;
 IF TG_TABLE_NAME='archive_placements' THEN
   PERFORM 1 FROM archive_discs d JOIN archive_sets prior ON prior.id=d.set_id JOIN archive_sets target_set ON target_set.id=target
     WHERE d.id=CASE WHEN TG_OP='DELETE' THEN OLD.disc_id ELSE NEW.disc_id END AND prior.sequence<=target_set.sequence;
   IF NOT FOUND THEN RAISE EXCEPTION 'future archive dependency' USING ERRCODE='23514'; END IF;
 END IF;
 IF TG_OP='DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
END $$;
CREATE TRIGGER archive_source_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_sources FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_entry_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_entries FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_object_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_objects FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_extent_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_extents FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_disc_guard BEFORE INSERT OR UPDATE ON archive_discs FOR EACH ROW EXECUTE FUNCTION archive_guard_content();
CREATE TRIGGER archive_placement_guard BEFORE INSERT OR UPDATE OR DELETE ON archive_placements FOR EACH ROW EXECUTE FUNCTION archive_guard_content();

CREATE FUNCTION archive_guard_artifact() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.disc_id) IS DISTINCT FROM (OLD.id,OLD.disc_id)
 OR (OLD.sha256 IS NOT NULL AND (NEW.sha256,NEW.length) IS DISTINCT FROM (OLD.sha256,OLD.length))
 OR (OLD.state='expired' AND NEW.state<>'expired')
 OR (OLD.state='available' AND NEW.state NOT IN ('available','expired'))
 THEN RAISE EXCEPTION 'immutable archive artifact' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER archive_artifact_guard BEFORE UPDATE ON archive_artifacts FOR EACH ROW EXECUTE FUNCTION archive_guard_artifact();

CREATE FUNCTION archive_guard_collection() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.owner_id,NEW.created_at) IS DISTINCT FROM (OLD.id,OLD.owner_id,OLD.created_at)
 OR NEW.next_disc<OLD.next_disc OR NEW.next_set<OLD.next_set
 THEN RAISE EXCEPTION 'permanent archive identity/high-water mark' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER archive_collection_guard BEFORE UPDATE ON archive_collections FOR EACH ROW EXECUTE FUNCTION archive_guard_collection();
CREATE FUNCTION archive_guard_copy() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.disc_id,NEW.created_at) IS DISTINCT FROM (OLD.id,OLD.disc_id,OLD.created_at)
 THEN RAISE EXCEPTION 'permanent physical copy identity' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER archive_copy_guard BEFORE UPDATE ON archive_copies FOR EACH ROW EXECUTE FUNCTION archive_guard_copy();
