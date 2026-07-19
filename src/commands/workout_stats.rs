//! Workout stats: PRs, volume, summary, history, load progression (repslog parity).

use crate::bodyweight;
use crate::commands::workout::resolve_exercise;
use crate::phase;
use crate::set_order;
use crate::utils::{format_datetime, print_json, print_table};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;

/// Inclusive calendar window bind for `date(started_at, 'localtime') >= date('now', 'localtime', ?)`.
fn since_bind(days: i64) -> String {
    format!("-{} days", days.saturating_sub(1))
}

/// Resolve `--period` / `--days` for volume. Returns `(days, period_label)`.
pub fn resolve_volume_window(
    period: Option<&str>,
    days: Option<i64>,
) -> Result<(i64, Option<String>)> {
    match (period, days) {
        (Some(_), Some(_)) => Err(anyhow!(
            "provide either --period or --days for volume, not both"
        )),
        (Some(p), None) => {
            let d = match p {
                "30d" => 30,
                "90d" => 90,
                "1y" => 365,
                other => {
                    return Err(anyhow!(
                        "invalid --period '{other}'. Expected one of: 30d, 90d, 1y"
                    ));
                }
            };
            Ok((d, Some(p.to_string())))
        }
        (None, Some(d)) => Ok((d, None)),
        (None, None) => Ok((30, None)),
    }
}

pub fn handle_prs(conn: &Connection, exercise: Option<&str>, json: bool) -> Result<()> {
    let exercise_name = if let Some(ex) = exercise {
        Some(resolve_exercise(conn, ex)?.name)
    } else {
        None
    };

    let mut sql = String::from(
        "SELECT e.name,
                MAX(CASE WHEN e.load_type = 'body_mass'
                    THEN s.weight_kg + COALESCE(s.external_load_kg, 0)
                    ELSE s.weight_kg END) as max_weight,
                MAX(s.reps) as max_reps
         FROM exercise_sets s
         JOIN workout_exercises we ON s.workout_exercise_id = we.id
         JOIN exercises e ON we.exercise_id = e.id
         JOIN workouts w ON we.workout_id = w.id
         WHERE s.deleted_at IS NULL AND w.deleted_at IS NULL",
    );
    if exercise_name.is_some() {
        sql.push_str(" AND e.name = ?1");
    }
    sql.push_str(" GROUP BY e.name ORDER BY e.name");

    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<PrRow> = if let Some(ref name) = exercise_name {
        stmt.query_map(params![name], map_pr)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], map_pr)?.filter_map(|r| r.ok()).collect()
    };

    if json {
        print_json(&rows);
    } else {
        println!("Personal Records:");
        let table_rows: Vec<Vec<String>> = rows
            .iter()
            .map(|pr| {
                vec![
                    pr.exercise.clone(),
                    pr.max_weight
                        .map(|w| format!("{:.2} kg", w))
                        .unwrap_or_default(),
                    pr.max_reps.map(|r| r.to_string()).unwrap_or_default(),
                ]
            })
            .collect();
        print_table(vec!["Exercise", "Max Weight", "Max Reps"], table_rows);
    }
    Ok(())
}

fn map_pr(r: &rusqlite::Row<'_>) -> rusqlite::Result<PrRow> {
    Ok(PrRow {
        exercise: r.get(0)?,
        max_weight: r.get(1)?,
        max_reps: r.get(2)?,
    })
}

#[derive(Serialize)]
struct PrRow {
    exercise: String,
    max_weight: Option<f64>,
    max_reps: Option<i32>,
}

pub fn handle_volume(
    conn: &Connection,
    exercise: Option<&str>,
    period: Option<&str>,
    days: Option<i64>,
    json: bool,
) -> Result<()> {
    let (days, period_label) = resolve_volume_window(period, days)?;
    let since = since_bind(days);
    let exercise_name = if let Some(ex) = exercise {
        Some(resolve_exercise(conn, ex)?.name)
    } else {
        None
    };

    let mut sql = String::from(
        "SELECT e.name,
                COUNT(s.id) as sets,
                COALESCE(SUM(s.reps), 0) as total_reps,
                COALESCE(SUM(CASE
                    WHEN s.weight_kg IS NULL OR s.reps IS NULL THEN 0.0
                    WHEN e.load_type = 'body_mass'
                        THEN (s.weight_kg + COALESCE(s.external_load_kg, 0)) * s.reps
                    ELSE s.weight_kg * s.reps
                END), 0) as total_volume,
                COALESCE(SUM(s.effective_reps), 0) as total_eff_reps
         FROM exercise_sets s
         JOIN workout_exercises we ON s.workout_exercise_id = we.id
         JOIN exercises e ON we.exercise_id = e.id
         JOIN workouts w ON we.workout_id = w.id
         WHERE s.deleted_at IS NULL AND w.deleted_at IS NULL
           AND date(w.started_at, 'localtime') >= date('now', 'localtime', ?1)",
    );
    if exercise_name.is_some() {
        sql.push_str(" AND e.name = ?2");
    }
    sql.push_str(" GROUP BY e.name ORDER BY total_volume DESC LIMIT 50");

    let mut stmt = conn.prepare(&sql)?;
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<VolRow> {
        Ok(VolRow {
            exercise: r.get(0)?,
            sets: r.get(1)?,
            total_reps: r.get(2)?,
            total_volume: r.get(3)?,
            total_eff_reps: r.get(4)?,
        })
    };
    let by_exercise: Vec<VolRow> = if let Some(ref name) = exercise_name {
        stmt.query_map(params![since, name], map)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map(params![since], map)?
            .filter_map(|r| r.ok())
            .collect()
    };

    #[derive(Serialize)]
    struct VolumeOut {
        days: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        period: Option<String>,
        by_exercise: Vec<VolRow>,
    }

    let out = VolumeOut {
        days,
        period: period_label.clone(),
        by_exercise,
    };

    if json {
        print_json(&out);
    } else {
        match &period_label {
            Some(p) => println!("Training volume for period: {p} (last {days} days)"),
            None => println!("Workout volume (last {days} days):"),
        }
        let table_rows: Vec<Vec<String>> = out
            .by_exercise
            .iter()
            .map(|v| {
                vec![
                    v.exercise.clone(),
                    v.sets.to_string(),
                    v.total_reps.to_string(),
                    format!("{:.2}", v.total_volume),
                    v.total_eff_reps.to_string(),
                ]
            })
            .collect();
        print_table(
            vec!["Exercise", "Sets", "Reps", "Volume (kg·reps)", "Eff reps"],
            table_rows,
        );
    }
    Ok(())
}

#[derive(Serialize)]
struct VolRow {
    exercise: String,
    sets: i64,
    total_reps: i64,
    total_volume: f64,
    total_eff_reps: i64,
}

pub fn handle_summary(conn: &Connection, days: i64, json: bool) -> Result<()> {
    let since = since_bind(days);

    let mut stmt = conn.prepare(
        "SELECT id, duration_minutes, date(started_at, 'localtime')
         FROM workouts
         WHERE deleted_at IS NULL
           AND date(started_at, 'localtime') >= date('now', 'localtime', ?1)
         ORDER BY started_at DESC",
    )?;
    let workouts: Vec<(i64, Option<i32>, String)> = stmt
        .query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let total_workouts = workouts.len();
    let total_duration_minutes: i32 = workouts.iter().filter_map(|(_, d, _)| *d).sum();
    let average_duration_minutes = if total_workouts > 0 {
        total_duration_minutes / total_workouts as i32
    } else {
        0
    };
    let mut distinct_days: Vec<String> = workouts.iter().map(|(_, _, d)| d.clone()).collect();
    distinct_days.sort();
    distinct_days.dedup();
    let days_trained = distinct_days.len();

    let set_count: i64 = conn
        .query_row(
            "SELECT COUNT(s.id)
             FROM exercise_sets s
             JOIN workout_exercises we ON s.workout_exercise_id = we.id
             JOIN workouts w ON we.workout_id = w.id
             WHERE s.deleted_at IS NULL AND w.deleted_at IS NULL
               AND date(w.started_at, 'localtime') >= date('now', 'localtime', ?1)",
            params![since],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);

    #[derive(Serialize)]
    struct SummaryOut {
        days: i64,
        total_workouts: usize,
        total_duration_minutes: i32,
        average_duration_minutes: i32,
        set_count: i64,
        days_trained: usize,
    }

    let out = SummaryOut {
        days,
        total_workouts,
        total_duration_minutes,
        average_duration_minutes,
        set_count,
        days_trained,
    };

    if json {
        print_json(&out);
    } else {
        println!("Summary for last {days} days:");
        println!("Total Workouts: {total_workouts}");
        println!("Days Trained: {days_trained}");
        println!("Total Sets: {set_count}");
        println!("Total Duration: {total_duration_minutes} min");
        if total_workouts > 0 {
            println!("Average Duration: {average_duration_minutes} min");
        }
    }
    Ok(())
}

pub fn handle_history(conn: &Connection, exercise: &str, days: i64, json: bool) -> Result<()> {
    let exercise_name = resolve_exercise(conn, exercise)?.name;
    let since = since_bind(days);

    let mut stmt = conn.prepare(
        "SELECT w.id AS workout_id, w.started_at, w.workout_type,
                e.name AS exercise_name, e.load_type AS exercise_load_type,
                s.id, s.workout_exercise_id, s.set_number, s.reps, s.weight_kg, s.external_load_kg,
                s.duration_seconds, s.side, s.phase, s.rir, s.effective_reps, s.notes
         FROM exercise_sets s
         JOIN workout_exercises we ON s.workout_exercise_id = we.id
         JOIN exercises e ON we.exercise_id = e.id
         JOIN workouts w ON we.workout_id = w.id
         WHERE s.deleted_at IS NULL AND w.deleted_at IS NULL
           AND e.name = ?1 AND date(w.started_at, 'localtime') >= date('now', 'localtime', ?2)
         ORDER BY w.started_at ASC, s.set_number ASC, s.id ASC",
    )?;

    struct RawHist {
        workout_id: i64,
        started_at: String,
        workout_type: Option<String>,
        exercise: String,
        exercise_load_type: String,
        set_id: i64,
        we_id: i64,
        frozen_sn: i32,
        reps: Option<i32>,
        weight_kg: Option<f64>,
        external_load_kg: Option<f64>,
        duration_seconds: Option<i32>,
        side: Option<String>,
        phase: String,
        rir: Option<f64>,
        effective_reps: Option<i32>,
        notes: Option<String>,
    }

    let raw: Vec<RawHist> = stmt
        .query_map(params![exercise_name, since], |r| {
            Ok(RawHist {
                workout_id: r.get(0)?,
                started_at: r.get(1)?,
                workout_type: r.get(2)?,
                exercise: r.get(3)?,
                exercise_load_type: r.get(4)?,
                set_id: r.get(5)?,
                we_id: r.get(6)?,
                frozen_sn: r.get(7)?,
                reps: r.get(8)?,
                weight_kg: r.get(9)?,
                external_load_kg: r.get(10)?,
                duration_seconds: r.get(11)?,
                side: r.get(12)?,
                phase: r.get(13)?,
                rir: r.get(14)?,
                effective_reps: r.get(15)?,
                notes: r.get(16)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Display set_number from effective order per workout_exercise (F4).
    let mut display_cache: HashMap<i64, HashMap<i64, i64>> = HashMap::new();
    for row in &raw {
        if let std::collections::hash_map::Entry::Vacant(e) = display_cache.entry(row.we_id) {
            let order = set_order::effective_set_order(conn, row.we_id)?;
            e.insert(set_order::set_display_numbers(&order));
        }
    }

    let mut entries: Vec<HistoryEntry> = raw
        .into_iter()
        .map(|r| {
            let display_sn = display_cache
                .get(&r.we_id)
                .and_then(|m| m.get(&r.set_id).copied())
                .unwrap_or(r.frozen_sn as i64) as i32;
            HistoryEntry {
                workout_id: r.workout_id,
                date: format_datetime(&r.started_at),
                workout_type: r.workout_type,
                exercise: r.exercise,
                exercise_load_type: r.exercise_load_type,
                set_number: display_sn,
                reps: r.reps,
                weight_kg: r.weight_kg,
                external_load_kg: r.external_load_kg,
                duration_seconds: r.duration_seconds,
                side: r.side,
                phase: r.phase,
                rir: r.rir,
                effective_reps: r.effective_reps,
                notes: r.notes,
                _started_at: r.started_at,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        a._started_at
            .cmp(&b._started_at)
            .then_with(|| a.set_number.cmp(&b.set_number))
    });

    if json {
        print_json(&entries);
    } else {
        println!("Set history for '{}' (last {} days):", exercise_name, days);
        if entries.is_empty() {
            println!("No sets found in this period.");
        } else {
            let table_rows: Vec<Vec<String>> = entries
                .iter()
                .map(|e| {
                    let phase_label = {
                        let label = phase::format_phase_label(&e.phase);
                        if label.is_empty() {
                            "full".to_string()
                        } else {
                            label
                        }
                    };
                    vec![
                        e.date.clone(),
                        e.workout_id.to_string(),
                        e.set_number.to_string(),
                        e.reps.map(|r| r.to_string()).unwrap_or_else(|| {
                            e.duration_seconds
                                .map(|d| format!("{d}s"))
                                .unwrap_or_default()
                        }),
                        bodyweight::format_load_display(
                            &e.exercise_load_type,
                            e.weight_kg,
                            e.external_load_kg,
                        ),
                        e.side.clone().unwrap_or_default(),
                        phase_label,
                        e.notes.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            print_table(
                vec![
                    "Date", "Workout", "Set", "Reps", "Weight", "Side", "Phase", "Notes",
                ],
                table_rows,
            );
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct HistoryEntry {
    workout_id: i64,
    date: String,
    workout_type: Option<String>,
    exercise: String,
    exercise_load_type: String,
    set_number: i32,
    reps: Option<i32>,
    weight_kg: Option<f64>,
    external_load_kg: Option<f64>,
    duration_seconds: Option<i32>,
    side: Option<String>,
    phase: String,
    rir: Option<f64>,
    effective_reps: Option<i32>,
    notes: Option<String>,
    /// Sort key only; not part of JSON/agent contract.
    #[serde(skip)]
    _started_at: String,
}

pub fn handle_weight(conn: &Connection, exercise: &str, json: bool) -> Result<()> {
    let exercise_name = resolve_exercise(conn, exercise)?.name;

    let mut stmt = conn.prepare(
        "SELECT w.started_at, s.set_number, s.weight_kg, s.external_load_kg,
                e.load_type AS exercise_load_type, s.reps, s.notes
         FROM exercise_sets s
         JOIN workout_exercises we ON s.workout_exercise_id = we.id
         JOIN exercises e ON we.exercise_id = e.id
         JOIN workouts w ON we.workout_id = w.id
         WHERE s.deleted_at IS NULL AND w.deleted_at IS NULL
           AND e.name = ?1 AND s.weight_kg IS NOT NULL
         ORDER BY w.started_at ASC, s.set_number ASC",
    )?;

    let loads: Vec<LoadRow> = stmt
        .query_map(params![exercise_name], |r| {
            let started_at: String = r.get(0)?;
            let weight_kg: f64 = r.get(2)?;
            let external_load_kg: Option<f64> = r.get(3)?;
            let exercise_load_type: String = r.get(4)?;
            let load_display = bodyweight::format_load_display(
                &exercise_load_type,
                Some(weight_kg),
                external_load_kg,
            );
            Ok(LoadRow {
                date: format_datetime(&started_at),
                set: r.get(1)?,
                weight_kg,
                external_load_kg,
                exercise_load_type,
                load_display,
                reps: r.get(5)?,
                notes: r.get(6)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    if json {
        print_json(&loads);
    } else {
        println!("Load history for '{exercise_name}':");
        let table_rows: Vec<Vec<String>> = loads
            .iter()
            .map(|l| {
                vec![
                    l.date.clone(),
                    l.set.to_string(),
                    l.load_display.clone(),
                    l.reps.map(|r| r.to_string()).unwrap_or_default(),
                    l.notes.clone().unwrap_or_default(),
                ]
            })
            .collect();
        print_table(vec!["Date", "Set", "Load", "Reps", "Notes"], table_rows);
    }
    Ok(())
}

#[derive(Serialize)]
struct LoadRow {
    date: String,
    set: i32,
    weight_kg: f64,
    external_load_kg: Option<f64>,
    exercise_load_type: String,
    load_display: String,
    reps: Option<i32>,
    notes: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_maps_correctly() {
        assert_eq!(
            resolve_volume_window(Some("30d"), None).unwrap(),
            (30, Some("30d".into()))
        );
        assert_eq!(
            resolve_volume_window(Some("90d"), None).unwrap(),
            (90, Some("90d".into()))
        );
        assert_eq!(
            resolve_volume_window(Some("1y"), None).unwrap(),
            (365, Some("1y".into()))
        );
        assert_eq!(resolve_volume_window(None, Some(14)).unwrap(), (14, None));
        assert_eq!(resolve_volume_window(None, None).unwrap(), (30, None));
    }

    #[test]
    fn period_and_days_conflict() {
        assert!(resolve_volume_window(Some("30d"), Some(14)).is_err());
    }

    #[test]
    fn invalid_period() {
        assert!(resolve_volume_window(Some("2w"), None).is_err());
    }
}
