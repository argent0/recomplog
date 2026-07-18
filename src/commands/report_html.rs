//! Self-contained Chart.js HTML dashboard (`report html`).

use crate::db;
use crate::hr_zones::median_f64;
use crate::stats::{
    metric_trend_from_points, regression_with_ci_at_points, DataPoint, MetricTrend, TrendDirection,
    BF_FLAT_PCT_PER_DAY, WEIGHT_FLAT_KG_PER_DAY,
};
use crate::utils::print_json;
use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::Path;

/// Flat band for skeletal muscle % (same scale as body-fat %/day).
const MUSCLE_PCT_FLAT_PER_DAY: f64 = BF_FLAT_PCT_PER_DAY;
/// Flat band for muscle mass kg (same scale as weight kg/day).
const MUSCLE_KG_FLAT_PER_DAY: f64 = WEIGHT_FLAT_KG_PER_DAY;

#[derive(Debug, Clone)]
struct MeasPoint {
    date: String,
    weight_kg: Option<f64>,
    body_fat_pct: Option<f64>,
    skeletal_muscle_pct: Option<f64>,
    lean_mass_kg: Option<f64>,
    muscle_mass_kg: Option<f64>,
    resting_metabolism_kcal: Option<i64>,
}

#[derive(Debug, Clone)]
struct SleepPoint {
    date: String,
    total_sleep_minutes: Option<i64>,
    rem_minutes: Option<i64>,
    deep_minutes: Option<i64>,
    light_minutes: Option<i64>,
    awake_minutes: Option<i64>,
    sleep_efficiency_pct: Option<f64>,
    sleep_score: Option<i64>,
}

#[derive(Debug, Clone)]
struct NutDay {
    date: String,
    energy_kcal: f64,
    protein_g: f64,
    carbohydrates_g: f64,
    fat_g: f64,
    fiber_g: f64,
    sugars_g: f64,
}

#[derive(Debug, Clone)]
struct TrainingSummary {
    workout_count: i64,
    days_trained: i64,
    set_count: i64,
    total_volume: f64,
}

/// One body metric ready for scatter + regression + CI in Chart.js.
#[derive(Debug, Clone)]
struct RegSeries {
    labels: Vec<String>,
    values: Vec<Option<f64>>,
    reg_y: Vec<Option<f64>>,
    ci_lo: Vec<Option<f64>>,
    ci_hi: Vec<Option<f64>>,
    r_squared: Option<f64>,
}

fn derive_composition(w: Option<f64>, bf: Option<f64>) -> (Option<f64>, Option<f64>) {
    match (w, bf) {
        (Some(w), Some(bf)) => {
            let fat = w * bf / 100.0;
            let lean = w * (1.0 - bf / 100.0);
            (Some(fat), Some(lean))
        }
        _ => (None, None),
    }
}

fn derive_muscle_mass(w: Option<f64>, sm: Option<f64>) -> Option<f64> {
    match (w, sm) {
        (Some(w), Some(sm)) => Some(w * sm / 100.0),
        _ => None,
    }
}

fn night_has_any_stage(s: &SleepPoint) -> bool {
    s.rem_minutes.is_some()
        || s.deep_minutes.is_some()
        || s.light_minutes.is_some()
        || s.awake_minutes.is_some()
}

fn stage_or_zero(v: Option<i64>) -> f64 {
    v.unwrap_or(0) as f64
}

fn fetch_html_measurements(conn: &Connection, since: &str, until: &str) -> Result<Vec<MeasPoint>> {
    // One point per calendar day (last sample by created_at, then id).
    let mut stmt = conn.prepare(
        "SELECT date, weight_kg, body_fat_pct, skeletal_muscle_pct, resting_metabolism_kcal
         FROM measurements m
         WHERE date >= ?1 AND date <= ?2
           AND id = (
             SELECT id FROM measurements m2
             WHERE m2.date = m.date
             ORDER BY m2.created_at DESC, m2.id DESC LIMIT 1
           )
         ORDER BY date ASC",
    )?;
    let rows = stmt
        .query_map(params![since, until], |r| {
            let weight_kg: Option<f64> = r.get(1)?;
            let body_fat_pct: Option<f64> = r.get(2)?;
            let skeletal_muscle_pct: Option<f64> = r.get(3)?;
            let resting_metabolism_kcal: Option<i64> = r.get(4)?;
            let (_, lean_mass_kg) = derive_composition(weight_kg, body_fat_pct);
            let muscle_mass_kg = derive_muscle_mass(weight_kg, skeletal_muscle_pct);
            Ok(MeasPoint {
                date: r.get(0)?,
                weight_kg,
                body_fat_pct,
                skeletal_muscle_pct,
                lean_mass_kg,
                muscle_mass_kg,
                resting_metabolism_kcal,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn fetch_html_sleep(conn: &Connection, since: &str, until: &str) -> Result<Vec<SleepPoint>> {
    // One night per wake-up date (last sample by created_at, then id).
    let mut stmt = conn.prepare(
        "SELECT date, total_sleep_minutes, rem_minutes, deep_minutes, light_minutes,
                awake_minutes, sleep_efficiency_pct, sleep_score
         FROM sleep s
         WHERE date >= ?1 AND date <= ?2
           AND id = (
             SELECT id FROM sleep s2
             WHERE s2.date = s.date
             ORDER BY s2.created_at DESC, s2.id DESC LIMIT 1
           )
         ORDER BY date ASC",
    )?;
    let rows = stmt
        .query_map(params![since, until], |r| {
            Ok(SleepPoint {
                date: r.get(0)?,
                total_sleep_minutes: r.get(1)?,
                rem_minutes: r.get(2)?,
                deep_minutes: r.get(3)?,
                light_minutes: r.get(4)?,
                awake_minutes: r.get(5)?,
                sleep_efficiency_pct: r.get(6)?,
                sleep_score: r.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn fetch_html_nutrition_daily(conn: &Connection, since: &str, until: &str) -> Result<Vec<NutDay>> {
    // Aggregate in Rust so discrete units (bar, cup, serving) use the same
    // consumption_scale rules as `report nutrition` / `report brief`.
    // Macros come from the effective (merge keeper) product.
    let mut stmt = conn.prepare(
        "SELECT date(c.consumed_at, 'localtime') AS d,
                c.quantity, c.unit, c.product_id
         FROM consumptions c
         WHERE date(c.consumed_at, 'localtime') >= date(?1)
           AND date(c.consumed_at, 'localtime') <= date(?2)
         ORDER BY d ASC",
    )?;
    let mut by_day: std::collections::BTreeMap<String, NutDay> = std::collections::BTreeMap::new();
    let mut rows = stmt.query(params![since, until])?;
    while let Some(r) = rows.next()? {
        let date: String = r.get(0)?;
        let qty: f64 = r.get(1)?;
        let unit: Option<String> = r.get(2)?;
        let logged_pid: i64 = r.get(3)?;
        let effective =
            crate::product_resolve::resolve_effective_product_id(conn, logged_pid)?;
        let nutrition: Option<(f64, String, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>)> =
            conn.query_row(
                "SELECT reference_quantity, reference_unit,
                        energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g
                 FROM product_nutritions WHERE product_id = ?1",
                [effective],
                |nr| {
                    Ok((
                        nr.get(0)?,
                        nr.get(1)?,
                        nr.get(2)?,
                        nr.get(3)?,
                        nr.get(4)?,
                        nr.get(5)?,
                        nr.get(6)?,
                        nr.get(7)?,
                    ))
                },
            )
            .optional()?;
        let scale = match &nutrition {
            Some((rq, ru, ..)) => {
                crate::nutrition_units::consumption_scale(qty, *rq, unit.as_deref(), ru)
            }
            None => 0.0,
        };
        let day = by_day.entry(date.clone()).or_insert(NutDay {
            date,
            energy_kcal: 0.0,
            protein_g: 0.0,
            carbohydrates_g: 0.0,
            fat_g: 0.0,
            fiber_g: 0.0,
            sugars_g: 0.0,
        });
        if let Some((_, _, energy, protein, carbs, fat, fiber, sugars)) = nutrition {
            if let Some(v) = energy {
                day.energy_kcal += v * scale;
            }
            if let Some(v) = protein {
                day.protein_g += v * scale;
            }
            if let Some(v) = carbs {
                day.carbohydrates_g += v * scale;
            }
            if let Some(v) = fat {
                day.fat_g += v * scale;
            }
            if let Some(v) = fiber {
                day.fiber_g += v * scale;
            }
            if let Some(v) = sugars {
                day.sugars_g += v * scale;
            }
        }
    }
    Ok(by_day.into_values().collect())
}

fn fetch_html_training(conn: &Connection, since: &str, until: &str) -> Result<TrainingSummary> {
    // Workouts store UTC `started_at` (`db::now_utc`); HTML period is local calendar
    // days — convert with SQLite `localtime` so evening sessions still land on today.
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
        // Body-mass CASE matches workout_stats::handle_volume.
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

    Ok(TrainingSummary {
        workout_count,
        days_trained,
        set_count,
        total_volume,
    })
}

fn parse_ymd(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// Axis tick labels: never year; omit month when all dates share one calendar month.
fn short_date_labels(dates: &[String]) -> Vec<String> {
    let parsed: Vec<Option<NaiveDate>> = dates.iter().map(|d| parse_ymd(d)).collect();
    let valid: Vec<NaiveDate> = parsed.iter().copied().flatten().collect();
    let same_month = match (valid.iter().min(), valid.iter().max()) {
        (Some(a), Some(b)) => a.year() == b.year() && a.month() == b.month(),
        _ => true,
    };
    dates
        .iter()
        .zip(parsed.iter())
        .map(|(raw, d)| match d {
            Some(date) if same_month => format!("{}", date.day()),
            Some(date) => format!("{}/{}", date.month(), date.day()),
            None => raw.clone(),
        })
        .collect()
}

fn series_points(dates: &[String], values: &[Option<f64>], origin: NaiveDate) -> Vec<DataPoint> {
    dates
        .iter()
        .zip(values.iter())
        .filter_map(|(d, y)| {
            let date = parse_ymd(d)?;
            let y = (*y)?;
            Some(DataPoint {
                x: (date - origin).num_days() as f64,
                y,
            })
        })
        .collect()
}

/// Build scatter series from non-null samples only, with OLS + 95% CI at each point.
fn build_reg_series(dates: &[String], values: &[Option<f64>], origin: NaiveDate) -> RegSeries {
    let mut full_dates = Vec::new();
    let mut ys = Vec::new();
    for (d, v) in dates.iter().zip(values.iter()) {
        if let Some(y) = v {
            full_dates.push(d.clone());
            ys.push(*y);
        }
    }
    let labels = short_date_labels(&full_dates);
    let values_opt: Vec<Option<f64>> = ys.iter().copied().map(Some).collect();

    if full_dates.len() < 2 {
        return RegSeries {
            labels,
            values: values_opt,
            reg_y: vec![None; full_dates.len()],
            ci_lo: vec![None; full_dates.len()],
            ci_hi: vec![None; full_dates.len()],
            r_squared: None,
        };
    }

    let pts = series_points(&full_dates, &values_opt, origin);
    match regression_with_ci_at_points(&pts) {
        Some((reg, band)) => RegSeries {
            labels,
            values: values_opt,
            reg_y: band.iter().map(|c| Some(c.y)).collect(),
            ci_lo: band.iter().map(|c| Some(c.lower)).collect(),
            ci_hi: band.iter().map(|c| Some(c.upper)).collect(),
            r_squared: Some(reg.r_squared),
        },
        None => RegSeries {
            labels,
            values: values_opt,
            reg_y: vec![None; full_dates.len()],
            ci_lo: vec![None; full_dates.len()],
            ci_hi: vec![None; full_dates.len()],
            r_squared: None,
        },
    }
}

fn day_distance(a: NaiveDate, b: NaiveDate) -> i64 {
    (a - b).num_days().unsigned_abs() as i64
}

/// Nearest measurement (by calendar day) where `extract` returns Some.
fn nearest_meas_value<T: Copy>(
    measurements: &[MeasPoint],
    date_s: &str,
    extract: impl Fn(&MeasPoint) -> Option<T>,
) -> Option<T> {
    let target = parse_ymd(date_s)?;
    let mut best: Option<(i64, T)> = None;
    for m in measurements {
        let Some(v) = extract(m) else {
            continue;
        };
        let Some(md) = parse_ymd(&m.date) else {
            continue;
        };
        let dist = day_distance(md, target);
        match best {
            None => best = Some((dist, v)),
            Some((bd, _)) if dist < bd => best = Some((dist, v)),
            _ => {}
        }
    }
    best.map(|(_, v)| v)
}

fn trend_css_class(dir: TrendDirection) -> &'static str {
    match dir {
        TrendDirection::Up => "trend-up",
        TrendDirection::Down => "trend-down",
        TrendDirection::Flat | TrendDirection::InsufficientData => "trend-flat",
    }
}

fn latest_fmt(v: Option<f64>, unit: &str) -> String {
    match v {
        Some(x) => format!("{:.1} {}", x, unit),
        None => "—".into(),
    }
}

fn median_of_opts(values: &[Option<f64>]) -> Option<f64> {
    let vals: Vec<f64> = values.iter().copied().flatten().collect();
    median_f64(&vals)
}

/// One overview card: label, N-day median (primary), trend, last value.
fn metric_card_html(
    label: &str,
    median: Option<f64>,
    last: Option<f64>,
    unit: &str,
    trend: &MetricTrend,
) -> String {
    format!(
        r#"    <div class="card"><div class="label">{label}</div><div class="value">{median}</div><div class="trend {trend_cls}">{trend_label}</div><div class="last">last {last}</div></div>
"#,
        label = label,
        median = latest_fmt(median, unit),
        last = latest_fmt(last, unit),
        trend_label = trend.label,
        trend_cls = trend_css_class(trend.direction),
    )
}

fn j<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

fn reg_title(name: &str, r2: Option<f64>) -> String {
    match r2 {
        Some(r) => format!("{} · R² = {:.3}", name, r),
        None => name.to_string(),
    }
}

pub fn handle_html(
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

    let measurements = fetch_html_measurements(&conn, &since_s, &until_s)?;
    let sleeps = fetch_html_sleep(&conn, &since_s, &until_s)?;
    let nutrition = fetch_html_nutrition_daily(&conn, &since_s, &until_s)?;
    let training = fetch_html_training(&conn, &since_s, &until_s)?;

    let weight_labels: Vec<String> = measurements.iter().map(|m| m.date.clone()).collect();
    let weights: Vec<Option<f64>> = measurements.iter().map(|m| m.weight_kg).collect();
    let bf: Vec<Option<f64>> = measurements.iter().map(|m| m.body_fat_pct).collect();
    let muscle_pct: Vec<Option<f64>> = measurements.iter().map(|m| m.skeletal_muscle_pct).collect();
    let muscle_kg: Vec<Option<f64>> = measurements.iter().map(|m| m.muscle_mass_kg).collect();

    let weight_trend = metric_trend_from_points(
        &series_points(&weight_labels, &weights, since),
        "kg",
        WEIGHT_FLAT_KG_PER_DAY,
    );
    let body_fat_trend = metric_trend_from_points(
        &series_points(&weight_labels, &bf, since),
        "%",
        BF_FLAT_PCT_PER_DAY,
    );
    let muscle_pct_trend = metric_trend_from_points(
        &series_points(&weight_labels, &muscle_pct, since),
        "%",
        MUSCLE_PCT_FLAT_PER_DAY,
    );
    let muscle_kg_trend = metric_trend_from_points(
        &series_points(&weight_labels, &muscle_kg, since),
        "kg",
        MUSCLE_KG_FLAT_PER_DAY,
    );

    let latest_weight = measurements.iter().rev().find_map(|m| m.weight_kg);
    let latest_bf = measurements.iter().rev().find_map(|m| m.body_fat_pct);
    let latest_muscle_pct = measurements
        .iter()
        .rev()
        .find_map(|m| m.skeletal_muscle_pct);
    let latest_muscle_kg = measurements.iter().rev().find_map(|m| m.muscle_mass_kg);

    let median_weight = median_of_opts(&weights);
    let median_bf = median_of_opts(&bf);
    let median_muscle_pct = median_of_opts(&muscle_pct);
    let median_muscle_kg = median_of_opts(&muscle_kg);

    let weight_series = build_reg_series(&weight_labels, &weights, since);
    let bf_series = build_reg_series(&weight_labels, &bf, since);
    let mm_pct_series = build_reg_series(&weight_labels, &muscle_pct, since);
    let mm_kg_series = build_reg_series(&weight_labels, &muscle_kg, since);

    let overview = serde_json::json!({
        "median_weight_kg": median_weight,
        "latest_weight_kg": latest_weight,
        "median_body_fat_pct": median_bf,
        "latest_body_fat_pct": latest_bf,
        "median_muscle_pct": median_muscle_pct,
        "latest_muscle_pct": latest_muscle_pct,
        "median_muscle_mass_kg": median_muscle_kg,
        "latest_muscle_mass_kg": latest_muscle_kg,
        "measurement_count": measurements.len(),
        "sleep_nights": sleeps.len(),
        "weight_trend": weight_trend,
        "body_fat_trend": body_fat_trend,
        "muscle_pct_trend": muscle_pct_trend,
        "muscle_mass_trend": muscle_kg_trend,
        "training": {
            "workout_count": training.workout_count,
            "days_trained": training.days_trained,
            "set_count": training.set_count,
            "total_volume": training.total_volume,
        },
    });

    let html = generate_html(
        days,
        &since_s,
        &until_s,
        &weight_series,
        &bf_series,
        &mm_pct_series,
        &mm_kg_series,
        &measurements,
        &sleeps,
        &nutrition,
        median_weight,
        latest_weight,
        median_bf,
        latest_bf,
        median_muscle_pct,
        latest_muscle_pct,
        median_muscle_kg,
        latest_muscle_kg,
        &weight_trend,
        &body_fat_trend,
        &muscle_pct_trend,
        &muscle_kg_trend,
    );

    fs::create_dir_all(output_dir)?;
    let out_path = Path::new(output_dir).join(name);
    fs::write(&out_path, html.as_bytes())?;

    if json {
        print_json(&serde_json::json!({
            "success": true,
            "path": out_path.display().to_string(),
            "days": days,
            "overview": overview,
        }));
    } else if !quiet {
        println!("Wrote HTML dashboard to {}", out_path.display());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_html(
    days: u32,
    since: &str,
    until: &str,
    weight_series: &RegSeries,
    bf_series: &RegSeries,
    mm_pct_series: &RegSeries,
    mm_kg_series: &RegSeries,
    measurements: &[MeasPoint],
    sleeps: &[SleepPoint],
    nutrition: &[NutDay],
    median_weight: Option<f64>,
    latest_weight: Option<f64>,
    median_bf: Option<f64>,
    latest_bf: Option<f64>,
    median_muscle_pct: Option<f64>,
    latest_muscle_pct: Option<f64>,
    median_muscle_kg: Option<f64>,
    latest_muscle_kg: Option<f64>,
    weight_trend: &MetricTrend,
    body_fat_trend: &MetricTrend,
    muscle_pct_trend: &MetricTrend,
    muscle_kg_trend: &MetricTrend,
) -> String {
    let sleep_iso: Vec<String> = sleeps.iter().map(|s| s.date.clone()).collect();
    let sleep_labels = short_date_labels(&sleep_iso);
    let sleep_mins: Vec<Option<f64>> = sleeps
        .iter()
        .map(|s| s.total_sleep_minutes.map(|m| m as f64))
        .collect();
    let sleep_eff: Vec<Option<f64>> = sleeps.iter().map(|s| s.sleep_efficiency_pct).collect();
    let sleep_score: Vec<Option<f64>> = sleeps
        .iter()
        .map(|s| s.sleep_score.map(|m| m as f64))
        .collect();

    // Stage chart: only nights with any stage; missing stages → 0.
    let stage_nights: Vec<&SleepPoint> = sleeps.iter().filter(|s| night_has_any_stage(s)).collect();
    let stage_iso: Vec<String> = stage_nights.iter().map(|s| s.date.clone()).collect();
    let stage_labels = short_date_labels(&stage_iso);
    let stage_deep: Vec<f64> = stage_nights
        .iter()
        .map(|s| stage_or_zero(s.deep_minutes))
        .collect();
    let stage_light: Vec<f64> = stage_nights
        .iter()
        .map(|s| stage_or_zero(s.light_minutes))
        .collect();
    let stage_rem: Vec<f64> = stage_nights
        .iter()
        .map(|s| stage_or_zero(s.rem_minutes))
        .collect();
    let stage_awake: Vec<f64> = stage_nights
        .iter()
        .map(|s| stage_or_zero(s.awake_minutes))
        .collect();
    let show_stages = !stage_nights.is_empty();

    let show_quality = sleeps
        .iter()
        .any(|s| s.sleep_efficiency_pct.is_some() || s.sleep_score.is_some());

    let nut_iso: Vec<String> = nutrition.iter().map(|n| n.date.clone()).collect();
    let nut_labels = short_date_labels(&nut_iso);
    let nut_kcal: Vec<f64> = nutrition.iter().map(|n| n.energy_kcal).collect();
    let nut_protein: Vec<f64> = nutrition.iter().map(|n| n.protein_g).collect();
    let nut_carbs: Vec<f64> = nutrition.iter().map(|n| n.carbohydrates_g).collect();
    let nut_fat: Vec<f64> = nutrition.iter().map(|n| n.fat_g).collect();
    let nut_fiber: Vec<f64> = nutrition.iter().map(|n| n.fiber_g).collect();
    let nut_sugars: Vec<f64> = nutrition.iter().map(|n| n.sugars_g).collect();
    let show_nutrition = !nutrition.is_empty();

    // BMR line + bar colors vs resting metabolism (nearest measurement with value).
    let nut_bmr: Vec<Option<f64>> = nutrition
        .iter()
        .map(|n| {
            nearest_meas_value(measurements, &n.date, |m| {
                m.resting_metabolism_kcal.map(|v| v as f64)
            })
        })
        .collect();
    let has_bmr = nut_bmr.iter().any(|v| v.is_some());
    let nut_kcal_colors: Vec<String> = nutrition
        .iter()
        .zip(nut_bmr.iter())
        .map(|(n, bmr)| match bmr {
            Some(b) if n.energy_kcal > *b => "#d45b5b".to_string(),
            Some(_) => "#5bd4a0".to_string(),
            None => "#5bd4a0".to_string(),
        })
        .collect();

    // % calories by Atwater factors.
    let mut pct_protein = Vec::with_capacity(nutrition.len());
    let mut pct_carbs = Vec::with_capacity(nutrition.len());
    let mut pct_fat = Vec::with_capacity(nutrition.len());
    for n in nutrition {
        let p_kcal = n.protein_g * 4.0;
        let c_kcal = n.carbohydrates_g * 4.0;
        let f_kcal = n.fat_g * 9.0;
        let sum = p_kcal + c_kcal + f_kcal;
        if sum > 0.0 {
            pct_protein.push(Some(100.0 * p_kcal / sum));
            pct_carbs.push(Some(100.0 * c_kcal / sum));
            pct_fat.push(Some(100.0 * f_kcal / sum));
        } else {
            pct_protein.push(None);
            pct_carbs.push(None);
            pct_fat.push(None);
        }
    }

    // Protein ratios (g/kg) using nearest body composition.
    let prot_lean: Vec<Option<f64>> = nutrition
        .iter()
        .map(|n| {
            let lean = nearest_meas_value(measurements, &n.date, |m| m.lean_mass_kg)?;
            if lean > 0.0 {
                Some(n.protein_g / lean)
            } else {
                None
            }
        })
        .collect();
    let prot_muscle: Vec<Option<f64>> = nutrition
        .iter()
        .map(|n| {
            let mm = nearest_meas_value(measurements, &n.date, |m| m.muscle_mass_kg)?;
            if mm > 0.0 {
                Some(n.protein_g / mm)
            } else {
                None
            }
        })
        .collect();

    let dash = serde_json::json!({
        "weight": {
            "labels": weight_series.labels,
            "values": weight_series.values,
            "reg": weight_series.reg_y,
            "ciLo": weight_series.ci_lo,
            "ciHi": weight_series.ci_hi,
        },
        "bf": {
            "labels": bf_series.labels,
            "values": bf_series.values,
            "reg": bf_series.reg_y,
            "ciLo": bf_series.ci_lo,
            "ciHi": bf_series.ci_hi,
        },
        "mmPct": {
            "labels": mm_pct_series.labels,
            "values": mm_pct_series.values,
            "reg": mm_pct_series.reg_y,
            "ciLo": mm_pct_series.ci_lo,
            "ciHi": mm_pct_series.ci_hi,
        },
        "mmKg": {
            "labels": mm_kg_series.labels,
            "values": mm_kg_series.values,
            "reg": mm_kg_series.reg_y,
            "ciLo": mm_kg_series.ci_lo,
            "ciHi": mm_kg_series.ci_hi,
        },
        "sleepLabels": sleep_labels,
        "sleepMins": sleep_mins,
        "sleepEff": sleep_eff,
        "sleepScore": sleep_score,
        "stageLabels": stage_labels,
        "stageDeep": stage_deep,
        "stageLight": stage_light,
        "stageRem": stage_rem,
        "stageAwake": stage_awake,
        "nutLabels": nut_labels,
        "nutKcal": nut_kcal,
        "nutKcalColors": nut_kcal_colors,
        "nutBmr": nut_bmr,
        "hasBmr": has_bmr,
        "nutProtein": nut_protein,
        "nutCarbs": nut_carbs,
        "nutFat": nut_fat,
        "nutFiber": nut_fiber,
        "nutSugars": nut_sugars,
        "pctProtein": pct_protein,
        "pctCarbs": pct_carbs,
        "pctFat": pct_fat,
        "protLean": prot_lean,
        "protMuscle": prot_muscle,
    });

    let body_cards = format!(
        "{}{}{}{}",
        metric_card_html("Weight", median_weight, latest_weight, "kg", weight_trend),
        metric_card_html("Body fat", median_bf, latest_bf, "%", body_fat_trend),
        metric_card_html(
            "Muscle %",
            median_muscle_pct,
            latest_muscle_pct,
            "%",
            muscle_pct_trend
        ),
        metric_card_html(
            "Muscle mass",
            median_muscle_kg,
            latest_muscle_kg,
            "kg",
            muscle_kg_trend
        ),
    );

    let mut chart_cards = String::new();
    chart_cards.push_str(&format!(
        r#"    <div class="chart-card"><h2>{}</h2><div class="chart-wrap"><canvas id="wChart"></canvas></div></div>
    <div class="chart-card"><h2>{}</h2><div class="chart-wrap"><canvas id="bfChart"></canvas></div></div>
    <div class="chart-card"><h2>{}</h2><div class="chart-wrap"><canvas id="mmPctChart"></canvas></div></div>
    <div class="chart-card"><h2>{}</h2><div class="chart-wrap"><canvas id="mmKgChart"></canvas></div></div>
    <div class="chart-card"><h2>Sleep total (min)</h2><div class="chart-wrap"><canvas id="sChart"></canvas></div></div>
"#,
        reg_title("Weight (kg)", weight_series.r_squared),
        reg_title("Body fat %", bf_series.r_squared),
        reg_title("Muscle mass %", mm_pct_series.r_squared),
        reg_title("Muscle mass (kg)", mm_kg_series.r_squared),
    ));
    if show_stages {
        chart_cards.push_str(
            r#"    <div class="chart-card"><h2>Sleep stages (min)</h2><div class="chart-wrap"><canvas id="ssChart"></canvas></div></div>
"#,
        );
    }
    if show_quality {
        chart_cards.push_str(
            r#"    <div class="chart-card"><h2>Sleep quality</h2><div class="chart-wrap"><canvas id="sqChart"></canvas></div></div>
"#,
        );
    }
    if show_nutrition {
        chart_cards.push_str(
            r#"    <div class="chart-card"><h2>Energy (kcal)</h2><div class="chart-wrap"><canvas id="nKcalChart"></canvas></div></div>
    <div class="chart-card"><h2>Macros (g)</h2><div class="chart-wrap"><canvas id="nMacroChart"></canvas></div></div>
    <div class="chart-card"><h2>Calories by source (%)</h2><div class="chart-wrap"><canvas id="nCalSourceChart"></canvas></div></div>
    <div class="chart-card"><h2>Protein / lean mass (g/kg)</h2><div class="chart-wrap"><canvas id="nProtLeanChart"></canvas></div></div>
    <div class="chart-card"><h2>Protein / muscle mass (g/kg)</h2><div class="chart-wrap"><canvas id="nProtMusChart"></canvas></div></div>
"#,
        );
    }

    let mut chart_js = String::from(
        r#"
regressionChart('wChart', D.weight, '#5b9fd4');
regressionChart('bfChart', D.bf, '#d4a05b');
regressionChart('mmPctChart', D.mmPct, '#7bd45b');
regressionChart('mmKgChart', D.mmKg, '#5bd4a0');
new Chart(document.getElementById('sChart'), {
  type: 'bar',
  data: { labels: D.sleepLabels, datasets: [{ label: 'sleep min', data: D.sleepMins, backgroundColor: '#6b8fd4' }] },
  options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { display: false } },
    scales: { x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
               y: { ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } } } }
});
"#,
    );

    if show_stages {
        chart_js.push_str(
            r#"
new Chart(document.getElementById('ssChart'), {
  type: 'bar',
  data: {
    labels: D.stageLabels,
    datasets: [
      { label: 'Deep', data: D.stageDeep, backgroundColor: '#5b7fd4', stack: 's' },
      { label: 'Light', data: D.stageLight, backgroundColor: '#7b9fd4', stack: 's' },
      { label: 'REM', data: D.stageRem, backgroundColor: '#d4a05b', stack: 's' },
      { label: 'Awake', data: D.stageAwake, backgroundColor: '#d45b5b', stack: 's' }
    ]
  },
  options: {
    responsive: true, maintainAspectRatio: false,
    scales: {
      x: { stacked: true, ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
      y: { stacked: true, ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } }
    }
  }
});
"#,
        );
    }

    if show_quality {
        chart_js.push_str(
            r#"
new Chart(document.getElementById('sqChart'), {
  type: 'line',
  data: {
    labels: D.sleepLabels,
    datasets: [
      { label: 'Efficiency %', data: D.sleepEff, borderColor: '#5bd4a0', tension: 0.2, spanGaps: true, pointRadius: 2 },
      { label: 'Score', data: D.sleepScore, borderColor: '#d4a05b', tension: 0.2, spanGaps: true, pointRadius: 2 }
    ]
  },
  options: {
    responsive: true, maintainAspectRatio: false,
    plugins: { legend: { display: true, labels: { color: '#8b9bb4' } } },
    scales: {
      x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
      y: { min: 0, max: 100, ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } }
    }
  }
});
"#,
        );
    }

    if show_nutrition {
        chart_js.push_str(
            r#"
(function() {
  const kcalDatasets = [
    { label: 'kcal', data: D.nutKcal, backgroundColor: D.nutKcalColors, order: 2 }
  ];
  if (D.hasBmr) {
    kcalDatasets.push({
      label: 'Resting metabolism',
      data: D.nutBmr,
      type: 'line',
      borderColor: '#e7ecf3',
      backgroundColor: 'transparent',
      pointRadius: 0,
      borderWidth: 2,
      borderDash: [6, 4],
      spanGaps: true,
      order: 1
    });
  }
  new Chart(document.getElementById('nKcalChart'), {
    type: 'bar',
    data: { labels: D.nutLabels, datasets: kcalDatasets },
    options: { responsive: true, maintainAspectRatio: false,
      plugins: { legend: { display: D.hasBmr, labels: { color: '#8b9bb4' } } },
      scales: { x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
                 y: { ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } } } }
  });
})();
new Chart(document.getElementById('nMacroChart'), {
  type: 'bar',
  data: { labels: D.nutLabels, datasets: [
    { label: 'protein g', data: D.nutProtein, backgroundColor: '#d45b8f' },
    { label: 'carbs g', data: D.nutCarbs, backgroundColor: '#d4a05b' },
    { label: 'fat g', data: D.nutFat, backgroundColor: '#5b9fd4' },
    { label: 'fiber g', data: D.nutFiber, backgroundColor: '#7bd45b' },
    { label: 'sugars g', data: D.nutSugars, backgroundColor: '#d47b5b' }
  ]},
  options: { responsive: true, maintainAspectRatio: false,
    scales: { x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
               y: { ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } } } }
});
new Chart(document.getElementById('nCalSourceChart'), {
  type: 'bar',
  data: { labels: D.nutLabels, datasets: [
    { label: 'protein %', data: D.pctProtein, backgroundColor: '#d45b8f', stack: 'c' },
    { label: 'carbs %', data: D.pctCarbs, backgroundColor: '#d4a05b', stack: 'c' },
    { label: 'fat %', data: D.pctFat, backgroundColor: '#5b9fd4', stack: 'c' }
  ]},
  options: { responsive: true, maintainAspectRatio: false,
    scales: {
      x: { stacked: true, ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
      y: { stacked: true, min: 0, max: 100, ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } }
    }
  }
});
new Chart(document.getElementById('nProtLeanChart'), {
  type: 'bar',
  data: { labels: D.nutLabels, datasets: [
    { label: 'g/kg lean', data: D.protLean, backgroundColor: '#d45b8f' }
  ]},
  options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { display: false } },
    scales: { x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
               y: { ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } } } }
});
new Chart(document.getElementById('nProtMusChart'), {
  type: 'bar',
  data: { labels: D.nutLabels, datasets: [
    { label: 'g/kg muscle', data: D.protMuscle, backgroundColor: '#5bd4a0' }
  ]},
  options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { display: false } },
    scales: { x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
               y: { ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } } } }
});
"#,
        );
    }

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
  .card .trend {{ font-size:0.75rem; margin-top:0.2rem; }}
  .card .last {{ font-size:0.7rem; color:var(--muted); margin-top:0.15rem; }}
  .trend-up {{ color: #81c784; }}
  .trend-down {{ color: #ef5350; }}
  .trend-flat {{ color: var(--muted); }}
  .charts {{ display:grid; grid-template-columns:1fr; gap:1rem; }}
  @media(min-width:800px) {{ .charts {{ grid-template-columns:1fr 1fr; }} }}
  .chart-card {{ background:var(--card); border-radius:10px; padding:1rem; }}
  .chart-card h2 {{ font-size:0.95rem; margin:0 0 0.75rem; color:var(--muted); font-weight:500; }}
  .chart-wrap {{ position:relative; height:220px; }}
</style>
</head>
<body>
  <h1>recomplog</h1>
  <p class="sub">{days} days · {since} → {until}</p>
  <div class="cards">
{body_cards}  </div>
  <div class="charts">
{chart_cards}  </div>
<script>
const D = {dash};
function regressionChart(id, s, color) {{
  const el = document.getElementById(id);
  if (!el) return;
  const datasets = [
    {{ label: 'Observed', data: s.values, borderColor: color, backgroundColor: color,
      showLine: false, pointRadius: 4, pointHoverRadius: 5, order: 3 }},
    {{ label: '95% CI Upper', data: s.ciHi, borderColor: 'rgba(129,199,132,0.35)',
      backgroundColor: 'rgba(129,199,132,0.15)', fill: '+1', tension: 0.1,
      pointRadius: 0, borderDash: [4, 4], order: 1 }},
    {{ label: '95% CI Lower', data: s.ciLo, borderColor: 'rgba(129,199,132,0.35)',
      backgroundColor: 'rgba(129,199,132,0.15)', fill: false, tension: 0.1,
      pointRadius: 0, borderDash: [4, 4], order: 1 }},
    {{ label: 'Regression', data: s.reg, borderColor: '#81c784', backgroundColor: 'transparent',
      tension: 0.1, pointRadius: 0, order: 2 }}
  ];
  new Chart(el, {{
    type: 'line',
    data: {{ labels: s.labels, datasets }},
    options: {{
      responsive: true, maintainAspectRatio: false,
      plugins: {{
        legend: {{ display: false }},
        tooltip: {{ mode: 'index', intersect: false }}
      }},
      interaction: {{ mode: 'nearest', axis: 'x', intersect: false }},
      scales: {{
        x: {{ ticks: {{ color: '#8b9bb4', maxTicksLimit: 8 }}, grid: {{ color: '#243044' }} }},
        y: {{ ticks: {{ color: '#8b9bb4' }}, grid: {{ color: '#243044' }} }}
      }}
    }}
  }});
}}
{chart_js}
</script>
</body>
</html>
"##,
        days = days,
        since = since,
        until = until,
        body_cards = body_cards,
        chart_cards = chart_cards,
        dash = j(&dash),
        chart_js = chart_js,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_both_present() {
        let (fat, lean) = derive_composition(Some(80.0), Some(25.0));
        assert!((fat.unwrap() - 20.0).abs() < 1e-9);
        assert!((lean.unwrap() - 60.0).abs() < 1e-9);
    }

    #[test]
    fn composition_missing() {
        assert_eq!(derive_composition(Some(80.0), None), (None, None));
        assert_eq!(derive_composition(None, Some(20.0)), (None, None));
    }

    #[test]
    fn muscle_mass_derived() {
        assert!((derive_muscle_mass(Some(80.0), Some(40.0)).unwrap() - 32.0).abs() < 1e-9);
        assert!(derive_muscle_mass(Some(80.0), None).is_none());
    }

    #[test]
    fn median_of_opts_even_and_sparse() {
        assert!((median_of_opts(&[Some(82.0), Some(81.0)]).unwrap() - 81.5).abs() < 1e-9);
        assert_eq!(median_of_opts(&[None, Some(80.0)]), Some(80.0));
        assert_eq!(median_of_opts(&[None, None]), None);
        assert_eq!(median_of_opts(&[]), None);
    }

    #[test]
    fn short_labels_same_month_day_only() {
        let labels = short_date_labels(&["2026-07-01".into(), "2026-07-15".into()]);
        assert_eq!(labels, vec!["1".to_string(), "15".to_string()]);
    }

    #[test]
    fn short_labels_cross_month_no_year() {
        let labels = short_date_labels(&["2026-06-28".into(), "2026-07-05".into()]);
        assert_eq!(labels, vec!["6/28".to_string(), "7/5".to_string()]);
        for l in &labels {
            assert!(!l.contains("2026"));
        }
    }
}
