//! Event time (when it happened) must differ from storage time (when logged).

use assert_cmd::Command;
use chrono::{Duration, Local, Utc};
use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn db_path(dir: &TempDir) -> String {
    dir.path().join("t.db").display().to_string()
}

fn setup_product(db: &str) {
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
            "10",
            "--fat-g",
            "5",
            "--fiber-g",
            "0",
            "--sugars-g",
            "0",
        ])
        .assert()
        .success();
}

#[test]
fn consumption_backdate_separates_event_and_storage() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    bin().args(["--db", &db, "init"]).assert().success();
    setup_product(&db);

    let event = "2020-06-15T09:00:00-03:00";
    let out = bin()
        .args([
            "--db",
            &db,
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
            "--consumed-at",
            event,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(v["success"], true);
    assert_eq!(v["consumed_at"], "2020-06-15T12:00:00Z");
    let created = v["created_at"].as_str().expect("created_at");
    assert!(created.ends_with('Z'), "created_at={created}");
    assert_ne!(v["consumed_at"], v["created_at"]);

    // created_at should be near now (not 2020)
    let now = Utc::now();
    let created_dt = chrono::DateTime::parse_from_rfc3339(created).unwrap();
    let lag = now.signed_duration_since(created_dt.with_timezone(&Utc));
    assert!(lag.num_seconds().abs() < 60, "created_at lag={lag}");

    // List still buckets by event day
    let list = bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "consumption",
            "list",
            "--since",
            "2020-06-15",
            "--until",
            "2020-06-15",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rows: Value = serde_json::from_slice(&list).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["consumed_at"], "2020-06-15T12:00:00Z");
    assert!(rows[0]["created_at"].as_str().is_some());
}

#[test]
fn purchase_backdate_separates_event_and_storage() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    bin().args(["--db", &db, "init"]).assert().success();
    setup_product(&db);

    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "purchase",
            "create",
            "--product",
            "1",
            "--quantity",
            "2",
            "--purchased-at",
            "2020-06-14T18:00:00-03:00",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(v["purchased_at"], "2020-06-14T21:00:00Z");
    assert_ne!(v["purchased_at"], v["created_at"]);
    assert!(v["created_at"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn workout_backdate_separates_event_and_storage() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    bin().args(["--db", &db, "init"]).assert().success();

    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "create",
            "--type",
            "Push",
            "--started-at",
            "2020-01-15T10:00:00Z",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(v["started_at"], "2020-01-15T10:00:00Z");
    assert_ne!(v["started_at"], v["created_at"]);
    assert!(v["created_at"].as_str().unwrap().ends_with('Z'));

    let show = bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let w: Value = serde_json::from_slice(&show).unwrap();
    assert_eq!(w["started_at"], "2020-01-15T10:00:00Z");
    assert!(w["created_at"].as_str().is_some());
}

#[test]
fn measurement_event_day_differs_from_storage_day() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    bin().args(["--db", &db, "init"]).assert().success();

    let yesterday = (Local::now().date_naive() - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            &yesterday,
            "--weight-kg",
            "80",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(v["date"], yesterday);
    let created = v["created_at"].as_str().expect("created_at");
    assert!(
        created.ends_with('Z') && created.contains('T'),
        "created_at={created}"
    );
    // Event is a calendar day string; storage is a full instant near now (not the event day alone).
    assert_ne!(created, yesterday);
    let created_dt = chrono::DateTime::parse_from_rfc3339(created).unwrap();
    let lag = Utc::now().signed_duration_since(created_dt.with_timezone(&Utc));
    assert!(lag.num_seconds().abs() < 60, "created_at lag={lag}");
}

#[test]
fn date_alias_still_works_for_consumption() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    bin().args(["--db", &db, "init"]).assert().success();
    setup_product(&db);
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "consumption",
            "create",
            "--product",
            "1",
            "--quantity",
            "10",
            "--unit",
            "g",
            "--date",
            "2020-06-15T12:00:00Z",
        ])
        .assert()
        .success();
    let conn = Connection::open(&db).unwrap();
    let (c_at, cr_at): (String, String) = conn
        .query_row(
            "SELECT consumed_at, created_at FROM consumptions WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(c_at, "2020-06-15T12:00:00Z");
    assert_ne!(c_at, cr_at);
}
