//! Integration tests for `db check append` (F3a orphans + F3b triggers).

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
    // Migrations applied via CLI open first (creates _recomplog_write_allow + triggers).
    Connection::open(db).expect("open db")
}

/// Bypass F3b triggers for test setup that deliberately creates orphan rows.
fn with_raw_allow(conn: &Connection, op: &str, f: impl FnOnce(&Connection)) {
    conn.execute("INSERT INTO _recomplog_write_allow (op) VALUES (?1)", [op])
        .unwrap();
    f(conn);
    let _ = conn.execute(
        "DELETE FROM _recomplog_write_allow WHERE rowid = (
            SELECT MAX(rowid) FROM _recomplog_write_allow WHERE op = ?1
        )",
        [op],
    );
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
    assert!(v["schema"]
        .get("missing_triggers")
        .map(|t| t.as_array().map(|a| a.is_empty()).unwrap_or(true))
        .unwrap_or(true));
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

    // Soft-delete without entity_audit (write allow only for trigger bypass).
    let conn = open(&db);
    with_raw_allow(&conn, "soft_delete", |conn| {
        conn.execute(
            "UPDATE measurements SET deleted_at = '2026-07-19T12:00:00Z', delete_reason = 'raw'
             WHERE id = 1",
            [],
        )
        .unwrap();
    });
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
    with_raw_allow(&conn, "correct", |conn| {
        conn.execute(
            "UPDATE measurements SET weight_kg = 79.0, updated_at = '2026-07-19T15:00:00Z'
             WHERE id = 1",
            [],
        )
        .unwrap();
    });
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
fn raw_event_update_without_allow_is_denied() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    let conn = open(&db);
    let err = conn
        .execute("UPDATE measurements SET weight_kg = 70.0 WHERE id = 1", [])
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("append-only") || msg.contains("WRITE ALLOW") || msg.contains("write allow"),
        "expected append-only abort, got: {msg}"
    );
}

#[test]
fn raw_entity_audit_delete_is_denied() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    let conn = open(&db);
    // Ensure at least one audit row exists (create wrote one).
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entity_audit", [], |r| r.get(0))
        .unwrap();
    assert!(count >= 1);

    let err = conn
        .execute("DELETE FROM entity_audit WHERE id = 1", [])
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("append-only") || msg.contains("insert-only"),
        "expected insert-only abort, got: {msg}"
    );
}

#[test]
fn missing_trigger_fails_check() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);
    seed_minimal(&db);

    let conn = open(&db);
    conn.execute("DROP TRIGGER ao_workouts_update_guard", [])
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
    assert_eq!(v["schema"]["ok"], false);
    let missing = v["schema"]["missing_triggers"].as_array().unwrap();
    assert!(missing
        .iter()
        .any(|t| t.as_str() == Some("ao_workouts_update_guard")));
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
