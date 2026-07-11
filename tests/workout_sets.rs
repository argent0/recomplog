use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

#[test]
fn cluster_and_unilateral_and_cardio() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin().args(["--db", &db, "init"]).assert().success();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();

    // cluster on bench press (seeded)
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "add-cluster",
            "--workout",
            "1",
            "--exercise",
            "bench press",
            "--reps",
            "10,5,5",
            "--weight",
            "100",
            "--phase",
            "full",
            "--rir",
            "0,0,1",
            "--effective-reps",
            "6,4,3",
            "--rest",
            "15",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("cluster_id"));

    // unilateral
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "add-unilateral",
            "--workout",
            "1",
            "--exercise",
            "lunges",
            "--reps",
            "8,8",
            "--weight",
            "80",
            "--phase",
            "full",
            "--side",
            "both",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("set_ids"));

    // cardio
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "add-cardio",
            "--workout",
            "1",
            "--exercise",
            "running",
            "--distance",
            "5",
            "--duration",
            "1500",
            "--avg-heart-rate",
            "150",
            "--max-heart-rate",
            "175",
            "--pace",
            "5.0",
            "--calories",
            "400",
            "--hr-zones",
            r#"{"z1_seconds":60,"z2_seconds":1200,"z3_seconds":240,"z4_seconds":0,"z5_seconds":0}"#,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("cardio set added"));

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cluster_id"))
        .stdout(predicate::str::contains("heart_rate_zones"));
}

#[test]
fn set_update_and_move() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
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
        .success();
    bin()
        .args([
            "--db",
            &db,
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
            "105",
            "--phase",
            "full",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "update", "1", "--reps", "6",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("set updated"));
    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "move", "2", "--to", "1",
        ])
        .assert()
        .success();
}
