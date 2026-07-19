//! Nutrition domain handlers (nutlog parity under grouped CLI).

use crate::cli::{
    ConsumptionAction, InfoodsAction, MicronutrientAction, NutritionAction, ProductAction,
    ProductNutritionAction, PurchaseAction, StoreAction, TagModifyAction, TaxonomyAction,
};
use crate::db;
use crate::entity_audit;
use crate::infoods::{self, EnsureMode};
use crate::macro_names::{is_macronutrient_name, macro_flag_hint};
use crate::models::Success;
use crate::sanity::SanityWarning;
use crate::utils::{
    parse_date_to_ymd, parse_rfc3339_instant_for_db, parse_rfc3339_to_utc, print_error_json,
    print_json, quiet_print, refuse_consumption_midnight,
};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use strsim::jaro_winkler;

/// Macros stored as columns on `product_nutritions`.
#[derive(Debug, Default, Clone)]
struct ProductMacros {
    energy_kcal: Option<f64>,
    protein_g: Option<f64>,
    carbohydrates_g: Option<f64>,
    fat_g: Option<f64>,
    fiber_g: Option<f64>,
    sugars_g: Option<f64>,
    saturated_fat_g: Option<f64>,
    trans_fat_g: Option<f64>,
    monounsaturated_fat_g: Option<f64>,
    polyunsaturated_fat_g: Option<f64>,
    cholesterol_mg: Option<f64>,
    added_sugars_g: Option<f64>,
}

/// Classic six macros required on every product nutrition set (all non-null).
/// Explicit `Some(0.0)` is allowed; `None` is not.
fn classic_macro_fields(macros: &ProductMacros) -> [(&'static str, &'static str, Option<f64>); 6] {
    [
        ("energy_kcal", "--energy-kcal", macros.energy_kcal),
        ("protein_g", "--protein-g", macros.protein_g),
        (
            "carbohydrates_g",
            "--carbohydrates-g",
            macros.carbohydrates_g,
        ),
        ("fat_g", "--fat-g", macros.fat_g),
        ("fiber_g", "--fiber-g", macros.fiber_g),
        ("sugars_g", "--sugars-g", macros.sugars_g),
    ]
}

fn missing_classic_macro_flags(macros: &ProductMacros) -> Vec<&'static str> {
    classic_macro_fields(macros)
        .into_iter()
        .filter(|(_, _, v)| v.is_none())
        .map(|(_, flag, _)| flag)
        .collect()
}

fn classic_macros_complete_opts(
    energy_kcal: Option<f64>,
    protein_g: Option<f64>,
    carbohydrates_g: Option<f64>,
    fat_g: Option<f64>,
    fiber_g: Option<f64>,
    sugars_g: Option<f64>,
) -> bool {
    energy_kcal.is_some()
        && protein_g.is_some()
        && carbohydrates_g.is_some()
        && fat_g.is_some()
        && fiber_g.is_some()
        && sugars_g.is_some()
}

/// Validate classic six for nutrition set. Returns non-fatal zero warnings.
fn validate_classic_macros_for_set(macros: &ProductMacros) -> Result<Vec<SanityWarning>> {
    let missing = missing_classic_macro_flags(macros);
    if !missing.is_empty() {
        return Err(anyhow!(
            "product nutrition requires all classic macros (energy/protein/carbs/fat/fiber/sugars); \
             missing: {}. Use explicit 0 when truly zero (rare; emits a warning)",
            missing.join(", ")
        ));
    }
    let mut warnings = Vec::new();
    // Explicit zero is valid but rare — warn so agents/users double-check.
    for (field, _, val) in classic_macro_fields(macros) {
        if let Some(v) = val {
            if v == 0.0 {
                warnings.push(SanityWarning {
                    field: field.into(),
                    kind: "zero_macro".into(),
                    message: format!(
                        "{field}=0 is unusual; confirm this is known-zero (not missing data). \
                         Explicit 0 is correct when true; inspect later with `db check --zero-macros`"
                    ),
                    previous_value: None,
                    previous_date: None,
                    new_value: Some(0.0),
                    delta: None,
                    allowed_delta: None,
                    days_gap: None,
                });
            }
        }
    }
    Ok(warnings)
}

fn emit_nutrition_warnings(warnings: &[SanityWarning], quiet: bool) {
    if quiet {
        return;
    }
    for w in warnings {
        eprintln!("Warning: {}", w.message);
    }
}

pub fn handle(
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
        NutritionAction::Micronutrient { action } => {
            handle_micronutrient(action, db_override, json, quiet)
        }
        NutritionAction::Infoods { action } => handle_infoods(action, db_override, json, quiet),
        NutritionAction::ProductTag { action } => handle_taxonomy(
            "product_tags",
            "product_tag_associations",
            "tag_id",
            action,
            db_override,
            json,
            quiet,
        ),
        NutritionAction::Store { action } => handle_store(action, db_override, json, quiet),
        NutritionAction::StoreTag { action } => handle_taxonomy(
            "store_tags",
            "store_tag_associations",
            "tag_id",
            action,
            db_override,
            json,
            quiet,
        ),
    }
}

/// Split on non-alphanumeric so "Iron Bar" / "vit-d" tokenize usefully.
fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Score how well a single name token matches a single query token.
///
/// Full-string / bare Jaro-Winkler is too loose for short queries (e.g. "iron"
/// matching "virgin", "original", "rojo"). Prefer exact, prefix, and substring
/// token matches; only accept high Jaro-Winkler when token lengths are similar.
fn word_match_score(word: &str, query_word: &str) -> f64 {
    if word == query_word {
        return 1.0;
    }
    if word.starts_with(query_word) {
        return 0.9;
    }
    if query_word.starts_with(word) && word.len() >= 2 {
        return 0.85;
    }
    if query_word.len() >= 2 && word.contains(query_word) {
        return 0.8;
    }
    let len_ratio = word.len() as f64 / query_word.len() as f64;
    if (0.5..=2.0).contains(&len_ratio) {
        let jw = jaro_winkler(word, query_word);
        if jw >= 0.85 {
            return jw;
        }
    }
    0.0
}

/// Score how well `name` matches `query` using token-aware matching (nutlog parity).
fn name_match_score(name: &str, query: &str) -> f64 {
    let name_lower = name.to_lowercase();
    let query_lower = query.to_lowercase();

    if name_lower == query_lower {
        return 1.0;
    }
    if name_lower.contains(&query_lower) {
        return 0.95;
    }

    let name_words = tokenize(name);
    let query_words = tokenize(query);
    if query_words.is_empty() {
        return 0.0;
    }

    let mut total = 0.0;
    let mut matched = 0u32;
    for qw in &query_words {
        let best = name_words
            .iter()
            .map(|w| word_match_score(w, qw))
            .fold(0.0_f64, f64::max);
        if best > 0.0 {
            matched += 1;
            total += best;
        }
    }

    // Every query token must match something; otherwise reject.
    if matched != query_words.len() as u32 {
        return 0.0;
    }

    total / query_words.len() as f64
}

fn fuzzy_rank(items: Vec<(i64, String)>, query: &str) -> Vec<(i64, String, f64)> {
    const MIN_SCORE: f64 = 0.5;
    let mut ranked: Vec<_> = items
        .into_iter()
        .map(|(id, name)| {
            let score = name_match_score(&name, query);
            (id, name, score)
        })
        .filter(|(_, _, s)| *s >= MIN_SCORE)
        .collect();
    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(30);
    ranked
}

fn ensure_tag(conn: &Connection, table: &str, name: &str) -> Result<i64> {
    let now = db::now_utc();
    conn.execute(
        &format!("INSERT OR IGNORE INTO {table} (name, created_at) VALUES (?1, ?2)"),
        params![name, now],
    )?;
    Ok(conn.query_row(
        &format!("SELECT id FROM {table} WHERE name = ?1"),
        [name],
        |r| r.get(0),
    )?)
}

fn set_product_nutrition(
    conn: &Connection,
    id: i64,
    reference_quantity: f64,
    reference_unit: &str,
    macros: &ProductMacros,
    micros: &[(String, f64, String)],
) -> Result<()> {
    let exists: Option<i64> = conn
        .query_row("SELECT id FROM products WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    if exists.is_none() {
        return Err(anyhow!("product not found"));
    }
    let (reference_quantity, reference_unit) =
        crate::nutrition_units::validate_product_reference(reference_quantity, reference_unit)?;
    reject_macro_names_as_micros(micros)?;
    conn.execute(
        "INSERT INTO product_nutritions
         (product_id, reference_quantity, reference_unit,
          energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g,
          saturated_fat_g, trans_fat_g, monounsaturated_fat_g, polyunsaturated_fat_g,
          cholesterol_mg, added_sugars_g)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(product_id) DO UPDATE SET
           reference_quantity=excluded.reference_quantity,
           reference_unit=excluded.reference_unit,
           energy_kcal=excluded.energy_kcal,
           protein_g=excluded.protein_g,
           carbohydrates_g=excluded.carbohydrates_g,
           fat_g=excluded.fat_g,
           fiber_g=excluded.fiber_g,
           sugars_g=excluded.sugars_g,
           saturated_fat_g=excluded.saturated_fat_g,
           trans_fat_g=excluded.trans_fat_g,
           monounsaturated_fat_g=excluded.monounsaturated_fat_g,
           polyunsaturated_fat_g=excluded.polyunsaturated_fat_g,
           cholesterol_mg=excluded.cholesterol_mg,
           added_sugars_g=excluded.added_sugars_g",
        params![
            id,
            reference_quantity,
            reference_unit,
            macros.energy_kcal,
            macros.protein_g,
            macros.carbohydrates_g,
            macros.fat_g,
            macros.fiber_g,
            macros.sugars_g,
            macros.saturated_fat_g,
            macros.trans_fat_g,
            macros.monounsaturated_fat_g,
            macros.polyunsaturated_fat_g,
            macros.cholesterol_mg,
            macros.added_sugars_g,
        ],
    )?;
    // Replace micronutrients when provided
    if !micros.is_empty() {
        conn.execute(
            "DELETE FROM product_micronutrients WHERE product_id = ?1",
            [id],
        )?;
        for (name, amount, unit) in micros {
            let ensured = infoods::ensure_micronutrient(conn, name, unit, EnsureMode::ProductSet)?;
            let unit = infoods::normalize_unit(unit);
            conn.execute(
                "INSERT INTO product_micronutrients (product_id, micronutrient_id, amount, unit)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, ensured.id, amount, unit],
            )?;
        }
    }
    let micro_n = micros.len();
    let summary = if micro_n > 0 {
        format!("nutrition set ({micro_n} micronutrient(s))")
    } else {
        "nutrition set".into()
    };
    entity_audit::append_catalog(
        conn,
        entity_audit::entity::PRODUCT,
        id,
        &summary,
        None,
        Some(&serde_json::json!({
            "reference_quantity": reference_quantity,
            "reference_unit": reference_unit,
            "energy_kcal": macros.energy_kcal,
            "protein_g": macros.protein_g,
            "carbohydrates_g": macros.carbohydrates_g,
            "fat_g": macros.fat_g,
        })),
    )?;
    Ok(())
}

fn reject_macro_names_as_micros(micros: &[(String, f64, String)]) -> Result<()> {
    for (name, _, _) in micros {
        if let Some(flag) = macro_flag_hint(name) {
            return Err(anyhow!(
                "'{name}' is a macronutrient; use {flag} on product nutrition set \
                 (not --micronutrient)"
            ));
        }
    }
    Ok(())
}

fn parse_micronutrient_triples(flat: &[String]) -> Result<Vec<(String, f64, String)>> {
    if flat.len() % 3 != 0 {
        return Err(anyhow!(
            "--micronutrient requires triples: NAME AMOUNT UNIT (got {} values)",
            flat.len()
        ));
    }
    let mut out = vec![];
    for chunk in flat.chunks(3) {
        let amount: f64 = chunk[1]
            .parse()
            .map_err(|_| anyhow!("invalid micronutrient amount '{}'", chunk[1]))?;
        out.push((chunk[0].clone(), amount, chunk[2].clone()));
    }
    reject_macro_names_as_micros(&out)?;
    Ok(out)
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
            entity_audit::append_create(&conn, entity_audit::entity::PRODUCT, id, None)?;
            if let Some(ts) = tags {
                for t in ts {
                    let t = t.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let tag_id = ensure_tag(&conn, "product_tags", t)?;
                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO product_tag_associations (product_id, tag_id) VALUES (?1, ?2)",
                        params![id, tag_id],
                    );
                }
            }
            if json {
                print_json(&Success::created(
                    id,
                    name.clone(),
                    format!("product created: {name}"),
                ));
            } else {
                quiet_print(quiet, format!("Created product {id} ({name})"));
            }
        }
        ProductAction::List => {
            // Active products only (retired merge aliases hidden).
            let mut stmt = conn.prepare(
                "SELECT id, name, created_at FROM products
                 WHERE retired_at IS NULL
                 ORDER BY id DESC",
            )?;
            let products: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "created_at": r.get::<_, String>(2)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&products);
            } else if products.is_empty() {
                println!("(no products)");
            } else {
                for p in &products {
                    println!("{}: {}", p["id"], p["name"]);
                }
            }
        }
        ProductAction::Search { name, tag } => {
            if let Some(n) = name {
                let mut stmt =
                    conn.prepare("SELECT id, name FROM products WHERE retired_at IS NULL")?;
                let cands: Vec<_> = stmt
                    .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                    .filter_map(|r| r.ok())
                    .collect();
                let ranked = fuzzy_rank(cands, &n);
                if json {
                    let out: Vec<_> = ranked
                        .iter()
                        .map(|(id, nm, score)| {
                            serde_json::json!({"id": id, "name": nm, "score": score})
                        })
                        .collect();
                    print_json(&out);
                } else {
                    for (id, nm, score) in ranked {
                        println!("{id}: {nm} ({score:.2})");
                    }
                }
            } else if let Some(t) = tag {
                let mut stmt = conn.prepare(
                    "SELECT p.id, p.name FROM products p
                     JOIN product_tag_associations a ON a.product_id = p.id
                     JOIN product_tags t ON t.id = a.tag_id
                     WHERE t.name = ?1 COLLATE NOCASE
                       AND p.retired_at IS NULL
                     ORDER BY p.name",
                )?;
                let rows: Vec<_> = stmt
                    .query_map([&t], |r| {
                        Ok(serde_json::json!({
                            "id": r.get::<_, i64>(0)?,
                            "name": r.get::<_, String>(1)?,
                        }))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                if json {
                    print_json(&rows);
                } else {
                    for p in &rows {
                        println!("{}: {}", p["id"], p["name"]);
                    }
                }
            } else {
                return Err(anyhow!("provide --name or --tag for search"));
            }
        }
        ProductAction::Show { id } => show_product(&conn, id, json)?,
        ProductAction::Rename { id, name } => {
            let old_name: Option<String> = conn
                .query_row("SELECT name FROM products WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            let Some(old_name) = old_name else {
                return Err(anyhow!("product not found"));
            };
            let n = conn.execute(
                "UPDATE products SET name = ?1, updated_at = ?2 WHERE id = ?3",
                params![name, db::now_utc(), id],
            )?;
            if n == 0 {
                return Err(anyhow!("product not found"));
            }
            let fields = [entity_audit::FieldChange::new(
                "name",
                serde_json::json!(old_name),
                serde_json::json!(name),
            )];
            entity_audit::append_catalog(
                &conn,
                entity_audit::entity::PRODUCT,
                id,
                &format!("renamed to {name}"),
                Some(&fields),
                None,
            )?;
            if json {
                print_json(&Success::created(id, name.clone(), "product renamed"));
            } else {
                quiet_print(quiet, format!("Renamed product {id} to {name}"));
            }
        }
        ProductAction::Delete { id, force } => {
            if !force {
                let purchases: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM purchases WHERE product_id = ?1",
                    [id],
                    |r| r.get(0),
                )?;
                let consumptions: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM consumptions WHERE product_id = ?1",
                    [id],
                    |r| r.get(0),
                )?;
                if purchases > 0 || consumptions > 0 {
                    return Err(anyhow!(
                        "product has purchases/consumptions; use --force to delete"
                    ));
                }
            }
            let n = conn.execute("DELETE FROM products WHERE id = ?1", [id])?;
            if n == 0 {
                return Err(anyhow!("product not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted product {id}"));
            }
        }
        ProductAction::Merge {
            into,
            from,
            name,
            dry_run,
        } => {
            merge_products(&conn, into, &from, name.as_deref(), dry_run, json, quiet)?;
        }
        ProductAction::Nutrition {
            action:
                ProductNutritionAction::Set {
                    id,
                    reference_quantity,
                    reference_unit,
                    energy_kcal,
                    protein_g,
                    carbohydrates_g,
                    fat_g,
                    fiber_g,
                    sugars_g,
                    saturated_fat_g,
                    trans_fat_g,
                    monounsaturated_fat_g,
                    polyunsaturated_fat_g,
                    cholesterol_mg,
                    added_sugars_g,
                    micronutrient,
                    json_file,
                },
        } => {
            let (rq, ru, macros, micros) = if let Some(path) = json_file {
                let raw = std::fs::read_to_string(&path)?;
                let v: serde_json::Value = serde_json::from_str(&raw)?;
                let ref_obj = &v["reference"];
                let rq = ref_obj["quantity"]
                    .as_f64()
                    .or_else(|| v["reference_quantity"].as_f64())
                    .ok_or_else(|| anyhow!("json-file missing reference quantity"))?;
                let ru = ref_obj["unit"]
                    .as_str()
                    .or_else(|| v["reference_unit"].as_str())
                    .unwrap_or("g")
                    .to_string();
                // Validated/normalized in set_product_nutrition.
                let m = &v["macros"];
                let mut micros = vec![];
                if let Some(arr) = v["micronutrients"].as_array() {
                    for mi in arr {
                        micros.push((
                            mi["name"].as_str().unwrap_or("").to_string(),
                            mi["amount"].as_f64().unwrap_or(0.0),
                            mi["unit"].as_str().unwrap_or("mg").to_string(),
                        ));
                    }
                }
                let macros = ProductMacros {
                    energy_kcal: m["energy_kcal"].as_f64().or(v["energy_kcal"].as_f64()),
                    protein_g: m["protein_g"].as_f64().or(v["protein_g"].as_f64()),
                    carbohydrates_g: m["carbohydrates_g"]
                        .as_f64()
                        .or(v["carbohydrates_g"].as_f64()),
                    fat_g: m["fat_g"].as_f64().or(v["fat_g"].as_f64()),
                    fiber_g: m["fiber_g"].as_f64().or(v["fiber_g"].as_f64()),
                    sugars_g: m["sugars_g"].as_f64().or(v["sugars_g"].as_f64()),
                    saturated_fat_g: m["saturated_fat_g"]
                        .as_f64()
                        .or(v["saturated_fat_g"].as_f64()),
                    trans_fat_g: m["trans_fat_g"].as_f64().or(v["trans_fat_g"].as_f64()),
                    monounsaturated_fat_g: m["monounsaturated_fat_g"]
                        .as_f64()
                        .or(v["monounsaturated_fat_g"].as_f64()),
                    polyunsaturated_fat_g: m["polyunsaturated_fat_g"]
                        .as_f64()
                        .or(v["polyunsaturated_fat_g"].as_f64()),
                    cholesterol_mg: m["cholesterol_mg"]
                        .as_f64()
                        .or(v["cholesterol_mg"].as_f64()),
                    added_sugars_g: m["added_sugars_g"]
                        .as_f64()
                        .or(v["added_sugars_g"].as_f64()),
                };
                (rq, ru, macros, micros)
            } else {
                let rq = reference_quantity.ok_or_else(|| {
                    anyhow!("--reference-quantity required unless --json-file is used")
                })?;
                let ru = reference_unit.unwrap_or_else(|| "g".into());
                let micros = parse_micronutrient_triples(&micronutrient)?;
                let macros = ProductMacros {
                    energy_kcal,
                    protein_g,
                    carbohydrates_g,
                    fat_g,
                    fiber_g,
                    sugars_g,
                    saturated_fat_g,
                    trans_fat_g,
                    monounsaturated_fat_g,
                    polyunsaturated_fat_g,
                    cholesterol_mg,
                    added_sugars_g,
                };
                (rq, ru, macros, micros)
            };
            let warnings = validate_classic_macros_for_set(&macros)?;
            set_product_nutrition(&conn, id, rq, &ru, &macros, &micros)?;
            emit_nutrition_warnings(&warnings, quiet);
            if json {
                print_json(&Success::created_with_warnings(
                    id,
                    "nutrition",
                    "product nutrition set",
                    warnings,
                ));
            } else {
                quiet_print(quiet, format!("Set nutrition for product {id}"));
            }
        }
        ProductAction::Tag { action } => match action {
            TagModifyAction::Add { id, tag } => tag_add_product(&conn, id, &tag, json, quiet)?,
            TagModifyAction::Remove { id, tag } => {
                tag_remove_product(&conn, id, &tag, json, quiet)?
            }
        },
        ProductAction::SetLegacy {
            id,
            reference_quantity,
            reference_unit,
            energy_kcal,
            protein_g,
            carbohydrates_g,
            fat_g,
            fiber_g,
            sugars_g,
        } => {
            let macros = ProductMacros {
                energy_kcal,
                protein_g,
                carbohydrates_g,
                fat_g,
                fiber_g,
                sugars_g,
                ..Default::default()
            };
            let warnings = validate_classic_macros_for_set(&macros)?;
            set_product_nutrition(&conn, id, reference_quantity, &reference_unit, &macros, &[])?;
            emit_nutrition_warnings(&warnings, quiet);
            if json {
                print_json(&Success::created_with_warnings(
                    id,
                    "nutrition",
                    "product nutrition set",
                    warnings,
                ));
            } else {
                quiet_print(quiet, format!("Set nutrition for product {id}"));
            }
        }
        ProductAction::TagAdd { id, tag } => tag_add_product(&conn, id, &tag, json, quiet)?,
        ProductAction::TagRemove { id, tag } => tag_remove_product(&conn, id, &tag, json, quiet)?,
        ProductAction::Audit { id, limit } => {
            handle_event_audit(
                &conn,
                "products",
                entity_audit::entity::PRODUCT,
                id,
                limit,
                json,
                |c, i| {
                    c.query_row(
                        "SELECT id, name, created_at, updated_at, merged_into_id, retired_at
                         FROM products WHERE id=?1",
                        [i],
                        |r| {
                            Ok(serde_json::json!({
                                "id": r.get::<_, i64>(0)?,
                                "name": r.get::<_, String>(1)?,
                                "created_at": r.get::<_, String>(2)?,
                                "updated_at": r.get::<_, String>(3)?,
                                "merged_into_id": r.get::<_, Option<i64>>(4)?,
                                "retired_at": r.get::<_, Option<String>>(5)?,
                            }))
                        },
                    )
                    .optional()
                    .map_err(Into::into)
                },
            )?;
        }
    }
    Ok(())
}

/// Per-source summary for product merge output.
#[derive(Debug, Clone)]
struct MergeSourceReport {
    id: i64,
    name: String,
    purchases: i64,
    consumptions: i64,
    tags_copied: i64,
    nutrition_copied: bool,
    micronutrients_filled: i64,
}

/// Merge `from` product IDs into keeper `into` as catalog aliases.
///
/// - Leaves purchase/consumption `product_id` unchanged (append-only event FKs).
/// - Soft-retires each source (`merged_into_id` + `retired_at`) instead of DELETE.
/// - Copies tags the keeper does not already have.
/// - If the keeper has no `product_nutritions` row, copies nutrition from the
///   first source that has one (macros + micronutrients).
/// - If the keeper already has nutrition, fills missing micronutrient rows only
///   (does not overwrite macros or existing micro amounts).
/// - Source catalog rows (nutrition/tags) remain as a forensic snapshot.
fn merge_products(
    conn: &Connection,
    into: i64,
    from: &[i64],
    rename: Option<&str>,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if from.is_empty() {
        return Err(anyhow!("provide at least one source product id to merge"));
    }

    let into_row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT name, retired_at FROM products WHERE id = ?1",
            [into],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((into_name, into_retired)) = into_row else {
        return Err(anyhow!("product {into} not found (--into)"));
    };
    if into_retired.is_some() || !crate::product_resolve::is_active_product(conn, into)? {
        return Err(anyhow!(
            "product {into} ({into_name}) is retired; merge into an active product"
        ));
    }

    // Deduplicate sources while preserving order; reject into-in-from.
    let mut seen = std::collections::HashSet::new();
    let mut sources: Vec<i64> = Vec::new();
    for &id in from {
        if id == into {
            return Err(anyhow!(
                "cannot merge product {id} into itself; omit it from the source list"
            ));
        }
        if seen.insert(id) {
            sources.push(id);
        }
    }

    for &id in &sources {
        let row: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT name, retired_at FROM products WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((src_name, retired_at)) = row else {
            return Err(anyhow!("product {id} not found (source)"));
        };
        if retired_at.is_some() {
            return Err(anyhow!(
                "product {id} ({src_name}) is already retired; cannot merge again"
            ));
        }
    }

    let mut warnings: Vec<SanityWarning> = Vec::new();
    let into_has_nutrition = product_has_nutrition(conn, into)?;
    let into_unit_kind = product_unit_kind(conn, into)?;

    let mut reports: Vec<MergeSourceReport> = Vec::new();
    let mut nutrition_copied_from: Option<i64> = None;
    let mut total_purchases = 0i64;
    let mut total_consumptions = 0i64;
    let mut total_tags = 0i64;
    let mut total_micros_filled = 0i64;
    // Tracks whether keeper gained nutrition during this merge (dry-run or live).
    let mut keeper_has_nutrition = into_has_nutrition;

    if !dry_run {
        conn.execute("BEGIN IMMEDIATE", [])?;
    }

    let result = (|| -> Result<()> {
        for &src in &sources {
            let src_name: String =
                conn.query_row("SELECT name FROM products WHERE id = ?1", [src], |r| {
                    r.get(0)
                })?;
            let purchases: i64 = conn.query_row(
                "SELECT COUNT(*) FROM purchases WHERE product_id = ?1",
                [src],
                |r| r.get(0),
            )?;
            let consumptions: i64 = conn.query_row(
                "SELECT COUNT(*) FROM consumptions WHERE product_id = ?1",
                [src],
                |r| r.get(0),
            )?;

            let src_has_nutrition = product_has_nutrition(conn, src)?;
            let src_unit_kind = product_unit_kind(conn, src)?;

            // Unit-kind mismatch can make historical consumptions scale wrong under
            // the keeper's macros (reports resolve to keeper) — surface it.
            if let (Some(into_k), Some(src_k)) = (into_unit_kind, src_unit_kind) {
                if into_k != src_k && consumptions > 0 {
                    warnings.push(SanityWarning {
                        field: "reference_unit".into(),
                        kind: "unit_kind_mismatch".into(),
                        message: format!(
                            "source {src} ({src_name}) unit kind `{src_k}` differs from \
                             keeper {into} `{into_k}`; {consumptions} consumption(s) still \
                             reference the source id but reports use keeper macros — \
                             review historical quantities"
                        ),
                        previous_value: None,
                        previous_date: None,
                        new_value: None,
                        delta: None,
                        allowed_delta: None,
                        days_gap: None,
                    });
                }
            }

            // Count tags that would be newly associated on the keeper.
            let tags_copied: i64 = conn.query_row(
                "SELECT COUNT(*) FROM product_tag_associations s
                 WHERE s.product_id = ?1
                   AND NOT EXISTS (
                     SELECT 1 FROM product_tag_associations t
                     WHERE t.product_id = ?2 AND t.tag_id = s.tag_id
                   )",
                params![src, into],
                |r| r.get(0),
            )?;

            let mut nutrition_copied = false;
            let mut micronutrients_filled = 0i64;

            if !keeper_has_nutrition && src_has_nutrition {
                // Copy full nutrition from the first source that has it.
                if !dry_run {
                    copy_product_nutrition(conn, src, into)?;
                    copy_all_product_micronutrients(conn, src, into)?;
                }
                nutrition_copied = true;
                keeper_has_nutrition = true;
                nutrition_copied_from = Some(src);
                // All micros from source are new on the keeper.
                micronutrients_filled = conn.query_row(
                    "SELECT COUNT(*) FROM product_micronutrients WHERE product_id = ?1",
                    [src],
                    |r| r.get(0),
                )?;
            } else if keeper_has_nutrition && src_has_nutrition {
                // Keeper already has macros; only fill missing micronutrient rows.
                if !dry_run {
                    micronutrients_filled = fill_missing_product_micronutrients(conn, src, into)?;
                } else {
                    micronutrients_filled = count_fillable_micronutrients(conn, src, into)?;
                }
                // Source nutrition is discarded when the keeper already had macros
                // (pre-merge or copied from an earlier source in this run).
                if into_has_nutrition || nutrition_copied_from.is_some_and(|id| id != src) {
                    warnings.push(SanityWarning {
                        field: "nutrition".into(),
                        kind: "nutrition_kept_from_into".into(),
                        message: format!(
                            "source {src} ({src_name}) had nutrition; keeper {into} macros \
                             retained (source nutrition discarded after merge)"
                        ),
                        previous_value: None,
                        previous_date: None,
                        new_value: None,
                        delta: None,
                        allowed_delta: None,
                        days_gap: None,
                    });
                }
            }

            if !dry_run {
                // Catalog only: copy tags onto keeper. Event FKs stay on `src`.
                conn.execute(
                    "INSERT OR IGNORE INTO product_tag_associations (product_id, tag_id)
                     SELECT ?1, tag_id FROM product_tag_associations WHERE product_id = ?2",
                    params![into, src],
                )?;
                // Soft-retire source as alias of keeper (no DELETE, no event UPDATE).
                let now = db::now_utc();
                let n = conn.execute(
                    "UPDATE products
                     SET merged_into_id = ?1, retired_at = ?2, updated_at = ?2
                     WHERE id = ?3 AND retired_at IS NULL",
                    params![into, now, src],
                )?;
                if n == 0 {
                    return Err(anyhow!(
                        "product {src} disappeared or was already retired during merge"
                    ));
                }
            }

            total_purchases += purchases;
            total_consumptions += consumptions;
            total_tags += tags_copied;
            total_micros_filled += micronutrients_filled;
            reports.push(MergeSourceReport {
                id: src,
                name: src_name,
                purchases,
                consumptions,
                tags_copied,
                nutrition_copied,
                micronutrients_filled,
            });
        }

        if let Some(new_name) = rename {
            if !dry_run {
                conn.execute(
                    "UPDATE products SET name = ?1, updated_at = ?2 WHERE id = ?3",
                    params![new_name, db::now_utc(), into],
                )?;
            }
        } else if !dry_run {
            conn.execute(
                "UPDATE products SET updated_at = ?1 WHERE id = ?2",
                params![db::now_utc(), into],
            )?;
        }

        // Append-only merge trail (keeper + each source). Dry-run never writes.
        if !dry_run {
            let from_ids: Vec<i64> = reports.iter().map(|r| r.id).collect();
            for r in &reports {
                entity_audit::append_merge(
                    conn,
                    entity_audit::entity::PRODUCT,
                    r.id,
                    &format!("merged into {into}"),
                    Some(&serde_json::json!({
                        "role": "source",
                        "into_id": into,
                        "purchases": r.purchases,
                        "consumptions": r.consumptions,
                        "tags_copied": r.tags_copied,
                        "nutrition_copied": r.nutrition_copied,
                    })),
                )?;
            }
            let mut keeper_meta = serde_json::json!({
                "role": "keeper",
                "from_ids": from_ids,
                "purchases_aliased": total_purchases,
                "consumptions_aliased": total_consumptions,
                "tags_copied": total_tags,
                "nutrition_copied_from": nutrition_copied_from,
                "micronutrients_filled": total_micros_filled,
            });
            if let Some(n) = rename {
                keeper_meta["name"] = serde_json::json!(n);
            }
            entity_audit::append_merge(
                conn,
                entity_audit::entity::PRODUCT,
                into,
                &format!("merged {} source(s) as aliases", reports.len()),
                Some(&keeper_meta),
            )?;
        }
        Ok(())
    })();

    if !dry_run {
        match result {
            Ok(()) => {
                conn.execute("COMMIT", [])?;
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK", []);
                return Err(e);
            }
        }
    } else {
        result?;
    }

    let final_name = rename.unwrap_or(&into_name).to_string();
    let retired_ids: Vec<i64> = reports.iter().map(|r| r.id).collect();
    let message = if dry_run {
        format!(
            "dry-run: would merge {} product(s) into {into} ({final_name}) as aliases \
             (event product_ids unchanged)",
            reports.len()
        )
    } else {
        format!(
            "merged {} product(s) into {into} ({final_name}) as aliases \
             (event product_ids unchanged)",
            reports.len()
        )
    };

    emit_nutrition_warnings(&warnings, quiet);

    if json {
        let merged: Vec<_> = reports
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "name": r.name,
                    "purchases": r.purchases,
                    "consumptions": r.consumptions,
                    "tags_copied": r.tags_copied,
                    "nutrition_copied": r.nutrition_copied,
                    "micronutrients_filled": r.micronutrients_filled,
                })
            })
            .collect();
        print_json(&serde_json::json!({
            "success": true,
            "id": into,
            "into_id": into,
            "into_name": final_name,
            "merged_ids": retired_ids,
            "merged": merged,
            "purchases_aliased": total_purchases,
            "consumptions_aliased": total_consumptions,
            "tags_copied": total_tags,
            "nutrition_copied_from": nutrition_copied_from,
            "micronutrients_filled": total_micros_filled,
            "retired_ids": if dry_run { serde_json::Value::Null } else {
                serde_json::json!(retired_ids)
            },
            "dry_run": dry_run,
            "message": message,
            "warnings": if warnings.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!(warnings)
            },
        }));
    } else if !quiet {
        println!("{message}");
        for r in &reports {
            println!(
                "  {} ({}): {} purchase(s), {} consumption(s) still on source id, {} tag(s){}",
                r.id,
                r.name,
                r.purchases,
                r.consumptions,
                r.tags_copied,
                if r.nutrition_copied {
                    ", nutrition copied"
                } else if r.micronutrients_filled > 0 {
                    ", micros filled"
                } else {
                    ""
                }
            );
        }
        if let Some(src) = nutrition_copied_from {
            println!("  nutrition: copied from product {src}");
        }
        if !dry_run {
            println!("  kept product {into}: {final_name}");
            println!("  retired as aliases: {retired_ids:?}");
        }
    }

    Ok(())
}

fn product_has_nutrition(conn: &Connection, product_id: i64) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM product_nutritions WHERE product_id = ?1",
        [product_id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn product_unit_kind(conn: &Connection, product_id: i64) -> Result<Option<&'static str>> {
    let unit: Option<String> = conn
        .query_row(
            "SELECT reference_unit FROM product_nutritions WHERE product_id = ?1",
            [product_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(unit
        .as_deref()
        .and_then(|u| crate::nutrition_units::parse_unit(u).ok())
        .map(|p| p.kind.as_str()))
}

fn copy_product_nutrition(conn: &Connection, from: i64, into: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO product_nutritions (
            product_id, reference_quantity, reference_unit,
            energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g,
            saturated_fat_g, trans_fat_g, monounsaturated_fat_g, polyunsaturated_fat_g,
            cholesterol_mg, added_sugars_g
         )
         SELECT ?1, reference_quantity, reference_unit,
            energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g,
            saturated_fat_g, trans_fat_g, monounsaturated_fat_g, polyunsaturated_fat_g,
            cholesterol_mg, added_sugars_g
         FROM product_nutritions WHERE product_id = ?2",
        params![into, from],
    )?;
    Ok(())
}

fn copy_all_product_micronutrients(conn: &Connection, from: i64, into: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO product_micronutrients
         (product_id, micronutrient_id, amount, unit)
         SELECT ?1, micronutrient_id, amount, unit
         FROM product_micronutrients WHERE product_id = ?2",
        params![into, from],
    )?;
    Ok(())
}

fn fill_missing_product_micronutrients(conn: &Connection, from: i64, into: i64) -> Result<i64> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO product_micronutrients
         (product_id, micronutrient_id, amount, unit)
         SELECT ?1, micronutrient_id, amount, unit
         FROM product_micronutrients WHERE product_id = ?2",
        params![into, from],
    )?;
    Ok(n as i64)
}

fn count_fillable_micronutrients(conn: &Connection, from: i64, into: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM product_micronutrients s
         WHERE s.product_id = ?1
           AND NOT EXISTS (
             SELECT 1 FROM product_micronutrients t
             WHERE t.product_id = ?2 AND t.micronutrient_id = s.micronutrient_id
           )",
        params![from, into],
        |r| r.get(0),
    )?)
}

fn tag_add_product(conn: &Connection, id: i64, tag: &str, json: bool, quiet: bool) -> Result<()> {
    let tag_id = ensure_tag(conn, "product_tags", tag)?;
    conn.execute(
        "INSERT OR IGNORE INTO product_tag_associations (product_id, tag_id) VALUES (?1, ?2)",
        params![id, tag_id],
    )?;
    if json {
        print_json(&Success::ok(format!("tag {tag} added to product {id}")));
    } else {
        quiet_print(quiet, format!("Tagged product {id} with {tag}"));
    }
    Ok(())
}

fn tag_remove_product(
    conn: &Connection,
    id: i64,
    tag: &str,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let tag_id: Option<i64> = conn
        .query_row("SELECT id FROM product_tags WHERE name = ?1", [tag], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(tid) = tag_id {
        conn.execute(
            "DELETE FROM product_tag_associations WHERE product_id = ?1 AND tag_id = ?2",
            params![id, tid],
        )?;
    }
    if json {
        print_json(&Success::ok(format!("tag {tag} removed from product {id}")));
    } else {
        quiet_print(quiet, format!("Removed tag {tag} from product {id}"));
    }
    Ok(())
}

fn show_product(conn: &Connection, id: i64, json: bool) -> Result<()> {
    let row: Option<(String, Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT name, merged_into_id, retired_at FROM products WHERE id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((n, merged_into_id, retired_at)) = row else {
        if json {
            print_error_json("product not found");
        }
        return Err(anyhow!("product not found"));
    };
    let effective_id = crate::product_resolve::resolve_effective_product_id(conn, id)?;
    let effective_name: String = if effective_id == id {
        n.clone()
    } else {
        conn.query_row(
            "SELECT name FROM products WHERE id = ?1",
            [effective_id],
            |r| r.get(0),
        )?
    };
    let mut from_stmt = conn.prepare(
        "SELECT id, name, retired_at FROM products
         WHERE merged_into_id = ?1
         ORDER BY id",
    )?;
    let merged_from: Vec<_> = from_stmt
        .query_map([id], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "retired_at": r.get::<_, Option<String>>(2)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();
    let mut stmt = conn.prepare(
        "SELECT t.name FROM product_tags t
         JOIN product_tag_associations a ON a.tag_id = t.id
         WHERE a.product_id = ?1",
    )?;
    let tags: Vec<String> = stmt
        .query_map([id], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    let nutrition = conn
        .query_row(
            "SELECT reference_quantity, reference_unit, energy_kcal, protein_g, carbohydrates_g,
                    fat_g, fiber_g, sugars_g, saturated_fat_g, trans_fat_g, monounsaturated_fat_g,
                    polyunsaturated_fat_g, cholesterol_mg, added_sugars_g
             FROM product_nutritions WHERE product_id = ?1",
            [id],
            |r| {
                let rq: f64 = r.get(0)?;
                let ru: String = r.get(1)?;
                let kind = crate::nutrition_units::parse_unit(&ru)
                    .map(|p| p.kind.as_str())
                    .unwrap_or("unknown");
                let per = crate::nutrition_units::format_reference_serving(rq, &ru);
                Ok(serde_json::json!({
                    "reference_quantity": rq,
                    "reference_unit": ru,
                    "unit_kind": kind,
                    "per": per,
                    "energy_kcal": r.get::<_, Option<f64>>(2)?,
                    "protein_g": r.get::<_, Option<f64>>(3)?,
                    "carbohydrates_g": r.get::<_, Option<f64>>(4)?,
                    "fat_g": r.get::<_, Option<f64>>(5)?,
                    "fiber_g": r.get::<_, Option<f64>>(6)?,
                    "sugars_g": r.get::<_, Option<f64>>(7)?,
                    "saturated_fat_g": r.get::<_, Option<f64>>(8)?,
                    "trans_fat_g": r.get::<_, Option<f64>>(9)?,
                    "monounsaturated_fat_g": r.get::<_, Option<f64>>(10)?,
                    "polyunsaturated_fat_g": r.get::<_, Option<f64>>(11)?,
                    "cholesterol_mg": r.get::<_, Option<f64>>(12)?,
                    "added_sugars_g": r.get::<_, Option<f64>>(13)?,
                }))
            },
        )
        .optional()?;
    let mut mstmt = conn.prepare(
        "SELECT n.name, pm.amount, pm.unit FROM product_micronutrients pm
         JOIN micronutrients n ON n.id = pm.micronutrient_id
         WHERE pm.product_id = ?1",
    )?;
    let micros: Vec<_> = mstmt
        .query_map([id], |r| {
            Ok(serde_json::json!({
                "name": r.get::<_, String>(0)?,
                "amount": r.get::<_, f64>(1)?,
                "unit": r.get::<_, String>(2)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();
    let out = serde_json::json!({
        "id": id,
        "name": n,
        "merged_into_id": merged_into_id,
        "retired_at": retired_at,
        "effective_id": effective_id,
        "effective_name": effective_name,
        "merged_from": merged_from,
        "tags": tags,
        "nutrition": nutrition,
        "micronutrients": micros,
    });
    if json {
        print_json(&out);
    } else {
        println!("{id}: {n}");
        if let Some(ref at) = retired_at {
            println!(
                "  retired at {at} → effective {} ({})",
                effective_id, effective_name
            );
        } else if !merged_from.is_empty() {
            let ids: Vec<String> = merged_from
                .iter()
                .filter_map(|m| m["id"].as_i64().map(|i| i.to_string()))
                .collect();
            println!("  merge keeper for: {}", ids.join(", "));
        }
        if !tags.is_empty() {
            println!("  tags: {}", tags.join(", "));
        }
        if let Some(nu) = &nutrition {
            let per = nu["per"].as_str().unwrap_or("?");
            let kind = nu["unit_kind"].as_str().unwrap_or("?");
            println!("  nutrition ({kind}, {per}):");
            if let Some(v) = nu["energy_kcal"].as_f64() {
                println!("    energy: {v} kcal");
            }
            if let Some(v) = nu["protein_g"].as_f64() {
                println!("    protein: {v} g");
            }
            if let Some(v) = nu["carbohydrates_g"].as_f64() {
                println!("    carbs: {v} g");
            }
            if let Some(v) = nu["fat_g"].as_f64() {
                println!("    fat: {v} g");
            }
            if let Some(v) = nu["fiber_g"].as_f64() {
                println!("    fiber: {v} g");
            }
            if let Some(v) = nu["sugars_g"].as_f64() {
                println!("    sugars: {v} g");
            }
            if let Some(v) = nu["saturated_fat_g"].as_f64() {
                println!("    saturated fat: {v} g");
            }
            if let Some(v) = nu["trans_fat_g"].as_f64() {
                println!("    trans fat: {v} g");
            }
            if let Some(v) = nu["monounsaturated_fat_g"].as_f64() {
                println!("    monounsaturated fat: {v} g");
            }
            if let Some(v) = nu["polyunsaturated_fat_g"].as_f64() {
                println!("    polyunsaturated fat: {v} g");
            }
            if let Some(v) = nu["cholesterol_mg"].as_f64() {
                println!("    cholesterol: {v} mg");
            }
            if let Some(v) = nu["added_sugars_g"].as_f64() {
                println!("    added sugars: {v} g");
            }
            println!(
                "    reference: {} {}",
                nu["reference_quantity"], nu["reference_unit"]
            );
        } else {
            println!("  nutrition: (not set)");
        }
        for m in &micros {
            println!("  micro: {} {} {}", m["name"], m["amount"], m["unit"]);
        }
    }
    Ok(())
}

fn handle_purchase(
    action: PurchaseAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        PurchaseAction::Create {
            product,
            quantity,
            price,
            store,
            purchased_at,
        } => {
            crate::product_resolve::require_active_product(&conn, product)?;
            let when = parse_rfc3339_instant_for_db(&purchased_at)?;
            let now = db::now_utc();
            let price_cents: Option<i64> = price
                .and_then(|p| p.replace(['$', ','], "").parse::<f64>().ok())
                .map(|v| (v * 100.0).round() as i64);
            conn.execute(
                "INSERT INTO purchases (product_id, quantity, price_cents, store_id, purchased_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![product, quantity, price_cents, store, when, now],
            )?;
            let id = conn.last_insert_rowid();
            entity_audit::append_create(&conn, entity_audit::entity::PURCHASE, id, None)?;
            if json {
                print_json(&Success::created_purchase(
                    id,
                    when,
                    now,
                    "purchase recorded",
                ));
            } else {
                quiet_print(
                    quiet,
                    format!("Purchase {id} recorded (happened {when}, stored {now})"),
                );
            }
        }
        PurchaseAction::Correct {
            id,
            product,
            quantity,
            price,
            clear_store,
            store,
            purchased_at,
            reason,
            dry_run,
        } => {
            correct_purchase(
                &conn,
                id,
                product,
                quantity,
                price.as_deref(),
                clear_store,
                store,
                purchased_at.as_deref(),
                &reason,
                dry_run,
                json,
                quiet,
            )?;
        }
        PurchaseAction::List {
            since,
            until,
            product,
            store,
        } => {
            let mut sql = String::from(
                "SELECT pu.id, pu.product_id, p.name, pu.quantity, pu.price_cents, pu.store_id, \
                 pu.purchased_at, pu.created_at
                 FROM purchases pu LEFT JOIN products p ON p.id = pu.product_id \
                 WHERE pu.deleted_at IS NULL",
            );
            let mut binds: Vec<String> = vec![];
            if let Some(s) = since {
                sql.push_str(" AND date(pu.purchased_at, 'localtime') >= date(?)");
                binds.push(parse_date_to_ymd(&s)?);
            }
            if let Some(u) = until {
                sql.push_str(" AND date(pu.purchased_at, 'localtime') <= date(?)");
                binds.push(parse_date_to_ymd(&u)?);
            }
            if let Some(pid) = product {
                sql.push_str(&format!(" AND pu.product_id = {pid}"));
            }
            if let Some(sid) = store {
                sql.push_str(&format!(" AND pu.store_id = {sid}"));
            }
            sql.push_str(" ORDER BY pu.id DESC LIMIT 100");
            let mut stmt = conn.prepare(&sql)?;
            let mut rows: Vec<_> = stmt
                .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "product_id": r.get::<_, i64>(1)?,
                        "product_name": r.get::<_, Option<String>>(2)?,
                        "quantity": r.get::<_, f64>(3)?,
                        "price_cents": r.get::<_, Option<i64>>(4)?,
                        "store_id": r.get::<_, Option<i64>>(5)?,
                        "purchased_at": r.get::<_, String>(6)?,
                        "created_at": r.get::<_, String>(7)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            for row in &mut rows {
                if let Some(pid) = row["product_id"].as_i64() {
                    let effective =
                        crate::product_resolve::resolve_effective_product_id(&conn, pid)?;
                    if effective != pid {
                        if let Some(obj) = row.as_object_mut() {
                            let ename: String = conn.query_row(
                                "SELECT name FROM products WHERE id = ?1",
                                [effective],
                                |r| r.get(0),
                            )?;
                            obj.insert("effective_product_id".into(), serde_json::json!(effective));
                            obj.insert("effective_product_name".into(), serde_json::json!(ename));
                        }
                    }
                }
            }
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    println!(
                        "{}: product={} qty={} price_cents={:?} purchased_at={} created_at={}",
                        r["id"],
                        r["product_id"],
                        r["quantity"],
                        r["price_cents"],
                        r["purchased_at"].as_str().unwrap_or(""),
                        r["created_at"].as_str().unwrap_or(""),
                    );
                }
            }
        }
        PurchaseAction::Show { id } => {
            let row = conn
                .query_row(
                    "SELECT id, product_id, quantity, price_cents, store_id, purchased_at, created_at, \
                     supersedes_id, deleted_at, delete_reason
                     FROM purchases WHERE id=?1",
                    [id],
                    |r| {
                        Ok(serde_json::json!({
                            "id": r.get::<_, i64>(0)?,
                            "product_id": r.get::<_, i64>(1)?,
                            "quantity": r.get::<_, f64>(2)?,
                            "price_cents": r.get::<_, Option<i64>>(3)?,
                            "store_id": r.get::<_, Option<i64>>(4)?,
                            "purchased_at": r.get::<_, String>(5)?,
                            "created_at": r.get::<_, String>(6)?,
                            "supersedes_id": r.get::<_, Option<i64>>(7)?,
                            "deleted_at": r.get::<_, Option<String>>(8)?,
                            "delete_reason": r.get::<_, Option<String>>(9)?,
                        }))
                    },
                )
                .optional()?;
            match row {
                Some(v) => {
                    if json {
                        print_json(&v);
                    } else {
                        println!("{v}");
                    }
                }
                None => return Err(anyhow!("purchase not found")),
            }
        }
        PurchaseAction::Delete {
            id,
            reason,
            purge,
            force,
        } => {
            handle_event_delete(
                &conn,
                "purchases",
                entity_audit::entity::PURCHASE,
                id,
                reason.as_deref(),
                purge,
                force,
                json,
                quiet,
                "purchase",
            )?;
        }
        PurchaseAction::Audit { id, limit } => {
            handle_event_audit(
                &conn,
                "purchases",
                entity_audit::entity::PURCHASE,
                id,
                limit,
                json,
                |c, i| {
                    c.query_row(
                        "SELECT id, product_id, quantity, price_cents, store_id, purchased_at, \
                         created_at, supersedes_id, deleted_at, delete_reason
                         FROM purchases WHERE id=?1",
                        [i],
                        |r| {
                            Ok(serde_json::json!({
                                "id": r.get::<_, i64>(0)?,
                                "product_id": r.get::<_, i64>(1)?,
                                "quantity": r.get::<_, f64>(2)?,
                                "price_cents": r.get::<_, Option<i64>>(3)?,
                                "store_id": r.get::<_, Option<i64>>(4)?,
                                "purchased_at": r.get::<_, String>(5)?,
                                "created_at": r.get::<_, String>(6)?,
                                "supersedes_id": r.get::<_, Option<i64>>(7)?,
                                "deleted_at": r.get::<_, Option<String>>(8)?,
                                "delete_reason": r.get::<_, Option<String>>(9)?,
                            }))
                        },
                    )
                    .optional()
                    .map_err(Into::into)
                },
            )?;
        }
    }
    Ok(())
}

fn handle_consumption(
    action: ConsumptionAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        ConsumptionAction::Create {
            product,
            quantity,
            unit,
            consumed_at,
            allow_midnight,
        } => {
            let when_dt = parse_rfc3339_to_utc(&consumed_at)?;
            refuse_consumption_midnight(when_dt, allow_midnight)?;
            let when = parse_rfc3339_instant_for_db(&consumed_at)?;
            let now = db::now_utc();
            crate::product_resolve::require_active_product(&conn, product)?;
            let nutrition = load_product_nutrition_gate(&conn, product)?;
            let resolved = crate::nutrition_units::resolve_consumption(
                quantity,
                unit.as_deref(),
                nutrition.ref_qty,
                &nutrition.ref_unit,
            )?;
            conn.execute(
                "INSERT INTO consumptions (product_id, quantity, unit, consumed_at, created_at) VALUES (?1,?2,?3,?4,?5)",
                params![product, resolved.quantity, resolved.unit, when, now],
            )?;
            let id = conn.last_insert_rowid();
            entity_audit::append_create(&conn, entity_audit::entity::CONSUMPTION, id, None)?;
            if json {
                print_json(&serde_json::json!({
                    "success": true,
                    "id": id,
                    "message": "consumption logged",
                    "product_id": product,
                    "quantity": resolved.quantity,
                    "unit": resolved.unit,
                    "unit_kind": crate::nutrition_units::parse_unit(&resolved.unit)
                        .map(|p| p.kind.as_str())
                        .unwrap_or("unknown"),
                    "product_reference_unit": nutrition.ref_unit,
                    "consumed_at": when,
                    "created_at": now,
                }));
            } else {
                quiet_print(
                    quiet,
                    format!(
                        "Consumption {id} logged: {} {} (happened {when}, stored {now})",
                        resolved.quantity, resolved.unit
                    ),
                );
            }
        }
        ConsumptionAction::Correct {
            id,
            product,
            quantity,
            unit,
            consumed_at,
            allow_midnight,
            reason,
            dry_run,
        } => {
            correct_consumption(
                &conn,
                id,
                product,
                quantity,
                unit.as_deref(),
                consumed_at.as_deref(),
                allow_midnight,
                &reason,
                dry_run,
                json,
                quiet,
            )?;
        }
        ConsumptionAction::List {
            since,
            until,
            product,
        } => {
            let mut sql = String::from(
                "SELECT c.id, c.product_id, p.name, c.quantity, c.unit, c.consumed_at, c.created_at
                 FROM consumptions c LEFT JOIN products p ON p.id = c.product_id \
                 WHERE c.deleted_at IS NULL",
            );
            let mut binds: Vec<String> = vec![];
            if let Some(s) = since {
                sql.push_str(" AND date(c.consumed_at, 'localtime') >= date(?)");
                binds.push(parse_date_to_ymd(&s)?);
            }
            if let Some(u) = until {
                sql.push_str(" AND date(c.consumed_at, 'localtime') <= date(?)");
                binds.push(parse_date_to_ymd(&u)?);
            }
            if let Some(pid) = product {
                sql.push_str(&format!(" AND c.product_id = {pid}"));
            }
            sql.push_str(" ORDER BY c.consumed_at DESC LIMIT 100");
            let mut stmt = conn.prepare(&sql)?;
            let mut rows: Vec<_> = stmt
                .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "product_id": r.get::<_, i64>(1)?,
                        "product_name": r.get::<_, Option<String>>(2)?,
                        "quantity": r.get::<_, f64>(3)?,
                        "unit": r.get::<_, Option<String>>(4)?,
                        "consumed_at": r.get::<_, String>(5)?,
                        "created_at": r.get::<_, String>(6)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            for row in &mut rows {
                if let Some(pid) = row["product_id"].as_i64() {
                    let effective =
                        crate::product_resolve::resolve_effective_product_id(&conn, pid)?;
                    if effective != pid {
                        if let Some(obj) = row.as_object_mut() {
                            let ename: String = conn.query_row(
                                "SELECT name FROM products WHERE id = ?1",
                                [effective],
                                |r| r.get(0),
                            )?;
                            obj.insert("effective_product_id".into(), serde_json::json!(effective));
                            obj.insert("effective_product_name".into(), serde_json::json!(ename));
                        }
                    }
                }
            }
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    let unit = r["unit"].as_str().unwrap_or("?");
                    let name = r
                        .get("effective_product_name")
                        .and_then(|v| v.as_str())
                        .or_else(|| r["product_name"].as_str())
                        .unwrap_or("?");
                    println!(
                        "{}: {} {} {} on {}",
                        r["id"], name, r["quantity"], unit, r["consumed_at"]
                    );
                }
            }
        }
        ConsumptionAction::Delete {
            id,
            reason,
            purge,
            force,
        } => {
            handle_event_delete(
                &conn,
                "consumptions",
                entity_audit::entity::CONSUMPTION,
                id,
                reason.as_deref(),
                purge,
                force,
                json,
                quiet,
                "consumption",
            )?;
        }
        ConsumptionAction::Audit { id, limit } => {
            handle_event_audit(
                &conn,
                "consumptions",
                entity_audit::entity::CONSUMPTION,
                id,
                limit,
                json,
                |c, i| {
                    c.query_row(
                        "SELECT id, product_id, quantity, unit, consumed_at, created_at, \
                         supersedes_id, deleted_at, delete_reason
                         FROM consumptions WHERE id=?1",
                        [i],
                        |r| {
                            Ok(serde_json::json!({
                                "id": r.get::<_, i64>(0)?,
                                "product_id": r.get::<_, i64>(1)?,
                                "quantity": r.get::<_, f64>(2)?,
                                "unit": r.get::<_, Option<String>>(3)?,
                                "consumed_at": r.get::<_, String>(4)?,
                                "created_at": r.get::<_, String>(5)?,
                                "supersedes_id": r.get::<_, Option<i64>>(6)?,
                                "deleted_at": r.get::<_, Option<String>>(7)?,
                                "delete_reason": r.get::<_, Option<String>>(8)?,
                            }))
                        },
                    )
                    .optional()
                    .map_err(Into::into)
                },
            )?;
        }
    }
    Ok(())
}

/// Ref + classic six required for consumption logging.
struct ProductNutritionGate {
    ref_qty: f64,
    ref_unit: String,
    energy_kcal: Option<f64>,
    protein_g: Option<f64>,
    carbohydrates_g: Option<f64>,
    fat_g: Option<f64>,
    fiber_g: Option<f64>,
    sugars_g: Option<f64>,
}

fn load_product_nutrition_gate(conn: &Connection, product: i64) -> Result<ProductNutritionGate> {
    let nutrition: Option<ProductNutritionGate> = conn
        .query_row(
            "SELECT reference_quantity, reference_unit,
                    energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g
             FROM product_nutritions
             WHERE product_id = ?1",
            [product],
            |r| {
                Ok(ProductNutritionGate {
                    ref_qty: r.get(0)?,
                    ref_unit: r.get(1)?,
                    energy_kcal: r.get(2)?,
                    protein_g: r.get(3)?,
                    carbohydrates_g: r.get(4)?,
                    fat_g: r.get(5)?,
                    fiber_g: r.get(6)?,
                    sugars_g: r.get(7)?,
                })
            },
        )
        .optional()?;
    let Some(nutrition) = nutrition else {
        return Err(anyhow!(
            "product {product} has no nutrition set; run \
             `nutrition product nutrition set {product} --reference-quantity … \
             --reference-unit g|ml|unit` first"
        ));
    };
    if !classic_macros_complete_opts(
        nutrition.energy_kcal,
        nutrition.protein_g,
        nutrition.carbohydrates_g,
        nutrition.fat_g,
        nutrition.fiber_g,
        nutrition.sugars_g,
    ) {
        return Err(anyhow!(
            "product {product} has incomplete classic macros \
             (energy/protein/carbs/fat/fiber/sugars must all be set); run \
             `nutrition product nutrition set {product} --energy-kcal … \
             --protein-g … --carbohydrates-g … --fat-g … --fiber-g … --sugars-g …` \
             (use explicit 0 when truly zero)"
        ));
    }
    Ok(nutrition)
}

#[allow(clippy::too_many_arguments)]
fn correct_consumption(
    conn: &Connection,
    old_id: i64,
    product: Option<i64>,
    quantity: Option<f64>,
    unit: Option<&str>,
    consumed_at: Option<&str>,
    allow_midnight: bool,
    reason: &str,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(anyhow!("consumption correct requires a non-empty --reason"));
    }
    let old: (
        i64,
        f64,
        Option<String>,
        String,
        Option<String>,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT product_id, quantity, unit, consumed_at, deleted_at, supersedes_id
             FROM consumptions WHERE id = ?1",
            [old_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("consumption {old_id} not found"))?;
    let (old_product, old_qty, old_unit, old_when, deleted_at, _old_sup) = old;
    if deleted_at.is_some() {
        return Err(anyhow!(
            "consumption {old_id} is already soft-deleted (cannot supersede)"
        ));
    }
    let new_product = product.unwrap_or(old_product);
    crate::product_resolve::require_active_product(conn, new_product)?;
    let nutrition = load_product_nutrition_gate(conn, new_product)?;
    let qty_in = quantity.unwrap_or(old_qty);
    let unit_in = unit
        .map(|s| s.to_string())
        .or(old_unit.clone())
        .unwrap_or_else(|| nutrition.ref_unit.clone());
    let resolved = crate::nutrition_units::resolve_consumption(
        qty_in,
        Some(&unit_in),
        nutrition.ref_qty,
        &nutrition.ref_unit,
    )?;
    let when = if let Some(raw) = consumed_at {
        let when_dt = parse_rfc3339_to_utc(raw)?;
        refuse_consumption_midnight(when_dt, allow_midnight)?;
        parse_rfc3339_instant_for_db(raw)?
    } else {
        old_when.clone()
    };
    let mut changes: Vec<entity_audit::FieldChange> = Vec::new();
    if new_product != old_product {
        changes.push(entity_audit::FieldChange::new(
            "product_id",
            serde_json::json!(old_product),
            serde_json::json!(new_product),
        ));
    }
    if (resolved.quantity - old_qty).abs() > f64::EPSILON {
        changes.push(entity_audit::FieldChange::new(
            "quantity",
            serde_json::json!(old_qty),
            serde_json::json!(resolved.quantity),
        ));
    }
    let old_u = old_unit.as_deref().unwrap_or("");
    if resolved.unit != old_u {
        changes.push(entity_audit::FieldChange::new(
            "unit",
            serde_json::json!(old_unit),
            serde_json::json!(resolved.unit),
        ));
    }
    if when != old_when {
        changes.push(entity_audit::FieldChange::new(
            "consumed_at",
            serde_json::json!(old_when),
            serde_json::json!(when),
        ));
    }
    if dry_run {
        if json {
            print_json(&serde_json::json!({
                "success": true,
                "dry_run": true,
                "mode": "supersede",
                "supersedes_id": old_id,
                "product_id": new_product,
                "quantity": resolved.quantity,
                "unit": resolved.unit,
                "consumed_at": when,
                "reason": reason,
                "fields": changes.iter().map(|f| serde_json::json!({
                    "name": f.name, "old": f.old, "new": f.new
                })).collect::<Vec<_>>(),
            }));
        } else {
            quiet_print(
                quiet,
                format!("Dry-run: would supersede consumption {old_id} (reason: {reason})"),
            );
        }
        return Ok(());
    }
    let now = db::now_utc();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO consumptions (product_id, quantity, unit, consumed_at, created_at, supersedes_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            new_product,
            resolved.quantity,
            resolved.unit,
            when,
            now,
            old_id
        ],
    )?;
    let new_id = tx.last_insert_rowid();
    entity_audit::append_supersede_create(
        &tx,
        entity_audit::entity::CONSUMPTION,
        new_id,
        old_id,
        reason,
        Some(&changes),
    )?;
    let deleted_at = entity_audit::supersede_retire(
        &tx,
        "consumptions",
        entity_audit::entity::CONSUMPTION,
        old_id,
        new_id,
        reason,
        Some(&changes),
    )?;
    tx.commit()?;
    if json {
        print_json(&serde_json::json!({
            "success": true,
            "id": new_id,
            "supersedes_id": old_id,
            "mode": "supersede",
            "message": "consumption corrected (supersede)",
            "product_id": new_product,
            "quantity": resolved.quantity,
            "unit": resolved.unit,
            "consumed_at": when,
            "created_at": now,
            "old_deleted_at": deleted_at,
            "reason": reason,
        }));
    } else {
        quiet_print(
            quiet,
            format!("Consumption {new_id} supersedes {old_id} (reason: {reason})"),
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn correct_purchase(
    conn: &Connection,
    old_id: i64,
    product: Option<i64>,
    quantity: Option<f64>,
    price: Option<&str>,
    clear_store: bool,
    store: Option<i64>,
    purchased_at: Option<&str>,
    reason: &str,
    dry_run: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(anyhow!("purchase correct requires a non-empty --reason"));
    }
    let old: (i64, f64, Option<i64>, Option<i64>, String, Option<String>) = conn
        .query_row(
            "SELECT product_id, quantity, price_cents, store_id, purchased_at, deleted_at
             FROM purchases WHERE id = ?1",
            [old_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("purchase {old_id} not found"))?;
    let (old_product, old_qty, old_price, old_store, old_when, deleted_at) = old;
    if deleted_at.is_some() {
        return Err(anyhow!(
            "purchase {old_id} is already soft-deleted (cannot supersede)"
        ));
    }
    let new_product = product.unwrap_or(old_product);
    crate::product_resolve::require_active_product(conn, new_product)?;
    let new_qty = quantity.unwrap_or(old_qty);
    let new_price: Option<i64> = if let Some(p) = price {
        Some(
            p.replace(['$', ','], "")
                .parse::<f64>()
                .map(|v| (v * 100.0).round() as i64)
                .map_err(|_| anyhow!("invalid --price: {p}"))?,
        )
    } else {
        old_price
    };
    let new_store = if clear_store {
        None
    } else {
        store.or(old_store)
    };
    let when = if let Some(raw) = purchased_at {
        parse_rfc3339_instant_for_db(raw)?
    } else {
        old_when.clone()
    };
    let mut changes: Vec<entity_audit::FieldChange> = Vec::new();
    if new_product != old_product {
        changes.push(entity_audit::FieldChange::new(
            "product_id",
            serde_json::json!(old_product),
            serde_json::json!(new_product),
        ));
    }
    if (new_qty - old_qty).abs() > f64::EPSILON {
        changes.push(entity_audit::FieldChange::new(
            "quantity",
            serde_json::json!(old_qty),
            serde_json::json!(new_qty),
        ));
    }
    if new_price != old_price {
        changes.push(entity_audit::FieldChange::new(
            "price_cents",
            serde_json::json!(old_price),
            serde_json::json!(new_price),
        ));
    }
    if new_store != old_store {
        changes.push(entity_audit::FieldChange::new(
            "store_id",
            serde_json::json!(old_store),
            serde_json::json!(new_store),
        ));
    }
    if when != old_when {
        changes.push(entity_audit::FieldChange::new(
            "purchased_at",
            serde_json::json!(old_when),
            serde_json::json!(when),
        ));
    }
    if dry_run {
        if json {
            print_json(&serde_json::json!({
                "success": true,
                "dry_run": true,
                "mode": "supersede",
                "supersedes_id": old_id,
                "product_id": new_product,
                "quantity": new_qty,
                "price_cents": new_price,
                "store_id": new_store,
                "purchased_at": when,
                "reason": reason,
                "fields": changes.iter().map(|f| serde_json::json!({
                    "name": f.name, "old": f.old, "new": f.new
                })).collect::<Vec<_>>(),
            }));
        } else {
            quiet_print(
                quiet,
                format!("Dry-run: would supersede purchase {old_id} (reason: {reason})"),
            );
        }
        return Ok(());
    }
    let now = db::now_utc();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO purchases (product_id, quantity, price_cents, store_id, purchased_at, created_at, supersedes_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new_product,
            new_qty,
            new_price,
            new_store,
            when,
            now,
            old_id
        ],
    )?;
    let new_id = tx.last_insert_rowid();
    entity_audit::append_supersede_create(
        &tx,
        entity_audit::entity::PURCHASE,
        new_id,
        old_id,
        reason,
        Some(&changes),
    )?;
    let deleted_at = entity_audit::supersede_retire(
        &tx,
        "purchases",
        entity_audit::entity::PURCHASE,
        old_id,
        new_id,
        reason,
        Some(&changes),
    )?;
    tx.commit()?;
    if json {
        print_json(&serde_json::json!({
            "success": true,
            "id": new_id,
            "supersedes_id": old_id,
            "mode": "supersede",
            "message": "purchase corrected (supersede)",
            "product_id": new_product,
            "quantity": new_qty,
            "price_cents": new_price,
            "store_id": new_store,
            "purchased_at": when,
            "created_at": now,
            "old_deleted_at": deleted_at,
            "reason": reason,
        }));
    } else {
        quiet_print(
            quiet,
            format!("Purchase {new_id} supersedes {old_id} (reason: {reason})"),
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_event_delete(
    conn: &rusqlite::Connection,
    table: &str,
    entity_type: &str,
    id: i64,
    reason: Option<&str>,
    purge: bool,
    force: bool,
    json: bool,
    quiet: bool,
    label: &str,
) -> Result<()> {
    if purge {
        if !force {
            return Err(anyhow!(
                "{label} --purge requires --force (hard-deletes the row)"
            ));
        }
        entity_audit::purge(conn, table, entity_type, id, reason, None)?;
        if json {
            print_json(&serde_json::json!({
                "success": true,
                "deleted_id": id,
                "id": id,
                "mode": "purge",
                "message": format!("purged {label} {id}"),
            }));
        } else {
            quiet_print(quiet, format!("Purged {label} {id}"));
        }
        return Ok(());
    }
    let deleted_at = entity_audit::soft_delete(conn, table, entity_type, id, reason)?;
    if json {
        print_json(&serde_json::json!({
            "success": true,
            "deleted_id": id,
            "id": id,
            "mode": "soft_delete",
            "deleted_at": deleted_at,
            "message": format!("soft-deleted {label} {id}"),
        }));
    } else {
        quiet_print(quiet, format!("Soft-deleted {label} {id}"));
    }
    Ok(())
}

fn handle_event_audit<F>(
    conn: &rusqlite::Connection,
    _table: &str,
    entity_type: &str,
    id: i64,
    limit: i64,
    json: bool,
    fetch_current: F,
) -> Result<()>
where
    F: FnOnce(&rusqlite::Connection, i64) -> Result<Option<serde_json::Value>>,
{
    let current = fetch_current(conn, id)?;
    let current = entity_audit::enrich_current_supersede(conn, entity_type, current)?;
    let history = entity_audit::list_history(conn, entity_type, id, limit)?;
    if current.is_none() && history.is_empty() {
        return Err(anyhow!("{entity_type} {id} not found"));
    }
    let resp = entity_audit::audit_response(entity_type, id, current, history);
    if json {
        print_json(&resp);
    } else {
        entity_audit::print_audit_human(&resp);
    }
    Ok(())
}

fn handle_micronutrient(
    action: MicronutrientAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        MicronutrientAction::List => {
            let mut stmt = conn.prepare(
                "SELECT id, name, unit, recommended_intake, infoods_tag
                 FROM micronutrients ORDER BY name",
            )?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "unit": r.get::<_, String>(2)?,
                        "recommended_intake": r.get::<_, Option<f64>>(3)?,
                        "infoods_tag": r.get::<_, Option<String>>(4)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for n in &rows {
                    let tag = n["infoods_tag"].as_str().unwrap_or("-");
                    println!("{}: {} ({}) infoods={}", n["id"], n["name"], n["unit"], tag);
                }
            }
        }
        MicronutrientAction::Create {
            name,
            unit,
            recommended_intake,
            infoods,
            force,
        } => {
            create_micronutrient(
                &conn,
                &name,
                &unit,
                recommended_intake,
                infoods.as_deref(),
                force,
                json,
                quiet,
            )?;
        }
        MicronutrientAction::Show { id } => {
            let row = conn
                .query_row(
                    "SELECT id, name, unit, recommended_intake, infoods_tag
                     FROM micronutrients WHERE id=?1",
                    [id],
                    |r| {
                        Ok(serde_json::json!({
                            "id": r.get::<_, i64>(0)?,
                            "name": r.get::<_, String>(1)?,
                            "unit": r.get::<_, String>(2)?,
                            "recommended_intake": r.get::<_, Option<f64>>(3)?,
                            "infoods_tag": r.get::<_, Option<String>>(4)?,
                        }))
                    },
                )
                .optional()?;
            match row {
                Some(v) => {
                    if json {
                        print_json(&v);
                    } else {
                        println!("{v}");
                    }
                }
                None => return Err(anyhow!("micronutrient not found")),
            }
        }
        MicronutrientAction::Search { query } => {
            let mut stmt = conn.prepare("SELECT id, name FROM micronutrients")?;
            let cands: Vec<_> = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            let ranked = fuzzy_rank(cands, &query);
            if json {
                let out: Vec<_> = ranked
                    .iter()
                    .map(|(id, nm, s)| serde_json::json!({"id": id, "name": nm, "score": s}))
                    .collect();
                print_json(&out);
            } else {
                for (id, nm, s) in ranked {
                    println!("{id}: {nm} ({s:.2})");
                }
            }
        }
        MicronutrientAction::Delete { id, force } => {
            if !force {
                let refs: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM product_micronutrients WHERE micronutrient_id=?1",
                    [id],
                    |r| r.get(0),
                )?;
                if refs > 0 {
                    return Err(anyhow!(
                        "micronutrient referenced by products; use --force to delete"
                    ));
                }
            } else {
                conn.execute(
                    "DELETE FROM product_micronutrients WHERE micronutrient_id=?1",
                    [id],
                )?;
            }
            let n = conn.execute("DELETE FROM micronutrients WHERE id=?1", [id])?;
            if n == 0 {
                return Err(anyhow!("micronutrient not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted micronutrient {id}"));
            }
        }
        MicronutrientAction::Audit { id, limit } => {
            handle_event_audit(
                &conn,
                "micronutrients",
                entity_audit::entity::MICRONUTRIENT,
                id,
                limit,
                json,
                |c, i| {
                    c.query_row(
                        "SELECT id, name, unit, recommended_intake, created_at, infoods_tag
                         FROM micronutrients WHERE id=?1",
                        [i],
                        |r| {
                            Ok(serde_json::json!({
                                "id": r.get::<_, i64>(0)?,
                                "name": r.get::<_, String>(1)?,
                                "unit": r.get::<_, String>(2)?,
                                "recommended_intake": r.get::<_, Option<f64>>(3)?,
                                "created_at": r.get::<_, String>(4)?,
                                "infoods_tag": r.get::<_, Option<String>>(5)?,
                            }))
                        },
                    )
                    .optional()
                    .map_err(Into::into)
                },
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_micronutrient(
    conn: &Connection,
    name: &str,
    unit: &str,
    recommended_intake: Option<f64>,
    infoods_tag: Option<&str>,
    force: bool,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("micronutrient name must not be empty"));
    }
    if is_macronutrient_name(name) {
        let flag = macro_flag_hint(name).unwrap_or("product nutrition set macro flags");
        return Err(anyhow!(
            "'{name}' is a macronutrient; use {flag} on product nutrition set \
             (not micronutrient create)"
        ));
    }

    // Case-insensitive name uniqueness (no --force bypass).
    if let Some((id, existing)) = conn
        .query_row(
            "SELECT id, name FROM micronutrients WHERE name = ?1 COLLATE NOCASE",
            [name],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        return Err(anyhow!(
            "micronutrient already exists: id={id} name={existing}"
        ));
    }

    let unit = infoods::normalize_unit(unit);
    let mut resolved_tag: Option<String> = None;
    let mut force_warnings: Vec<SanityWarning> = Vec::new();

    if let Some(tag) = infoods_tag {
        let tag = tag.trim();
        if !infoods::tag_exists(conn, tag)? {
            return Err(anyhow!("unknown INFOODS tag '{tag}'"));
        }
        if let Some((id, existing)) = conn
            .query_row(
                "SELECT id, name FROM micronutrients WHERE infoods_tag = ?1 COLLATE NOCASE",
                [tag],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            return Err(anyhow!(
                "INFOODS tag '{tag}' already linked to micronutrient id={id} name={existing}"
            ));
        }
        // Canonical tag casing from catalog
        let (canon, _, _) = infoods::get_component(conn, tag)?
            .ok_or_else(|| anyhow!("unknown INFOODS tag '{tag}'"))?;
        resolved_tag = Some(canon);
    } else {
        let blockers = infoods::find_create_blockers(conn, name)?;
        if !blockers.is_empty() {
            if !force {
                return Err(anyhow!(infoods::format_blockers_message(name, &blockers)));
            }
            let msg = infoods::force_warning_message(name, &blockers);
            eprintln!("Warning: {msg}");
            force_warnings.push(SanityWarning {
                field: "infoods".into(),
                kind: "force".into(),
                message: msg,
                previous_value: None,
                previous_date: None,
                new_value: None,
                delta: None,
                allowed_delta: None,
                days_gap: None,
            });
        }
    }

    let now = db::now_utc();
    conn.execute(
        "INSERT INTO micronutrients (name, unit, recommended_intake, created_at, infoods_tag)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![name, unit, recommended_intake, now, resolved_tag],
    )?;
    let id = conn.last_insert_rowid();
    entity_audit::append_create(conn, entity_audit::entity::MICRONUTRIENT, id, None)?;
    if json {
        print_json(&Success::created_with_warnings(
            id,
            name.to_string(),
            format!("micronutrient: {name}"),
            force_warnings,
        ));
    } else {
        quiet_print(
            quiet,
            format!(
                "Created micronutrient {id}: {name}{}",
                resolved_tag
                    .as_ref()
                    .map(|t| format!(" infoods={t}"))
                    .unwrap_or_default()
            ),
        );
    }
    Ok(())
}

fn handle_infoods(
    action: InfoodsAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let _ = quiet;
    let conn = db::open_db(db_override)?;
    match action {
        InfoodsAction::List { limit } => {
            let limit = limit.max(1);
            let mut stmt = conn.prepare(
                "SELECT tag, name, unit, source FROM infoods_components
                 ORDER BY tag LIMIT ?1",
            )?;
            let rows: Vec<_> = stmt
                .query_map([limit], |r| {
                    Ok(serde_json::json!({
                        "tag": r.get::<_, String>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "unit": r.get::<_, Option<String>>(2)?,
                        "source": r.get::<_, String>(3)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    let unit = r["unit"].as_str().unwrap_or("?");
                    println!("{}: {} ({})", r["tag"], r["name"], unit);
                }
            }
        }
        InfoodsAction::Show { tag } => {
            let row = conn
                .query_row(
                    "SELECT tag, name, unit, synonyms, comments, tables_note, source
                     FROM infoods_components WHERE tag = ?1 COLLATE NOCASE",
                    [&tag],
                    |r| {
                        Ok(serde_json::json!({
                            "tag": r.get::<_, String>(0)?,
                            "name": r.get::<_, String>(1)?,
                            "unit": r.get::<_, Option<String>>(2)?,
                            "synonyms": r.get::<_, Option<String>>(3)?,
                            "comments": r.get::<_, Option<String>>(4)?,
                            "tables_note": r.get::<_, Option<String>>(5)?,
                            "source": r.get::<_, String>(6)?,
                        }))
                    },
                )
                .optional()?;
            match row {
                Some(v) => {
                    if json {
                        print_json(&v);
                    } else {
                        println!(
                            "{} — {} ({})",
                            v["tag"].as_str().unwrap_or("?"),
                            v["name"].as_str().unwrap_or("?"),
                            v["unit"].as_str().unwrap_or("?")
                        );
                        if let Some(s) = v["synonyms"].as_str() {
                            println!("  synonyms: {s}");
                        }
                        if let Some(c) = v["comments"].as_str() {
                            println!("  comments: {c}");
                        }
                    }
                }
                None => return Err(anyhow!("INFOODS tag not found: {tag}")),
            }
        }
        InfoodsAction::Search { query } => {
            // Exact first, then synonym substring, then fuzzy name.
            let mut hits = infoods::find_exact_matches(&conn, &query)?;
            {
                let pattern = format!("%{}%", query.trim());
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT c.tag, c.name, c.unit
                     FROM infoods_synonyms s
                     JOIN infoods_components c ON c.tag = s.tag
                     WHERE s.synonym LIKE ?1 COLLATE NOCASE
                     LIMIT 40",
                )?;
                let rows = stmt.query_map([&pattern], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?;
                for row in rows {
                    let (tag, name, unit) = row?;
                    if hits.iter().any(|h| h.tag.eq_ignore_ascii_case(&tag)) {
                        continue;
                    }
                    hits.push(infoods::InfoodsMatch {
                        tag,
                        name,
                        unit,
                        via: "synonym",
                        score: 0.9,
                    });
                }
            }
            if hits.len() < 10 {
                for h in infoods::find_fuzzy_matches(&conn, &query, 20)? {
                    if hits.iter().any(|x| x.tag.eq_ignore_ascii_case(&h.tag)) {
                        continue;
                    }
                    hits.push(h);
                }
            }
            if hits.len() < 5 {
                let mut stmt = conn.prepare("SELECT tag, name, unit FROM infoods_components")?;
                let cands: Vec<_> = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                let mut ranked: Vec<_> = cands
                    .into_iter()
                    .map(|(tag, name, unit)| {
                        let score = name_match_score(&format!("{tag} {name}"), &query)
                            .max(name_match_score(&name, &query));
                        (tag, name, unit, score)
                    })
                    .filter(|(_, _, _, s)| *s >= 0.5)
                    .collect();
                ranked.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
                ranked.truncate(30);
                for (tag, name, unit, score) in ranked {
                    if hits.iter().any(|h| h.tag.eq_ignore_ascii_case(&tag)) {
                        continue;
                    }
                    hits.push(infoods::InfoodsMatch {
                        tag,
                        name,
                        unit,
                        via: "search",
                        score,
                    });
                }
            }
            hits.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            hits.truncate(30);
            if json {
                let out: Vec<_> = hits
                    .iter()
                    .map(|h| {
                        serde_json::json!({
                            "tag": h.tag,
                            "name": h.name,
                            "unit": h.unit,
                            "via": h.via,
                            "score": h.score,
                        })
                    })
                    .collect();
                print_json(&out);
            } else {
                for h in hits {
                    let unit = h.unit.as_deref().unwrap_or("?");
                    println!("{}: {} ({}) [{:.2}]", h.tag, h.name, unit, h.score);
                }
            }
        }
    }
    Ok(())
}

fn handle_taxonomy(
    table: &str,
    assoc_table: &str,
    assoc_tag_col: &str,
    action: TaxonomyAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        TaxonomyAction::Create { name } => {
            let id = ensure_tag(&conn, table, &name)?;
            if json {
                print_json(&Success::created(id, name.clone(), "created"));
            } else {
                quiet_print(quiet, format!("Created {table} {id}: {name}"));
            }
        }
        TaxonomyAction::List => {
            let mut stmt = conn.prepare(&format!("SELECT id, name FROM {table} ORDER BY name"))?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    println!("{}: {}", r["id"], r["name"]);
                }
            }
        }
        TaxonomyAction::Search { query } => {
            let mut stmt = conn.prepare(&format!("SELECT id, name FROM {table}"))?;
            let cands: Vec<_> = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            let ranked = fuzzy_rank(cands, &query);
            if json {
                let out: Vec<_> = ranked
                    .iter()
                    .map(|(id, nm, s)| serde_json::json!({"id": id, "name": nm, "score": s}))
                    .collect();
                print_json(&out);
            } else {
                for (id, nm, s) in ranked {
                    println!("{id}: {nm} ({s:.2})");
                }
            }
        }
        TaxonomyAction::Show { id } => {
            let name: Option<String> = conn
                .query_row(
                    &format!("SELECT name FROM {table} WHERE id=?1"),
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            match name {
                Some(n) => {
                    if json {
                        print_json(&serde_json::json!({"id": id, "name": n}));
                    } else {
                        println!("{id}: {n}");
                    }
                }
                None => return Err(anyhow!("not found")),
            }
        }
        TaxonomyAction::Delete { id } => {
            let _ = conn.execute(
                &format!("DELETE FROM {assoc_table} WHERE {assoc_tag_col}=?1"),
                [id],
            );
            let n = conn.execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])?;
            if n == 0 {
                return Err(anyhow!("not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted {id}"));
            }
        }
    }
    Ok(())
}

fn handle_store(
    action: StoreAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        StoreAction::Create { name } => {
            let now = db::now_utc();
            conn.execute(
                "INSERT INTO stores (name, created_at) VALUES (?1, ?2)",
                params![name, now],
            )?;
            let id = conn.last_insert_rowid();
            entity_audit::append_create(&conn, entity_audit::entity::STORE, id, None)?;
            if json {
                print_json(&Success::created(id, name.clone(), "store created"));
            } else {
                quiet_print(quiet, format!("Created store {id}: {name}"));
            }
        }
        StoreAction::List => {
            let mut stmt = conn.prepare("SELECT id, name FROM stores ORDER BY name")?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    println!("{}: {}", r["id"], r["name"]);
                }
            }
        }
        StoreAction::Show { id } => {
            let name: Option<String> = conn
                .query_row("SELECT name FROM stores WHERE id=?1", [id], |r| r.get(0))
                .optional()?;
            match name {
                Some(n) => {
                    if json {
                        print_json(&serde_json::json!({"id": id, "name": n}));
                    } else {
                        println!("{id}: {n}");
                    }
                }
                None => return Err(anyhow!("store not found")),
            }
        }
        StoreAction::Rename { id, name } => {
            let old_name: Option<String> = conn
                .query_row("SELECT name FROM stores WHERE id = ?1", [id], |r| r.get(0))
                .optional()?;
            let Some(old_name) = old_name else {
                return Err(anyhow!("store not found"));
            };
            let n = conn.execute(
                "UPDATE stores SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
            if n == 0 {
                return Err(anyhow!("store not found"));
            }
            let fields = [entity_audit::FieldChange::new(
                "name",
                serde_json::json!(old_name),
                serde_json::json!(name),
            )];
            entity_audit::append_catalog(
                &conn,
                entity_audit::entity::STORE,
                id,
                &format!("renamed to {name}"),
                Some(&fields),
                None,
            )?;
            if json {
                print_json(&Success::created(id, name, "store renamed"));
            } else {
                quiet_print(quiet, format!("Renamed store {id}"));
            }
        }
        StoreAction::Delete { id } => {
            let n = conn.execute("DELETE FROM stores WHERE id=?1", [id])?;
            if n == 0 {
                return Err(anyhow!("store not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted store {id}"));
            }
        }
        StoreAction::Tag { action } => match action {
            TagModifyAction::Add { id, tag } => {
                let tid = ensure_tag(&conn, "store_tags", &tag)?;
                conn.execute(
                    "INSERT OR IGNORE INTO store_tag_associations (store_id, tag_id) VALUES (?1,?2)",
                    params![id, tid],
                )?;
                if json {
                    print_json(&Success::ok(format!("tag {tag} added to store {id}")));
                } else {
                    quiet_print(quiet, format!("Tagged store {id} with {tag}"));
                }
            }
            TagModifyAction::Remove { id, tag } => {
                let tid: Option<i64> = conn
                    .query_row("SELECT id FROM store_tags WHERE name=?1", [&tag], |r| {
                        r.get(0)
                    })
                    .optional()?;
                if let Some(t) = tid {
                    conn.execute(
                        "DELETE FROM store_tag_associations WHERE store_id=?1 AND tag_id=?2",
                        params![id, t],
                    )?;
                }
                if json {
                    print_json(&Success::ok(format!("tag {tag} removed from store {id}")));
                } else {
                    quiet_print(quiet, format!("Removed tag {tag} from store {id}"));
                }
            }
        },
        StoreAction::Audit { id, limit } => {
            handle_event_audit(
                &conn,
                "stores",
                entity_audit::entity::STORE,
                id,
                limit,
                json,
                |c, i| {
                    c.query_row(
                        "SELECT id, name, created_at FROM stores WHERE id=?1",
                        [i],
                        |r| {
                            Ok(serde_json::json!({
                                "id": r.get::<_, i64>(0)?,
                                "name": r.get::<_, String>(1)?,
                                "created_at": r.get::<_, String>(2)?,
                            }))
                        },
                    )
                    .optional()
                    .map_err(Into::into)
                },
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod search_tests {
    use super::{fuzzy_rank, name_match_score};

    #[test]
    fn substring_beats_fuzzy_false_positive() {
        let milk_score = name_match_score("Cappuccino (whole milk, no sugar)", "milk");
        let milanesa_score = name_match_score("Milanesa de Ternera Ofe", "milk");
        assert!(milk_score > milanesa_score);
        assert!(milk_score >= 0.9);
        assert_eq!(milanesa_score, 0.0);
    }

    #[test]
    fn prefix_match_for_short_query() {
        assert!(name_match_score("Banana Bunch", "ban") >= 0.85);
    }

    #[test]
    fn multi_word_query_matches_vitamin_d() {
        let score = name_match_score("Vitamin D", "vit d");
        assert!(score >= 0.8);
        assert_eq!(name_match_score("Vitamin B6", "vit d"), 0.0);
        assert_eq!(
            name_match_score("Pantothenic acid (Vitamin B5)", "vit d"),
            0.0
        );
    }

    #[test]
    fn short_query_does_not_match_unrelated_words() {
        // Regression: bare Jaro-Winkler scored "virgin"/"original"/"rojo" ~0.67–0.75 for "iron".
        assert_eq!(name_match_score("Virgin Olive Oil", "iron"), 0.0);
        assert_eq!(name_match_score("Espadol Jabon 80g Original", "iron"), 0.0);
        assert_eq!(
            name_match_score("Morrón Rojo (Red Bell Pepper)", "iron"),
            0.0
        );
        assert!(name_match_score("Gentech Iron Bar Dulce de Leche Crunch", "iron") >= 0.9);
    }

    #[test]
    fn fuzzy_rank_filters_irrelevant_products() {
        let items = vec![
            (37, "Milanesa de Ternera Ofe".into()),
            (15, "Cappuccino (whole milk, no sugar)".into()),
            (1, "Pomelo".into()),
            (3, "Gentech Iron Bar Dulce de Leche Crunch".into()),
            (16, "Virgin Olive Oil".into()),
        ];
        let ranked = fuzzy_rank(items, "milk");
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, 15);

        let ranked_iron = fuzzy_rank(
            vec![
                (3, "Gentech Iron Bar Dulce de Leche Crunch".into()),
                (16, "Virgin Olive Oil".into()),
                (41, "Espadol Jabon 80g Original".into()),
            ],
            "iron",
        );
        assert_eq!(ranked_iron.len(), 1);
        assert_eq!(ranked_iron[0].0, 3);
    }
}
