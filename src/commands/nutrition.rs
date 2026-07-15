//! Nutrition domain handlers (nutlog parity under grouped CLI).

use crate::cli::{
    ConsumptionAction, NutrientAction, NutritionAction, ProductAction, ProductNutritionAction,
    PurchaseAction, StoreAction, TagModifyAction, TaxonomyAction,
};
use crate::db;
use crate::models::Success;
use crate::utils::{
    parse_date_to_ymd, parse_rfc3339_instant_for_db, parse_rfc3339_to_utc, print_error_json,
    print_json, quiet_print, refuse_consumption_midnight,
};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use strsim::jaro_winkler;

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
        NutritionAction::Nutrient { action } => handle_nutrient(action, db_override, json, quiet),
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

#[allow(clippy::too_many_arguments)]
fn set_product_nutrition(
    conn: &Connection,
    id: i64,
    reference_quantity: f64,
    reference_unit: &str,
    energy_kcal: Option<f64>,
    protein_g: Option<f64>,
    carbohydrates_g: Option<f64>,
    fat_g: Option<f64>,
    fiber_g: Option<f64>,
    sugars_g: Option<f64>,
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
    conn.execute(
        "INSERT INTO product_nutritions
         (product_id, reference_quantity, reference_unit, energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(product_id) DO UPDATE SET
           reference_quantity=excluded.reference_quantity,
           reference_unit=excluded.reference_unit,
           energy_kcal=excluded.energy_kcal,
           protein_g=excluded.protein_g,
           carbohydrates_g=excluded.carbohydrates_g,
           fat_g=excluded.fat_g,
           fiber_g=excluded.fiber_g,
           sugars_g=excluded.sugars_g",
        params![
            id,
            reference_quantity,
            reference_unit,
            energy_kcal,
            protein_g,
            carbohydrates_g,
            fat_g,
            fiber_g,
            sugars_g
        ],
    )?;
    // Replace micronutrients when provided
    if !micros.is_empty() {
        conn.execute(
            "DELETE FROM product_micronutrients WHERE product_id = ?1",
            [id],
        )?;
        let now = db::now_utc();
        for (name, amount, unit) in micros {
            conn.execute(
                "INSERT OR IGNORE INTO nutrients (name, unit, created_at) VALUES (?1, ?2, ?3)",
                params![name, unit, now],
            )?;
            let nid: i64 = conn.query_row(
                "SELECT id FROM nutrients WHERE name = ?1 COLLATE NOCASE",
                [name],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO product_micronutrients (product_id, nutrient_id, amount, unit)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, nid, amount, unit],
            )?;
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
            let mut stmt =
                conn.prepare("SELECT id, name, created_at FROM products ORDER BY id DESC")?;
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
                let mut stmt = conn.prepare("SELECT id, name FROM products")?;
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
            let n = conn.execute(
                "UPDATE products SET name = ?1, updated_at = ?2 WHERE id = ?3",
                params![name, db::now_utc(), id],
            )?;
            if n == 0 {
                return Err(anyhow!("product not found"));
            }
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
                    micronutrient,
                    json_file,
                },
        } => {
            let (rq, ru, e, p, c, f, fi, su, micros) = if let Some(path) = json_file {
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
                let macros = &v["macros"];
                let mut micros = vec![];
                if let Some(arr) = v["micronutrients"].as_array() {
                    for m in arr {
                        micros.push((
                            m["name"].as_str().unwrap_or("").to_string(),
                            m["amount"].as_f64().unwrap_or(0.0),
                            m["unit"].as_str().unwrap_or("mg").to_string(),
                        ));
                    }
                }
                (
                    rq,
                    ru,
                    macros["energy_kcal"].as_f64().or(v["energy_kcal"].as_f64()),
                    macros["protein_g"].as_f64().or(v["protein_g"].as_f64()),
                    macros["carbohydrates_g"]
                        .as_f64()
                        .or(v["carbohydrates_g"].as_f64()),
                    macros["fat_g"].as_f64().or(v["fat_g"].as_f64()),
                    macros["fiber_g"].as_f64().or(v["fiber_g"].as_f64()),
                    macros["sugars_g"].as_f64().or(v["sugars_g"].as_f64()),
                    micros,
                )
            } else {
                let rq = reference_quantity.ok_or_else(|| {
                    anyhow!("--reference-quantity required unless --json-file is used")
                })?;
                let ru = reference_unit.unwrap_or_else(|| "g".into());
                let micros = parse_micronutrient_triples(&micronutrient)?;
                (
                    rq,
                    ru,
                    energy_kcal,
                    protein_g,
                    carbohydrates_g,
                    fat_g,
                    fiber_g,
                    sugars_g,
                    micros,
                )
            };
            set_product_nutrition(&conn, id, rq, &ru, e, p, c, f, fi, su, &micros)?;
            if json {
                print_json(&Success::created(id, "nutrition", "product nutrition set"));
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
            set_product_nutrition(
                &conn,
                id,
                reference_quantity,
                &reference_unit,
                energy_kcal,
                protein_g,
                carbohydrates_g,
                fat_g,
                fiber_g,
                sugars_g,
                &[],
            )?;
            if json {
                print_json(&Success::created(id, "nutrition", "product nutrition set"));
            } else {
                quiet_print(quiet, format!("Set nutrition for product {id}"));
            }
        }
        ProductAction::TagAdd { id, tag } => tag_add_product(&conn, id, &tag, json, quiet)?,
        ProductAction::TagRemove { id, tag } => tag_remove_product(&conn, id, &tag, json, quiet)?,
    }
    Ok(())
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
    let name: Option<String> = conn
        .query_row("SELECT name FROM products WHERE id=?", [id], |r| r.get(0))
        .optional()?;
    let Some(n) = name else {
        if json {
            print_error_json("product not found");
        }
        return Err(anyhow!("product not found"));
    };
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
            "SELECT reference_quantity, reference_unit, energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g
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
                }))
            },
        )
        .optional()?;
    let mut mstmt = conn.prepare(
        "SELECT n.name, pm.amount, pm.unit FROM product_micronutrients pm
         JOIN nutrients n ON n.id = pm.nutrient_id
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
        "tags": tags,
        "nutrition": nutrition,
        "micronutrients": micros,
    });
    if json {
        print_json(&out);
    } else {
        println!("{id}: {n}");
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
        PurchaseAction::List {
            since,
            until,
            product,
            store,
        } => {
            let mut sql = String::from(
                "SELECT pu.id, pu.product_id, p.name, pu.quantity, pu.price_cents, pu.store_id, \
                 pu.purchased_at, pu.created_at
                 FROM purchases pu LEFT JOIN products p ON p.id = pu.product_id WHERE 1=1",
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
            let rows: Vec<_> = stmt
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
                    "SELECT id, product_id, quantity, price_cents, store_id, purchased_at, created_at \
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
        PurchaseAction::Delete { id } => {
            let n = conn.execute("DELETE FROM purchases WHERE id=?1", [id])?;
            if n == 0 {
                return Err(anyhow!("purchase not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted purchase {id}"));
            }
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
            let product_name: Option<String> = conn
                .query_row("SELECT name FROM products WHERE id = ?1", [product], |r| {
                    r.get(0)
                })
                .optional()?;
            if product_name.is_none() {
                return Err(anyhow!("product {product} not found"));
            }
            let nutrition: Option<(f64, String)> = conn
                .query_row(
                    "SELECT reference_quantity, reference_unit FROM product_nutritions
                     WHERE product_id = ?1",
                    [product],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((ref_qty, ref_unit)) = nutrition else {
                return Err(anyhow!(
                    "product {product} has no nutrition set; run \
                     `nutrition product nutrition set {product} --reference-quantity … \
                     --reference-unit g|ml|unit` first"
                ));
            };
            let resolved = crate::nutrition_units::resolve_consumption(
                quantity,
                unit.as_deref(),
                ref_qty,
                &ref_unit,
            )?;
            conn.execute(
                "INSERT INTO consumptions (product_id, quantity, unit, consumed_at, created_at) VALUES (?1,?2,?3,?4,?5)",
                params![product, resolved.quantity, resolved.unit, when, now],
            )?;
            let id = conn.last_insert_rowid();
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
                    "product_reference_unit": ref_unit,
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
        ConsumptionAction::List {
            since,
            until,
            product,
        } => {
            let mut sql = String::from(
                "SELECT c.id, c.product_id, p.name, c.quantity, c.unit, c.consumed_at, c.created_at
                 FROM consumptions c LEFT JOIN products p ON p.id = c.product_id WHERE 1=1",
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
            let rows: Vec<_> = stmt
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
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    let unit = r["unit"].as_str().unwrap_or("?");
                    println!(
                        "{}: {} {} {} on {}",
                        r["id"], r["product_name"], r["quantity"], unit, r["consumed_at"]
                    );
                }
            }
        }
        ConsumptionAction::Delete { id } => {
            let n = conn.execute("DELETE FROM consumptions WHERE id=?1", [id])?;
            if n == 0 {
                return Err(anyhow!("consumption not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted consumption {id}"));
            }
        }
    }
    Ok(())
}

fn handle_nutrient(
    action: NutrientAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        NutrientAction::List => {
            let mut stmt = conn.prepare(
                "SELECT id, name, unit, recommended_intake FROM nutrients ORDER BY name",
            )?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "unit": r.get::<_, String>(2)?,
                        "recommended_intake": r.get::<_, Option<f64>>(3)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for n in &rows {
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
                print_json(&Success::created(
                    id,
                    name.clone(),
                    format!("nutrient: {name}"),
                ));
            } else {
                quiet_print(quiet, format!("Created nutrient {id}: {name}"));
            }
        }
        NutrientAction::Show { id } => {
            let row = conn
                .query_row(
                    "SELECT id, name, unit, recommended_intake FROM nutrients WHERE id=?1",
                    [id],
                    |r| {
                        Ok(serde_json::json!({
                            "id": r.get::<_, i64>(0)?,
                            "name": r.get::<_, String>(1)?,
                            "unit": r.get::<_, String>(2)?,
                            "recommended_intake": r.get::<_, Option<f64>>(3)?,
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
                None => return Err(anyhow!("nutrient not found")),
            }
        }
        NutrientAction::Search { query } => {
            let mut stmt = conn.prepare("SELECT id, name FROM nutrients")?;
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
        NutrientAction::Delete { id, force } => {
            if !force {
                let refs: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM product_micronutrients WHERE nutrient_id=?1",
                    [id],
                    |r| r.get(0),
                )?;
                if refs > 0 {
                    return Err(anyhow!(
                        "nutrient referenced by products; use --force to delete"
                    ));
                }
            } else {
                conn.execute(
                    "DELETE FROM product_micronutrients WHERE nutrient_id=?1",
                    [id],
                )?;
            }
            let n = conn.execute("DELETE FROM nutrients WHERE id=?1", [id])?;
            if n == 0 {
                return Err(anyhow!("nutrient not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted nutrient {id}"));
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
            let n = conn.execute(
                "UPDATE stores SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
            if n == 0 {
                return Err(anyhow!("store not found"));
            }
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
