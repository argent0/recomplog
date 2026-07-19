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
            "--db", &db, "--json", "workout", "set", "update", "1", "--reps", "6", "--reason",
            "miscount",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("set updated"))
        .stdout(predicate::str::contains("\"kind\": \"correction\""));
    // F4: move appends revision; response includes revision_id
    bin()
        .args([
            "--db", &db, "--json", "workout", "set", "move", "2", "--to", "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"revision_id\""))
        .stdout(predicate::str::contains("set moved"));
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
fn body_mass_set_uses_measurement_weight_when_weight_omitted() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-10",
            "--weight-kg",
            "81.2",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "create",
            "--type",
            "Pull",
            "--started-at",
            "2026-07-14T17:00:00Z",
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
            "pull up",
            "--category",
            "pull",
            "--load-type",
            "body_mass",
        ])
        .assert()
        .success();

    // No --weight: should take 81.2 from body measurement
    let out = bin()
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
            "pull up",
            "--reps",
            "8",
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(
        (v["would"]["weight_kg"].as_f64().unwrap() - 81.2).abs() < 1e-9,
        "expected measured body weight 81.2, got {}",
        v
    );

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
            "pull up",
            "--reps",
            "8",
            "--quiet",
        ])
        .assert()
        .success();

    bin()
        .args(["--db", &db, "workout", "show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("81.2 kg BW"));
}

#[test]
fn body_mass_set_errors_without_weight_or_measurement() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();

    bin()
        .args(["--db", &db, "workout", "create", "--type", "Pull"])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "workout",
            "exercise",
            "create",
            "pull up",
            "--category",
            "pull",
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
            "pull up",
            "--reps",
            "8",
            "--quiet",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("body measurement")
                .or(predicate::str::contains("requires --weight")),
        );
}

#[test]
fn body_mass_set_prefers_measurement_on_or_before_workout_day() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();

    // Older measurement (should be used for a workout on 2026-07-10)
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-05",
            "--weight-kg",
            "80.0",
        ])
        .assert()
        .success();
    // Newer measurement after the workout day
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-15",
            "--weight-kg",
            "82.0",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "create",
            "--type",
            "Pull",
            "--started-at",
            "2026-07-10T17:00:00Z",
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
            "chin up",
            "--category",
            "pull",
            "--load-type",
            "body_mass",
        ])
        .assert()
        .success();

    let out = bin()
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
            "chin up",
            "--reps",
            "5",
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(
        (v["would"]["weight_kg"].as_f64().unwrap() - 80.0).abs() < 1e-9,
        "expected on-or-before measurement 80.0, got {}",
        v
    );
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
            "--reason",
            "device reprocess",
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
