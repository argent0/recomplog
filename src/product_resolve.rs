//! Product merge alias resolution (schema v9 / S2).
//!
//! Merged sources keep their rows with `merged_into_id` + `retired_at`.
//! Event FKs stay on the logged product id; reads that need "the product the
//! user means" walk the chain to the active keeper.

use anyhow::{anyhow, Result};
use rusqlite::{Connection, OptionalExtension};

/// Max hops when walking `merged_into_id` (guards cycles / pathological graphs).
const MAX_MERGE_DEPTH: usize = 32;

/// True when the product exists and is not retired (`retired_at IS NULL`).
pub fn is_active_product(conn: &Connection, id: i64) -> Result<bool> {
    let row: Option<(Option<String>,)> = conn
        .query_row(
            "SELECT retired_at FROM products WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?,)),
        )
        .optional()?;
    Ok(matches!(row, Some((None,))))
}

/// Walk `merged_into_id` until an active product (or a row with no further link).
///
/// Returns the effective product id used for display and nutrition joins.
/// Errors if the id does not exist or a cycle / depth limit is hit.
pub fn resolve_effective_product_id(conn: &Connection, id: i64) -> Result<i64> {
    let mut current = id;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..MAX_MERGE_DEPTH {
        if !seen.insert(current) {
            return Err(anyhow!(
                "product {id} merge chain has a cycle (at {current})"
            ));
        }
        let row: Option<(Option<i64>, Option<String>)> = conn
            .query_row(
                "SELECT merged_into_id, retired_at FROM products WHERE id = ?1",
                [current],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((merged_into, retired_at)) = row else {
            return Err(anyhow!("product {current} not found"));
        };
        match (merged_into, retired_at) {
            (Some(next), _) if next != current => {
                current = next;
            }
            _ => return Ok(current),
        }
    }
    Err(anyhow!(
        "product {id} merge chain exceeds {MAX_MERGE_DEPTH} hops"
    ))
}

/// Require an active (non-retired) product; error names the keeper when merged.
pub fn require_active_product(conn: &Connection, id: i64) -> Result<()> {
    let row: Option<(String, Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT name, merged_into_id, retired_at FROM products WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((name, merged_into, retired_at)) = row else {
        return Err(anyhow!("product {id} not found"));
    };
    if retired_at.is_some() {
        let keeper = merged_into
            .map(|k| format!("; use product {k} (merge keeper)"))
            .unwrap_or_default();
        return Err(anyhow!(
            "product {id} ({name}) is retired (merged away){keeper}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn mem_products() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE products (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                merged_into_id INTEGER REFERENCES products(id),
                retired_at TEXT
            );
            "#,
        )
        .unwrap();
        conn
    }

    fn insert(conn: &Connection, id: i64, name: &str) {
        conn.execute(
            "INSERT INTO products (id, name, created_at, updated_at) VALUES (?1, ?2, 't', 't')",
            params![id, name],
        )
        .unwrap();
    }

    fn retire(conn: &Connection, id: i64, into: i64) {
        conn.execute(
            "UPDATE products SET merged_into_id = ?1, retired_at = '2026-07-18T12:00:00Z'
             WHERE id = ?2",
            params![into, id],
        )
        .unwrap();
    }

    #[test]
    fn active_product_without_merge() {
        let conn = mem_products();
        insert(&conn, 1, "Oats");
        assert!(is_active_product(&conn, 1).unwrap());
        assert!(!is_active_product(&conn, 99).unwrap());
        assert_eq!(resolve_effective_product_id(&conn, 1).unwrap(), 1);
        require_active_product(&conn, 1).unwrap();
    }

    #[test]
    fn single_hop_resolve() {
        let conn = mem_products();
        insert(&conn, 1, "Keeper");
        insert(&conn, 2, "Source");
        retire(&conn, 2, 1);
        assert!(!is_active_product(&conn, 2).unwrap());
        assert!(is_active_product(&conn, 1).unwrap());
        assert_eq!(resolve_effective_product_id(&conn, 2).unwrap(), 1);
        assert_eq!(resolve_effective_product_id(&conn, 1).unwrap(), 1);
        let err = require_active_product(&conn, 2).unwrap_err().to_string();
        assert!(err.contains("retired"), "{err}");
        assert!(err.contains("1"), "{err}");
    }

    #[test]
    fn multi_hop_resolve() {
        let conn = mem_products();
        insert(&conn, 1, "C");
        insert(&conn, 2, "B");
        insert(&conn, 3, "A");
        // A → B → C
        retire(&conn, 3, 2);
        retire(&conn, 2, 1);
        assert_eq!(resolve_effective_product_id(&conn, 3).unwrap(), 1);
        assert_eq!(resolve_effective_product_id(&conn, 2).unwrap(), 1);
    }

    #[test]
    fn cycle_detected() {
        let conn = mem_products();
        insert(&conn, 1, "A");
        insert(&conn, 2, "B");
        // Force a cycle (should never be written by merge, but resolve must fail).
        conn.execute(
            "UPDATE products SET merged_into_id = 2, retired_at = 't' WHERE id = 1",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE products SET merged_into_id = 1, retired_at = 't' WHERE id = 2",
            [],
        )
        .unwrap();
        let err = resolve_effective_product_id(&conn, 1).unwrap_err().to_string();
        assert!(err.contains("cycle"), "{err}");
    }

    #[test]
    fn missing_product() {
        let conn = mem_products();
        let err = resolve_effective_product_id(&conn, 42).unwrap_err().to_string();
        assert!(err.contains("not found"), "{err}");
    }
}
