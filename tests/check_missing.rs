//! Integration tests for `db check missing` (logging completeness).

use assert_cmd::Command;
use chrono::{Duration, Local};
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn db_path(dir: &TempDir) -> String {
    dir.path().join("t.db").display().to_string()
}

fn ymd_offset(days_ago: i64) -> String {
    let d = Local::now().date_naive() - Duration::days(days_ago);
    d.format("%Y-%m-%d").to_string()
}

/// BA noon for a calendar day as RFC3339 (create-path instant).
fn day_noon_rfc3339(ymd: &str) -> String {
    format!("{ymd}T12:00:00-03:00")
}

fn seed_product(db: &str) {
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "product",
            "create",
            "Oats",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "product",
            "nutrition",
            "set",
            "1",
            "--reference-quantity",
            "100",
            "--reference-unit",
            "g",
            "--energy-kcal",
            "389",
            "--protein-g",
            "17",
        ])
        .assert()
        .success();
}

fn seed_consumption(db: &str, date: &str) {
    let at = day_noon_rfc3339(date);
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "consumption",
            "create",
            "--product",
            "1",
            "--quantity",
            "50",
            "--unit",
            "g",
            "--date",
            &at,
        ])
        .assert()
        .success();
}

fn seed_measurement(db: &str, date: &str) {
    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            date,
            "--weight-kg",
            "80",
        ])
        .assert()
        .success();
}

fn seed_sleep(db: &str, date: &str) {
    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "sleep",
            "create",
            "--date",
            date,
            "--total-sleep",
            "7h",
        ])
        .assert()
        .success();
}

fn seed_daily(db: &str, date: &str) {
    seed_measurement(db, date);
    seed_sleep(db, date);
    seed_consumption(db, date);
}

fn seed_workout_on(db: &str, started_at: &str) {
    // Direct insert so we can place the session on a specific calendar day.
    let conn = Connection::open(db).unwrap();
    conn.execute(
        "INSERT INTO workouts (started_at, workout_type) VALUES (?1, 'Push')",
        [started_at],
    )
    .unwrap();
}

#[test]
fn missing_empty_db_reports_all_gaps() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();

    let assert = bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "check",
            "missing",
            "--days",
            "2",
            "--workout-days",
            "2",
        ])
        .assert()
        .failure();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let v: Value = serde_json::from_str(&stdout).expect(&stdout);

    assert_eq!(v["ok"], false);
    assert_eq!(v["days"], 2);
    assert_eq!(v["workout_days"], 2);
    assert_eq!(v["measurement"]["expected_days"], 2);
    assert_eq!(v["measurement"]["present_days"], 0);
    assert_eq!(
        v["measurement"]["missing_dates"].as_array().unwrap().len(),
        2
    );
    assert_eq!(v["sleep"]["missing_dates"].as_array().unwrap().len(), 2);
    assert_eq!(v["nutrition"]["missing_dates"].as_array().unwrap().len(), 2);
    assert_eq!(v["workout"]["ok"], false);
    assert_eq!(v["workout"]["count"], 0);
    assert!(v["workout"]["last_workout_date"].is_null());
}

#[test]
fn missing_complete_window_exits_ok() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();
    seed_product(&db);

    let today = ymd_offset(0);
    let yesterday = ymd_offset(1);
    seed_daily(&db, &today);
    seed_daily(&db, &yesterday);
    seed_workout_on(&db, &format!("{today}T15:00:00Z")); // BA noon

    let assert = bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "check",
            "missing",
            "--days",
            "2",
            "--workout-days",
            "1",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let v: Value = serde_json::from_str(&stdout).expect(&stdout);

    assert_eq!(v["ok"], true);
    assert!(v["measurement"]["missing_dates"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(v["sleep"]["missing_dates"].as_array().unwrap().is_empty());
    assert!(v["nutrition"]["missing_dates"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(v["workout"]["ok"], true);
    assert!(v["workout"]["count"].as_i64().unwrap() >= 1);
}

#[test]
fn missing_workout_only_fails_when_stale() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();
    seed_product(&db);

    let today = ymd_offset(0);
    seed_daily(&db, &today);

    // Workout 5 days ago — outside a 2-day workout window.
    let old = ymd_offset(5);
    seed_workout_on(&db, &format!("{old} 10:00:00"));

    let assert = bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "check",
            "missing",
            "--days",
            "1",
            "--workout-days",
            "2",
        ])
        .assert()
        .failure();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let v: Value = serde_json::from_str(&stdout).expect(&stdout);

    assert_eq!(v["ok"], false);
    assert!(v["measurement"]["missing_dates"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(v["sleep"]["missing_dates"].as_array().unwrap().is_empty());
    assert!(v["nutrition"]["missing_dates"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(v["workout"]["ok"], false);
    assert_eq!(v["workout"]["count"], 0);
    assert_eq!(v["workout"]["last_workout_date"], old);
    assert_eq!(v["workout"]["days_since_last"], 5);
}

#[test]
fn bare_check_still_runs_sanity_audit() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();

    // Empty DB: sanity audit should succeed (nothing to violate).
    bin()
        .args(["--db", &db, "--json", "db", "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ok\": true"))
        .stdout(predicate::str::contains("\"measurement_count\""))
        .stdout(predicate::str::contains("\"set_count\""));
}

#[test]
fn missing_human_output_mentions_incomplete() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();

    bin()
        .args([
            "--db",
            &db,
            "db",
            "check",
            "missing",
            "--days",
            "1",
            "--workout-days",
            "1",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Missing-entry check"))
        .stdout(predicate::str::contains("measurement:"))
        .stdout(predicate::str::contains("INCOMPLETE"));
}

#[test]
fn missing_skip_today_succeeds_with_yesterday_only() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();
    seed_product(&db);

    let today = ymd_offset(0);
    let yesterday = ymd_offset(1);
    seed_daily(&db, &yesterday);
    seed_workout_on(&db, &format!("{yesterday}T15:00:00Z"));

    let assert = bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "check",
            "missing",
            "--days",
            "1",
            "--workout-days",
            "1",
            "--skip-today",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let v: Value = serde_json::from_str(&stdout).expect(&stdout);

    assert_eq!(v["ok"], true);
    assert_eq!(v["skip_today"], true);
    assert_eq!(v["period"]["until"], yesterday);
    assert_ne!(v["period"]["until"], today);
    assert!(v["measurement"]["missing_dates"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(v["sleep"]["missing_dates"].as_array().unwrap().is_empty());
    assert!(v["nutrition"]["missing_dates"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(v["workout"]["ok"], true);
}

#[test]
fn missing_without_skip_today_fails_when_today_empty() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();
    seed_product(&db);

    let today = ymd_offset(0);
    let yesterday = ymd_offset(1);
    seed_daily(&db, &yesterday);
    seed_workout_on(&db, &format!("{yesterday}T15:00:00Z"));

    let assert = bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "check",
            "missing",
            "--days",
            "1",
            "--workout-days",
            "1",
        ])
        .assert()
        .failure();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let v: Value = serde_json::from_str(&stdout).expect(&stdout);

    assert_eq!(v["ok"], false);
    assert_eq!(v["skip_today"], false);
    assert_eq!(v["period"]["until"], today);
    let missing = v["measurement"]["missing_dates"].as_array().unwrap();
    assert!(
        missing.iter().any(|d| d.as_str() == Some(today.as_str())),
        "today should be missing: {missing:?}"
    );
}

#[test]
fn missing_skip_today_preserves_window_length() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();

    let today = ymd_offset(0);
    let yesterday = ymd_offset(1);
    let day_before = ymd_offset(2);

    let assert = bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "check",
            "missing",
            "--days",
            "2",
            "--workout-days",
            "2",
            "--skip-today",
        ])
        .assert()
        .failure();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let v: Value = serde_json::from_str(&stdout).expect(&stdout);

    assert_eq!(v["skip_today"], true);
    assert_eq!(v["measurement"]["expected_days"], 2);
    assert_eq!(v["period"]["until"], yesterday);
    assert_eq!(v["period"]["since"], day_before);

    let missing = v["measurement"]["missing_dates"].as_array().unwrap();
    assert_eq!(missing.len(), 2);
    let missing_strs: Vec<&str> = missing.iter().filter_map(|d| d.as_str()).collect();
    assert!(missing_strs.contains(&yesterday.as_str()));
    assert!(missing_strs.contains(&day_before.as_str()));
    assert!(!missing_strs.contains(&today.as_str()));
}
