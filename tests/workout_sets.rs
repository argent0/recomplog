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

#[test]
fn dry_run_set_add_no_write() {
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
            "full",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\": true"))
        .stdout(predicate::str::contains("\"would\""))
        .stdout(predicate::str::contains("would_create_workout_exercise"));

    // No sets / no workout_exercise created
    let show = bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&show);
    assert!(
        !s.contains("bench press") || s.contains("\"exercises\": []") || s.contains("\"sets\": []"),
        "dry-run must not create sets; got: {s}"
    );
    // Prefer explicit empty exercises check
    assert!(
        s.contains("\"exercises\": []") || !s.contains("\"reps\""),
        "expected no sets after dry-run: {s}"
    );
}

#[test]
fn dry_run_validation_failure() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Test"])
        .assert()
        .success();

    // Missing workout
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "add",
            "--workout",
            "999",
            "--exercise",
            "bench press",
            "--reps",
            "5",
            "--weight",
            "100",
            "--dry-run",
        ])
        .assert()
        .failure();

    // Absurd weight (sanity reject)
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
            "99999",
            "--dry-run",
        ])
        .assert()
        .failure();
}

#[test]
fn cardio_cadence_ascent_and_update_zones() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Run"])
        .assert()
        .success();

    let zones =
        r#"{"z1_seconds":60,"z2_seconds":1200,"z3_seconds":240,"z4_seconds":0,"z5_seconds":0}"#;
    let laps = r#"[{"lap_number":1,"distance_km":1.0,"duration_seconds":300,"pace_min_per_km":5.0,"avg_heart_rate_bpm":140.0}]"#;

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
            "--cadence",
            "170",
            "--ascent",
            "120",
            "--descent",
            "115",
            "--hr-zones",
            zones,
            "--laps",
            laps,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("cardio set added"));

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("avg_cadence_spm"))
        .stdout(predicate::str::contains("170"))
        .stdout(predicate::str::contains("total_ascent_m"))
        .stdout(predicate::str::contains("120"))
        .stdout(predicate::str::contains("total_descent_m"));

    let zones2 =
        r#"{"z1_seconds":100,"z2_seconds":1100,"z3_seconds":300,"z4_seconds":0,"z5_seconds":0}"#;
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "set",
            "update",
            "1",
            "--hr-zones",
            zones2,
            "--cadence",
            "172",
            "--ascent",
            "125",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("set updated"));

    bin()
        .args(["--db", &db, "--json", "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("172"))
        .stdout(predicate::str::contains("125"))
        .stdout(
            predicate::str::contains("z1_seconds\": 100")
                .or(predicate::str::contains("\"z1_seconds\":100")),
        );
}

#[test]
fn require_zones_laps_flag() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    bin()
        .args(["--db", &db, "workout", "create", "--type", "Run"])
        .assert()
        .success();

    // Without zones/laps + require flag → fail
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
            "--require-zones-laps",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("require-zones-laps").or(predicate::str::contains("hr-zones")),
        );

    // Without flag → still succeeds
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
        ])
        .assert()
        .success();
}
