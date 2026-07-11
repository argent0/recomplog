//! Cross-domain reports. HTML dashboard lives in `report_html`.

use crate::cli::{
    NutritionPeriodArgs, NutritionReportAction, NutritionReportValue, ReportAction, SpendingBy,
};
use crate::config::SanityLimits;
use crate::db;
use crate::models::{
    BriefConsumption, BriefReport, BriefWorkout, BriefWorkouts, DailyNutritionEntry, MacroTotals,
    Measurement, MicroTotal, NutritionDailyReport, NutritionReport, Period, ProductSpending, Sleep,
    SpendingReport, StoreSpending, WorkoutPeriodOverview,
};
use crate::repository::BodyRepository;
use crate::utils::{format_minutes, parse_date_to_ymd, print_json, print_table};
use anyhow::{anyhow, Result};
use chrono::{Duration, Local, NaiveDate};
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap};

pub fn handle(
    action: ReportAction,
    db_override: Option<&str>,
    _sanity: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        ReportAction::Body { action } => {
            let conn = db::open_db(db_override)?;
            let mut repo = BodyRepository::new(conn);
            crate::commands::body::handle_body_report(&mut repo, action, json, quiet)
                .map_err(Into::into)
        }
        ReportAction::Sleep(args) => {
            let conn = db::open_db(db_override)?;
            let mut repo = BodyRepository::new(conn);
            crate::commands::body::handle_sleep_report_cmd(&mut repo, args, json)
                .map_err(Into::into)
        }
        ReportAction::Summary(args) => {
            let conn = db::open_db(db_override)?;
            let mut repo = BodyRepository::new(conn);
            // reuse body summary
            crate::commands::body::handle_body_report(
                &mut repo,
                crate::cli::BodyReportAction::Summary(args),
                json,
                quiet,
            )
            .map_err(Into::into)
        }
        ReportAction::Brief { days, date } => {
            handle_brief(days, date.as_deref(), db_override, json)
        }
        ReportAction::Nutrition { action } => handle_nutrition_report(action, db_override, json),
        ReportAction::Html {
            days,
            output_dir,
            name,
        } => super::report_html::handle_html(days, &output_dir, &name, db_override, json, quiet),
    }
}

// ---------- Brief report (multi-section daily dump) ----------

fn handle_brief(
    days: u32,
    date: Option<&str>,
    db_override: Option<&str>,
    json: bool,
) -> Result<()> {
    if days == 0 {
        return Err(anyhow!("--days must be >= 1"));
    }

    let local_today = Local::now().date_naive();
    let anchor = match date {
        Some(s) => crate::utils::parse_flexible_date(s)?,
        None => local_today,
    };
    let anchor_s = anchor.format("%Y-%m-%d").to_string();
    let since_date = anchor - Duration::days(i64::from(days) - 1);
    let since_s = since_date.format("%Y-%m-%d").to_string();

    let period = Period {
        since: Some(since_s.clone()),
        until: Some(anchor_s.clone()),
        days: Some(days),
    };

    // Previous N calendar days before the anchor (same N as --days).
    let prev_until = anchor - Duration::days(1);
    let prev_since = anchor - Duration::days(i64::from(days));
    let prev_since_s = prev_since.format("%Y-%m-%d").to_string();
    let prev_until_s = prev_until.format("%Y-%m-%d").to_string();
    let prev_period = Period {
        since: Some(prev_since_s.clone()),
        until: Some(prev_until_s.clone()),
        days: Some(days),
    };

    let conn = db::open_db(db_override)?;

    let consumption_today = fetch_brief_consumptions(&conn, &anchor_s)?;
    // Pass explicit since/until so nutrition is anchored to `--date`, not wall-clock today.
    let mut nutrition_daily = build_nutrition_daily_report(
        &conn,
        &NutritionPeriodArgs {
            since: Some(since_s.clone()),
            until: Some(anchor_s.clone()),
            days: None,
        },
        NutritionReportValue::Macronutrients,
    )?;
    // Preserve --days on the nested period for agents (resolve path leaves it unset).
    nutrition_daily.period.days = Some(days);
    let workouts_today = fetch_brief_workouts_detail_on_day(&conn, &anchor_s)?;
    let previous_workouts = fetch_brief_workouts_in_range(&conn, &prev_since_s, &prev_until_s)?;
    let previous_overview = fetch_workout_period_overview(&conn, prev_period, previous_workouts)?;

    let repo = BodyRepository::new(conn);
    let measurements = repo
        .list_measurements(Some(&since_s), Some(&anchor_s))
        .map_err(|e| anyhow!("{e}"))?;
    let sleep = repo
        .list_sleeps(Some(&since_s), Some(&anchor_s))
        .map_err(|e| anyhow!("{e}"))?;

    let report = BriefReport {
        period,
        consumption_today,
        nutrition_daily,
        measurements,
        sleep,
        workouts: BriefWorkouts {
            today: workouts_today,
            previous: previous_overview,
        },
    };

    if json {
        print_json(&report);
    } else {
        let day_label = if anchor == local_today {
            "today".to_string()
        } else {
            anchor_s.clone()
        };
        print_brief_human(&report, days, &day_label);
    }
    Ok(())
}

fn build_nutrition_daily_report(
    conn: &Connection,
    period: &NutritionPeriodArgs,
    value: NutritionReportValue,
) -> Result<NutritionDailyReport> {
    let resolved = resolve_nutrition_report_period(period)?;
    let rows = fetch_nutrition_consumptions(conn, &resolved)?;
    let days = build_daily_entries(&rows, resolved.fill_range, value)?;
    Ok(NutritionDailyReport {
        period: resolved.period,
        value: value.label().to_string(),
        days,
    })
}

fn fetch_brief_consumptions(conn: &Connection, today_ymd: &str) -> Result<Vec<BriefConsumption>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.product_id, p.name, c.quantity, c.unit, c.consumed_at
         FROM consumptions c
         LEFT JOIN products p ON p.id = c.product_id
         WHERE date(c.consumed_at) = date(?1)
         ORDER BY c.consumed_at DESC
         LIMIT 100",
    )?;
    let rows = stmt
        .query_map(params![today_ymd], |r| {
            Ok(BriefConsumption {
                id: r.get(0)?,
                product_id: r.get(1)?,
                product_name: r.get(2)?,
                quantity: r.get(3)?,
                unit: r.get(4)?,
                consumed_at: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn map_brief_workout(r: &rusqlite::Row<'_>) -> rusqlite::Result<BriefWorkout> {
    Ok(BriefWorkout {
        id: r.get(0)?,
        started_at: r.get(1)?,
        finished_at: r.get(2)?,
        workout_type: r.get(3)?,
        notes: r.get(4)?,
        duration_minutes: r.get(5)?,
        overall_feeling: r.get(6)?,
    })
}

/// Today's sessions with full detail (exercises + sets), newest first.
fn fetch_brief_workouts_detail_on_day(
    conn: &Connection,
    day_ymd: &str,
) -> Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM workouts
         WHERE date(started_at, 'localtime') = date(?1)
         ORDER BY started_at DESC",
    )?;
    let ids: Vec<i64> = stmt
        .query_map(params![day_ymd], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(detail) = super::workout::fetch_workout_detail(conn, id)? {
            out.push(detail);
        }
    }
    Ok(out)
}

fn fetch_brief_workouts_in_range(
    conn: &Connection,
    since_ymd: &str,
    until_ymd: &str,
) -> Result<Vec<BriefWorkout>> {
    let mut stmt = conn.prepare(
        "SELECT id, started_at, finished_at, workout_type, notes, duration_minutes, overall_feeling
         FROM workouts
         WHERE date(started_at, 'localtime') >= date(?1)
           AND date(started_at, 'localtime') <= date(?2)
         ORDER BY started_at DESC",
    )?;
    let rows = stmt
        .query_map(params![since_ymd, until_ymd], map_brief_workout)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn fetch_workout_period_overview(
    conn: &Connection,
    period: Period,
    workouts: Vec<BriefWorkout>,
) -> Result<WorkoutPeriodOverview> {
    let since = period.since.as_deref().unwrap_or("");
    let until = period.until.as_deref().unwrap_or("");

    let (workout_count, days_trained): (i64, i64) = conn.query_row(
        "SELECT COUNT(*) AS workout_count,
                COUNT(DISTINCT date(started_at, 'localtime')) AS days_trained
         FROM workouts
         WHERE date(started_at, 'localtime') >= date(?1)
           AND date(started_at, 'localtime') <= date(?2)",
        params![since, until],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let (total_volume, set_count): (f64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(CASE
                 WHEN s.weight_kg IS NULL OR s.reps IS NULL THEN 0.0
                 WHEN e.load_type = 'body_mass'
                   THEN (s.weight_kg + COALESCE(s.external_load_kg, 0)) * s.reps
                 ELSE s.weight_kg * s.reps
               END), 0) AS total_volume,
               COUNT(s.id) AS set_count
         FROM exercise_sets s
         JOIN workout_exercises we ON s.workout_exercise_id = we.id
         JOIN exercises e ON we.exercise_id = e.id
         JOIN workouts w ON we.workout_id = w.id
         WHERE date(w.started_at, 'localtime') >= date(?1)
           AND date(w.started_at, 'localtime') <= date(?2)",
        params![since, until],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    Ok(WorkoutPeriodOverview {
        period,
        workout_count,
        days_trained,
        set_count,
        total_volume,
        workouts,
    })
}

fn print_brief_human(report: &BriefReport, days: u32, day_label: &str) {
    println!("=== Consumption ({day_label}) ===");
    if report.consumption_today.is_empty() {
        println!("(no consumptions)");
    } else {
        for c in &report.consumption_today {
            let name = c.product_name.as_deref().unwrap_or("?");
            println!("{}: {} qty={} on {}", c.id, name, c.quantity, c.consumed_at);
        }
    }

    println!();
    println!("=== Nutrition by day (macronutrients, last {days} days) ===");
    print_nutrition_daily_table(&report.nutrition_daily);

    println!();
    println!("=== Measurements (last {days} days) ===");
    print_measurements_table(&report.measurements);

    println!();
    println!("=== Sleep (last {days} days) ===");
    print_sleep_table(&report.sleep);

    println!();
    println!("=== Workouts ({day_label}) ===");
    if report.workouts.today.is_empty() {
        println!("(no workouts)");
    } else {
        for (i, w) in report.workouts.today.iter().enumerate() {
            if i > 0 {
                println!();
            }
            super::workout::print_workout_detail(w);
        }
    }

    println!();
    let prev = &report.workouts.previous;
    let prev_since = prev.period.since.as_deref().unwrap_or("?");
    let prev_until = prev.period.until.as_deref().unwrap_or("?");
    println!("=== Workouts overview (previous {days} days: {prev_since} → {prev_until}) ===");
    if prev.workout_count == 0 {
        println!("(no workouts)");
    } else {
        println!(
            "Sessions: {} | Days trained: {} | Sets: {} | Volume: {:.0}",
            prev.workout_count, prev.days_trained, prev.set_count, prev.total_volume
        );
        print_workouts_table(&prev.workouts);
    }
}

fn print_nutrition_daily_table(report: &NutritionDailyReport) {
    if report.days.is_empty() {
        println!("(no days)");
        return;
    }
    let table_rows: Vec<Vec<String>> = report
        .days
        .iter()
        .map(|day| {
            vec![
                day.date.clone(),
                fmt_opt_f64(day.totals.energy_kcal),
                fmt_opt_f64(day.totals.protein_g),
                fmt_opt_f64(day.totals.carbohydrates_g),
                fmt_opt_f64(day.totals.fat_g),
                fmt_opt_f64(day.totals.fiber_g),
                fmt_opt_f64(day.totals.sugars_g),
                day.total_consumed_items.to_string(),
            ]
        })
        .collect();
    print_table(
        vec![
            "Date",
            "Energy (kcal)",
            "Protein (g)",
            "Carbs (g)",
            "Fat (g)",
            "Fiber (g)",
            "Sugars (g)",
            "Items",
        ],
        table_rows,
    );
}

fn print_measurements_table(measurements: &[Measurement]) {
    if measurements.is_empty() {
        println!("(no measurements)");
        return;
    }
    let rows: Vec<Vec<String>> = measurements
        .iter()
        .map(|m| {
            vec![
                m.id.to_string(),
                m.date.clone(),
                brief_opt_f64(m.weight_kg),
                brief_opt_f64(m.body_fat_pct),
                brief_opt_f64(m.skeletal_muscle_pct),
                brief_opt_i64(m.visceral_fat_level),
                brief_opt_f64(m.bmi),
                brief_opt_i64(m.resting_metabolism_kcal),
            ]
        })
        .collect();
    print_table(
        vec![
            "ID",
            "Date",
            "Weight (kg)",
            "Body Fat %",
            "Muscle %",
            "Visceral",
            "BMI",
            "RMR (kcal)",
        ],
        rows,
    );
}

fn print_sleep_table(sleeps: &[Sleep]) {
    if sleeps.is_empty() {
        println!("(no sleep entries)");
        return;
    }
    let rows: Vec<Vec<String>> = sleeps
        .iter()
        .map(|s| {
            vec![
                s.id.to_string(),
                s.date.clone(),
                s.bedtime.clone().unwrap_or_default(),
                s.wake_time.clone().unwrap_or_default(),
                s.total_sleep_minutes
                    .map(format_minutes)
                    .unwrap_or_default(),
                s.sleep_efficiency_pct
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
                brief_opt_i64(s.sleep_score),
                brief_opt_i64(s.subjective_quality),
            ]
        })
        .collect();
    print_table(
        vec![
            "ID",
            "Date",
            "Bedtime",
            "Wake",
            "Total Sleep",
            "Eff%",
            "Score",
            "Quality",
        ],
        rows,
    );
}

fn print_workouts_table(workouts: &[BriefWorkout]) {
    if workouts.is_empty() {
        println!("(no workouts)");
        return;
    }
    let table_rows: Vec<Vec<String>> = workouts
        .iter()
        .map(|w| {
            vec![
                w.id.to_string(),
                w.started_at.clone(),
                w.workout_type.clone().unwrap_or_default(),
                w.duration_minutes
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    print_table(vec!["id", "started", "type", "min"], table_rows);
}

fn brief_opt_f64(v: Option<f64>) -> String {
    v.map(|x| format!("{:.1}", x)).unwrap_or_default()
}

fn brief_opt_i64(v: Option<i64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

// ---------- Nutrition reports (nutlog parity) ----------

struct ResolvedNutritionPeriod {
    period: Period,
    since_ymd: Option<String>,
    until_ymd: Option<String>,
    /// Inclusive local calendar dates for zero-filling daily list entries.
    fill_range: Option<(NaiveDate, NaiveDate)>,
}

struct NutritionConsumptionRow {
    consumed_at: String,
    product_id: i64,
    scale: f64,
    energy_kcal: Option<f64>,
    protein_g: Option<f64>,
    carbohydrates_g: Option<f64>,
    fat_g: Option<f64>,
    fiber_g: Option<f64>,
    sugars_g: Option<f64>,
}

fn resolve_nutrition_report_period(args: &NutritionPeriodArgs) -> Result<ResolvedNutritionPeriod> {
    let today = Local::now().date_naive();

    if let Some(n) = args.days {
        if n == 0 {
            return Err(anyhow!("--days must be >= 1"));
        }
        let since_date = today - chrono::Duration::days(i64::from(n) - 1);
        let since_str = since_date.format("%Y-%m-%d").to_string();
        let until_str = today.format("%Y-%m-%d").to_string();
        return Ok(ResolvedNutritionPeriod {
            period: Period {
                since: Some(since_str.clone()),
                until: Some(until_str.clone()),
                days: Some(n),
            },
            since_ymd: Some(since_str),
            until_ymd: Some(until_str),
            fill_range: Some((since_date, today)),
        });
    }

    let since_ymd = args.since.as_deref().map(parse_date_to_ymd).transpose()?;
    let until_ymd = args.until.as_deref().map(parse_date_to_ymd).transpose()?;

    let fill_range = match (&since_ymd, &until_ymd) {
        (Some(s), Some(u)) => {
            let sd = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|e| anyhow!("invalid since date: {e}"))?;
            let ud = NaiveDate::parse_from_str(u, "%Y-%m-%d")
                .map_err(|e| anyhow!("invalid until date: {e}"))?;
            Some((sd, ud))
        }
        (Some(s), None) => {
            let sd = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|e| anyhow!("invalid since date: {e}"))?;
            Some((sd, today))
        }
        (None, Some(_)) | (None, None) => None,
    };

    Ok(ResolvedNutritionPeriod {
        period: Period {
            since: since_ymd.clone(),
            until: until_ymd.clone(),
            days: None,
        },
        since_ymd,
        until_ymd,
        fill_range,
    })
}

fn scale_factor(qty: f64, ref_qty: f64) -> f64 {
    if ref_qty > 0.0 {
        qty / ref_qty
    } else {
        0.0
    }
}

fn fetch_nutrition_consumptions(
    conn: &Connection,
    resolved: &ResolvedNutritionPeriod,
) -> Result<Vec<NutritionConsumptionRow>> {
    let mut sql = String::from(
        "SELECT c.quantity, pn.reference_quantity,
                pn.energy_kcal, pn.protein_g, pn.carbohydrates_g, pn.fat_g, pn.fiber_g, pn.sugars_g,
                c.product_id, c.consumed_at
         FROM consumptions c
         JOIN product_nutritions pn ON pn.product_id = c.product_id
         WHERE 1=1",
    );
    let mut bind: Vec<String> = vec![];
    if let Some(ref s) = resolved.since_ymd {
        sql.push_str(" AND date(c.consumed_at) >= date(?)");
        bind.push(s.clone());
    }
    if let Some(ref u) = resolved.until_ymd {
        sql.push_str(" AND date(c.consumed_at) <= date(?)");
        bind.push(u.clone());
    }
    sql.push_str(" ORDER BY c.consumed_at ASC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bind.iter()), |r| {
            let qty: f64 = r.get(0)?;
            let ref_q: f64 = r.get(1)?;
            Ok(NutritionConsumptionRow {
                scale: scale_factor(qty, ref_q),
                energy_kcal: r.get(2)?,
                protein_g: r.get(3)?,
                carbohydrates_g: r.get(4)?,
                fat_g: r.get(5)?,
                fiber_g: r.get(6)?,
                sugars_g: r.get(7)?,
                product_id: r.get(8)?,
                consumed_at: r.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn add_row_macros(totals: &mut MacroTotals, row: &NutritionConsumptionRow) {
    let scale = row.scale;
    if let Some(v) = row.energy_kcal {
        totals.energy_kcal = Some(totals.energy_kcal.unwrap_or(0.0) + v * scale);
    }
    if let Some(v) = row.protein_g {
        totals.protein_g = Some(totals.protein_g.unwrap_or(0.0) + v * scale);
    }
    if let Some(v) = row.carbohydrates_g {
        totals.carbohydrates_g = Some(totals.carbohydrates_g.unwrap_or(0.0) + v * scale);
    }
    if let Some(v) = row.fat_g {
        totals.fat_g = Some(totals.fat_g.unwrap_or(0.0) + v * scale);
    }
    if let Some(v) = row.fiber_g {
        totals.fiber_g = Some(totals.fiber_g.unwrap_or(0.0) + v * scale);
    }
    if let Some(v) = row.sugars_g {
        totals.sugars_g = Some(totals.sugars_g.unwrap_or(0.0) + v * scale);
    }
}

fn aggregate_micronutrients(
    conn: &Connection,
    rows: &[NutritionConsumptionRow],
) -> Result<Vec<MicroTotal>> {
    let mut micro_map: HashMap<i64, (String, String, f64)> = HashMap::new();
    for row in rows {
        let mut mstmt = conn.prepare(
            "SELECT pm.nutrient_id, pm.amount, pm.unit, n.name
             FROM product_micronutrients pm
             JOIN nutrients n ON n.id = pm.nutrient_id
             WHERE pm.product_id = ?",
        )?;
        let micros = mstmt
            .query_map([row.product_id], |mr| {
                Ok((
                    mr.get::<_, i64>(0)?,
                    mr.get::<_, f64>(1)? * row.scale,
                    mr.get::<_, String>(2)?,
                    mr.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (nid, amt, unit, nm) in micros {
            let entry = micro_map.entry(nid).or_insert((nm, unit, 0.0));
            entry.2 += amt;
        }
    }
    let mut out: Vec<MicroTotal> = micro_map
        .into_iter()
        .map(|(nid, (nm, un, tot))| MicroTotal {
            nutrient_id: nid,
            name: nm,
            unit: un,
            total_amount: tot,
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

fn apply_value_filter(totals: MacroTotals, value: NutritionReportValue) -> MacroTotals {
    match value {
        NutritionReportValue::Macronutrients => totals,
        NutritionReportValue::Calories => MacroTotals {
            energy_kcal: totals.energy_kcal,
            ..Default::default()
        },
        NutritionReportValue::Protein => MacroTotals {
            protein_g: totals.protein_g,
            ..Default::default()
        },
        NutritionReportValue::Carbohydrates => MacroTotals {
            carbohydrates_g: totals.carbohydrates_g,
            ..Default::default()
        },
        NutritionReportValue::Fat => MacroTotals {
            fat_g: totals.fat_g,
            ..Default::default()
        },
        NutritionReportValue::Fiber => MacroTotals {
            fiber_g: totals.fiber_g,
            ..Default::default()
        },
        NutritionReportValue::Sugars => MacroTotals {
            sugars_g: totals.sugars_g,
            ..Default::default()
        },
    }
}

fn consumption_day(consumed_at: &str) -> Result<NaiveDate> {
    // recomplog stores YYYY-MM-DD; tolerate longer datetime prefixes.
    let date_part = consumed_at.get(..10).unwrap_or(consumed_at);
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
        .map_err(|e| anyhow!("invalid consumed_at '{consumed_at}': {e}"))
}

fn build_daily_entries(
    rows: &[NutritionConsumptionRow],
    fill_range: Option<(NaiveDate, NaiveDate)>,
    value: NutritionReportValue,
) -> Result<Vec<DailyNutritionEntry>> {
    let mut buckets: BTreeMap<NaiveDate, (MacroTotals, i64)> = BTreeMap::new();
    for row in rows {
        let day = consumption_day(&row.consumed_at)?;
        let entry = buckets.entry(day).or_default();
        add_row_macros(&mut entry.0, row);
        entry.1 += 1;
    }

    let dates: Vec<NaiveDate> = if let Some((start, end)) = fill_range {
        let mut d = start;
        let mut out = vec![];
        while d <= end {
            out.push(d);
            d += chrono::Duration::days(1);
        }
        out
    } else {
        buckets.keys().copied().collect()
    };

    Ok(dates
        .into_iter()
        .map(|d| {
            let (totals, count) = buckets.remove(&d).unwrap_or_default();
            DailyNutritionEntry {
                date: d.format("%Y-%m-%d").to_string(),
                total_consumed_items: count,
                totals: apply_value_filter(totals, value),
            }
        })
        .collect())
}

fn print_macro_totals_human(totals: &MacroTotals, indent: &str) {
    if let Some(v) = totals.energy_kcal {
        println!("{indent}energy: {v:.1} kcal");
    }
    if let Some(v) = totals.protein_g {
        println!("{indent}protein: {v:.1} g");
    }
    if let Some(v) = totals.carbohydrates_g {
        println!("{indent}carbohydrates: {v:.1} g");
    }
    if let Some(v) = totals.fat_g {
        println!("{indent}fat: {v:.1} g");
    }
    if let Some(v) = totals.fiber_g {
        println!("{indent}fiber: {v:.1} g");
    }
    if let Some(v) = totals.sugars_g {
        println!("{indent}sugars: {v:.1} g");
    }
}

fn fmt_opt_f64(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.1}"),
        None => "—".into(),
    }
}

fn format_money_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}${}.{:02}", abs / 100, abs % 100)
}

fn push_date_filters(
    sql: &mut String,
    bind: &mut Vec<String>,
    column: &str,
    resolved: &ResolvedNutritionPeriod,
) {
    if let Some(ref s) = resolved.since_ymd {
        sql.push_str(&format!(" AND date({column}) >= date(?)"));
        bind.push(s.clone());
    }
    if let Some(ref u) = resolved.until_ymd {
        sql.push_str(&format!(" AND date({column}) <= date(?)"));
        bind.push(u.clone());
    }
}

fn nutrition_summary(conn: &Connection, period: &NutritionPeriodArgs, json: bool) -> Result<()> {
    let resolved = resolve_nutrition_report_period(period)?;
    let rows = fetch_nutrition_consumptions(conn, &resolved)?;
    let mut totals = MacroTotals::default();
    for row in &rows {
        add_row_macros(&mut totals, row);
    }
    let count = rows.len() as i64;
    let micros = aggregate_micronutrients(conn, &rows)?;

    let report = NutritionReport {
        period: resolved.period,
        total_consumed_items: count,
        totals,
        micronutrients: micros,
    };

    if json {
        print_json(&report);
    } else {
        println!("Nutrition report ({} items)", count);
        print_macro_totals_human(&report.totals, "  ");
        if !report.micronutrients.is_empty() {
            println!("  key micros:");
            for m in report.micronutrients.iter().take(5) {
                println!("    {}: {:.2} {}", m.name, m.total_amount, m.unit);
            }
        }
    }
    Ok(())
}

fn nutrition_list(
    conn: &Connection,
    period: &NutritionPeriodArgs,
    value: NutritionReportValue,
    json: bool,
) -> Result<()> {
    let report = build_nutrition_daily_report(conn, period, value)?;

    if json {
        print_json(&report);
    } else if report.days.is_empty() {
        println!("Nutrition by day ({}): (no days)", report.value);
    } else if value == NutritionReportValue::Macronutrients {
        println!("Nutrition by day ({})", report.value);
        print_nutrition_daily_table(&report);
    } else {
        println!("Nutrition by day ({})", report.value);
        let value_header = match value {
            NutritionReportValue::Calories => "Energy (kcal)",
            NutritionReportValue::Protein => "Protein (g)",
            NutritionReportValue::Carbohydrates => "Carbs (g)",
            NutritionReportValue::Fat => "Fat (g)",
            NutritionReportValue::Fiber => "Fiber (g)",
            NutritionReportValue::Sugars => "Sugars (g)",
            NutritionReportValue::Macronutrients => "Value",
        };
        let table_rows: Vec<Vec<String>> = report
            .days
            .iter()
            .map(|day| {
                let amount = match value {
                    NutritionReportValue::Calories => day.totals.energy_kcal,
                    NutritionReportValue::Protein => day.totals.protein_g,
                    NutritionReportValue::Carbohydrates => day.totals.carbohydrates_g,
                    NutritionReportValue::Fat => day.totals.fat_g,
                    NutritionReportValue::Fiber => day.totals.fiber_g,
                    NutritionReportValue::Sugars => day.totals.sugars_g,
                    NutritionReportValue::Macronutrients => None,
                };
                vec![
                    day.date.clone(),
                    fmt_opt_f64(amount),
                    day.total_consumed_items.to_string(),
                ]
            })
            .collect();
        print_table(vec!["Date", value_header, "Items"], table_rows);
    }
    Ok(())
}

fn nutrition_spending(
    conn: &Connection,
    period: &NutritionPeriodArgs,
    by: SpendingBy,
    json: bool,
) -> Result<()> {
    // Default to last 30 days when no period flags (preserves previous CLI habit).
    let period_args = if period.days.is_none() && period.since.is_none() && period.until.is_none() {
        NutritionPeriodArgs {
            days: Some(30),
            since: None,
            until: None,
        }
    } else {
        period.clone()
    };
    let resolved = resolve_nutrition_report_period(&period_args)?;

    let mut total_sql =
        String::from("SELECT COALESCE(SUM(price_cents), 0) FROM purchases pu WHERE 1=1");
    let mut total_bind: Vec<String> = vec![];
    push_date_filters(
        &mut total_sql,
        &mut total_bind,
        "pu.purchased_at",
        &resolved,
    );
    let total_cents: i64 = {
        let mut stmt = conn.prepare(&total_sql)?;
        stmt.query_row(rusqlite::params_from_iter(total_bind.iter()), |r| r.get(0))?
    };

    let mut store_sql = String::from(
        "SELECT pu.store_id, COALESCE(s.name, '(no store)'), COALESCE(SUM(pu.price_cents),0), COUNT(*)
         FROM purchases pu
         LEFT JOIN stores s ON s.id = pu.store_id
         WHERE 1=1",
    );
    let mut store_bind: Vec<String> = vec![];
    push_date_filters(
        &mut store_sql,
        &mut store_bind,
        "pu.purchased_at",
        &resolved,
    );
    store_sql.push_str(" GROUP BY pu.store_id ORDER BY SUM(pu.price_cents) DESC");

    let mut by_store = vec![];
    {
        let mut stmt = conn.prepare(&store_sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(store_bind.iter()), |r| {
            let cents: i64 = r.get(2)?;
            Ok(StoreSpending {
                store_id: r.get(0)?,
                store_name: r.get(1)?,
                cents,
                amount: format_money_cents(cents),
                purchase_count: r.get(3)?,
            })
        })?;
        for row in rows {
            by_store.push(row?);
        }
    }

    let by_product = if by == SpendingBy::Product {
        let mut psql = String::from(
            "SELECT pu.product_id, p.name, COALESCE(SUM(pu.price_cents),0), COUNT(*)
             FROM purchases pu
             JOIN products p ON p.id = pu.product_id
             WHERE 1=1",
        );
        let mut pbind: Vec<String> = vec![];
        push_date_filters(&mut psql, &mut pbind, "pu.purchased_at", &resolved);
        psql.push_str(" GROUP BY pu.product_id ORDER BY SUM(pu.price_cents) DESC");
        let mut prods = vec![];
        let mut stmt = conn.prepare(&psql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(pbind.iter()), |r| {
            let cents: i64 = r.get(2)?;
            Ok(ProductSpending {
                product_id: r.get(0)?,
                product_name: r.get(1)?,
                cents,
                amount: format_money_cents(cents),
                purchase_count: r.get(3)?,
            })
        })?;
        for row in rows {
            prods.push(row?);
        }
        Some(prods)
    } else {
        None
    };

    let report = SpendingReport {
        period: resolved.period,
        total_cents,
        total: format_money_cents(total_cents),
        by_store,
        by_product,
    };

    if json {
        print_json(&report);
    } else {
        println!("Spending total: {}", report.total);
        println!("By store:");
        for s in &report.by_store {
            println!(
                "  {}: {} ({} purchases)",
                s.store_name, s.amount, s.purchase_count
            );
        }
        if let Some(ps) = &report.by_product {
            println!("By product:");
            for p in ps {
                println!("  {}: {} ({}x)", p.product_name, p.amount, p.purchase_count);
            }
        }
    }
    Ok(())
}

fn handle_nutrition_report(
    action: NutritionReportAction,
    db_override: Option<&str>,
    json: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        NutritionReportAction::Summary(period) => nutrition_summary(&conn, &period, json),
        NutritionReportAction::List { period, value } => {
            nutrition_list(&conn, &period, value, json)
        }
        NutritionReportAction::Spending { period, by } => {
            nutrition_spending(&conn, &period, by, json)
        }
    }
}
