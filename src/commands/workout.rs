//! Workout / exercise / set handlers (repslog parity under grouped CLI).

use crate::bodyweight;
use crate::cli::{ExerciseAction, SetAction, WorkoutAction, WorkoutStatsAction};
use crate::commands::workout_stats;
use crate::config::WorkoutSanityLimits;
use crate::db;
use crate::entity_audit::{self, CascadeCounts};
use crate::load_type;
use crate::models::{Exercise, HeartRateZones, Laps, Success, Trackpoint};
use crate::phase;
use crate::sanity::{self, ProposedSetMetrics};
use crate::set_order;
use crate::track_metrics::{
    compute, compute_with_zones, RouteKind, TrackMetrics, ZoneRecomputeContext,
};
use crate::utils::{
    format_duration, format_hr_zones_bar, format_pace, parse_rfc3339_instant_for_db,
    print_error_json, print_json, print_table, quiet_print,
};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};

fn emit_dry_run(json: bool, quiet: bool, would: serde_json::Value) -> Result<()> {
    if json {
        print_json(&serde_json::json!({
            "success": true,
            "dry_run": true,
            "would": would,
        }));
    } else {
        quiet_print(quiet, format!("dry-run: would {would}"));
    }
    Ok(())
}

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
            finished_at,
            workout_type,
            notes,
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            let started = match started_at {
                Some(s) => parse_rfc3339_instant_for_db(&s)?,
                None => db::now_utc(),
            };
            let finished = finished_at
                .as_ref()
                .map(|s| parse_rfc3339_instant_for_db(s))
                .transpose()?;
            let created = db::now_utc();
            if dry_run {
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "workout create",
                        "started_at": started,
                        "finished_at": finished,
                        "workout_type": workout_type,
                        "notes": notes,
                    }),
                );
            }
            conn.execute(
                "INSERT INTO workouts (started_at, finished_at, workout_type, notes, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![started, finished, workout_type, notes, created],
            )?;
            let id = conn.last_insert_rowid();
            entity_audit::append_create(&conn, entity_audit::entity::WORKOUT, id, None)?;
            if json {
                print_json(&Success::created_workout(
                    id,
                    started.clone(),
                    finished.clone(),
                    created,
                    "workout created",
                ));
            } else {
                quiet_print(
                    quiet,
                    format!("Created workout {id} (started {started}, stored {created})"),
                );
            }
            Ok(())
        }
        WorkoutAction::List { days, limit } => {
            let conn = db::open_db(db_override)?;
            let lim = limit.max(1);
            let mut sql = String::from(
                "SELECT id, started_at, finished_at, workout_type, notes, duration_minutes, \
                 overall_feeling, created_at
                 FROM workouts WHERE deleted_at IS NULL",
            );
            let mut binds: Vec<String> = vec![];
            if let Some(d) = days {
                sql.push_str(" AND date(started_at, 'localtime') >= date('now', 'localtime', ?)");
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
                        "finished_at": r.get::<_, Option<String>>(2)?,
                        "workout_type": r.get::<_, Option<String>>(3)?,
                        "notes": r.get::<_, Option<String>>(4)?,
                        "duration_minutes": r.get::<_, Option<i64>>(5)?,
                        "overall_feeling": r.get::<_, Option<i64>>(6)?,
                        "created_at": r.get::<_, String>(7)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else if rows.is_empty() {
                println!("(no workouts)");
            } else {
                let mut table_rows = Vec::new();
                for w in &rows {
                    let id = w["id"].as_i64().unwrap_or(0);
                    let summary = workout_list_summary(
                        &conn,
                        id,
                        w["workout_type"].as_str(),
                        w["notes"].as_str(),
                    )?;
                    table_rows.push(vec![
                        id.to_string(),
                        w["started_at"].as_str().unwrap_or("").to_string(),
                        w["workout_type"].as_str().unwrap_or("").to_string(),
                        w["duration_minutes"]
                            .as_i64()
                            .map(|d| d.to_string())
                            .unwrap_or_default(),
                        summary,
                    ]);
                }
                print_table(
                    vec!["ID", "Started At", "Type", "Dur", "Summary"],
                    table_rows,
                );
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
            finished_at,
            reason,
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            // before-image for audit field diffs
            type WorkoutBefore = (
                Option<String>, // deleted_at
                Option<String>, // workout_type
                Option<String>, // notes
                Option<i64>,    // duration_minutes
                Option<i64>,    // overall_feeling
                String,         // started_at
                Option<String>, // finished_at
            );
            let before: Option<WorkoutBefore> = conn
                .query_row(
                    "SELECT deleted_at, workout_type, notes, duration_minutes, overall_feeling, \
                     started_at, finished_at FROM workouts WHERE id=?1",
                    [id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .optional()?;
            let (old_type, old_notes, old_duration, old_feeling, old_started, old_finished) =
                match before {
                    None => return Err(anyhow!("workout {id} not found")),
                    Some((Some(_), ..)) => {
                        return Err(anyhow!(
                            "workout {id} is soft-deleted (restore not implemented; use audit to inspect)"
                        ));
                    }
                    Some((None, t, n, d, f, s, fin)) => (t, n, d, f, s, fin),
                };
            if let Some(f) = feeling {
                if !(1..=5).contains(&f) {
                    return Err(anyhow!("feeling must be 1-5"));
                }
            }
            let started = started_at
                .as_ref()
                .map(|s| parse_rfc3339_instant_for_db(s))
                .transpose()?;
            let finished = finished_at
                .as_ref()
                .map(|s| parse_rfc3339_instant_for_db(s))
                .transpose()?;
            // Dynamic partial update
            let mut sets = vec![];
            let mut vals: Vec<Box<dyn rusqlite::ToSql>> = vec![];
            let mut changes: Vec<entity_audit::FieldChange> = Vec::new();
            if let Some(v) = workout_type {
                changes.push(entity_audit::FieldChange::new(
                    "workout_type",
                    opt_str_json(old_type.as_deref()),
                    serde_json::json!(v),
                ));
                sets.push("workout_type = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = notes {
                changes.push(entity_audit::FieldChange::new(
                    "notes",
                    opt_str_json(old_notes.as_deref()),
                    serde_json::json!(v),
                ));
                sets.push("notes = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = duration {
                changes.push(entity_audit::FieldChange::new(
                    "duration_minutes",
                    opt_i64_json(old_duration),
                    serde_json::json!(v),
                ));
                sets.push("duration_minutes = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = feeling {
                changes.push(entity_audit::FieldChange::new(
                    "overall_feeling",
                    opt_i64_json(old_feeling),
                    serde_json::json!(v),
                ));
                sets.push("overall_feeling = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = started {
                changes.push(entity_audit::FieldChange::new(
                    "started_at",
                    serde_json::json!(old_started),
                    serde_json::json!(v),
                ));
                sets.push("started_at = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = finished {
                changes.push(entity_audit::FieldChange::new(
                    "finished_at",
                    opt_str_json(old_finished.as_deref()),
                    serde_json::json!(v),
                ));
                sets.push("finished_at = ?");
                vals.push(Box::new(v));
            }
            if sets.is_empty() {
                return Err(anyhow!("provide at least one field to update"));
            }
            let class = entity_audit::classify_field_changes(&changes);
            if dry_run {
                let reason_preview = reason.as_deref().map(str::trim).filter(|s| !s.is_empty());
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "workout update",
                        "id": id,
                        "fields": sets,
                        "kind": class.as_str(),
                        "reason_required": class == entity_audit::UpdateClass::Correction,
                        "reason": reason_preview,
                    }),
                );
            }
            let reason_stored = entity_audit::require_reason_for_class(class, reason.as_deref())?;
            let sql = format!("UPDATE workouts SET {} WHERE id = ?", sets.join(", "));
            vals.push(Box::new(id));
            let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            let write_op = crate::append_guard::op_for_update_class(class);
            crate::append_guard::with_write_allow(&conn, write_op, |conn| {
                conn.execute(&sql, refs.as_slice())?;
                Ok(())
            })?;
            entity_audit::append_field_change(
                &conn,
                entity_audit::entity::WORKOUT,
                id,
                &changes,
                class,
                reason_stored.as_deref(),
                None,
            )?;
            if json {
                print_json(&Success::updated(
                    id,
                    class.as_str(),
                    reason_stored,
                    "workout updated",
                ));
            } else {
                quiet_print(quiet, format!("Updated workout {id} ({})", class.as_str()));
            }
            Ok(())
        }
        WorkoutAction::Correct {
            id,
            workout_type,
            notes,
            duration,
            feeling,
            started_at,
            finished_at,
            reason,
            dry_run,
        } => handle_workout_correct(
            db_override,
            id,
            workout_type,
            notes,
            duration,
            feeling,
            started_at,
            finished_at,
            reason,
            dry_run,
            json,
            quiet,
        ),
        WorkoutAction::Delete {
            id,
            reason,
            purge,
            force,
            dry_run,
        } => handle_workout_delete(db_override, id, reason, purge, force, dry_run, json, quiet),
        WorkoutAction::Audit { id, limit } => handle_workout_audit(db_override, id, limit, json),
        WorkoutAction::Stats { action, days } => {
            let conn = db::open_db(db_override)?;
            match action {
                None => workout_stats::handle_volume(&conn, None, None, Some(days), json),
                Some(WorkoutStatsAction::Prs { exercise }) => {
                    workout_stats::handle_prs(&conn, exercise.as_deref(), json)
                }
                Some(WorkoutStatsAction::Volume {
                    exercise,
                    period,
                    days: vol_days,
                }) => workout_stats::handle_volume(
                    &conn,
                    exercise.as_deref(),
                    period.as_deref(),
                    vol_days,
                    json,
                ),
                Some(WorkoutStatsAction::Summary { days }) => {
                    workout_stats::handle_summary(&conn, days, json)
                }
                Some(WorkoutStatsAction::History { exercise, days }) => {
                    workout_stats::handle_history(&conn, &exercise, days, json)
                }
                Some(WorkoutStatsAction::Weight { exercise }) => {
                    workout_stats::handle_weight(&conn, &exercise, json)
                }
            }
        }
        WorkoutAction::Exercise { action } => handle_exercise(action, db_override, json, quiet),
        WorkoutAction::Set { action } => {
            handle_set(*action, db_override, workout_limits, json, quiet)
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

fn is_cardio_set(s: &serde_json::Value) -> bool {
    s["distance_km"].as_f64().is_some()
        || s["duration_seconds"].as_i64().is_some()
        || s["avg_heart_rate_bpm"].as_f64().is_some()
}

/// One-line list summary: cardio stats when present, else workout notes (repslog parity).
fn workout_list_summary(
    conn: &Connection,
    workout_id: i64,
    workout_type: Option<&str>,
    notes: Option<&str>,
) -> Result<String> {
    let mut stmt = conn.prepare(
        r#"SELECT s.distance_km, s.duration_seconds, s.avg_heart_rate_bpm
           FROM exercise_sets s
           JOIN workout_exercises we ON we.id = s.workout_exercise_id
           WHERE we.workout_id = ?1 AND s.deleted_at IS NULL"#,
    )?;
    let mut total_distance = 0.0f64;
    let mut total_duration: u32 = 0;
    let mut hr_samples: Vec<f64> = Vec::new();
    let mut cardio_found = false;
    let mut rows = stmt.query([workout_id])?;
    while let Some(r) = rows.next()? {
        let dist: Option<f64> = r.get(0)?;
        let dur: Option<i64> = r.get(1)?;
        let hr: Option<f64> = r.get(2)?;
        if let Some(d) = dist {
            total_distance += d;
            cardio_found = true;
        }
        if let Some(d) = dur {
            total_duration = total_duration.saturating_add(d.max(0) as u32);
            cardio_found = true;
        }
        if let Some(h) = hr {
            hr_samples.push(h);
            cardio_found = true;
        }
    }

    if cardio_found {
        let pace = if total_distance > 0.0 {
            format_pace((total_duration as f64 / 60.0) / total_distance)
        } else {
            "--".to_string()
        };
        let avg_hr = if hr_samples.is_empty() {
            "--".to_string()
        } else {
            format!(
                "{:.0}",
                hr_samples.iter().sum::<f64>() / hr_samples.len() as f64
            )
        };
        Ok(format!(
            "{} • {:.2} km • {} • {} • {} bpm",
            workout_type.unwrap_or("Run"),
            total_distance,
            format_duration(total_duration),
            pace,
            avg_hr
        ))
    } else {
        Ok(notes.unwrap_or("").to_string())
    }
}

fn zones_total_seconds(z: &HeartRateZones) -> u32 {
    z.z1_seconds + z.z2_seconds + z.z3_seconds + z.z4_seconds + z.z5_seconds
}

fn parse_zones_value(v: &serde_json::Value) -> Option<HeartRateZones> {
    if v.is_null() {
        return None;
    }
    serde_json::from_value(v.clone()).ok()
}

fn parse_laps_value(v: &serde_json::Value) -> Vec<crate::models::Lap> {
    if v.is_null() {
        return Vec::new();
    }
    if let Ok(laps) = serde_json::from_value::<Laps>(v.clone()) {
        return laps.0;
    }
    if let Ok(laps) = serde_json::from_value::<Vec<crate::models::Lap>>(v.clone()) {
        return laps;
    }
    Vec::new()
}

/// Aggregate cardio fields across all cardio sets (repslog `cardio_summary` parity).
fn build_cardio_summary(exercises: &[serde_json::Value]) -> Option<serde_json::Value> {
    let mut total_dist = 0.0f64;
    let mut total_dur: u32 = 0;
    let mut total_cals: i32 = 0;
    let mut hr_samples: Vec<f64> = Vec::new();
    let mut max_hr = 0.0f64;
    let mut aggregated_zones = HeartRateZones::default();
    let mut cadence_samples: Vec<f64> = Vec::new();
    let mut ascent = 0.0f64;
    let mut descent = 0.0f64;
    let mut laps_all: Vec<crate::models::Lap> = Vec::new();
    let mut primary_track: Option<serde_json::Value> = None;
    let mut any = false;

    for ex in exercises {
        let empty = vec![];
        for s in ex["sets"].as_array().unwrap_or(&empty) {
            if !is_cardio_set(s) {
                continue;
            }
            any = true;
            total_dist += s["distance_km"].as_f64().unwrap_or(0.0);
            total_dur += s["duration_seconds"].as_i64().unwrap_or(0).max(0) as u32;
            total_cals += s["calories_burned"].as_i64().unwrap_or(0) as i32;
            if let Some(hr) = s["avg_heart_rate_bpm"].as_f64() {
                hr_samples.push(hr);
            }
            if let Some(hr) = s["max_heart_rate_bpm"].as_f64() {
                if hr > max_hr {
                    max_hr = hr;
                }
            }
            if let Some(z) = parse_zones_value(&s["heart_rate_zones"]) {
                aggregated_zones.z1_seconds += z.z1_seconds;
                aggregated_zones.z2_seconds += z.z2_seconds;
                aggregated_zones.z3_seconds += z.z3_seconds;
                aggregated_zones.z4_seconds += z.z4_seconds;
                aggregated_zones.z5_seconds += z.z5_seconds;
            }
            if let Some(c) = s["avg_cadence_spm"].as_f64() {
                cadence_samples.push(c);
            }
            ascent += s["total_ascent_m"].as_f64().unwrap_or(0.0);
            descent += s["total_descent_m"].as_f64().unwrap_or(0.0);
            laps_all.extend(parse_laps_value(&s["laps"]));
            if primary_track.is_none() {
                if let Some(tm) = s.get("track_metrics") {
                    if !tm.is_null() {
                        primary_track = Some(tm.clone());
                    }
                }
            }
        }
    }

    if !any {
        return None;
    }

    let avg_hr = if hr_samples.is_empty() {
        None
    } else {
        Some((hr_samples.iter().sum::<f64>() / hr_samples.len() as f64).round())
    };
    let avg_pace = if total_dist > 0.0 {
        Some((total_dur as f64 / 60.0) / total_dist)
    } else {
        None
    };
    let avg_cadence = if cadence_samples.is_empty() {
        None
    } else {
        Some(cadence_samples.iter().sum::<f64>() / cadence_samples.len() as f64)
    };
    let zones_json = if zones_total_seconds(&aggregated_zones) > 0 {
        Some(aggregated_zones)
    } else {
        None
    };

    Some(serde_json::json!({
        "total_distance_km": total_dist,
        "total_duration_seconds": total_dur,
        "avg_pace_min_per_km": avg_pace,
        "avg_heart_rate_bpm": avg_hr,
        "max_heart_rate_bpm": if max_hr > 0.0 { Some(max_hr.round()) } else { None },
        "total_calories": total_cals,
        "avg_cadence_spm": avg_cadence,
        "total_ascent_m": if ascent > 0.0 { Some(ascent) } else { None },
        "total_descent_m": if descent > 0.0 { Some(descent) } else { None },
        "hr_zones": zones_json,
        "laps": if laps_all.is_empty() { None } else { Some(laps_all) },
        "track": primary_track,
    }))
}

fn print_track_metrics(
    m: &TrackMetrics,
    device_distance_km: Option<f64>,
    stored_zones_empty: bool,
    show_synthetic_splits: bool,
) {
    println!("\nTRACK METRICS");
    println!("  Samples  {}", m.sample_count);

    let moving_pace = m
        .moving_pace_min_per_km
        .map(format_pace)
        .unwrap_or_else(|| "--".into());
    println!(
        "  Moving   {}  (stopped {})    Moving pace  {}",
        format_duration(m.moving_seconds),
        format_duration(m.stopped_seconds),
        moving_pace
    );

    if let Some(ref pace) = m.pace {
        let cv = m
            .pace_cv
            .map(|c| format!("  · CV {:.0}%", c * 100.0))
            .unwrap_or_default();
        println!(
            "  Pace     med {}  · {}–{}{}",
            format_pace(pace.median),
            format_pace(pace.min),
            format_pace(pace.max),
            cv
        );
    }

    if !m.best_efforts.is_empty() {
        let parts: Vec<String> = m
            .best_efforts
            .iter()
            .take(4)
            .map(|b| {
                if let Some(dur) = b.duration_seconds {
                    if b.label.contains("min") {
                        format!(
                            "{} {}",
                            b.label,
                            b.distance_km
                                .map(|d| format!("{d:.2} km"))
                                .unwrap_or_else(|| "--".into())
                        )
                    } else {
                        format!("{} {}", b.label, format_duration(dur))
                    }
                } else {
                    b.label.clone()
                }
            })
            .collect();
        println!("  Best     {}", parts.join("  ·  "));
    }

    if let Some(ref cad) = m.cadence {
        let cv = m
            .cadence_cv
            .map(|c| format!("  · CV {:.0}%", c * 100.0))
            .unwrap_or_default();
        let stride = m
            .avg_stride_m
            .map(|s| format!("  · stride ~{s:.2} m"))
            .unwrap_or_default();
        println!(
            "  Cadence  med {:.0}  · {:.0}–{:.0}{}{}  (device units)",
            cad.median, cad.min, cad.max, cv, stride
        );
    }

    if m.elev_min_m.is_some() || m.elev_max_m.is_some() {
        let mut parts = Vec::new();
        if let (Some(lo), Some(hi)) = (m.elev_min_m, m.elev_max_m) {
            parts.push(format!("{lo:.0}–{hi:.0} m"));
        }
        if let Some(net) = m.elev_net_m {
            parts.push(format!("net {net:+.0} m"));
        }
        if let (Some(a), Some(d)) = (m.ascent_m, m.descent_m) {
            parts.push(format!("↑{a:.0} ↓{d:.0} (smoothed)"));
        }
        if let Some(gap) = m.grade_adj_pace_min_per_km {
            parts.push(format!("GAP {}", format_pace(gap)));
        }
        if let Some(vam) = m.vam_m_per_hour {
            parts.push(format!("VAM {vam:.0} m/h"));
        }
        if !parts.is_empty() {
            println!("  Elev     {}", parts.join("  ·  "));
        }
    }

    {
        let mut hr_parts = Vec::new();
        if let Some(min) = m.hr_min {
            hr_parts.push(format!("min {min:.0}"));
        }
        if let Some(drift) = m.hr_drift_pct {
            hr_parts.push(format!("drift {drift:+.1}%"));
        }
        if !hr_parts.is_empty() {
            println!("  HR       {}", hr_parts.join("  ·  "));
        }
    }

    if stored_zones_empty {
        if let Some(ref z) = m.hr_zones_recomputed {
            if zones_total_seconds(z) > 0 {
                println!("  Track zones: {}", format_hr_zones_bar(z));
            }
        }
    }

    if let Some(ref route) = m.route {
        let kind = match route.kind {
            RouteKind::Loop => "loop",
            RouteKind::PointToPoint => "point-to-point",
            RouteKind::Unknown => "unknown",
        };
        let mut parts = vec![kind.to_string()];
        if let Some(gps) = route.gps_distance_km {
            if let Some(dev) = device_distance_km {
                parts.push(format!("GPS {gps:.2} km (device {dev:.2})"));
            } else {
                parts.push(format!("GPS {gps:.2} km"));
            }
        }
        if let Some(gap) = route.start_end_gap_m {
            parts.push(format!("start–end {gap:.0} m"));
        }
        println!("  Route    {}", parts.join("  ·  "));
    }

    if show_synthetic_splits {
        let full: Vec<_> = m
            .synthetic_km_splits
            .iter()
            .filter(|s| !s.partial || s.distance_km >= 0.2)
            .collect();
        if full.iter().any(|s| !s.partial) {
            println!("\nCOMPUTED KM SPLITS");
            let show_hr = full.iter().any(|s| s.avg_hr.is_some());
            let mut rows = Vec::new();
            for s in full {
                let label = if s.partial {
                    format!("{:.2}*", s.distance_km)
                } else {
                    s.km_index.to_string()
                };
                let mut row = vec![
                    label,
                    format!("{:.2} km", s.distance_km),
                    format_duration(s.duration_seconds),
                    format_pace(s.pace_min_per_km),
                ];
                if show_hr {
                    row.push(
                        s.avg_hr
                            .map(|h| format!("{h:.0}"))
                            .unwrap_or_else(|| "--".into()),
                    );
                }
                rows.push(row);
            }
            if show_hr {
                print_table(vec!["Km", "Distance", "Time", "Pace", "Avg HR"], rows);
            } else {
                print_table(vec!["Km", "Distance", "Time", "Pace"], rows);
            }
        }
    }
}

/// Load full workout detail (header + exercises + sets), same shape as `workout show`.
/// Returns `Ok(None)` when the workout id does not exist or is soft-deleted.
pub(crate) fn fetch_workout_detail(
    conn: &Connection,
    id: i64,
) -> Result<Option<serde_json::Value>> {
    let row = conn
        .query_row(
            "SELECT id, started_at, finished_at, workout_type, notes, overall_feeling, \
             duration_minutes, created_at, deleted_at
             FROM workouts WHERE id = ?1 AND deleted_at IS NULL",
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
                    "created_at": r.get::<_, String>(7)?,
                    "deleted_at": r.get::<_, Option<String>>(8)?,
                }))
            },
        )
        .optional()?;
    let Some(mut w) = row else {
        return Ok(None);
    };
    let started_at = w["started_at"].as_str().unwrap_or("").to_string();
    let activity_date = activity_date_prefix(&started_at);
    let mut stmt = conn.prepare(
        r#"SELECT we.id, e.name, we."order", we.notes, e.load_type, we.goal_reps
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
                r.get::<_, Option<i64>>(5)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    let mut ex_json = Vec::new();
    for (we_id, name, order, notes, load_type, goal_reps) in exercises {
        let sets = list_sets_json(conn, we_id, &activity_date)?;
        ex_json.push(serde_json::json!({
            "workout_exercise_id": we_id,
            "name": name,
            "order": order,
            "notes": notes,
            "load_type": load_type,
            "goal_reps": goal_reps,
            "sets": sets,
        }));
    }
    if let Some(summary) = build_cardio_summary(&ex_json) {
        w["cardio_summary"] = summary;
    }
    w["exercises"] = serde_json::json!(ex_json);
    Ok(Some(w))
}

/// Human-readable dump matching `workout show` (no `--json`).
/// Cardio sessions include CARDIO SUMMARY / TRACK METRICS / splits (repslog parity).
pub(crate) fn print_workout_detail(w: &serde_json::Value) {
    let id = w["id"].as_i64().unwrap_or(0);
    println!("Workout ID: {id}");
    println!("Type: {}", w["workout_type"].as_str().unwrap_or("General"));
    println!("Started: {}", w["started_at"].as_str().unwrap_or(""));
    if let Some(notes) = w["notes"].as_str() {
        if !notes.is_empty() {
            println!("Notes: {notes}");
        }
    }

    let empty = vec![];
    let exercises = w["exercises"].as_array().unwrap_or(&empty);

    // Collect cardio sets for summary blocks.
    let mut cardio_sets: Vec<(&serde_json::Value, &serde_json::Value)> = Vec::new();
    for ex in exercises {
        for s in ex["sets"].as_array().unwrap_or(&empty) {
            if is_cardio_set(s) {
                cardio_sets.push((ex, s));
            }
        }
    }

    if !cardio_sets.is_empty() {
        println!("\nCARDIO SUMMARY");
        let mut total_dist = 0.0f64;
        let mut total_dur: u32 = 0;
        let mut total_cals: i32 = 0;
        let mut hr_samples: Vec<f64> = Vec::new();
        let mut max_hr = 0.0f64;
        let mut aggregated_zones = HeartRateZones::default();
        let mut cadence_samples: Vec<f64> = Vec::new();
        let mut ascent = 0.0f64;
        let mut descent = 0.0f64;

        for (_, s) in &cardio_sets {
            total_dist += s["distance_km"].as_f64().unwrap_or(0.0);
            total_dur += s["duration_seconds"].as_i64().unwrap_or(0).max(0) as u32;
            total_cals += s["calories_burned"].as_i64().unwrap_or(0) as i32;
            if let Some(hr) = s["avg_heart_rate_bpm"].as_f64() {
                hr_samples.push(hr);
            }
            if let Some(hr) = s["max_heart_rate_bpm"].as_f64() {
                if hr > max_hr {
                    max_hr = hr;
                }
            }
            if let Some(z) = parse_zones_value(&s["heart_rate_zones"]) {
                aggregated_zones.z1_seconds += z.z1_seconds;
                aggregated_zones.z2_seconds += z.z2_seconds;
                aggregated_zones.z3_seconds += z.z3_seconds;
                aggregated_zones.z4_seconds += z.z4_seconds;
                aggregated_zones.z5_seconds += z.z5_seconds;
            }
            if let Some(c) = s["avg_cadence_spm"].as_f64() {
                cadence_samples.push(c);
            }
            ascent += s["total_ascent_m"].as_f64().unwrap_or(0.0);
            descent += s["total_descent_m"].as_f64().unwrap_or(0.0);
        }

        let avg_hr = if hr_samples.is_empty() {
            0.0
        } else {
            hr_samples.iter().sum::<f64>() / hr_samples.len() as f64
        };
        let avg_pace = if total_dist > 0.0 {
            (total_dur as f64 / 60.0) / total_dist
        } else {
            0.0
        };
        let hr_display = if hr_samples.is_empty() && max_hr == 0.0 {
            "--".to_string()
        } else {
            format!("{} / {} bpm", avg_hr.round(), max_hr.round())
        };
        let cadence_display = if cadence_samples.is_empty() {
            "--".to_string()
        } else {
            let avg_c = cadence_samples.iter().sum::<f64>() / cadence_samples.len() as f64;
            format!("{avg_c:.0} spm")
        };
        let elev_display = if ascent > 0.0 || descent > 0.0 {
            format!("↑{ascent:.0}m ↓{descent:.0}m")
        } else {
            "--".to_string()
        };

        print_table(
            vec![
                "Total Dist",
                "Total Time",
                "Avg Pace",
                "Avg/Max HR",
                "Calories",
                "Cadence",
                "Elev",
            ],
            vec![vec![
                format!("{total_dist:.2} km"),
                format_duration(total_dur),
                format_pace(avg_pace),
                hr_display,
                format!("{total_cals} kcal"),
                cadence_display,
                elev_display,
            ]],
        );

        if total_dur > 0 && zones_total_seconds(&aggregated_zones) > 0 {
            println!("HR Zones: {}", format_hr_zones_bar(&aggregated_zones));
        }

        let mut all_laps: Vec<crate::models::Lap> = Vec::new();
        for (_, s) in &cardio_sets {
            all_laps.extend(parse_laps_value(&s["laps"]));
        }
        if !all_laps.is_empty() {
            println!("\nLAPS / SPLITS");
            let show_lap_hr = all_laps.iter().any(|l| l.avg_heart_rate_bpm.is_some());
            let mut lap_rows = Vec::new();
            for lap in all_laps {
                let mut row = vec![
                    lap.lap_number.to_string(),
                    format!("{:.2} km", lap.distance_km),
                    format_duration(lap.duration_seconds),
                    format_pace(lap.pace_min_per_km),
                ];
                if show_lap_hr {
                    row.push(
                        lap.avg_heart_rate_bpm
                            .map(|h| format!("{h:.0}"))
                            .unwrap_or_else(|| "--".into()),
                    );
                }
                lap_rows.push(row);
            }
            if show_lap_hr {
                print_table(vec!["Lap", "Distance", "Time", "Pace", "Avg HR"], lap_rows);
            } else {
                print_table(vec!["Lap", "Distance", "Time", "Pace"], lap_rows);
            }
        }

        // Trackpoint-derived metrics from first cardio set that has them.
        for (_, s) in &cardio_sets {
            if let Some(tm_val) = s.get("track_metrics") {
                if tm_val.is_null() {
                    continue;
                }
                if let Ok(tm) = serde_json::from_value::<TrackMetrics>(tm_val.clone()) {
                    let has_device_laps = !parse_laps_value(&s["laps"]).is_empty();
                    let stored_zones_empty = parse_zones_value(&s["heart_rate_zones"])
                        .map(|z| zones_total_seconds(&z) == 0)
                        .unwrap_or(true);
                    print_track_metrics(
                        &tm,
                        s["distance_km"].as_f64(),
                        stored_zones_empty,
                        !has_device_laps,
                    );
                    break;
                }
            }
        }
    }

    println!("\nEXERCISES");
    for ex in exercises {
        let name = ex["name"].as_str().unwrap_or("?");
        let we_id = ex["workout_exercise_id"].as_i64().unwrap_or(0);
        let load_type = ex["load_type"].as_str().unwrap_or("external");
        println!("{name} (WE ID: {we_id})");
        if let Some(notes) = ex["notes"].as_str() {
            if !notes.is_empty() {
                println!("Notes: {notes}");
            }
        }

        let mut set_rows = Vec::new();
        let mut left_reps = 0i32;
        let mut right_reps = 0i32;
        let mut both_or_unspec_reps = 0i32;
        let mut has_side = false;
        let goal_reps = ex["goal_reps"].as_i64().map(|g| g as i32);

        for s in ex["sets"].as_array().unwrap_or(&empty) {
            if let Some(sd) = s["side"].as_str() {
                has_side = true;
                match sd {
                    "left" => left_reps += s["reps"].as_i64().unwrap_or(0) as i32,
                    "right" => right_reps += s["reps"].as_i64().unwrap_or(0) as i32,
                    _ => both_or_unspec_reps += s["reps"].as_i64().unwrap_or(0) as i32,
                }
            } else {
                both_or_unspec_reps += s["reps"].as_i64().unwrap_or(0) as i32;
            }

            let cluster_label = s["cluster_id"]
                .as_i64()
                .map(|cid| format!(" [C{cid}]"))
                .unwrap_or_default();

            let mut details = Vec::new();
            if let Some(reps) = s["reps"].as_i64() {
                details.push(phase::format_reps_with_phase(
                    reps as i32,
                    s["phase"].as_str().unwrap_or("full"),
                ));
            }
            let load = bodyweight::format_load_display(
                load_type,
                s["weight_kg"].as_f64(),
                s["external_load_kg"].as_f64(),
            );
            if !load.is_empty() {
                details.push(load);
            }
            if let Some(dist) = s["distance_km"].as_f64() {
                details.push(format!("{dist:.2} km"));
            }
            if let Some(dur) = s["duration_seconds"].as_i64() {
                details.push(format_duration(dur.max(0) as u32));
            }
            if let Some(rpe) = s["rpe"].as_f64() {
                details.push(format!("RPE {rpe}"));
            }
            if let Some(rir) = s["rir"].as_f64() {
                details.push(format!("RIR {rir}"));
            }

            let cardio_info = if s["avg_heart_rate_bpm"].as_f64().is_some() {
                format!(
                    "{} bpm | {} | {} cal",
                    s["avg_heart_rate_bpm"]
                        .as_f64()
                        .map(|v| v.round().to_string())
                        .unwrap_or_else(|| "--".into()),
                    s["avg_pace_min_per_km"]
                        .as_f64()
                        .map(format_pace)
                        .unwrap_or_else(|| "--".into()),
                    s["calories_burned"]
                        .as_i64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "--".into())
                )
            } else {
                String::new()
            };

            let side_label = s["side"]
                .as_str()
                .map(|sd| sd.to_uppercase())
                .unwrap_or_else(|| "-".to_string());
            let phase_label = {
                let label = phase::format_phase_label(s["phase"].as_str().unwrap_or("full"));
                if label.is_empty() {
                    "full".to_string()
                } else {
                    label
                }
            };
            set_rows.push(vec![
                format!("{}{}", s["set_number"].as_i64().unwrap_or(0), cluster_label),
                side_label,
                phase_label,
                details.join(" • "),
                cardio_info,
                s["notes"].as_str().unwrap_or("").to_string(),
            ]);
        }

        if has_side || goal_reps.is_some() {
            let mut summary_parts = Vec::new();
            if left_reps > 0 || right_reps > 0 {
                summary_parts.push(format!("Left: {left_reps} reps | Right: {right_reps} reps"));
            }
            if both_or_unspec_reps > 0 && (left_reps > 0 || right_reps > 0) {
                summary_parts.push(format!("Other: {both_or_unspec_reps} reps"));
            }
            if let Some(g) = goal_reps {
                let actual = left_reps + right_reps + both_or_unspec_reps;
                summary_parts.push(format!("Goal: {g} | Actual: {actual}"));
            }
            if !summary_parts.is_empty() {
                println!("  {}", summary_parts.join("  •  "));
            }
        }

        print_table(
            vec!["Set #", "Side", "Phase", "Details", "Cardio", "Notes"],
            set_rows,
        );
    }
}

fn show_workout(db_override: Option<&str>, id: i64, json: bool) -> Result<()> {
    let conn = db::open_db(db_override)?;
    let Some(w) = fetch_workout_detail(&conn, id)? else {
        if json {
            print_error_json(&format!("workout {id} not found"));
        }
        return Err(anyhow!("workout not found"));
    };
    if json {
        print_json(&w);
    } else {
        print_workout_detail(&w);
    }
    Ok(())
}

fn list_sets_json(
    conn: &Connection,
    we_id: i64,
    activity_date: &str,
) -> Result<Vec<serde_json::Value>> {
    // Display order from set_order_revisions (F4); set_number in JSON is derived 1..n.
    let order = set_order::effective_set_order(conn, we_id)?;
    let display = set_order::set_display_numbers(&order);

    let mut sstmt = conn.prepare(
        "SELECT id, set_number, reps, weight_kg, external_load_kg, distance_km, duration_seconds,
                rpe, rir, effective_reps, cluster_id, rest_seconds, notes, side, phase,
                avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km, calories_burned,
                avg_cadence_spm, total_ascent_m, total_descent_m,
                heart_rate_zones, laps, date_of_birth, resting_hr_bpm
         FROM exercise_sets WHERE workout_exercise_id = ?1 AND deleted_at IS NULL",
    )?;
    let mut by_id: std::collections::HashMap<i64, serde_json::Value> =
        std::collections::HashMap::new();
    let mut rows = sstmt.query([we_id])?;
    while let Some(r) = rows.next()? {
        let set_id: i64 = r.get(0)?;
        let frozen_sn: i64 = r.get(1)?;
        let distance_km: Option<f64> = r.get(5)?;
        let zones: Option<String> = r.get(22)?;
        let laps: Option<String> = r.get(23)?;
        let date_of_birth: Option<String> = r.get(24)?;
        let resting_hr_bpm: Option<f64> = r.get(25)?;
        let display_n = display.get(&set_id).copied().unwrap_or(frozen_sn);
        let mut set_json = serde_json::json!({
            "id": set_id,
            "set_number": display_n,
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
            "avg_cadence_spm": r.get::<_, Option<f64>>(19)?,
            "total_ascent_m": r.get::<_, Option<f64>>(20)?,
            "total_descent_m": r.get::<_, Option<f64>>(21)?,
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
        by_id.insert(set_id, set_json);
    }
    let mut sets = Vec::with_capacity(order.len());
    for id in order {
        if let Some(s) = by_id.remove(&id) {
            sets.push(s);
        }
    }
    // Any active set missing from order (should not happen) appends last.
    for (_, s) in by_id {
        sets.push(s);
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
            dry_run,
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
            if dry_run {
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "exercise create",
                        "name": name,
                        "category": category,
                        "equipment": eq,
                        "load_type": lt,
                        "muscle_groups": muscles,
                        "description": description,
                    }),
                );
            }
            conn.execute(
                "INSERT INTO exercises (name, category, equipment, load_type, muscle_groups, description, is_custom, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
                params![name, category, eq, lt, muscles, description, db::now_utc()],
            )?;
            let id = conn.last_insert_rowid();
            entity_audit::append_create(&conn, entity_audit::entity::EXERCISE, id, None)?;
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
            dry_run,
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
            if dry_run {
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "exercise update",
                        "id": ex.id,
                        "name": ex.name,
                        "fields": sets,
                    }),
                );
            }
            let sql = format!("UPDATE exercises SET {} WHERE id = ?", sets.join(", "));
            vals.push(Box::new(ex.id));
            let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, refs.as_slice())?;
            entity_audit::append_catalog(
                &conn,
                entity_audit::entity::EXERCISE,
                ex.id,
                "exercise updated",
                None,
                Some(&serde_json::json!({ "fields": sets })),
            )?;
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
        ExerciseAction::Audit { exercise, limit } => {
            let ex = resolve_exercise(&conn, &exercise)?;
            let current = conn
                .query_row(
                    "SELECT id, name, category, equipment, load_type, muscle_groups, description,
                            is_custom, created_at
                     FROM exercises WHERE id = ?1",
                    [ex.id],
                    |r| {
                        Ok(serde_json::json!({
                            "id": r.get::<_, i64>(0)?,
                            "name": r.get::<_, String>(1)?,
                            "category": r.get::<_, String>(2)?,
                            "equipment": r.get::<_, Option<String>>(3)?,
                            "load_type": r.get::<_, String>(4)?,
                            "muscle_groups": r.get::<_, Option<String>>(5)?,
                            "description": r.get::<_, Option<String>>(6)?,
                            "is_custom": r.get::<_, Option<i64>>(7)?,
                            "created_at": r.get::<_, String>(8)?,
                        }))
                    },
                )
                .optional()?;
            let history =
                entity_audit::list_history(&conn, entity_audit::entity::EXERCISE, ex.id, limit)?;
            if current.is_none() && history.is_empty() {
                return Err(anyhow!("exercise {} not found", ex.id));
            }
            let resp = entity_audit::audit_response(
                entity_audit::entity::EXERCISE,
                ex.id,
                current,
                history,
            );
            if json {
                print_json(&resp);
            } else {
                entity_audit::print_audit_human(&resp);
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

pub(crate) fn resolve_exercise(conn: &Connection, exercise: &str) -> Result<Exercise> {
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

/// Result of resolving a workout_exercise link (may not create on dry-run).
struct ResolvedWe {
    /// Present when the row already exists, or was created (non-dry-run).
    we_id: Option<i64>,
    exercise: Exercise,
    workout_id: Option<i64>,
    would_create_workout_exercise: bool,
}

fn resolve_we_id(
    conn: &Connection,
    workout: Option<i64>,
    exercise: Option<&str>,
    workout_exercise: Option<i64>,
    dry_run: bool,
) -> Result<ResolvedWe> {
    if let Some(we_id) = workout_exercise {
        let (ex_id, workout_id): (i64, i64) = conn
            .query_row(
                "SELECT exercise_id, workout_id FROM workout_exercises WHERE id = ?1",
                [we_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| anyhow!("workout_exercise {we_id} not found"))?;
        let ex = resolve_exercise(conn, &ex_id.to_string())?;
        return Ok(ResolvedWe {
            we_id: Some(we_id),
            exercise: ex,
            workout_id: Some(workout_id),
            would_create_workout_exercise: false,
        });
    }
    let workout = workout
        .ok_or_else(|| anyhow!("provide --workout and --exercise, or --workout-exercise"))?;
    let exercise = exercise
        .ok_or_else(|| anyhow!("provide --workout and --exercise, or --workout-exercise"))?;
    let ex = resolve_exercise(conn, exercise)?;
    let _: i64 = conn
        .query_row(
            "SELECT id FROM workouts WHERE id = ?1 AND deleted_at IS NULL",
            [workout],
            |r| r.get(0),
        )
        .map_err(|_| anyhow!("workout not found: {workout}"))?;
    match conn
        .query_row(
            "SELECT id FROM workout_exercises WHERE workout_id = ?1 AND exercise_id = ?2 LIMIT 1",
            params![workout, ex.id],
            |r| r.get(0),
        )
        .optional()?
    {
        Some(id) => Ok(ResolvedWe {
            we_id: Some(id),
            exercise: ex,
            workout_id: Some(workout),
            would_create_workout_exercise: false,
        }),
        None if dry_run => Ok(ResolvedWe {
            we_id: None,
            exercise: ex,
            workout_id: Some(workout),
            would_create_workout_exercise: true,
        }),
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
            Ok(ResolvedWe {
                we_id: Some(conn.last_insert_rowid()),
                exercise: ex,
                workout_id: Some(workout),
                would_create_workout_exercise: false,
            })
        }
    }
}

fn next_set_number(conn: &Connection, we_id: i64) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(set_number), 0) + 1 FROM exercise_sets WHERE workout_exercise_id = ?1",
        [we_id],
        |r| r.get(0),
    )
    .unwrap_or(1)
}

/// Calendar day (YYYY-MM-DD) from workout `started_at` for body-weight lookup.
fn workout_event_date(conn: &Connection, workout_id: Option<i64>) -> Result<Option<String>> {
    let Some(id) = workout_id else {
        return Ok(None);
    };
    let started: String = conn
        .query_row("SELECT started_at FROM workouts WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .map_err(|_| anyhow!("workout not found: {id}"))?;
    Ok(Some(activity_date_prefix(&started)))
}

/// Resolve body-mass load, defaulting `--weight` from the latest body measurement when omitted.
#[allow(clippy::too_many_arguments)]
fn resolve_set_load(
    conn: &Connection,
    exercise_name: &str,
    load_type: &str,
    weight: Option<f64>,
    external_load: Option<f64>,
    no_weight_recorded: bool,
    requires_body_weight: bool,
    workout_id: Option<i64>,
    quiet: bool,
    json: bool,
) -> Result<(Option<f64>, Option<f64>)> {
    let measured = if weight.is_none()
        && !no_weight_recorded
        && load_type::is_body_mass(load_type)
        && requires_body_weight
    {
        let on_or_before = workout_event_date(conn, workout_id)?;
        bodyweight::lookup_measured_body_weight(conn, on_or_before.as_deref())
            .map_err(|e| anyhow!("{e}"))?
    } else {
        None
    };

    if let Some((meas_date, kg)) = &measured {
        if weight.is_none() && !quiet && !json {
            eprintln!("Using body weight {kg:.1} kg from measurement on {meas_date}");
        }
    }

    bodyweight::resolve_bodyweight_load(
        exercise_name,
        load_type,
        weight,
        external_load,
        no_weight_recorded,
        requires_body_weight,
        measured.map(|(_, kg)| kg),
    )
    .map_err(|e| anyhow!("{e}"))
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
    avg_cadence_spm: Option<f64>,
    total_ascent_m: Option<f64>,
    total_descent_m: Option<f64>,
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
        avg_cadence_spm,
        total_ascent_m,
        total_descent_m,
        heart_rate_zones: hr_zones.cloned(),
        laps: laps.map(|l| l.0.clone()),
    };
    sanity::validate_set_metrics(&proposed, limits).map_err(|e| anyhow!("{e}"))?;

    conn.execute(
        "INSERT INTO exercise_sets
         (workout_exercise_id, set_number, reps, weight_kg, external_load_kg,
          distance_km, duration_seconds, rpe, rir, effective_reps, cluster_id, rest_seconds,
          notes, side, phase, avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km,
          calories_burned, avg_cadence_spm, total_ascent_m, total_descent_m,
          heart_rate_zones, laps, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
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
            avg_cadence_spm,
            total_ascent_m,
            total_descent_m,
            zones_json,
            laps_json,
            db::now_utc(),
        ],
    )?;
    let id = conn.last_insert_rowid();
    entity_audit::append_create(conn, entity_audit::entity::EXERCISE_SET, id, None)?;
    Ok(id)
}

/// Validate set metrics without writing (for dry-run paths).
#[allow(clippy::too_many_arguments)]
fn validate_set_payload(
    reps: Option<i32>,
    weight: Option<f64>,
    external_load: Option<f64>,
    distance: Option<f64>,
    duration: Option<i32>,
    rpe: Option<f64>,
    rir: Option<f64>,
    effective_reps: Option<i32>,
    rest_seconds: Option<i32>,
    avg_hr: Option<f64>,
    max_hr: Option<f64>,
    pace: Option<f64>,
    calories: Option<i32>,
    hr_zones: Option<&HeartRateZones>,
    laps: Option<&Laps>,
    avg_cadence_spm: Option<f64>,
    total_ascent_m: Option<f64>,
    total_descent_m: Option<f64>,
    limits: &WorkoutSanityLimits,
) -> Result<()> {
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
        avg_cadence_spm,
        total_ascent_m,
        total_descent_m,
        heart_rate_zones: hr_zones.cloned(),
        laps: laps.map(|l| l.0.clone()),
    };
    sanity::validate_set_metrics(&proposed, limits).map_err(|e| anyhow!("{e}"))
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
            cadence,
            ascent,
            descent,
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            let resolved = resolve_we_id(
                &conn,
                workout,
                exercise.as_deref(),
                workout_exercise,
                dry_run,
            )?;
            let ex = &resolved.exercise;
            let resolved_phase = phase::normalize_phase(&phase).map_err(|e| anyhow!("{e}"))?;
            let requires =
                reps.is_some() || weight.is_some() || duration.is_some() || external_load.is_some();
            let (w, el) = resolve_set_load(
                &conn,
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                requires,
                resolved.workout_id,
                quiet,
                json,
            )?;
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
            validate_set_payload(
                reps,
                w,
                el,
                distance,
                duration,
                rpe,
                rir,
                effective_reps,
                rest_seconds,
                avg_heart_rate,
                max_heart_rate,
                pace,
                calories,
                zones.as_ref(),
                laps_v.as_ref(),
                cadence,
                ascent,
                descent,
                limits,
            )?;
            let sn = match resolved.we_id {
                Some(we_id) => next_set_number(&conn, we_id),
                None => 1,
            };
            if dry_run {
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "set add",
                        "workout_id": resolved.workout_id,
                        "exercise": ex.name,
                        "workout_exercise_id": resolved.we_id,
                        "would_create_workout_exercise": resolved.would_create_workout_exercise,
                        "set_number": sn,
                        "reps": reps,
                        "weight_kg": w,
                        "external_load_kg": el,
                        "distance_km": distance,
                        "duration_seconds": duration,
                        "phase": resolved_phase,
                        "avg_cadence_spm": cadence,
                        "total_ascent_m": ascent,
                        "total_descent_m": descent,
                    }),
                );
            }
            let we_id = resolved
                .we_id
                .ok_or_else(|| anyhow!("internal error: missing workout_exercise after resolve"))?;
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
                cadence,
                ascent,
                descent,
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
            cadence,
            ascent,
            descent,
            require_zones_laps,
            notes,
            phase,
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            let resolved = resolve_we_id(
                &conn,
                workout,
                exercise.as_deref(),
                workout_exercise,
                dry_run,
            )?;
            let resolved_phase = phase::normalize_phase(&phase).map_err(|e| anyhow!("{e}"))?;
            let zones_opt = parse_hr_zones(hr_zones.as_deref())?;
            let laps_v = parse_laps(laps.as_deref())?;
            if require_zones_laps {
                if zones_opt.is_none() {
                    return Err(anyhow!(
                        "--require-zones-laps: provide --hr-zones JSON (zones are required)"
                    ));
                }
                if laps_v.is_none() {
                    return Err(anyhow!(
                        "--require-zones-laps: provide --laps JSON (laps are required)"
                    ));
                }
            }
            let zones = zones_opt.unwrap_or_default();
            validate_set_payload(
                None,
                None,
                None,
                Some(distance),
                Some(duration),
                None,
                None,
                None,
                None,
                Some(avg_heart_rate),
                Some(max_heart_rate),
                Some(pace),
                Some(calories),
                Some(&zones),
                laps_v.as_ref(),
                cadence,
                ascent,
                descent,
                limits,
            )?;
            let sn = match resolved.we_id {
                Some(we_id) => next_set_number(&conn, we_id),
                None => 1,
            };
            if dry_run {
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "set add-cardio",
                        "workout_id": resolved.workout_id,
                        "exercise": resolved.exercise.name,
                        "workout_exercise_id": resolved.we_id,
                        "would_create_workout_exercise": resolved.would_create_workout_exercise,
                        "set_number": sn,
                        "distance_km": distance,
                        "duration_seconds": duration,
                        "avg_heart_rate_bpm": avg_heart_rate,
                        "max_heart_rate_bpm": max_heart_rate,
                        "avg_pace_min_per_km": pace,
                        "calories_burned": calories,
                        "avg_cadence_spm": cadence,
                        "total_ascent_m": ascent,
                        "total_descent_m": descent,
                        "heart_rate_zones": zones,
                        "laps": laps_v.as_ref().map(|l| &l.0),
                    }),
                );
            }
            let we_id = resolved
                .we_id
                .ok_or_else(|| anyhow!("internal error: missing workout_exercise after resolve"))?;
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
                cadence,
                ascent,
                descent,
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
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            let resolved = resolve_we_id(
                &conn,
                workout,
                exercise.as_deref(),
                workout_exercise,
                dry_run,
            )?;
            let ex = &resolved.exercise;
            let resolved_phase = phase::normalize_phase(&phase).map_err(|e| anyhow!("{e}"))?;
            let reps_list = parse_csv_i32(&reps, "reps")?;
            let rir_list = parse_csv_f64(&rir, "rir")?;
            let eff_list = parse_csv_i32(&effective_reps, "effective-reps")?;
            if reps_list.len() != rir_list.len() || reps_list.len() != eff_list.len() {
                return Err(anyhow!(
                    "reps, rir, and effective-reps must have the same number of values"
                ));
            }
            let (w, el) = resolve_set_load(
                &conn,
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                true,
                resolved.workout_id,
                quiet,
                json,
            )?;
            // Validate each planned set before any writes
            for (i, ((r, ri), eff)) in reps_list
                .iter()
                .zip(rir_list.iter())
                .zip(eff_list.iter())
                .enumerate()
            {
                let rest = if i > 0 { Some(rest_seconds) } else { None };
                validate_set_payload(
                    Some(*r),
                    w,
                    el,
                    None,
                    None,
                    None,
                    Some(*ri),
                    Some(*eff),
                    rest,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    limits,
                )?;
            }
            if dry_run {
                let planned: Vec<_> = reps_list
                    .iter()
                    .zip(rir_list.iter())
                    .zip(eff_list.iter())
                    .enumerate()
                    .map(|(i, ((r, ri), eff))| {
                        serde_json::json!({
                            "reps": r,
                            "rir": ri,
                            "effective_reps": eff,
                            "rest_seconds": if i > 0 { Some(rest_seconds) } else { None },
                            "weight_kg": w,
                            "external_load_kg": el,
                            "side": side,
                            "phase": resolved_phase,
                        })
                    })
                    .collect();
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "set add-cluster",
                        "workout_id": resolved.workout_id,
                        "exercise": ex.name,
                        "workout_exercise_id": resolved.we_id,
                        "would_create_workout_exercise": resolved.would_create_workout_exercise,
                        "sets": planned,
                    }),
                );
            }
            let we_id = resolved
                .we_id
                .ok_or_else(|| anyhow!("internal error: missing workout_exercise after resolve"))?;
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
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            let resolved = resolve_we_id(
                &conn,
                workout,
                exercise.as_deref(),
                workout_exercise,
                dry_run,
            )?;
            let ex = &resolved.exercise;
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
            let (w, el) = resolve_set_load(
                &conn,
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                true,
                resolved.workout_id,
                quiet,
                json,
            )?;
            let sides: Vec<&str> = match side.as_str() {
                "both" => vec!["left", "right"],
                s => vec![s],
            };
            for (i, ((r, ri), eff)) in reps_list
                .iter()
                .zip(rir_list.iter())
                .zip(eff_list.iter())
                .enumerate()
            {
                for sd in &sides {
                    let rest = if i > 0 { rest_seconds } else { None };
                    validate_set_payload(
                        Some(*r),
                        w,
                        el,
                        None,
                        None,
                        None,
                        Some(*ri),
                        Some(*eff),
                        rest,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        limits,
                    )?;
                    let _ = sd;
                }
            }
            if dry_run {
                let mut planned = vec![];
                for (i, ((r, ri), eff)) in reps_list
                    .iter()
                    .zip(rir_list.iter())
                    .zip(eff_list.iter())
                    .enumerate()
                {
                    for sd in &sides {
                        planned.push(serde_json::json!({
                            "reps": r,
                            "rir": ri,
                            "effective_reps": eff,
                            "rest_seconds": if i > 0 { rest_seconds } else { None },
                            "weight_kg": w,
                            "side": sd,
                            "phase": resolved_phase,
                        }));
                    }
                }
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "set add-unilateral",
                        "workout_id": resolved.workout_id,
                        "exercise": ex.name,
                        "workout_exercise_id": resolved.we_id,
                        "would_create_workout_exercise": resolved.would_create_workout_exercise,
                        "sets": planned,
                    }),
                );
            }
            let we_id = resolved
                .we_id
                .ok_or_else(|| anyhow!("internal error: missing workout_exercise after resolve"))?;
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
                            let mut parts = vec![format!("{} samples", tm.sample_count)];
                            parts.push(format!(
                                "moving {} (stopped {})",
                                format_duration(tm.moving_seconds),
                                format_duration(tm.stopped_seconds)
                            ));
                            if let Some(p) = tm.moving_pace_min_per_km {
                                parts.push(format!("pace ~{}", format_pace(p)));
                            }
                            if let Some(a) = tm.ascent_m {
                                parts.push(format!("↑{a:.0}m"));
                            }
                            println!("  track: {}", parts.join(" · "));
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
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            let resolved = resolve_we_id(&conn, Some(workout), Some(&exercise), None, dry_run)?;
            let ex = &resolved.exercise;
            if reps.is_none() && weight.is_none() && duration.is_none() {
                if dry_run {
                    return emit_dry_run(
                        json,
                        quiet,
                        serde_json::json!({
                            "action": "set quick",
                            "workout_id": resolved.workout_id,
                            "exercise": ex.name,
                            "workout_exercise_id": resolved.we_id,
                            "would_create_workout_exercise": resolved.would_create_workout_exercise,
                            "set": null,
                        }),
                    );
                }
                let we_id = resolved.we_id.ok_or_else(|| {
                    anyhow!("internal error: missing workout_exercise after resolve")
                })?;
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
            let (w, el) = resolve_set_load(
                &conn,
                &ex.name,
                &ex.load_type,
                weight,
                external_load,
                no_weight_recorded,
                true,
                resolved.workout_id,
                quiet,
                json,
            )?;
            validate_set_payload(
                reps, w, el, None, duration, None, None, None, None, None, None, None, None, None,
                None, None, None, None, limits,
            )?;
            let sn = match resolved.we_id {
                Some(we_id) => next_set_number(&conn, we_id),
                None => 1,
            };
            if dry_run {
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "set quick",
                        "workout_id": resolved.workout_id,
                        "exercise": ex.name,
                        "workout_exercise_id": resolved.we_id,
                        "would_create_workout_exercise": resolved.would_create_workout_exercise,
                        "set_number": sn,
                        "reps": reps,
                        "weight_kg": w,
                        "duration_seconds": duration,
                        "phase": ph,
                    }),
                );
            }
            let we_id = resolved
                .we_id
                .ok_or_else(|| anyhow!("internal error: missing workout_exercise after resolve"))?;
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
            cadence,
            ascent,
            descent,
            hr_zones,
            laps,
            reason,
            dry_run,
        } => {
            let conn = db::open_db(db_override)?;
            // Load before-image for audit (all columns that set update can touch).
            type SetBefore = (
                Option<i32>,    // reps
                Option<f64>,    // weight_kg
                Option<f64>,    // external_load_kg
                Option<i32>,    // duration_seconds
                Option<f64>,    // distance_km
                Option<f64>,    // rpe
                Option<f64>,    // rir
                Option<i32>,    // effective_reps
                Option<i32>,    // rest_seconds
                Option<String>, // notes
                Option<String>, // side
                String,         // phase
                Option<f64>,    // avg_heart_rate_bpm
                Option<f64>,    // max_heart_rate_bpm
                Option<f64>,    // avg_pace_min_per_km
                Option<i32>,    // calories_burned
                Option<f64>,    // avg_cadence_spm
                Option<f64>,    // total_ascent_m
                Option<f64>,    // total_descent_m
                Option<String>, // heart_rate_zones
                Option<String>, // laps
            );
            let before: Option<SetBefore> = conn
                .query_row(
                    "SELECT reps, weight_kg, external_load_kg, duration_seconds, distance_km,
                            rpe, rir, effective_reps, rest_seconds, notes, side, phase,
                            avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km,
                            calories_burned, avg_cadence_spm, total_ascent_m, total_descent_m,
                            heart_rate_zones, laps
                     FROM exercise_sets WHERE id=?1",
                    [id],
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
                            r.get(9)?,
                            r.get(10)?,
                            r.get(11)?,
                            r.get(12)?,
                            r.get(13)?,
                            r.get(14)?,
                            r.get(15)?,
                            r.get(16)?,
                            r.get(17)?,
                            r.get(18)?,
                            r.get(19)?,
                            r.get(20)?,
                        ))
                    },
                )
                .optional()?;
            let Some((
                old_reps,
                old_weight,
                old_external,
                old_duration,
                old_distance,
                old_rpe,
                old_rir,
                old_effective,
                old_rest,
                old_notes,
                old_side,
                old_phase,
                old_avg_hr,
                old_max_hr,
                old_pace,
                old_calories,
                old_cadence,
                old_ascent,
                old_descent,
                old_zones,
                old_laps,
            )) = before
            else {
                return Err(anyhow!("set {id} not found"));
            };
            let resolved_phase = phase
                .as_ref()
                .map(|p| phase::normalize_phase(p))
                .transpose()
                .map_err(|e| anyhow!("{e}"))?;
            let zones = parse_hr_zones(hr_zones.as_deref())?;
            let laps_v = parse_laps(laps.as_deref())?;
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
                avg_cadence_spm: cadence,
                total_ascent_m: ascent,
                total_descent_m: descent,
                heart_rate_zones: zones.clone(),
                laps: laps_v.as_ref().map(|l| l.0.clone()),
            };
            // Only validate fields that were provided (zeros of absent are skipped in checks)
            sanity::validate_set_metrics(&proposed, limits).map_err(|e| anyhow!("{e}"))?;

            let mut sets = vec![];
            let mut vals: Vec<Box<dyn rusqlite::ToSql>> = vec![];
            let mut changes: Vec<entity_audit::FieldChange> = Vec::new();
            macro_rules! push_opt_i32 {
                ($field:expr, $col:expr, $old:expr) => {
                    if let Some(v) = $field {
                        changes.push(entity_audit::FieldChange::new(
                            $col,
                            opt_i32_json($old),
                            serde_json::json!(v),
                        ));
                        sets.push(concat!($col, " = ?"));
                        vals.push(Box::new(v));
                    }
                };
            }
            macro_rules! push_opt_f64 {
                ($field:expr, $col:expr, $old:expr) => {
                    if let Some(v) = $field {
                        changes.push(entity_audit::FieldChange::new(
                            $col,
                            opt_f64_json($old),
                            serde_json::json!(v),
                        ));
                        sets.push(concat!($col, " = ?"));
                        vals.push(Box::new(v));
                    }
                };
            }
            push_opt_i32!(reps, "reps", old_reps);
            push_opt_f64!(weight, "weight_kg", old_weight);
            push_opt_f64!(external_load, "external_load_kg", old_external);
            push_opt_i32!(duration, "duration_seconds", old_duration);
            push_opt_f64!(distance, "distance_km", old_distance);
            push_opt_f64!(rpe, "rpe", old_rpe);
            push_opt_f64!(rir, "rir", old_rir);
            push_opt_i32!(effective_reps, "effective_reps", old_effective);
            push_opt_i32!(rest_seconds, "rest_seconds", old_rest);
            if let Some(v) = notes {
                changes.push(entity_audit::FieldChange::new(
                    "notes",
                    opt_str_json(old_notes.as_deref()),
                    serde_json::json!(v),
                ));
                sets.push("notes = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = side {
                changes.push(entity_audit::FieldChange::new(
                    "side",
                    opt_str_json(old_side.as_deref()),
                    serde_json::json!(v),
                ));
                sets.push("side = ?");
                vals.push(Box::new(v));
            }
            if let Some(v) = resolved_phase {
                let new_phase = v.to_string();
                changes.push(entity_audit::FieldChange::new(
                    "phase",
                    serde_json::json!(old_phase),
                    serde_json::json!(new_phase),
                ));
                sets.push("phase = ?");
                vals.push(Box::new(new_phase));
            }
            push_opt_f64!(avg_heart_rate, "avg_heart_rate_bpm", old_avg_hr);
            push_opt_f64!(max_heart_rate, "max_heart_rate_bpm", old_max_hr);
            push_opt_f64!(pace, "avg_pace_min_per_km", old_pace);
            push_opt_i32!(calories, "calories_burned", old_calories);
            push_opt_f64!(cadence, "avg_cadence_spm", old_cadence);
            push_opt_f64!(ascent, "total_ascent_m", old_ascent);
            push_opt_f64!(descent, "total_descent_m", old_descent);
            if let Some(z) = zones {
                let zones_json = serde_json::to_string(&z).map_err(|e| anyhow!("{e}"))?;
                changes.push(entity_audit::FieldChange::new(
                    "heart_rate_zones",
                    opt_str_json(old_zones.as_deref()),
                    serde_json::json!(zones_json),
                ));
                sets.push("heart_rate_zones = ?");
                vals.push(Box::new(zones_json));
            }
            if let Some(l) = laps_v {
                let laps_json = serde_json::to_string(&l.0).map_err(|e| anyhow!("{e}"))?;
                changes.push(entity_audit::FieldChange::new(
                    "laps",
                    opt_str_json(old_laps.as_deref()),
                    serde_json::json!(laps_json),
                ));
                sets.push("laps = ?");
                vals.push(Box::new(laps_json));
            }
            if sets.is_empty() {
                return Err(anyhow!("provide at least one field to update"));
            }
            let class = entity_audit::classify_field_changes(&changes);
            if dry_run {
                let reason_preview = reason.as_deref().map(str::trim).filter(|s| !s.is_empty());
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "set update",
                        "id": id,
                        "fields": sets,
                        "kind": class.as_str(),
                        "reason_required": class == entity_audit::UpdateClass::Correction,
                        "reason": reason_preview,
                    }),
                );
            }
            let reason_stored = entity_audit::require_reason_for_class(class, reason.as_deref())?;
            let sql = format!("UPDATE exercise_sets SET {} WHERE id = ?", sets.join(", "));
            vals.push(Box::new(id));
            let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            let write_op = crate::append_guard::op_for_update_class(class);
            crate::append_guard::with_write_allow(&conn, write_op, |conn| {
                conn.execute(&sql, refs.as_slice())?;
                Ok(())
            })?;
            entity_audit::append_field_change(
                &conn,
                entity_audit::entity::EXERCISE_SET,
                id,
                &changes,
                class,
                reason_stored.as_deref(),
                None,
            )?;
            if json {
                print_json(&Success::updated(
                    id,
                    class.as_str(),
                    reason_stored,
                    "set updated",
                ));
            } else {
                quiet_print(quiet, format!("Updated set {id} ({})", class.as_str()));
            }
            Ok(())
        }
        SetAction::Correct {
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
            cadence,
            ascent,
            descent,
            hr_zones,
            laps,
            reason,
            dry_run,
        } => handle_set_correct(
            db_override,
            limits,
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
            cadence,
            ascent,
            descent,
            hr_zones,
            laps,
            reason,
            dry_run,
            json,
            quiet,
        ),
        SetAction::Move {
            id,
            to,
            reason,
            dry_run,
        } => {
            if to < 1 {
                return Err(anyhow!("--to must be >= 1"));
            }
            let conn = db::open_db(db_override)?;
            let (we_id, deleted_at): (i64, Option<String>) = conn
                .query_row(
                    "SELECT workout_exercise_id, deleted_at FROM exercise_sets WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| anyhow!("set {id} not found"))?;
            if deleted_at.is_some() {
                return Err(anyhow!(
                    "set {id} is soft-deleted; restore or use an active set id"
                ));
            }

            let order_before = set_order::effective_set_order(&conn, we_id)?;
            let (order_after, from_pos, to_pos) = set_order::splice_move(&order_before, id, to)?;

            if from_pos == to_pos {
                if dry_run {
                    return emit_dry_run(
                        json,
                        quiet,
                        serde_json::json!({
                            "action": "set move",
                            "id": id,
                            "from": from_pos,
                            "to": to_pos,
                            "order_before": order_before,
                            "order_after": order_after,
                            "noop": true,
                        }),
                    );
                }
                if json {
                    print_json(&Success::ok("already at position"));
                } else {
                    quiet_print(quiet, format!("Set {id} already at position {from_pos}"));
                }
                return Ok(());
            }

            if dry_run {
                return emit_dry_run(
                    json,
                    quiet,
                    serde_json::json!({
                        "action": "set move",
                        "id": id,
                        "from": from_pos,
                        "to": to_pos,
                        "order_before": order_before,
                        "order_after": order_after,
                    }),
                );
            }

            // Append-only: insert order revision; never UPDATE exercise_sets.set_number.
            let revision_id = set_order::insert_revision(
                &conn,
                we_id,
                &order_after,
                Some("cli"),
                reason.as_deref(),
            )?;
            let meta = serde_json::json!({
                "revision_id": revision_id,
                "workout_exercise_id": we_id,
                "order_before": order_before,
                "order_after": order_after,
                "from": from_pos,
                "to": to_pos,
                "reason": reason,
            });
            entity_audit::append_set_move(
                &conn,
                id,
                &format!("moved set to position {to_pos}"),
                &meta,
            )?;

            if json {
                print_json(&serde_json::json!({
                    "success": true,
                    "id": id,
                    "revision_id": revision_id,
                    "from": from_pos,
                    "to": to_pos,
                    "order_before": order_before,
                    "order_after": order_after,
                    "message": "set moved",
                }));
            } else {
                quiet_print(
                    quiet,
                    format!("Moved set {id} to position {to_pos} (revision {revision_id})"),
                );
            }
            Ok(())
        }
        SetAction::Delete {
            id,
            reason,
            purge,
            force,
            dry_run,
        } => handle_set_delete(db_override, id, reason, purge, force, dry_run, json, quiet),
        SetAction::Audit { id, limit } => handle_set_audit(db_override, id, limit, json),
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_workout_delete(
    db_override: Option<&str>,
    id: i64,
    reason: Option<String>,
    purge: bool,
    force: bool,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT workout_type, deleted_at FROM workouts WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((workout_type, deleted_at)) = row else {
        return Err(anyhow!("workout {id} not found"));
    };
    let cascade = entity_audit::cascade_counts_workout(&conn, id)?;
    let mode = if purge { "purge" } else { "soft_delete" };

    if dry_run {
        return emit_dry_run(
            json,
            quiet,
            serde_json::json!({
                "action": "workout delete",
                "delete_id": id,
                "mode": mode,
                "already_soft_deleted": deleted_at.is_some(),
                "workout_type": workout_type,
                "cascade": cascade.to_json(),
            }),
        );
    }

    if purge {
        if cascade.total_children() > 0 && !force {
            return Err(anyhow!(
                "workout {id} purge would CASCADE-remove {} child row(s); re-run with --purge --force\n{}",
                cascade.total_children(),
                entity_audit::format_cascade_human(&cascade)
            ));
        }
        entity_audit::purge(
            &conn,
            "workouts",
            entity_audit::entity::WORKOUT,
            id,
            reason.as_deref(),
            Some(serde_json::json!({ "cascade": cascade.to_json() })),
        )?;
        emit_delete_result(
            json,
            quiet,
            id,
            "purge",
            None,
            Some(&cascade),
            "purged workout",
        )
    } else {
        let deleted_at = entity_audit::soft_delete(
            &conn,
            "workouts",
            entity_audit::entity::WORKOUT,
            id,
            reason.as_deref(),
        )?;
        emit_delete_result(
            json,
            quiet,
            id,
            "soft_delete",
            Some(&deleted_at),
            Some(&cascade),
            "soft-deleted workout",
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_set_delete(
    db_override: Option<&str>,
    id: i64,
    reason: Option<String>,
    purge: bool,
    force: bool,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    let exists: Option<Option<String>> = conn
        .query_row(
            "SELECT deleted_at FROM exercise_sets WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(anyhow!("set {id} not found"));
    }
    let cascade = entity_audit::cascade_counts_set(&conn, id)?;
    let mode = if purge { "purge" } else { "soft_delete" };

    if dry_run {
        return emit_dry_run(
            json,
            quiet,
            serde_json::json!({
                "action": "set delete",
                "delete_id": id,
                "mode": mode,
                "cascade": cascade.to_json(),
            }),
        );
    }

    if purge {
        if cascade.total_children() > 0 && !force {
            return Err(anyhow!(
                "set {id} purge would CASCADE-remove {} trackpoint(s); re-run with --purge --force\n{}",
                cascade.activity_trackpoints.unwrap_or(0),
                entity_audit::format_cascade_human(&cascade)
            ));
        }
        entity_audit::purge(
            &conn,
            "exercise_sets",
            entity_audit::entity::EXERCISE_SET,
            id,
            reason.as_deref(),
            Some(serde_json::json!({ "cascade": cascade.to_json() })),
        )?;
        emit_delete_result(json, quiet, id, "purge", None, Some(&cascade), "purged set")
    } else {
        let deleted_at = entity_audit::soft_delete(
            &conn,
            "exercise_sets",
            entity_audit::entity::EXERCISE_SET,
            id,
            reason.as_deref(),
        )?;
        emit_delete_result(
            json,
            quiet,
            id,
            "soft_delete",
            Some(&deleted_at),
            Some(&cascade),
            "soft-deleted set",
        )
    }
}

fn emit_delete_result(
    json: bool,
    quiet: bool,
    id: i64,
    mode: &str,
    deleted_at: Option<&str>,
    cascade: Option<&CascadeCounts>,
    human_verb: &str,
) -> Result<()> {
    if json {
        print_json(&serde_json::json!({
            "success": true,
            "id": id,
            "deleted_id": id,
            "mode": mode,
            "deleted_at": deleted_at,
            "cascade": cascade.map(|c| c.to_json()),
            "message": format!("{human_verb} {id}"),
        }));
    } else {
        let mut msg = format!("{human_verb} {id}");
        if let Some(c) = cascade {
            if c.total_children() > 0 {
                msg.push('\n');
                msg.push_str(&entity_audit::format_cascade_human(c));
            }
        }
        quiet_print(quiet, msg);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_workout_correct(
    db_override: Option<&str>,
    old_id: i64,
    workout_type: Option<String>,
    notes: Option<String>,
    duration: Option<i32>,
    feeling: Option<i32>,
    started_at: Option<String>,
    finished_at: Option<String>,
    reason: String,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(anyhow!("workout correct requires a non-empty --reason"));
    }
    let conn = db::open_db(db_override)?;
    type WorkoutRow = (
        Option<String>, // deleted_at
        Option<String>, // workout_type
        Option<String>, // notes
        Option<i64>,    // duration_minutes
        Option<i64>,    // overall_feeling
        String,         // started_at
        Option<String>, // finished_at
    );
    let before: Option<WorkoutRow> = conn
        .query_row(
            "SELECT deleted_at, workout_type, notes, duration_minutes, overall_feeling, \
             started_at, finished_at FROM workouts WHERE id=?1",
            [old_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()?;
    let (old_type, old_notes, old_duration, old_feeling, old_started, old_finished) = match before {
        None => return Err(anyhow!("workout {old_id} not found")),
        Some((Some(_), ..)) => {
            return Err(anyhow!(
                "workout {old_id} is already soft-deleted (cannot supersede)"
            ));
        }
        Some((None, t, n, d, f, s, fin)) => (t, n, d, f, s, fin),
    };
    let live_sets: i64 = conn.query_row(
        "SELECT COUNT(*) FROM exercise_sets s
         JOIN workout_exercises we ON we.id = s.workout_exercise_id
         WHERE we.workout_id = ?1 AND s.deleted_at IS NULL",
        [old_id],
        |r| r.get(0),
    )?;
    if live_sets > 0 {
        return Err(anyhow!(
            "workout {old_id} has {live_sets} live set(s); superseding the session would hide them \
             from reports. Use `workout set correct` for set mistakes, or `workout update` for \
             lifecycle fills (e.g. finished_at) without moving the tree"
        ));
    }
    if let Some(f) = feeling {
        if !(1..=5).contains(&f) {
            return Err(anyhow!("feeling must be 1-5"));
        }
    }
    let new_type = workout_type.or(old_type.clone());
    let new_notes = notes.or(old_notes.clone());
    let new_duration = duration.map(|d| d as i64).or(old_duration);
    let new_feeling = feeling.map(|f| f as i64).or(old_feeling);
    let new_started = if let Some(s) = started_at {
        parse_rfc3339_instant_for_db(&s)?
    } else {
        old_started.clone()
    };
    let new_finished = if let Some(s) = finished_at {
        Some(parse_rfc3339_instant_for_db(&s)?)
    } else {
        old_finished.clone()
    };
    let mut changes: Vec<entity_audit::FieldChange> = Vec::new();
    if new_type != old_type {
        changes.push(entity_audit::FieldChange::new(
            "workout_type",
            opt_str_json(old_type.as_deref()),
            opt_str_json(new_type.as_deref()),
        ));
    }
    if new_notes != old_notes {
        changes.push(entity_audit::FieldChange::new(
            "notes",
            opt_str_json(old_notes.as_deref()),
            opt_str_json(new_notes.as_deref()),
        ));
    }
    if new_duration != old_duration {
        changes.push(entity_audit::FieldChange::new(
            "duration_minutes",
            opt_i64_json(old_duration),
            opt_i64_json(new_duration),
        ));
    }
    if new_feeling != old_feeling {
        changes.push(entity_audit::FieldChange::new(
            "overall_feeling",
            opt_i64_json(old_feeling),
            opt_i64_json(new_feeling),
        ));
    }
    if new_started != old_started {
        changes.push(entity_audit::FieldChange::new(
            "started_at",
            serde_json::json!(old_started),
            serde_json::json!(new_started),
        ));
    }
    if new_finished != old_finished {
        changes.push(entity_audit::FieldChange::new(
            "finished_at",
            opt_str_json(old_finished.as_deref()),
            opt_str_json(new_finished.as_deref()),
        ));
    }
    if dry_run {
        return emit_dry_run(
            json,
            quiet,
            serde_json::json!({
                "action": "workout correct",
                "mode": "supersede",
                "supersedes_id": old_id,
                "reason": reason,
                "fields": changes.iter().map(|f| serde_json::json!({
                    "name": f.name, "old": f.old, "new": f.new
                })).collect::<Vec<_>>(),
            }),
        );
    }
    let created = db::now_utc();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO workouts (started_at, finished_at, workout_type, notes, duration_minutes, \
         overall_feeling, created_at, supersedes_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            new_started,
            new_finished,
            new_type,
            new_notes,
            new_duration,
            new_feeling,
            created,
            old_id
        ],
    )?;
    let new_id = tx.last_insert_rowid();
    entity_audit::append_supersede_create(
        &tx,
        entity_audit::entity::WORKOUT,
        new_id,
        old_id,
        reason,
        Some(&changes),
    )?;
    let deleted_at = entity_audit::supersede_retire(
        &tx,
        "workouts",
        entity_audit::entity::WORKOUT,
        old_id,
        new_id,
        reason,
        Some(&changes),
    )?;
    tx.commit()?;
    if json {
        print_json(&serde_json::json!({
            "success": true,
            "id": new_id,
            "supersedes_id": old_id,
            "mode": "supersede",
            "started_at": new_started,
            "finished_at": new_finished,
            "created_at": created,
            "old_deleted_at": deleted_at,
            "reason": reason,
            "message": "workout corrected (supersede)",
        }));
    } else {
        quiet_print(
            quiet,
            format!("Workout {new_id} supersedes {old_id} (reason: {reason})"),
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_set_correct(
    db_override: Option<&str>,
    limits: &crate::config::WorkoutSanityLimits,
    old_id: i64,
    reps: Option<i32>,
    weight: Option<f64>,
    external_load: Option<f64>,
    duration: Option<i32>,
    distance: Option<f64>,
    rpe: Option<f64>,
    rir: Option<f64>,
    effective_reps: Option<i32>,
    rest_seconds: Option<i32>,
    notes: Option<String>,
    side: Option<String>,
    phase: Option<String>,
    avg_heart_rate: Option<f64>,
    max_heart_rate: Option<f64>,
    pace: Option<f64>,
    calories: Option<i32>,
    cadence: Option<f64>,
    ascent: Option<f64>,
    descent: Option<f64>,
    hr_zones: Option<String>,
    laps: Option<String>,
    reason: String,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(anyhow!("set correct requires a non-empty --reason"));
    }
    let conn = db::open_db(db_override)?;
    let before = conn
        .query_row(
            "SELECT workout_exercise_id, set_number, reps, weight_kg, external_load_kg,
                    duration_seconds, distance_km, rpe, rir, effective_reps, rest_seconds,
                    notes, side, phase, avg_heart_rate_bpm, max_heart_rate_bpm,
                    avg_pace_min_per_km, calories_burned, avg_cadence_spm, total_ascent_m,
                    total_descent_m, heart_rate_zones, laps, cluster_id, deleted_at
             FROM exercise_sets WHERE id=?1",
            [old_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i32>>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                    r.get::<_, Option<i32>>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                    r.get::<_, Option<f64>>(8)?,
                    r.get::<_, Option<i32>>(9)?,
                    r.get::<_, Option<i32>>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, Option<String>>(12)?,
                    r.get::<_, String>(13)?,
                    r.get::<_, Option<f64>>(14)?,
                    r.get::<_, Option<f64>>(15)?,
                    r.get::<_, Option<f64>>(16)?,
                    r.get::<_, Option<i32>>(17)?,
                    r.get::<_, Option<f64>>(18)?,
                    r.get::<_, Option<f64>>(19)?,
                    r.get::<_, Option<f64>>(20)?,
                    r.get::<_, Option<String>>(21)?,
                    r.get::<_, Option<String>>(22)?,
                    r.get::<_, Option<i64>>(23)?,
                    r.get::<_, Option<String>>(24)?,
                ))
            },
        )
        .optional()?;
    let Some((
        we_id,
        set_number,
        old_reps,
        old_weight,
        old_external,
        old_duration,
        old_distance,
        old_rpe,
        old_rir,
        old_effective,
        old_rest,
        old_notes,
        old_side,
        old_phase,
        old_avg_hr,
        old_max_hr,
        old_pace,
        old_calories,
        old_cadence,
        old_ascent,
        old_descent,
        old_zones,
        old_laps,
        cluster_id,
        deleted_at,
    )) = before
    else {
        return Err(anyhow!("set {old_id} not found"));
    };
    if deleted_at.is_some() {
        return Err(anyhow!(
            "set {old_id} is already soft-deleted (cannot supersede)"
        ));
    }
    let new_reps = reps.or(old_reps);
    let new_weight = weight.or(old_weight);
    let new_external = external_load.or(old_external);
    let new_duration = duration.or(old_duration);
    let new_distance = distance.or(old_distance);
    let new_rpe = rpe.or(old_rpe);
    let new_rir = rir.or(old_rir);
    let new_effective = effective_reps.or(old_effective);
    let new_rest = rest_seconds.or(old_rest);
    let new_notes = notes.or(old_notes.clone());
    let new_side = side.or(old_side.clone());
    let new_phase = if let Some(p) = phase {
        phase::normalize_phase(&p)
            .map(|v| v.to_string())
            .map_err(|e| anyhow!("{e}"))?
    } else {
        old_phase.clone()
    };
    let new_avg_hr = avg_heart_rate.or(old_avg_hr);
    let new_max_hr = max_heart_rate.or(old_max_hr);
    let new_pace = pace.or(old_pace);
    let new_calories = calories.or(old_calories);
    let new_cadence = cadence.or(old_cadence);
    let new_ascent = ascent.or(old_ascent);
    let new_descent = descent.or(old_descent);
    let zones = parse_hr_zones(hr_zones.as_deref())?;
    let laps_v = parse_laps(laps.as_deref())?;
    let new_zones_json = if let Some(z) = zones {
        Some(serde_json::to_string(&z).map_err(|e| anyhow!("{e}"))?)
    } else {
        old_zones.clone()
    };
    let new_laps_json = if let Some(l) = laps_v {
        Some(serde_json::to_string(&l.0).map_err(|e| anyhow!("{e}"))?)
    } else {
        old_laps.clone()
    };
    let proposed = ProposedSetMetrics {
        reps: new_reps,
        weight_kg: new_weight,
        external_load_kg: new_external,
        distance_km: new_distance,
        duration_seconds: new_duration,
        rpe: new_rpe,
        rir: new_rir,
        effective_reps: new_effective,
        rest_seconds: new_rest,
        avg_heart_rate_bpm: new_avg_hr,
        max_heart_rate_bpm: new_max_hr,
        avg_pace_min_per_km: new_pace,
        calories_burned: new_calories,
        avg_cadence_spm: new_cadence,
        total_ascent_m: new_ascent,
        total_descent_m: new_descent,
        heart_rate_zones: new_zones_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        laps: new_laps_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
    };
    sanity::validate_set_metrics(&proposed, limits).map_err(|e| anyhow!("{e}"))?;

    let mut changes: Vec<entity_audit::FieldChange> = Vec::new();
    if new_reps != old_reps {
        changes.push(entity_audit::FieldChange::new(
            "reps",
            opt_i32_json(old_reps),
            opt_i32_json(new_reps),
        ));
    }
    if new_weight != old_weight {
        changes.push(entity_audit::FieldChange::new(
            "weight_kg",
            opt_f64_json(old_weight),
            opt_f64_json(new_weight),
        ));
    }
    if new_external != old_external {
        changes.push(entity_audit::FieldChange::new(
            "external_load_kg",
            opt_f64_json(old_external),
            opt_f64_json(new_external),
        ));
    }
    if new_duration != old_duration {
        changes.push(entity_audit::FieldChange::new(
            "duration_seconds",
            opt_i32_json(old_duration),
            opt_i32_json(new_duration),
        ));
    }
    if new_distance != old_distance {
        changes.push(entity_audit::FieldChange::new(
            "distance_km",
            opt_f64_json(old_distance),
            opt_f64_json(new_distance),
        ));
    }
    if new_rpe != old_rpe {
        changes.push(entity_audit::FieldChange::new(
            "rpe",
            opt_f64_json(old_rpe),
            opt_f64_json(new_rpe),
        ));
    }
    if new_rir != old_rir {
        changes.push(entity_audit::FieldChange::new(
            "rir",
            opt_f64_json(old_rir),
            opt_f64_json(new_rir),
        ));
    }
    if new_effective != old_effective {
        changes.push(entity_audit::FieldChange::new(
            "effective_reps",
            opt_i32_json(old_effective),
            opt_i32_json(new_effective),
        ));
    }
    if new_rest != old_rest {
        changes.push(entity_audit::FieldChange::new(
            "rest_seconds",
            opt_i32_json(old_rest),
            opt_i32_json(new_rest),
        ));
    }
    if new_notes != old_notes {
        changes.push(entity_audit::FieldChange::new(
            "notes",
            opt_str_json(old_notes.as_deref()),
            opt_str_json(new_notes.as_deref()),
        ));
    }
    if new_side != old_side {
        changes.push(entity_audit::FieldChange::new(
            "side",
            opt_str_json(old_side.as_deref()),
            opt_str_json(new_side.as_deref()),
        ));
    }
    if new_phase != old_phase {
        changes.push(entity_audit::FieldChange::new(
            "phase",
            serde_json::json!(old_phase),
            serde_json::json!(new_phase),
        ));
    }
    if new_avg_hr != old_avg_hr {
        changes.push(entity_audit::FieldChange::new(
            "avg_heart_rate_bpm",
            opt_f64_json(old_avg_hr),
            opt_f64_json(new_avg_hr),
        ));
    }
    if new_max_hr != old_max_hr {
        changes.push(entity_audit::FieldChange::new(
            "max_heart_rate_bpm",
            opt_f64_json(old_max_hr),
            opt_f64_json(new_max_hr),
        ));
    }
    if new_pace != old_pace {
        changes.push(entity_audit::FieldChange::new(
            "avg_pace_min_per_km",
            opt_f64_json(old_pace),
            opt_f64_json(new_pace),
        ));
    }
    if new_calories != old_calories {
        changes.push(entity_audit::FieldChange::new(
            "calories_burned",
            opt_i32_json(old_calories),
            opt_i32_json(new_calories),
        ));
    }
    if new_cadence != old_cadence {
        changes.push(entity_audit::FieldChange::new(
            "avg_cadence_spm",
            opt_f64_json(old_cadence),
            opt_f64_json(new_cadence),
        ));
    }
    if new_ascent != old_ascent {
        changes.push(entity_audit::FieldChange::new(
            "total_ascent_m",
            opt_f64_json(old_ascent),
            opt_f64_json(new_ascent),
        ));
    }
    if new_descent != old_descent {
        changes.push(entity_audit::FieldChange::new(
            "total_descent_m",
            opt_f64_json(old_descent),
            opt_f64_json(new_descent),
        ));
    }
    if new_zones_json != old_zones {
        changes.push(entity_audit::FieldChange::new(
            "heart_rate_zones",
            opt_str_json(old_zones.as_deref()),
            opt_str_json(new_zones_json.as_deref()),
        ));
    }
    if new_laps_json != old_laps {
        changes.push(entity_audit::FieldChange::new(
            "laps",
            opt_str_json(old_laps.as_deref()),
            opt_str_json(new_laps_json.as_deref()),
        ));
    }
    if dry_run {
        return emit_dry_run(
            json,
            quiet,
            serde_json::json!({
                "action": "set correct",
                "mode": "supersede",
                "supersedes_id": old_id,
                "reason": reason,
                "fields": changes.iter().map(|f| serde_json::json!({
                    "name": f.name, "old": f.old, "new": f.new
                })).collect::<Vec<_>>(),
            }),
        );
    }
    let created = db::now_utc();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO exercise_sets
         (workout_exercise_id, set_number, reps, weight_kg, external_load_kg,
          distance_km, duration_seconds, rpe, rir, effective_reps, cluster_id, rest_seconds,
          notes, side, phase, avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km,
          calories_burned, avg_cadence_spm, total_ascent_m, total_descent_m,
          heart_rate_zones, laps, created_at, supersedes_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)",
        params![
            we_id,
            set_number,
            new_reps,
            new_weight,
            new_external,
            new_distance,
            new_duration,
            new_rpe,
            new_rir,
            new_effective,
            cluster_id,
            new_rest,
            new_notes,
            new_side,
            new_phase,
            new_avg_hr,
            new_max_hr,
            new_pace,
            new_calories,
            new_cadence,
            new_ascent,
            new_descent,
            new_zones_json,
            new_laps_json,
            created,
            old_id
        ],
    )?;
    let new_id = tx.last_insert_rowid();
    entity_audit::append_supersede_create(
        &tx,
        entity_audit::entity::EXERCISE_SET,
        new_id,
        old_id,
        reason,
        Some(&changes),
    )?;
    let deleted_at = entity_audit::supersede_retire(
        &tx,
        "exercise_sets",
        entity_audit::entity::EXERCISE_SET,
        old_id,
        new_id,
        reason,
        Some(&changes),
    )?;
    tx.commit()?;
    if json {
        print_json(&serde_json::json!({
            "success": true,
            "id": new_id,
            "supersedes_id": old_id,
            "mode": "supersede",
            "reps": new_reps,
            "weight_kg": new_weight,
            "created_at": created,
            "old_deleted_at": deleted_at,
            "reason": reason,
            "message": "set corrected (supersede)",
        }));
    } else {
        quiet_print(
            quiet,
            format!("Set {new_id} supersedes {old_id} (reason: {reason})"),
        );
    }
    Ok(())
}

fn handle_workout_audit(db_override: Option<&str>, id: i64, limit: i64, json: bool) -> Result<()> {
    let conn = db::open_db(db_override)?;
    let current = conn
        .query_row(
            "SELECT id, started_at, finished_at, workout_type, notes, overall_feeling, \
             duration_minutes, created_at, supersedes_id, deleted_at, delete_reason
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
                    "created_at": r.get::<_, String>(7)?,
                    "supersedes_id": r.get::<_, Option<i64>>(8)?,
                    "deleted_at": r.get::<_, Option<String>>(9)?,
                    "delete_reason": r.get::<_, Option<String>>(10)?,
                }))
            },
        )
        .optional()?;
    let history = entity_audit::list_history(&conn, entity_audit::entity::WORKOUT, id, limit)?;
    // If row never existed and no audit, treat as not found.
    if current.is_none() && history.is_empty() {
        return Err(anyhow!("workout {id} not found"));
    }
    let resp = entity_audit::audit_response(entity_audit::entity::WORKOUT, id, current, history);
    if json {
        print_json(&resp);
    } else {
        entity_audit::print_audit_human(&resp);
    }
    Ok(())
}

fn handle_set_audit(db_override: Option<&str>, id: i64, limit: i64, json: bool) -> Result<()> {
    let conn = db::open_db(db_override)?;
    let current = conn
        .query_row(
            "SELECT id, workout_exercise_id, set_number, reps, weight_kg, created_at, \
             supersedes_id, deleted_at, delete_reason
             FROM exercise_sets WHERE id = ?1",
            [id],
            |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "workout_exercise_id": r.get::<_, i64>(1)?,
                    "set_number": r.get::<_, i64>(2)?,
                    "reps": r.get::<_, Option<i32>>(3)?,
                    "weight_kg": r.get::<_, Option<f64>>(4)?,
                    "created_at": r.get::<_, Option<String>>(5)?,
                    "supersedes_id": r.get::<_, Option<i64>>(6)?,
                    "deleted_at": r.get::<_, Option<String>>(7)?,
                    "delete_reason": r.get::<_, Option<String>>(8)?,
                }))
            },
        )
        .optional()?;
    let history = entity_audit::list_history(&conn, entity_audit::entity::EXERCISE_SET, id, limit)?;
    if current.is_none() && history.is_empty() {
        return Err(anyhow!("set {id} not found"));
    }
    let resp =
        entity_audit::audit_response(entity_audit::entity::EXERCISE_SET, id, current, history);
    if json {
        print_json(&resp);
    } else {
        entity_audit::print_audit_human(&resp);
    }
    Ok(())
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
                "INSERT INTO exercises (name, category, muscle_groups, equipment, load_type, description, is_custom, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,0,?7)",
                params![name, cat, muscles, eq, lt, desc, db::now_utc()],
            )?;
            added.push(name.to_string());
        }
    }
    Ok(added)
}

fn opt_str_json(v: Option<&str>) -> serde_json::Value {
    match v {
        Some(s) => serde_json::json!(s),
        None => serde_json::Value::Null,
    }
}

fn opt_i64_json(v: Option<i64>) -> serde_json::Value {
    match v {
        Some(n) => serde_json::json!(n),
        None => serde_json::Value::Null,
    }
}

fn opt_i32_json(v: Option<i32>) -> serde_json::Value {
    match v {
        Some(n) => serde_json::json!(n),
        None => serde_json::Value::Null,
    }
}

fn opt_f64_json(v: Option<f64>) -> serde_json::Value {
    match v {
        Some(n) => serde_json::json!(n),
        None => serde_json::Value::Null,
    }
}
