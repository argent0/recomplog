//! Shell completion candidates for clap_complete dynamic completion.
//!
//! Static lists stay aligned with runtime validators (`phase`, `load_type`,
//! `nutrition_units`). Completers must never write to stdout and must fail soft.
//!
//! Dynamic completers open the **default** DB path read-only (no migrations).
//! Global `--db` on the partial command line is not consulted (MVP).

use std::ffi::OsStr;

use clap_complete::CompletionCandidate;
use rusqlite::Connection;

use crate::db;

/// Max rows returned per dynamic query (keep tab completion snappy).
const DYNAMIC_LIMIT: i64 = 50;

/// Canonical + common alias phase values for `--phase`.
pub const PHASES: &[&str] = &["full", "eccentric", "concentric", "ecc", "conc"];

/// `--side` values (matches `value_parser` on set commands).
pub const SIDES: &[&str] = &["left", "right", "both"];

/// Canonical load types for `--load-type`.
pub const LOAD_TYPES: &[&str] = &["body_mass", "external", "none"];

/// Canonical nutrition units (`g` / `ml` / `unit`).
pub const NUTRITION_UNITS: &[&str] = &["g", "ml", "unit"];

/// Flexible calendar-day shortcuts offered alongside free-form dates.
pub const DATE_SHORTCUTS: &[&str] = &["today", "yesterday"];

/// Legacy import `--domain` values.
pub const IMPORT_DOMAINS: &[&str] = &["workout", "body", "nutrition"];

/// Filter static candidates by case-insensitive prefix of `current`.
pub fn filter_prefix(current: &OsStr, options: &[&str]) -> Vec<CompletionCandidate> {
    let Some(cur) = current.to_str() else {
        return Vec::new();
    };
    let cur_lower = cur.to_ascii_lowercase();
    options
        .iter()
        .filter(|opt| opt.to_ascii_lowercase().starts_with(cur_lower.as_str()))
        .map(|opt| CompletionCandidate::new(*opt))
        .collect()
}

pub fn complete_phase(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, PHASES)
}

pub fn complete_side(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, SIDES)
}

pub fn complete_load_type(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, LOAD_TYPES)
}

pub fn complete_nutrition_unit(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, NUTRITION_UNITS)
}

pub fn complete_date_shortcut(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, DATE_SHORTCUTS)
}

pub fn complete_import_domain(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, IMPORT_DOMAINS)
}

// ---------- Dynamic (DB-backed) completers ----------

fn with_db<F>(f: F) -> Vec<CompletionCandidate>
where
    F: FnOnce(&Connection) -> Vec<CompletionCandidate>,
{
    match db::open_db_readonly_for_completion(None) {
        Some(conn) => f(&conn),
        None => Vec::new(),
    }
}

fn prefix_str(current: &OsStr) -> Option<&str> {
    current.to_str()
}

/// Exercise names (NOCASE prefix match). Used for `--exercise` and similar.
pub fn complete_exercise(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = prefix_str(current) else {
        return Vec::new();
    };
    with_db(|conn| query_exercises(conn, prefix).unwrap_or_default())
}

/// Product ids; help text is the product name. Prefix matches id string or name.
pub fn complete_product(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = prefix_str(current) else {
        return Vec::new();
    };
    with_db(|conn| query_products(conn, prefix).unwrap_or_default())
}

/// Recent workout ids; help is type + started_at. Prefix matches id string.
pub fn complete_workout(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = prefix_str(current) else {
        return Vec::new();
    };
    with_db(|conn| query_workouts(conn, prefix).unwrap_or_default())
}

/// Store ids; help is store name. Prefix matches id string or name.
pub fn complete_store(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = prefix_str(current) else {
        return Vec::new();
    };
    with_db(|conn| query_stores(conn, prefix).unwrap_or_default())
}

fn query_exercises(conn: &Connection, prefix: &str) -> rusqlite::Result<Vec<CompletionCandidate>> {
    let like = format!("{}%", prefix);
    let mut stmt = conn.prepare(
        "SELECT name FROM exercises
         WHERE name LIKE ?1 COLLATE NOCASE
         ORDER BY name
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![like, DYNAMIC_LIMIT], |row| {
        let name: String = row.get(0)?;
        Ok(CompletionCandidate::new(name))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn query_products(conn: &Connection, prefix: &str) -> rusqlite::Result<Vec<CompletionCandidate>> {
    let like = format!("{}%", prefix);
    // Active products only (merge aliases with retired_at are hidden).
    let mut stmt = conn.prepare(
        "SELECT id, name FROM products
         WHERE retired_at IS NULL
           AND (CAST(id AS TEXT) LIKE ?1 OR name LIKE ?1 COLLATE NOCASE)
         ORDER BY id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![like, DYNAMIC_LIMIT], |row| {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok(CompletionCandidate::new(id.to_string()).help(Some(name.into())))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn query_workouts(conn: &Connection, prefix: &str) -> rusqlite::Result<Vec<CompletionCandidate>> {
    let like = format!("{}%", prefix);
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(workout_type, ''), started_at FROM workouts
         WHERE CAST(id AS TEXT) LIKE ?1
         ORDER BY started_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![like, DYNAMIC_LIMIT], |row| {
        let id: i64 = row.get(0)?;
        let wtype: String = row.get(1)?;
        let started: String = row.get(2)?;
        let help = if wtype.is_empty() {
            started
        } else {
            format!("{wtype} {started}")
        };
        Ok(CompletionCandidate::new(id.to_string()).help(Some(help.into())))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn query_stores(conn: &Connection, prefix: &str) -> rusqlite::Result<Vec<CompletionCandidate>> {
    let like = format!("{}%", prefix);
    let mut stmt = conn.prepare(
        "SELECT id, name FROM stores
         WHERE CAST(id AS TEXT) LIKE ?1 OR name LIKE ?1 COLLATE NOCASE
         ORDER BY name
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![like, DYNAMIC_LIMIT], |row| {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok(CompletionCandidate::new(id.to_string()).help(Some(name.into())))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn values(cands: &[CompletionCandidate]) -> Vec<String> {
        cands
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn phase_prefix_filters() {
        let c = complete_phase(&OsString::from("ecc"));
        let v = values(&c);
        assert!(v.contains(&"eccentric".to_string()) || v.contains(&"ecc".to_string()));
        assert!(!v.iter().any(|s| s == "full"));
    }

    #[test]
    fn empty_prefix_returns_all_phases() {
        let c = complete_phase(&OsString::from(""));
        assert_eq!(values(&c).len(), PHASES.len());
    }

    #[test]
    fn unit_prefix_g() {
        let c = complete_nutrition_unit(&OsString::from("g"));
        assert_eq!(values(&c), vec!["g".to_string()]);
    }

    #[test]
    fn date_shortcuts() {
        let c = complete_date_shortcut(&OsString::from("tod"));
        assert_eq!(values(&c), vec!["today".to_string()]);
    }

    #[test]
    fn import_domain() {
        let c = complete_import_domain(&OsString::from("work"));
        assert_eq!(values(&c), vec!["workout".to_string()]);
    }

    #[test]
    fn dynamic_queries_on_temp_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("c.db");
        // Build a minimal schema and seed rows via full open_db path.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE exercises (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                 CREATE TABLE products (
                   id INTEGER PRIMARY KEY,
                   name TEXT NOT NULL,
                   retired_at TEXT
                 );
                 CREATE TABLE stores (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                 CREATE TABLE workouts (
                   id INTEGER PRIMARY KEY,
                   workout_type TEXT,
                   started_at TEXT NOT NULL
                 );
                 INSERT INTO exercises (name) VALUES ('bench press'), ('pull up');
                 INSERT INTO products (id, name) VALUES (3, 'Oats'), (12, 'Whey');
                 INSERT INTO stores (id, name) VALUES (1, 'Local Mart');
                 INSERT INTO workouts (id, workout_type, started_at)
                   VALUES (7, 'Push', '2026-07-14T17:00:00Z');",
            )
            .unwrap();
        }

        let conn =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();

        let ex = query_exercises(&conn, "ben").unwrap();
        assert_eq!(values(&ex), vec!["bench press".to_string()]);

        let prods = query_products(&conn, "3").unwrap();
        assert!(values(&prods).contains(&"3".to_string()));

        let prods_name = query_products(&conn, "Whe").unwrap();
        assert_eq!(values(&prods_name), vec!["12".to_string()]);

        let wos = query_workouts(&conn, "").unwrap();
        assert_eq!(values(&wos), vec!["7".to_string()]);

        let stores = query_stores(&conn, "Loc").unwrap();
        assert_eq!(values(&stores), vec!["1".to_string()]);
    }

    #[test]
    fn missing_db_returns_empty() {
        // complete_* uses default path; if no DB, empty is fine (fail soft).
        // Just ensure it does not panic.
        let _ = complete_exercise(&OsString::from("bench"));
        let _ = complete_product(&OsString::from("1"));
        let _ = complete_workout(&OsString::from(""));
        let _ = complete_store(&OsString::from(""));
    }
}
