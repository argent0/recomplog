//! F4: append-only session set order (`set_order_revisions`).
//! Move never rewrites sibling `exercise_sets.set_number`.

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn add_set(db: &str, workout: i64, exercise: &str, reps: i32, weight: f64) {
    bin()
        .args([
            "--db",
            db,
            "workout",
            "set",
            "add",
            "--workout",
            &workout.to_string(),
            "--exercise",
            exercise,
            "--reps",
            &reps.to_string(),
            "--weight",
            &weight.to_string(),
            "--phase",
            "full",
        ])
        .assert()
        .success();
}

fn frozen_set_numbers(db: &str) -> Vec<(i64, i64)> {
    let conn = Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, set_number FROM exercise_sets
             WHERE deleted_at IS NULL ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn workout_set_ids_display(db: &str, workout_id: i64) -> Vec<(i64, i64)> {
    let out = bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "show",
            &workout_id.to_string(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    // workout show: exercises[].sets[] with id + set_number
    let mut pairs = Vec::new();
    if let Some(exercises) = v["exercises"].as_array() {
        for ex in exercises {
            if let Some(sets) = ex["sets"].as_array() {
                for s in sets {
                    pairs.push((s["id"].as_i64().unwrap(), s["set_number"].as_i64().unwrap()));
                }
            }
        }
    }
    pairs
}

fn revision_count(db: &str) -> i64 {
    let conn = Connection::open(db).unwrap();
    conn.query_row("SELECT COUNT(*) FROM set_order_revisions", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn legacy_order_without_revision() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    add_set(&db, 1, "bench press", 5, 100.0);
    add_set(&db, 1, "bench press", 5, 105.0);
    add_set(&db, 1, "bench press", 5, 110.0);

    assert_eq!(revision_count(&db), 0);
    let frozen = frozen_set_numbers(&db);
    assert_eq!(frozen, vec![(1, 1), (2, 2), (3, 3)]);
    let display = workout_set_ids_display(&db, 1);
    assert_eq!(display, vec![(1, 1), (2, 2), (3, 3)]);
}

#[test]
fn move_does_not_rewrite_sibling_set_number() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    add_set(&db, 1, "bench press", 5, 100.0);
    add_set(&db, 1, "bench press", 5, 105.0);
    add_set(&db, 1, "bench press", 5, 110.0);

    let before = frozen_set_numbers(&db);
    assert_eq!(before, vec![(1, 1), (2, 2), (3, 3)]);

    let move_out = bin()
        .args([
            "--db", &db, "--json", "workout", "set", "move", "3", "--to", "1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mv: Value = serde_json::from_slice(&move_out).unwrap();
    assert_eq!(mv["success"], true);
    assert!(mv["revision_id"].as_i64().unwrap() >= 1);
    assert_eq!(mv["from"], 3);
    assert_eq!(mv["to"], 1);
    assert_eq!(
        mv["order_before"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        mv["order_after"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![3, 1, 2]
    );

    // Frozen insert-time set_number unchanged on all rows.
    let after = frozen_set_numbers(&db);
    assert_eq!(after, before);

    // Display order derived from revision.
    let display = workout_set_ids_display(&db, 1);
    assert_eq!(display, vec![(3, 1), (1, 2), (2, 3)]);

    assert_eq!(revision_count(&db), 1);

    // Audit shows kind move with revision linkage.
    bin()
        .args(["--db", &db, "--json", "workout", "set", "audit", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\": \"move\""))
        .stdout(predicate::str::contains("revision_id"));
}

#[test]
fn move_then_add_appends_new_set_at_end() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    add_set(&db, 1, "bench press", 5, 100.0);
    add_set(&db, 1, "bench press", 5, 105.0);
    add_set(&db, 1, "bench press", 5, 110.0);

    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "move", "3", "--to", "1",
        ])
        .assert()
        .success();

    add_set(&db, 1, "bench press", 3, 115.0);

    let frozen = frozen_set_numbers(&db);
    assert_eq!(frozen, vec![(1, 1), (2, 2), (3, 3), (4, 4)]);

    // Effective: revised [3,1,2] then new set 4 at end.
    let display = workout_set_ids_display(&db, 1);
    assert_eq!(display, vec![(3, 1), (1, 2), (2, 3), (4, 4)]);
}

#[test]
fn soft_delete_omitted_from_effective_order() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    add_set(&db, 1, "bench press", 5, 100.0);
    add_set(&db, 1, "bench press", 5, 105.0);
    add_set(&db, 1, "bench press", 5, 110.0);

    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "delete", "2", "--reason", "mislog",
        ])
        .assert()
        .success();

    // No revision required for soft-delete.
    assert_eq!(revision_count(&db), 0);

    let display = workout_set_ids_display(&db, 1);
    assert_eq!(display, vec![(1, 1), (3, 2)]);
}

#[test]
fn dry_run_move_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    add_set(&db, 1, "bench press", 5, 100.0);
    add_set(&db, 1, "bench press", 5, 105.0);

    let before = frozen_set_numbers(&db);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "move",
            "2",
            "--to",
            "1",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\": true"))
        .stdout(predicate::str::contains("order_before"))
        .stdout(predicate::str::contains("order_after"));

    assert_eq!(revision_count(&db), 0);
    assert_eq!(frozen_set_numbers(&db), before);

    // No move audit on dry-run.
    let audit = bin()
        .args(["--db", &db, "--json", "workout", "set", "audit", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let a: Value = serde_json::from_slice(&audit).unwrap();
    let history = a["history"].as_array().unwrap();
    assert!(
        !history.iter().any(|h| h["kind"] == "move"),
        "dry-run must not write move audit"
    );
}

#[test]
fn noop_move_no_revision() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    add_set(&db, 1, "bench press", 5, 100.0);
    add_set(&db, 1, "bench press", 5, 105.0);

    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "move", "1", "--to", "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("already at position"));

    assert_eq!(revision_count(&db), 0);
}

#[test]
fn move_then_soft_delete_filters_revision_ids() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    add_set(&db, 1, "bench press", 5, 100.0);
    add_set(&db, 1, "bench press", 5, 105.0);
    add_set(&db, 1, "bench press", 5, 110.0);

    // Order becomes [3,1,2]
    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "move", "3", "--to", "1",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "delete", "1", "--reason", "drop",
        ])
        .assert()
        .success();

    // Revision still has 1; effective order filters it → [3,2] display 1..2
    let display = workout_set_ids_display(&db, 1);
    assert_eq!(display, vec![(3, 1), (2, 2)]);
}
