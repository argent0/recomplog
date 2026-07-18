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
            "--carbohydrates-g",
            "10",
            "--fat-g",
            "5",
            "--fiber-g",
            "0",
            "--sugars-g",
            "0",
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

/// E1: classic six macros required on set; zeros warn; incomplete blocks consumption;
/// db check flags incomplete; --zero-macros lists rare zeros for inspection.
#[test]
fn classic_macros_gates_and_zero_inspection() {
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
            "Mystery Food",
        ])
        .assert()
        .success();

    // Ref only → fail
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
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("classic macros"));

    // Energy only → fail (missing protein/carbs/fat/fiber/sugars)
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
            "100",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--protein-g"));

    // Full classic six with energy=0 → success + zero_macro warnings
    let out = bin()
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
            "0",
            "--protein-g",
            "0",
            "--carbohydrates-g",
            "0",
            "--fat-g",
            "0",
            "--fiber-g",
            "0",
            "--sugars-g",
            "0",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["success"], true);
    let warnings = v["warnings"].as_array().expect("zero-macro warnings");
    assert!(
        warnings
            .iter()
            .any(|w| w["kind"] == "zero_macro" && w["field"] == "energy_kcal"),
        "expected zero_macro for energy_kcal, got {warnings:?}"
    );

    // Consumption of complete product succeeds
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
            "2026-07-14T12:00:00-03:00",
        ])
        .assert()
        .success();

    // Incomplete product (legacy/import-style row): CLI set cannot create it; raw SQL can.
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "product",
            "create",
            "Legacy Incomplete",
        ])
        .assert()
        .success();
    let pid = {
        use rusqlite::Connection;
        let conn = Connection::open(&db).unwrap();
        let pid: i64 = conn
            .query_row(
                "SELECT id FROM products WHERE name = 'Legacy Incomplete'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO product_nutritions
             (product_id, reference_quantity, reference_unit, energy_kcal, protein_g,
              carbohydrates_g, fat_g, fiber_g, sugars_g)
             VALUES (?1, 100, 'g', NULL, NULL, NULL, NULL, NULL, NULL)",
            [pid],
        )
        .unwrap();
        pid
    };
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
            "50",
            "--unit",
            "g",
            "--consumed-at",
            "2026-07-14T13:00:00-03:00",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("incomplete classic macros"));

    // Insert consumption directly so db check can flag it
    {
        use rusqlite::Connection;
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO consumptions (product_id, quantity, unit, consumed_at, created_at)
             VALUES (?1, 50, 'g', '2026-07-14T16:00:00Z', '2026-07-14T16:00:00Z')",
            [pid],
        )
        .unwrap();
    }

    // db check should fail with incomplete-macro findings
    let check = bin()
        .args(["--db", &db, "--json", "db", "check"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&check).unwrap();
    assert_eq!(report["ok"], false);
    assert!(
        report["products_with_incomplete_macros"]["count"]
            .as_i64()
            .unwrap()
            >= 1
    );
    assert!(
        report["consumptions_with_incomplete_macros"]["count"]
            .as_i64()
            .unwrap()
            >= 1
    );
    // without flag, zero list omitted
    assert!(report["products_with_zero_macros"].is_null());

    // --zero-macros lists product 1 zeros; does not change that we already fail on incomplete
    let check_z = bin()
        .args(["--db", &db, "--json", "db", "check", "--zero-macros"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let rz: serde_json::Value = serde_json::from_slice(&check_z).unwrap();
    assert_eq!(rz["checked_zero_macros"], true);
    assert!(
        rz["products_with_zero_macros"]["count"].as_i64().unwrap() >= 1,
        "expected zero-macro products listed, got {}",
        rz["products_with_zero_macros"]
    );
    let zitems = rz["products_with_zero_macros"]["items"].as_array().unwrap();
    assert!(zitems.iter().any(|i| {
        i["name"] == "Mystery Food"
            && i["zero_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "energy_kcal")
    }));
}
