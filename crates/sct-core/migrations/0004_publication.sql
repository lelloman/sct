-- A retired copy may remain readable under a live read pin while already charged
-- to staging pending cleanup. Deletion must not charge it a second time.
ALTER TABLE object_copies ADD COLUMN pending_charge boolean NOT NULL DEFAULT false;
UPDATE object_copies SET pending_charge=true WHERE state='deleting';
