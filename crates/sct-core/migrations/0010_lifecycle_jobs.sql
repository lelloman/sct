ALTER TABLE lifecycle_jobs ADD COLUMN run_principal_id text REFERENCES principals(id);
UPDATE lifecycle_jobs SET run_principal_id=principal_id;
ALTER TABLE lifecycle_jobs ALTER COLUMN run_principal_id SET NOT NULL;
ALTER TABLE lifecycle_jobs ADD COLUMN total bigint CHECK(total BETWEEN 0 AND 9007199254740991);
CREATE UNIQUE INDEX applied_preview ON lifecycle_jobs(root_id,preview_id) WHERE preview_id IS NOT NULL;
