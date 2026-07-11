//! Import: legacy DB + FIT.

use crate::cli::ImportAction;
use crate::db;
use crate::fit::{parse_fit_path, ImportPlan};
use crate::models::HrZoneProfile;
use crate::utils::print_json;
use anyhow::{anyhow, Result};
use chrono::Datelike;
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub fn handle(action: ImportAction, db_override: Option<&str>, json: bool) -> Result<()> {
    match action {
        ImportAction::Fit {
            path,
            exercise,
            workout_type,
            notes,
            force,
            hr_zone_bounds,
            no_profile_hr,
            dry_run,
        } => handle_fit(
            &path,
            exercise.as_deref(),
            workout_type.as_deref(),
            notes.as_deref(),
            force,
            hr_zone_bounds.as_deref(),
            no_profile_hr,
            dry_run,
            db_override,
            json,
        ),
        ImportAction::Legacy {
            from_db,
            domain,
            dry_run,
        } => handle_legacy_import(&from_db, domain.as_deref(), dry_run, db_override, json),
    }
}

fn parse_hr_zone_bounds(s: &str) -> Result<[f64; 5]> {
    let parts: Vec<&str> = s
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 5 {
        return Err(anyhow!(
            "expected 5 comma-separated bpm bounds for zones 1-5 (e.g. 120,140,160,175,190)"
        ));
    }
    let mut out = [0.0; 5];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p
            .parse::<f64>()
            .map_err(|_| anyhow!("invalid bpm value '{p}'"))?;
        if out[i] <= 0.0 {
            return Err(anyhow!("zone bound {} must be > 0", i + 1));
        }
        if i > 0 && out[i] < out[i - 1] {
            return Err(anyhow!("zone bounds must be non-decreasing"));
        }
    }
    Ok(out)
}

fn file_sha256(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn load_hr_profile(conn: &Connection) -> Option<HrZoneProfile> {
    let dob: Option<String> = conn
        .query_row(
            "SELECT date_of_birth FROM user_profile WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten();
    let dob = dob?;
    // median sleep HR last 14 days
    let mut stmt = conn
        .prepare(
            "SELECT heart_rate_bpm FROM sleep
             WHERE heart_rate_bpm IS NOT NULL
               AND date >= date('now', '-14 days')
             ORDER BY date DESC",
        )
        .ok()?;
    let hrs: Vec<f64> = stmt
        .query_map([], |r| r.get::<_, f64>(0))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    let resting = if hrs.is_empty() {
        None
    } else {
        let mut v = hrs;
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len();
        Some(if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        })
    };
    // age-based HRmax Tanaka
    let age = chrono::NaiveDate::parse_from_str(&dob, "%Y-%m-%d")
        .ok()
        .map(|d| {
            let today = chrono::Local::now().date_naive();
            (today.year() - d.year()) as u32
        })?;
    let hr_max = 208.0 - 0.7 * age as f64;
    let (bounds, method) = if let Some(rhr) = resting {
        if hr_max > rhr {
            let fracs = [0.60, 0.70, 0.80, 0.90, 1.00];
            let hrr = hr_max - rhr;
            let mut out = [0.0; 5];
            for (i, p) in fracs.iter().enumerate() {
                out[i] = (rhr + p * hrr).round();
            }
            (out, format!("karvonen age={age} rhr={rhr:.0}"))
        } else {
            return None;
        }
    } else {
        // %HRmax only
        let fracs = [0.60, 0.70, 0.80, 0.90, 1.00];
        let mut out = [0.0; 5];
        for (i, p) in fracs.iter().enumerate() {
            out[i] = (hr_max * p).round();
        }
        (out, format!("pct_hrmax age={age}"))
    };
    Some(HrZoneProfile {
        date_of_birth: dob,
        resting_hr_bpm: resting,
        bounds,
        method,
    })
}

#[allow(clippy::too_many_arguments)]
fn handle_fit(
    path: &str,
    exercise: Option<&str>,
    workout_type: Option<&str>,
    notes: Option<&str>,
    force: bool,
    hr_zone_bounds: Option<&str>,
    no_profile_hr: bool,
    dry_run: bool,
    db_override: Option<&str>,
    json: bool,
) -> Result<()> {
    let path_obj = Path::new(path);
    if !path_obj.exists() {
        return Err(anyhow!("FIT file not found: {path}"));
    }
    let sha = file_sha256(path_obj)?;
    let activity = parse_fit_path(path_obj).map_err(|e| anyhow!("{e}"))?;
    let bounds = hr_zone_bounds.map(parse_hr_zone_bounds).transpose()?;

    let conn = db::open_db(db_override)?;
    if !force {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM activity_imports WHERE file_sha256 = ?1",
                [&sha],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(anyhow!(
                "file already imported (sha256={sha}); use --force to import again"
            ));
        }
    }

    let profile = if no_profile_hr {
        None
    } else {
        load_hr_profile(&conn)
    };

    let plan = ImportPlan::from_activity(
        &activity,
        workout_type,
        notes,
        path_obj
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path),
        bounds.as_ref(),
        profile.as_ref(),
    )
    .map_err(|e| anyhow!("{e}"))?;

    let exercise_name = exercise
        .map(|s| s.to_lowercase())
        .or_else(|| activity.sport.as_ref().map(|s| s.to_lowercase()))
        .unwrap_or_else(|| "running".to_string());

    let exercise_id: i64 = conn
        .query_row(
            "SELECT id FROM exercises WHERE name = ?1 COLLATE NOCASE",
            [&exercise_name],
            |r| r.get(0),
        )
        .map_err(|_| {
            anyhow!("exercise '{exercise_name}' not in catalog; create it first or pass --exercise")
        })?;

    if dry_run {
        let out = serde_json::json!({
            "dry_run": true,
            "sha256": sha,
            "exercise": exercise_name,
            "exercise_id": exercise_id,
            "started_at": plan.started_at,
            "distance_km": plan.distance_km,
            "duration_seconds": plan.duration_seconds,
            "avg_heart_rate_bpm": plan.avg_heart_rate_bpm,
            "trackpoints": plan.trackpoints.len(),
            "hr_zones": plan.heart_rate_zones,
            "laps": plan.laps.as_ref().map(|l| l.len()),
        });
        if json {
            print_json(&out);
        } else {
            println!("FIT dry-run: {}", serde_json::to_string_pretty(&out)?);
        }
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO workouts (started_at, workout_type, notes, duration_minutes)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            plan.started_at,
            plan.workout_type,
            plan.notes,
            plan.duration_minutes
        ],
    )?;
    let workout_id = tx.last_insert_rowid();
    tx.execute(
        r#"INSERT INTO workout_exercises (workout_id, exercise_id, "order") VALUES (?1, ?2, 1)"#,
        params![workout_id, exercise_id],
    )?;
    let we_id = tx.last_insert_rowid();
    let zones_json = plan
        .heart_rate_zones
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let laps_json = plan.laps.as_ref().map(serde_json::to_string).transpose()?;
    tx.execute(
        "INSERT INTO exercise_sets
         (workout_exercise_id, set_number, distance_km, duration_seconds,
          avg_heart_rate_bpm, max_heart_rate_bpm, avg_pace_min_per_km, calories_burned,
          avg_cadence_spm, total_ascent_m, total_descent_m, heart_rate_zones, laps,
          date_of_birth, resting_hr_bpm, phase)
         VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'full')",
        params![
            we_id,
            plan.distance_km,
            plan.duration_seconds,
            plan.avg_heart_rate_bpm,
            plan.max_heart_rate_bpm,
            plan.avg_pace_min_per_km,
            plan.calories_burned,
            plan.avg_cadence_spm,
            plan.total_ascent_m,
            plan.total_descent_m,
            zones_json,
            laps_json,
            plan.date_of_birth,
            plan.resting_hr_bpm,
        ],
    )?;
    let set_id = tx.last_insert_rowid();
    for tp in &plan.trackpoints {
        tx.execute(
            "INSERT INTO activity_trackpoints
             (exercise_set_id, recorded_at, latitude, longitude, altitude_m,
              heart_rate_bpm, cadence_spm, distance_km, speed_m_s)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                set_id,
                tp.recorded_at,
                tp.latitude,
                tp.longitude,
                tp.altitude_m,
                tp.heart_rate_bpm,
                tp.cadence_spm,
                tp.distance_km,
                tp.speed_m_s,
            ],
        )?;
    }
    // force: allow re-import by deleting old hash first
    if force {
        let _ = tx.execute(
            "DELETE FROM activity_imports WHERE file_sha256 = ?1",
            [&sha],
        );
    }
    tx.execute(
        "INSERT INTO activity_imports
         (workout_id, source_format, source_filename, file_sha256, device_name,
          manufacturer_id, product_id, fit_sport, fit_sub_sport)
         VALUES (?1, 'fit', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            workout_id,
            path_obj.file_name().and_then(|s| s.to_str()),
            sha,
            plan.device_name,
            plan.manufacturer_id,
            plan.product_id,
            plan.fit_sport,
            plan.fit_sub_sport,
        ],
    )?;
    tx.commit()?;

    let out = serde_json::json!({
        "success": true,
        "workout_id": workout_id,
        "set_id": set_id,
        "exercise": exercise_name,
        "sha256": sha,
        "trackpoints": plan.trackpoints.len(),
        "distance_km": plan.distance_km,
        "duration_seconds": plan.duration_seconds,
    });
    if json {
        print_json(&out);
    } else {
        println!(
            "Imported FIT → workout {} set {} ({:.2} km, {} s, {} trackpoints)",
            workout_id,
            set_id,
            plan.distance_km,
            plan.duration_seconds,
            plan.trackpoints.len()
        );
    }
    Ok(())
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

    let run_all = domain_filter.is_none();
    let want = |name: &str| {
        run_all
            || domain_filter
                .map(|d| d.eq_ignore_ascii_case(name))
                .unwrap_or(false)
    };

    if dry_run {
        let mut would_copy = serde_json::Map::new();
        if detected.contains(&"workout") && want("workout") {
            would_copy.insert("workout".into(), estimate_workout_counts(&src)?);
        }
        if detected.contains(&"body") && want("body") {
            would_copy.insert("body".into(), estimate_body_counts(&src)?);
        }
        if detected.contains(&"nutrition") && want("nutrition") {
            would_copy.insert("nutrition".into(), estimate_nutrition_counts(&src)?);
        }
        if json {
            print_json(&serde_json::json!({
                "source": from_path,
                "detected": detected,
                "dry_run": true,
                "would_copy": would_copy,
            }));
        } else {
            println!("Legacy: {}", from_path);
            println!("Detected domains: {:?}", detected);
            println!("Would copy: {}", serde_json::Value::Object(would_copy));
            println!("Dry run — nothing copied.");
        }
        return Ok(());
    }

    let mut target = db::open_db(target_override)?;
    let mut copied = vec![];
    let mut counts = serde_json::Map::new();

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

    // stores must be loaded before purchases (purchases.store_id FK)
    if let Ok(mut stmt) = src.prepare("SELECT id, name, created_at FROM stores") {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (id, name, ca) = row?;
            let _ = tx.execute(
                "INSERT OR IGNORE INTO stores (id, name, created_at) VALUES (?1,?2,?3)",
                params![id, name, ca],
            );
        }
    }
    if let Ok(mut stmt) = src.prepare("SELECT id, name, created_at FROM store_tags") {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (id, name, ca) = row?;
            let _ = tx.execute(
                "INSERT OR IGNORE INTO store_tags (id, name, created_at) VALUES (?1,?2,?3)",
                params![id, name, ca],
            );
        }
    }
    if let Ok(mut stmt) = src.prepare("SELECT store_id, tag_id FROM store_tag_associations") {
        for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (sid, tid) = row?;
            let _ = tx.execute(
                "INSERT OR IGNORE INTO store_tag_associations (store_id, tag_id) VALUES (?1,?2)",
                params![sid, tid],
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

    // micronutrients
    if let Ok(mut stmt) =
        src.prepare("SELECT product_id, nutrient_id, amount, unit FROM product_micronutrients")
    {
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            let (pid, nid, amt, unit) = row?;
            let _ = tx.execute(
                "INSERT OR IGNORE INTO product_micronutrients (product_id, nutrient_id, amount, unit) VALUES (?1,?2,?3,?4)",
                params![pid, nid, amt, unit],
            );
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

/// Preferred columns for parent workout tables. Copied when present on both DBs.
/// Real repslog DBs omit `finished_at` (recomplog-only); older ones may omit `load_type` / `goal_reps`.
const EXERCISE_COLUMNS: &[&str] = &[
    "id",
    "name",
    "category",
    "muscle_groups",
    "equipment",
    "load_type",
    "description",
    "is_custom",
    "created_at",
];

const WORKOUT_COLUMNS: &[&str] = &[
    "id",
    "started_at",
    "finished_at",
    "workout_type",
    "notes",
    "overall_feeling",
    "duration_minutes",
    "created_at",
];

const WORKOUT_EXERCISE_COLUMNS: &[&str] = &[
    "id",
    "workout_id",
    "exercise_id",
    "order",
    "notes",
    "goal_reps",
];

/// Preferred `exercise_sets` columns (baseline + cardio/provenance). Copied when present on both DBs.
const EXERCISE_SET_COLUMNS: &[&str] = &[
    "id",
    "workout_exercise_id",
    "set_number",
    "reps",
    "weight_kg",
    "external_load_kg",
    "distance_km",
    "duration_seconds",
    "rpe",
    "rir",
    "effective_reps",
    "cluster_id",
    "rest_seconds",
    "notes",
    "side",
    "phase",
    "extra_metrics",
    "avg_heart_rate_bpm",
    "max_heart_rate_bpm",
    "avg_pace_min_per_km",
    "calories_burned",
    "avg_cadence_spm",
    "total_ascent_m",
    "total_descent_m",
    "date_of_birth",
    "resting_hr_bpm",
    "heart_rate_zones",
    "laps",
    "created_at",
];

const ACTIVITY_IMPORT_COLUMNS: &[&str] = &[
    "id",
    "workout_id",
    "source_format",
    "source_filename",
    "file_sha256",
    "device_name",
    "manufacturer_id",
    "product_id",
    "fit_sport",
    "fit_sub_sport",
    "imported_at",
];

const ACTIVITY_TRACKPOINT_COLUMNS: &[&str] = &[
    "id",
    "exercise_set_id",
    "recorded_at",
    "latitude",
    "longitude",
    "altitude_m",
    "heart_rate_bpm",
    "cadence_spm",
    "distance_km",
    "speed_m_s",
];

fn copy_workout(src: &Connection, dst: &mut Connection) -> Result<serde_json::Value> {
    let tx = dst.transaction()?;
    let mut sets_with_zones = 0i64;
    let mut sets_with_laps = 0i64;
    let mut activity_imports = 0i64;
    let mut activity_trackpoints = 0i64;
    let mut imports_skipped = 0i64;
    let mut trackpoints_skipped = 0i64;

    // Exercises: match by name when target already has seeds / prior data; remap FKs.
    let (exercises, exercise_id_map) = if table_exists(src, "exercises") {
        copy_exercises_with_remap(src, &tx)?
    } else {
        (0, std::collections::HashMap::new())
    };
    // Workouts: column intersection so real repslog (no finished_at) works.
    let workouts = if table_exists(src, "workouts") {
        copy_rows_by_columns(src, &tx, "workouts", WORKOUT_COLUMNS)?
    } else {
        0
    };
    let we = if table_exists(src, "workout_exercises") {
        copy_workout_exercises(src, &tx, &exercise_id_map)?
    } else {
        0
    };

    // exercise_sets — intersection of preferred columns present on both source and target
    let mut sets = 0i64;
    let mut weight_zero_to_null = 0i64;
    if table_exists(src, "exercise_sets") {
        let cols = intersect_columns(
            EXERCISE_SET_COLUMNS,
            &table_columns(src, "exercise_sets")?,
            &table_columns(&tx, "exercise_sets")?,
        );
        if !cols.is_empty() {
            let zones_idx = cols.iter().position(|c| c == "heart_rate_zones");
            let laps_idx = cols.iter().position(|c| c == "laps");
            let weight_idx = cols.iter().position(|c| c == "weight_kg");
            let col_list = sql_column_list(&cols);
            let placeholders = (1..=cols.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let select_sql = format!("SELECT {col_list} FROM exercise_sets ORDER BY id");
            let insert_sql =
                format!("INSERT OR IGNORE INTO exercise_sets ({col_list}) VALUES ({placeholders})");
            let mut select = src.prepare(&select_sql)?;
            let mut insert = tx.prepare(&insert_sql)?;
            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let mut values = row_values(row, cols.len())?;
                // Legacy bodyweight sets often store weight_kg=0 meaning "unloaded".
                // recomplog treats 0 as invalid (min 0.001); NULL is the correct sentinel.
                let mut normalized_zero_weight = false;
                if let Some(wi) = weight_idx {
                    if value_is_zero_number(&values[wi]) {
                        values[wi] = Value::Null;
                        normalized_zero_weight = true;
                    }
                }
                let n = insert.execute(params_from_iter(values.iter()))? as i64;
                sets += n;
                if n > 0 {
                    if normalized_zero_weight {
                        weight_zero_to_null += 1;
                    }
                    if zones_idx.is_some_and(|i| !matches!(values[i], Value::Null)) {
                        sets_with_zones += 1;
                    }
                    if laps_idx.is_some_and(|i| !matches!(values[i], Value::Null)) {
                        sets_with_laps += 1;
                    }
                }
            }
        }
    }

    // activity_imports (after workouts; skip orphans)
    if table_exists(src, "activity_imports") {
        let cols = intersect_columns(
            ACTIVITY_IMPORT_COLUMNS,
            &table_columns(src, "activity_imports")?,
            &table_columns(&tx, "activity_imports")?,
        );
        if let Some(wid_idx) = cols.iter().position(|c| c == "workout_id") {
            let parent_ids = load_id_set(&tx, "workouts")?;
            let col_list = sql_column_list(&cols);
            let placeholders = (1..=cols.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let select_sql = format!("SELECT {col_list} FROM activity_imports ORDER BY id");
            let insert_sql = format!(
                "INSERT OR IGNORE INTO activity_imports ({col_list}) VALUES ({placeholders})"
            );
            let mut select = src.prepare(&select_sql)?;
            let mut insert = tx.prepare(&insert_sql)?;
            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let values = row_values(row, cols.len())?;
                let parent = value_as_i64(&values[wid_idx]);
                if parent.map(|id| !parent_ids.contains(&id)).unwrap_or(true) {
                    imports_skipped += 1;
                    continue;
                }
                activity_imports += insert.execute(params_from_iter(values.iter()))? as i64;
            }
        }
    }

    // activity_trackpoints (after sets; skip orphans)
    if table_exists(src, "activity_trackpoints") {
        let cols = intersect_columns(
            ACTIVITY_TRACKPOINT_COLUMNS,
            &table_columns(src, "activity_trackpoints")?,
            &table_columns(&tx, "activity_trackpoints")?,
        );
        if let Some(sid_idx) = cols.iter().position(|c| c == "exercise_set_id") {
            let parent_ids = load_id_set(&tx, "exercise_sets")?;
            let col_list = sql_column_list(&cols);
            let placeholders = (1..=cols.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let select_sql = format!("SELECT {col_list} FROM activity_trackpoints ORDER BY id");
            let insert_sql = format!(
                "INSERT OR IGNORE INTO activity_trackpoints ({col_list}) VALUES ({placeholders})"
            );
            let mut select = src.prepare(&select_sql)?;
            let mut insert = tx.prepare(&insert_sql)?;
            let mut rows = select.query([])?;
            while let Some(row) = rows.next()? {
                let values = row_values(row, cols.len())?;
                let parent = value_as_i64(&values[sid_idx]);
                if parent.map(|id| !parent_ids.contains(&id)).unwrap_or(true) {
                    trackpoints_skipped += 1;
                    continue;
                }
                activity_trackpoints += insert.execute(params_from_iter(values.iter()))? as i64;
            }
        }
    }

    tx.commit()?;
    Ok(serde_json::json!({
        "exercises": exercises,
        "workouts": workouts,
        "workout_exercises": we,
        "sets": sets,
        "activity_imports": activity_imports,
        "activity_trackpoints": activity_trackpoints,
        "sets_with_zones": sets_with_zones,
        "sets_with_laps": sets_with_laps,
        "weight_zero_to_null": weight_zero_to_null,
        "imports_skipped": imports_skipped,
        "trackpoints_skipped": trackpoints_skipped,
    }))
}

/// Quote SQL identifiers that are reserved words (e.g. `order`).
fn sql_ident(col: &str) -> String {
    if col.eq_ignore_ascii_case("order") {
        format!("\"{col}\"")
    } else {
        col.to_string()
    }
}

fn sql_column_list(cols: &[String]) -> String {
    cols.iter()
        .map(|c| sql_ident(c))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Copy all rows of `table` using the intersection of `preferred` columns present on both DBs.
fn copy_rows_by_columns(
    src: &Connection,
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    preferred: &[&str],
) -> Result<i64> {
    let cols = intersect_columns(
        preferred,
        &table_columns(src, table)?,
        &table_columns(tx, table)?,
    );
    if cols.is_empty() {
        return Ok(0);
    }
    let col_list = sql_column_list(&cols);
    let placeholders = (1..=cols.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!("SELECT {col_list} FROM {table} ORDER BY id");
    let insert_sql = format!("INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({placeholders})");
    let mut select = src.prepare(&select_sql)?;
    let mut insert = tx.prepare(&insert_sql)?;
    let mut rows = select.query([])?;
    let mut n = 0i64;
    while let Some(row) = rows.next()? {
        let values = row_values(row, cols.len())?;
        n += insert.execute(params_from_iter(values.iter()))? as i64;
    }
    Ok(n)
}

/// Import exercises, preserving source IDs when free, otherwise remapping by name or new auto IDs.
/// Returns (newly_inserted_count, source_id → target_id).
fn copy_exercises_with_remap(
    src: &Connection,
    tx: &rusqlite::Transaction<'_>,
) -> Result<(i64, std::collections::HashMap<i64, i64>)> {
    use std::collections::HashMap;

    let src_cols = table_columns(src, "exercises")?;
    let dst_cols = table_columns(tx, "exercises")?;
    let cols = intersect_columns(EXERCISE_COLUMNS, &src_cols, &dst_cols);
    if cols.is_empty() || !cols.iter().any(|c| c == "id") || !cols.iter().any(|c| c == "name") {
        return Ok((0, HashMap::new()));
    }
    let id_idx = cols.iter().position(|c| c == "id").unwrap();
    let name_idx = cols.iter().position(|c| c == "name").unwrap();
    let col_list = sql_column_list(&cols);
    let select_sql = format!("SELECT {col_list} FROM exercises ORDER BY id");
    let mut select = src.prepare(&select_sql)?;
    let mut rows = select.query([])?;

    let mut map = HashMap::new();
    let mut inserted = 0i64;

    while let Some(row) = rows.next()? {
        let values = row_values(row, cols.len())?;
        let src_id = value_as_i64(&values[id_idx])
            .ok_or_else(|| anyhow!("exercise row missing integer id during legacy import"))?;
        let name = match &values[name_idx] {
            Value::Text(s) => s.clone(),
            other => {
                return Err(anyhow!("exercise id {src_id} has non-text name: {other:?}"));
            }
        };

        // Prefer existing exercise with same name (handles seeded catalogs).
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM exercises WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(tid) = existing {
            map.insert(src_id, tid);
            continue;
        }

        let id_taken: bool = tx
            .query_row(
                "SELECT 1 FROM exercises WHERE id = ?1",
                params![src_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);

        if id_taken {
            // Keep source fields but allocate a new primary key.
            let cols_no_id: Vec<String> = cols.iter().filter(|c| *c != "id").cloned().collect();
            let vals_no_id: Vec<Value> = cols
                .iter()
                .zip(values.iter())
                .filter(|(c, _)| *c != "id")
                .map(|(_, v)| v.clone())
                .collect();
            let col_list_ni = sql_column_list(&cols_no_id);
            let ph = (1..=cols_no_id.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            tx.execute(
                &format!("INSERT INTO exercises ({col_list_ni}) VALUES ({ph})"),
                params_from_iter(vals_no_id.iter()),
            )?;
            let new_id = tx.last_insert_rowid();
            map.insert(src_id, new_id);
            inserted += 1;
        } else {
            let ph = (1..=cols.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            tx.execute(
                &format!("INSERT INTO exercises ({col_list}) VALUES ({ph})"),
                params_from_iter(values.iter()),
            )?;
            map.insert(src_id, src_id);
            inserted += 1;
        }
    }

    Ok((inserted, map))
}

/// Copy workout_exercises, remapping exercise_id through `exercise_id_map`.
fn copy_workout_exercises(
    src: &Connection,
    tx: &rusqlite::Transaction<'_>,
    exercise_id_map: &std::collections::HashMap<i64, i64>,
) -> Result<i64> {
    let cols = intersect_columns(
        WORKOUT_EXERCISE_COLUMNS,
        &table_columns(src, "workout_exercises")?,
        &table_columns(tx, "workout_exercises")?,
    );
    if cols.is_empty() {
        return Ok(0);
    }
    let eid_idx = cols.iter().position(|c| c == "exercise_id");
    let col_list = sql_column_list(&cols);
    let placeholders = (1..=cols.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!("SELECT {col_list} FROM workout_exercises ORDER BY id");
    let insert_sql =
        format!("INSERT OR IGNORE INTO workout_exercises ({col_list}) VALUES ({placeholders})");
    let mut select = src.prepare(&select_sql)?;
    let mut insert = tx.prepare(&insert_sql)?;
    let mut rows = select.query([])?;
    let mut n = 0i64;
    while let Some(row) = rows.next()? {
        let mut values = row_values(row, cols.len())?;
        if let Some(ei) = eid_idx {
            if let Some(src_eid) = value_as_i64(&values[ei]) {
                let target_eid = exercise_id_map.get(&src_eid).copied().unwrap_or(src_eid);
                values[ei] = Value::Integer(target_eid);
            }
        }
        n += insert.execute(params_from_iter(values.iter()))? as i64;
    }
    Ok(n)
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
    let mut out = HashSet::new();
    for n in names {
        out.insert(n?);
    }
    Ok(out)
}

fn intersect_columns(
    preferred: &[&str],
    src: &HashSet<String>,
    dst: &HashSet<String>,
) -> Vec<String> {
    preferred
        .iter()
        .filter(|c| src.contains(**c) && dst.contains(**c))
        .map(|s| (*s).to_string())
        .collect()
}

fn row_values(row: &rusqlite::Row<'_>, n: usize) -> rusqlite::Result<Vec<Value>> {
    let mut values = Vec::with_capacity(n);
    for i in 0..n {
        values.push(row.get(i)?);
    }
    Ok(values)
}

fn value_as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i),
        Value::Real(f) => Some(*f as i64),
        Value::Text(s) => s.parse().ok(),
        _ => None,
    }
}

fn value_is_zero_number(v: &Value) -> bool {
    match v {
        Value::Integer(0) => true,
        Value::Real(f) if *f == 0.0 => true,
        Value::Text(s) => matches!(s.trim(), "0" | "0.0" | "0.00"),
        _ => false,
    }
}

fn load_id_set(conn: &Connection, table: &str) -> Result<HashSet<i64>> {
    let mut stmt = conn.prepare(&format!("SELECT id FROM {table}"))?;
    let ids = stmt.query_map([], |r| r.get(0))?;
    let mut out = HashSet::new();
    for id in ids {
        out.insert(id?);
    }
    Ok(out)
}

fn table_count(conn: &Connection, table: &str) -> Result<i64> {
    if !table_exists(conn, table) {
        return Ok(0);
    }
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
    Ok(n)
}

fn estimate_workout_counts(src: &Connection) -> Result<serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert(
        "exercises".into(),
        serde_json::json!(table_count(src, "exercises")?),
    );
    m.insert(
        "workouts".into(),
        serde_json::json!(table_count(src, "workouts")?),
    );
    m.insert(
        "workout_exercises".into(),
        serde_json::json!(table_count(src, "workout_exercises")?),
    );
    m.insert(
        "sets".into(),
        serde_json::json!(table_count(src, "exercise_sets")?),
    );
    if table_exists(src, "activity_imports") {
        m.insert(
            "activity_imports".into(),
            serde_json::json!(table_count(src, "activity_imports")?),
        );
    }
    if table_exists(src, "activity_trackpoints") {
        m.insert(
            "activity_trackpoints".into(),
            serde_json::json!(table_count(src, "activity_trackpoints")?),
        );
    }
    Ok(serde_json::Value::Object(m))
}

fn estimate_body_counts(src: &Connection) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "measurements": table_count(src, "measurements")?,
        "sleep": table_count(src, if table_exists(src, "sleep_sessions") {
            "sleep_sessions"
        } else {
            "sleep"
        })?,
    }))
}

fn estimate_nutrition_counts(src: &Connection) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "products": table_count(src, "products")?,
        "purchases": table_count(src, "purchases")?,
        "consumptions": table_count(src, "consumptions")?,
        "nutrients": table_count(src, "nutrients")?,
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
