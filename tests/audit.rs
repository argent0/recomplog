//! S7a/S7b/S7c: `audit` CLI + event, catalog, merge, and import writers.

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
    assert_real_create(&pa);

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
    assert_real_create(&sa);

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
    assert_real_create(&ma);

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
    assert_real_create(&ea);
}

#[test]
fn product_rename_and_nutrition_write_catalog_audit() {
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
    let pid = product["id"].as_i64().unwrap().to_string();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "product",
            "rename",
            &pid,
            "--name",
            "Morixe Oats",
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
            &pid,
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

    let audit = json_stdout(
        bin()
            .args(["--db", &db, "--json", "nutrition", "product", "audit", &pid])
            .assert()
            .success(),
    );
    let kinds = history_kinds(&audit);
    assert_eq!(kinds, vec!["create", "catalog", "catalog"]);
    assert_eq!(audit["current"]["name"], "Morixe Oats");
    let rename = &audit["history"][1];
    assert!(rename["summary"]
        .as_str()
        .unwrap_or("")
        .contains("Morixe Oats"));
    let fields = rename["fields"].as_array().expect("rename fields");
    let name_f = fields
        .iter()
        .find(|f| f["name"] == "name")
        .expect("name field");
    assert_eq!(name_f["old"], "Oats");
    assert_eq!(name_f["new"], "Morixe Oats");
    assert!(audit["history"][2]["summary"]
        .as_str()
        .unwrap_or("")
        .contains("nutrition"));
}

#[test]
fn product_merge_writes_merge_audit_on_keeper_and_source() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    let keeper = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "product",
                "create",
                "Keeper Oats",
            ])
            .assert()
            .success(),
    );
    let source = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "product",
                "create",
                "Dup Oats",
            ])
            .assert()
            .success(),
    );
    let kid = keeper["id"].as_i64().unwrap();
    let sid = source["id"].as_i64().unwrap();

    // Nutrition required before consumption.
    for id in [kid, sid] {
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "product",
                "nutrition",
                "set",
                &id.to_string(),
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
    }

    // One consumption on source so merge meta can report counts.
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "consumption",
            "create",
            "--product",
            &sid.to_string(),
            "--quantity",
            "50",
            "--unit",
            "g",
            "--consumed-at",
            "2026-07-14T08:30:00-03:00",
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
            "merge",
            "--into",
            &kid.to_string(),
            &sid.to_string(),
        ])
        .assert()
        .success();

    let src_audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "product",
                "audit",
                &sid.to_string(),
            ])
            .assert()
            .success(),
    );
    assert!(history_kinds(&src_audit).contains(&"merge"));
    let merge_src = src_audit["history"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["kind"] == "merge")
        .expect("source merge");
    assert_eq!(merge_src["meta"]["role"], "source");
    assert_eq!(merge_src["meta"]["into_id"], kid);
    assert_eq!(merge_src["meta"]["consumptions"], 1);
    assert!(src_audit["current"]["retired_at"].as_str().is_some());
    assert_eq!(src_audit["current"]["merged_into_id"], kid);

    let k_audit = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "product",
                "audit",
                &kid.to_string(),
            ])
            .assert()
            .success(),
    );
    assert!(history_kinds(&k_audit).contains(&"merge"));
    let merge_k = k_audit["history"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["kind"] == "merge")
        .expect("keeper merge");
    assert_eq!(merge_k["meta"]["role"], "keeper");
    assert_eq!(merge_k["meta"]["from_ids"][0], sid);
    assert_eq!(merge_k["meta"]["consumptions_aliased"], 1);

    // Event FK still points at source (alias model).
    let cons = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "nutrition",
                "consumption",
                "list",
                "--since",
                "2026-07-01",
                "--until",
                "2026-07-31",
            ])
            .assert()
            .success(),
    );
    let rows = cons.as_array().expect("consumption list");
    assert_eq!(rows[0]["product_id"], sid);
}

#[test]
fn fit_import_writes_import_audit_on_workout_and_set() {
    let fit = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/Zepp20260710164935.fit");
    assert!(fit.exists(), "FIT fixture missing: {}", fit.display());

    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    let fit_s = fit.display().to_string();

    bin().args(["--db", &db, "init"]).assert().success();

    let imported = json_stdout(
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "import",
                "fit",
                &fit_s,
                "--exercise",
                "running",
                "--no-profile-hr",
            ])
            .assert()
            .success(),
    );
    let wid = imported["workout_id"].as_i64().unwrap();
    let set_id = imported["set_id"].as_i64().unwrap();
    let sha = imported["sha256"].as_str().expect("sha256");

    let wa = json_stdout(
        bin()
            .args(["--db", &db, "--json", "workout", "audit", &wid.to_string()])
            .assert()
            .success(),
    );
    assert_eq!(history_kinds(&wa), vec!["import"]);
    assert_eq!(wa["history"][0]["actor"], "import");
    assert_eq!(wa["history"][0]["meta"]["source"], "fit");
    assert_eq!(wa["history"][0]["meta"]["sha256"], sha);

    let sa = json_stdout(
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
    assert_eq!(history_kinds(&sa), vec!["import"]);
    assert_eq!(sa["history"][0]["meta"]["sha256"], sha);
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
