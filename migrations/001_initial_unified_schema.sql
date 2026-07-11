PRAGMA foreign_keys = ON;

-- ============================================================
-- WORKOUT DOMAIN (adapted from repslog)
-- ============================================================

CREATE TABLE IF NOT EXISTS exercises (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    category TEXT NOT NULL,
    muscle_groups TEXT,
    equipment TEXT,
    load_type TEXT NOT NULL DEFAULT 'weight',
    description TEXT,
    is_custom INTEGER DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS workouts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    workout_type TEXT,
    notes TEXT,
    overall_feeling INTEGER CHECK (overall_feeling BETWEEN 1 AND 5 OR overall_feeling IS NULL),
    duration_minutes INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS workout_exercises (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workout_id INTEGER NOT NULL REFERENCES workouts(id) ON DELETE CASCADE,
    exercise_id INTEGER NOT NULL REFERENCES exercises(id),
    "order" INTEGER NOT NULL,
    notes TEXT,
    goal_reps INTEGER
);

CREATE TABLE IF NOT EXISTS exercise_sets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workout_exercise_id INTEGER NOT NULL REFERENCES workout_exercises(id) ON DELETE CASCADE,
    set_number INTEGER NOT NULL,
    reps INTEGER,
    weight_kg REAL,
    external_load_kg REAL,
    distance_km REAL,
    duration_seconds INTEGER,
    rpe REAL,
    rir REAL,
    effective_reps INTEGER,
    cluster_id INTEGER,
    rest_seconds INTEGER,
    notes TEXT,
    side TEXT,
    phase TEXT NOT NULL DEFAULT 'working',
    extra_metrics TEXT,
    avg_heart_rate_bpm REAL,
    max_heart_rate_bpm REAL,
    avg_pace_min_per_km REAL,
    calories_burned INTEGER,
    avg_cadence_spm REAL,
    total_ascent_m REAL,
    total_descent_m REAL,
    date_of_birth TEXT,
    resting_hr_bpm REAL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS activity_imports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workout_id INTEGER NOT NULL REFERENCES workouts(id) ON DELETE CASCADE,
    source_format TEXT NOT NULL,
    source_filename TEXT,
    file_sha256 TEXT NOT NULL UNIQUE,
    device_name TEXT,
    manufacturer_id INTEGER,
    product_id INTEGER,
    fit_sport INTEGER,
    fit_sub_sport INTEGER,
    imported_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS activity_trackpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    exercise_set_id INTEGER NOT NULL REFERENCES exercise_sets(id) ON DELETE CASCADE,
    recorded_at TEXT NOT NULL,
    latitude REAL,
    longitude REAL,
    altitude_m REAL,
    heart_rate_bpm REAL,
    cadence_spm REAL,
    distance_km REAL,
    speed_m_s REAL
);

CREATE INDEX IF NOT EXISTS idx_sets_workout_ex ON exercise_sets(workout_exercise_id);
CREATE INDEX IF NOT EXISTS idx_trackpoints_set ON activity_trackpoints(exercise_set_id);

-- ============================================================
-- BODY DOMAIN (from bodylog)
-- ============================================================

CREATE TABLE IF NOT EXISTS measurements (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL UNIQUE,
    weight_kg REAL,
    body_fat_pct REAL,
    skeletal_muscle_pct REAL,
    visceral_fat_level INTEGER,
    bmi REAL,
    resting_metabolism_kcal INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_measurements_date ON measurements(date);

CREATE TABLE IF NOT EXISTS user_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    height_cm REAL,
    date_of_birth TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS sleep (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL UNIQUE,
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

CREATE INDEX IF NOT EXISTS idx_sleep_date ON sleep(date);

-- ============================================================
-- NUTRITION DOMAIN (from nutlog)
-- ============================================================

CREATE TABLE IF NOT EXISTS products (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS nutrients (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    unit TEXT NOT NULL,
    recommended_intake REAL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS product_tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS store_tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS stores (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS product_tag_associations (
    product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES product_tags(id) ON DELETE CASCADE,
    PRIMARY KEY (product_id, tag_id)
);

CREATE TABLE IF NOT EXISTS store_tag_associations (
    store_id INTEGER NOT NULL REFERENCES stores(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES store_tags(id) ON DELETE CASCADE,
    PRIMARY KEY (store_id, tag_id)
);

CREATE TABLE IF NOT EXISTS product_nutritions (
    product_id INTEGER PRIMARY KEY REFERENCES products(id) ON DELETE CASCADE,
    reference_quantity REAL NOT NULL,
    reference_unit TEXT NOT NULL,
    energy_kcal REAL,
    protein_g REAL,
    carbohydrates_g REAL,
    fat_g REAL,
    fiber_g REAL,
    sugars_g REAL
);

CREATE TABLE IF NOT EXISTS product_micronutrients (
    product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    nutrient_id INTEGER NOT NULL REFERENCES nutrients(id),
    amount REAL NOT NULL,
    unit TEXT NOT NULL,
    PRIMARY KEY (product_id, nutrient_id)
);

CREATE TABLE IF NOT EXISTS purchases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    product_id INTEGER NOT NULL REFERENCES products(id),
    quantity REAL NOT NULL,
    price_cents INTEGER,
    store_id INTEGER REFERENCES stores(id),
    purchased_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS consumptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    product_id INTEGER NOT NULL REFERENCES products(id),
    quantity REAL NOT NULL,
    unit TEXT,
    consumed_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_purchases_product ON purchases(product_id);
CREATE INDEX IF NOT EXISTS idx_consumptions_product ON consumptions(product_id);
CREATE INDEX IF NOT EXISTS idx_purchases_date ON purchases(purchased_at);
