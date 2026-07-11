use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

#[test]
fn product_create_search_set_nutrition() {
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
            "Greek Yogurt",
            "--tags",
            "dairy,protein",
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
            "search",
            "--name",
            "yogurt",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Greek Yogurt"));

    // Short fuzzy queries must not match unrelated tokens (e.g. "iron" ≠ "virgin").
    bin()
        .args([
            "--db",
            &db,
            "nutrition",
            "product",
            "create",
            "Gentech Iron Bar",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "nutrition",
            "product",
            "create",
            "Virgin Olive Oil",
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
            "search",
            "--name",
            "iron",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Gentech Iron Bar"))
        .stdout(predicate::str::contains("Virgin Olive Oil").not());

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "product",
            "set",
            "1",
            "--energy-kcal",
            "59",
            "--protein-g",
            "10",
            "--reference-quantity",
            "100",
        ])
        .assert()
        .success();

    bin()
        .args(["--db", &db, "--json", "nutrition", "product", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("protein_g"));
}
