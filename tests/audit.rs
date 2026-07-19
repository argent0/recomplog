//! S7a: `audit` CLI on all matrix entities (current + synthetic/create history).

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

fn history_kinds(audit: &Value) -> Vec<&str> {
    audit["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["kind"].as_str())
        .collect()
}

#[test]
fn measurement_audit_synthetic_create_and_soft_delete() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    let created = json_stdout(
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
            ])
            .assert()
            .success(),
    );
    let id = created["id"].as_i64().unwrap();

    let audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "body",
                "measurement",
                "audit",
                "--id",
                &id.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(audit["entity"], "measurement");
    assert_eq!(audit["id"], id);
    assert_eq!(audit["current"]["weight_kg"], 81.2);
    assert!(audit["current"]["created_at"].as_str().is_some());
    let kinds = history_kinds(&audit);
    assert!(
        kinds.contains(&"create"),
        "expected synthetic or real create: {audit}"
    );

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
                &id.to_string(),
                "--reason",
                "typo",
            ])
            .assert()
            .success(),
    );
    assert_eq!(del["mode"], "soft_delete");

    let audit2 = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "body",
                "measurement",
                "audit",
                "--id",
                &id.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(audit2["current"]["deleted_at"], del["deleted_at"]);
    assert!(history_kinds(&audit2).contains(&"soft_delete"));
}

#[test]
fn measurement_audit_by_date_multi_sample() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    for w in ["80.0", "80.5"] {
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "body",
                "measurement",
                "create",
                "--date",
                "2026-07-11",
                "--weight-kg",
                w,
            ])
            .assert()
            .success();
    }

    let audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "body",
                "measurement",
                "audit",
                "--date",
                "2026-07-11",
            ])
            .assert()
            .success(),
    );
    assert_eq!(audit["count"], 2);
    assert_eq!(audit["date"], "2026-07-11");
    let samples = audit["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0]["entity"], "measurement");
}

#[test]
fn sleep_audit_by_id() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    let created = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "body",
                "sleep",
                "create",
                "--date",
                "2026-07-10",
                "--total-sleep",
                "7h 30m",
            ])
            .assert()
            .success(),
    );
    let id = created["id"].as_i64().unwrap();

    let audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "body",
                "sleep",
                "audit",
                "--id",
                &id.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(audit["entity"], "sleep");
    assert!(history_kinds(&audit).contains(&"create"));
}

#[test]
fn product_store_micronutrient_exercise_audit() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    let product = json_stdout(
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
            .success(),
    );
    let pid = product["id"].as_i64().unwrap();
    let pa = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "product",
                "audit",
                &pid.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(pa["entity"], "product");
    assert_eq!(pa["current"]["name"], "Oats");
    assert!(history_kinds(&pa).contains(&"create"));

    let store = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "store",
                "create",
                "Market",
            ])
            .assert()
            .success(),
    );
    let sid = store["id"].as_i64().unwrap();
    let sa = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "store",
                "audit",
                &sid.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(sa["entity"], "store");
    assert!(history_kinds(&sa).contains(&"create"));

    let micro = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "micronutrient",
                "create",
                "CustomZyme",
                "--unit",
                "mg",
                "--force",
            ])
            .assert()
            .success(),
    );
    let mid = micro["id"].as_i64().unwrap();
    let ma = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "micronutrient",
                "audit",
                &mid.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(ma["entity"], "micronutrient");
    assert!(history_kinds(&ma).contains(&"create"));

    let ex = json_stdout(
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
            .success(),
    );
    let eid = ex["id"].as_i64().unwrap();
    let ea = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "workout",
                "exercise",
                "audit",
                &eid.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(ea["entity"], "exercise");
    assert!(history_kinds(&ea).contains(&"create"));
}

#[test]
fn workout_audit_has_synthetic_create_before_delete() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();

    let audit = json_stdout(
        bin()
            .args(["--db", &db, "--json", "workout", "audit", "1"])
            .assert()
            .success(),
    );
    assert_eq!(audit["entity"], "workout");
    assert!(history_kinds(&audit).contains(&"create"));
    assert_eq!(audit["history"][0]["meta"]["synthetic"], true);
}

#[test]
fn audit_unknown_id_fails() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "product",
            "audit",
            "999",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "audit",
            "--id",
            "999",
        ])
        .assert()
        .failure();
}
