use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn db_path(dir: &TempDir) -> String {
    dir.path().join("t.db").display().to_string()
}

#[test]
fn check_reports_absurd_set_weight() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();

    bin()
        .args(["--db", &db, "--json", "workout", "create", "--type", "Push"])
        .assert()
        .success();

    // Valid set via CLI (write-path sanity passes).
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
        ])
        .assert()
        .success();

    // Plant an outlier that bypasses write-path validation (e.g. legacy import).
    let conn = Connection::open(&db).unwrap();
    let we_id: i64 = conn
        .query_row(
            "SELECT id FROM workout_exercises WHERE workout_id = 1 LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO exercise_sets
         (workout_exercise_id, set_number, reps, weight_kg, phase)
         VALUES (?1, 99, 1, 9999.0, 'working')",
        [we_id],
    )
    .unwrap();
    drop(conn);

    // Full check (all-time): must fail with a set violation.
    let assert = bin()
        .args(["--db", &db, "--json", "check"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("\"set_count\""))
        .stdout(predicate::str::contains("\"entity\": \"set\""))
        .stdout(predicate::str::contains("weight_kg"))
        .stdout(predicate::str::contains("\"hard_violation_count\""));

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("9999") || stdout.contains("weight_kg"),
        "expected weight_kg violation in: {stdout}"
    );

    // Empty body-only DB style: set_count present and ok when no outliers.
    let dir2 = TempDir::new().unwrap();
    let db2 = db_path(&dir2);
    bin().args(["--db", &db2, "init"]).assert().success();
    bin()
        .args([
            "--db",
            &db2,
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
        .success();
    bin()
        .args(["--db", &db2, "--json", "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"set_count\": 0"))
        .stdout(predicate::str::contains("\"ok\": true"));
}

#[test]
fn check_date_window_uses_workout_session_day() {
    let dir = TempDir::new().unwrap();
    let db = db_path(&dir);

    bin().args(["--db", &db, "init"]).assert().success();

    // Workout far in the past with an absurd set (raw SQL for both).
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO workouts (id, started_at, workout_type) VALUES (1, '2020-01-15 10:00:00', 'Push')",
        [],
    )
    .unwrap();
    let exercise_id: i64 = conn
        .query_row(
            "SELECT id FROM exercises WHERE name = 'bench press' COLLATE NOCASE",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO workout_exercises (id, workout_id, exercise_id, \"order\")
         VALUES (1, 1, ?1, 1)",
        [exercise_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO exercise_sets
         (workout_exercise_id, set_number, reps, weight_kg, phase)
         VALUES (1, 1, 1, 9999.0, 'working')",
        [],
    )
    .unwrap();
    drop(conn);

    // Recent window should not include the 2020 workout.
    bin()
        .args(["--db", &db, "--json", "check", "--days", "7"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"set_count\": 0"))
        .stdout(predicate::str::contains("\"ok\": true"));

    // Explicit range covering 2020 should flag it.
    bin()
        .args([
            "--db",
            &db,
            "--json",
            "check",
            "--since",
            "2020-01-01",
            "--until",
            "2020-01-31",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("\"entity\": \"set\""))
        .stdout(predicate::str::contains("2020-01-15"));
}
