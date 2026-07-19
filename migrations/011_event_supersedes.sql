-- Event supersede chains (schema v11).
-- Applied in Rust: apply_v11_event_supersedes.
--
-- Append-only correction: a new event row points at the prior head via
-- supersedes_id; the prior head is soft-deleted. Prior payload is never
-- rewritten. ON DELETE SET NULL so purging an ancestor does not block purge.
-- See reports/append/F1-no-supersede-correction-model.md.

ALTER TABLE consumptions ADD COLUMN supersedes_id INTEGER REFERENCES consumptions(id) ON DELETE SET NULL;
ALTER TABLE purchases ADD COLUMN supersedes_id INTEGER REFERENCES purchases(id) ON DELETE SET NULL;
ALTER TABLE workouts ADD COLUMN supersedes_id INTEGER REFERENCES workouts(id) ON DELETE SET NULL;
ALTER TABLE exercise_sets ADD COLUMN supersedes_id INTEGER REFERENCES exercise_sets(id) ON DELETE SET NULL;
ALTER TABLE measurements ADD COLUMN supersedes_id INTEGER REFERENCES measurements(id) ON DELETE SET NULL;
ALTER TABLE sleep ADD COLUMN supersedes_id INTEGER REFERENCES sleep(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_consumptions_supersedes ON consumptions(supersedes_id);
CREATE INDEX IF NOT EXISTS idx_purchases_supersedes ON purchases(supersedes_id);
CREATE INDEX IF NOT EXISTS idx_workouts_supersedes ON workouts(supersedes_id);
CREATE INDEX IF NOT EXISTS idx_exercise_sets_supersedes ON exercise_sets(supersedes_id);
CREATE INDEX IF NOT EXISTS idx_measurements_supersedes ON measurements(supersedes_id);
CREATE INDEX IF NOT EXISTS idx_sleep_supersedes ON sleep(supersedes_id);
