//! E2E FIT import using the Zepp running fixture.

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use std::path::PathBuf;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Zepp20260710164935.fit")
}

#[test]
fn import_fit_zepp_running_e2e() {
    let path = fixture_path();
    assert!(
        path.exists(),
        "fixture missing: {} (copy Zepp20260710164935.fit into tests/fixtures/)",
        path.display()
    );

    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db");
    let db_s = db.display().to_string();
    let fit = path.display().to_string();

    bin().args(["--db", &db_s, "init"]).assert().success();

    // dry-run
    bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "fit",
            &fit,
            "--exercise",
            "running",
            "--dry-run",
            "--no-profile-hr",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\": true"))
        .stdout(predicate::str::contains("8.027"))
        .stdout(predicate::str::contains("2808"))
        .stdout(predicate::str::contains("2809"));

    // real import
    bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "fit",
            &fit,
            "--type",
            "Run",
            "--notes",
            "test import",
            "--no-profile-hr",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"))
        .stdout(predicate::str::contains("\"workout_id\""));

    // Assert DB contents (repslog parity numbers)
    let conn = Connection::open(&db).unwrap();

    let (wtype, notes, started, dur_min): (Option<String>, Option<String>, String, Option<i64>) =
        conn.query_row(
            "SELECT workout_type, notes, started_at, duration_minutes FROM workouts WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(wtype.as_deref(), Some("Run"));
    assert!(notes.as_ref().unwrap().contains("test import"));
    assert!(started.starts_with("2026-07-10"));
    assert_eq!(dur_min, Some(47)); // 2808/60 ≈ 46.8 → 47

    let (dist, duration, avg_hr, max_hr, cal, cadence, ascent, descent, pace): (
        f64,
        i32,
        f64,
        f64,
        i32,
        f64,
        f64,
        f64,
        f64,
    ) = conn
        .query_row(
            "SELECT distance_km, duration_seconds, avg_heart_rate_bpm, max_heart_rate_bpm,
                    calories_burned, avg_cadence_spm, total_ascent_m, total_descent_m,
                    avg_pace_min_per_km
             FROM exercise_sets WHERE id = 1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .unwrap();

    assert!((dist - 8.027).abs() < 0.02);
    assert_eq!(duration, 2808);
    assert_eq!(avg_hr, 156.0);
    assert_eq!(max_hr, 175.0);
    assert_eq!(cal, 597);
    assert_eq!(cadence, 77.0);
    assert_eq!(ascent, 12.0);
    assert_eq!(descent, 11.0);
    assert!(pace > 5.0 && pace < 6.5);

    let tp_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM activity_trackpoints WHERE exercise_set_id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        tp_count > 1000,
        "expected many trackpoints, got {tp_count}"
    );

    let imports: i64 = conn
        .query_row("SELECT COUNT(*) FROM activity_imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(imports, 1);

    // Duplicate import fails
    bin()
        .args([
            "--db",
            &db_s,
            "import",
            "fit",
            &fit,
            "--no-profile-hr",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already imported"));

    // Force re-import succeeds
    bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "fit",
            &fit,
            "--force",
            "--no-profile-hr",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));
}

#[test]
fn import_fit_with_hr_zone_bounds() {
    let path = fixture_path();
    assert!(path.exists());

    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db");
    let db_s = db.display().to_string();
    let fit = path.display().to_string();

    bin().args(["--db", &db_s, "init"]).assert().success();

    bin()
        .args([
            "--db",
            &db_s,
            "--json",
            "import",
            "fit",
            &fit,
            "--hr-zone-bounds",
            "120,140,160,175,190",
            "--no-profile-hr",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    let conn = Connection::open(&db).unwrap();
    let zones: Option<String> = conn
        .query_row(
            "SELECT heart_rate_zones FROM exercise_sets WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        zones.is_some(),
        "expected heart_rate_zones JSON when --hr-zone-bounds set"
    );
    let z: serde_json::Value = serde_json::from_str(zones.as_ref().unwrap()).unwrap();
    assert!(z.get("z1_seconds").is_some());
}
