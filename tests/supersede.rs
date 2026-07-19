//! F1a: consumption/purchase supersede (correct) chains.

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
            "380",
            "--protein-g",
            "13",
            "--carbohydrates-g",
            "67",
            "--fat-g",
            "7",
            "--fiber-g",
            "10",
            "--sugars-g",
            "1",
        ])
        .assert()
        .success();
}

#[test]
fn consumption_correct_supersedes_and_hides_old() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    seed_product(&db);

    let created = json_stdout(
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
                "2026-07-14T13:45:00-03:00",
            ])
            .assert()
            .success(),
    );
    let old_id = created["id"].as_i64().unwrap();

    let corrected = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "correct",
                &old_id.to_string(),
                "--quantity",
                "90",
                "--reason",
                "weighed again",
            ])
            .assert()
            .success(),
    );
    assert_eq!(corrected["mode"], "supersede");
    assert_eq!(corrected["supersedes_id"], old_id);
    assert_eq!(corrected["quantity"], 90.0);
    let new_id = corrected["id"].as_i64().unwrap();
    assert_ne!(new_id, old_id);

    let list = json_stdout(
        bin()
            .args(["--db", &db, "--json", "nutrition", "consumption", "list"])
            .assert()
            .success(),
    );
    let ids: Vec<i64> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_i64())
        .collect();
    assert!(ids.contains(&new_id));
    assert!(
        !ids.contains(&old_id),
        "old head must be soft-deleted: {list}"
    );

    let audit_old = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "audit",
                &old_id.to_string(),
            ])
            .assert()
            .success(),
    );
    assert!(audit_old["current"]["deleted_at"].as_str().is_some());
    let old_kinds: Vec<_> = audit_old["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["kind"].as_str())
        .collect();
    assert!(old_kinds.contains(&"supersede"), "old audit: {audit_old}");
    assert_eq!(
        audit_old["history"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["kind"] == "supersede")
            .unwrap()["meta"]["superseded_by"],
        new_id
    );

    let audit_new = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "audit",
                &new_id.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_eq!(audit_new["current"]["supersedes_id"], old_id);
    assert!(audit_new["current"]["deleted_at"].is_null());
    let create = audit_new["history"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["kind"] == "create")
        .expect("create on new");
    assert_eq!(create["meta"]["supersedes"], old_id);
    assert_eq!(create["meta"]["reason"], "weighed again");
}

#[test]
fn consumption_correct_requires_reason_and_refuses_deleted() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    seed_product(&db);

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
            "50",
            "--unit",
            "g",
            "--consumed-at",
            "2026-07-14T08:00:00-03:00",
        ])
        .assert()
        .success();

    // clap requires --reason; empty reason fails in handler.
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "consumption",
            "correct",
            "1",
            "--quantity",
            "55",
            "--reason",
            "   ",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("reason"));

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
            "dup",
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
            "correct",
            "1",
            "--quantity",
            "55",
            "--reason",
            "oops",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("soft-deleted"));
}

#[test]
fn consumption_correct_dry_run_no_write() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    seed_product(&db);

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
            "40",
            "--unit",
            "g",
            "--consumed-at",
            "2026-07-14T09:00:00-03:00",
        ])
        .assert()
        .success();

    let dry = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "correct",
                "1",
                "--quantity",
                "45",
                "--reason",
                "preview",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["supersedes_id"], 1);

    let list = json_stdout(
        bin()
            .args(["--db", &db, "--json", "nutrition", "consumption", "list"])
            .assert()
            .success(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], 1);
    assert_eq!(list[0]["quantity"], 40.0);
}

#[test]
fn purchase_correct_supersedes() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    seed_product(&db);

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
        .success();

    let created = json_stdout(
        bin()
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
                "--price",
                "3.50",
                "--store",
                "1",
                "--purchased-at",
                "2026-07-14T18:00:00-03:00",
            ])
            .assert()
            .success(),
    );
    let old_id = created["id"].as_i64().unwrap();

    let corrected = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "purchase",
                "correct",
                &old_id.to_string(),
                "--quantity",
                "3",
                "--reason",
                "bought three packs",
            ])
            .assert()
            .success(),
    );
    assert_eq!(corrected["mode"], "supersede");
    assert_eq!(corrected["supersedes_id"], old_id);
    assert_eq!(corrected["quantity"], 3.0);

    let list = json_stdout(
        bin()
            .args(["--db", &db, "--json", "nutrition", "purchase", "list"])
            .assert()
            .success(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["quantity"], 3.0);
    assert_ne!(list[0]["id"], old_id);
}
