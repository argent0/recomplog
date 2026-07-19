//! Append-only entity audit trail and event soft-delete helpers (S3 / S7).
//!
//! Audit rows never CASCADE-delete with entities. Soft-delete sets `deleted_at`
//! (storage clock) without rewriting event payload fields.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::db;

/// Well-known audit kinds (extensible; see reports/append/S7).
pub mod kind {
    pub const SOFT_DELETE: &str = "soft_delete";
    pub const PURGE: &str = "purge";
    #[allow(dead_code)]
    pub const RESTORE: &str = "restore";
    #[allow(dead_code)]
    pub const CREATE: &str = "create";
}

/// Entity type strings stored in `entity_audit.entity_type`.
pub mod entity {
    pub const WORKOUT: &str = "workout";
    pub const EXERCISE_SET: &str = "exercise_set";
    pub const MEASUREMENT: &str = "measurement";
    pub const SLEEP: &str = "sleep";
    pub const CONSUMPTION: &str = "consumption";
    pub const PURCHASE: &str = "purchase";
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CascadeCounts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workout_exercises: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exercise_sets: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_trackpoints: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_imports: Option<i64>,
}

impl CascadeCounts {
    pub fn total_children(&self) -> i64 {
        self.workout_exercises.unwrap_or(0)
            + self.exercise_sets.unwrap_or(0)
            + self.activity_trackpoints.unwrap_or(0)
            + self.activity_imports.unwrap_or(0)
    }

    pub fn to_json(&self) -> JsonValue {
        serde_json::to_value(self).unwrap_or(JsonValue::Null)
    }
}

/// Append one audit row. Never updates or deletes prior audit history.
#[allow(clippy::too_many_arguments)]
pub fn append(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    kind: &str,
    actor: Option<&str>,
    summary: Option<&str>,
    fields_json: Option<&str>,
    meta_json: Option<&str>,
) -> Result<i64> {
    let at = db::now_utc();
    conn.execute(
        "INSERT INTO entity_audit (entity_type, entity_id, at, kind, actor, summary, fields_json, meta_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            entity_type,
            entity_id,
            at,
            kind,
            actor,
            summary,
            fields_json,
            meta_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Soft-delete a single event row. Errors if missing or already soft-deleted.
///
/// `table` must be a known event table (caller-controlled, not user input).
pub fn soft_delete(
    conn: &Connection,
    table: &str,
    entity_type: &str,
    id: i64,
    reason: Option<&str>,
) -> Result<String> {
    validate_event_table(table)?;
    let existing: Option<(Option<String>,)> = conn
        .query_row(
            &format!("SELECT deleted_at FROM {table} WHERE id = ?1"),
            [id],
            |r| Ok((r.get(0)?,)),
        )
        .optional()?;
    match existing {
        None => return Err(anyhow!("{entity_type} {id} not found")),
        Some((Some(_),)) => {
            return Err(anyhow!(
                "{entity_type} {id} is already soft-deleted (use --purge to hard-remove)"
            ));
        }
        Some((None,)) => {}
    }
    let deleted_at = db::now_utc();
    let n = conn.execute(
        &format!(
            "UPDATE {table} SET deleted_at = ?1, delete_reason = ?2 WHERE id = ?3 AND deleted_at IS NULL"
        ),
        params![deleted_at, reason, id],
    )?;
    if n == 0 {
        return Err(anyhow!(
            "{entity_type} {id} not found or already soft-deleted"
        ));
    }
    let meta = match reason {
        Some(r) if !r.is_empty() => Some(serde_json::json!({ "reason": r }).to_string()),
        _ => None,
    };
    append(
        conn,
        entity_type,
        id,
        kind::SOFT_DELETE,
        Some("cli"),
        Some("soft-deleted"),
        None,
        meta.as_deref(),
    )?;
    Ok(deleted_at)
}

/// Hard-delete (purge) a row. Caller is responsible for force policy and CASCADE awareness.
/// Writes a `purge` audit row **before** DELETE so the trail survives.
pub fn purge(
    conn: &Connection,
    table: &str,
    entity_type: &str,
    id: i64,
    reason: Option<&str>,
    meta_extra: Option<JsonValue>,
) -> Result<()> {
    validate_event_table(table)?;
    let exists: Option<i64> = conn
        .query_row(
            &format!("SELECT id FROM {table} WHERE id = ?1"),
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(anyhow!("{entity_type} {id} not found"));
    }
    let mut meta = serde_json::Map::new();
    if let Some(r) = reason {
        if !r.is_empty() {
            meta.insert("reason".into(), JsonValue::String(r.into()));
        }
    }
    if let Some(JsonValue::Object(extra)) = meta_extra {
        for (k, v) in extra {
            meta.insert(k, v);
        }
    } else if let Some(v) = meta_extra {
        meta.insert("extra".into(), v);
    }
    let meta_s = if meta.is_empty() {
        None
    } else {
        Some(JsonValue::Object(meta).to_string())
    };
    append(
        conn,
        entity_type,
        id,
        kind::PURGE,
        Some("cli"),
        Some("purged"),
        None,
        meta_s.as_deref(),
    )?;
    let n = conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
    if n == 0 {
        return Err(anyhow!("{entity_type} {id} not found during purge"));
    }
    Ok(())
}

fn validate_event_table(table: &str) -> Result<()> {
    match table {
        "workouts" | "exercise_sets" | "measurements" | "sleep" | "consumptions" | "purchases" => {
            Ok(())
        }
        _ => Err(anyhow!("internal error: invalid soft-delete table {table}")),
    }
}

pub fn cascade_counts_workout(conn: &Connection, workout_id: i64) -> Result<CascadeCounts> {
    let workout_exercises: i64 = conn.query_row(
        "SELECT COUNT(*) FROM workout_exercises WHERE workout_id = ?1",
        [workout_id],
        |r| r.get(0),
    )?;
    let exercise_sets: i64 = conn.query_row(
        "SELECT COUNT(*) FROM exercise_sets s
         JOIN workout_exercises we ON we.id = s.workout_exercise_id
         WHERE we.workout_id = ?1",
        [workout_id],
        |r| r.get(0),
    )?;
    let activity_trackpoints: i64 = conn.query_row(
        "SELECT COUNT(*) FROM activity_trackpoints t
         JOIN exercise_sets s ON s.id = t.exercise_set_id
         JOIN workout_exercises we ON we.id = s.workout_exercise_id
         WHERE we.workout_id = ?1",
        [workout_id],
        |r| r.get(0),
    )?;
    let activity_imports: i64 = conn.query_row(
        "SELECT COUNT(*) FROM activity_imports WHERE workout_id = ?1",
        [workout_id],
        |r| r.get(0),
    )?;
    Ok(CascadeCounts {
        workout_exercises: Some(workout_exercises),
        exercise_sets: Some(exercise_sets),
        activity_trackpoints: Some(activity_trackpoints),
        activity_imports: Some(activity_imports),
    })
}

pub fn cascade_counts_set(conn: &Connection, set_id: i64) -> Result<CascadeCounts> {
    let activity_trackpoints: i64 = conn.query_row(
        "SELECT COUNT(*) FROM activity_trackpoints WHERE exercise_set_id = ?1",
        [set_id],
        |r| r.get(0),
    )?;
    Ok(CascadeCounts {
        activity_trackpoints: Some(activity_trackpoints),
        ..Default::default()
    })
}

/// List audit history for an entity (oldest first).
pub fn list_history(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    limit: i64,
) -> Result<Vec<JsonValue>> {
    let lim = limit.max(1);
    let mut stmt = conn.prepare(
        "SELECT id, at, kind, actor, summary, fields_json, meta_json
         FROM entity_audit
         WHERE entity_type = ?1 AND entity_id = ?2
         ORDER BY at ASC, id ASC
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![entity_type, entity_id, lim], |r| {
            let fields_raw: Option<String> = r.get(5)?;
            let meta_raw: Option<String> = r.get(6)?;
            let fields = fields_raw
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(JsonValue::Null);
            let meta = meta_raw
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(JsonValue::Null);
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "at": r.get::<_, String>(1)?,
                "kind": r.get::<_, String>(2)?,
                "actor": r.get::<_, Option<String>>(3)?,
                "summary": r.get::<_, Option<String>>(4)?,
                "fields": fields,
                "meta": meta,
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // seq is 1-based chronological for agents
    let mut out = Vec::with_capacity(rows.len());
    for (i, mut row) in rows.into_iter().enumerate() {
        if let Some(obj) = row.as_object_mut() {
            obj.insert("seq".into(), JsonValue::from((i + 1) as i64));
        }
        out.push(row);
    }
    Ok(out)
}

/// Build a standard audit response envelope.
pub fn audit_response(
    entity: &str,
    id: i64,
    current: Option<JsonValue>,
    history: Vec<JsonValue>,
) -> JsonValue {
    serde_json::json!({
        "success": true,
        "entity": entity,
        "id": id,
        "current": current,
        "history": history,
        "related": [],
    })
}

/// Serialize cascade counts into a human multi-line summary.
pub fn format_cascade_human(counts: &CascadeCounts) -> String {
    let mut lines = Vec::new();
    if let Some(n) = counts.workout_exercises {
        lines.push(format!("  workout_exercises: {n}"));
    }
    if let Some(n) = counts.exercise_sets {
        lines.push(format!("  exercise_sets: {n}"));
    }
    if let Some(n) = counts.activity_trackpoints {
        lines.push(format!("  activity_trackpoints: {n}"));
    }
    if let Some(n) = counts.activity_imports {
        lines.push(format!("  activity_imports: {n}"));
    }
    if lines.is_empty() {
        "  (no cascade children)".into()
    } else {
        lines.join("\n")
    }
}
