//! Completeness checks: missing daily logs and workout inactivity.
//!
//! Sanity-limit audit remains in `body::handle_check`.

use crate::cli::CheckMissingArgs;
use crate::db;
use crate::models::Period;
use crate::utils::print_json;
use anyhow::{anyhow, Result};
use chrono::{Duration, Local, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;

/// One domain's presence summary over the daily window.
#[derive(Debug, Serialize)]
pub struct DomainPresence {
    pub expected_days: u32,
    pub present_days: u32,
    pub missing_dates: Vec<String>,
}

/// Workout inactivity section (separate window from daily domains).
#[derive(Debug, Serialize)]
pub struct WorkoutInactivity {
    pub window_days: u32,
    pub period: Period,
    pub count: i64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_workout_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_since_last: Option<i64>,
}

/// Full report from `recomplog db check missing`.
#[derive(Debug, Serialize)]
pub struct MissingReport {
    pub ok: bool,
    pub days: u32,
    pub workout_days: u32,
    pub skip_today: bool,
    pub period: Period,
    pub measurement: DomainPresence,
    pub sleep: DomainPresence,
    pub nutrition: DomainPresence,
    pub workout: WorkoutInactivity,
}

/// Inclusive local calendar window of `days` length.
///
/// Ends at **today** by default, or **yesterday** when `skip_today` is set:
/// `until - (days-1) … until`.
fn calendar_window(days: u32, skip_today: bool) -> Result<(NaiveDate, NaiveDate, Vec<String>)> {
    if days == 0 {
        return Err(anyhow!("--days / --workout-days must be >= 1"));
    }
    let today = Local::now().date_naive();
    let until = if skip_today {
        today - Duration::days(1)
    } else {
        today
    };
    let since = until - Duration::days(i64::from(days) - 1);
    let mut dates = Vec::with_capacity(days as usize);
    let mut d = since;
    while d <= until {
        dates.push(d.format("%Y-%m-%d").to_string());
        d += Duration::days(1);
    }
    Ok((since, until, dates))
}

fn period_from_window(since: NaiveDate, until: NaiveDate, days: u32) -> Period {
    Period {
        since: Some(since.format("%Y-%m-%d").to_string()),
        until: Some(until.format("%Y-%m-%d").to_string()),
        days: Some(days),
    }
}

fn missing_dates(expected: &[String], present: &HashSet<String>) -> Vec<String> {
    expected
        .iter()
        .filter(|d| !present.contains(d.as_str()))
        .cloned()
        .collect()
}

fn domain_presence(expected: &[String], present: HashSet<String>) -> DomainPresence {
    let missing = missing_dates(expected, &present);
    DomainPresence {
        expected_days: expected.len() as u32,
        present_days: (expected.len() - missing.len()) as u32,
        missing_dates: missing,
    }
}

fn distinct_dates(
    conn: &Connection,
    sql: &str,
    since: &str,
    until: &str,
) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![since, until], |r| r.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set)
}

fn list_measurement_dates(conn: &Connection, since: &str, until: &str) -> Result<HashSet<String>> {
    distinct_dates(
        conn,
        "SELECT DISTINCT date FROM measurements
         WHERE deleted_at IS NULL AND date >= ?1 AND date <= ?2
         ORDER BY date",
        since,
        until,
    )
}

fn list_sleep_dates(conn: &Connection, since: &str, until: &str) -> Result<HashSet<String>> {
    distinct_dates(
        conn,
        "SELECT DISTINCT date FROM sleep
         WHERE deleted_at IS NULL AND date >= ?1 AND date <= ?2
         ORDER BY date",
        since,
        until,
    )
}

fn list_consumption_dates(conn: &Connection, since: &str, until: &str) -> Result<HashSet<String>> {
    distinct_dates(
        conn,
        "SELECT DISTINCT date(consumed_at, 'localtime') FROM consumptions
         WHERE deleted_at IS NULL
           AND date(consumed_at, 'localtime') >= date(?1) AND date(consumed_at, 'localtime') <= date(?2)
         ORDER BY 1",
        since,
        until,
    )
}

fn count_workouts_in_window(conn: &Connection, since: &str, until: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM workouts
         WHERE deleted_at IS NULL
           AND date(started_at, 'localtime') >= date(?1)
           AND date(started_at, 'localtime') <= date(?2)",
        params![since, until],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Most recent workout calendar day on or before `until` (local).
fn last_workout_date_on_or_before(conn: &Connection, until: &str) -> Result<Option<String>> {
    let date: Option<String> = conn
        .query_row(
            "SELECT date(started_at, 'localtime') FROM workouts
             WHERE deleted_at IS NULL
               AND date(started_at, 'localtime') <= date(?1)
             ORDER BY started_at DESC LIMIT 1",
            params![until],
            |r| r.get(0),
        )
        .optional()?;
    Ok(date)
}

fn days_between(earlier: &str, later: NaiveDate) -> Option<i64> {
    let e = NaiveDate::parse_from_str(earlier, "%Y-%m-%d").ok()?;
    Some((later - e).num_days())
}

pub fn handle_check_missing(
    args: CheckMissingArgs,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if args.days == 0 {
        return Err(anyhow!("--days must be >= 1"));
    }
    if args.workout_days == 0 {
        return Err(anyhow!("--workout-days must be >= 1"));
    }

    let (since_d, until_d, expected) = calendar_window(args.days, args.skip_today)?;
    let since_s = since_d.format("%Y-%m-%d").to_string();
    let until_s = until_d.format("%Y-%m-%d").to_string();
    let period = period_from_window(since_d, until_d, args.days);

    let (w_since_d, w_until_d, _) = calendar_window(args.workout_days, args.skip_today)?;
    let w_since_s = w_since_d.format("%Y-%m-%d").to_string();
    let w_until_s = w_until_d.format("%Y-%m-%d").to_string();
    let workout_period = period_from_window(w_since_d, w_until_d, args.workout_days);

    let conn = db::open_db(db_override)?;

    let measurement = domain_presence(
        &expected,
        list_measurement_dates(&conn, &since_s, &until_s)?,
    );
    let sleep = domain_presence(&expected, list_sleep_dates(&conn, &since_s, &until_s)?);
    let nutrition = domain_presence(
        &expected,
        list_consumption_dates(&conn, &since_s, &until_s)?,
    );

    let workout_count = count_workouts_in_window(&conn, &w_since_s, &w_until_s)?;
    let last_workout_date = last_workout_date_on_or_before(&conn, &until_s)?;
    let days_since_last = last_workout_date
        .as_deref()
        .and_then(|d| days_between(d, until_d));

    let workout_ok = workout_count > 0;
    let workout = WorkoutInactivity {
        window_days: args.workout_days,
        period: workout_period,
        count: workout_count,
        ok: workout_ok,
        last_workout_date,
        days_since_last,
    };

    let ok = measurement.missing_dates.is_empty()
        && sleep.missing_dates.is_empty()
        && nutrition.missing_dates.is_empty()
        && workout_ok;

    let report = MissingReport {
        ok,
        days: args.days,
        workout_days: args.workout_days,
        skip_today: args.skip_today,
        period,
        measurement,
        sleep,
        nutrition,
        workout,
    };

    if json {
        print_json(&report);
    } else if quiet {
        if report.ok {
            println!("ok");
        } else {
            println!("incomplete");
        }
    } else {
        print_missing_human(&report);
    }

    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn print_domain_line(name: &str, d: &DomainPresence) {
    if d.missing_dates.is_empty() {
        println!("  {name}: OK ({}/{})", d.present_days, d.expected_days);
    } else {
        let n = d.missing_dates.len();
        let list = d.missing_dates.join(", ");
        println!("  {name}: {n} missing — {list}");
    }
}

fn print_missing_human(report: &MissingReport) {
    let since = report.period.since.as_deref().unwrap_or("?");
    let until = report.period.until.as_deref().unwrap_or("?");
    let skip_note = if report.skip_today {
        ", skip-today"
    } else {
        ""
    };
    println!(
        "Missing-entry check — daily window {since} … {until} ({} days{skip_note})",
        report.days
    );
    print_domain_line("measurement", &report.measurement);
    print_domain_line("sleep", &report.sleep);
    print_domain_line("nutrition", &report.nutrition);

    let w_since = report.workout.period.since.as_deref().unwrap_or("?");
    let w_until = report.workout.period.until.as_deref().unwrap_or("?");
    println!(
        "Workout inactivity — window {w_since} … {w_until} ({} days{skip_note})",
        report.workout.window_days
    );
    if report.workout.ok {
        println!("  OK — {} workout(s) in window", report.workout.count);
    } else {
        match (
            &report.workout.last_workout_date,
            report.workout.days_since_last,
        ) {
            (Some(d), Some(n)) => {
                println!("  no workouts in window (last workout {d}, {n} days ago)");
            }
            (Some(d), None) => {
                println!("  no workouts in window (last workout {d})");
            }
            _ => {
                println!("  no workouts in window (no prior workout found)");
            }
        }
    }

    if report.ok {
        println!("OK — no missing entries.");
    } else {
        println!("INCOMPLETE");
    }
}
