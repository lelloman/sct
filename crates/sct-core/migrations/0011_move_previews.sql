ALTER TABLE change_previews ADD COLUMN namespace_done boolean NOT NULL DEFAULT true;
CREATE TABLE preview_entry_work (
 root_id text NOT NULL, preview_id text NOT NULL, entry_id text NOT NULL,
 path_length integer NOT NULL CHECK(path_length BETWEEN 0 AND 4096), after_child text NOT NULL DEFAULT '',
 PRIMARY KEY(root_id,preview_id,entry_id),
 FOREIGN KEY(root_id,preview_id) REFERENCES change_previews(root_id,id),
 FOREIGN KEY(root_id,entry_id) REFERENCES entries(root_id,id)
);
CREATE INDEX namespace_children_ids ON entries(root_id,parent_id,id);
