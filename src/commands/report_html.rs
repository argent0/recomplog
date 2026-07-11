//! Self-contained Chart.js HTML dashboard (`report html`).

use crate::db;
use crate::stats::{
    metric_trend_from_points, DataPoint, MetricTrend, TrendDirection, BF_FLAT_PCT_PER_DAY,
    WEIGHT_FLAT_KG_PER_DAY,
};
use crate::utils::print_json;
use anyhow::Result;
use chrono::{Local, NaiveDate};
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
struct MeasPoint {
    date: String,
    weight_kg: Option<f64>,
    body_fat_pct: Option<f64>,
    fat_mass_kg: Option<f64>,
    lean_mass_kg: Option<f64>,
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
    let mut stmt = conn.prepare(
        "SELECT date, weight_kg, body_fat_pct, skeletal_muscle_pct, bmi
         FROM measurements WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC",
    )?;
    let rows = stmt
        .query_map(params![since, until], |r| {
            let weight_kg: Option<f64> = r.get(1)?;
            let body_fat_pct: Option<f64> = r.get(2)?;
            let (fat_mass_kg, lean_mass_kg) = derive_composition(weight_kg, body_fat_pct);
            Ok(MeasPoint {
                date: r.get(0)?,
                weight_kg,
                body_fat_pct,
                fat_mass_kg,
                lean_mass_kg,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn fetch_html_sleep(conn: &Connection, since: &str, until: &str) -> Result<Vec<SleepPoint>> {
    let mut stmt = conn.prepare(
        "SELECT date, total_sleep_minutes, rem_minutes, deep_minutes, light_minutes,
                awake_minutes, sleep_efficiency_pct, sleep_score
         FROM sleep WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC",
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
    let mut stmt = conn.prepare(
        "SELECT date(c.consumed_at) AS d,
          SUM(CASE WHEN pn.energy_kcal IS NOT NULL AND pn.reference_quantity > 0
              THEN c.quantity / pn.reference_quantity * pn.energy_kcal ELSE 0 END) AS kcal,
          SUM(CASE WHEN pn.protein_g IS NOT NULL AND pn.reference_quantity > 0
              THEN c.quantity / pn.reference_quantity * pn.protein_g ELSE 0 END) AS protein,
          SUM(CASE WHEN pn.carbohydrates_g IS NOT NULL AND pn.reference_quantity > 0
              THEN c.quantity / pn.reference_quantity * pn.carbohydrates_g ELSE 0 END) AS carbs,
          SUM(CASE WHEN pn.fat_g IS NOT NULL AND pn.reference_quantity > 0
              THEN c.quantity / pn.reference_quantity * pn.fat_g ELSE 0 END) AS fat,
          SUM(CASE WHEN pn.fiber_g IS NOT NULL AND pn.reference_quantity > 0
              THEN c.quantity / pn.reference_quantity * pn.fiber_g ELSE 0 END) AS fiber,
          SUM(CASE WHEN pn.sugars_g IS NOT NULL AND pn.reference_quantity > 0
              THEN c.quantity / pn.reference_quantity * pn.sugars_g ELSE 0 END) AS sugars
         FROM consumptions c
         LEFT JOIN product_nutritions pn ON pn.product_id = c.product_id
         WHERE date(c.consumed_at) >= date(?1) AND date(c.consumed_at) <= date(?2)
         GROUP BY date(c.consumed_at)
         ORDER BY d ASC",
    )?;
    let rows = stmt
        .query_map(params![since, until], |r| {
            Ok(NutDay {
                date: r.get(0)?,
                energy_kcal: r.get(1)?,
                protein_g: r.get(2)?,
                carbohydrates_g: r.get(3)?,
                fat_g: r.get(4)?,
                fiber_g: r.get(5)?,
                sugars_g: r.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
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

fn j<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
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
    let fat_mass: Vec<Option<f64>> = measurements.iter().map(|m| m.fat_mass_kg).collect();
    let lean_mass: Vec<Option<f64>> = measurements.iter().map(|m| m.lean_mass_kg).collect();

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

    let latest_weight = measurements.iter().rev().find_map(|m| m.weight_kg);
    let latest_bf = measurements.iter().rev().find_map(|m| m.body_fat_pct);
    let (fat_mass_latest, lean_mass_latest) = derive_composition(latest_weight, latest_bf);

    let overview = serde_json::json!({
        "latest_weight_kg": latest_weight,
        "latest_body_fat_pct": latest_bf,
        "fat_mass_kg": fat_mass_latest,
        "lean_mass_kg": lean_mass_latest,
        "measurement_count": measurements.len(),
        "sleep_nights": sleeps.len(),
        "weight_trend": weight_trend,
        "body_fat_trend": body_fat_trend,
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
        &weight_labels,
        &weights,
        &bf,
        &fat_mass,
        &lean_mass,
        &sleeps,
        &nutrition,
        &training,
        latest_weight,
        latest_bf,
        fat_mass_latest,
        lean_mass_latest,
        measurements.len(),
        sleeps.len(),
        &weight_trend,
        &body_fat_trend,
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
    weight_labels: &[String],
    weights: &[Option<f64>],
    bf: &[Option<f64>],
    fat_mass: &[Option<f64>],
    lean_mass: &[Option<f64>],
    sleeps: &[SleepPoint],
    nutrition: &[NutDay],
    training: &TrainingSummary,
    latest_weight: Option<f64>,
    latest_bf: Option<f64>,
    fat_mass_latest: Option<f64>,
    lean_mass_latest: Option<f64>,
    measurement_count: usize,
    sleep_nights: usize,
    weight_trend: &MetricTrend,
    body_fat_trend: &MetricTrend,
) -> String {
    let sleep_labels: Vec<String> = sleeps.iter().map(|s| s.date.clone()).collect();
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
    let stage_labels: Vec<String> = stage_nights.iter().map(|s| s.date.clone()).collect();
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

    let nut_labels: Vec<String> = nutrition.iter().map(|n| n.date.clone()).collect();
    let nut_kcal: Vec<f64> = nutrition.iter().map(|n| n.energy_kcal).collect();
    let nut_protein: Vec<f64> = nutrition.iter().map(|n| n.protein_g).collect();
    let nut_carbs: Vec<f64> = nutrition.iter().map(|n| n.carbohydrates_g).collect();
    let nut_fat: Vec<f64> = nutrition.iter().map(|n| n.fat_g).collect();
    let nut_fiber: Vec<f64> = nutrition.iter().map(|n| n.fiber_g).collect();
    let nut_sugars: Vec<f64> = nutrition.iter().map(|n| n.sugars_g).collect();
    let show_nutrition = !nutrition.is_empty();

    let show_training = training.workout_count > 0;

    let dash = serde_json::json!({
        "weightLabels": weight_labels,
        "weights": weights,
        "bf": bf,
        "fatMass": fat_mass,
        "leanMass": lean_mass,
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
        "nutProtein": nut_protein,
        "nutCarbs": nut_carbs,
        "nutFat": nut_fat,
        "nutFiber": nut_fiber,
        "nutSugars": nut_sugars,
    });

    let training_cards = if show_training {
        format!(
            r#"    <div class="card"><div class="label">Workouts</div><div class="value">{wc}</div></div>
    <div class="card"><div class="label">Volume</div><div class="value">{vol:.0}</div></div>
"#,
            wc = training.workout_count,
            vol = training.total_volume,
        )
    } else {
        String::new()
    };

    let mut chart_cards = String::new();
    chart_cards.push_str(
        r#"    <div class="chart-card"><h2>Weight (kg)</h2><div class="chart-wrap"><canvas id="wChart"></canvas></div></div>
    <div class="chart-card"><h2>Body fat %</h2><div class="chart-wrap"><canvas id="bfChart"></canvas></div></div>
    <div class="chart-card"><h2>Fat / lean mass (kg)</h2><div class="chart-wrap"><canvas id="fmChart"></canvas></div></div>
    <div class="chart-card"><h2>Sleep total (min)</h2><div class="chart-wrap"><canvas id="sChart"></canvas></div></div>
"#,
    );
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
"#,
        );
    }

    let mut chart_js = String::from(
        r#"
lineChart('wChart', D.weightLabels, D.weights, '#5b9fd4', 'weight');
lineChart('bfChart', D.weightLabels, D.bf, '#d4a05b', 'bf%');
new Chart(document.getElementById('fmChart'), {
  type: 'line',
  data: { labels: D.weightLabels, datasets: [
    { label: 'Fat mass', data: D.fatMass, borderColor: '#d45b5b', tension: 0.2, spanGaps: true, pointRadius: 2 },
    { label: 'Lean mass', data: D.leanMass, borderColor: '#5bd4a0', tension: 0.2, spanGaps: true, pointRadius: 2 }
  ]},
  options: { responsive: true, maintainAspectRatio: false,
    plugins: { legend: { display: true, labels: { color: '#8b9bb4' } } },
    scales: { x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
               y: { ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } } } }
});
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
new Chart(document.getElementById('nKcalChart'), {
  type: 'bar',
  data: { labels: D.nutLabels, datasets: [
    { label: 'kcal', data: D.nutKcal, backgroundColor: '#5bd4a0' }
  ]},
  options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { display: false } },
    scales: { x: { ticks: { color: '#8b9bb4', maxTicksLimit: 8 }, grid: { color: '#243044' } },
               y: { ticks: { color: '#8b9bb4' }, grid: { color: '#243044' } } } }
});
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
    <div class="card"><div class="label">Weight</div><div class="value">{weight}</div><div class="trend {w_trend_cls}">{w_trend}</div></div>
    <div class="card"><div class="label">Body fat</div><div class="value">{bf_val}</div><div class="trend {bf_trend_cls}">{bf_trend}</div></div>
    <div class="card"><div class="label">Fat mass</div><div class="value">{fat}</div></div>
    <div class="card"><div class="label">Lean mass</div><div class="value">{lean}</div></div>
    <div class="card"><div class="label">Measurements</div><div class="value">{mc}</div></div>
    <div class="card"><div class="label">Sleep nights</div><div class="value">{sc}</div></div>
{training_cards}  </div>
  <div class="charts">
{chart_cards}  </div>
<script>
const D = {dash};
function lineChart(id, labels, data, color, label) {{
  new Chart(document.getElementById(id), {{
    type: 'line',
    data: {{ labels, datasets: [{{ label, data, borderColor: color, tension: 0.2, spanGaps: true, pointRadius: 2 }}] }},
    options: {{ responsive: true, maintainAspectRatio: false, plugins: {{ legend: {{ display: false }} }},
      scales: {{ x: {{ ticks: {{ color: '#8b9bb4', maxTicksLimit: 8 }}, grid: {{ color: '#243044' }} }},
                 y: {{ ticks: {{ color: '#8b9bb4' }}, grid: {{ color: '#243044' }} }} }} }}
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
        weight = latest_fmt(latest_weight, "kg"),
        bf_val = latest_fmt(latest_bf, "%"),
        fat = latest_fmt(fat_mass_latest, "kg"),
        lean = latest_fmt(lean_mass_latest, "kg"),
        mc = measurement_count,
        sc = sleep_nights,
        w_trend = weight_trend.label,
        bf_trend = body_fat_trend.label,
        w_trend_cls = trend_css_class(weight_trend.direction),
        bf_trend_cls = trend_css_class(body_fat_trend.direction),
        training_cards = training_cards,
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
}
