//! Command handlers for recomplog.
//!
//! Uses the new grouped CLI surface:
//!   recomplog workout ...
//!   recomplog body measurement ...
//!   recomplog body sleep ...
//!   recomplog nutrition product ...
//!   recomplog report ...   (top level)
//!   etc.

use crate::cli::*;
use crate::db;
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// Common JSON success shape used by mutating commands.
#[derive(serde::Serialize, Debug)]
pub struct Success {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Success {
    pub fn created(id: i64, msg: impl Into<String>) -> Self {
        Self {
            success: true,
            id: Some(id),
            message: Some(msg.into()),
        }
    }
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            id: None,
            message: Some(msg.into()),
        }
    }
}

fn print_json<T: serde::Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

fn print_error_json(err: &str) {
    #[derive(serde::Serialize)]
    struct ErrOut {
        success: bool,
        error: String,
    }
    print_json(&ErrOut {
        success: false,
        error: err.to_string(),
    });
}

pub fn dispatch(cli: Cli) -> Result<()> {
    let db_override = cli.db.as_deref();
    let json = cli.json;
    let quiet = cli.quiet;

    match cli.command {
        Commands::Version => {
            println!("recomplog {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Commands::Init { dry_run } => handle_init(dry_run, db_override, json),
        Commands::Migrate {
            status,
            dry_run,
            force,
        } => handle_migrate(status, dry_run, force, db_override, json),
        Commands::Import { action } => handle_import(action, db_override, json),
        Commands::Check(args) => handle_check(args, json),

        // === GROUPED DOMAINS ===
        Commands::Workout { action } => handle_workout(action, db_override, json, quiet),
        Commands::Body { action } => handle_body(action, db_override, json, quiet),
        Commands::Nutrition { action } => handle_nutrition(action, db_override, json, quiet),

        // Reports stay top-level
        Commands::Report { action } => handle_report(action, db_override, json, quiet),

        Commands::Config { action } => handle_config(action, json),
    }
}

// ---------- Workout group (training) ----------

fn handle_workout(
    action: WorkoutAction,
    db_override: Option<&str>,
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
            let started = started_at
                .map(|s| db::parse_flexible_date(&s).unwrap_or(s))
                .unwrap_or_else(db::now_utc);

            conn.execute(
                "INSERT INTO workouts (started_at, workout_type, notes) VALUES (?1, ?2, ?3)",
                params![started, workout_type, notes],
            )?;
            let id = conn.last_insert_rowid();

            if json {
                print_json(&Success::created(id, "workout created"));
            } else if !quiet {
                println!("Created workout {} (started {})", id, started);
            }
            Ok(())
        }
        WorkoutAction::List { days } => {
            let conn = db::open_db(db_override)?;
            let limit = days.unwrap_or(30);
            let mut stmt = conn.prepare(
                "SELECT id, started_at, workout_type, notes FROM workouts
                 ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?;

            if json {
                let items: Vec<_> = rows.filter_map(|r| r.ok()).collect();
                print_json(&items);
            } else {
                for (id, started, wtype) in rows.filter_map(|r| r.ok()) {
                    let t = wtype.unwrap_or_default();
                    println!("{}  {}  {}", id, started, t);
                }
            }
            Ok(())
        }
        WorkoutAction::Show { id } => {
            // very basic for now
            if json {
                print_json(&serde_json::json!({"id": id, "note": "full show not yet implemented"}));
            } else {
                println!(
                    "Workout {}: (detailed view not fully implemented in skeleton)",
                    id
                );
            }
            Ok(())
        }
        WorkoutAction::Exercise { action } => {
            handle_workout_exercise(action, db_override, json, quiet)
        }
        WorkoutAction::Set { action } => handle_workout_set(action, db_override, json, quiet),
    }
}

fn handle_workout_exercise(
    action: ExerciseAction,
    db_override: Option<&str>,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        ExerciseAction::List { search, category } => {
            let mut sql = "SELECT id, name, category, equipment FROM exercises".to_string();
            let mut params_vec: Vec<String> = vec![];
            if let Some(cat) = &category {
                sql.push_str(" WHERE category = ?1");
                params_vec.push(cat.clone());
            }
            if let Some(term) = &search {
                if params_vec.is_empty() {
                    sql.push_str(" WHERE name LIKE ?1");
                } else {
                    sql.push_str(" AND name LIKE ?1");
                }
                params_vec.push(format!("%{}%", term));
            }
            sql.push_str(" ORDER BY name LIMIT 100");

            let mut stmt = conn.prepare(&sql)?;
            // simple handling
            let rows = stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;

            if json {
                let list: Vec<_> = rows.filter_map(|r| r.ok()).collect();
                print_json(&list);
            } else {
                for (id, name, cat) in rows.filter_map(|r| r.ok()) {
                    println!("{}: {} ({})", id, name, cat);
                }
            }
        }
        ExerciseAction::Create {
            name,
            category,
            equipment,
            load_type,
            muscles,
        } => {
            let lt = load_type.unwrap_or_else(|| "weight".to_string());
            conn.execute(
                "INSERT OR IGNORE INTO exercises (name, category, equipment, load_type, muscle_groups) VALUES (?1,?2,?3,?4,?5)",
                params![name, category, equipment, lt, muscles],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(id, format!("exercise created: {}", name)));
            } else {
                println!("Exercise {}: {}", id, name);
            }
        }
        ExerciseAction::Search { term } => {
            let mut stmt = conn
                .prepare("SELECT id, name, category FROM exercises WHERE name LIKE ?1 LIMIT 30")?;
            let rows = stmt.query_map([format!("%{}%", term)], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            if json {
                let res: Vec<_> = rows.filter_map(|r| r.ok()).collect();
                print_json(&res);
            } else {
                for (id, name, cat) in rows.filter_map(|r| r.ok()) {
                    println!("{}: {} ({})", id, name, cat);
                }
            }
        }
    }
    Ok(())
}

fn handle_workout_set(
    _action: SetAction,
    _db_override: Option<&str>,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    if json {
        print_error_json("set commands are not fully implemented yet");
    } else {
        println!("`set` commands under workout are still being ported.");
    }
    Ok(())
}

// ---------- Body group ----------

fn handle_body(
    action: BodyAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        BodyAction::Measurement { action } => handle_measurement(action, db_override, json, quiet),
        BodyAction::Sleep { action } => handle_sleep(action, db_override, json, quiet),
    }
}

fn handle_measurement(
    action: MeasurementAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;

    match action {
        MeasurementAction::Create(args) => {
            let date = db::parse_flexible_date(&args.date)?;
            let now = db::now_utc();

            // Very basic insert (real version would do sanity checks)
            conn.execute(
                r#"
                INSERT INTO measurements
                    (date, weight_kg, body_fat_pct, skeletal_muscle_pct,
                     visceral_fat_level, bmi, resting_metabolism_kcal,
                     created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                ON CONFLICT(date) DO UPDATE SET
                    weight_kg=excluded.weight_kg,
                    body_fat_pct=excluded.body_fat_pct,
                    skeletal_muscle_pct=excluded.skeletal_muscle_pct,
                    visceral_fat_level=excluded.visceral_fat_level,
                    bmi=excluded.bmi,
                    resting_metabolism_kcal=excluded.resting_metabolism_kcal,
                    updated_at=excluded.updated_at
                "#,
                params![
                    date,
                    args.weight_kg,
                    args.body_fat_pct,
                    args.skeletal_muscle_pct,
                    args.visceral_fat_level,
                    args.bmi,
                    args.resting_metabolism_kcal,
                    now
                ],
            )?;

            // fetch id
            let id: i64 = conn.query_row(
                "SELECT id FROM measurements WHERE date = ?1",
                [&date],
                |r| r.get(0),
            )?;

            if json {
                print_json(&Success::created(id, format!("measurement for {}", date)));
            } else if !quiet {
                println!("Logged measurement for {} (id {})", date, id);
            }
        }

        MeasurementAction::List(args) => {
            let mut sql =
                "SELECT id, date, weight_kg, body_fat_pct, skeletal_muscle_pct FROM measurements"
                    .to_string();
            let mut where_clauses = vec![];
            let mut p: Vec<String> = vec![];

            if let Some(d) = &args.days {
                // last N days
                where_clauses.push("date >= date('now', ?)".to_string());
                p.push(format!("-{} days", d));
            } else if let Some(s) = &args.since {
                where_clauses.push("date >= ?".to_string());
                p.push(db::parse_flexible_date(s)?);
            }

            if let Some(u) = &args.until {
                where_clauses.push("date <= ?".to_string());
                p.push(db::parse_flexible_date(u)?);
            }

            if !where_clauses.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&where_clauses.join(" AND "));
            }
            sql.push_str(" ORDER BY date DESC");

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(p.iter()), |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "date": r.get::<_, String>(1)?,
                    "weight_kg": r.get::<_, Option<f64>>(2)?,
                    "body_fat_pct": r.get::<_, Option<f64>>(3)?,
                    "skeletal_muscle_pct": r.get::<_, Option<f64>>(4)?,
                }))
            })?;

            let list: Vec<_> = rows.filter_map(|r| r.ok()).collect();

            if json {
                print_json(&list);
            } else if list.is_empty() {
                println!("(no measurements)");
            } else {
                for item in &list {
                    println!(
                        "{}: weight={:?} bf%={:?} muscle%={:?}",
                        item["date"],
                        item["weight_kg"],
                        item["body_fat_pct"],
                        item["skeletal_muscle_pct"]
                    );
                }
            }
        }

        MeasurementAction::Show(args) => {
            let row = if let Some(id) = args.id {
                conn.query_row(
                    "SELECT id,date,weight_kg,body_fat_pct FROM measurements WHERE id=?",
                    [id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<f64>>(2)?,
                        ))
                    },
                )
                .optional()?
            } else if let Some(d) = args.date {
                let date = db::parse_flexible_date(&d)?;
                conn.query_row(
                    "SELECT id,date,weight_kg,body_fat_pct FROM measurements WHERE date=?",
                    [&date],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<f64>>(2)?,
                        ))
                    },
                )
                .optional()?
            } else {
                None
            };

            match row {
                Some((id, date, w)) => {
                    if json {
                        print_json(&serde_json::json!({"id":id,"date":date,"weight_kg":w}));
                    } else {
                        println!("{} (id {}): weight = {:?}", date, id, w);
                    }
                }
                None => {
                    if json {
                        print_error_json("measurement not found");
                    } else {
                        eprintln!("not found");
                    }
                }
            }
        }

        MeasurementAction::Update(_) | MeasurementAction::Delete(_) => {
            if json {
                print_error_json("update/delete for measurements not implemented in this step");
            } else {
                println!("update/delete coming soon");
            }
        }
    }
    Ok(())
}

fn handle_sleep(
    action: SleepAction,
    db_override: Option<&str>,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        SleepAction::Create {
            date, total_sleep, ..
        } => {
            let d = db::parse_flexible_date(&date)?;
            let now = db::now_utc();
            // Minimal insert for skeleton
            conn.execute(
                "INSERT OR REPLACE INTO sleep (date, total_sleep_minutes, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                params![d, total_sleep.as_ref().and_then(|s| parse_duration_to_minutes(s).ok()), now],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(id, format!("sleep logged for {}", d)));
            } else {
                println!("Sleep logged for {}", d);
            }
        }
        SleepAction::List(_args) => {
            // simple list
            let mut stmt = conn.prepare(
                "SELECT id, date, total_sleep_minutes FROM sleep ORDER BY date DESC LIMIT 20",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_,i64>(0)?,
                    "date": r.get::<_,String>(1)?,
                    "total_sleep_minutes": r.get::<_,Option<i64>>(2)?
                }))
            })?;
            let data: Vec<_> = rows.filter_map(|x| x.ok()).collect();
            if json {
                print_json(&data);
            } else {
                for d in data {
                    println!("{}: {} min", d["date"], d["total_sleep_minutes"]);
                }
            }
        }
    }
    Ok(())
}

fn parse_duration_to_minutes(s: &str) -> Result<i64> {
    // very rough "7h 30m" or "450" support for skeleton
    if let Ok(mins) = s.parse::<i64>() {
        return Ok(mins);
    }
    // naive
    let mut total = 0i64;
    for part in s.split_whitespace() {
        if part.ends_with('h') || part.ends_with('H') {
            if let Ok(h) = part
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<i64>()
            {
                total += h * 60;
            }
        } else if part.ends_with('m') || part.ends_with('M') {
            if let Ok(m) = part
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<i64>()
            {
                total += m;
            }
        }
    }
    Ok(total)
}

// ---------- Nutrition group ----------

fn handle_nutrition(
    action: NutritionAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        NutritionAction::Product { action } => handle_product(action, db_override, json, quiet),
        NutritionAction::Purchase { action } => handle_purchase(action, db_override, json, quiet),
        NutritionAction::Consumption { action } => {
            handle_consumption(action, db_override, json, quiet)
        }
        NutritionAction::Nutrient { action } => handle_nutrient(action, db_override, json, quiet),
    }
}

fn handle_product(
    action: ProductAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        ProductAction::Create { name, tags } => {
            let now = db::now_utc();
            conn.execute(
                "INSERT INTO products (name, created_at, updated_at) VALUES (?1, ?2, ?2)",
                params![name, now],
            )?;
            let id = conn.last_insert_rowid();

            if let Some(ts) = tags {
                for t in ts {
                    conn.execute(
                        "INSERT OR IGNORE INTO product_tags (name, created_at) VALUES (?1, ?2)",
                        params![t, now],
                    )?;
                    let tag_id: i64 =
                        conn.query_row("SELECT id FROM product_tags WHERE name = ?1", [&t], |r| {
                            r.get(0)
                        })?;
                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO product_tag_associations (product_id, tag_id) VALUES (?1, ?2)",
                        params![id, tag_id],
                    );
                }
            }

            if json {
                print_json(&Success::created(id, format!("product created: {}", name)));
            } else if !quiet {
                println!("Created product {} ({})", id, name);
            }
        }
        ProductAction::List => {
            let mut stmt =
                conn.prepare("SELECT id, name, created_at FROM products ORDER BY id DESC")?;
            let rows = stmt.query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "name": r.get::<_, String>(1)?,
                }))
            })?;
            let products: Vec<_> = rows.filter_map(|r| r.ok()).collect();

            if json {
                print_json(&products);
            } else if products.is_empty() {
                println!("(no products)");
            } else {
                for p in products {
                    println!("{}: {}", p["id"], p["name"]);
                }
            }
        }
        ProductAction::Search { name, tag } => {
            // simple implementation
            if let Some(n) = name {
                let mut stmt =
                    conn.prepare("SELECT id, name FROM products WHERE name LIKE ? LIMIT 30")?;
                let rows = stmt.query_map([format!("%{}%", n)], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?;
                for (id, nm) in rows.filter_map(|r| r.ok()) {
                    println!("{}: {}", id, nm);
                }
            } else if let Some(t) = tag {
                // very basic tag filter
                println!("(tag filter not fully wired: {})", t);
            }
        }
        ProductAction::Show { id } => {
            let name: Option<String> = conn
                .query_row("SELECT name FROM products WHERE id=?", [id], |r| r.get(0))
                .optional()?;
            match name {
                Some(n) => {
                    if json {
                        print_json(&serde_json::json!({"id":id,"name":n}));
                    } else {
                        println!("{}: {}", id, n);
                    }
                }
                None => {
                    if json {
                        print_error_json("product not found");
                    } else {
                        eprintln!("product not found");
                    }
                }
            }
        }
    }
    Ok(())
}

fn handle_purchase(
    action: PurchaseAction,
    db_override: Option<&str>,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        PurchaseAction::Create {
            product,
            quantity,
            price,
            store,
        } => {
            let now = db::now_utc();
            let price_cents: Option<i64> = price
                .and_then(|p| p.replace('$', "").parse::<f64>().ok())
                .map(|v| (v * 100.0).round() as i64);

            conn.execute(
                "INSERT INTO purchases (product_id, quantity, price_cents, store_id, purchased_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![product, quantity, price_cents, store, now],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(id, "purchase recorded"));
            } else {
                println!("Purchase {} recorded", id);
            }
        }
        PurchaseAction::List => {
            let mut stmt = conn.prepare(
                "SELECT id, product_id, quantity, price_cents FROM purchases ORDER BY id DESC LIMIT 30",
            )?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_,i64>(0)?,
                        "product_id": r.get::<_,i64>(1)?,
                        "quantity": r.get::<_,f64>(2)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for r in rows {
                    println!("{:?}", r);
                }
            }
        }
    }
    Ok(())
}

fn handle_consumption(
    action: ConsumptionAction,
    db_override: Option<&str>,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        ConsumptionAction::Create {
            product,
            quantity,
            date,
        } => {
            let when = if let Some(d) = date {
                db::parse_flexible_date(&d)?
            } else {
                chrono::Local::now()
                    .date_naive()
                    .format("%Y-%m-%d")
                    .to_string()
            };
            let now = db::now_utc();
            conn.execute(
                "INSERT INTO consumptions (product_id, quantity, consumed_at, created_at) VALUES (?1,?2,?3,?4)",
                params![product, quantity, when, now],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(id, "consumption logged"));
            } else {
                println!("Consumption {} logged", id);
            }
        }
        ConsumptionAction::List => {
            let mut stmt = conn.prepare(
                "SELECT id, product_id, quantity, consumed_at FROM consumptions ORDER BY consumed_at DESC LIMIT 20",
            )?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_,i64>(0)?,
                        "product_id": r.get::<_,i64>(1)?,
                        "quantity": r.get::<_,f64>(2)?,
                        "consumed_at": r.get::<_,String>(3)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for r in rows {
                    println!("{:?}", r);
                }
            }
        }
    }
    Ok(())
}

fn handle_nutrient(
    action: NutrientAction,
    db_override: Option<&str>,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        NutrientAction::List => {
            let mut stmt = conn.prepare("SELECT id, name, unit FROM nutrients ORDER BY name")?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_,i64>(0)?,
                        "name": r.get::<_,String>(1)?,
                        "unit": r.get::<_,String>(2)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for n in rows {
                    println!("{}: {} ({})", n["id"], n["name"], n["unit"]);
                }
            }
        }
        NutrientAction::Create {
            name,
            unit,
            recommended_intake,
        } => {
            let now = db::now_utc();
            conn.execute(
                "INSERT INTO nutrients (name, unit, recommended_intake, created_at) VALUES (?1,?2,?3,?4)",
                params![name, unit, recommended_intake, now],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(id, format!("nutrient: {}", name)));
            } else {
                println!("Created nutrient {}: {}", id, name);
            }
        }
    }
    Ok(())
}

// ---------- Report (top level) ----------

fn handle_report(
    action: ReportAction,
    _db_override: Option<&str>,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    match action {
        ReportAction::Html { days, .. } => {
            if json {
                print_json(&serde_json::json!({
                    "success": true,
                    "message": format!("html report for last {} days would be generated here (direct DB access)", days)
                }));
            } else {
                println!(
                    "HTML dashboard generation ({} days) is ready to be wired to direct queries.",
                    days
                );
                println!("(See bodydashboard/ in the old repo for the template.)");
            }
        }
        _ => {
            if json {
                print_json(&serde_json::json!({"note": "other reports not fully implemented"}));
            } else {
                println!("Other report subcommands are still being ported to the unified DB.");
            }
        }
    }
    Ok(())
}

// ---------- Other top level ----------

fn handle_init(dry_run: bool, db_override: Option<&str>, json: bool) -> Result<()> {
    if dry_run {
        if json {
            println!(r#"{{"success":true,"message":"dry-run: would initialize database"}}"#);
        } else {
            println!("dry-run: would create/open database and apply migrations");
        }
        return Ok(());
    }
    let _conn = db::open_db(db_override)?;
    if json {
        println!(r#"{{"success":true,"message":"database initialized"}}"#);
    } else {
        println!("Database initialized (or already up to date).");
    }
    Ok(())
}

fn handle_migrate(
    status: bool,
    dry_run: bool,
    _force: bool,
    db_override: Option<&str>,
    json: bool,
) -> Result<()> {
    let target = 1;
    if status || dry_run {
        if json {
            println!(
                r#"{{"current":1,"latest":{},"dry_run":{}}}"#,
                target, dry_run
            );
        } else {
            println!("Current schema version: 1 (target: {})", target);
        }
        return Ok(());
    }
    let _conn = db::open_db(db_override)?;
    println!("Migrations are applied automatically when opening the database.");
    Ok(())
}

fn handle_import(action: ImportAction, db_override: Option<&str>, json: bool) -> Result<()> {
    match action {
        ImportAction::Fit { path, exercise } => {
            if json {
                print_error_json("FIT import not yet ported");
            } else {
                println!(
                    "FIT import not implemented (path={}, exercise={:?})",
                    path, exercise
                );
            }
            Ok(())
        }
        ImportAction::Legacy {
            from_db,
            domain,
            dry_run,
        } => handle_legacy_import(&from_db, domain.as_deref(), dry_run, db_override, json),
    }
}

fn handle_legacy_import(
    from_path: &str,
    domain_filter: Option<&str>,
    dry_run: bool,
    target_override: Option<&str>,
    json: bool,
) -> Result<()> {
    let src = db::open_legacy_db_readonly(from_path)?;
    let tables: Vec<String> = src
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;

    let detected = detect_domains(&tables);

    if let Some(d) = domain_filter {
        if !detected.iter().any(|x| x.eq_ignore_ascii_case(d)) {
            return Err(anyhow!("domain '{}' not present in source DB", d));
        }
    }

    if dry_run {
        if json {
            print_json(&serde_json::json!({
                "source": from_path,
                "detected": detected,
                "dry_run": true
            }));
        } else {
            println!("Legacy: {}", from_path);
            println!("Detected domains: {:?}", detected);
            println!("Dry run — nothing copied.");
        }
        return Ok(());
    }

    // Real import
    let mut target = db::open_db(target_override)?;

    let mut copied = vec![];

    if detected.iter().any(|d| d == &"nutrition") {
        copy_nutrition(&src, &mut target)?;
        copied.push("nutrition");
    }
    if detected.iter().any(|d| d == &"body") {
        copy_body(&src, &mut target)?;
        copied.push("body");
    }
    if detected.iter().any(|d| d == &"workout") {
        // More complex (FK ordering). For now just note.
        copied.push("workout (structure only - full copy later)");
    }

    if json {
        print_json(&serde_json::json!({
            "success": true,
            "source": from_path,
            "imported_domains": copied
        }));
    } else {
        println!("Imported from {} into recomplog.db", from_path);
        println!("Domains: {:?}", copied);
    }

    Ok(())
}

fn copy_nutrition(src: &Connection, dst: &mut Connection) -> Result<()> {
    let tx = dst.transaction()?;

    // Products
    {
        let mut stmt = src.prepare("SELECT id, name, created_at, updated_at FROM products")?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            let (id, name, ca, ua) = row?;
            tx.execute(
                "INSERT OR IGNORE INTO products (id, name, created_at, updated_at) VALUES (?1,?2,?3,?4)",
                params![id, name, ca, ua],
            )?;
        }
    }

    // Nutrients (master list)
    {
        let mut stmt =
            src.prepare("SELECT id, name, unit, recommended_intake, created_at FROM nutrients")?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })? {
            let (id, name, unit, rec, ca) = row?;
            tx.execute(
                "INSERT OR IGNORE INTO nutrients (id, name, unit, recommended_intake, created_at) VALUES (?1,?2,?3,?4,?5)",
                params![id, name, unit, rec, ca],
            )?;
        }
    }

    // Purchases (basic)
    {
        let mut stmt = src.prepare("SELECT id, product_id, quantity, price_cents, store_id, purchased_at, created_at FROM purchases")?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })? {
            let (id, pid, qty, price, sid, pa, ca) = row?;
            tx.execute(
                "INSERT OR IGNORE INTO purchases (id, product_id, quantity, price_cents, store_id, purchased_at, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![id, pid, qty, price, sid, pa, ca],
            )?;
        }
    }

    // Consumptions
    {
        let mut stmt = src.prepare(
            "SELECT id, product_id, quantity, unit, consumed_at, created_at FROM consumptions",
        )?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })? {
            let (id, pid, qty, unit, ca_at, ca) = row?;
            tx.execute(
                "INSERT OR IGNORE INTO consumptions (id, product_id, quantity, unit, consumed_at, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
                params![id, pid, qty, unit, ca_at, ca],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn copy_body(src: &Connection, dst: &mut Connection) -> Result<()> {
    let tx = dst.transaction()?;

    // Measurements - tolerant path
    {
        let mut stmt = match src.prepare(
            "SELECT id,date,weight_kg,body_fat_pct,skeletal_muscle_pct,created_at,updated_at FROM measurements"
        ) {
            Ok(s) => s,
            Err(_) => src.prepare("SELECT id,date,weight_kg,body_fat_pct,skeletal_muscle_pct,created_at,updated_at FROM measurements")?,
        };
        for row in stmt.query_map([], |r| {
            let id: i64 = r.get(0).unwrap_or(0);
            let date: String = r.get(1).unwrap_or_default();
            let w: Option<f64> = r.get(2).ok().flatten();
            let bf: Option<f64> = r.get(3).ok().flatten();
            let sm: Option<f64> = r.get(4).ok().flatten();
            let ca: String = r.get(5).unwrap_or_default();
            let ua: String = r.get(6).unwrap_or_default();
            Ok((id, date, w, bf, sm, ca, ua))
        })? {
            let (id, date, w, bf, sm, ca, ua) = row?;
            tx.execute(
                r#"INSERT OR REPLACE INTO measurements
                   (id, date, weight_kg, body_fat_pct, skeletal_muscle_pct,
                    visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at)
                   VALUES (?1,?2,?3,?4,?5, NULL, NULL, NULL, ?6, ?7)"#,
                params![id, date, w, bf, sm, ca, ua],
            )?;
        }
    }

    // Sleep - tolerant
    {
        let mut stmt = match src.prepare("SELECT id,date,total_sleep_minutes,created_at,updated_at FROM sleep") {
            Ok(s) => s,
            Err(_) => src.prepare("SELECT id,date,total_sleep_minutes,created_at,updated_at FROM sleep")?,
        };
        for row in stmt.query_map([], |r| {
            let id: i64 = r.get(0).unwrap_or(0);
            let date: String = r.get(1).unwrap_or_default();
            let total: Option<i64> = r.get(2).ok().flatten();
            let ca: String = r.get(3).unwrap_or_default();
            let ua: String = r.get(4).unwrap_or_default();
            Ok((id, date, total, ca, ua))
        })? {
            let (id, date, total, ca, ua) = row?;
            tx.execute(
                "INSERT OR REPLACE INTO sleep (id,date,total_sleep_minutes,created_at,updated_at) VALUES (?1,?2,?3,?4,?5)",
                params![id, date, total, ca, ua],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn detect_domains(tables: &[String]) -> Vec<&'static str> {
    let mut out = vec![];
    let has = |name: &str| tables.iter().any(|t| t.eq_ignore_ascii_case(name));

    if has("workouts") || has("exercise_sets") || has("exercises") {
        out.push("workout");
    }
    if has("measurements") || has("sleep") {
        out.push("body");
    }
    if has("products") || has("purchases") || has("consumptions") || has("nutrients") {
        out.push("nutrition");
    }
    out
}

fn handle_config(action: ConfigAction, json: bool) -> Result<()> {
    match action {
        ConfigAction::Show | ConfigAction::Path => {
            if json {
                print_json(&serde_json::json!({"config": "unified config not yet fully wired"}));
            } else {
                println!("Config management will use ~/.config/recomplog/config.toml (sanity limits etc.)");
            }
        }
        ConfigAction::Generate { .. } => {
            if json {
                print_json(&Success::ok("default config would be written"));
            } else {
                println!("(config generate not implemented in skeleton)");
            }
        }
    }
    Ok(())
}

fn handle_check(args: CheckArgs, json: bool) -> Result<()> {
    if json {
        print_json(&serde_json::json!({
            "ok": true,
            "variations": args.variations,
            "note": "check is a skeleton; full port of sanity rules coming"
        }));
    } else {
        println!("`check` (sanity audit) not fully implemented yet.");
    }
    Ok(())
}
