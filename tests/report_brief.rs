//! Integration tests for `report brief`.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn setup_db() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("brief.db").display().to_string();
    (dir, db)
}

fn seed_brief_fixture(db: &str) {
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
            "--carbohydrates-g",
            "66",
            "--fat-g",
            "7",
            "--fiber-g",
            "11",
        ])
        .assert()
        .success();

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
            "80",
            "--unit",
            "g",
            "--date",
            "today",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "today",
            "--weight-kg",
            "80.5",
            "--body-fat-pct",
            "17.2",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "sleep",
            "create",
            "--date",
            "today",
            "--total-sleep",
            "7h 30m",
            "--sleep-score",
            "85",
        ])
        .assert()
        .success();

    bin()
        .args(["--db", db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "exercise",
            "create",
            "bench press",
            "--category",
            "strength",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "set",
            "add",
            "--workout",
            "1",
            "--exercise",
            "bench press",
            "--reps",
            "5",
            "--weight",
            "100",
            "--phase",
            "working",
        ])
        .assert()
        .success();

    // Older workout outside "today" but inside a multi-day lookback when --days is large.
    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "create",
            "--type",
            "Pull",
            "--started-at",
            "2020-01-15 10:00:00",
        ])
        .assert()
        .success();
}

#[test]
fn brief_human_has_all_section_headers() {
    let (_dir, db) = setup_db();
    seed_brief_fixture(&db);

    bin()
        .args(["--db", &db, "report", "brief", "--days", "7"])
        .assert()
        .success()
        .stdout(predicate::str::contains("=== Consumption (today) ==="))
        .stdout(predicate::str::contains(
            "=== Nutrition by day (macronutrients, last 7 days) ===",
        ))
        .stdout(predicate::str::contains(
            "=== Measurements (last 7 days) ===",
        ))
        .stdout(predicate::str::contains("=== Sleep (last 7 days) ==="))
        // Same columns as `body sleep list`.
        .stdout(predicate::str::contains("Time in Bed"))
        .stdout(predicate::str::contains("Total Sleep"))
        .stdout(predicate::str::contains("REM"))
        .stdout(predicate::str::contains("Deep"))
        .stdout(predicate::str::contains("Light"))
        .stdout(predicate::str::contains("Awake"))
        .stdout(predicate::str::contains("Hyp/hr"))
        .stdout(predicate::str::contains("=== Workouts (today) ==="))
        .stdout(predicate::str::contains(
            "=== Workouts overview (previous 7 days:",
        ))
        .stdout(predicate::str::contains("Oats"))
        .stdout(predicate::str::contains("Push"))
        // Today's workout is shown in full detail (workout show shape).
        .stdout(predicate::str::contains("bench press"))
        .stdout(predicate::str::contains("EXERCISES"))
        .stdout(predicate::str::contains("100.00 kg"));
}

#[test]
fn brief_json_shape_and_today_data() {
    let (_dir, db) = setup_db();
    seed_brief_fixture(&db);

    let out = bin()
        .args(["--db", &db, "--json", "report", "brief", "--days", "7"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let v: Value = serde_json::from_slice(&out).expect("json");
    assert!(v["period"]["days"].as_u64() == Some(7) || v["period"]["days"].as_i64() == Some(7));
    assert!(v["period"]["since"].is_string());
    assert!(v["period"]["until"].is_string());

    assert!(v["consumption_today"].is_array());
    assert!(!v["consumption_today"].as_array().unwrap().is_empty());
    assert_eq!(v["consumption_today"][0]["product_name"], "Oats");

    assert_eq!(v["nutrition_daily"]["value"], "macronutrients");
    assert!(v["nutrition_daily"]["days"].is_array());
    assert_eq!(v["nutrition_daily"]["days"].as_array().unwrap().len(), 7);

    assert!(v["measurements"].is_array());
    assert!(!v["measurements"].as_array().unwrap().is_empty());
    assert!(v["sleep"].is_array());
    assert!(!v["sleep"].as_array().unwrap().is_empty());

    assert!(v["workouts"]["today"].is_array());
    assert!(!v["workouts"]["today"].as_array().unwrap().is_empty());
    let today_w = &v["workouts"]["today"][0];
    assert_eq!(today_w["workout_type"], "Push");
    // Full detail: exercises + sets (same shape as `workout show`).
    assert!(today_w["exercises"].is_array());
    assert!(!today_w["exercises"].as_array().unwrap().is_empty());
    assert_eq!(today_w["exercises"][0]["name"], "bench press");
    assert!(today_w["exercises"][0]["sets"].is_array());
    assert!(!today_w["exercises"][0]["sets"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(today_w["exercises"][0]["sets"][0]["reps"], 5);
    assert_eq!(today_w["exercises"][0]["sets"][0]["weight_kg"], 100.0);

    assert!(
        v["workouts"]["previous"]["period"]["days"]
            .as_u64()
            .or_else(|| v["workouts"]["previous"]["period"]["days"]
                .as_i64()
                .map(|n| n as u64))
            == Some(7)
    );
    assert!(v["workouts"]["previous"]["workouts"].is_array());
    // Old 2020 workout is not in the previous 7 days window.
    assert_eq!(v["workouts"]["previous"]["workout_count"], 0);
}

#[test]
fn brief_days_1_still_lists_today_consumption() {
    let (_dir, db) = setup_db();
    seed_brief_fixture(&db);

    let out = bin()
        .args(["--db", &db, "--json", "report", "brief", "--days", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let v: Value = serde_json::from_slice(&out).expect("json");
    assert!(v["period"]["days"].as_u64() == Some(1) || v["period"]["days"].as_i64() == Some(1));
    assert_eq!(v["nutrition_daily"]["days"].as_array().unwrap().len(), 1);
    assert!(!v["consumption_today"].as_array().unwrap().is_empty());
    assert!(!v["workouts"]["today"].as_array().unwrap().is_empty());
}

#[test]
fn brief_empty_db_still_succeeds() {
    let (_dir, db) = setup_db();

    bin()
        .args(["--db", &db, "report", "brief"])
        .assert()
        .success()
        .stdout(predicate::str::contains("=== Consumption (today) ==="))
        .stdout(predicate::str::contains("(no consumptions)"))
        .stdout(predicate::str::contains("=== Workouts (today) ==="));

    let out = bin()
        .args(["--db", &db, "--json", "report", "brief"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(v["consumption_today"].as_array().unwrap().len(), 0);
    assert_eq!(v["workouts"]["today"].as_array().unwrap().len(), 0);
    assert_eq!(v["workouts"]["previous"]["workout_count"], 0);
}

/// Seed product + logs on a fixed historical day (and one workout the day before).
fn seed_brief_on_date(db: &str, date: &str, prev_workout_started: &str) {
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "product",
            "create",
            "Rice",
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
            "130",
            "--protein-g",
            "2.7",
            "--carbohydrates-g",
            "28",
            "--fat-g",
            "0.3",
        ])
        .assert()
        .success();

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
            "150",
            "--date",
            date,
        ])
        .assert()
        .success();

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
            "79.0",
        ])
        .assert()
        .success();

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
            "8h",
            "--sleep-score",
            "90",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "create",
            "--type",
            "Legs",
            "--started-at",
            &format!("{date} 09:00:00"),
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "exercise",
            "create",
            "squat",
            "--category",
            "strength",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "set",
            "add",
            "--workout",
            "1",
            "--exercise",
            "squat",
            "--reps",
            "5",
            "--weight",
            "120",
            "--phase",
            "working",
        ])
        .assert()
        .success();

    // Day before anchor — should land in previous-workouts window for --days 3.
    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "create",
            "--type",
            "Upper",
            "--started-at",
            prev_workout_started,
        ])
        .assert()
        .success();
}

#[test]
fn brief_date_anchors_json_window() {
    let (_dir, db) = setup_db();
    seed_brief_on_date(&db, "2020-06-15", "2020-06-14 18:00:00");

    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "report",
            "brief",
            "--date",
            "2020-06-15",
            "--days",
            "3",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let v: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(v["period"]["until"], "2020-06-15");
    assert_eq!(v["period"]["since"], "2020-06-13");
    assert!(v["period"]["days"].as_u64() == Some(3) || v["period"]["days"].as_i64() == Some(3));

    assert!(!v["consumption_today"].as_array().unwrap().is_empty());
    assert_eq!(v["consumption_today"][0]["product_name"], "Rice");

    assert_eq!(v["nutrition_daily"]["days"].as_array().unwrap().len(), 3);
    assert_eq!(v["nutrition_daily"]["period"]["until"], "2020-06-15");
    assert_eq!(v["nutrition_daily"]["period"]["since"], "2020-06-13");

    assert!(!v["measurements"].as_array().unwrap().is_empty());
    assert!(!v["sleep"].as_array().unwrap().is_empty());

    assert!(!v["workouts"]["today"].as_array().unwrap().is_empty());
    assert_eq!(v["workouts"]["today"][0]["workout_type"], "Legs");
    assert!(v["workouts"]["today"][0]["exercises"].is_array());
    assert_eq!(v["workouts"]["today"][0]["exercises"][0]["name"], "squat");

    assert_eq!(v["workouts"]["previous"]["period"]["until"], "2020-06-14");
    assert_eq!(v["workouts"]["previous"]["period"]["since"], "2020-06-12");
    assert!(v["workouts"]["previous"]["workout_count"].as_i64() >= Some(1));
    // Day-before workout is in previous window.
    let prev_types: Vec<&str> = v["workouts"]["previous"]["workouts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["workout_type"].as_str())
        .collect();
    assert!(prev_types.contains(&"Upper"));
}

#[test]
fn brief_date_human_uses_ymd_label() {
    let (_dir, db) = setup_db();
    seed_brief_on_date(&db, "2020-06-15", "2020-06-14 18:00:00");

    bin()
        .args([
            "--db",
            &db,
            "report",
            "brief",
            "--date",
            "2020-06-15",
            "--days",
            "3",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("=== Consumption (2020-06-15) ==="))
        .stdout(predicate::str::contains("=== Workouts (2020-06-15) ==="))
        .stdout(predicate::str::contains("Rice"))
        .stdout(predicate::str::contains("Legs"))
        .stdout(predicate::str::contains("squat"));
}

#[test]
fn brief_date_yesterday_succeeds() {
    let (_dir, db) = setup_db();
    seed_brief_fixture(&db);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "report",
            "brief",
            "--date",
            "yesterday",
            "--days",
            "1",
        ])
        .assert()
        .success();
}
