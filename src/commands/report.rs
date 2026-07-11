//! Cross-domain reports and HTML dashboard.

use crate::cli::{
    NutritionPeriodArgs, NutritionReportAction, NutritionReportValue, ReportAction, SpendingBy,
};
use crate::config::SanityLimits;
use crate::db;
use crate::models::{
    DailyNutritionEntry, MacroTotals, MicroTotal, NutritionDailyReport, NutritionReport, Period,
    ProductSpending, SpendingReport, StoreSpending,
};
use crate::repository::BodyRepository;
use crate::utils::{parse_date_to_ymd, print_json, print_table};
use anyhow::{anyhow, Result};
use chrono::{Local, NaiveDate};
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

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
        ReportAction::Nutrition { action } => handle_nutrition_report(action, db_override, json),
        ReportAction::Html {
            days,
            output_dir,
            name,
        } => handle_html(days, &output_dir, &name, db_override, json, quiet),
    }
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
    let resolved = resolve_nutrition_report_period(period)?;
    let rows = fetch_nutrition_consumptions(conn, &resolved)?;
    let days = build_daily_entries(&rows, resolved.fill_range, value)?;
    let report = NutritionDailyReport {
        period: resolved.period,
        value: value.label().to_string(),
        days,
    };

    if json {
        print_json(&report);
    } else if report.days.is_empty() {
        println!("Nutrition by day ({}): (no days)", report.value);
    } else if value == NutritionReportValue::Macronutrients {
        println!("Nutrition by day ({})", report.value);
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

fn handle_html(
    days: u32,
    output_dir: &str,
    name: &str,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    let today = Local::now().date_naive();
    let since = today - chrono::Duration::days(days as i64 - 1);
    let since_s = since.format("%Y-%m-%d").to_string();
    let until_s = today.format("%Y-%m-%d").to_string();

    // measurements
    let mut mstmt = conn.prepare(
        "SELECT date, weight_kg, body_fat_pct, skeletal_muscle_pct, bmi
         FROM measurements WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC",
    )?;
    let measurements: Vec<_> = mstmt
        .query_map(params![since_s, until_s], |r| {
            Ok(serde_json::json!({
                "date": r.get::<_, String>(0)?,
                "weight_kg": r.get::<_, Option<f64>>(1)?,
                "body_fat_pct": r.get::<_, Option<f64>>(2)?,
                "skeletal_muscle_pct": r.get::<_, Option<f64>>(3)?,
                "bmi": r.get::<_, Option<f64>>(4)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // sleep
    let mut sstmt = conn.prepare(
        "SELECT date, total_sleep_minutes, sleep_efficiency_pct, sleep_score
         FROM sleep WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC",
    )?;
    let sleeps: Vec<_> = sstmt
        .query_map(params![since_s, until_s], |r| {
            Ok(serde_json::json!({
                "date": r.get::<_, String>(0)?,
                "total_sleep_minutes": r.get::<_, Option<i64>>(1)?,
                "sleep_efficiency_pct": r.get::<_, Option<f64>>(2)?,
                "sleep_score": r.get::<_, Option<i64>>(3)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // nutrition daily kcal (approx)
    let mut nstmt = conn.prepare(
        "SELECT date(c.consumed_at) as d,
                SUM(CASE WHEN pn.energy_kcal IS NOT NULL AND pn.reference_quantity > 0
                    THEN c.quantity / pn.reference_quantity * pn.energy_kcal
                    ELSE 0 END) as kcal,
                SUM(CASE WHEN pn.protein_g IS NOT NULL AND pn.reference_quantity > 0
                    THEN c.quantity / pn.reference_quantity * pn.protein_g
                    ELSE 0 END) as protein
         FROM consumptions c
         LEFT JOIN product_nutritions pn ON pn.product_id = c.product_id
         WHERE date(c.consumed_at) >= date(?1) AND date(c.consumed_at) <= date(?2)
         GROUP BY date(c.consumed_at)
         ORDER BY d ASC",
    )?;
    let nutrition: Vec<_> = nstmt
        .query_map(params![since_s, until_s], |r| {
            Ok(serde_json::json!({
                "date": r.get::<_, String>(0)?,
                "energy_kcal": r.get::<_, f64>(1)?,
                "protein_g": r.get::<_, f64>(2)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let latest_weight = measurements
        .iter()
        .rev()
        .find_map(|m| m["weight_kg"].as_f64());
    let latest_bf = measurements
        .iter()
        .rev()
        .find_map(|m| m["body_fat_pct"].as_f64());
    let fat_mass = match (latest_weight, latest_bf) {
        (Some(w), Some(bf)) => Some(w * bf / 100.0),
        _ => None,
    };
    let muscle_mass = match (latest_weight, latest_bf) {
        (Some(w), Some(bf)) => Some(w * (1.0 - bf / 100.0)),
        _ => None,
    };

    let data = serde_json::json!({
        "period": {"since": since_s, "until": until_s, "days": days},
        "measurements": measurements,
        "sleep": sleeps,
        "nutrition": nutrition,
        "overview": {
            "latest_weight_kg": latest_weight,
            "latest_body_fat_pct": latest_bf,
            "fat_mass_kg": fat_mass,
            "lean_mass_kg": muscle_mass,
            "measurement_count": measurements.len(),
            "sleep_nights": sleeps.len(),
        }
    });

    let html = generate_html(&data);
    fs::create_dir_all(output_dir)?;
    let out_path = Path::new(output_dir).join(name);
    fs::write(&out_path, html.as_bytes())?;

    if json {
        print_json(&serde_json::json!({
            "success": true,
            "path": out_path.display().to_string(),
            "days": days,
            "overview": data["overview"],
        }));
    } else if !quiet {
        println!("Wrote HTML dashboard to {}", out_path.display());
    }
    Ok(())
}

fn generate_html(data: &serde_json::Value) -> String {
    let weight_labels: Vec<String> = data["measurements"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["date"].as_str().map(|s| s.to_string()))
        .collect();
    let weights: Vec<Option<f64>> = data["measurements"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|m| m["weight_kg"].as_f64())
        .collect();
    let bf: Vec<Option<f64>> = data["measurements"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|m| m["body_fat_pct"].as_f64())
        .collect();
    let sleep_labels: Vec<String> = data["sleep"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["date"].as_str().map(|s| s.to_string()))
        .collect();
    let sleep_mins: Vec<Option<f64>> = data["sleep"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|m| {
            m["total_sleep_minutes"]
                .as_f64()
                .or_else(|| m["total_sleep_minutes"].as_i64().map(|i| i as f64))
        })
        .collect();
    let nut_labels: Vec<String> = data["nutrition"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["date"].as_str().map(|s| s.to_string()))
        .collect();
    let nut_kcal: Vec<f64> = data["nutrition"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|m| m["energy_kcal"].as_f64().unwrap_or(0.0))
        .collect();
    let nut_protein: Vec<f64> = data["nutrition"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|m| m["protein_g"].as_f64().unwrap_or(0.0))
        .collect();

    let ov = &data["overview"];
    let period = &data["period"];

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>recomplog dashboard</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.1/dist/chart.umd.min.js"></script>
<style>
  :root {{ --bg:#0f1419; --card:#1a2332; --text:#e7ecf3; --muted:#8b9bb4; --accent:#5b9fd4; }}
  * {{ box-sizing: border-box; }}
  body {{ margin:0; font-family: system-ui, sans-serif; background:var(--bg); color:var(--text); padding:1rem; }}
  h1 {{ font-size:1.25rem; margin:0 0 0.25rem; }}
  .sub {{ color:var(--muted); margin-bottom:1rem; font-size:0.9rem; }}
  .cards {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(140px,1fr)); gap:0.75rem; margin-bottom:1.25rem; }}
  .card {{ background:var(--card); border-radius:10px; padding:0.85rem; }}
  .card .label {{ color:var(--muted); font-size:0.75rem; text-transform:uppercase; }}
  .card .value {{ font-size:1.35rem; font-weight:600; margin-top:0.25rem; }}
  .charts {{ display:grid; grid-template-columns:1fr; gap:1rem; }}
  @media(min-width:800px) {{ .charts {{ grid-template-columns:1fr 1fr; }} }}
  .chart-card {{ background:var(--card); border-radius:10px; padding:1rem; }}
  .chart-card h2 {{ font-size:0.95rem; margin:0 0 0.75rem; color:var(--muted); font-weight:500; }}
  .chart-wrap {{ position:relative; height:220px; }}
</style>
</head>
<body>
  <h1>recomplog</h1>
  <p class="sub">{period} days · {since} → {until}</p>
  <div class="cards">
    <div class="card"><div class="label">Weight</div><div class="value">{weight}</div></div>
    <div class="card"><div class="label">Body fat</div><div class="value">{bf}</div></div>
    <div class="card"><div class="label">Fat mass</div><div class="value">{fat}</div></div>
    <div class="card"><div class="label">Lean mass</div><div class="value">{lean}</div></div>
    <div class="card"><div class="label">Measurements</div><div class="value">{mc}</div></div>
    <div class="card"><div class="label">Sleep nights</div><div class="value">{sc}</div></div>
  </div>
  <div class="charts">
    <div class="chart-card"><h2>Weight (kg)</h2><div class="chart-wrap"><canvas id="wChart"></canvas></div></div>
    <div class="chart-card"><h2>Body fat %</h2><div class="chart-wrap"><canvas id="bfChart"></canvas></div></div>
    <div class="chart-card"><h2>Sleep (minutes)</h2><div class="chart-wrap"><canvas id="sChart"></canvas></div></div>
    <div class="chart-card"><h2>Nutrition (kcal / protein g)</h2><div class="chart-wrap"><canvas id="nChart"></canvas></div></div>
  </div>
<script>
const weightLabels = {wl};
const weights = {wv};
const bfVals = {bfv};
const sleepLabels = {sl};
const sleepMins = {sv};
const nutLabels = {nl};
const nutKcal = {nk};
const nutProtein = {np};

function lineChart(id, labels, data, color, label) {{
  new Chart(document.getElementById(id), {{
    type: 'line',
    data: {{ labels, datasets: [{{ label, data, borderColor: color, tension: 0.2, spanGaps: true, pointRadius: 2 }}] }},
    options: {{ responsive: true, maintainAspectRatio: false, plugins: {{ legend: {{ display: false }} }},
      scales: {{ x: {{ ticks: {{ color: '#8b9bb4', maxTicksLimit: 8 }}, grid: {{ color: '#243044' }} }},
                 y: {{ ticks: {{ color: '#8b9bb4' }}, grid: {{ color: '#243044' }} }} }} }}
  }});
}}
lineChart('wChart', weightLabels, weights, '#5b9fd4', 'weight');
lineChart('bfChart', weightLabels, bfVals, '#d4a05b', 'bf%');
new Chart(document.getElementById('sChart'), {{
  type: 'bar',
  data: {{ labels: sleepLabels, datasets: [{{ label: 'sleep min', data: sleepMins, backgroundColor: '#6b8fd4' }}] }},
  options: {{ responsive: true, maintainAspectRatio: false, plugins: {{ legend: {{ display: false }} }},
    scales: {{ x: {{ ticks: {{ color: '#8b9bb4', maxTicksLimit: 8 }}, grid: {{ color: '#243044' }} }},
               y: {{ ticks: {{ color: '#8b9bb4' }}, grid: {{ color: '#243044' }} }} }} }}
}});
new Chart(document.getElementById('nChart'), {{
  type: 'bar',
  data: {{ labels: nutLabels, datasets: [
    {{ label: 'kcal', data: nutKcal, backgroundColor: '#5bd4a0' }},
    {{ label: 'protein g', data: nutProtein, backgroundColor: '#d45b8f' }}
  ] }},
  options: {{ responsive: true, maintainAspectRatio: false,
    scales: {{ x: {{ ticks: {{ color: '#8b9bb4', maxTicksLimit: 8 }}, grid: {{ color: '#243044' }} }},
               y: {{ ticks: {{ color: '#8b9bb4' }}, grid: {{ color: '#243044' }} }} }} }}
}});
</script>
</body>
</html>
"##,
        period = period["days"],
        since = period["since"].as_str().unwrap_or(""),
        until = period["until"].as_str().unwrap_or(""),
        weight = latest_fmt(ov["latest_weight_kg"].as_f64(), "kg"),
        bf = latest_fmt(ov["latest_body_fat_pct"].as_f64(), "%"),
        fat = latest_fmt(ov["fat_mass_kg"].as_f64(), "kg"),
        lean = latest_fmt(ov["lean_mass_kg"].as_f64(), "kg"),
        mc = ov["measurement_count"],
        sc = ov["sleep_nights"],
        wl = serde_json::to_string(&weight_labels).unwrap_or_else(|_| "[]".into()),
        wv = serde_json::to_string(&weights).unwrap_or_else(|_| "[]".into()),
        bfv = serde_json::to_string(&bf).unwrap_or_else(|_| "[]".into()),
        sl = serde_json::to_string(&sleep_labels).unwrap_or_else(|_| "[]".into()),
        sv = serde_json::to_string(&sleep_mins).unwrap_or_else(|_| "[]".into()),
        nl = serde_json::to_string(&nut_labels).unwrap_or_else(|_| "[]".into()),
        nk = serde_json::to_string(&nut_kcal).unwrap_or_else(|_| "[]".into()),
        np = serde_json::to_string(&nut_protein).unwrap_or_else(|_| "[]".into()),
    )
}

fn latest_fmt(v: Option<f64>, unit: &str) -> String {
    match v {
        Some(x) => format!("{:.1} {}", x, unit),
        None => "—".into(),
    }
}
