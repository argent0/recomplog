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

    // Human strength show: exercise table (repslog parity), not reps=… debug lines.
    bin()
        .args(["--db", &db, "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Workout ID: 1"))
        .stdout(predicate::str::contains("EXERCISES"))
        .stdout(predicate::str::contains("bench press"))
        .stdout(predicate::str::contains("5 reps"))
        .stdout(predicate::str::contains("100.00 kg"))
        .stdout(predicate::str::contains("Set #"));
}

#[test]
fn strength_unilateral_and_list_summary() {
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
            "Legs",
            "--notes",
            "unilateral lower body",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "workout",
            "exercise",
            "create",
            "pistol squat",
            "--category",
            "legs",
            "--load-type",
            "body_mass",
        ])
        .assert()
        .success();

    // Left / right with phases and bodyweight
    for (side, phase, reps) in [
        ("left", "concentric", "2"),
        ("left", "eccentric", "5"),
        ("right", "concentric", "2"),
        ("right", "eccentric", "5"),
    ] {
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
                "pistol squat",
                "--reps",
                reps,
                "--weight",
                "80",
                "--side",
                side,
                "--phase",
                phase,
            ])
            .assert()
            .success();
    }

    bin()
        .args(["--db", &db, "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("EXERCISES"))
        .stdout(predicate::str::contains("pistol squat"))
        .stdout(predicate::str::contains("Left: 7 reps | Right: 7 reps"))
        .stdout(predicate::str::contains("LEFT"))
        .stdout(predicate::str::contains("RIGHT"))
        .stdout(predicate::str::contains("eccentric"))
        .stdout(predicate::str::contains("80.0 kg BW"));

    // List: strength uses notes as summary (no cardio sets)
    bin()
        .args(["--db", &db, "workout", "list", "--limit", "5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Summary"))
        .stdout(predicate::str::contains("unilateral lower body"))
        .stdout(predicate::str::contains("Legs"));
}

#[test]
fn timed_holds_show_duration_and_cardio_summary() {
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
            "Static Holds",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "workout",
            "exercise",
            "create",
            "wall sit",
            "--category",
            "legs",
            "--load-type",
            "body_mass",
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
            "wall sit",
            "--duration",
            "60",
            "--no-weight-recorded",
            "--rir",
            "0",
        ])
        .assert()
        .success();

    bin()
        .args(["--db", &db, "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CARDIO SUMMARY"))
        .stdout(predicate::str::contains("1:00"))
        .stdout(predicate::str::contains("wall sit"))
        .stdout(predicate::str::contains("RIR 0"));
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
