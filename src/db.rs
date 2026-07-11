use anyhow::{Context, Result};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension};
use std::path::PathBuf;

/// Default DB path: $XDG_DATA_HOME/recomplog/recomplog.db
pub fn default_db_path() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("com", "recomplog", "recomplog") {
        let mut path = proj_dirs.data_dir().to_path_buf();
        path.push("recomplog.db");
        path
    } else {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        let mut path = home;
        path.push(".local/share/recomplog/recomplog.db");
        path
    }
}

/// Resolve DB path (override or default) and ensure parent dir exists.
pub fn resolve_db_path(override_path: Option<&str>) -> Result<PathBuf> {
    let path = match override_path {
        Some(p) => PathBuf::from(p),
        None => default_db_path(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create database directory: {}", parent.display())
        })?;
    }
    Ok(path)
}

/// Open DB, enable FKs, run migrations, return connection.
pub fn open_db(override_path: Option<&str>) -> Result<Connection> {
    let path = resolve_db_path(override_path)?;
    let conn = Connection::open(&path)
        .with_context(|| format!("failed to open database at {}", path.display()))?;
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    run_migrations(&conn)?;
    Ok(conn)
}

/// Current schema version. Bump when adding a new migration block.
const CURRENT_VERSION: i32 = 4;

fn run_migrations(conn: &Connection) -> Result<()> {
    let current: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .optional()?
        .unwrap_or(0);

    if current >= CURRENT_VERSION {
        return Ok(());
    }

    // For the initial unified release we apply everything in v1.
    // Future changes will append additional versioned blocks.
    if current < 1 {
        apply_initial_schema(conn)?;
        conn.execute("PRAGMA user_version = 1", [])?;
    }
    if current < 2 {
        apply_v2_cardio_json(conn)?;
        conn.execute("PRAGMA user_version = 2", [])?;
    }
    if current < 3 {
        normalize_nutrition_units(conn)?;
        conn.execute("PRAGMA user_version = 3", [])?;
    }
    if current < 4 {
        // Recover products that v3 rewrote as N×reference grams/ml when they
        // were really whole packages (e.g. 1 bar → 46 g → back to 1 unit).
        promote_whole_package_products(conn)?;
        conn.execute("PRAGMA user_version = 4", [])?;
    }

    Ok(())
}

/// Re-run unit normalization (also used after legacy import).
pub fn normalize_nutrition_units_public(conn: &Connection) -> Result<()> {
    normalize_nutrition_units(conn)?;
    promote_whole_package_products(conn)?;
    Ok(())
}

/// If every consumption is an integer multiple of the product’s reference amount
/// (e.g. only 46 g and 92 g of a 46 g bar), treat the product as package/`unit`.
fn promote_whole_package_products(conn: &Connection) -> Result<()> {
    use crate::nutrition_units::{parse_unit, UnitKind};

    let products: Vec<(i64, f64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT product_id, reference_quantity, reference_unit FROM product_nutritions",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };

    for (pid, ref_qty, ref_unit) in products {
        let Ok(ref_parsed) = parse_unit(&ref_unit) else {
            continue;
        };
        if ref_parsed.kind == UnitKind::Package || ref_qty <= 0.0 {
            continue;
        }

        let rows: Vec<(i64, f64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, quantity, unit FROM consumptions
                 WHERE product_id = ?1 AND unit IS NOT NULL AND trim(unit) != ''",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![pid], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        if rows.is_empty() {
            continue;
        }

        let mut scales: Vec<(i64, f64)> = Vec::new();
        let mut all_whole = true;
        for (cid, qty, unit) in &rows {
            let Ok(cu) = parse_unit(unit) else {
                all_whole = false;
                break;
            };
            if cu.kind != ref_parsed.kind {
                all_whole = false;
                break;
            }
            let scale = qty / ref_qty;
            let nearest = scale.round();
            if !(1.0..=20.0).contains(&nearest) || (scale - nearest).abs() > 1e-6 {
                all_whole = false;
                break;
            }
            scales.push((*cid, nearest));
        }
        if !all_whole || scales.is_empty() {
            continue;
        }

        conn.execute(
            "UPDATE product_nutritions SET reference_quantity = 1, reference_unit = 'unit'
             WHERE product_id = ?1",
            rusqlite::params![pid],
        )?;
        for (cid, n_units) in scales {
            conn.execute(
                "UPDATE consumptions SET quantity = ?1, unit = 'unit' WHERE id = ?2",
                rusqlite::params![n_units, cid],
            )?;
        }
    }
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for n in names {
        if n? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn apply_v2_cardio_json(conn: &Connection) -> Result<()> {
    // Fresh DBs from v1 apply may already include columns if schema was updated;
    // ALTER only when missing (idempotent for re-runs / mixed paths).
    if !column_exists(conn, "exercise_sets", "heart_rate_zones")? {
        conn.execute(
            "ALTER TABLE exercise_sets ADD COLUMN heart_rate_zones TEXT",
            [],
        )?;
    }
    if !column_exists(conn, "exercise_sets", "laps")? {
        conn.execute("ALTER TABLE exercise_sets ADD COLUMN laps TEXT", [])?;
    }
    Ok(())
}

/// Normalize nutrition units to the explicit vocabulary: `g`, `ml`, `unit`.
///
/// - Product reference units: aliases → canonical kind unit.
/// - Products only ever consumed as package counts against a mass/volume
///   reference are converted to `1 unit` (macros already describe one package).
/// - Package-style consumptions against remaining mass/volume products are
///   rewritten to the reference amount (1 bar of a 46 g product → 46 g).
/// - All consumption units are stored as `g` / `ml` / `unit`.
fn normalize_nutrition_units(conn: &Connection) -> Result<()> {
    use crate::nutrition_units::{parse_unit, UnitKind};

    // --- 1. Normalize product reference units ---
    let products: Vec<(i64, f64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT product_id, reference_quantity, reference_unit FROM product_nutritions",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };

    for (pid, ref_qty, ref_unit) in &products {
        let Ok(parsed) = parse_unit(ref_unit) else {
            continue;
        };
        let canonical = parsed.canonical();
        if ref_unit != canonical {
            // e.g. capsule → unit: keep quantity (usually 1).
            // e.g. kg → g: convert quantity into base units.
            let new_qty = if parsed.kind == UnitKind::Package {
                *ref_qty
            } else {
                ref_qty * parsed.to_base
            };
            conn.execute(
                "UPDATE product_nutritions SET reference_quantity = ?1, reference_unit = ?2
                 WHERE product_id = ?3",
                rusqlite::params![new_qty, canonical, pid],
            )?;
        }
    }

    // Re-read after product updates
    let products: Vec<(i64, f64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT product_id, reference_quantity, reference_unit FROM product_nutritions",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };

    // --- 2. Decide which mass/volume products should become package products ---
    for (pid, ref_qty, ref_unit) in &products {
        let Ok(ref_parsed) = parse_unit(ref_unit) else {
            continue;
        };
        if ref_parsed.kind == UnitKind::Package {
            continue;
        }

        let mut stmt = conn.prepare(
            "SELECT unit FROM consumptions WHERE product_id = ?1 AND unit IS NOT NULL AND trim(unit) != ''",
        )?;
        let units: Vec<String> = stmt
            .query_map(rusqlite::params![pid], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if units.is_empty() {
            continue;
        }

        let mut any_package = false;
        let mut any_matching_measure = false;
        for u in &units {
            let Ok(p) = parse_unit(u) else {
                continue;
            };
            if p.kind == UnitKind::Package {
                any_package = true;
            } else if p.kind == ref_parsed.kind {
                any_matching_measure = true;
            }
        }

        // Only package counts logged → product is package-oriented (e.g. protein bar).
        if any_package && !any_matching_measure {
            conn.execute(
                "UPDATE product_nutritions SET reference_quantity = 1, reference_unit = 'unit'
                 WHERE product_id = ?1",
                rusqlite::params![pid],
            )?;
            // Consumptions: 1 bar → 1 unit (quantity unchanged).
            conn.execute(
                "UPDATE consumptions SET unit = 'unit'
                 WHERE product_id = ?1
                   AND unit IS NOT NULL
                   AND lower(trim(unit)) IN (
                     'unit','units','package','packages','pack','packs','packet','packets',
                     'serving','servings','portion','portions','bar','bars','cup','cups',
                     'capsule','capsules','cap','caps','tablet','tablets','pill','pills',
                     'scoop','scoops','piece','pieces','item','items','bottle','bottles',
                     'can','cans','slice','slices','drink','drinks','spoon','spoons'
                   )",
                rusqlite::params![pid],
            )?;
            let _ = ref_qty; // macros already describe one package serving
        }
    }

    // --- 3. Rewrite remaining package consumptions against mass/volume products ---
    let products: Vec<(i64, f64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT product_id, reference_quantity, reference_unit FROM product_nutritions",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };

    for (pid, ref_qty, ref_unit) in &products {
        let Ok(ref_parsed) = parse_unit(ref_unit) else {
            continue;
        };
        if ref_parsed.kind == UnitKind::Package {
            continue;
        }
        let mut stmt = conn.prepare(
            "SELECT id, quantity, unit FROM consumptions
             WHERE product_id = ?1 AND unit IS NOT NULL AND trim(unit) != ''",
        )?;
        let rows: Vec<(i64, f64, String)> = stmt
            .query_map(rusqlite::params![pid], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (cid, qty, unit) in rows {
            let Ok(cu) = parse_unit(&unit) else {
                continue;
            };
            if cu.kind == UnitKind::Package {
                // Mixed history: keep mass product, expand package counts to amount.
                // (Pure package products were already converted in step 2.)
                let new_qty = qty * ref_qty;
                conn.execute(
                    "UPDATE consumptions SET quantity = ?1, unit = ?2 WHERE id = ?3",
                    rusqlite::params![new_qty, ref_parsed.canonical(), cid],
                )?;
            }
        }
    }

    // --- 4. Normalize all remaining consumption units to g|ml|unit ---
    let consumptions: Vec<(i64, Option<String>, i64)> = {
        let mut stmt = conn.prepare("SELECT id, unit, product_id FROM consumptions")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };

    for (cid, unit, pid) in consumptions {
        let unit_trim = unit.as_deref().map(str::trim).unwrap_or("");
        if unit_trim.is_empty() {
            // Default to product reference unit when present.
            let ref_u: Option<String> = conn
                .query_row(
                    "SELECT reference_unit FROM product_nutritions WHERE product_id = ?1",
                    [pid],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(ru) = ref_u {
                if let Ok(p) = parse_unit(&ru) {
                    conn.execute(
                        "UPDATE consumptions SET unit = ?1 WHERE id = ?2",
                        rusqlite::params![p.canonical(), cid],
                    )?;
                }
            }
            continue;
        }
        if let Ok(p) = parse_unit(unit_trim) {
            let canon = p.canonical();
            if unit_trim != canon {
                // Convert quantity into base units when alias had a factor
                // (e.g. 0.1 kg → 100 g).
                let qty: f64 = conn.query_row(
                    "SELECT quantity FROM consumptions WHERE id = ?1",
                    [cid],
                    |r| r.get(0),
                )?;
                let new_qty = if p.kind == UnitKind::Package {
                    qty
                } else {
                    qty * p.to_base
                };
                conn.execute(
                    "UPDATE consumptions SET quantity = ?1, unit = ?2 WHERE id = ?3",
                    rusqlite::params![new_qty, canon, cid],
                )?;
            }
        }
    }

    Ok(())
}

fn apply_initial_schema(conn: &Connection) -> Result<()> {
    // This schema is a pragmatic merge of the latest known schemas from
    // repslog, bodylog, and nutlog. It is intentionally simple and denormalized
    // only where the original tools were.

    let schema = r#"
PRAGMA foreign_keys = ON;

-- ============================================================
-- WORKOUT DOMAIN (adapted from repslog latest schema)
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
    heart_rate_zones TEXT,
    laps TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- FIT / activity import provenance
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
"#;

    conn.execute_batch(schema)?;
    Ok(())
}

/// Open an arbitrary existing SQLite file read-only (used for legacy import).
/// Does not run migrations on it.
pub fn open_legacy_db_readonly(path: &str) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open legacy database at {}", path))?;
    Ok(conn)
}

/// Common timestamp helper (UTC ISO-ish string).
pub fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Re-export flexible date helpers for convenience.
#[allow(unused_imports)]
pub use crate::utils::{parse_date_to_ymd, parse_flexible_date, parse_flexible_datetime};
