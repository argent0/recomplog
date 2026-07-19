//! Completeness checks: missing daily logs and workout inactivity.
//! Append-only integrity: `db check append` (F3a).
//!
//! Sanity-limit audit remains in `body::handle_check`.

use crate::cli::CheckMissingArgs;
use crate::db;
use crate::entity_audit;
use crate::models::Period;
use crate::utils::print_json;
use anyhow::{anyhow, Result};
use chrono::{Duration, Local, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;

/// Cap samples in JSON/human output (full count still reported).
const APPEND_FINDINGS_LIMIT: usize = 50;

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

// ---------------------------------------------------------------------------
// db check append (F3a)
// ---------------------------------------------------------------------------

/// One orphan row sample for append-only checks.
#[derive(Debug, Serialize)]
pub struct AppendFinding {
    pub entity: String,
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AppendFindingGroup {
    pub ok: bool,
    pub count: usize,
    pub findings: Vec<AppendFinding>,
}

#[derive(Debug, Serialize)]
pub struct AppendSchemaReport {
    pub ok: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_tables: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_columns: Vec<String>,
}

/// Full report from `recomplog db check append`.
#[derive(Debug, Serialize)]
pub struct AppendReport {
    pub ok: bool,
    pub schema: AppendSchemaReport,
    pub orphan_soft_deletes: AppendFindingGroup,
    pub orphan_updates: AppendFindingGroup,
    /// Documented allowlist (informational; does not fail the check).
    pub policy: AppendPolicyNotes,
}

#[derive(Debug, Serialize)]
pub struct AppendPolicyNotes {
    /// Columns/paths that may still UPDATE event rows under append-only policy.
    pub allowed_event_updates: Vec<&'static str>,
    /// Patterns that must not be reintroduced.
    pub forbidden: Vec<&'static str>,
    /// Legacy debt still present in the CLI (informational).
    pub legacy_debt: Vec<&'static str>,
}

fn append_policy_notes() -> AppendPolicyNotes {
    AppendPolicyNotes {
        allowed_event_updates: vec![
            "soft-delete: deleted_at / delete_reason via entity_audit::soft_delete",
            "supersede: tombstone old head + INSERT new row (supersedes_id)",
            "lifecycle: null→value fills (e.g. first finished_at) with audit kind update",
            "legacy in-place correct: settled field overwrite with --reason + audit kind correct",
            "body measurement/sleep: updated_at refresh only through repository update helpers",
        ],
        forbidden: vec![
            "bulk UPDATE of consumptions/purchases/exercise_sets/measurements/sleep/workouts payload outside helpers",
            "silent re-run of unit normalizers on open/import (I2/I3)",
            "INSERT OR REPLACE of event rows (I1)",
            "set_number sibling renumber on move (use set_order_revisions / F4)",
        ],
        legacy_debt: vec![
            "event update still overwrites some settled fields (prefer supersede correct; S5)",
            "hard purge CASCADE erases trees (soft-delete is default; S3)",
        ],
    }
}

/// Tables that must exist for append-only tooling.
const REQUIRED_TABLES: &[&str] = &["entity_audit", "set_order_revisions"];

/// (table, required columns) for soft-delete / supersede event model.
const EVENT_SCHEMA: &[(&str, &[&str])] = &[
    (
        "workouts",
        &["deleted_at", "delete_reason", "supersedes_id"],
    ),
    (
        "exercise_sets",
        &["deleted_at", "delete_reason", "supersedes_id"],
    ),
    (
        "measurements",
        &[
            "deleted_at",
            "delete_reason",
            "supersedes_id",
            "updated_at",
            "created_at",
        ],
    ),
    (
        "sleep",
        &[
            "deleted_at",
            "delete_reason",
            "supersedes_id",
            "updated_at",
            "created_at",
        ],
    ),
    (
        "consumptions",
        &["deleted_at", "delete_reason", "supersedes_id"],
    ),
    (
        "purchases",
        &["deleted_at", "delete_reason", "supersedes_id"],
    ),
];

/// (SQL table, entity_audit.entity_type) for soft-delete orphan scan.
const SOFT_DELETE_ENTITIES: &[(&str, &str)] = &[
    ("workouts", entity_audit::entity::WORKOUT),
    ("exercise_sets", entity_audit::entity::EXERCISE_SET),
    ("measurements", entity_audit::entity::MEASUREMENT),
    ("sleep", entity_audit::entity::SLEEP),
    ("consumptions", entity_audit::entity::CONSUMPTION),
    ("purchases", entity_audit::entity::PURCHASE),
];

/// Body tables with updated_at that signal in-place mutation when ≠ created_at.
const UPDATE_TRACKED: &[(&str, &str)] = &[
    ("measurements", entity_audit::entity::MEASUREMENT),
    ("sleep", entity_audit::entity::SLEEP),
];

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(cols)
}

fn check_append_schema(conn: &Connection) -> Result<AppendSchemaReport> {
    let mut missing_tables = Vec::new();
    let mut missing_columns = Vec::new();

    for t in REQUIRED_TABLES {
        if !table_exists(conn, t)? {
            missing_tables.push((*t).to_string());
        }
    }

    for (table, cols) in EVENT_SCHEMA {
        if !table_exists(conn, table)? {
            missing_tables.push((*table).to_string());
            continue;
        }
        let present = table_columns(conn, table)?;
        for col in *cols {
            if !present.contains(*col) {
                missing_columns.push(format!("{table}.{col}"));
            }
        }
    }

    Ok(AppendSchemaReport {
        ok: missing_tables.is_empty() && missing_columns.is_empty(),
        missing_tables,
        missing_columns,
    })
}

fn find_orphan_soft_deletes(conn: &Connection) -> Result<AppendFindingGroup> {
    let mut findings = Vec::new();
    let mut count = 0usize;

    // soft_delete and supersede are the legitimate tombstone audit kinds.
    let kinds = format!(
        "'{}', '{}'",
        entity_audit::kind::SOFT_DELETE,
        entity_audit::kind::SUPERSEDE
    );

    for (table, entity_type) in SOFT_DELETE_ENTITIES {
        let sql = format!(
            "SELECT t.id FROM {table} t
             WHERE t.deleted_at IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1 FROM entity_audit a
                 WHERE a.entity_type = ?1
                   AND a.entity_id = t.id
                   AND a.kind IN ({kinds})
               )
             ORDER BY t.id
             LIMIT ?"
        );
        // Full count first.
        let full_count: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM {table} t
                 WHERE t.deleted_at IS NOT NULL
                   AND NOT EXISTS (
                     SELECT 1 FROM entity_audit a
                     WHERE a.entity_type = ?1
                       AND a.entity_id = t.id
                       AND a.kind IN ({kinds})
                   )"
            ),
            params![entity_type],
            |r| r.get(0),
        )?;
        count += full_count as usize;

        let remaining = APPEND_FINDINGS_LIMIT.saturating_sub(findings.len());
        if remaining == 0 {
            continue;
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![entity_type, remaining as i64], |r| {
            r.get::<_, i64>(0)
        })?;
        for row in rows {
            let id = row?;
            findings.push(AppendFinding {
                entity: (*entity_type).to_string(),
                id,
                detail: Some("deleted_at set without soft_delete/supersede audit".into()),
            });
        }
    }

    Ok(AppendFindingGroup {
        ok: count == 0,
        count,
        findings,
    })
}

fn find_orphan_updates(conn: &Connection) -> Result<AppendFindingGroup> {
    let mut findings = Vec::new();
    let mut count = 0usize;

    let kinds = format!(
        "'{}', '{}'",
        entity_audit::kind::UPDATE,
        entity_audit::kind::CORRECT
    );

    for (table, entity_type) in UPDATE_TRACKED {
        let full_count: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM {table} t
                 WHERE t.updated_at IS NOT NULL
                   AND t.updated_at != t.created_at
                   AND NOT EXISTS (
                     SELECT 1 FROM entity_audit a
                     WHERE a.entity_type = ?1
                       AND a.entity_id = t.id
                       AND a.kind IN ({kinds})
                   )"
            ),
            params![entity_type],
            |r| r.get(0),
        )?;
        count += full_count as usize;

        let remaining = APPEND_FINDINGS_LIMIT.saturating_sub(findings.len());
        if remaining == 0 {
            continue;
        }
        let sql = format!(
            "SELECT t.id, t.created_at, t.updated_at FROM {table} t
             WHERE t.updated_at IS NOT NULL
               AND t.updated_at != t.created_at
               AND NOT EXISTS (
                 SELECT 1 FROM entity_audit a
                 WHERE a.entity_type = ?1
                   AND a.entity_id = t.id
                   AND a.kind IN ({kinds})
               )
             ORDER BY t.id
             LIMIT ?"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![entity_type, remaining as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, created_at, updated_at) = row?;
            findings.push(AppendFinding {
                entity: (*entity_type).to_string(),
                id,
                detail: Some(format!(
                    "updated_at ({updated_at}) != created_at ({created_at}) without update/correct audit"
                )),
            });
        }
    }

    Ok(AppendFindingGroup {
        ok: count == 0,
        count,
        findings,
    })
}

/// Run append-only integrity checks (F3a). Exit 1 when `ok` is false.
pub fn handle_check_append(db_override: Option<&str>, json: bool, quiet: bool) -> Result<()> {
    let conn = db::open_db(db_override)?;
    let schema = check_append_schema(&conn)?;
    // Orphan scans require entity_audit; if missing, schema already fails.
    let (orphan_soft_deletes, orphan_updates) =
        if schema.missing_tables.iter().any(|t| t == "entity_audit") {
            (
                AppendFindingGroup {
                    ok: true,
                    count: 0,
                    findings: vec![],
                },
                AppendFindingGroup {
                    ok: true,
                    count: 0,
                    findings: vec![],
                },
            )
        } else {
            (
                find_orphan_soft_deletes(&conn)?,
                find_orphan_updates(&conn)?,
            )
        };

    let ok = schema.ok && orphan_soft_deletes.ok && orphan_updates.ok;
    let report = AppendReport {
        ok,
        schema,
        orphan_soft_deletes,
        orphan_updates,
        policy: append_policy_notes(),
    };

    if json {
        print_json(&report);
    } else if quiet {
        if report.ok {
            println!("ok");
        } else {
            println!("append-only issues");
        }
    } else {
        print_append_human(&report);
    }

    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn print_append_human(report: &AppendReport) {
    println!("Append-only integrity check");
    if report.schema.ok {
        println!("  schema: OK");
    } else {
        println!("  schema: FAIL");
        for t in &report.schema.missing_tables {
            println!("    missing table: {t}");
        }
        for c in &report.schema.missing_columns {
            println!("    missing column: {c}");
        }
    }

    print_finding_group("orphan soft-deletes", &report.orphan_soft_deletes);
    print_finding_group("orphan updates", &report.orphan_updates);

    if report.ok {
        println!("OK — append-only integrity holds.");
    } else {
        println!("FAIL — see findings (inspect via … audit <id>).");
    }
}

fn print_finding_group(name: &str, g: &AppendFindingGroup) {
    if g.ok {
        println!("  {name}: OK (0)");
    } else {
        println!("  {name}: {} issue(s)", g.count);
        for f in &g.findings {
            match &f.detail {
                Some(d) => println!("    {} {} — {d}", f.entity, f.id),
                None => println!("    {} {}", f.entity, f.id),
            }
        }
        if g.count > g.findings.len() {
            println!(
                "    … and {} more (use --json for count; cap {})",
                g.count - g.findings.len(),
                APPEND_FINDINGS_LIMIT
            );
        }
    }
}
