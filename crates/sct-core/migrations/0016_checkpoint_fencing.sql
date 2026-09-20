ALTER TABLE checkpoint_barrier ADD COLUMN phase text NOT NULL DEFAULT 'draining' CHECK(phase IN ('draining','capturing'));
ALTER TABLE checkpoints ADD COLUMN credential_id text REFERENCES credentials(id);
CREATE INDEX checkpoint_live_state ON checkpoints(state,id);
