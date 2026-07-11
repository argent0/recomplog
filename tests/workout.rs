use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

#[test]
fn workout_flow_create_set_show() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

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
            "working",
            "--rir",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("set added"));

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bench press"))
        .stdout(predicate::str::contains("100"));
}

#[test]
fn finished_at_create_update_list_show() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "create",
            "--type",
            "Push",
            "--started-at",
            "2026-07-10 17:00:00",
            "--finished-at",
            "2026-07-10 18:30:00",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("finished_at"))
        .stdout(predicate::str::contains("18:30:00"));

    bin()
        .args(["--db", &db, "--json", "workout", "list", "--limit", "5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("finished_at"))
        .stdout(predicate::str::contains("18:30:00"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "update",
            "1",
            "--finished-at",
            "2026-07-10 19:00:00",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("workout updated"));

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("19:00:00"));

    // dry-run update does not change
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "update",
            "1",
            "--finished-at",
            "2026-07-10 20:00:00",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\": true"));

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("19:00:00"));
}
