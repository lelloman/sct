ALTER TABLE change_previews ADD COLUMN authority jsonb NOT NULL DEFAULT '{}';
ALTER TABLE change_previews ADD COLUMN retire_count bigint NOT NULL DEFAULT 0 CHECK(retire_count>=0);
ALTER TABLE change_previews ADD COLUMN loss_count bigint NOT NULL DEFAULT 0 CHECK(loss_count>=0);
ALTER TABLE change_previews ADD COLUMN estimated_bytes bigint NOT NULL DEFAULT 0 CHECK(estimated_bytes BETWEEN 0 AND 9007199254740991);
CREATE INDEX preview_history_scan ON file_versions(root_id,entry_id,(-sequence)) WHERE NOT retired;
