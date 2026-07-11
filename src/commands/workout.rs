//! Workout / exercise / set handlers.

use crate::cli::{ExerciseAction, SetAction, WorkoutAction};
use crate::db;
use crate::models::Success;
use crate::utils::{parse_flexible_datetime, print_error_json, print_json, quiet_print};
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};

pub fn handle(
    action: WorkoutAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        WorkoutAction::Create {
            started_at,
            workout_type,
            notes,
        } => {
            let conn = db::open_db(db_override)?;
            let started = match started_at {
                Some(s) => parse_flexible_datetime(&s)?,
                None => db::now_utc(),
            };
            conn.execute(
                "INSERT INTO workouts (started_at, workout_type, notes) VALUES (?1, ?2, ?3)",
                params![started, workout_type, notes],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(id, started.clone(), "workout created"));
            } else {
                quiet_print(
                    quiet,
                    format!("Created workout {} (started {})", id, started),
                );
            }
            Ok(())
        }
        WorkoutAction::List { days } => {
            let conn = db::open_db(db_override)?;
            let limit = days.unwrap_or(30);
            let mut stmt = conn.prepare(
                "SELECT id, started_at, workout_type, notes FROM workouts
                 ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows: Vec<_> = stmt
                .query_map([limit], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "started_at": r.get::<_, String>(1)?,
                        "workout_type": r.get::<_, Option<String>>(2)?,
                        "notes": r.get::<_, Option<String>>(3)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else if rows.is_empty() {
                println!("(no workouts)");
            } else {
                for w in &rows {
                    println!(
                        "{}  {}  {}",
                        w["id"],
                        w["started_at"].as_str().unwrap_or(""),
                        w["workout_type"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
        WorkoutAction::Show { id } => {
            let conn = db::open_db(db_override)?;
            let row = conn
                .query_row(
                    "SELECT id, started_at, finished_at, workout_type, notes, overall_feeling, duration_minutes
                     FROM workouts WHERE id = ?1",
                    [id],
                    |r| {
                        Ok(serde_json::json!({
                            "id": r.get::<_, i64>(0)?,
                            "started_at": r.get::<_, String>(1)?,
                            "finished_at": r.get::<_, Option<String>>(2)?,
                            "workout_type": r.get::<_, Option<String>>(3)?,
                            "notes": r.get::<_, Option<String>>(4)?,
                            "overall_feeling": r.get::<_, Option<i64>>(5)?,
                            "duration_minutes": r.get::<_, Option<i64>>(6)?,
                        }))
                    },
                )
                .optional()?;
            match row {
                Some(mut w) => {
                    // attach exercises + sets
                    let mut stmt = conn.prepare(
                        "SELECT we.id, e.name, we.\"order\", we.notes
                         FROM workout_exercises we
                         JOIN exercises e ON e.id = we.exercise_id
                         WHERE we.workout_id = ?1
                         ORDER BY we.\"order\"",
                    )?;
                    let exercises: Vec<_> = stmt
                        .query_map([id], |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, i64>(2)?,
                                r.get::<_, Option<String>>(3)?,
                            ))
                        })?
                        .filter_map(|r| r.ok())
                        .collect();
                    let mut ex_json = Vec::new();
                    for (we_id, name, order, notes) in exercises {
                        let mut sstmt = conn.prepare(
                            "SELECT id, set_number, reps, weight_kg, distance_km, duration_seconds, rpe, rir, phase, side
                             FROM exercise_sets WHERE workout_exercise_id = ?1 ORDER BY set_number",
                        )?;
                        let sets: Vec<_> = sstmt
                            .query_map([we_id], |r| {
                                Ok(serde_json::json!({
                                    "id": r.get::<_, i64>(0)?,
                                    "set_number": r.get::<_, i64>(1)?,
                                    "reps": r.get::<_, Option<i32>>(2)?,
                                    "weight_kg": r.get::<_, Option<f64>>(3)?,
                                    "distance_km": r.get::<_, Option<f64>>(4)?,
                                    "duration_seconds": r.get::<_, Option<i32>>(5)?,
                                    "rpe": r.get::<_, Option<f64>>(6)?,
                                    "rir": r.get::<_, Option<f64>>(7)?,
                                    "phase": r.get::<_, String>(8)?,
                                    "side": r.get::<_, Option<String>>(9)?,
                                }))
                            })?
                            .filter_map(|r| r.ok())
                            .collect();
                        ex_json.push(serde_json::json!({
                            "workout_exercise_id": we_id,
                            "name": name,
                            "order": order,
                            "notes": notes,
                            "sets": sets,
                        }));
                    }
                    w["exercises"] = serde_json::json!(ex_json);
                    if json {
                        print_json(&w);
                    } else {
                        println!(
                            "Workout {} — {} ({})",
                            id,
                            w["started_at"].as_str().unwrap_or(""),
                            w["workout_type"].as_str().unwrap_or("-")
                        );
                        for ex in &ex_json {
                            println!(
                                "  {}. {} (we_id={})",
                                ex["order"], ex["name"], ex["workout_exercise_id"]
                            );
                            for s in ex["sets"].as_array().unwrap_or(&vec![]) {
                                println!(
                                    "     set {}: reps={:?} weight={:?}",
                                    s["set_number"], s["reps"], s["weight_kg"]
                                );
                            }
                        }
                    }
                    Ok(())
                }
                None => {
                    if json {
                        print_error_json(&format!("workout {} not found", id));
                    } else {
                        eprintln!("workout {} not found", id);
                    }
                    Err(anyhow!("workout not found"))
                }
            }
        }
        WorkoutAction::Delete { id } => {
            let conn = db::open_db(db_override)?;
            let n = conn.execute("DELETE FROM workouts WHERE id = ?1", [id])?;
            if n == 0 {
                return Err(anyhow!("workout {} not found", id));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted workout {}", id));
            }
            Ok(())
        }
        WorkoutAction::Exercise { action } => handle_exercise(action, db_override, json, quiet),
        WorkoutAction::Set { action } => handle_set(action, db_override, json, quiet),
    }
}

fn handle_exercise(
    action: ExerciseAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        ExerciseAction::List { search, category } => {
            let mut sql =
                "SELECT id, name, category, equipment, load_type FROM exercises WHERE 1=1"
                    .to_string();
            let mut params_vec: Vec<String> = vec![];
            if let Some(cat) = &category {
                sql.push_str(&format!(" AND category = ?{}", params_vec.len() + 1));
                params_vec.push(cat.clone());
            }
            if let Some(term) = &search {
                sql.push_str(&format!(" AND name LIKE ?{}", params_vec.len() + 1));
                params_vec.push(format!("%{}%", term));
            }
            sql.push_str(" ORDER BY name LIMIT 200");
            let mut stmt = conn.prepare(&sql)?;
            let rows: Vec<_> = stmt
                .query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "category": r.get::<_, String>(2)?,
                        "equipment": r.get::<_, Option<String>>(3)?,
                        "load_type": r.get::<_, String>(4)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for e in &rows {
                    println!(
                        "{}: {} ({})",
                        e["id"],
                        e["name"].as_str().unwrap_or(""),
                        e["category"].as_str().unwrap_or("")
                    );
                }
            }
        }
        ExerciseAction::Create {
            name,
            category,
            equipment,
            load_type,
            muscles,
        } => {
            let name = name.trim().to_lowercase();
            let lt = load_type.unwrap_or_else(|| "weight".to_string());
            conn.execute(
                "INSERT INTO exercises (name, category, equipment, load_type, muscle_groups)
                 VALUES (?1,?2,?3,?4,?5)",
                params![name, category, equipment, lt, muscles],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(
                    id,
                    name.clone(),
                    format!("exercise created: {}", name),
                ));
            } else {
                quiet_print(quiet, format!("Exercise {}: {}", id, name));
            }
        }
        ExerciseAction::Search { term } => {
            let mut stmt = conn.prepare(
                "SELECT id, name, category FROM exercises WHERE name LIKE ?1 ORDER BY name LIMIT 50",
            )?;
            let rows: Vec<_> = stmt
                .query_map([format!("%{}%", term)], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "category": r.get::<_, String>(2)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for e in &rows {
                    println!("{}: {} ({})", e["id"], e["name"], e["category"]);
                }
            }
        }
    }
    Ok(())
}

fn resolve_exercise_id(conn: &rusqlite::Connection, exercise: &str) -> Result<i64> {
    if let Ok(id) = exercise.parse::<i64>() {
        return Ok(id);
    }
    let name = exercise.trim().to_lowercase();
    conn.query_row(
        "SELECT id FROM exercises WHERE name = ?1 COLLATE NOCASE",
        [&name],
        |r| r.get(0),
    )
    .map_err(|_| anyhow!("exercise not found: {}", exercise))
}

fn ensure_workout_exercise(
    conn: &rusqlite::Connection,
    workout: i64,
    exercise_id: i64,
) -> Result<i64> {
    let _: i64 = conn
        .query_row("SELECT id FROM workouts WHERE id = ?1", [workout], |r| {
            r.get(0)
        })
        .map_err(|_| anyhow!("workout not found: {}", workout))?;
    match conn
        .query_row(
            "SELECT id FROM workout_exercises WHERE workout_id = ?1 AND exercise_id = ?2 LIMIT 1",
            params![workout, exercise_id],
            |r| r.get(0),
        )
        .optional()?
    {
        Some(id) => Ok(id),
        None => {
            let order: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(\"order\"), 0) + 1 FROM workout_exercises WHERE workout_id = ?1",
                    [workout],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            conn.execute(
                "INSERT INTO workout_exercises (workout_id, exercise_id, \"order\") VALUES (?1,?2,?3)",
                params![workout, exercise_id, order],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }
}

fn next_set_number(conn: &rusqlite::Connection, we_id: i64) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(set_number), 0) + 1 FROM exercise_sets WHERE workout_exercise_id = ?1",
        [we_id],
        |r| r.get(0),
    )
    .unwrap_or(1)
}

fn handle_set(action: SetAction, db_override: Option<&str>, json: bool, quiet: bool) -> Result<()> {
    match action {
        SetAction::Add {
            workout,
            exercise,
            reps,
            weight,
            external_load,
            duration,
            distance,
            rpe,
            rir,
            effective_reps,
            rest_seconds,
            notes,
            side,
            phase,
        } => {
            let conn = db::open_db(db_override)?;
            let exercise_id = resolve_exercise_id(&conn, &exercise)?;
            let we_id = ensure_workout_exercise(&conn, workout, exercise_id)?;
            let set_number = next_set_number(&conn, we_id);
            if reps.is_none()
                && weight.is_none()
                && duration.is_none()
                && distance.is_none()
                && external_load.is_none()
            {
                return Err(anyhow!(
                    "provide at least one of --reps, --weight, --duration, --distance, --external-load"
                ));
            }
            conn.execute(
                "INSERT INTO exercise_sets
                 (workout_exercise_id, set_number, reps, weight_kg, external_load_kg,
                  distance_km, duration_seconds, rpe, rir, effective_reps, rest_seconds,
                  notes, side, phase)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    we_id,
                    set_number,
                    reps,
                    weight,
                    external_load,
                    distance,
                    duration,
                    rpe,
                    rir,
                    effective_reps,
                    rest_seconds,
                    notes,
                    side,
                    phase
                ],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(
                    id,
                    format!("set {}", set_number),
                    "set added",
                ));
            } else {
                quiet_print(
                    quiet,
                    format!(
                        "Added set {} (id {}) to workout {}",
                        set_number, id, workout
                    ),
                );
            }
            Ok(())
        }
        SetAction::AddCardio {
            workout,
            exercise,
            distance,
            duration,
            avg_heart_rate,
            max_heart_rate,
            pace,
            calories,
            notes,
        } => {
            let conn = db::open_db(db_override)?;
            let exercise_id = resolve_exercise_id(&conn, &exercise)?;
            let we_id = ensure_workout_exercise(&conn, workout, exercise_id)?;
            let set_number = next_set_number(&conn, we_id);
            if distance.is_none() && duration.is_none() {
                return Err(anyhow!("cardio set needs --distance and/or --duration"));
            }
            conn.execute(
                "INSERT INTO exercise_sets
                 (workout_exercise_id, set_number, distance_km, duration_seconds,
                  avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km, calories_burned,
                  notes, phase)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'working')",
                params![
                    we_id,
                    set_number,
                    distance,
                    duration,
                    avg_heart_rate,
                    max_heart_rate,
                    pace,
                    calories,
                    notes
                ],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(
                    id,
                    format!("set {}", set_number),
                    "cardio set added",
                ));
            } else {
                quiet_print(
                    quiet,
                    format!("Added cardio set {} (id {})", set_number, id),
                );
            }
            Ok(())
        }
        SetAction::Delete { id } => {
            let conn = db::open_db(db_override)?;
            let n = conn.execute("DELETE FROM exercise_sets WHERE id = ?1", [id])?;
            if n == 0 {
                return Err(anyhow!("set {} not found", id));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted set {}", id));
            }
            Ok(())
        }
    }
}
