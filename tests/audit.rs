//! S7a/S7b: `audit` CLI + real create/update writers on event entities.

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

/// Real (non-synthetic) create: stored audit row id is an integer.
fn assert_real_create(audit: &Value) {
    let hist = audit["history"].as_array().expect("history array");
    let create = hist
        .iter()
        .find(|h| h["kind"] == "create")
        .expect("create entry");
    assert!(
        create["id"].as_i64().is_some(),
        "expected real audit id, got synthetic?: {create}"
    );
    assert_ne!(
        create["meta"]["synthetic"], true,
        "expected non-synthetic create: {create}"
    );
}

#[test]
fn measurement_audit_real_create_and_soft_delete() {
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
    assert_real_create(&audit);

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
    assert_real_create(&audit);
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
fn workout_audit_has_real_create() {
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
    assert_real_create(&audit);
}

#[test]
fn measurement_update_writes_audit_fields() {
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
                "2026-07-12",
                "--weight-kg",
                "81.0",
            ])
            .assert()
            .success(),
    );
    let id = created["id"].as_i64().unwrap();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "update",
            "--id",
            &id.to_string(),
            "--weight-kg",
            "80.5",
            "--no-sanity-check",
        ])
        .assert()
        .success();

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
    let kinds = history_kinds(&audit);
    assert_eq!(kinds, vec!["create", "update"]);
    let update = &audit["history"][1];
    assert_eq!(update["kind"], "update");
    let fields = update["fields"].as_array().expect("fields array");
    let weight = fields
        .iter()
        .find(|f| f["name"] == "weight_kg")
        .expect("weight_kg field");
    assert_eq!(weight["old"], 81.0);
    assert_eq!(weight["new"], 80.5);
    assert_eq!(audit["current"]["weight_kg"], 80.5);
}

#[test]
fn set_create_and_update_audit() {
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

    let set = json_stdout(
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
                "--phase",
                "full",
            ])
            .assert()
            .success(),
    );
    let set_id = set["id"]
        .as_i64()
        .or_else(|| set["set_id"].as_i64())
        .expect("set id");

    let audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "workout",
                "set",
                "audit",
                &set_id.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_real_create(&audit);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "update",
            &set_id.to_string(),
            "--reps",
            "6",
        ])
        .assert()
        .success();

    let audit2 = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "workout",
                "set",
                "audit",
                &set_id.to_string(),
            ])
            .assert()
            .success(),
    );
    let kinds = history_kinds(&audit2);
    assert_eq!(kinds, vec!["create", "update"]);
    let fields = audit2["history"][1]["fields"].as_array().expect("fields");
    let reps = fields
        .iter()
        .find(|f| f["name"] == "reps")
        .expect("reps field");
    assert_eq!(reps["old"], 5);
    assert_eq!(reps["new"], 6);
}

#[test]
fn consumption_create_writes_real_audit() {
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
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "product",
            "nutrition",
            "set",
            &pid.to_string(),
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

    let cons = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "create",
                "--product",
                &pid.to_string(),
                "--quantity",
                "80",
                "--unit",
                "g",
                "--consumed-at",
                "2026-07-14T08:30:00-03:00",
            ])
            .assert()
            .success(),
    );
    let cid = cons["id"].as_i64().unwrap();

    let audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "audit",
                &cid.to_string(),
            ])
            .assert()
            .success(),
    );
    assert_real_create(&audit);
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
