use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn db_path(dir: &TempDir) -> String {
    dir.path().join("t.db").display().to_string()
}

#[test]
fn measurement_create_list_show_json() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "today",
            "--weight-kg",
            "81.2",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "list",
            "--days",
            "7",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("81.2"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "show",
            "--date",
            "today",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("weight_kg"));
}

#[test]
fn measurement_update_delete() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-01-01",
            "--weight-kg",
            "80",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "update",
            "--date",
            "2026-01-01",
            "--weight-kg",
            "79.5",
            "--reason",
            "scale typo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"))
        .stdout(predicate::str::contains("\"kind\": \"correction\""));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "delete",
            "--date",
            "2026-01-01",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted_id"));
}

#[test]
fn measurement_medians_window_and_sparse() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    // 5 consecutive days with weights 70..74; body fat only on some days
    let seeds = [
        ("2026-01-01", "70", Some("20")),
        ("2026-01-02", "71", Some("21")),
        ("2026-01-03", "72", None), // no BF
        ("2026-01-04", "73", Some("23")),
        ("2026-01-05", "74", Some("24")),
    ];
    for (date, w, bf) in seeds {
        let mut args = vec![
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            date,
            "--weight-kg",
            w,
        ];
        if let Some(bf) = bf {
            args.extend(["--body-fat-pct", bf]);
        }
        bin().args(&args).assert().success();
    }

    // window 3 on 2026-01-05: weights 72,73,74 → median 73; n=3
    // BF: none, 23, 24 → median 23.5; n_by_field bf=2 → human "3 bf:2"
    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "medians",
            "--window",
            "3",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let row = &v[0];
    assert_eq!(row["date"], "2026-01-05");
    assert_eq!(row["n"], 3);
    assert!((row["weight_kg"].as_f64().unwrap() - 73.0).abs() < 1e-9);
    assert!((row["body_fat_pct"].as_f64().unwrap() - 23.5).abs() < 1e-9);
    assert_eq!(row["n_by_field"]["weight_kg"], 3);
    assert_eq!(row["n_by_field"]["body_fat_pct"], 2);
    assert_eq!(row["n_by_field"]["bmi"], 0);

    // Gap: no measurement on 2026-01-03 — use a fresh DB with gap for n < window
    let dir2 = TempDir::new().unwrap();
    let db2 = db_path(&dir2);
    for (date, w) in [("2026-02-01", "80"), ("2026-02-03", "82")] {
        bin()
            .args([
                "--db",
                &db2,
                "body",
                "measurement",
                "create",
                "--date",
                date,
                "--weight-kg",
                w,
            ])
            .assert()
            .success();
    }
    // On 2026-02-03 window 3 calendar days [02-01, 02-03]: only 2 rows → n=2, median (80+82)/2=81
    let out = bin()
        .args([
            "--db",
            &db2,
            "--json",
            "body",
            "measurement",
            "medians",
            "--window",
            "3",
            "--since",
            "2026-02-03",
            "--until",
            "2026-02-03",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v[0]["n"], 2);
    assert!((v[0]["weight_kg"].as_f64().unwrap() - 81.0).abs() < 1e-9);

    // Lookback beyond display: display only 2026-01-05, window 5 needs 01-01..01-05
    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "medians",
            "--window",
            "5",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    // weights 70,71,72,73,74 → median 72
    assert_eq!(v[0]["n"], 5);
    assert!((v[0]["weight_kg"].as_f64().unwrap() - 72.0).abs() < 1e-9);

    // Human path includes N header / sparse suffix
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "medians",
            "--window",
            "3",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("N"))
        .stdout(predicate::str::contains("bf:2"));

    // Invalid window
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "medians",
            "--window",
            "0",
            "--days",
            "7",
        ])
        .assert()
        .failure();

    // Default window is 7 (no --window flag)
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "medians",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"n\": 5")); // all 5 seeded days in 7-day window
}

#[test]
fn sleep_create_and_list() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "sleep",
            "create",
            "--date",
            "today",
            "--total-sleep",
            "7h 30m",
            "--rem",
            "90",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    bin()
        .args([
            "--db", &db, "--json", "body", "sleep", "list", "--days", "3",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("total_sleep_minutes"));
}

/// S1: same-day create always appends (no UNIQUE day / no upsert).
#[test]
fn measurement_same_day_multi_sample_append() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-10",
            "--weight-kg",
            "81.0",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-10",
            "--weight-kg",
            "81.5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "list",
            "--since",
            "2026-07-10",
            "--until",
            "2026-07-10",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let weights: Vec<f64> = arr.iter().filter_map(|r| r["weight_kg"].as_f64()).collect();
    assert!(weights.contains(&81.0));
    assert!(weights.contains(&81.5));

    // Show by date → latest sample (81.5, higher id / later created_at).
    let show = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "show",
            "--date",
            "2026-07-10",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s: serde_json::Value = serde_json::from_slice(&show).unwrap();
    assert_eq!(s["weight_kg"].as_f64().unwrap(), 81.5);

    // Update by date is ambiguous when multiple rows exist.
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "update",
            "--date",
            "2026-07-10",
            "--weight-kg",
            "80.0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("multiple measurements"));

    // Delete by date also ambiguous; delete by id works.
    let id = s["id"].as_i64().unwrap();
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "delete",
            "--date",
            "2026-07-10",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("multiple measurements"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "delete",
            "--id",
            &id.to_string(),
        ])
        .assert()
        .success();

    let out2 = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "list",
            "--since",
            "2026-07-10",
            "--until",
            "2026-07-10",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v2: serde_json::Value = serde_json::from_slice(&out2).unwrap();
    assert_eq!(v2.as_array().unwrap().len(), 1);
}

#[test]
fn sleep_same_day_multi_sample_append() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "body",
            "sleep",
            "create",
            "--date",
            "2026-07-10",
            "--total-sleep",
            "6h",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "body",
            "sleep",
            "create",
            "--date",
            "2026-07-10",
            "--total-sleep",
            "1h 30m",
            "--notes",
            "nap",
        ])
        .assert()
        .success();

    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "sleep",
            "list",
            "--since",
            "2026-07-10",
            "--until",
            "2026-07-10",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
}

#[test]
fn measurement_medians_collapses_same_day_samples() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    // Day1 one sample; day2 two samples; day3 one sample.
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-02-01",
            "--weight-kg",
            "70",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-02-02",
            "--weight-kg",
            "71",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-02-02",
            "--weight-kg",
            "72",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-02-03",
            "--weight-kg",
            "73",
        ])
        .assert()
        .success();

    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "medians",
            "--window",
            "3",
            "--since",
            "2026-02-01",
            "--until",
            "2026-02-03",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&out).unwrap();
    // One median row per distinct date (not one per sample).
    assert_eq!(rows.len(), 3);
    // Window on 2026-02-03 uses daily-last: 70, 72, 73 → median 72, n=3
    let last = rows
        .iter()
        .find(|r| r["date"] == "2026-02-03")
        .expect("row for 2026-02-03");
    assert_eq!(last["n"].as_i64().unwrap(), 3);
    assert_eq!(last["weight_kg"].as_f64().unwrap(), 72.0);
}

/// Upgrade from UNIQUE(date) schema (user_version 7) drops day uniqueness.
#[test]
fn migration_v8_drops_unique_date_allows_multi_sample() {
    use rusqlite::Connection;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("v7body.db");
    let path_s = path.display().to_string();

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            PRAGMA user_version = 7;
            CREATE TABLE measurements (
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
            CREATE TABLE sleep (
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
            CREATE TABLE user_profile (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                height_cm REAL,
                date_of_birth TEXT,
                updated_at TEXT
            );
            INSERT INTO measurements (id, date, weight_kg, created_at, updated_at)
            VALUES (1, '2026-07-10', 80.0, '2026-07-10T08:00:00Z', '2026-07-10T08:00:00Z');
            "#,
        )
        .unwrap();
    }

    // Second sample same day must succeed after v8 migration.
    bin()
        .args([
            "--db",
            &path_s,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-10",
            "--weight-kg",
            "80.5",
        ])
        .assert()
        .success();

    let conn = Connection::open(&path).unwrap();
    let ver: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert!(ver >= 8, "expected user_version >= 8, got {ver}");
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM measurements WHERE date = '2026-07-10'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
}
