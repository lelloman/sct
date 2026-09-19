ALTER TABLE session_objects ADD COLUMN expanded boolean NOT NULL DEFAULT false;
CREATE INDEX session_frontier ON session_objects(root_id,session_id,digest) WHERE NOT expanded;
CREATE TABLE transfer_retire (
 root_id text NOT NULL, session_id text NOT NULL, version_id text NOT NULL,
 reason text NOT NULL CHECK(reason IN ('rotation','admission')), applied boolean NOT NULL DEFAULT false,
 PRIMARY KEY(root_id,session_id,version_id),
 FOREIGN KEY(root_id,session_id) REFERENCES transfer_sessions(root_id,id),
 FOREIGN KEY(root_id,version_id) REFERENCES file_versions(root_id,id)
);
