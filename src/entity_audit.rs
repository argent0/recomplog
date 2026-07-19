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
    pub const CREATE: &str = "create";
    pub const UPDATE: &str = "update";
}

/// One field change for `kind: update` (`fields_json` entries).
#[derive(Debug, Clone)]
pub struct FieldChange {
    pub name: String,
    pub old: JsonValue,
    pub new: JsonValue,
}

impl FieldChange {
    pub fn new(name: impl Into<String>, old: JsonValue, new: JsonValue) -> Self {
        Self {
            name: name.into(),
            old,
            new,
        }
    }

    /// True when old and new serialize to the same JSON (no meaningful change).
    pub fn is_noop(&self) -> bool {
        self.old == self.new
    }
}

/// Append a `create` audit row (CLI / import / etc.).
pub fn append_create(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    actor: Option<&str>,
) -> Result<i64> {
    append(
        conn,
        entity_type,
        entity_id,
        kind::CREATE,
        Some(actor.unwrap_or("cli")),
        Some("created"),
        None,
        None,
    )
}

/// Append an `update` audit row with field-level old/new (skips no-op pairs).
///
/// Returns `Ok(None)` when every field was a no-op (caller should not call this
/// after a real SQL update that changed nothing meaningful, but it is safe).
pub fn append_update(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    fields: &[FieldChange],
    actor: Option<&str>,
) -> Result<Option<i64>> {
    let changed: Vec<&FieldChange> = fields.iter().filter(|f| !f.is_noop()).collect();
    if changed.is_empty() {
        return Ok(None);
    }
    let fields_json = JsonValue::Array(
        changed
            .iter()
            .map(|f| {
                serde_json::json!({
                    "name": f.name,
                    "old": f.old,
                    "new": f.new,
                })
            })
            .collect(),
    );
    let summary = match changed.len() {
        1 => {
            let f = changed[0];
            Some(format!(
                "{} {}→{}",
                f.name,
                json_compact(&f.old),
                json_compact(&f.new)
            ))
        }
        n => Some(format!("updated {n} fields")),
    };
    let id = append(
        conn,
        entity_type,
        entity_id,
        kind::UPDATE,
        Some(actor.unwrap_or("cli")),
        summary.as_deref(),
        Some(&fields_json.to_string()),
        None,
    )?;
    Ok(Some(id))
}

fn json_compact(v: &JsonValue) -> String {
    match v {
        JsonValue::Null => "null".into(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Entity type strings stored in `entity_audit.entity_type`.
pub mod entity {
    pub const WORKOUT: &str = "workout";
    pub const EXERCISE_SET: &str = "exercise_set";
    pub const EXERCISE: &str = "exercise";
    pub const MEASUREMENT: &str = "measurement";
    pub const SLEEP: &str = "sleep";
    pub const CONSUMPTION: &str = "consumption";
    pub const PURCHASE: &str = "purchase";
    pub const PRODUCT: &str = "product";
    pub const STORE: &str = "store";
    pub const MICRONUTRIENT: &str = "micronutrient";
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

/// If no stored audit rows exist, synthesize a single `create` from `current.created_at`
/// (S7a v0: inspectable history before writers cover every create path).
pub fn enrich_history(current: &Option<JsonValue>, history: Vec<JsonValue>) -> Vec<JsonValue> {
    if !history.is_empty() {
        return history;
    }
    let Some(cur) = current else {
        return history;
    };
    let Some(at) = cur.get("created_at").and_then(|v| v.as_str()) else {
        return history;
    };
    if at.is_empty() {
        return history;
    }
    vec![serde_json::json!({
        "seq": 1,
        "id": null,
        "at": at,
        "kind": kind::CREATE,
        "actor": null,
        "summary": "created (inferred from created_at)",
        "fields": null,
        "meta": { "synthetic": true },
    })]
}

/// Build a standard audit response envelope.
/// Applies synthetic create when history is empty and current has `created_at`.
pub fn audit_response(
    entity: &str,
    id: i64,
    current: Option<JsonValue>,
    history: Vec<JsonValue>,
) -> JsonValue {
    let history = enrich_history(&current, history);
    serde_json::json!({
        "success": true,
        "entity": entity,
        "id": id,
        "current": current,
        "history": history,
        "related": [],
    })
}

/// Human-readable audit dump (shared by command handlers).
pub fn print_audit_human(resp: &JsonValue) {
    let entity = resp["entity"].as_str().unwrap_or("?");
    let id = resp["id"].as_i64().unwrap_or(0);
    println!("{entity} {id} audit");
    if resp["current"].is_null() {
        println!("  current: (purged / missing)");
    } else if let Some(del) = resp["current"]["deleted_at"].as_str() {
        println!("  current: soft-deleted at {del}");
    } else {
        println!("  current: present");
    }
    if let Some(hist) = resp["history"].as_array() {
        if hist.is_empty() {
            println!("  history: (none)");
        }
        for h in hist {
            let seq = h["seq"].as_i64().unwrap_or(0);
            let at = h["at"].as_str().unwrap_or("?");
            let kind = h["kind"].as_str().unwrap_or("?");
            let summary = h["summary"].as_str().unwrap_or("");
            println!("  {seq}. [{at}] {kind} {summary}");
        }
    }
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
