-- Allow multiple measurement/sleep samples per calendar day (schema v8).
-- Applied in Rust: apply_v8_body_multi_sample_per_day.
--
-- Drops UNIQUE on measurements.date and sleep.date so event history can grow by
-- insertion (same-day re-weigh, nap vs night, late corrections via new rows).
-- Existing one-row-per-day data remains valid. Non-unique date indexes retained.

-- measurements: rebuild without UNIQUE(date)
CREATE TABLE measurements_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    weight_kg REAL,
    body_fat_pct REAL,
    skeletal_muscle_pct REAL,
    visceral_fat_level INTEGER,
    bmi REAL,
    resting_metabolism_kcal INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO measurements_new (
    id, date, weight_kg, body_fat_pct, skeletal_muscle_pct,
    visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at
)
SELECT
    id, date, weight_kg, body_fat_pct, skeletal_muscle_pct,
    visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at
FROM measurements;

DROP TABLE measurements;
ALTER TABLE measurements_new RENAME TO measurements;
CREATE INDEX IF NOT EXISTS idx_measurements_date ON measurements(date);

-- sleep: rebuild without UNIQUE(date)
CREATE TABLE sleep_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    bedtime TEXT,
    wake_time TEXT,
    time_in_bed_minutes INTEGER,
    total_sleep_minutes INTEGER,
    rem_minutes INTEGER,
    deep_minutes INTEGER,
    light_minutes INTEGER,
    awake_minutes INTEGER,
    sleep_efficiency_pct REAL,
    sleep_score INTEGER,
    subjective_quality INTEGER,
    awakenings INTEGER,
    heart_rate_bpm REAL,
    hypopnea_per_hr REAL,
    respiratory_rate REAL,
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO sleep_new (
    id, date, bedtime, wake_time, time_in_bed_minutes, total_sleep_minutes,
    rem_minutes, deep_minutes, light_minutes, awake_minutes,
    sleep_efficiency_pct, sleep_score, subjective_quality, awakenings,
    heart_rate_bpm, hypopnea_per_hr, respiratory_rate, notes,
    created_at, updated_at
)
SELECT
    id, date, bedtime, wake_time, time_in_bed_minutes, total_sleep_minutes,
    rem_minutes, deep_minutes, light_minutes, awake_minutes,
    sleep_efficiency_pct, sleep_score, subjective_quality, awakenings,
    heart_rate_bpm, hypopnea_per_hr, respiratory_rate, notes,
    created_at, updated_at
FROM sleep;

DROP TABLE sleep;
ALTER TABLE sleep_new RENAME TO sleep;
CREATE INDEX IF NOT EXISTS idx_sleep_date ON sleep(date);
