//! Nutrition domain handlers.

use crate::cli::{
    ConsumptionAction, NutrientAction, NutritionAction, ProductAction, PurchaseAction,
};
use crate::db;
use crate::models::Success;
use crate::utils::{parse_date_to_ymd, print_error_json, print_json, quiet_print};
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
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
    }
}

fn name_match_score(name: &str, query: &str) -> f64 {
    let name_l = name.to_lowercase();
    let query_l = query.to_lowercase();
    if name_l.contains(&query_l) {
        return 0.95 + 0.05 * (query_l.len() as f64 / name_l.len().max(1) as f64);
    }
    let name_words: Vec<&str> = name_l.split_whitespace().collect();
    let query_words: Vec<&str> = query_l.split_whitespace().collect();
    if query_words.is_empty() {
        return 0.0;
    }
    let mut total = 0.0;
    for qw in &query_words {
        let best = name_words
            .iter()
            .map(|nw| jaro_winkler(nw, qw))
            .fold(0.0_f64, f64::max);
        total += best;
    }
    total / query_words.len() as f64
}

fn fuzzy_rank(items: Vec<(i64, String)>, query: &str) -> Vec<(i64, String, f64)> {
    let mut ranked: Vec<_> = items
        .into_iter()
        .map(|(id, name)| {
            let score = name_match_score(&name, query);
            (id, name, score)
        })
        .filter(|(_, _, s)| *s >= 0.55)
        .collect();
    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(30);
    ranked
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
                    conn.execute(
                        "INSERT OR IGNORE INTO product_tags (name, created_at) VALUES (?1, ?2)",
                        params![t, now],
                    )?;
                    let tag_id: i64 =
                        conn.query_row("SELECT id FROM product_tags WHERE name = ?1", [t], |r| {
                            r.get(0)
                        })?;
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
                    format!("product created: {}", name),
                ));
            } else {
                quiet_print(quiet, format!("Created product {} ({})", id, name));
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
                        println!("{}: {} ({:.2})", id, nm, score);
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
        ProductAction::Show { id } => {
            let name: Option<String> = conn
                .query_row("SELECT name FROM products WHERE id=?", [id], |r| r.get(0))
                .optional()?;
            match name {
                Some(n) => {
                    // tags
                    let mut stmt = conn.prepare(
                        "SELECT t.name FROM product_tags t
                         JOIN product_tag_associations a ON a.tag_id = t.id
                         WHERE a.product_id = ?1",
                    )?;
                    let tags: Vec<String> = stmt
                        .query_map([id], |r| r.get(0))?
                        .filter_map(|r| r.ok())
                        .collect();
                    // nutrition
                    let nutrition = conn
                        .query_row(
                            "SELECT reference_quantity, reference_unit, energy_kcal, protein_g, carbohydrates_g, fat_g, fiber_g, sugars_g
                             FROM product_nutritions WHERE product_id = ?1",
                            [id],
                            |r| {
                                Ok(serde_json::json!({
                                    "reference_quantity": r.get::<_, f64>(0)?,
                                    "reference_unit": r.get::<_, String>(1)?,
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
                    let out = serde_json::json!({
                        "id": id,
                        "name": n,
                        "tags": tags,
                        "nutrition": nutrition,
                    });
                    if json {
                        print_json(&out);
                    } else {
                        println!("{}: {}", id, n);
                        if !tags.is_empty() {
                            println!("  tags: {}", tags.join(", "));
                        }
                        if let Some(nu) = nutrition {
                            println!("  nutrition: {}", nu);
                        }
                    }
                }
                None => {
                    if json {
                        print_error_json("product not found");
                    } else {
                        eprintln!("product not found");
                    }
                    return Err(anyhow!("product not found"));
                }
            }
        }
        ProductAction::Rename { id, new_name } => {
            let n = conn.execute(
                "UPDATE products SET name = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_name, db::now_utc(), id],
            )?;
            if n == 0 {
                return Err(anyhow!("product not found"));
            }
            if json {
                print_json(&Success::created(id, new_name.clone(), "product renamed"));
            } else {
                quiet_print(quiet, format!("Renamed product {} to {}", id, new_name));
            }
        }
        ProductAction::Delete { id } => {
            let n = conn.execute("DELETE FROM products WHERE id = ?1", [id])?;
            if n == 0 {
                return Err(anyhow!("product not found"));
            }
            if json {
                print_json(&Success::deleted(id));
            } else {
                quiet_print(quiet, format!("Deleted product {}", id));
            }
        }
        ProductAction::Set {
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
            let exists: Option<i64> = conn
                .query_row("SELECT id FROM products WHERE id=?1", [id], |r| r.get(0))
                .optional()?;
            if exists.is_none() {
                return Err(anyhow!("product not found"));
            }
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
            if json {
                print_json(&Success::created(id, "nutrition", "product nutrition set"));
            } else {
                quiet_print(quiet, format!("Set nutrition for product {}", id));
            }
        }
        ProductAction::TagAdd { id, tag } => {
            let now = db::now_utc();
            conn.execute(
                "INSERT OR IGNORE INTO product_tags (name, created_at) VALUES (?1, ?2)",
                params![tag, now],
            )?;
            let tag_id: i64 =
                conn.query_row("SELECT id FROM product_tags WHERE name = ?1", [&tag], |r| {
                    r.get(0)
                })?;
            conn.execute(
                "INSERT OR IGNORE INTO product_tag_associations (product_id, tag_id) VALUES (?1, ?2)",
                params![id, tag_id],
            )?;
            if json {
                print_json(&Success::ok(format!("tag {} added to product {}", tag, id)));
            } else {
                quiet_print(quiet, format!("Tagged product {} with {}", id, tag));
            }
        }
        ProductAction::TagRemove { id, tag } => {
            let tag_id: Option<i64> = conn
                .query_row("SELECT id FROM product_tags WHERE name = ?1", [&tag], |r| {
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
                print_json(&Success::ok(format!(
                    "tag {} removed from product {}",
                    tag, id
                )));
            } else {
                quiet_print(quiet, format!("Removed tag {} from product {}", tag, id));
            }
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
        } => {
            let now = db::now_utc();
            let price_cents: Option<i64> = price
                .and_then(|p| p.replace(['$', ','], "").parse::<f64>().ok())
                .map(|v| (v * 100.0).round() as i64);
            conn.execute(
                "INSERT INTO purchases (product_id, quantity, price_cents, store_id, purchased_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![product, quantity, price_cents, store, now],
            )?;
            let id = conn.last_insert_rowid();
            if json {
                print_json(&Success::created(id, "purchase", "purchase recorded"));
            } else {
                quiet_print(quiet, format!("Purchase {} recorded", id));
            }
        }
        PurchaseAction::List => {
            let mut stmt = conn.prepare(
                "SELECT pu.id, pu.product_id, p.name, pu.quantity, pu.price_cents, pu.purchased_at
                 FROM purchases pu LEFT JOIN products p ON p.id = pu.product_id
                 ORDER BY pu.id DESC LIMIT 50",
            )?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "product_id": r.get::<_, i64>(1)?,
                        "product_name": r.get::<_, Option<String>>(2)?,
                        "quantity": r.get::<_, f64>(3)?,
                        "price_cents": r.get::<_, Option<i64>>(4)?,
                        "purchased_at": r.get::<_, String>(5)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    println!(
                        "{}: product={} qty={} price_cents={:?}",
                        r["id"], r["product_id"], r["quantity"], r["price_cents"]
                    );
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
    quiet: bool,
) -> Result<()> {
    let conn = db::open_db(db_override)?;
    match action {
        ConsumptionAction::Create {
            product,
            quantity,
            date,
        } => {
            let when = if let Some(d) = date {
                parse_date_to_ymd(&d)?
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
                print_json(&Success::created(id, when, "consumption logged"));
            } else {
                quiet_print(quiet, format!("Consumption {} logged", id));
            }
        }
        ConsumptionAction::List => {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.product_id, p.name, c.quantity, c.consumed_at
                 FROM consumptions c LEFT JOIN products p ON p.id = c.product_id
                 ORDER BY c.consumed_at DESC LIMIT 50",
            )?;
            let rows: Vec<_> = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "product_id": r.get::<_, i64>(1)?,
                        "product_name": r.get::<_, Option<String>>(2)?,
                        "quantity": r.get::<_, f64>(3)?,
                        "consumed_at": r.get::<_, String>(4)?,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
            if json {
                print_json(&rows);
            } else {
                for r in &rows {
                    println!(
                        "{}: {} qty={} on {}",
                        r["id"], r["product_name"], r["quantity"], r["consumed_at"]
                    );
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
                    format!("nutrient: {}", name),
                ));
            } else {
                quiet_print(quiet, format!("Created nutrient {}: {}", id, name));
            }
        }
    }
    Ok(())
}
