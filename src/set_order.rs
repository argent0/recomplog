//! Append-only session set order (F4).
//!
//! Display order for sets within a workout_exercise is derived from the latest
//! `set_order_revisions` row (when present), not by mutating `exercise_sets.set_number`.
//! Create path inserts sets only; move appends a revision. Soft-delete filters on read.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};

use crate::db;

/// Active set ids in display order for a workout_exercise.
///
/// 1. Latest revision for `workout_exercise_id` by `(at, id)`.
/// 2. Keep only non-deleted existing ids from that list.
/// 3. Append active sets missing from the revision by `(created_at, id)`.
/// 4. No revision → active sets by frozen `(set_number, id)`.
pub fn effective_set_order(conn: &Connection, workout_exercise_id: i64) -> Result<Vec<i64>> {
    let active = active_sets(conn, workout_exercise_id)?;
    let active_ids: HashSet<i64> = active.iter().map(|s| s.id).collect();

    let latest: Option<String> = conn
        .query_row(
            "SELECT order_json FROM set_order_revisions
             WHERE workout_exercise_id = ?1
             ORDER BY at DESC, id DESC
             LIMIT 1",
            [workout_exercise_id],
            |r| r.get(0),
        )
        .optional()?;

    let Some(order_json) = latest else {
        // Legacy / create-only: frozen set_number, then id (active_sets already ordered).
        return Ok(active.into_iter().map(|s| s.id).collect());
    };

    let parsed: Vec<i64> = parse_order_json(&order_json)?;
    let mut seen = HashSet::new();
    let mut order = Vec::new();
    for id in parsed {
        if active_ids.contains(&id) && seen.insert(id) {
            order.push(id);
        }
    }

    // New sets logged after last move: create order at the end.
    let mut extras: Vec<ActiveSet> = active
        .into_iter()
        .filter(|s| !seen.contains(&s.id))
        .collect();
    extras.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    for s in extras {
        order.push(s.id);
    }

    Ok(order)
}

/// 1-based display index for each set id in effective order.
pub fn set_display_numbers(order: &[i64]) -> HashMap<i64, i64> {
    order
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, (i as i64) + 1))
        .collect()
}

/// Insert an order revision; returns revision id.
pub fn insert_revision(
    conn: &Connection,
    workout_exercise_id: i64,
    order: &[i64],
    actor: Option<&str>,
    reason: Option<&str>,
) -> Result<i64> {
    let at = db::now_utc();
    let order_json =
        serde_json::to_string(order).map_err(|e| anyhow!("serialize order_json: {e}"))?;
    conn.execute(
        "INSERT INTO set_order_revisions (workout_exercise_id, at, actor, reason, order_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            workout_exercise_id,
            at,
            actor.unwrap_or("cli"),
            reason,
            order_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Splice `set_id` to 1-based position `to` within `order`. Clamps `to` to `1..=len`.
/// Returns `(new_order, from_pos, to_pos)` where positions are 1-based.
/// Errors if `set_id` is not in the order.
pub fn splice_move(order: &[i64], set_id: i64, to: i32) -> Result<(Vec<i64>, i64, i64)> {
    if order.is_empty() {
        return Err(anyhow!("no active sets to reorder"));
    }
    let from_idx = order
        .iter()
        .position(|&id| id == set_id)
        .ok_or_else(|| anyhow!("set {set_id} is not in the active order for this exercise"))?;
    let from_pos = (from_idx as i64) + 1;

    let mut rebuilt = order.to_vec();
    let item = rebuilt.remove(from_idx);
    // Desired 1-based index in the final list (clamp to valid range after removal).
    let insert_at = (to.max(1) as usize - 1).min(rebuilt.len());
    rebuilt.insert(insert_at, item);
    let to_pos = (insert_at as i64) + 1;
    Ok((rebuilt, from_pos, to_pos))
}

fn parse_order_json(s: &str) -> Result<Vec<i64>> {
    let v: JsonValue = serde_json::from_str(s)
        .map_err(|e| anyhow!("invalid set_order_revisions.order_json: {e}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow!("order_json must be a JSON array of set ids"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let id = item
            .as_i64()
            .ok_or_else(|| anyhow!("order_json entries must be integers"))?;
        out.push(id);
    }
    Ok(out)
}

struct ActiveSet {
    id: i64,
    #[allow(dead_code)]
    set_number: i64,
    created_at: String,
}

fn active_sets(conn: &Connection, workout_exercise_id: i64) -> Result<Vec<ActiveSet>> {
    let mut stmt = conn.prepare(
        "SELECT id, set_number, created_at FROM exercise_sets
         WHERE workout_exercise_id = ?1 AND deleted_at IS NULL
         ORDER BY set_number ASC, id ASC",
    )?;
    let rows = stmt.query_map([workout_exercise_id], |r| {
        Ok(ActiveSet {
            id: r.get(0)?,
            set_number: r.get(1)?,
            created_at: r.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::splice_move;

    #[test]
    fn splice_move_to_front() {
        let (order, from, to) = splice_move(&[10, 20, 30], 30, 1).unwrap();
        assert_eq!(order, vec![30, 10, 20]);
        assert_eq!(from, 3);
        assert_eq!(to, 1);
    }

    #[test]
    fn splice_move_to_end() {
        let (order, from, to) = splice_move(&[10, 20, 30], 10, 3).unwrap();
        assert_eq!(order, vec![20, 30, 10]);
        assert_eq!(from, 1);
        assert_eq!(to, 3);
    }

    #[test]
    fn splice_move_noop() {
        let (order, from, to) = splice_move(&[10, 20, 30], 20, 2).unwrap();
        assert_eq!(order, vec![10, 20, 30]);
        assert_eq!(from, 2);
        assert_eq!(to, 2);
    }

    #[test]
    fn splice_move_clamp_high() {
        let (order, from, to) = splice_move(&[10, 20, 30], 10, 99).unwrap();
        assert_eq!(order, vec![20, 30, 10]);
        assert_eq!(from, 1);
        assert_eq!(to, 3);
    }
}
