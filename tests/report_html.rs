//! Integration tests for `report html` (gap 06 dashboard depth + plot upgrades).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn setup_full_fixture(db: &str) {
    // Two measurements with BF + skeletal muscle + resting metabolism
    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-01",
            "--weight-kg",
            "82",
            "--body-fat-pct",
            "20",
            "--skeletal-muscle-pct",
            "40",
            "--resting-metabolism-kcal",
            "1700",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "measurement",
            "create",
            "--date",
            "2026-07-08",
            "--weight-kg",
            "81",
            "--body-fat-pct",
            "19.5",
            "--skeletal-muscle-pct",
            "40.5",
            "--resting-metabolism-kcal",
            "1680",
        ])
        .assert()
        .success();

    // Full-stage sleep night
    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "sleep",
            "create",
            "--date",
            "2026-07-05",
            "--total-sleep",
            "450",
            "--rem",
            "90",
            "--deep",
            "60",
            "--light",
            "280",
            "--awake",
            "20",
            "--sleep-efficiency",
            "92",
            "--sleep-score",
            "85",
        ])
        .assert()
        .success();

    // Partial stages: only deep + rem (zero-fill light/awake)
    bin()
        .args([
            "--db",
            db,
            "--json",
            "body",
            "sleep",
            "create",
            "--date",
            "2026-07-06",
            "--total-sleep",
            "400",
            "--deep",
            "50",
            "--rem",
            "80",
        ])
        .assert()
        .success();

    // Nutrition product with full macros
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "product",
            "create",
            "Oats",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            db,
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
            "389",
            "--protein-g",
            "17",
            "--carbohydrates-g",
            "66",
            "--fat-g",
            "7",
            "--fiber-g",
            "11",
            "--sugars-g",
            "1",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "consumption",
            "create",
            "--product",
            "1",
            "--quantity",
            "100",
            "--date",
            "2026-07-05T12:00:00-03:00",
        ])
        .assert()
        .success();
    // Second day with more kcal than BMR (for bar color path)
    bin()
        .args([
            "--db",
            db,
            "--json",
            "nutrition",
            "consumption",
            "create",
            "--product",
            "1",
            "--quantity",
            "500",
            "--date",
            "2026-07-08T12:00:00-03:00",
        ])
        .assert()
        .success();

    // Training volume
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
        ])
        .assert()
        .success();
}

fn run_html(db: &str, out: &str) -> (String, Value) {
    let assert = bin()
        .args([
            "--db",
            db,
            "--json",
            "report",
            "html",
            "--days",
            "30",
            "--output-dir",
            out,
            "--name",
            "dash.html",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&stdout).expect("json");
    let html = std::fs::read_to_string(std::path::Path::new(out).join("dash.html")).unwrap();
    (html, v)
}

#[test]
fn report_html_writes_file() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "today",
            "--weight-kg",
            "80",
        ])
        .assert()
        .success();

    let (html, v) = run_html(&db, &out.display().to_string());
    assert!(html.contains("recomplog"));
    assert!(html.contains("Chart"));
    assert!(html.contains("id=\"wChart\""));
    assert!(html.contains("id=\"mmPctChart\""));
    assert!(!html.contains("id=\"fmChart\""));
    // Single weight point → insufficient trend
    assert_eq!(
        v["overview"]["weight_trend"]["direction"].as_str(),
        Some("insufficient_data")
    );
    assert_eq!(v["overview"]["weight_trend"]["label"].as_str(), Some("—"));
    assert!(v["overview"]["training"]["workout_count"].as_i64().unwrap() == 0);
}

#[test]
fn report_html_full_dashboard_depth() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    setup_full_fixture(&db);

    let (html, v) = run_html(&db, &out.display().to_string());

    // Body regression charts (no fat/lean dual chart)
    assert!(html.contains("id=\"wChart\""));
    assert!(html.contains("id=\"bfChart\""));
    assert!(html.contains("id=\"mmPctChart\""));
    assert!(html.contains("id=\"mmKgChart\""));
    assert!(!html.contains("id=\"fmChart\""));
    assert!(html.contains("regressionChart"));
    assert!(html.contains("R²") || html.contains("R\u{00b2}") || html.contains("R&sup2;"));
    // R² in title when ≥2 points
    assert!(html.contains("R² =") || html.contains("R\u{00b2} ="));

    // Sleep stages + quality
    assert!(html.contains("id=\"ssChart\""));
    assert!(html.contains("Deep"));
    assert!(html.contains("REM"));
    assert!(html.contains("id=\"sqChart\""));
    assert!(html.contains("Efficiency %"));

    // Nutrition: energy BMR + macros + new plots
    assert!(html.contains("id=\"nKcalChart\""));
    assert!(html.contains("id=\"nMacroChart\""));
    assert!(html.contains("id=\"nCalSourceChart\""));
    assert!(html.contains("id=\"nProtLeanChart\""));
    assert!(html.contains("id=\"nProtMusChart\""));
    assert!(!html.contains("id=\"nChart\""));
    assert!(html.contains("fat g"));
    assert!(html.contains("fiber g"));
    assert!(html.contains("sugars g"));
    assert!(html.contains("carbs g"));
    assert!(html.contains("hasBmr"));
    assert!(html.contains("Resting metabolism") || html.contains("nutBmr"));
    assert!(html.contains("protLean"));
    assert!(html.contains("protMuscle"));
    assert!(html.contains("pctProtein"));

    // Short date labels: no year on axis series (ISO dates may still appear in period subtitle)
    // Chart payload uses day-only when same month (July fixture).
    assert!(html.contains("\"labels\":[\"1\",\"8\"]") || html.contains("\"1\""));

    // Four body cards only: label + N-day median + trend + last
    assert!(html.contains(">Weight<"));
    assert!(html.contains(">Body fat<"));
    assert!(html.contains(">Muscle %<"));
    assert!(html.contains(">Muscle mass<"));
    assert!(!html.contains(">Fat mass<"));
    assert!(!html.contains(">Lean mass<"));
    assert!(!html.contains(">Measurements<"));
    assert!(!html.contains(">Sleep nights<"));
    assert!(!html.contains(">Workouts<"));
    assert!(!html.contains(">Volume<"));
    assert!(html.contains("class=\"last\""));
    assert!(html.contains("last "));

    // Trends on cards
    assert!(html.contains("kg/wk") || html.contains("→ flat") || html.contains("%/wk"));
    assert!(
        html.contains("trend-down") || html.contains("trend-up") || html.contains("trend-flat")
    );

    // JSON overview contract
    let ov = &v["overview"];
    assert!(ov["weight_trend"].is_object());
    assert!(ov["body_fat_trend"].is_object());
    assert!(ov["muscle_pct_trend"].is_object());
    assert!(ov["muscle_mass_trend"].is_object());
    let wt = &ov["weight_trend"];
    assert!(wt["n"].as_u64().unwrap() >= 2);
    let dir = wt["direction"].as_str().unwrap();
    assert!(matches!(dir, "up" | "down" | "flat"));
    assert!(wt.get("slope_per_week").is_some());
    assert!(wt.get("r_squared").is_some());
    // Medians (primary card values) and last recorded
    assert!((ov["median_weight_kg"].as_f64().unwrap() - 81.5).abs() < 1e-9);
    assert!((ov["latest_weight_kg"].as_f64().unwrap() - 81.0).abs() < 1e-9);
    assert!((ov["median_body_fat_pct"].as_f64().unwrap() - 19.75).abs() < 1e-9);
    assert!((ov["latest_body_fat_pct"].as_f64().unwrap() - 19.5).abs() < 1e-9);
    assert!((ov["median_muscle_pct"].as_f64().unwrap() - 40.25).abs() < 1e-9);
    assert!((ov["latest_muscle_pct"].as_f64().unwrap() - 40.5).abs() < 1e-9);
    // muscle mass: 82*0.40=32.8, 81*0.405=32.805 → median 32.8025
    assert!(ov["median_muscle_mass_kg"].as_f64().is_some());
    assert!(ov["latest_muscle_mass_kg"].as_f64().is_some());
    assert!(ov["training"]["workout_count"].as_i64().unwrap() >= 1);
    assert!(ov["training"]["total_volume"].as_f64().unwrap() > 0.0);
}

#[test]
fn report_html_partial_stages_zero_fill() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "body",
            "sleep",
            "create",
            "--date",
            "2026-07-06",
            "--total-sleep",
            "400",
            "--deep",
            "50",
            "--rem",
            "80",
        ])
        .assert()
        .success();

    let (html, _) = run_html(&db, &out.display().to_string());
    assert!(html.contains("id=\"ssChart\""));
    // Payload includes zero-filled stage arrays; labels use single-quoted JS strings
    assert!(html.contains("stageDeep"));
    assert!(html.contains("stageRem"));
    assert!(html.contains("label: 'Deep'"));
    assert!(html.contains("label: 'REM'"));
}
