ALTER TABLE integrity_findings ADD CONSTRAINT finding_root_identity UNIQUE(root_id,id);
CREATE TABLE mirror_repair_work (
 job_id text NOT NULL, id text NOT NULL, root_id text NOT NULL,
 backend_id text NOT NULL, digest text NOT NULL,
 state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','completed')),
 generation text REFERENCES object_copies(generation),
 PRIMARY KEY(job_id,id), UNIQUE(job_id,backend_id,digest),
 FOREIGN KEY(root_id,job_id) REFERENCES maintenance_jobs(root_id,id),
 FOREIGN KEY(root_id,id) REFERENCES integrity_findings(root_id,id),
 FOREIGN KEY(root_id,backend_id) REFERENCES root_backends(root_id,backend_id),
 FOREIGN KEY(root_id,digest) REFERENCES content_objects(root_id,digest),
 CHECK((state='completed')=(generation IS NOT NULL))
);
