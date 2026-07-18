use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

/// After schema is current, open/migrate must not bulk-rewrite consumption units.
#[test]
fn open_db_does_not_rerun_consumption_unit_heuristics() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("t.db");
    let db = db_path.display().to_string();

    bin().args(["--db", &db, "init"]).assert().success();

    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO products (name, created_at, updated_at) VALUES ('Seed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    let pid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO product_nutritions
         (product_id, reference_quantity, reference_unit, energy_kcal, protein_g,
          carbohydrates_g, fat_g, fiber_g, sugars_g)
         VALUES (?1, 100.0, 'g', 100.0, 1.0, 1.0, 1.0, 0.0, 0.0)",
        [pid],
    )
    .unwrap();
    // Non-canonical unit that v3 heuristics would convert if re-run.
    conn.execute(
        "INSERT INTO consumptions (product_id, quantity, unit, consumed_at, created_at)
         VALUES (?1, 0.2, 'kg', '2026-01-02T12:00:00Z', '2026-01-02T12:00:00Z')",
        [pid],
    )
    .unwrap();
    let cid = conn.last_insert_rowid();
    let before: (f64, String) = conn
        .query_row(
            "SELECT quantity, unit FROM consumptions WHERE id = ?1",
            [cid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(before.1, "kg");
    drop(conn);

    // Any command that opens the DB runs migrations; version is already current.
    bin()
        .args(["--db", &db, "--json", "nutrition", "product", "list"])
        .assert()
        .success();

    let conn = Connection::open(&db_path).unwrap();
    let after: (f64, String) = conn
        .query_row(
            "SELECT quantity, unit FROM consumptions WHERE id = ?1",
            [cid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        before, after,
        "settled consumption units must not change on open"
    );
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
