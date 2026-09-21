ALTER TABLE archive_accounts ADD COLUMN revision text NOT NULL DEFAULT md5(random()::text);
ALTER TABLE archive_accounts ADD COLUMN pin_quota_bytes bigint NOT NULL DEFAULT 1099511627776 CHECK(pin_quota_bytes BETWEEN 0 AND 9007199254740991);
ALTER TABLE archive_accounts ADD COLUMN pinned_bytes bigint NOT NULL DEFAULT 0 CHECK(pinned_bytes>=0);
ALTER TABLE archive_accounts ADD CONSTRAINT archive_pin_ceiling CHECK(pinned_bytes<=pin_quota_bytes);
ALTER TABLE archive_accounts ADD COLUMN max_active integer NOT NULL DEFAULT 4 CHECK(max_active BETWEEN 1 AND 64);
ALTER TABLE archive_accounts ADD COLUMN max_capture_records bigint NOT NULL DEFAULT 100000000 CHECK(max_capture_records BETWEEN 1 AND 100000000);
ALTER TABLE archive_accounts ADD COLUMN pin_deadline_seconds integer NOT NULL DEFAULT 86400 CHECK(pin_deadline_seconds BETWEEN 60 AND 604800);
ALTER TABLE archive_sets ADD COLUMN pinned_bytes bigint NOT NULL DEFAULT 0 CHECK(pinned_bytes>=0);
ALTER TABLE archive_sets ADD COLUMN capture_records bigint NOT NULL DEFAULT 0 CHECK(capture_records>=0);
ALTER TABLE archive_sets ADD COLUMN capture_record_limit bigint NOT NULL DEFAULT 100000000 CHECK(capture_record_limit BETWEEN 1 AND 100000000);
CREATE TABLE archive_capture_pages (
 set_id text NOT NULL REFERENCES archive_sets(id), path text NOT NULL,
 root_id text NOT NULL, version_id text NOT NULL, manifest text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('page','directory_page')),
 offset_bytes bigint NOT NULL CHECK(offset_bytes>=0), height integer NOT NULL CHECK(height BETWEEN 0 AND 8),
 expanded boolean NOT NULL DEFAULT false,
 PRIMARY KEY(set_id,path,manifest,offset_bytes)
);
CREATE INDEX archive_capture_pending ON archive_capture_pages(set_id,path,manifest,offset_bytes) WHERE NOT expanded;
ALTER TABLE archive_frontier ADD COLUMN expected_length bigint NOT NULL DEFAULT 0 CHECK(expected_length>=0);
ALTER TABLE archive_frontier ADD COLUMN expected_chunks bigint CHECK(expected_chunks>=0);
ALTER TABLE archive_capture_pages ADD COLUMN expected_length bigint NOT NULL CHECK(expected_length>=0);
ALTER TABLE archive_capture_pages ADD COLUMN expected_chunks bigint NOT NULL CHECK(expected_chunks>=0);
ALTER TABLE archive_capture_pages ADD COLUMN expected_entries bigint CHECK(expected_entries>=0);
