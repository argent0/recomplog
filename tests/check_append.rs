//! Integration tests for `db check append` (F3a append-only integrity).

use assert_cmd::Command;
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

fn open(db: &str) -> Connection {
    // Migrations applied via CLI open first.
    Connection::open(db).expect("open db")
}

fn seed_minimal(db: &str) {
    // Touch DB so migrations run.
    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-10",
            "--weight-kg",
            "80",
        ])
        .assert()
        .success();
}

#[test]
fn clean_db_passes_append_check() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    let output = bin()
        .args(["--db", &db, "--json", "db", "check", "append"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&output).expect("json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["schema"]["ok"], true);
    assert_eq!(v["orphan_soft_deletes"]["ok"], true);
    assert_eq!(v["orphan_soft_deletes"]["count"], 0);
    assert_eq!(v["orphan_updates"]["ok"], true);
    assert_eq!(v["orphan_updates"]["count"], 0);
    assert!(
        v["policy"]["allowed_event_updates"]
            .as_array()
            .unwrap()
            .len()
            >= 1
    );
}

#[test]
fn soft_delete_via_cli_still_passes() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "delete",
            "--id",
            "1",
            "--reason",
            "test delete",
        ])
        .assert()
        .success();

    let output = bin()
        .args(["--db", &db, "--json", "db", "check", "append"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&output).expect("json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["orphan_soft_deletes"]["count"], 0);
}

#[test]
fn orphan_soft_delete_fails() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    // Raw SQL soft-delete without entity_audit — append-only violation.
    let conn = open(&db);
    conn.execute(
        "UPDATE measurements SET deleted_at = '2026-07-19T12:00:00Z', delete_reason = 'raw'
         WHERE id = 1",
        [],
    )
    .unwrap();
    drop(conn);

    let output = bin()
        .args(["--db", &db, "--json", "db", "check", "append"])
        .assert()
        .failure()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&output).expect("json");
    assert_eq!(v["ok"], false);
    assert_eq!(v["orphan_soft_deletes"]["ok"], false);
    assert!(v["orphan_soft_deletes"]["count"].as_u64().unwrap() >= 1);
    let findings = v["orphan_soft_deletes"]["findings"].as_array().unwrap();
    assert!(findings
        .iter()
        .any(|f| f["entity"] == "measurement" && f["id"] == 1));
}

#[test]
fn orphan_update_fails() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    let conn = open(&db);
    conn.execute(
        "UPDATE measurements SET weight_kg = 79.0, updated_at = '2026-07-19T15:00:00Z'
         WHERE id = 1",
        [],
    )
    .unwrap();
    drop(conn);

    let output = bin()
        .args(["--db", &db, "--json", "db", "check", "append"])
        .assert()
        .failure()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&output).expect("json");
    assert_eq!(v["ok"], false);
    assert_eq!(v["orphan_updates"]["ok"], false);
    assert!(v["orphan_updates"]["count"].as_u64().unwrap() >= 1);
    let findings = v["orphan_updates"]["findings"].as_array().unwrap();
    assert!(findings
        .iter()
        .any(|f| f["entity"] == "measurement" && f["id"] == 1));
}

#[test]
fn in_place_update_via_cli_passes() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "update",
            "--id",
            "1",
            "--weight-kg",
            "79.5",
            "--reason",
            "scale typo",
        ])
        .assert()
        .success();

    bin()
        .args(["--db", &db, "--json", "db", "check", "append"])
        .assert()
        .success();
}

#[test]
fn human_quiet_ok() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    bin()
        .args(["--db", &db, "--quiet", "db", "check", "append"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));
}
