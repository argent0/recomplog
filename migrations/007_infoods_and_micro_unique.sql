-- INFOODS reference catalog + case-insensitive micronutrient names (schema v7).
-- Applied in Rust: apply_v7_infoods_and_micro_unique.
--
-- Goals:
--   1. Full FAO INFOODS tagnames as infoods_components (+ synonym index).
--   2. micronutrients.name UNIQUE COLLATE NOCASE (prevent Iron/iron forks).
--   3. Optional micronutrients.infoods_tag → infoods_components(tag).
--
-- Seed data: data/infoods/infoods_components.json (vendored; no runtime network).

CREATE TABLE IF NOT EXISTS infoods_components (
    tag TEXT PRIMARY KEY COLLATE NOCASE,
    name TEXT NOT NULL,
    unit TEXT,
    synonyms TEXT,
    comments TEXT,
    tables_note TEXT,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS infoods_synonyms (
    synonym TEXT NOT NULL COLLATE NOCASE,
    tag TEXT NOT NULL REFERENCES infoods_components(tag) ON DELETE CASCADE,
    PRIMARY KEY (synonym, tag)
);

CREATE INDEX IF NOT EXISTS idx_infoods_synonyms_synonym ON infoods_synonyms(synonym);

-- Target micronutrients shape (rebuild in Rust after case-only merge):
-- CREATE TABLE micronutrients (
--     id INTEGER PRIMARY KEY AUTOINCREMENT,
--     name TEXT NOT NULL UNIQUE COLLATE NOCASE,
--     unit TEXT NOT NULL,
--     recommended_intake REAL,
--     created_at TEXT NOT NULL,
--     infoods_tag TEXT UNIQUE COLLATE NOCASE REFERENCES infoods_components(tag)
-- );
