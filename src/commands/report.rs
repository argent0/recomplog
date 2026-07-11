//! Cross-domain reports and HTML dashboard.

use crate::cli::{NutritionReportAction, ReportAction};
use crate::config::SanityLimits;
use crate::db;
use crate::repository::BodyRepository;
use crate::utils::{print_json, resolve_date_range};
use anyhow::Result;
use chrono::Local;
use rusqlite::params;
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

fn handle_nutrition_report(
    action: NutritionReportAction,
    db_override: Option<&str>,
    json: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        NutritionReportAction::List { days, since, until } => {
            let (s, u) =
                resolve_date_range(since.as_deref(), until.as_deref(), days.map(|d| d as i64))?;
            let mut sql = String::from(
                "SELECT c.consumed_at, c.product_id, p.name, c.quantity,
                        pn.energy_kcal, pn.protein_g, pn.carbohydrates_g, pn.fat_g, pn.fiber_g, pn.sugars_g,
                        pn.reference_quantity
                 FROM consumptions c
                 LEFT JOIN products p ON p.id = c.product_id
                 LEFT JOIN product_nutritions pn ON pn.product_id = c.product_id
                 WHERE 1=1",
            );
            let mut bind: Vec<String> = vec![];
            if let Some(ref ss) = s {
                sql.push_str(" AND date(c.consumed_at) >= date(?)");
                bind.push(ss.clone());
            }
            if let Some(ref uu) = u {
                sql.push_str(" AND date(c.consumed_at) <= date(?)");
                bind.push(uu.clone());
            }
            sql.push_str(" ORDER BY c.consumed_at DESC");

            let mut stmt = conn.prepare(&sql)?;
            let rows: Vec<_> = stmt
                .query_map(rusqlite::params_from_iter(bind.iter()), |r| {
                    let qty: f64 = r.get(3)?;
                    let ref_q: Option<f64> = r.get(10)?;
                    let scale = match ref_q {
                        Some(rq) if rq > 0.0 => qty / rq,
                        _ => qty,
                    };
                    let scale_opt = |v: Option<f64>| v.map(|x| x * scale);
                    Ok(serde_json::json!({
                        "date": r.get::<_, String>(0)?,
                        "product_id": r.get::<_, i64>(1)?,
                        "product_name": r.get::<_, Option<String>>(2)?,
                        "quantity": qty,
                        "energy_kcal": scale_opt(r.get(4)?),
                        "protein_g": scale_opt(r.get(5)?),
                        "carbohydrates_g": scale_opt(r.get(6)?),
                        "fat_g": scale_opt(r.get(7)?),
                        "fiber_g": scale_opt(r.get(8)?),
                        "sugars_g": scale_opt(r.get(9)?),
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();

            // totals
            let mut tot_kcal = 0.0;
            let mut tot_protein = 0.0;
            for r in &rows {
                if let Some(v) = r["energy_kcal"].as_f64() {
                    tot_kcal += v;
                }
                if let Some(v) = r["protein_g"].as_f64() {
                    tot_protein += v;
                }
            }
            let out = serde_json::json!({
                "period": {"since": s, "until": u},
                "entries": rows,
                "totals": {"energy_kcal": tot_kcal, "protein_g": tot_protein},
            });
            if json {
                print_json(&out);
            } else {
                println!(
                    "Nutrition report ({} entries): {:.0} kcal, {:.1}g protein",
                    out["entries"].as_array().map(|a| a.len()).unwrap_or(0),
                    tot_kcal,
                    tot_protein
                );
                for r in out["entries"].as_array().unwrap_or(&vec![]) {
                    println!(
                        "  {}  {}  qty={}  kcal={:?}",
                        r["date"], r["product_name"], r["quantity"], r["energy_kcal"]
                    );
                }
            }
            Ok(())
        }
        NutritionReportAction::Spending { days } => {
            let days = days.unwrap_or(30) as i64;
            let since = Local::now().date_naive() - chrono::Duration::days(days - 1);
            let since_s = since.format("%Y-%m-%d").to_string();
            let mut stmt = conn.prepare(
                "SELECT COALESCE(SUM(price_cents), 0), COUNT(*) FROM purchases
                 WHERE date(purchased_at) >= date(?1)",
            )?;
            let (cents, count): (i64, i64) =
                stmt.query_row(params![since_s], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let out = serde_json::json!({
                "days": days,
                "purchase_count": count,
                "total_cents": cents,
                "total": format!("{:.2}", cents as f64 / 100.0),
            });
            if json {
                print_json(&out);
            } else {
                println!(
                    "Spending last {} days: {} purchases, ${:.2}",
                    days,
                    count,
                    cents as f64 / 100.0
                );
            }
            Ok(())
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
