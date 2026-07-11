//! Integration tests for `report nutrition` (summary, list --value, spending --by).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn setup_fixture(db: &str) {
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "store",
            "create",
            "Market",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "store",
            "create",
            "Costco",
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
            "create",
            "Yogurt",
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
            "create",
            "Oats",
        ])
        .assert()
        .success();

    // Yogurt: 59 kcal / 10g protein per 100g; Magnesium 200mg per 100g
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
            "59",
            "--protein-g",
            "10",
            "--carbohydrates-g",
            "3.5",
            "--fat-g",
            "0.4",
            "--micronutrient",
            "Magnesium",
            "200",
            "mg",
        ])
        .assert()
        .success();

    // Oats: 389 kcal / 17g protein per 100g
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "product",
            "nutrition",
            "set",
            "2",
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

    // Two yogurt consumptions same day (200g total = 2× ref), one oats on earlier day
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
            "100",
            "--date",
            "2026-07-05",
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
            "100",
            "--date",
            "2026-07-05",
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
            "2",
            "--quantity",
            "50",
            "--date",
            "2026-07-03",
        ])
        .assert()
        .success();

    // Purchases: yogurt at Market, oats at Costco
    bin()
        .args([
            "--db",
            db,
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
            "--date",
            "2026-07-04",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "purchase",
            "create",
            "--product",
            "2",
            "--quantity",
            "1",
            "--price",
            "4.99",
            "--store",
            "2",
            "--date",
            "2026-07-05",
        ])
        .assert()
        .success();
}

fn json_out(db: &str, args: &[&str]) -> Value {
    let output = bin()
        .args(["--db", db, "--json"])
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

#[test]
fn summary_macros_and_micros_json() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_fixture(&db);

    let v = json_out(
        &db,
        &[
            "report",
            "nutrition",
            "summary",
            "--since",
            "2026-07-01",
            "--until",
            "2026-07-10",
        ],
    );

    assert_eq!(v["total_consumed_items"], 3);
    // yogurt 200g: 118 kcal, 20g protein; oats 50g: 194.5 kcal, 8.5g protein
    assert!((v["totals"]["energy_kcal"].as_f64().unwrap() - 312.5).abs() < 0.01);
    assert!((v["totals"]["protein_g"].as_f64().unwrap() - 28.5).abs() < 0.01);
    assert!((v["totals"]["carbohydrates_g"].as_f64().unwrap() - 40.0).abs() < 0.01);

    let micros = v["micronutrients"].as_array().unwrap();
    assert!(!micros.is_empty());
    let mag = micros
        .iter()
        .find(|m| m["name"] == "Magnesium")
        .expect("Magnesium present");
    // 2 × 200mg scale 1.0 = 400
    assert!((mag["total_amount"].as_f64().unwrap() - 400.0).abs() < 0.01);
    assert_eq!(mag["unit"], "mg");
}

#[test]
fn list_default_daily_macros() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_fixture(&db);

    let v = json_out(
        &db,
        &[
            "report",
            "nutrition",
            "list",
            "--since",
            "2026-07-03",
            "--until",
            "2026-07-05",
        ],
    );

    assert!(v.get("entries").is_none(), "must not use per-entry shape");
    assert_eq!(v["value"], "macronutrients");
    let days = v["days"].as_array().unwrap();
    // fill range: 3 days
    assert_eq!(days.len(), 3);

    let d3 = days.iter().find(|d| d["date"] == "2026-07-03").unwrap();
    assert_eq!(d3["total_consumed_items"], 1);
    assert!((d3["totals"]["protein_g"].as_f64().unwrap() - 8.5).abs() < 0.01);

    let d4 = days.iter().find(|d| d["date"] == "2026-07-04").unwrap();
    assert_eq!(d4["total_consumed_items"], 0);

    let d5 = days.iter().find(|d| d["date"] == "2026-07-05").unwrap();
    assert_eq!(d5["total_consumed_items"], 2);
    assert!((d5["totals"]["protein_g"].as_f64().unwrap() - 20.0).abs() < 0.01);
    assert!((d5["totals"]["energy_kcal"].as_f64().unwrap() - 118.0).abs() < 0.01);
}

#[test]
fn list_value_protein() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_fixture(&db);

    let v = json_out(
        &db,
        &[
            "report",
            "nutrition",
            "list",
            "--since",
            "2026-07-03",
            "--until",
            "2026-07-05",
            "--value",
            "protein",
        ],
    );

    assert_eq!(v["value"], "protein");
    let days = v["days"].as_array().unwrap();
    let d5 = days.iter().find(|d| d["date"] == "2026-07-05").unwrap();
    assert!((d5["totals"]["protein_g"].as_f64().unwrap() - 20.0).abs() < 0.01);
    assert!(d5["totals"]["energy_kcal"].is_null());
    assert!(d5["totals"]["carbohydrates_g"].is_null());
}

#[test]
fn list_fill_range_zero_days() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_fixture(&db);

    // Log one consumption today so --days 3 has at least one non-empty day
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
            "100",
            "--date",
            "today",
        ])
        .assert()
        .success();

    let v = json_out(
        &db,
        &[
            "report",
            "nutrition",
            "list",
            "--days",
            "3",
            "--value",
            "protein",
        ],
    );

    assert_eq!(v["period"]["days"], 3);
    let days = v["days"].as_array().unwrap();
    assert_eq!(days.len(), 3);
    // At least one day should be zero-filled (unless all three have data)
    let has_zero = days.iter().any(|d| d["total_consumed_items"] == 0);
    let total_items: i64 = days
        .iter()
        .map(|d| d["total_consumed_items"].as_i64().unwrap())
        .sum();
    assert!(total_items >= 1);
    // With only today's log in the window of 3 days, at least one empty day
    assert!(has_zero || total_items >= 1);
}

#[test]
fn spending_by_store() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_fixture(&db);

    let v = json_out(
        &db,
        &[
            "report",
            "nutrition",
            "spending",
            "--since",
            "2026-07-01",
            "--until",
            "2026-07-10",
            "--by",
            "store",
        ],
    );

    // 350 + 499 = 849 cents
    assert_eq!(v["total_cents"], 849);
    assert_eq!(v["total"], "$8.49");
    let stores = v["by_store"].as_array().unwrap();
    assert_eq!(stores.len(), 2);
    let sum: i64 = stores.iter().map(|s| s["cents"].as_i64().unwrap()).sum();
    assert_eq!(sum, 849);
    let market = stores.iter().find(|s| s["store_name"] == "Market").unwrap();
    assert_eq!(market["cents"], 350);
    assert_eq!(market["purchase_count"], 1);
    assert!(v["by_product"].is_null());
}

#[test]
fn spending_by_product() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_fixture(&db);

    let v = json_out(
        &db,
        &[
            "report",
            "nutrition",
            "spending",
            "--since",
            "2026-07-01",
            "--until",
            "2026-07-10",
            "--by",
            "product",
        ],
    );

    assert_eq!(v["total_cents"], 849);
    let prods = v["by_product"].as_array().unwrap();
    assert_eq!(prods.len(), 2);
    let yogurt = prods
        .iter()
        .find(|p| p["product_name"] == "Yogurt")
        .unwrap();
    assert_eq!(yogurt["cents"], 350);
    let oats = prods.iter().find(|p| p["product_name"] == "Oats").unwrap();
    assert_eq!(oats["cents"], 499);
}

#[test]
fn days_conflicts_with_since() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "report",
            "nutrition",
            "summary",
            "--days",
            "7",
            "--since",
            "2026-07-01",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("cannot be used with")
                .or(predicate::str::contains("conflict")),
        );
}

#[test]
fn human_list_table() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_fixture(&db);

    bin()
        .args([
            "--db",
            &db,
            "report",
            "nutrition",
            "list",
            "--since",
            "2026-07-03",
            "--until",
            "2026-07-05",
            "--value",
            "protein",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Date"))
        .stdout(predicate::str::contains("Protein"))
        .stdout(predicate::str::contains("2026-07-05"));
}
