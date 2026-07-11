//! Workout / exercise / set handlers (repslog parity under grouped CLI).

use crate::bodyweight;
use crate::cli::{ExerciseAction, SetAction, WorkoutAction};
use crate::config::WorkoutSanityLimits;
use crate::db;
use crate::load_type;
use crate::models::{Exercise, HeartRateZones, Laps, Success, Trackpoint};
use crate::phase;
use crate::sanity::{self, ProposedSetMetrics};
use crate::track_metrics::{compute, compute_with_zones, TrackMetrics, ZoneRecomputeContext};
use crate::utils::{
    parse_flexible_datetime, print_error_json, print_json, print_table, quiet_print,
};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub fn handle(
    action: WorkoutAction,
    db_override: Option<&str>,
    workout_limits: &WorkoutSanityLimits,
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
                quiet_print(quiet, format!("Created workout {id} (started {started})"));
            }
            Ok(())
        }
        WorkoutAction::List { days, limit } => {
            let conn = db::open_db(db_override)?;
            let lim = limit.max(1);
            let mut sql = String::from(
                "SELECT id, started_at, workout_type, notes, duration_minutes, overall_feeling
                 FROM workouts WHERE 1=1",
            );
            let mut binds: Vec<String> = vec![];
            if let Some(d) = days {
                sql.push_str(" AND date(started_at) >= date('now', ?)");
                binds.push(format!("-{} days", d.saturating_sub(1)));
            }
            sql.push_str(" ORDER BY started_at DESC LIMIT ?");
            let mut stmt = conn.prepare(&sql)?;
            let mut param_vals: Vec<Box<dyn rusqlite::ToSql>> = binds
                .into_iter()
                .map(|s| Box::new(s) as Box<dyn rusqlite::ToSql>)
                .collect();
            param_vals.push(Box::new(lim));
            let refs: Vec<&dyn rusqlite::ToSql> = param_vals.iter().map(|b| b.as_ref()).collect();
            let rows: Vec<_> = stmt
                .query_map(refs.as_slice(), |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "started_at": r.get::<_, String>(1)?,
                        "workout_type": r.get::<_, Option<String>>(2)?,
                        "notes": r.get::<_, Option<String>>(3)?,
                        "duration_minutes": r.get::<_, Option<i64>>(4)?,
                        "overall_feeling": r.get::<_, Option<i64>>(5)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else if rows.is_empty() {
                println!("(no workouts)");
            } else {
                let table_rows: Vec<Vec<String>> = rows
                    .iter()
                    .map(|w| {
                        vec![
                            w["id"].to_string(),
                            w["started_at"].as_str().unwrap_or("").to_string(),
                            w["workout_type"].as_str().unwrap_or("").to_string(),
                            w["duration_minutes"]
                                .as_i64()
                                .map(|d| d.to_string())
                                .unwrap_or_default(),
                        ]
                    })
                    .collect();
                print_table(vec!["id", "started", "type", "min"], table_rows);
            }
            Ok(())
        }
        WorkoutAction::Show { id } => show_workout(db_override, id, json),
        WorkoutAction::Update {
            id,
            workout_type,
            notes,
            duration,
            feeling,
            started_at,
        } => {
            let conn = db::open_db(db_override)?;
            let exists: Option<i64> = conn
                .query_row("SELECT id FROM workouts WHERE id=?1", [id], |r| r.get(0))
                .optional()?;
            if exists.is_none() {
                return Err(anyhow!("workout {id} not found"));
            }
            if let Some(f) = feeling {
                if !(1..=5).contains(&f) {
                    return Err(anyhow!("feeling must be 1-5"));
                }
            }
            let started = started_at
                .as_ref()
                .map(|s| parse_flexible_datetime(s))
                .transpose()?;
            // Dynamic partial update
            let mut sets = vec![];
            let mut vals: Vec<Box<dyn rusqlite::ToSql>> = vec![];
            if let Some(v) = workout_type {
                sets.push("workout_type = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = notes {
                sets.push("notes = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = duration {
                sets.push("duration_minutes = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = feeling {
                sets.push("overall_feeling = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = started {
                sets.push("started_at = ?");
                vals.push(Box::new(v));
            }
            if sets.is_empty() {
                return Err(anyhow!("provide at least one field to update"));
            }
            let sql = format!("UPDATE workouts SET {} WHERE id = ?", sets.join(", "));
            vals.push(Box::new(id));
            let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, refs.as_slice())?;
            if json {
                print_json(&Success::created(id, "updated", "workout updated"));
            } else {
                quiet_print(quiet, format!("Updated workout {id}"));
            }
            Ok(())
        }
        WorkoutAction::Delete { id } => {
            let conn = db::open_db(db_override)?;
            let n = conn.execute("DELETE FROM workouts WHERE id = ?1", [id])?;
            if n == 0 {
                return Err(anyhow!("workout {id} not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted workout {id}"));
            }
            Ok(())
        }
        WorkoutAction::Stats { days } => {
            let conn = db::open_db(db_override)?;
            let since = format!("-{} days", days.saturating_sub(1));
            let mut stmt = conn.prepare(
                "SELECT e.name,
                        COUNT(s.id) as sets,
                        COALESCE(SUM(s.reps), 0) as total_reps,
                        COALESCE(SUM(CASE WHEN s.weight_kg IS NOT NULL AND s.reps IS NOT NULL
                            THEN s.weight_kg * s.reps ELSE 0 END), 0) as volume
                 FROM exercise_sets s
                 JOIN workout_exercises we ON we.id = s.workout_exercise_id
                 JOIN exercises e ON e.id = we.exercise_id
                 JOIN workouts w ON w.id = we.workout_id
                 WHERE date(w.started_at) >= date('now', ?1)
                 GROUP BY e.name
                 ORDER BY volume DESC
                 LIMIT 50",
            )?;
            let rows: Vec<_> = stmt
                .query_map([&since], |r| {
                    Ok(serde_json::json!({
                        "exercise": r.get::<_, String>(0)?,
                        "sets": r.get::<_, i64>(1)?,
                        "total_reps": r.get::<_, i64>(2)?,
                        "volume_kg_reps": r.get::<_, f64>(3)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            let out = serde_json::json!({ "days": days, "by_exercise": rows });
            if json {
                print_json(&out);
            } else {
                println!("Workout volume (last {days} days):");
                for r in &rows {
                    println!(
                        "  {}: sets={} reps={} volume={:.0}",
                        r["exercise"], r["sets"], r["total_reps"], r["volume_kg_reps"]
                    );
                }
            }
            Ok(())
        }
        WorkoutAction::Exercise { action } => handle_exercise(action, db_override, json, quiet),
        WorkoutAction::Set { action } => {
            handle_set(action, db_override, workout_limits, json, quiet)
        }
    }
}

fn activity_date_prefix(started_at: &str) -> String {
    started_at.get(..10).unwrap_or(started_at).to_string()
}

fn list_trackpoints(conn: &Connection, exercise_set_id: i64) -> Result<Vec<Trackpoint>> {
    let mut stmt = conn.prepare(
        "SELECT recorded_at, latitude, longitude, altitude_m, heart_rate_bpm,
                cadence_spm, distance_km, speed_m_s
         FROM activity_trackpoints
         WHERE exercise_set_id = ?1
         ORDER BY recorded_at, id",
    )?;
    let rows = stmt
        .query_map([exercise_set_id], |r| {
            Ok(Trackpoint {
                recorded_at: r.get(0)?,
                latitude: r.get(1)?,
                longitude: r.get(2)?,
                altitude_m: r.get(3)?,
                heart_rate_bpm: r.get(4)?,
                cadence_spm: r.get(5)?,
                distance_km: r.get(6)?,
                speed_m_s: r.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn track_metrics_for_set(
    conn: &Connection,
    set_id: i64,
    distance_km: Option<f64>,
    date_of_birth: Option<String>,
    resting_hr_bpm: Option<f64>,
    activity_date: &str,
) -> Result<Option<TrackMetrics>> {
    let points = list_trackpoints(conn, set_id)?;
    if points.is_empty() {
        return Ok(None);
    }
    if date_of_birth.is_none() && resting_hr_bpm.is_none() {
        return Ok(compute(&points, distance_km));
    }
    let ctx = ZoneRecomputeContext {
        date_of_birth,
        resting_hr_bpm,
        activity_date: Some(activity_date.to_string()),
    };
    Ok(compute_with_zones(&points, distance_km, &ctx))
}

fn format_secs_compact(secs: u32) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn format_pace_min_per_km(pace: f64) -> String {
    if !pace.is_finite() || pace <= 0.0 {
        return "--".into();
    }
    let total_secs = (pace * 60.0).round() as i64;
    let m = total_secs / 60;
    let s = total_secs % 60;
    format!("{m}:{s:02}/km")
}

fn print_track_metrics_oneline(m: &TrackMetrics) {
    let mut parts = vec![format!("{} samples", m.sample_count)];
    parts.push(format!(
        "moving {} (stopped {})",
        format_secs_compact(m.moving_seconds),
        format_secs_compact(m.stopped_seconds)
    ));
    if let Some(p) = m.moving_pace_min_per_km {
        parts.push(format!("pace ~{}", format_pace_min_per_km(p)));
    }
    if let Some(a) = m.ascent_m {
        parts.push(format!("↑{a:.0}m"));
    }
    println!("       track: {}", parts.join(" · "));
}

fn show_workout(db_override: Option<&str>, id: i64, json: bool) -> Result<()> {
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
    let Some(mut w) = row else {
        if json {
            print_error_json(&format!("workout {id} not found"));
        }
        return Err(anyhow!("workout not found"));
    };
    let started_at = w["started_at"].as_str().unwrap_or("").to_string();
    let activity_date = activity_date_prefix(&started_at);
    let mut stmt = conn.prepare(
        r#"SELECT we.id, e.name, we."order", we.notes, e.load_type
           FROM workout_exercises we
           JOIN exercises e ON e.id = we.exercise_id
           WHERE we.workout_id = ?1
           ORDER BY we."order""#,
    )?;
    let exercises: Vec<_> = stmt
        .query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    let mut ex_json = Vec::new();
    for (we_id, name, order, notes, load_type) in exercises {
        let sets = list_sets_json(&conn, we_id, &activity_date)?;
        ex_json.push(serde_json::json!({
            "workout_exercise_id": we_id,
            "name": name,
            "order": order,
            "notes": notes,
            "load_type": load_type,
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
                    "     set {}: reps={:?} weight={:?} phase={}",
                    s["set_number"], s["reps"], s["weight_kg"], s["phase"]
                );
                if let Some(tm_val) = s.get("track_metrics") {
                    if let Ok(tm) = serde_json::from_value::<TrackMetrics>(tm_val.clone()) {
                        print_track_metrics_oneline(&tm);
                    }
                }
            }
        }
    }
    Ok(())
}

fn list_sets_json(
    conn: &Connection,
    we_id: i64,
    activity_date: &str,
) -> Result<Vec<serde_json::Value>> {
    let mut sstmt = conn.prepare(
        "SELECT id, set_number, reps, weight_kg, external_load_kg, distance_km, duration_seconds,
                rpe, rir, effective_reps, cluster_id, rest_seconds, notes, side, phase,
                avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km, calories_burned,
                heart_rate_zones, laps, date_of_birth, resting_hr_bpm
         FROM exercise_sets WHERE workout_exercise_id = ?1 ORDER BY set_number",
    )?;
    let mut sets = Vec::new();
    let mut rows = sstmt.query([we_id])?;
    while let Some(r) = rows.next()? {
        let set_id: i64 = r.get(0)?;
        let distance_km: Option<f64> = r.get(5)?;
        let zones: Option<String> = r.get(19)?;
        let laps: Option<String> = r.get(20)?;
        let date_of_birth: Option<String> = r.get(21)?;
        let resting_hr_bpm: Option<f64> = r.get(22)?;
        let mut set_json = serde_json::json!({
            "id": set_id,
            "set_number": r.get::<_, i64>(1)?,
            "reps": r.get::<_, Option<i32>>(2)?,
            "weight_kg": r.get::<_, Option<f64>>(3)?,
            "external_load_kg": r.get::<_, Option<f64>>(4)?,
            "distance_km": distance_km,
            "duration_seconds": r.get::<_, Option<i32>>(6)?,
            "rpe": r.get::<_, Option<f64>>(7)?,
            "rir": r.get::<_, Option<f64>>(8)?,
            "effective_reps": r.get::<_, Option<i32>>(9)?,
            "cluster_id": r.get::<_, Option<i64>>(10)?,
            "rest_seconds": r.get::<_, Option<i32>>(11)?,
            "notes": r.get::<_, Option<String>>(12)?,
            "side": r.get::<_, Option<String>>(13)?,
            "phase": r.get::<_, String>(14)?,
            "avg_heart_rate_bpm": r.get::<_, Option<f64>>(15)?,
            "max_heart_rate_bpm": r.get::<_, Option<f64>>(16)?,
            "avg_pace_min_per_km": r.get::<_, Option<f64>>(17)?,
            "calories_burned": r.get::<_, Option<i32>>(18)?,
            "heart_rate_zones": zones.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
            "laps": laps.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
        });
        if let Some(tm) = track_metrics_for_set(
            conn,
            set_id,
            distance_km,
            date_of_birth,
            resting_hr_bpm,
            activity_date,
        )? {
            if let Some(obj) = set_json.as_object_mut() {
                obj.insert("track_metrics".to_string(), serde_json::to_value(&tm)?);
            }
        }
        sets.push(set_json);
    }
    Ok(sets)
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
                params_vec.push(format!("%{term}%"));
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
                        "{}: {} ({}, {})",
                        e["id"],
                        e["name"].as_str().unwrap_or(""),
                        e["category"].as_str().unwrap_or(""),
                        e["load_type"].as_str().unwrap_or("")
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
            description,
            allow_phase_in_name,
        } => {
            let name = normalize_exercise_name(&name);
            phase::validate_exercise_name_phase(&name, allow_phase_in_name)
                .map_err(|e| anyhow!("{e}"))?;
            let (lt, eq, deprecated) = load_type::resolve_for_new_exercise(
                &category,
                equipment.as_deref(),
                load_type.as_deref(),
            )
            .map_err(|e| anyhow!("{e}"))?;
            if deprecated && !quiet {
                eprintln!("Note: equipment 'bodyweight' is deprecated; use --load-type body_mass");
            }
            conn.execute(
                "INSERT INTO exercises (name, category, equipment, load_type, muscle_groups, description, is_custom)
                 VALUES (?1,?2,?3,?4,?5,?6,1)",
                params![name, category, eq, lt, muscles, description],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(
                    id,
                    name.clone(),
                    format!("exercise created: {name}"),
                ));
            } else {
                quiet_print(quiet, format!("Exercise {id}: {name}"));
            }
        }
        ExerciseAction::Update {
            exercise,
            category,
            equipment,
            clear_equipment,
            load_type,
            muscles,
            description,
        } => {
            let ex = resolve_exercise(&conn, &exercise)?;
            let mut sets = vec![];
            let mut vals: Vec<Box<dyn rusqlite::ToSql>> = vec![];
            if let Some(v) = category {
                sets.push("category = ?");
                vals.push(Box::new(v));
            }
            if clear_equipment {
                sets.push("equipment = NULL");
            } else if let Some(v) = equipment {
                sets.push("equipment = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = load_type {
                let lt = load_type::normalize_load_type(&v).map_err(|e| anyhow!("{e}"))?;
                sets.push("load_type = ?");
                vals.push(Box::new(lt.to_string()));
            }
            if let Some(v) = muscles {
                sets.push("muscle_groups = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = description {
                sets.push("description = ?");
                vals.push(Box::new(v));
            }
            if sets.is_empty() {
                return Err(anyhow!("provide at least one field to update"));
            }
            let sql = format!("UPDATE exercises SET {} WHERE id = ?", sets.join(", "));
            vals.push(Box::new(ex.id));
            let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, refs.as_slice())?;
            if json {
                print_json(&Success::created(ex.id, ex.name, "exercise updated"));
            } else {
                quiet_print(quiet, format!("Updated exercise {}", ex.id));
            }
        }
        ExerciseAction::Search { term } => {
            let mut stmt = conn.prepare(
                "SELECT id, name, category, load_type FROM exercises WHERE name LIKE ?1 ORDER BY name LIMIT 50",
            )?;
            let rows: Vec<_> = stmt
                .query_map([format!("%{term}%")], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "category": r.get::<_, String>(2)?,
                        "load_type": r.get::<_, String>(3)?,
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

fn normalize_exercise_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn resolve_exercise(conn: &Connection, exercise: &str) -> Result<Exercise> {
    if let Ok(id) = exercise.parse::<i64>() {
        return conn
            .query_row(
                "SELECT id, name, category, muscle_groups, equipment, load_type, description, is_custom, created_at
                 FROM exercises WHERE id = ?1",
                [id],
                row_to_exercise,
            )
            .map_err(|_| anyhow!("exercise not found: {exercise}"));
    }
    let name = normalize_exercise_name(exercise);
    conn.query_row(
        "SELECT id, name, category, muscle_groups, equipment, load_type, description, is_custom, created_at
         FROM exercises WHERE name = ?1 COLLATE NOCASE",
        [&name],
        row_to_exercise,
    )
    .map_err(|_| anyhow!("exercise not found: {exercise}"))
}

fn row_to_exercise(r: &rusqlite::Row<'_>) -> rusqlite::Result<Exercise> {
    Ok(Exercise {
        id: r.get(0)?,
        name: r.get(1)?,
        category: r.get(2)?,
        muscle_groups: r.get(3)?,
        equipment: r.get(4)?,
        load_type: r.get(5)?,
        description: r.get(6)?,
        is_custom: r.get(7)?,
        created_at: r.get(8)?,
    })
}

fn resolve_we_id(
    conn: &Connection,
    workout: Option<i64>,
    exercise: Option<&str>,
    workout_exercise: Option<i64>,
) -> Result<(i64, Exercise)> {
    if let Some(we_id) = workout_exercise {
        let (ex_id,): (i64,) = conn
            .query_row(
                "SELECT exercise_id FROM workout_exercises WHERE id = ?1",
                [we_id],
                |r| Ok((r.get(0)?,)),
            )
            .map_err(|_| anyhow!("workout_exercise {we_id} not found"))?;
        let ex = resolve_exercise(conn, &ex_id.to_string())?;
        return Ok((we_id, ex));
    }
    let workout = workout
        .ok_or_else(|| anyhow!("provide --workout and --exercise, or --workout-exercise"))?;
    let exercise = exercise
        .ok_or_else(|| anyhow!("provide --workout and --exercise, or --workout-exercise"))?;
    let ex = resolve_exercise(conn, exercise)?;
    let _: i64 = conn
        .query_row("SELECT id FROM workouts WHERE id = ?1", [workout], |r| {
            r.get(0)
        })
        .map_err(|_| anyhow!("workout not found: {workout}"))?;
    let we_id = match conn
        .query_row(
            "SELECT id FROM workout_exercises WHERE workout_id = ?1 AND exercise_id = ?2 LIMIT 1",
            params![workout, ex.id],
            |r| r.get(0),
        )
        .optional()?
    {
        Some(id) => id,
        None => {
            let order: i64 = conn
                .query_row(
                    r#"SELECT COALESCE(MAX("order"), 0) + 1 FROM workout_exercises WHERE workout_id = ?1"#,
                    [workout],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            conn.execute(
                r#"INSERT INTO workout_exercises (workout_id, exercise_id, "order") VALUES (?1,?2,?3)"#,
                params![workout, ex.id, order],
            )?;
            conn.last_insert_rowid()
        }
    };
    Ok((we_id, ex))
}

fn next_set_number(conn: &Connection, we_id: i64) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(set_number), 0) + 1 FROM exercise_sets WHERE workout_exercise_id = ?1",
        [we_id],
        |r| r.get(0),
    )
    .unwrap_or(1)
}

fn next_cluster_id(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(cluster_id), 0) + 1 FROM exercise_sets",
        [],
        |r| r.get(0),
    )
    .unwrap_or(1)
}

fn parse_hr_zones(s: Option<&str>) -> Result<Option<HeartRateZones>> {
    match s {
        None => Ok(None),
        Some(raw) => {
            let z: HeartRateZones =
                serde_json::from_str(raw).map_err(|e| anyhow!("invalid --hr-zones JSON: {e}"))?;
            Ok(Some(z))
        }
    }
}

fn parse_laps(s: Option<&str>) -> Result<Option<Laps>> {
    match s {
        None => Ok(None),
        Some(raw) => {
            // Accept either {"laps":[...]} or bare array
            if let Ok(laps) = serde_json::from_str::<Laps>(raw) {
                return Ok(Some(laps));
            }
            if let Ok(v) = serde_json::from_str::<Vec<crate::models::Lap>>(raw) {
                return Ok(Some(Laps(v)));
            }
            Err(anyhow!("invalid --laps JSON"))
        }
    }
}

fn parse_csv_i32(s: &str, label: &str) -> Result<Vec<i32>> {
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<i32>()
                .map_err(|_| anyhow!("invalid {label}: '{p}'"))
        })
        .collect()
}

fn parse_csv_f64(s: &str, label: &str) -> Result<Vec<f64>> {
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<f64>()
                .map_err(|_| anyhow!("invalid {label}: '{p}'"))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn insert_set(
    conn: &Connection,
    we_id: i64,
    set_number: i64,
    reps: Option<i32>,
    weight: Option<f64>,
    external_load: Option<f64>,
    distance: Option<f64>,
    duration: Option<i32>,
    rpe: Option<f64>,
    rir: Option<f64>,
    effective_reps: Option<i32>,
    cluster_id: Option<i64>,
    rest_seconds: Option<i32>,
    notes: Option<&str>,
    side: Option<&str>,
    phase: &str,
    avg_hr: Option<f64>,
    max_hr: Option<f64>,
    pace: Option<f64>,
    calories: Option<i32>,
    hr_zones: Option<&HeartRateZones>,
    laps: Option<&Laps>,
    limits: &WorkoutSanityLimits,
) -> Result<i64> {
    let zones_json = hr_zones
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| anyhow!("{e}"))?;
    let laps_json = laps
        .map(|l| serde_json::to_string(&l.0))
        .transpose()
        .map_err(|e| anyhow!("{e}"))?;

    let proposed = ProposedSetMetrics {
        reps,
        weight_kg: weight,
        external_load_kg: external_load,
        distance_km: distance,
        duration_seconds: duration,
        rpe,
        rir,
        effective_reps,
        rest_seconds,
        avg_heart_rate_bpm: avg_hr,
        max_heart_rate_bpm: max_hr,
        avg_pace_min_per_km: pace,
        calories_burned: calories,
        heart_rate_zones: hr_zones.cloned(),
        laps: laps.map(|l| l.0.clone()),
        ..Default::default()
    };
    sanity::validate_set_metrics(&proposed, limits).map_err(|e| anyhow!("{e}"))?;

    conn.execute(
        "INSERT INTO exercise_sets
         (workout_exercise_id, set_number, reps, weight_kg, external_load_kg,
          distance_km, duration_seconds, rpe, rir, effective_reps, cluster_id, rest_seconds,
          notes, side, phase, avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km,
          calories_burned, heart_rate_zones, laps)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
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
            cluster_id,
            rest_seconds,
            notes,
            side,
            phase,
            avg_hr,
            max_hr,
            pace,
            calories,
            zones_json,
            laps_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn handle_set(
    action: SetAction,
    db_override: Option<&str>,
    limits: &WorkoutSanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        SetAction::Add {
            workout,
            exercise,
            workout_exercise,
            reps,
            weight,
            external_load,
            no_weight_recorded,
            duration,
            distance,
            rpe,
            rir,
            effective_reps,
            rest_seconds,
            notes,
            side,
            phase,
            avg_heart_rate,
            max_heart_rate,
            hr_zones,
            pace,
            calories,
            laps,
        } => {
            let conn = db::open_db(db_override)?;
            let (we_id, ex) = resolve_we_id(&conn, workout, exercise.as_deref(), workout_exercise)?;
            let resolved_phase = phase::normalize_phase(&phase).map_err(|e| anyhow!("{e}"))?;
            let requires =
                reps.is_some() || weight.is_some() || duration.is_some() || external_load.is_some();
            let (w, el) = bodyweight::resolve_bodyweight_load(
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                requires,
            )
            .map_err(|e| anyhow!("{e}"))?;
            if reps.is_none()
                && w.is_none()
                && duration.is_none()
                && distance.is_none()
                && el.is_none()
                && avg_heart_rate.is_none()
            {
                return Err(anyhow!(
                    "provide at least one metric (reps, weight, duration, distance, external-load, or heart rate)"
                ));
            }
            let zones = parse_hr_zones(hr_zones.as_deref())?;
            let laps_v = parse_laps(laps.as_deref())?;
            let sn = next_set_number(&conn, we_id);
            let id = insert_set(
                &conn,
                we_id,
                sn,
                reps,
                w,
                el,
                distance,
                duration,
                rpe,
                rir,
                effective_reps,
                None,
                rest_seconds,
                notes.as_deref(),
                side.as_deref(),
                resolved_phase,
                avg_heart_rate,
                max_heart_rate,
                pace,
                calories,
                zones.as_ref(),
                laps_v.as_ref(),
                limits,
            )?;
            if json {
                print_json(&Success::created(id, format!("set {sn}"), "set added"));
            } else {
                quiet_print(quiet, format!("Added set {sn} (id {id})"));
            }
            Ok(())
        }
        SetAction::AddCardio {
            workout,
            exercise,
            workout_exercise,
            distance,
            duration,
            avg_heart_rate,
            max_heart_rate,
            pace,
            calories,
            hr_zones,
            laps,
            notes,
            phase,
        } => {
            let conn = db::open_db(db_override)?;
            let (we_id, _) = resolve_we_id(&conn, workout, exercise.as_deref(), workout_exercise)?;
            let resolved_phase = phase::normalize_phase(&phase).map_err(|e| anyhow!("{e}"))?;
            let zones = parse_hr_zones(hr_zones.as_deref())?.unwrap_or_default();
            let laps_v = parse_laps(laps.as_deref())?;
            let sn = next_set_number(&conn, we_id);
            let id = insert_set(
                &conn,
                we_id,
                sn,
                None,
                None,
                None,
                Some(distance),
                Some(duration),
                None,
                None,
                None,
                None,
                None,
                notes.as_deref(),
                None,
                resolved_phase,
                Some(avg_heart_rate),
                Some(max_heart_rate),
                Some(pace),
                Some(calories),
                Some(&zones),
                laps_v.as_ref(),
                limits,
            )?;
            if json {
                print_json(&Success::created(
                    id,
                    format!("set {sn}"),
                    "cardio set added",
                ));
            } else {
                quiet_print(quiet, format!("Added cardio set {sn} (id {id})"));
            }
            Ok(())
        }
        SetAction::AddCluster {
            workout,
            exercise,
            workout_exercise,
            weight,
            external_load,
            no_weight_recorded,
            reps,
            rir,
            effective_reps,
            rest_seconds,
            notes,
            side,
            phase,
        } => {
            let conn = db::open_db(db_override)?;
            let (we_id, ex) = resolve_we_id(&conn, workout, exercise.as_deref(), workout_exercise)?;
            let resolved_phase = phase::normalize_phase(&phase).map_err(|e| anyhow!("{e}"))?;
            let reps_list = parse_csv_i32(&reps, "reps")?;
            let rir_list = parse_csv_f64(&rir, "rir")?;
            let eff_list = parse_csv_i32(&effective_reps, "effective-reps")?;
            if reps_list.len() != rir_list.len() || reps_list.len() != eff_list.len() {
                return Err(anyhow!(
                    "reps, rir, and effective-reps must have the same number of values"
                ));
            }
            let (w, el) = bodyweight::resolve_bodyweight_load(
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                true,
            )
            .map_err(|e| anyhow!("{e}"))?;
            let cluster_id = next_cluster_id(&conn);
            let mut ids = vec![];
            for (i, ((r, ri), eff)) in reps_list
                .into_iter()
                .zip(rir_list)
                .zip(eff_list)
                .enumerate()
            {
                let rest = if i > 0 { Some(rest_seconds) } else { None };
                let sn = next_set_number(&conn, we_id);
                let id = insert_set(
                    &conn,
                    we_id,
                    sn,
                    Some(r),
                    w,
                    el,
                    None,
                    None,
                    None,
                    Some(ri),
                    Some(eff),
                    Some(cluster_id),
                    rest,
                    notes.as_deref(),
                    side.as_deref(),
                    resolved_phase,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    limits,
                )?;
                ids.push(id);
            }
            if json {
                print_json(&serde_json::json!({
                    "success": true,
                    "cluster_id": cluster_id,
                    "set_ids": ids,
                    "message": "cluster added"
                }));
            } else {
                quiet_print(
                    quiet,
                    format!("Added cluster {cluster_id} with sets {ids:?}"),
                );
            }
            Ok(())
        }
        SetAction::AddUnilateral {
            workout,
            exercise,
            workout_exercise,
            reps,
            weight,
            external_load,
            no_weight_recorded,
            rir,
            effective_reps,
            rest_seconds,
            notes,
            side,
            phase,
        } => {
            let conn = db::open_db(db_override)?;
            let (we_id, ex) = resolve_we_id(&conn, workout, exercise.as_deref(), workout_exercise)?;
            let resolved_phase = phase::normalize_phase(&phase).map_err(|e| anyhow!("{e}"))?;
            let reps_list = parse_csv_i32(&reps, "reps")?;
            let rir_list = match rir {
                Some(s) => parse_csv_f64(&s, "rir")?,
                None => vec![0.0; reps_list.len()],
            };
            let eff_list = match effective_reps {
                Some(s) => parse_csv_i32(&s, "effective-reps")?,
                None => reps_list.clone(),
            };
            if rir_list.len() != reps_list.len() || eff_list.len() != reps_list.len() {
                return Err(anyhow!("rir/effective-reps length must match reps"));
            }
            let (w, el) = bodyweight::resolve_bodyweight_load(
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                true,
            )
            .map_err(|e| anyhow!("{e}"))?;
            let sides: Vec<&str> = match side.as_str() {
                "both" => vec!["left", "right"],
                s => vec![s],
            };
            let mut ids = vec![];
            for (i, ((r, ri), eff)) in reps_list
                .into_iter()
                .zip(rir_list)
                .zip(eff_list)
                .enumerate()
            {
                for sd in &sides {
                    let rest = if i > 0 { rest_seconds } else { None };
                    let sn = next_set_number(&conn, we_id);
                    let id = insert_set(
                        &conn,
                        we_id,
                        sn,
                        Some(r),
                        w,
                        el,
                        None,
                        None,
                        None,
                        Some(ri),
                        Some(eff),
                        None,
                        rest,
                        notes.as_deref(),
                        Some(sd),
                        resolved_phase,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        limits,
                    )?;
                    ids.push(id);
                }
            }
            if json {
                print_json(&serde_json::json!({
                    "success": true,
                    "set_ids": ids,
                    "message": "unilateral sets added"
                }));
            } else {
                quiet_print(quiet, format!("Added unilateral sets {ids:?}"));
            }
            Ok(())
        }
        SetAction::List { workout_exercise } => {
            let conn = db::open_db(db_override)?;
            let activity_date: String = conn
                .query_row(
                    r#"SELECT w.started_at FROM workouts w
                       JOIN workout_exercises we ON we.workout_id = w.id
                       WHERE we.id = ?1"#,
                    [workout_exercise],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .map(|s| activity_date_prefix(&s))
                .unwrap_or_default();
            let sets = list_sets_json(&conn, workout_exercise, &activity_date)?;
            if json {
                print_json(&sets);
            } else {
                for s in &sets {
                    println!(
                        "{}: set {} reps={:?} weight={:?}",
                        s["id"], s["set_number"], s["reps"], s["weight_kg"]
                    );
                    if let Some(tm_val) = s.get("track_metrics") {
                        if let Ok(tm) = serde_json::from_value::<TrackMetrics>(tm_val.clone()) {
                            print_track_metrics_oneline(&tm);
                        }
                    }
                }
            }
            Ok(())
        }
        SetAction::Quick {
            workout,
            exercise,
            reps,
            weight,
            external_load,
            no_weight_recorded,
            duration,
            notes,
            phase,
        } => {
            let conn = db::open_db(db_override)?;
            let (we_id, ex) = resolve_we_id(&conn, Some(workout), Some(&exercise), None)?;
            if reps.is_none() && weight.is_none() && duration.is_none() {
                if json {
                    print_json(&serde_json::json!({
                        "success": true,
                        "workout_exercise_id": we_id,
                        "message": "exercise added to workout (no set)"
                    }));
                } else {
                    quiet_print(quiet, format!("Added exercise to workout (we_id={we_id})"));
                }
                return Ok(());
            }
            let ph = phase
                .as_deref()
                .map(phase::normalize_phase)
                .transpose()
                .map_err(|e| anyhow!("{e}"))?
                .unwrap_or(phase::FULL);
            let (w, el) = bodyweight::resolve_bodyweight_load(
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                true,
            )
            .map_err(|e| anyhow!("{e}"))?;
            let sn = next_set_number(&conn, we_id);
            let id = insert_set(
                &conn,
                we_id,
                sn,
                reps,
                w,
                el,
                None,
                duration,
                None,
                None,
                None,
                None,
                None,
                notes.as_deref(),
                None,
                ph,
                None,
                None,
                None,
                None,
                None,
                None,
                limits,
            )?;
            if json {
                print_json(&serde_json::json!({
                    "success": true,
                    "workout_exercise_id": we_id,
                    "set_id": id,
                    "message": "quick set added"
                }));
            } else {
                quiet_print(quiet, format!("Quick: we_id={we_id} set_id={id}"));
            }
            Ok(())
        }
        SetAction::Update {
            id,
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
            avg_heart_rate,
            max_heart_rate,
            pace,
            calories,
        } => {
            let conn = db::open_db(db_override)?;
            let exists: Option<i64> = conn
                .query_row("SELECT id FROM exercise_sets WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(anyhow!("set {id} not found"));
            }
            let resolved_phase = phase
                .as_ref()
                .map(|p| phase::normalize_phase(p))
                .transpose()
                .map_err(|e| anyhow!("{e}"))?;
            let proposed = ProposedSetMetrics {
                reps,
                weight_kg: weight,
                external_load_kg: external_load,
                distance_km: distance,
                duration_seconds: duration,
                rpe,
                rir,
                effective_reps,
                rest_seconds,
                avg_heart_rate_bpm: avg_heart_rate,
                max_heart_rate_bpm: max_heart_rate,
                avg_pace_min_per_km: pace,
                calories_burned: calories,
                ..Default::default()
            };
            // Only validate fields that were provided (zeros of absent are skipped in checks)
            sanity::validate_set_metrics(&proposed, limits).map_err(|e| anyhow!("{e}"))?;

            let mut sets = vec![];
            let mut vals: Vec<Box<dyn rusqlite::ToSql>> = vec![];
            macro_rules! push_opt {
                ($field:expr, $col:expr) => {
                    if let Some(v) = $field {
                        sets.push(concat!($col, " = ?"));
                        vals.push(Box::new(v));
                    }
                };
            }
            push_opt!(reps, "reps");
            push_opt!(weight, "weight_kg");
            push_opt!(external_load, "external_load_kg");
            push_opt!(duration, "duration_seconds");
            push_opt!(distance, "distance_km");
            push_opt!(rpe, "rpe");
            push_opt!(rir, "rir");
            push_opt!(effective_reps, "effective_reps");
            push_opt!(rest_seconds, "rest_seconds");
            if let Some(v) = notes {
                sets.push("notes = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = side {
                sets.push("side = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = resolved_phase {
                sets.push("phase = ?");
                vals.push(Box::new(v.to_string()));
            }
            push_opt!(avg_heart_rate, "avg_heart_rate_bpm");
            push_opt!(max_heart_rate, "max_heart_rate_bpm");
            push_opt!(pace, "avg_pace_min_per_km");
            push_opt!(calories, "calories_burned");
            if sets.is_empty() {
                return Err(anyhow!("provide at least one field to update"));
            }
            let sql = format!("UPDATE exercise_sets SET {} WHERE id = ?", sets.join(", "));
            vals.push(Box::new(id));
            let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, refs.as_slice())?;
            if json {
                print_json(&Success::created(id, "updated", "set updated"));
            } else {
                quiet_print(quiet, format!("Updated set {id}"));
            }
            Ok(())
        }
        SetAction::Move { id, to } => {
            if to < 1 {
                return Err(anyhow!("--to must be >= 1"));
            }
            let conn = db::open_db(db_override)?;
            let (we_id, old_num): (i64, i64) = conn
                .query_row(
                    "SELECT workout_exercise_id, set_number FROM exercise_sets WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| anyhow!("set {id} not found"))?;
            let max_n: i64 = conn.query_row(
                "SELECT COALESCE(MAX(set_number), 0) FROM exercise_sets WHERE workout_exercise_id = ?1",
                [we_id],
                |r| r.get(0),
            )?;
            let target = to.min(max_n as i32) as i64;
            if target == old_num {
                if json {
                    print_json(&Success::ok("already at position"));
                }
                return Ok(());
            }
            // Temporary set_number swap using negative range
            conn.execute(
                "UPDATE exercise_sets SET set_number = -set_number WHERE workout_exercise_id = ?1",
                [we_id],
            )?;
            // Remap in order
            let mut stmt = conn.prepare(
                "SELECT id, -set_number as sn FROM exercise_sets WHERE workout_exercise_id = ?1 ORDER BY sn",
            )?;
            let mut order: Vec<(i64, i64)> = stmt
                .query_map([we_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            // remove moving id and reinsert at target-1
            if let Some(pos) = order.iter().position(|(sid, _)| *sid == id) {
                let item = order.remove(pos);
                let insert_at = (target as usize - 1).min(order.len());
                order.insert(insert_at, item);
            }
            for (i, (sid, _)) in order.iter().enumerate() {
                conn.execute(
                    "UPDATE exercise_sets SET set_number = ?1 WHERE id = ?2",
                    params![(i as i64) + 1, sid],
                )?;
            }
            if json {
                print_json(&Success::created(id, format!("pos {target}"), "set moved"));
            } else {
                quiet_print(quiet, format!("Moved set {id} to position {target}"));
            }
            Ok(())
        }
        SetAction::Delete { id } => {
            let conn = db::open_db(db_override)?;
            let n = conn.execute("DELETE FROM exercise_sets WHERE id = ?1", [id])?;
            if n == 0 {
                return Err(anyhow!("set {id} not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted set {id}"));
            }
            Ok(())
        }
    }
}

/// Seed default exercises (idempotent).
pub fn seed_default_exercises(conn: &Connection) -> Result<Vec<String>> {
    use crate::load_type::{BODY_MASS, EXTERNAL, NONE};
    #[allow(clippy::type_complexity)]
    let defaults: Vec<(&str, &str, Option<&str>, Option<&str>, &str, Option<&str>)> = vec![
        (
            "pushups",
            "calisthenics",
            Some("[\"chest\", \"triceps\"]"),
            None,
            BODY_MASS,
            Some("Basic pushup"),
        ),
        (
            "pullups",
            "calisthenics",
            Some("[\"back\", \"biceps\"]"),
            None,
            BODY_MASS,
            Some("Basic pullup"),
        ),
        (
            "dips",
            "calisthenics",
            Some("[\"chest\", \"triceps\"]"),
            None,
            BODY_MASS,
            Some("Basic dip"),
        ),
        (
            "squats",
            "calisthenics",
            Some("[\"legs\"]"),
            None,
            BODY_MASS,
            Some("Bodyweight squat"),
        ),
        (
            "lunges",
            "calisthenics",
            Some("[\"legs\"]"),
            None,
            BODY_MASS,
            Some("Lunges"),
        ),
        (
            "plank",
            "flexibility",
            Some("[\"core\"]"),
            None,
            BODY_MASS,
            Some("Timed plank"),
        ),
        (
            "bench press",
            "strength",
            Some("[\"chest\", \"triceps\"]"),
            Some("barbell"),
            EXTERNAL,
            Some("Bench press"),
        ),
        (
            "deadlift",
            "strength",
            Some("[\"back\", \"legs\"]"),
            Some("barbell"),
            EXTERNAL,
            Some("Deadlift"),
        ),
        (
            "squat (barbell)",
            "strength",
            Some("[\"legs\"]"),
            Some("barbell"),
            EXTERNAL,
            Some("Back squat"),
        ),
        (
            "running",
            "cardio",
            Some("[\"legs\"]"),
            Some("none"),
            NONE,
            Some("Run"),
        ),
    ];
    let mut added = vec![];
    for (name, cat, muscles, eq, lt, desc) in defaults {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM exercises WHERE name = ?1 COLLATE NOCASE",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            conn.execute(
                "INSERT INTO exercises (name, category, muscle_groups, equipment, load_type, description, is_custom)
                 VALUES (?1,?2,?3,?4,?5,?6,0)",
                params![name, cat, muscles, eq, lt, desc],
            )?;
            added.push(name.to_string());
        }
    }
    Ok(added)
}
