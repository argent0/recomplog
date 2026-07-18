use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

#[test]
fn store_product_micro_purchase() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

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
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    bin()
        .args([
            "--db",
            &db,
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
            "59",
            "--protein-g",
            "10",
            "--carbohydrates-g",
            "10",
            "--fat-g",
            "5",
            "--fiber-g",
            "0",
            "--sugars-g",
            "0",
            "--micronutrient",
            "Magnesium",
            "200",
            "mg",
        ])
        .assert()
        .success();

    bin()
        .args(["--db", &db, "--json", "nutrition", "product", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Magnesium"));

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
            "--date",
            "2026-07-14T15:30:00-03:00",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "product-tag",
            "create",
            "dairy",
        ])
        .assert()
        .success();
}
