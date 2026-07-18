//! INFOODS food-component tagnames (vendored FAO reference catalog).
//!
//! Seeded into `infoods_components` / `infoods_synonyms` on migration.
//! Used to prevent accidental duplicate micronutrient catalog rows.

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

/// Vendored JSON produced by `scripts/import_infoods.py`.
const INFOODS_JSON: &str = include_str!("../data/infoods/infoods_components.json");

#[derive(Debug, Clone, Deserialize)]
struct InfoodsFile {
    components: Vec<InfoodsComponentSeed>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InfoodsComponentSeed {
    pub tag: String,
    pub name: String,
    pub unit: Option<String>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    pub comments: Option<String>,
    pub tables_note: Option<String>,
    pub source: String,
}

/// A candidate match against the INFOODS catalog.
#[derive(Debug, Clone)]
pub struct InfoodsMatch {
    pub tag: String,
    pub name: String,
    pub unit: Option<String>,
    /// How it matched: "tag", "name", "synonym", "fuzzy".
    pub via: &'static str,
    pub score: f64,
}

/// Parse the vendored JSON (for tests / seed).
pub fn load_seed() -> Result<Vec<InfoodsComponentSeed>> {
    let file: InfoodsFile =
        serde_json::from_str(INFOODS_JSON).context("parse vendored infoods_components.json")?;
    Ok(file.components)
}

/// Create INFOODS tables and seed from vendored JSON (idempotent inserts).
pub fn ensure_schema_and_seed(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS infoods_components (
            tag TEXT PRIMARY KEY COLLATE NOCASE,
            name TEXT NOT NULL,
            unit TEXT,
            synonyms TEXT,
            comments TEXT,
            tables_note TEXT,
            source TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS infoods_synonyms (
            synonym TEXT NOT NULL COLLATE NOCASE,
            tag TEXT NOT NULL REFERENCES infoods_components(tag) ON DELETE CASCADE,
            PRIMARY KEY (synonym, tag)
        );
        CREATE INDEX IF NOT EXISTS idx_infoods_synonyms_synonym
            ON infoods_synonyms(synonym);
        "#,
    )?;

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM infoods_components", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }

    let now = crate::db::now_utc();
    let components = load_seed()?;
    let tx = conn.unchecked_transaction()?;
    {
        let mut ins = tx.prepare(
            "INSERT OR IGNORE INTO infoods_components
             (tag, name, unit, synonyms, comments, tables_note, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut ins_syn =
            tx.prepare("INSERT OR IGNORE INTO infoods_synonyms (synonym, tag) VALUES (?1, ?2)")?;
        for c in &components {
            let syn_blob = if c.synonyms.is_empty() {
                None
            } else {
                Some(c.synonyms.join("; "))
            };
            ins.execute(params![
                c.tag,
                c.name,
                c.unit,
                syn_blob,
                c.comments,
                c.tables_note,
                c.source,
                now,
            ])?;
            // Index primary name + each synonym for exact CI lookup.
            ins_syn.execute(params![c.name, c.tag])?;
            for s in &c.synonyms {
                let s = s.trim();
                if s.len() >= 2 {
                    ins_syn.execute(params![s, c.tag])?;
                }
            }
            // Tag itself as a synonym so "VITC" resolves.
            ins_syn.execute(params![c.tag, c.tag])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Normalize micronutrient / INFOODS unit spellings (no magnitude conversion).
pub fn normalize_unit(unit: &str) -> String {
    let u = unit.trim();
    let lower = u.to_lowercase().replace(['μ', 'µ'], "u"); // normalize micro sign spellings before match
    match lower.as_str() {
        "ug" | "mcg" => "µg".to_string(),
        "mg" => "mg".to_string(),
        "g" => "g".to_string(),
        "kg" => "kg".to_string(),
        "iu" => "IU".to_string(),
        "%" => "%".to_string(),
        _ => u.to_string(),
    }
}

/// Exact case-insensitive lookups against tag, name, and synonym index.
pub fn find_exact_matches(conn: &Connection, query: &str) -> Result<Vec<InfoodsMatch>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let mut by_tag: std::collections::BTreeMap<String, InfoodsMatch> =
        std::collections::BTreeMap::new();

    // Tag exact
    if let Some((tag, name, unit)) = conn
        .query_row(
            "SELECT tag, name, unit FROM infoods_components WHERE tag = ?1 COLLATE NOCASE",
            [q],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
    {
        by_tag.insert(
            tag.to_uppercase(),
            InfoodsMatch {
                tag,
                name,
                unit,
                via: "tag",
                score: 1.0,
            },
        );
    }

    // Name exact
    let mut stmt = conn
        .prepare("SELECT tag, name, unit FROM infoods_components WHERE name = ?1 COLLATE NOCASE")?;
    let rows = stmt.query_map([q], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (tag, name, unit) = row?;
        by_tag.entry(tag.to_uppercase()).or_insert(InfoodsMatch {
            tag,
            name,
            unit,
            via: "name",
            score: 1.0,
        });
    }

    // Synonym exact
    let mut stmt = conn.prepare(
        "SELECT c.tag, c.name, c.unit
         FROM infoods_synonyms s
         JOIN infoods_components c ON c.tag = s.tag
         WHERE s.synonym = ?1 COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([q], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (tag, name, unit) = row?;
        by_tag.entry(tag.to_uppercase()).or_insert(InfoodsMatch {
            tag,
            name,
            unit,
            via: "synonym",
            score: 0.98,
        });
    }

    Ok(by_tag.into_values().collect())
}

/// Fuzzy name matches (high threshold) — secondary signal for create gate.
pub fn find_fuzzy_matches(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<InfoodsMatch>> {
    use strsim::jaro_winkler;

    let q = query.trim().to_lowercase();
    if q.len() < 3 {
        return Ok(vec![]);
    }
    let mut stmt = conn.prepare("SELECT tag, name, unit FROM infoods_components")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut scored = Vec::new();
    for row in rows {
        let (tag, name, unit) = row?;
        let n = name.to_lowercase();
        let mut score = 0.0_f64;
        if n == q {
            score = 1.0;
        } else if n.contains(&q) || q.contains(&n) {
            score = 0.92;
        } else {
            let jw = jaro_winkler(&n, &q);
            if jw >= 0.92 {
                score = jw;
            }
        }
        if score >= 0.92 {
            scored.push(InfoodsMatch {
                tag,
                name,
                unit,
                via: "fuzzy",
                score,
            });
        }
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);
    Ok(scored)
}

/// All INFOODS candidates that should block an unforced micronutrient create.
pub fn find_create_blockers(conn: &Connection, name: &str) -> Result<Vec<InfoodsMatch>> {
    let mut exact = find_exact_matches(conn, name)?;
    if !exact.is_empty() {
        exact.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        return Ok(exact);
    }
    find_fuzzy_matches(conn, name, 5)
}

pub fn get_component(
    conn: &Connection,
    tag: &str,
) -> Result<Option<(String, String, Option<String>)>> {
    conn.query_row(
        "SELECT tag, name, unit FROM infoods_components WHERE tag = ?1 COLLATE NOCASE",
        [tag],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn tag_exists(conn: &Connection, tag: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM infoods_components WHERE tag = ?1 COLLATE NOCASE",
        [tag],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

pub fn format_blockers_message(name: &str, blockers: &[InfoodsMatch]) -> String {
    let mut lines = vec![format!(
        "name '{name}' matches INFOODS component(s); refuse create to avoid catalog forks.\n\
         Link deliberately with --infoods TAG, or pass --force if this is truly a custom nutrient:"
    )];
    for b in blockers.iter().take(8) {
        let unit = b.unit.as_deref().unwrap_or("?");
        lines.push(format!(
            "  - {} ({}) unit={} via={} score={:.2}",
            b.tag, b.name, unit, b.via, b.score
        ));
    }
    lines.push(
        "Example: recomplog nutrition micronutrient create \"Vitamin C\" --unit mg --infoods VITC"
            .into(),
    );
    lines.join("\n")
}

pub fn force_warning_message(name: &str, blockers: &[InfoodsMatch]) -> String {
    let tags: Vec<_> = blockers.iter().map(|b| b.tag.as_str()).collect();
    format!(
        "FORCE: created micronutrient '{name}' without INFOODS link despite similar INFOODS \
         component(s) [{}]. Prefer --infoods TAG to link, or map later. db check will flag \
         untagged micronutrients.",
        tags.join(", ")
    )
}

/// Result of resolving or inserting a micronutrient catalog row.
#[derive(Debug)]
pub struct EnsuredMicro {
    pub id: i64,
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub created: bool,
    #[allow(dead_code)]
    pub infoods_tag: Option<String>,
}

pub enum EnsureMode {
    /// Product nutrition set: get-or-create; auto-link single exact INFOODS hit.
    ProductSet,
}

/// Get-or-create micronutrient by case-insensitive name.
pub fn ensure_micronutrient(
    conn: &Connection,
    name: &str,
    unit: &str,
    mode: EnsureMode,
) -> Result<EnsuredMicro> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("micronutrient name must not be empty"));
    }
    if crate::macro_names::is_macronutrient_name(name) {
        let flag = crate::macro_names::macro_flag_hint(name)
            .unwrap_or("product nutrition set macro flags");
        return Err(anyhow!(
            "'{name}' is a macronutrient; use {flag} on product nutrition set \
             (not --micronutrient)"
        ));
    }
    let unit = normalize_unit(unit);

    if let Some((id, existing_name, tag)) = conn
        .query_row(
            "SELECT id, name, infoods_tag FROM micronutrients WHERE name = ?1 COLLATE NOCASE",
            [name],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
    {
        return Ok(EnsuredMicro {
            id,
            name: existing_name,
            created: false,
            infoods_tag: tag,
        });
    }

    let infoods_tag = match mode {
        EnsureMode::ProductSet => {
            let exact = find_exact_matches(conn, name)?;
            if exact.len() > 1 {
                let list: Vec<_> = exact
                    .iter()
                    .map(|m| format!("{} ({})", m.tag, m.name))
                    .collect();
                return Err(anyhow!(
                    "micronutrient '{name}' matches multiple INFOODS components: {}. \
                     Create explicitly with --infoods TAG first.",
                    list.join(", ")
                ));
            }
            exact.into_iter().next().map(|m| m.tag)
        }
    };

    // If auto-linked, unit may come from INFOODS when caller unit empty (not the case here).
    let now = crate::db::now_utc();
    conn.execute(
        "INSERT INTO micronutrients (name, unit, recommended_intake, created_at, infoods_tag)
         VALUES (?1, ?2, NULL, ?3, ?4)",
        params![name, unit, now, infoods_tag],
    )?;
    let id = conn.last_insert_rowid();
    Ok(EnsuredMicro {
        id,
        name: name.to_string(),
        created: true,
        infoods_tag,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_json_parses_and_has_classics() {
        let comps = load_seed().unwrap();
        assert!(
            comps.len() > 500,
            "expected full INFOODS list, got {}",
            comps.len()
        );
        let tags: std::collections::HashSet<_> = comps.iter().map(|c| c.tag.as_str()).collect();
        for t in ["VITC", "CA", "FE", "ZN", "MG", "NIA", "THIA", "RIBF"] {
            assert!(tags.contains(t), "missing {t}");
        }
    }

    #[test]
    fn normalize_unit_ug() {
        assert_eq!(normalize_unit("ug"), "µg");
        assert_eq!(normalize_unit("mcg"), "µg");
        assert_eq!(normalize_unit("MG"), "mg");
    }
}
