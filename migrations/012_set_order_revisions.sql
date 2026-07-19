-- Append-only session set order (schema v12).
-- Applied in Rust: apply_v12_set_order_revisions.
--
-- Order is a separate fact stream from exercise_sets payload.
-- set_number on exercise_sets is frozen at insert; workout set move
-- inserts a revision (full ordered id list), never UPDATEs sibling set_number.
-- See reports/append/F4-append-only-set-order.md.

CREATE TABLE IF NOT EXISTS set_order_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workout_exercise_id INTEGER NOT NULL
        REFERENCES workout_exercises(id) ON DELETE CASCADE,
    at TEXT NOT NULL,
    actor TEXT,
    reason TEXT,
    order_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_set_order_revisions_we
    ON set_order_revisions(workout_exercise_id, at, id);
