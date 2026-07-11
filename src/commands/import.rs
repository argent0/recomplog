//! Import: legacy DB + FIT.

use crate::cli::ImportAction;
use crate::db;
use crate::utils::{print_error_json, print_json};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub fn handle(action: ImportAction, db_override: Option<&str>, json: bool) -> Result<()> {
    match action {
        ImportAction::Fit { path, exercise } => {
            if json {
                print_error_json("FIT import not yet fully ported");
            } else {
                println!(
                    "FIT import stub (path={}, exercise={:?}). Use import legacy for old DBs.",
                    path, exercise
                );
            }
            // Soft success for skeleton - return error so agents know
            Err(anyhow!("FIT import not yet implemented"))
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

    let mut target = db::open_db(target_override)?;
    let mut copied = vec![];
    let mut counts = serde_json::Map::new();

    let run_all = domain_filter.is_none();
    let want = |name: &str| {
        run_all
            || domain_filter
                .map(|d| d.eq_ignore_ascii_case(name))
                .unwrap_or(false)
    };

    if detected.contains(&"nutrition") && want("nutrition") {
        let n = copy_nutrition(&src, &mut target)?;
        counts.insert("nutrition".into(), serde_json::json!(n));
        copied.push("nutrition");
    }
    if detected.contains(&"body") && want("body") {
        let n = copy_body(&src, &mut target)?;
        counts.insert("body".into(), serde_json::json!(n));
        copied.push("body");
    }
    if detected.contains(&"workout") && want("workout") {
        let n = copy_workout(&src, &mut target)?;
        counts.insert("workout".into(), serde_json::json!(n));
        copied.push("workout");
    }

    if json {
        print_json(&serde_json::json!({
            "success": true,
            "source": from_path,
            "imported_domains": copied,
            "counts": counts,
        }));
    } else {
        println!("Imported from {} into recomplog.db", from_path);
        println!("Domains: {:?}", copied);
        println!("Counts: {:?}", counts);
    }

    Ok(())
}

fn copy_nutrition(src: &Connection, dst: &mut Connection) -> Result<serde_json::Value> {
    let tx = dst.transaction()?;
    let mut products = 0i64;
    let mut nutrients = 0i64;
    let mut purchases = 0i64;
    let mut consumptions = 0i64;
    let mut tags = 0i64;

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
            products += tx.execute(
                "INSERT OR IGNORE INTO products (id, name, created_at, updated_at) VALUES (?1,?2,?3,?4)",
                params![id, name, ca, ua],
            )? as i64;
        }
    }
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
            nutrients += tx.execute(
                "INSERT OR IGNORE INTO nutrients (id, name, unit, recommended_intake, created_at) VALUES (?1,?2,?3,?4,?5)",
                params![id, name, unit, rec, ca],
            )? as i64;
        }
    }
    // tags
    if let Ok(mut stmt) = src.prepare("SELECT id, name, created_at FROM product_tags") {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (id, name, ca) = row?;
            tags += tx.execute(
                "INSERT OR IGNORE INTO product_tags (id, name, created_at) VALUES (?1,?2,?3)",
                params![id, name, ca],
            )? as i64;
        }
    }
    if let Ok(mut stmt) = src.prepare("SELECT product_id, tag_id FROM product_tag_associations") {
        for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (pid, tid) = row?;
            let _ = tx.execute(
                "INSERT OR IGNORE INTO product_tag_associations (product_id, tag_id) VALUES (?1,?2)",
                params![pid, tid],
            );
        }
    }
    // product_nutritions
    if let Ok(mut stmt) = src.prepare(
        "SELECT product_id, reference_quantity, reference_unit, energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g FROM product_nutritions",
    ) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, Option<f64>>(5)?,
                r.get::<_, Option<f64>>(6)?,
                r.get::<_, Option<f64>>(7)?,
                r.get::<_, Option<f64>>(8)?,
            ))
        })? {
            let (pid, rq, ru, e, p, c, f, fi, su) = row?;
            let _ = tx.execute(
                "INSERT OR IGNORE INTO product_nutritions
                 (product_id, reference_quantity, reference_unit, energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![pid, rq, ru, e, p, c, f, fi, su],
            );
        }
    }
    {
        let mut stmt = src.prepare(
            "SELECT id, product_id, quantity, price_cents, store_id, purchased_at, created_at FROM purchases",
        )?;
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
            purchases += tx.execute(
                "INSERT OR IGNORE INTO purchases (id, product_id, quantity, price_cents, store_id, purchased_at, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![id, pid, qty, price, sid, pa, ca],
            )? as i64;
        }
    }
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
            consumptions += tx.execute(
                "INSERT OR IGNORE INTO consumptions (id, product_id, quantity, unit, consumed_at, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
                params![id, pid, qty, unit, ca_at, ca],
            )? as i64;
        }
    }

    tx.commit()?;
    Ok(serde_json::json!({
        "products": products,
        "nutrients": nutrients,
        "tags": tags,
        "purchases": purchases,
        "consumptions": consumptions,
    }))
}

fn copy_body(src: &Connection, dst: &mut Connection) -> Result<serde_json::Value> {
    let tx = dst.transaction()?;
    let mut measurements = 0i64;
    let mut sleeps = 0i64;

    if let Ok(mut stmt) = src.prepare(
        "SELECT id, date, weight_kg, body_fat_pct, skeletal_muscle_pct, visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at FROM measurements",
    ) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<f64>>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<f64>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
            ))
        })? {
            let (id, date, w, bf, sm, vf, bmi, rmr, ca, ua) = row?;
            measurements += tx.execute(
                r#"INSERT OR REPLACE INTO measurements
                   (id, date, weight_kg, body_fat_pct, skeletal_muscle_pct,
                    visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"#,
                params![id, date, w, bf, sm, vf, bmi, rmr, ca, ua],
            )? as i64;
        }
    } else if let Ok(mut stmt) = src.prepare(
        "SELECT id, date, weight_kg, body_fat_pct, skeletal_muscle_pct, created_at, updated_at FROM measurements",
    ) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<f64>>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })? {
            let (id, date, w, bf, sm, ca, ua) = row?;
            measurements += tx.execute(
                r#"INSERT OR REPLACE INTO measurements
                   (id, date, weight_kg, body_fat_pct, skeletal_muscle_pct,
                    visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at)
                   VALUES (?1,?2,?3,?4,?5, NULL, NULL, NULL, ?6, ?7)"#,
                params![id, date, w, bf, sm, ca, ua],
            )? as i64;
        }
    }

    // sleep: table may be sleep or sleep_sessions
    let sleep_table = if table_exists(src, "sleep_sessions") {
        "sleep_sessions"
    } else {
        "sleep"
    };
    if let Ok(mut stmt) = src.prepare(&format!(
        "SELECT id, date, bedtime, wake_time, time_in_bed_minutes, total_sleep_minutes,
                rem_minutes, deep_minutes, light_minutes, awake_minutes,
                sleep_efficiency_pct, sleep_score, subjective_quality, awakenings,
                heart_rate_bpm, hypopnea_per_hr, respiratory_rate, notes, created_at, updated_at
         FROM {sleep_table}"
    )) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<f64>>(10)?,
                r.get::<_, Option<i64>>(11)?,
                r.get::<_, Option<i64>>(12)?,
                r.get::<_, Option<i64>>(13)?,
                r.get::<_, Option<f64>>(14)?,
                r.get::<_, Option<f64>>(15)?,
                r.get::<_, Option<f64>>(16)?,
                r.get::<_, Option<String>>(17)?,
                r.get::<_, String>(18)?,
                r.get::<_, String>(19)?,
            ))
        })? {
            let (
                id,
                date,
                bedtime,
                wake,
                tib,
                total,
                rem,
                deep,
                light,
                awake,
                eff,
                score,
                qual,
                awakenings,
                hr,
                hyp,
                resp,
                notes,
                ca,
                ua,
            ) = row?;
            sleeps += tx.execute(
                "INSERT OR REPLACE INTO sleep
                 (id, date, bedtime, wake_time, time_in_bed_minutes, total_sleep_minutes,
                  rem_minutes, deep_minutes, light_minutes, awake_minutes,
                  sleep_efficiency_pct, sleep_score, subjective_quality, awakenings,
                  heart_rate_bpm, hypopnea_per_hr, respiratory_rate, notes, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
                params![
                    id, date, bedtime, wake, tib, total, rem, deep, light, awake, eff, score, qual,
                    awakenings, hr, hyp, resp, notes, ca, ua
                ],
            )? as i64;
        }
    } else if let Ok(mut stmt) = src.prepare(&format!(
        "SELECT id, date, total_sleep_minutes, created_at, updated_at FROM {sleep_table}"
    )) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })? {
            let (id, date, total, ca, ua) = row?;
            sleeps += tx.execute(
                "INSERT OR REPLACE INTO sleep (id, date, total_sleep_minutes, created_at, updated_at) VALUES (?1,?2,?3,?4,?5)",
                params![id, date, total, ca, ua],
            )? as i64;
        }
    }

    // user_profile
    if let Ok(mut stmt) =
        src.prepare("SELECT height_cm, date_of_birth, updated_at FROM user_profile WHERE id = 1")
    {
        if let Ok(Some((h, dob, ua))) = stmt
            .query_row([], |r| {
                Ok((
                    r.get::<_, Option<f64>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .optional()
        {
            let _ = tx.execute(
                "INSERT OR REPLACE INTO user_profile (id, height_cm, date_of_birth, updated_at) VALUES (1, ?1, ?2, ?3)",
                params![h, dob, ua],
            );
        }
    }

    tx.commit()?;
    Ok(serde_json::json!({"measurements": measurements, "sleep": sleeps}))
}

fn copy_workout(src: &Connection, dst: &mut Connection) -> Result<serde_json::Value> {
    let tx = dst.transaction()?;
    let mut exercises = 0i64;
    let mut workouts = 0i64;
    let mut we = 0i64;
    let mut sets = 0i64;

    if let Ok(mut stmt) = src.prepare(
        "SELECT id, name, category, muscle_groups, equipment, load_type, description, is_custom, created_at FROM exercises",
    ) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5).unwrap_or_else(|_| "weight".into()),
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, String>(8).unwrap_or_default(),
            ))
        })? {
            let (id, name, cat, mg, eq, lt, desc, custom, ca) = row?;
            exercises += tx.execute(
                "INSERT OR IGNORE INTO exercises (id, name, category, muscle_groups, equipment, load_type, description, is_custom, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id, name, cat, mg, eq, lt, desc, custom, ca],
            )? as i64;
        }
    }

    if let Ok(mut stmt) = src.prepare(
        "SELECT id, started_at, finished_at, workout_type, notes, overall_feeling, duration_minutes, created_at FROM workouts",
    ) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, String>(7).unwrap_or_default(),
            ))
        })? {
            let (id, started, finished, wtype, notes, feeling, dur, ca) = row?;
            workouts += tx.execute(
                "INSERT OR IGNORE INTO workouts (id, started_at, finished_at, workout_type, notes, overall_feeling, duration_minutes, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![id, started, finished, wtype, notes, feeling, dur, ca],
            )? as i64;
        }
    }

    if let Ok(mut stmt) = src.prepare(
        r#"SELECT id, workout_id, exercise_id, "order", notes, goal_reps FROM workout_exercises"#,
    ) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })? {
            let (id, wid, eid, order, notes, goal) = row?;
            we += tx.execute(
                r#"INSERT OR IGNORE INTO workout_exercises (id, workout_id, exercise_id, "order", notes, goal_reps)
                 VALUES (?1,?2,?3,?4,?5,?6)"#,
                params![id, wid, eid, order, notes, goal],
            )? as i64;
        }
    }

    // exercise_sets - best effort on common columns
    if let Ok(mut stmt) = src.prepare(
        "SELECT id, workout_exercise_id, set_number, reps, weight_kg, external_load_kg, distance_km,
                duration_seconds, rpe, rir, effective_reps, rest_seconds, notes, side, phase, created_at
         FROM exercise_sets",
    ) {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i32>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, Option<f64>>(5)?,
                r.get::<_, Option<f64>>(6)?,
                r.get::<_, Option<i32>>(7)?,
                r.get::<_, Option<f64>>(8)?,
                r.get::<_, Option<f64>>(9)?,
                r.get::<_, Option<i32>>(10)?,
                r.get::<_, Option<i32>>(11)?,
                r.get::<_, Option<String>>(12)?,
                r.get::<_, Option<String>>(13)?,
                r.get::<_, String>(14).unwrap_or_else(|_| "working".into()),
                r.get::<_, String>(15).unwrap_or_default(),
            ))
        })? {
            let (id, weid, sn, reps, w, el, dist, dur, rpe, rir, er, rest, notes, side, phase, ca) =
                row?;
            sets += tx.execute(
                "INSERT OR IGNORE INTO exercise_sets
                 (id, workout_exercise_id, set_number, reps, weight_kg, external_load_kg, distance_km,
                  duration_seconds, rpe, rir, effective_reps, rest_seconds, notes, side, phase, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![id, weid, sn, reps, w, el, dist, dur, rpe, rir, er, rest, notes, side, phase, ca],
            )? as i64;
        }
    }

    tx.commit()?;
    Ok(serde_json::json!({
        "exercises": exercises,
        "workouts": workouts,
        "workout_exercises": we,
        "sets": sets,
    }))
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

fn detect_domains(tables: &[String]) -> Vec<&'static str> {
    let mut out = vec![];
    let has = |name: &str| tables.iter().any(|t| t.eq_ignore_ascii_case(name));
    if has("workouts") || has("exercise_sets") || has("exercises") {
        out.push("workout");
    }
    if has("measurements") || has("sleep") || has("sleep_sessions") {
        out.push("body");
    }
    if has("products") || has("purchases") || has("consumptions") || has("nutrients") {
        out.push("nutrition");
    }
    out
}
