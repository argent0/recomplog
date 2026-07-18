//! Micronutrient catalog split: macros are columns; micros are the catalog.

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

#[test]
fn micronutrient_crud_and_nutrient_alias() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    // Magnesium matches INFOODS MG — must link explicitly.
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "micronutrient",
            "create",
            "Magnesium",
            "--unit",
            "mg",
            "--recommended-intake",
            "420",
            "--infoods",
            "MG",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    // Hidden/visible alias still works.
    bin()
        .args(["--db", &db, "--json", "nutrition", "nutrient", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Magnesium"))
        .stdout(predicate::str::contains("MG"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "micronutrient",
            "search",
            "mag",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Magnesium"));
}

#[test]
fn reject_macro_as_micronutrient_create() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "nutrition",
            "micronutrient",
            "create",
            "Protein",
            "--unit",
            "g",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("macronutrient"));
}

#[test]
fn reject_macro_as_product_micronutrient_flag() {
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
            "Oil",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "nutrition",
            "product",
            "nutrition",
            "set",
            "1",
            "--reference-quantity",
            "15",
            "--reference-unit",
            "ml",
            "--energy-kcal",
            "120",
            "--fat-g",
            "14",
            "--protein-g",
            "10",
            "--carbohydrates-g",
            "10",
            "--fiber-g",
            "0",
            "--sugars-g",
            "0",
            "--micronutrient",
            "Saturated Fat",
            "2.5",
            "g",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--saturated-fat-g"));
}

#[test]
fn extended_macros_via_columns_not_in_micro_list() {
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
            "Olive oil",
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
            "15",
            "--reference-unit",
            "ml",
            "--energy-kcal",
            "120",
            "--fat-g",
            "14",
            "--protein-g",
            "10",
            "--carbohydrates-g",
            "10",
            "--fiber-g",
            "0",
            "--sugars-g",
            "0",
            "--saturated-fat-g",
            "2.5",
            "--cholesterol-mg",
            "0",
            "--micronutrient",
            "Vitamin E",
            "2",
            "mg",
        ])
        .assert()
        .success();

    let out = bin()
        .args(["--db", &db, "--json", "nutrition", "product", "show", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["nutrition"]["saturated_fat_g"], 2.5);
    assert_eq!(v["nutrition"]["cholesterol_mg"], 0.0);
    let micros = v["micronutrients"].as_array().unwrap();
    assert_eq!(micros.len(), 1);
    assert_eq!(micros[0]["name"], "Vitamin E");

    // Catalog should only have Vitamin E, not Saturated Fat.
    let list = bin()
        .args(["--db", &db, "--json", "nutrition", "micronutrient", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let catalog: serde_json::Value = serde_json::from_slice(&list).unwrap();
    let names: Vec<&str> = catalog
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Vitamin E"));
    assert!(!names
        .iter()
        .any(|n| n.eq_ignore_ascii_case("Saturated Fat")));
    assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("Protein")));
}

#[test]
fn migration_promotes_extended_macros_from_legacy_shape() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("legacy.db");
    let path_s = path.display().to_string();

    // Build a v1-shaped DB with conflated nutrients table, then open via CLI
    // so migrations run through v6.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            -- Start at v5 so only the micro/macro split (v6) runs.
            PRAGMA user_version = 5;
            CREATE TABLE products (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE nutrients (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                unit TEXT NOT NULL,
                recommended_intake REAL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE product_nutritions (
                product_id INTEGER PRIMARY KEY REFERENCES products(id) ON DELETE CASCADE,
                reference_quantity REAL NOT NULL,
                reference_unit TEXT NOT NULL,
                energy_kcal REAL,
                protein_g REAL,
                carbohydrates_g REAL,
                fat_g REAL,
                fiber_g REAL,
                sugars_g REAL
            );
            CREATE TABLE product_micronutrients (
                product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
                nutrient_id INTEGER NOT NULL REFERENCES nutrients(id),
                amount REAL NOT NULL,
                unit TEXT NOT NULL,
                PRIMARY KEY (product_id, nutrient_id)
            );
            CREATE TABLE product_tags (id INTEGER PRIMARY KEY, name TEXT UNIQUE, created_at TEXT);
            CREATE TABLE store_tags (id INTEGER PRIMARY KEY, name TEXT UNIQUE, created_at TEXT);
            CREATE TABLE stores (id INTEGER PRIMARY KEY, name TEXT, created_at TEXT);
            CREATE TABLE product_tag_associations (
                product_id INTEGER, tag_id INTEGER, PRIMARY KEY (product_id, tag_id)
            );
            CREATE TABLE store_tag_associations (
                store_id INTEGER, tag_id INTEGER, PRIMARY KEY (store_id, tag_id)
            );
            CREATE TABLE purchases (
                id INTEGER PRIMARY KEY, product_id INTEGER, quantity REAL,
                price_cents INTEGER, store_id INTEGER, purchased_at TEXT, created_at TEXT
            );
            CREATE TABLE consumptions (
                id INTEGER PRIMARY KEY, product_id INTEGER, quantity REAL,
                unit TEXT, consumed_at TEXT, created_at TEXT
            );
            INSERT INTO products (id, name, created_at, updated_at)
            VALUES (1, 'Steak', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
            INSERT INTO nutrients (id, name, unit, recommended_intake, created_at) VALUES
                (1, 'Protein', 'g', 50, '2026-01-01T00:00:00Z'),
                (2, 'Saturated Fat', 'g', NULL, '2026-01-01T00:00:00Z'),
                (3, 'Cholesterol', 'mg', NULL, '2026-01-01T00:00:00Z'),
                (4, 'Iron', 'mg', 18, '2026-01-01T00:00:00Z');
            INSERT INTO product_nutritions
                (product_id, reference_quantity, reference_unit, energy_kcal, protein_g, fat_g)
            VALUES (1, 100, 'g', 250, 26, 15);
            INSERT INTO product_micronutrients (product_id, nutrient_id, amount, unit) VALUES
                (1, 2, 6.0, 'g'),
                (1, 3, 80.0, 'mg'),
                (1, 4, 2.5, 'mg');
            "#,
        )
        .unwrap();
    }

    // Trigger migrations.
    bin()
        .args([
            "--db",
            &path_s,
            "--json",
            "nutrition",
            "product",
            "show",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"saturated_fat_g\": 6"))
        .stdout(predicate::str::contains("\"cholesterol_mg\": 80"))
        .stdout(predicate::str::contains("Iron"));

    let conn = Connection::open(&path).unwrap();
    let ver: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ver, 7);

    let has_nutrients: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='nutrients'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_nutrients, 0);

    let has_micros: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='micronutrients'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_micros, 1);

    let macro_stubs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM micronutrients WHERE name IN ('Protein','Saturated Fat','Cholesterol') COLLATE NOCASE",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(macro_stubs, 0);

    let iron: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM micronutrients WHERE name = 'Iron'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(iron, 1);

    let micro_links: i64 = conn
        .query_row("SELECT COUNT(*) FROM product_micronutrients", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(micro_links, 1); // Iron only

    let infoods: i64 = conn
        .query_row("SELECT COUNT(*) FROM infoods_components", [], |r| r.get(0))
        .unwrap();
    assert!(infoods > 500, "expected full INFOODS seed, got {infoods}");
}

#[test]
fn infoods_seeded_and_searchable() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "infoods",
            "show",
            "VITC",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("vitamin C"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "infoods",
            "search",
            "ascorbic",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("VITC"));
}

#[test]
fn create_refuses_infoods_name_without_link_or_force() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "nutrition",
            "micronutrient",
            "create",
            "Vitamin C",
            "--unit",
            "mg",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("INFOODS"))
        .stderr(predicate::str::contains("VITC"));
}

#[test]
fn create_with_infoods_and_case_insensitive_unique() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "micronutrient",
            "create",
            "Iron",
            "--unit",
            "mg",
            "--infoods",
            "FE",
        ])
        .assert()
        .success();

    // Case-only duplicate refused (no --force bypass for true name dups).
    bin()
        .args([
            "--db",
            &db,
            "nutrition",
            "micronutrient",
            "create",
            "iron",
            "--unit",
            "mg",
            "--force",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn product_set_reuses_case_and_auto_links_infoods() {
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
            "Spinach",
        ])
        .assert()
        .success();

    // Exact INFOODS name "magnesium" → auto-create with MG tag.
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
            "23",
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
            "magnesium",
            "79",
            "mg",
        ])
        .assert()
        .success();

    let list = bin()
        .args(["--db", &db, "--json", "nutrition", "micronutrient", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let catalog: serde_json::Value = serde_json::from_slice(&list).unwrap();
    let arr = catalog.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["infoods_tag"], "MG");

    // Second product set with different casing reuses the row.
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "product",
            "create",
            "Almonds",
        ])
        .assert()
        .success();
    // Classic six required (explicit zeros OK for micro-focused products).
    bin()
        .args([
            "--db",
            &db,
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
            "--micronutrient",
            "Magnesium",
            "270",
            "mg",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("zero_macro"));

    let list2 = bin()
        .args(["--db", &db, "--json", "nutrition", "micronutrient", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let catalog2: serde_json::Value = serde_json::from_slice(&list2).unwrap();
    assert_eq!(catalog2.as_array().unwrap().len(), 1);
}

#[test]
fn create_force_custom_near_infoods_warns() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    // "Vitamin C complex" fuzzy-matches vitamin C / VITC → needs --force
    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "micronutrient",
            "create",
            "Vitamin C complex",
            "--unit",
            "mg",
            "--force",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("FORCE") || stderr.contains("force"),
        "expected stern force warning, got: {stderr}"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["success"], true);
    // warnings present
    assert!(v["warnings"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false));
}

#[test]
fn custom_bioactive_create_without_infoods_ok() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "micronutrient",
            "create",
            "Caffeic acid customxyz",
            "--unit",
            "mg",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));
}

#[test]
fn db_check_flags_untagged_micronutrients() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "nutrition",
            "micronutrient",
            "create",
            "MyPolyphenolXYZ",
            "--unit",
            "mg",
        ])
        .assert()
        .success();

    let out = bin()
        .args(["--db", &db, "--json", "db", "check"])
        .assert()
        .failure() // ok=false → exit 1
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["ok"], false);
    assert!(
        v["micronutrients_without_infoods"]["count"]
            .as_i64()
            .unwrap()
            >= 1
    );
    let names: Vec<&str> = v["micronutrients_without_infoods"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(names.iter().any(|n| n.contains("MyPolyphenolXYZ")));
}
