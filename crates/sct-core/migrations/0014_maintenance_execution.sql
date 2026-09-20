-- Idempotency ownership remains immutable when a different administrator resumes.
ALTER TABLE maintenance_jobs ADD COLUMN run_principal_id text REFERENCES principals(id);
UPDATE maintenance_jobs SET run_principal_id=principal_id;
ALTER TABLE maintenance_jobs ALTER COLUMN run_principal_id SET NOT NULL;
CREATE INDEX maintenance_schedule ON maintenance_jobs(COALESCE(next_attempt_at,created_at),id)
 WHERE state IN ('queued','running','retry_wait');
