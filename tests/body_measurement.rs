use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn db_path(dir: &TempDir) -> String {
    dir.path().join("t.db").display().to_string()
}

#[test]
fn measurement_create_list_show_json() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "today",
            "--weight-kg",
            "81.2",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "list",
            "--days",
            "7",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("81.2"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "show",
            "--date",
            "today",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("weight_kg"));
}

#[test]
fn measurement_update_delete() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-01-01",
            "--weight-kg",
            "80",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "update",
            "--date",
            "2026-01-01",
            "--weight-kg",
            "79.5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "delete",
            "--date",
            "2026-01-01",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted_id"));
}

#[test]
fn sleep_create_and_list() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "sleep",
            "create",
            "--date",
            "today",
            "--total-sleep",
            "7h 30m",
            "--rem",
            "90",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    bin()
        .args([
            "--db", &db, "--json", "body", "sleep", "list", "--days", "3",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("total_sleep_minutes"));
}
