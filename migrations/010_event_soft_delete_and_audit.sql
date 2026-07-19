-- Event soft-delete (tombstone) + append-only entity_audit (schema v10).
-- Applied in Rust: apply_v10_event_soft_delete_and_audit.
--
-- Default delete on event rows sets deleted_at (storage clock) instead of hard DELETE.
-- Hard purge remains explicit (--purge) and still uses ON DELETE CASCADE for trees.
-- entity_audit has no FK to event tables so purge cannot erase the trail.
-- See reports/append/S3-hard-delete-cascade-event-trees.md and S7.

ALTER TABLE workouts ADD COLUMN deleted_at TEXT;
ALTER TABLE workouts ADD COLUMN delete_reason TEXT;

ALTER TABLE exercise_sets ADD COLUMN deleted_at TEXT;
ALTER TABLE exercise_sets ADD COLUMN delete_reason TEXT;

ALTER TABLE measurements ADD COLUMN deleted_at TEXT;
ALTER TABLE measurements ADD COLUMN delete_reason TEXT;

ALTER TABLE sleep ADD COLUMN deleted_at TEXT;
ALTER TABLE sleep ADD COLUMN delete_reason TEXT;

ALTER TABLE consumptions ADD COLUMN deleted_at TEXT;
ALTER TABLE consumptions ADD COLUMN delete_reason TEXT;

ALTER TABLE purchases ADD COLUMN deleted_at TEXT;
ALTER TABLE purchases ADD COLUMN delete_reason TEXT;

CREATE TABLE IF NOT EXISTS entity_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type TEXT NOT NULL,
    entity_id INTEGER NOT NULL,
    at TEXT NOT NULL,
    kind TEXT NOT NULL,
    actor TEXT,
    summary TEXT,
    fields_json TEXT,
    meta_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_entity_audit_lookup ON entity_audit(entity_type, entity_id, at);
CREATE INDEX IF NOT EXISTS idx_entity_audit_at ON entity_audit(at);
