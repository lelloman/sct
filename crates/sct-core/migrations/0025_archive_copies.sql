ALTER TABLE archive_copies ADD COLUMN location text NOT NULL DEFAULT '' CHECK(octet_length(location)<=1024);
CREATE TABLE archive_copy_events (
 id text PRIMARY KEY, copy_id text NOT NULL REFERENCES archive_copies(id),
 principal_id text NOT NULL REFERENCES principals(id),
 kind text NOT NULL CHECK(kind IN ('registered','updated','read_back')),
 observation jsonb NOT NULL, received_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX archive_copy_event_copy ON archive_copy_events(copy_id,id);
CREATE TRIGGER archive_copy_events_immutable BEFORE UPDATE OR DELETE ON archive_copy_events FOR EACH ROW EXECUTE FUNCTION archive_immutable_history();
CREATE TABLE archive_verification_queue (
 collection_id text PRIMARY KEY REFERENCES archive_collections(id),
 next_attempt_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 after_sequence bigint NOT NULL DEFAULT 0 CHECK(after_sequence BETWEEN 0 AND 1000000)
);
CREATE INDEX archive_disc_set ON archive_discs(set_id,id);
CREATE INDEX archive_placement_required_disc ON archive_placements(set_id,disc_id);
