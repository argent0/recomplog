use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn json_out(assert: assert_cmd::assert::Assert) -> Value {
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid json: {e}\n{stdout}"))
}

fn setup_bench(db: &str) {
    bin()
        .args(["--db", db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            db,
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
            db,
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
            "--effective-reps",
            "4",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            db,
            "--json",
            "workout",
            "set",
            "add",
            "--workout",
            "1",
            "--exercise",
            "bench press",
            "--reps",
            "15",
            "--weight",
            "80",
            "--phase",
            "working",
        ])
        .assert()
        .success();
}

#[test]
fn stats_volume_alias_and_subcommand() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_bench(&db);

    let bare = bin()
        .args(["--db", &db, "--json", "workout", "stats", "--days", "30"])
        .assert()
        .success();
    let bare_v = json_out(bare);
    assert_eq!(bare_v["days"], 30);
    assert!(bare_v["by_exercise"].as_array().unwrap().len() >= 1);
    let vol = &bare_v["by_exercise"][0];
    assert_eq!(vol["exercise"], "bench press");
    // 100*5 + 80*15 = 500 + 1200 = 1700
    assert!((vol["total_volume"].as_f64().unwrap() - 1700.0).abs() < 0.01);
    assert_eq!(vol["sets"], 2);
    assert_eq!(vol["total_reps"], 20);
    assert_eq!(vol["total_eff_reps"], 4);

    let sub = bin()
        .args([
            "--db", &db, "--json", "workout", "stats", "volume", "--days", "30",
        ])
        .assert()
        .success();
    let sub_v = json_out(sub);
    assert_eq!(sub_v["by_exercise"][0]["total_volume"], 1700.0);
}

#[test]
fn stats_volume_period() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_bench(&db);

    let out = bin()
        .args([
            "--db", &db, "--json", "workout", "stats", "volume", "--period", "90d",
        ])
        .assert()
        .success();
    let v = json_out(out);
    assert_eq!(v["days"], 90);
    assert_eq!(v["period"], "90d");

    bin()
        .args([
            "--db", &db, "--json", "workout", "stats", "volume", "--period", "bogus",
        ])
        .assert()
        .failure();

    bin()
        .args([
            "--db", &db, "--json", "workout", "stats", "volume", "--period", "30d", "--days", "14",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("either --period or --days"));
}

#[test]
fn stats_prs() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_bench(&db);

    let out = bin()
        .args(["--db", &db, "--json", "workout", "stats", "prs"])
        .assert()
        .success();
    let v = json_out(out);
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["exercise"], "bench press");
    assert!((arr[0]["max_weight"].as_f64().unwrap() - 100.0).abs() < 0.01);
    assert_eq!(arr[0]["max_reps"], 15);

    let filtered = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "prs",
            "--exercise",
            "bench press",
        ])
        .assert()
        .success();
    assert_eq!(json_out(filtered).as_array().unwrap().len(), 1);
}

#[test]
fn stats_prs_body_mass() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Pull"])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "exercise",
            "create",
            "pull up",
            "--category",
            "calisthenics",
            "--load-type",
            "body_mass",
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
            "pull up",
            "--reps",
            "8",
            "--weight",
            "80",
            "--external-load",
            "5",
            "--phase",
            "working",
        ])
        .assert()
        .success();

    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "prs",
            "--exercise",
            "pull up",
        ])
        .assert()
        .success();
    let v = json_out(out);
    assert!((v[0]["max_weight"].as_f64().unwrap() - 85.0).abs() < 0.01);

    let vol = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "volume",
            "--days",
            "30",
            "--exercise",
            "pull up",
        ])
        .assert()
        .success();
    let vv = json_out(vol);
    // (80+5)*8 = 680
    assert!((vv["by_exercise"][0]["total_volume"].as_f64().unwrap() - 680.0).abs() < 0.01);
}

#[test]
fn stats_summary() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_bench(&db);

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "update",
            "1",
            "--duration",
            "60",
        ])
        .assert()
        .success();

    let out = bin()
        .args([
            "--db", &db, "--json", "workout", "stats", "summary", "--days", "30",
        ])
        .assert()
        .success();
    let v = json_out(out);
    assert_eq!(v["days"], 30);
    assert_eq!(v["total_workouts"], 1);
    assert_eq!(v["total_duration_minutes"], 60);
    assert_eq!(v["average_duration_minutes"], 60);
    assert_eq!(v["set_count"], 2);
    assert_eq!(v["days_trained"], 1);
}

#[test]
fn stats_history_and_weight() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    setup_bench(&db);

    let hist = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "history",
            "--exercise",
            "bench press",
            "--days",
            "90",
        ])
        .assert()
        .success();
    let h = json_out(hist);
    let arr = h.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["set_number"], 1);
    assert_eq!(arr[0]["reps"], 5);
    assert_eq!(arr[1]["set_number"], 2);
    assert_eq!(arr[1]["reps"], 15);

    let weight = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "weight",
            "--exercise",
            "bench press",
        ])
        .assert()
        .success();
    let w = json_out(weight);
    let warr = w.as_array().unwrap();
    assert_eq!(warr.len(), 2);
    assert!((warr[0]["weight_kg"].as_f64().unwrap() - 100.0).abs() < 0.01);
    assert!(warr[0]["load_display"].as_str().unwrap().contains("100"));

    // Human path non-empty
    bin()
        .args([
            "--db",
            &db,
            "workout",
            "stats",
            "weight",
            "--exercise",
            "bench press",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Load history"));

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "history",
            "--exercise",
            "no such exercise",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("exercise not found"));
}

#[test]
fn stats_history_exact_name() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();
    for name in ["dip", "ring dip"] {
        bin()
            .args([
                "--db",
                &db,
                "--json",
                "workout",
                "exercise",
                "create",
                name,
                "--category",
                "calisthenics",
                "--load-type",
                "body_mass",
            ])
            .assert()
            .success();
    }
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
            "dip",
            "--reps",
            "5",
            "--weight",
            "80",
            "--phase",
            "working",
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
            "ring dip",
            "--reps",
            "8",
            "--weight",
            "80",
            "--phase",
            "working",
        ])
        .assert()
        .success();

    let dip = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "history",
            "--exercise",
            "dip",
            "--days",
            "30",
        ])
        .assert()
        .success();
    let d = json_out(dip);
    assert_eq!(d.as_array().unwrap().len(), 1);
    assert_eq!(d[0]["exercise"], "dip");
    assert_eq!(d[0]["reps"], 5);

    let ring = bin()
        .args([
            "--db",
            &db,
            "--json",
            "workout",
            "stats",
            "history",
            "--exercise",
            "ring dip",
            "--days",
            "30",
        ])
        .assert()
        .success();
    let r = json_out(ring);
    assert_eq!(r.as_array().unwrap().len(), 1);
    assert_eq!(r[0]["exercise"], "ring dip");
}
