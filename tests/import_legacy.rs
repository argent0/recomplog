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

#[test]
fn legacy_import_body_domain_still_works() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("body_min.db");
    let dst = dir.path().join("target.db");
    let src_s = src.display().to_string();
    let db_s = dst.display().to_string();

    let conn = Connection::open(&src).unwrap();
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
        INSERT INTO measurements (id, date, weight_kg, created_at, updated_at)
        VALUES (1, '2026-07-01', 80.5, '2026-07-01 07:00:00', '2026-07-01 07:00:00');
        "#,
    )
    .unwrap();
    drop(conn);

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
}
