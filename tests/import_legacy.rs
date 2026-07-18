//! Legacy import: workout provenance (sets cardio fields, activity_imports, trackpoints).

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::{params, Connection};
use std::path::Path;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

/// Minimal repslog-shaped SQLite with cardio set, zones/laps, import row, and trackpoints.
fn build_repslog_min(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE exercises (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            category TEXT NOT NULL,
            muscle_groups TEXT,
            equipment TEXT,
            load_type TEXT NOT NULL DEFAULT 'weight',
            description TEXT,
            is_custom INTEGER DEFAULT 0,
            created_at TEXT NOT NULL
        );

        CREATE TABLE workouts (
            id INTEGER PRIMARY KEY,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            workout_type TEXT,
            notes TEXT,
            overall_feeling INTEGER,
            duration_minutes INTEGER,
            created_at TEXT NOT NULL
        );

        CREATE TABLE workout_exercises (
            id INTEGER PRIMARY KEY,
            workout_id INTEGER NOT NULL REFERENCES workouts(id),
            exercise_id INTEGER NOT NULL REFERENCES exercises(id),
            "order" INTEGER NOT NULL,
            notes TEXT,
            goal_reps INTEGER
        );

        CREATE TABLE exercise_sets (
            id INTEGER PRIMARY KEY,
            workout_exercise_id INTEGER NOT NULL REFERENCES workout_exercises(id),
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
            created_at TEXT NOT NULL
        );

        CREATE TABLE activity_imports (
            id INTEGER PRIMARY KEY,
            workout_id INTEGER NOT NULL REFERENCES workouts(id),
            source_format TEXT NOT NULL,
            source_filename TEXT,
            file_sha256 TEXT NOT NULL UNIQUE,
            device_name TEXT,
            manufacturer_id INTEGER,
            product_id INTEGER,
            fit_sport INTEGER,
            fit_sub_sport INTEGER,
            imported_at TEXT NOT NULL
        );

        CREATE TABLE activity_trackpoints (
            id INTEGER PRIMARY KEY,
            exercise_set_id INTEGER NOT NULL REFERENCES exercise_sets(id),
            recorded_at TEXT NOT NULL,
            latitude REAL,
            longitude REAL,
            altitude_m REAL,
            heart_rate_bpm REAL,
            cadence_spm REAL,
            distance_km REAL,
            speed_m_s REAL
        );
        "#,
    )
    .unwrap();

    conn.execute(
        "INSERT INTO exercises (id, name, category, load_type, created_at)
         VALUES (1, 'running', 'cardio', 'bodyweight', '2026-01-01 00:00:00')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO workouts (id, started_at, finished_at, workout_type, duration_minutes, created_at)
         VALUES (1, '2026-07-01 08:00:00', '2026-07-01 08:45:00', 'Run', 45, '2026-07-01 08:00:00')",
        [],
    )
    .unwrap();
    conn.execute(
        r#"INSERT INTO workout_exercises (id, workout_id, exercise_id, "order")
           VALUES (1, 1, 1, 1)"#,
        [],
    )
    .unwrap();

    let zones = r#"{"z1":60,"z2":120,"z3":900,"z4":300,"z5":60}"#;
    let laps = r#"[{"distance_km":1.0,"duration_seconds":300},{"distance_km":1.0,"duration_seconds":310}]"#;
    conn.execute(
        "INSERT INTO exercise_sets (
            id, workout_exercise_id, set_number, distance_km, duration_seconds,
            avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km, calories_burned,
            avg_cadence_spm, total_ascent_m, total_descent_m,
            heart_rate_zones, laps, phase, created_at
         ) VALUES (1, 1, 1, 5.0, 1800, 145.0, 170.0, 6.0, 400, 80.0, 40.0, 35.0, ?1, ?2, 'full', '2026-07-01 08:00:00')",
        params![zones, laps],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO activity_imports (
            id, workout_id, source_format, source_filename, file_sha256,
            device_name, manufacturer_id, product_id, fit_sport, fit_sub_sport, imported_at
         ) VALUES (1, 1, 'fit', 'run.fit', 'aaaabbbbccccddddeeeeffff0000111122223333444455556666777788889999',
                   'TestWatch', 1, 2, 1, 0, '2026-07-01 09:00:00')",
        [],
    )
    .unwrap();

    for i in 0..5 {
        conn.execute(
            "INSERT INTO activity_trackpoints (
                id, exercise_set_id, recorded_at, latitude, longitude, altitude_m,
                heart_rate_bpm, cadence_spm, distance_km, speed_m_s
             ) VALUES (?1, 1, ?2, ?3, ?4, 100.0, 140.0, 80.0, ?5, 2.5)",
            params![
                i + 1,
                format!("2026-07-01 08:0{i}:00"),
                47.0 + (i as f64) * 0.001,
                8.0 + (i as f64) * 0.001,
                (i as f64) * 0.05,
            ],
        )
        .unwrap();
    }
}

fn init_target(db_s: &str) {
    bin().args(["--db", db_s, "init"]).assert().success();
}

#[test]
fn legacy_import_workout_provenance() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("repslog_min.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_repslog_min(&src);
    init_target(&db_s);

    let out = bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "workout",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["success"], true);
    let w = &v["counts"]["workout"];
    assert_eq!(w["sets"], 1);
    assert_eq!(w["activity_imports"], 1);
    assert_eq!(w["activity_trackpoints"], 5);
    assert_eq!(w["sets_with_zones"], 1);
    assert_eq!(w["sets_with_laps"], 1);
    assert_eq!(w["trackpoints_skipped"], 0);

    let conn = Connection::open(&dst).unwrap();
    let (dist, avg_hr, zones, laps): (f64, f64, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT distance_km, avg_heart_rate_bpm, heart_rate_zones, laps FROM exercise_sets WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert!((dist - 5.0).abs() < 1e-9);
    assert!((avg_hr - 145.0).abs() < 1e-9);
    assert!(zones.as_ref().unwrap().contains("z3"));
    assert!(laps.as_ref().unwrap().contains("duration_seconds"));

    let tp: i64 = conn
        .query_row("SELECT COUNT(*) FROM activity_trackpoints", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(tp, 5);

    let imports: i64 = conn
        .query_row("SELECT COUNT(*) FROM activity_imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(imports, 1);

    let sha: String = conn
        .query_row(
            "SELECT file_sha256 FROM activity_imports WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sha.starts_with("aaaabbbb"));
}

#[test]
fn legacy_import_idempotent() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("repslog_min.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_repslog_min(&src);
    init_target(&db_s);

    for _ in 0..2 {
        bin()
            .args([
                "--db",
                &db_s,
                "--json",
                "import",
                "legacy",
                "--from-db",
                &src_s,
                "--domain",
                "workout",
            ])
            .assert()
            .success();
    }

    let conn = Connection::open(&dst).unwrap();
    let sets: i64 = conn
        .query_row("SELECT COUNT(*) FROM exercise_sets", [], |r| r.get(0))
        .unwrap();
    let tp: i64 = conn
        .query_row("SELECT COUNT(*) FROM activity_trackpoints", [], |r| {
            r.get(0)
        })
        .unwrap();
    let imports: i64 = conn
        .query_row("SELECT COUNT(*) FROM activity_imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sets, 1);
    assert_eq!(tp, 5);
    assert_eq!(imports, 1);
}

#[test]
fn legacy_import_dry_run_provenance() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("repslog_min.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_repslog_min(&src);
    init_target(&db_s);

    let out = bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "workout",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\": true"))
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["dry_run"], true);
    let wc = &v["would_copy"]["workout"];
    assert_eq!(wc["sets"], 1);
    assert_eq!(wc["activity_imports"], 1);
    assert_eq!(wc["activity_trackpoints"], 5);
    assert_eq!(wc["workouts"], 1);

    // Target remains empty of workout data
    let conn = Connection::open(&dst).unwrap();
    let workouts: i64 = conn
        .query_row("SELECT COUNT(*) FROM workouts", [], |r| r.get(0))
        .unwrap();
    let tp: i64 = conn
        .query_row("SELECT COUNT(*) FROM activity_trackpoints", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(workouts, 0);
    assert_eq!(tp, 0);
}

/// Minimal bodylog-shaped DB: one measurement + one sleep row.
fn build_bodylog_min(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE measurements (
            id INTEGER PRIMARY KEY,
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
        CREATE TABLE sleep (
            id INTEGER PRIMARY KEY,
            date TEXT NOT NULL UNIQUE,
            total_sleep_minutes INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        INSERT INTO measurements (id, date, weight_kg, created_at, updated_at)
        VALUES (1, '2026-07-01', 80.5, '2026-07-01 07:00:00', '2026-07-01 07:00:00');
        INSERT INTO sleep (id, date, total_sleep_minutes, created_at, updated_at)
        VALUES (1, '2026-07-01', 450, '2026-07-01 07:00:00', '2026-07-01 07:00:00');
        "#,
    )
    .unwrap();
}

/// Minimal nutlog-shaped DB: product, nutrition, purchase, consumption.
/// `nutrients` is empty but present — `copy_nutrition` requires the table.
fn build_nutlog_min(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE products (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE nutrients (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            unit TEXT NOT NULL,
            recommended_intake REAL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE product_nutritions (
            product_id INTEGER PRIMARY KEY,
            reference_quantity REAL NOT NULL,
            reference_unit TEXT NOT NULL,
            energy_kcal REAL,
            protein_g REAL,
            carbohydrates_g REAL,
            fat_g REAL,
            fiber_g REAL,
            sugars_g REAL
        );
        CREATE TABLE purchases (
            id INTEGER PRIMARY KEY,
            product_id INTEGER NOT NULL,
            quantity REAL NOT NULL,
            price_cents INTEGER,
            store_id INTEGER,
            purchased_at TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE consumptions (
            id INTEGER PRIMARY KEY,
            product_id INTEGER NOT NULL,
            quantity REAL NOT NULL,
            unit TEXT,
            consumed_at TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        INSERT INTO products (id, name, created_at, updated_at)
        VALUES (1, 'Oats', '2026-07-01 08:00:00', '2026-07-01 08:00:00');
        INSERT INTO product_nutritions (
            product_id, reference_quantity, reference_unit,
            energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g
        ) VALUES (1, 100.0, 'g', 389.0, 17.0, 66.0, 7.0, 11.0, 1.0);
        INSERT INTO purchases (id, product_id, quantity, price_cents, store_id, purchased_at, created_at)
        VALUES (1, 1, 500.0, 399, NULL, '2026-07-01 09:00:00', '2026-07-01 09:00:00');
        INSERT INTO consumptions (id, product_id, quantity, unit, consumed_at, created_at)
        VALUES (1, 1, 100.0, 'g', '2026-07-01 12:00:00', '2026-07-01 12:00:00');
        "#,
    )
    .unwrap();
}

#[test]
fn legacy_import_body_domain_still_works() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("body_min.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_bodylog_min(&src);
    init_target(&db_s);

    let out = bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "body",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["success"], true);
    assert!(v["imported_domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d == "body"));

    let target = Connection::open(&dst).unwrap();
    let w: f64 = target
        .query_row(
            "SELECT weight_kg FROM measurements WHERE date = '2026-07-01'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((w - 80.5).abs() < 1e-9);

    let sleep_min: i64 = target
        .query_row(
            "SELECT total_sleep_minutes FROM sleep WHERE date = '2026-07-01'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sleep_min, 450);
}

/// Re-import body must not overwrite local corrections (INSERT OR IGNORE, not REPLACE).
#[test]
fn legacy_import_body_idempotent_preserves_corrections() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("body_min.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_bodylog_min(&src);
    init_target(&db_s);

    bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "body",
        ])
        .assert()
        .success();

    // Local correction after first import.
    let target = Connection::open(&dst).unwrap();
    target
        .execute(
            "UPDATE measurements SET weight_kg = 81.2, body_fat_pct = 15.0 WHERE date = '2026-07-01'",
            [],
        )
        .unwrap();
    target
        .execute(
            "UPDATE sleep SET total_sleep_minutes = 480 WHERE date = '2026-07-01'",
            [],
        )
        .unwrap();

    let out = bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "body",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["success"], true);
    let body = &v["counts"]["body"];
    assert_eq!(body["measurements"], 0);
    assert_eq!(body["measurements_skipped"], 1);
    assert_eq!(body["sleep"], 0);
    assert_eq!(body["sleep_skipped"], 1);

    let (w, bf): (f64, Option<f64>) = target
        .query_row(
            "SELECT weight_kg, body_fat_pct FROM measurements WHERE date = '2026-07-01'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        (w - 81.2).abs() < 1e-9,
        "weight must stay corrected, got {w}"
    );
    assert_eq!(bf, Some(15.0));

    let sleep_min: i64 = target
        .query_row(
            "SELECT total_sleep_minutes FROM sleep WHERE date = '2026-07-01'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sleep_min, 480);

    let m_count: i64 = target
        .query_row("SELECT COUNT(*) FROM measurements", [], |r| r.get(0))
        .unwrap();
    let s_count: i64 = target
        .query_row("SELECT COUNT(*) FROM sleep", [], |r| r.get(0))
        .unwrap();
    assert_eq!(m_count, 1);
    assert_eq!(s_count, 1);
}

#[test]
fn legacy_import_nutrition_domain() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("nutlog_min.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_nutlog_min(&src);
    init_target(&db_s);

    let out = bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "nutrition",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["success"], true);
    assert!(v["imported_domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d == "nutrition"));
    let n = &v["counts"]["nutrition"];
    assert_eq!(n["products"], 1);
    assert_eq!(n["purchases"], 1);
    assert_eq!(n["consumptions"], 1);

    let target = Connection::open(&dst).unwrap();
    let name: String = target
        .query_row("SELECT name FROM products WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "Oats");
    let kcal: f64 = target
        .query_row(
            "SELECT energy_kcal FROM product_nutritions WHERE product_id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((kcal - 389.0).abs() < 1e-9);
    let purchases: i64 = target
        .query_row("SELECT COUNT(*) FROM purchases", [], |r| r.get(0))
        .unwrap();
    let consumptions: i64 = target
        .query_row("SELECT COUNT(*) FROM consumptions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(purchases, 1);
    assert_eq!(consumptions, 1);

    // Re-import is idempotent (INSERT OR IGNORE).
    bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "nutrition",
        ])
        .assert()
        .success();
    let products: i64 = target
        .query_row("SELECT COUNT(*) FROM products", [], |r| r.get(0))
        .unwrap();
    let purchases2: i64 = target
        .query_row("SELECT COUNT(*) FROM purchases", [], |r| r.get(0))
        .unwrap();
    let consumptions2: i64 = target
        .query_row("SELECT COUNT(*) FROM consumptions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(products, 1);
    assert_eq!(purchases2, 1);
    assert_eq!(consumptions2, 1);
}

/// Real repslog DBs have no `finished_at` on workouts — import must still succeed.
fn build_repslog_no_finished_at(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        CREATE TABLE exercises (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            category TEXT NOT NULL,
            muscle_groups TEXT,
            equipment TEXT,
            description TEXT,
            is_custom INTEGER DEFAULT 0,
            created_at TEXT,
            load_type TEXT NOT NULL DEFAULT 'external'
        );
        CREATE TABLE workouts (
            id INTEGER PRIMARY KEY,
            started_at TEXT NOT NULL,
            workout_type TEXT,
            notes TEXT,
            overall_feeling INTEGER,
            duration_minutes INTEGER,
            created_at TEXT
        );
        CREATE TABLE workout_exercises (
            id INTEGER PRIMARY KEY,
            workout_id INTEGER NOT NULL REFERENCES workouts(id) ON DELETE CASCADE,
            exercise_id INTEGER NOT NULL REFERENCES exercises(id),
            "order" INTEGER NOT NULL,
            notes TEXT,
            goal_reps INTEGER
        );
        CREATE TABLE exercise_sets (
            id INTEGER PRIMARY KEY,
            workout_exercise_id INTEGER NOT NULL REFERENCES workout_exercises(id) ON DELETE CASCADE,
            set_number INTEGER NOT NULL,
            reps INTEGER,
            weight_kg REAL,
            phase TEXT NOT NULL DEFAULT 'working',
            created_at TEXT
        );
        INSERT INTO exercises (id, name, category, load_type, created_at)
        VALUES (1, 'bench press', 'strength', 'external', '2026-07-01 00:00:00');
        INSERT INTO workouts (id, started_at, workout_type, notes, created_at)
        VALUES (1, '2026-07-01 18:00:00', 'push', 'solid', '2026-07-01 18:00:00');
        INSERT INTO workout_exercises (id, workout_id, exercise_id, "order")
        VALUES (1, 1, 1, 1);
        INSERT INTO exercise_sets (id, workout_exercise_id, set_number, reps, weight_kg, phase, created_at)
        VALUES (1, 1, 1, 5, 100.0, 'full', '2026-07-01 18:00:00');
        "#,
    )
    .unwrap();
}

#[test]
fn legacy_import_repslog_without_finished_at() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("repslog_real.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_repslog_no_finished_at(&src);
    init_target(&db_s);

    let out = bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "workout",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["success"], true);
    let n = &v["counts"]["workout"];
    // exercises may be 0 when name already exists from seed — remapped by name.
    assert_eq!(n["workouts"], 1);
    assert_eq!(n["workout_exercises"], 1);
    assert_eq!(n["sets"], 1);

    let target = Connection::open(&dst).unwrap();
    let finished: Option<String> = target
        .query_row("SELECT finished_at FROM workouts WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(finished.is_none());
    let reps: i64 = target
        .query_row("SELECT reps FROM exercise_sets WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(reps, 5);
    // Name remap: source exercise id 1 ("bench press") must not attach to seed id 1 ("pushups").
    let ename: String = target
        .query_row(
            "SELECT e.name FROM workout_exercises we
             JOIN exercises e ON e.id = we.exercise_id
             WHERE we.id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ename.to_lowercase(), "bench press");
}

/// Purchases with store_id require stores to be imported first (FK).
fn build_nutlog_with_store(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE products (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE nutrients (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            unit TEXT NOT NULL,
            recommended_intake REAL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE stores (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE purchases (
            id INTEGER PRIMARY KEY,
            product_id INTEGER NOT NULL,
            quantity REAL NOT NULL,
            price_cents INTEGER,
            store_id INTEGER,
            purchased_at TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE consumptions (
            id INTEGER PRIMARY KEY,
            product_id INTEGER NOT NULL,
            quantity REAL NOT NULL,
            unit TEXT,
            consumed_at TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        INSERT INTO products (id, name, created_at, updated_at)
        VALUES (1, 'Chicken', '2026-07-01 08:00:00', '2026-07-01 08:00:00');
        INSERT INTO stores (id, name, created_at)
        VALUES (1, 'Local Market', '2026-07-01 08:00:00');
        INSERT INTO purchases (id, product_id, quantity, price_cents, store_id, purchased_at, created_at)
        VALUES (1, 1, 1.0, 999, 1, '2026-07-01 09:00:00', '2026-07-01 09:00:00');
        INSERT INTO consumptions (id, product_id, quantity, unit, consumed_at, created_at)
        VALUES (1, 1, 200.0, 'g', '2026-07-01 12:00:00', '2026-07-01 12:00:00');
        "#,
    )
    .unwrap();
}

#[test]
fn legacy_import_nutrition_with_store_fk() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("nutlog_store.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();
    build_nutlog_with_store(&src);
    init_target(&db_s);

    let out = bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "legacy",
            "--from-db",
            &src_s,
            "--domain",
            "nutrition",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["counts"]["nutrition"]["purchases"], 1);

    let target = Connection::open(&dst).unwrap();
    let store: String = target
        .query_row("SELECT name FROM stores WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(store, "Local Market");
    let sid: i64 = target
        .query_row("SELECT store_id FROM purchases WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(sid, 1);
}
