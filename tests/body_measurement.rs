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
fn measurement_medians_window_and_sparse() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    // 5 consecutive days with weights 70..74; body fat only on some days
    let seeds = [
        ("2026-01-01", "70", Some("20")),
        ("2026-01-02", "71", Some("21")),
        ("2026-01-03", "72", None), // no BF
        ("2026-01-04", "73", Some("23")),
        ("2026-01-05", "74", Some("24")),
    ];
    for (date, w, bf) in seeds {
        let mut args = vec![
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            date,
            "--weight-kg",
            w,
        ];
        if let Some(bf) = bf {
            args.extend(["--body-fat-pct", bf]);
        }
        bin().args(&args).assert().success();
    }

    // window 3 on 2026-01-05: weights 72,73,74 → median 73; n=3
    // BF: none, 23, 24 → median 23.5; n_by_field bf=2 → human "3 bf:2"
    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "medians",
            "--window",
            "3",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let row = &v[0];
    assert_eq!(row["date"], "2026-01-05");
    assert_eq!(row["n"], 3);
    assert!((row["weight_kg"].as_f64().unwrap() - 73.0).abs() < 1e-9);
    assert!((row["body_fat_pct"].as_f64().unwrap() - 23.5).abs() < 1e-9);
    assert_eq!(row["n_by_field"]["weight_kg"], 3);
    assert_eq!(row["n_by_field"]["body_fat_pct"], 2);
    assert_eq!(row["n_by_field"]["bmi"], 0);

    // Gap: no measurement on 2026-01-03 — use a fresh DB with gap for n < window
    let dir2 = TempDir::new().unwrap();
    let db2 = db_path(&dir2);
    for (date, w) in [("2026-02-01", "80"), ("2026-02-03", "82")] {
        bin()
            .args([
                "--db",
                &db2,
                "body",
                "measurement",
                "create",
                "--date",
                date,
                "--weight-kg",
                w,
            ])
            .assert()
            .success();
    }
    // On 2026-02-03 window 3 calendar days [02-01, 02-03]: only 2 rows → n=2, median (80+82)/2=81
    let out = bin()
        .args([
            "--db",
            &db2,
            "--json",
            "body",
            "measurement",
            "medians",
            "--window",
            "3",
            "--since",
            "2026-02-03",
            "--until",
            "2026-02-03",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v[0]["n"], 2);
    assert!((v[0]["weight_kg"].as_f64().unwrap() - 81.0).abs() < 1e-9);

    // Lookback beyond display: display only 2026-01-05, window 5 needs 01-01..01-05
    let out = bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "medians",
            "--window",
            "5",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    // weights 70,71,72,73,74 → median 72
    assert_eq!(v[0]["n"], 5);
    assert!((v[0]["weight_kg"].as_f64().unwrap() - 72.0).abs() < 1e-9);

    // Human path includes N header / sparse suffix
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "medians",
            "--window",
            "3",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("N"))
        .stdout(predicate::str::contains("bf:2"));

    // Invalid window
    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "medians",
            "--window",
            "0",
            "--days",
            "7",
        ])
        .assert()
        .failure();

    // Default window is 7 (no --window flag)
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "measurement",
            "medians",
            "--since",
            "2026-01-05",
            "--until",
            "2026-01-05",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"n\": 5")); // all 5 seeded days in 7-day window
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
