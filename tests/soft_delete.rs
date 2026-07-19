//! S3: soft-delete default, cascade dry-run, purge force, audit trail.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn json_stdout(assert: assert_cmd::assert::Assert) -> Value {
    let out = assert.get_output().stdout.clone();
    let s = String::from_utf8_lossy(&out);
    serde_json::from_str(s.trim()).expect("valid json")
}

#[test]
fn workout_soft_delete_hides_from_list_and_volume() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
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
            &db,
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
        ])
        .assert()
        .success();

    let del = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "workout",
                "delete",
                "1",
                "--reason",
                "abandoned",
            ])
            .assert()
            .success(),
    );
    assert_eq!(del["mode"], "soft_delete");
    assert!(del["deleted_at"].as_str().is_some());
    assert_eq!(del["cascade"]["exercise_sets"], 1);

    bin()
        .args(["--db", &db, "--json", "workout", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 1").not());

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .failure();

    let vol = json_stdout(
        bin()
            .args([
                "--db", &db, "--json", "workout", "stats", "volume", "--days", "30",
            ])
            .assert()
            .success(),
    );
    let by = vol["by_exercise"].as_array().cloned().unwrap_or_default();
    assert!(
        by.iter()
            .all(|r| r["exercise"].as_str() != Some("bench press")),
        "soft-deleted workout sets must not appear in volume: {vol}"
    );

    let audit = json_stdout(
        bin()
            .args(["--db", &db, "--json", "workout", "audit", "1"])
            .assert()
            .success(),
    );
    assert_eq!(audit["entity"], "workout");
    assert_eq!(audit["current"]["deleted_at"], del["deleted_at"]);
    let kinds: Vec<_> = audit["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"soft_delete"));
}

#[test]
fn workout_delete_dry_run_reports_cascade() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
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
            &db,
            "--json",
            "workout",
            "set",
            "add",
            "--workout",
            "1",
            "--exercise",
            "squat",
            "--reps",
            "3",
            "--weight",
            "120",
        ])
        .assert()
        .success();

    let dry = json_stdout(
        bin()
            .args(["--db", &db, "--json", "workout", "delete", "1", "--dry-run"])
            .assert()
            .success(),
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["would"]["mode"], "soft_delete");
    assert_eq!(dry["would"]["cascade"]["exercise_sets"], 1);

    // Still listed after dry-run
    bin()
        .args(["--db", &db, "--json", "workout", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 1"));
}

#[test]
fn workout_purge_requires_force_when_sets_exist() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "exercise",
            "create",
            "row",
            "--category",
            "strength",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "add",
            "--workout",
            "1",
            "--exercise",
            "row",
            "--reps",
            "8",
            "--weight",
            "60",
        ])
        .assert()
        .success();

    bin()
        .args(["--db", &db, "--json", "workout", "delete", "1", "--purge"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));

    let purged = json_stdout(
        bin()
            .args([
                "--db", &db, "--json", "workout", "delete", "1", "--purge", "--force",
            ])
            .assert()
            .success(),
    );
    assert_eq!(purged["mode"], "purge");

    let audit = json_stdout(
        bin()
            .args(["--db", &db, "--json", "workout", "audit", "1"])
            .assert()
            .success(),
    );
    assert!(audit["current"].is_null());
    let kinds: Vec<_> = audit["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"purge"));
}

#[test]
fn consumption_soft_delete_excluded_from_list() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
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
            &db,
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
            "380",
            "--protein-g",
            "13",
            "--carbohydrates-g",
            "60",
            "--fat-g",
            "7",
            "--fiber-g",
            "10",
            "--sugars-g",
            "1",
        ])
        .assert()
        .success();
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
            "80",
            "--unit",
            "g",
            "--consumed-at",
            "2026-07-14T08:30:00-03:00",
        ])
        .assert()
        .success();

    let del = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "delete",
                "1",
                "--reason",
                "duplicate",
            ])
            .assert()
            .success(),
    );
    assert_eq!(del["mode"], "soft_delete");

    bin()
        .args(["--db", &db, "--json", "nutrition", "consumption", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 1").not());

    let audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "audit",
                "1",
            ])
            .assert()
            .success(),
    );
    assert!(audit["current"]["deleted_at"].as_str().is_some());
}

#[test]
fn measurement_soft_delete_hides_from_list() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

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
            "81.2",
            "--no-sanity-check",
        ])
        .assert()
        .success();

    let del = json_stdout(
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
            ])
            .assert()
            .success(),
    );
    assert_eq!(del["mode"], "soft_delete");

    bin()
        .args(["--db", &db, "--json", "body", "measurement", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 1").not());
}

#[test]
fn double_soft_delete_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Pull"])
        .assert()
        .success();
    bin()
        .args(["--db", &db, "--json", "workout", "delete", "1"])
        .assert()
        .success();
    bin()
        .args(["--db", &db, "--json", "workout", "delete", "1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already soft-deleted"));
}
