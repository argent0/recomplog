//! Append-only entity audit trail and event soft-delete helpers (S3 / S7).
//!
//! Audit rows never CASCADE-delete with entities. Soft-delete sets `deleted_at`
//! (storage clock) without rewriting event payload fields.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::append_guard::{self, op as write_op};
use crate::db;

/// Well-known audit kinds (extensible; see reports/append/S7).
pub mod kind {
    pub const SOFT_DELETE: &str = "soft_delete";
    pub const PURGE: &str = "purge";
    #[allow(dead_code)]
    pub const RESTORE: &str = "restore";
    pub const CREATE: &str = "create";
    /// Lifecycle fill (null → value), e.g. first `finished_at` on a workout.
    pub const UPDATE: &str = "update";
    /// Honest correction that overwrites a settled field (requires reason).
    pub const CORRECT: &str = "correct";
    /// Catalog merge (product alias retire onto keeper).
    pub const MERGE: &str = "merge";
    /// Entity created by FIT/legacy import (not CLI create).
    pub const IMPORT: &str = "import";
    /// Catalog mutation: rename, nutrition set, tag change, etc.
    pub const CATALOG: &str = "catalog";
    /// Prior head retired because a new row supersedes it (F1).
    pub const SUPERSEDE: &str = "supersede";
    /// Session set reorder via append-only order revision (F4); does not rewrite set_number.
    pub const MOVE: &str = "move";
}

/// Classification of an in-place event field update (S5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateClass {
    /// Filling null/empty fields only (session completion, late optional metrics).
    Lifecycle,
    /// Overwriting at least one settled value (or always-correction fields).
    Correction,
}

impl UpdateClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lifecycle => "lifecycle",
            Self::Correction => "correction",
        }
    }

    pub fn audit_kind(self) -> &'static str {
        match self {
            Self::Lifecycle => kind::UPDATE,
            Self::Correction => kind::CORRECT,
        }
    }
}

/// One field change for `kind: update` / `correct` (`fields_json` entries).
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

    /// True when this change only fills a missing value (null or empty string).
    pub fn is_fill(&self) -> bool {
        match &self.old {
            JsonValue::Null => true,
            JsonValue::String(s) if s.is_empty() => true,
            _ => false,
        }
    }
}

/// Fields that always count as historical correction (event-time reshape).
const ALWAYS_CORRECTION_FIELDS: &[&str] = &["started_at"];

/// Classify non-noop field changes: lifecycle only when every change is a fill
/// and no always-correction field is present.
pub fn classify_field_changes(fields: &[FieldChange]) -> UpdateClass {
    let changed: Vec<&FieldChange> = fields.iter().filter(|f| !f.is_noop()).collect();
    if changed.is_empty() {
        return UpdateClass::Lifecycle;
    }
    for f in &changed {
        if ALWAYS_CORRECTION_FIELDS.contains(&f.name.as_str()) {
            return UpdateClass::Correction;
        }
        if !f.is_fill() {
            return UpdateClass::Correction;
        }
    }
    UpdateClass::Lifecycle
}

/// Require non-empty `--reason` for corrections. Lifecycle may omit it.
pub fn require_reason_for_class(
    class: UpdateClass,
    reason: Option<&str>,
) -> Result<Option<String>> {
    let trimmed = reason.map(str::trim).filter(|s| !s.is_empty());
    match class {
        UpdateClass::Lifecycle => Ok(trimmed.map(|s| s.to_string())),
        UpdateClass::Correction => match trimmed {
            Some(s) => Ok(Some(s.to_string())),
            None => Err(anyhow!(
                "this update overwrites settled field(s) and requires --reason \
                 (lifecycle fills of null fields do not). For large mistakes prefer \
                 soft-delete + create, then inspect with … audit <id>"
            )),
        },
    }
}

/// Append a `create` audit row (CLI / etc.).
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

/// Append an `import` audit row for an entity created by FIT/legacy import.
pub fn append_import(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    summary: &str,
    meta: Option<&JsonValue>,
) -> Result<i64> {
    let meta_s = meta.map(|m| m.to_string());
    append(
        conn,
        entity_type,
        entity_id,
        kind::IMPORT,
        Some("import"),
        Some(summary),
        None,
        meta_s.as_deref(),
    )
}

/// Append a `move` audit row for set reordering (F4). Meta includes revision id and id lists.
pub fn append_set_move(
    conn: &Connection,
    set_id: i64,
    summary: &str,
    meta: &JsonValue,
) -> Result<i64> {
    append(
        conn,
        entity::EXERCISE_SET,
        set_id,
        kind::MOVE,
        Some("cli"),
        Some(summary),
        None,
        Some(&meta.to_string()),
    )
}

/// Append a `merge` audit row (product alias merge, etc.).
pub fn append_merge(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    summary: &str,
    meta: Option<&JsonValue>,
) -> Result<i64> {
    let meta_s = meta.map(|m| m.to_string());
    append(
        conn,
        entity_type,
        entity_id,
        kind::MERGE,
        Some("cli"),
        Some(summary),
        None,
        meta_s.as_deref(),
    )
}

/// Append a `catalog` audit row (rename, nutrition set, tag change, …).
///
/// When `fields` is non-empty, stores field-level old/new like `update`.
pub fn append_catalog(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    summary: &str,
    fields: Option<&[FieldChange]>,
    meta: Option<&JsonValue>,
) -> Result<i64> {
    let fields_json = fields.and_then(|fs| {
        let changed: Vec<&FieldChange> = fs.iter().filter(|f| !f.is_noop()).collect();
        if changed.is_empty() {
            None
        } else {
            Some(
                JsonValue::Array(
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
                )
                .to_string(),
            )
        }
    });
    let meta_s = meta.map(|m| m.to_string());
    append(
        conn,
        entity_type,
        entity_id,
        kind::CATALOG,
        Some("cli"),
        Some(summary),
        fields_json.as_deref(),
        meta_s.as_deref(),
    )
}

/// Append a field-change audit row classified as lifecycle (`update`) or
/// correction (`correct`). Skips no-op pairs. Stores reason in `meta_json` when set.
///
/// Returns `Ok(None)` when every field was a no-op.
pub fn append_field_change(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    fields: &[FieldChange],
    class: UpdateClass,
    reason: Option<&str>,
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
    let meta = reason
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|r| serde_json::json!({ "reason": r }).to_string());
    let id = append(
        conn,
        entity_type,
        entity_id,
        class.audit_kind(),
        Some(actor.unwrap_or("cli")),
        summary.as_deref(),
        Some(&fields_json.to_string()),
        meta.as_deref(),
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

    /// All known entity_type values for `audit recent --entity` validation.
    pub const ALL: &[&str] = &[
        WORKOUT,
        EXERCISE_SET,
        EXERCISE,
        MEASUREMENT,
        SLEEP,
        CONSUMPTION,
        PURCHASE,
        PRODUCT,
        STORE,
        MICRONUTRIENT,
    ];
}

/// Map audit `entity_type` to the SQLite event table that holds `supersedes_id`.
/// Catalog entities return `None`.
pub fn event_table_for_entity(entity_type: &str) -> Option<&'static str> {
    match entity_type {
        entity::WORKOUT => Some("workouts"),
        entity::EXERCISE_SET => Some("exercise_sets"),
        entity::MEASUREMENT => Some("measurements"),
        entity::SLEEP => Some("sleep"),
        entity::CONSUMPTION => Some("consumptions"),
        entity::PURCHASE => Some("purchases"),
        _ => None,
    }
}

/// Validate `--entity` filter tokens; returns normalized type list or an error.
pub fn parse_entity_filter(raw: &[String]) -> Result<Vec<String>> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(raw.len());
    for token in raw {
        let t = token.trim();
        if t.is_empty() {
            continue;
        }
        // Accept common CLI aliases.
        let normalized = match t {
            "set" => entity::EXERCISE_SET,
            other => other,
        };
        if !entity::ALL.contains(&normalized) {
            return Err(anyhow!(
                "unknown entity type '{t}'; expected one of: {}",
                entity::ALL.join(", ")
            ));
        }
        if !out.iter().any(|e| e == normalized) {
            out.push(normalized.to_string());
        }
    }
    Ok(out)
}

/// Live successor of a superseded head (`WHERE supersedes_id = id AND deleted_at IS NULL`).
pub fn lookup_superseded_by(conn: &Connection, table: &str, entity_id: i64) -> Result<Option<i64>> {
    validate_event_table(table)?;
    conn.query_row(
        &format!(
            "SELECT id FROM {table} WHERE supersedes_id = ?1 AND deleted_at IS NULL \
             ORDER BY id ASC LIMIT 1"
        ),
        [entity_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Attach `superseded_by` onto an audit `current` object when a live successor exists.
pub fn attach_superseded_by(
    conn: &Connection,
    entity_type: &str,
    current: &mut JsonValue,
) -> Result<()> {
    let Some(table) = event_table_for_entity(entity_type) else {
        return Ok(());
    };
    let Some(obj) = current.as_object_mut() else {
        return Ok(());
    };
    let Some(id) = obj.get("id").and_then(|v| v.as_i64()) else {
        return Ok(());
    };
    // Only meaningful when this row was retired (soft-deleted) by supersede.
    let deleted = obj
        .get("deleted_at")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !deleted {
        obj.insert("superseded_by".into(), JsonValue::Null);
        return Ok(());
    }
    let by = lookup_superseded_by(conn, table, id)?;
    obj.insert(
        "superseded_by".into(),
        by.map(JsonValue::from).unwrap_or(JsonValue::Null),
    );
    Ok(())
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

/// Soft-delete `old_id` as superseded by `new_id` and write a single
/// `kind: supersede` audit row (not a bare soft_delete).
///
/// Does not insert the new row — caller inserts first, then calls this.
/// Optional field diffs describe old → new for agents.
pub fn supersede_retire(
    conn: &Connection,
    table: &str,
    entity_type: &str,
    old_id: i64,
    new_id: i64,
    reason: &str,
    fields: Option<&[FieldChange]>,
) -> Result<String> {
    validate_event_table(table)?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(anyhow!("supersede requires a non-empty --reason"));
    }
    let existing: Option<(Option<String>,)> = conn
        .query_row(
            &format!("SELECT deleted_at FROM {table} WHERE id = ?1"),
            [old_id],
            |r| Ok((r.get(0)?,)),
        )
        .optional()?;
    match existing {
        None => return Err(anyhow!("{entity_type} {old_id} not found")),
        Some((Some(_),)) => {
            return Err(anyhow!(
                "{entity_type} {old_id} is already soft-deleted (cannot supersede)"
            ));
        }
        Some((None,)) => {}
    }
    // Refuse if another live row already supersedes this head.
    let other: Option<i64> = conn
        .query_row(
            &format!(
                "SELECT id FROM {table} WHERE supersedes_id = ?1 AND deleted_at IS NULL LIMIT 1"
            ),
            [old_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(oid) = other {
        if oid != new_id {
            return Err(anyhow!(
                "{entity_type} {old_id} is already superseded by live {entity_type} {oid}"
            ));
        }
    }
    let deleted_at = db::now_utc();
    let n = append_guard::with_write_allow(conn, write_op::SUPERSEDE, |conn| {
        let n = conn.execute(
            &format!(
                "UPDATE {table} SET deleted_at = ?1, delete_reason = ?2 \
                 WHERE id = ?3 AND deleted_at IS NULL"
            ),
            params![deleted_at, reason, old_id],
        )?;
        Ok(n)
    })?;
    if n == 0 {
        return Err(anyhow!(
            "{entity_type} {old_id} not found or already soft-deleted"
        ));
    }
    let fields_json = fields.and_then(|fs| {
        let changed: Vec<&FieldChange> = fs.iter().filter(|f| !f.is_noop()).collect();
        if changed.is_empty() {
            None
        } else {
            Some(
                JsonValue::Array(
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
                )
                .to_string(),
            )
        }
    });
    let meta = serde_json::json!({
        "reason": reason,
        "superseded_by": new_id,
    })
    .to_string();
    append(
        conn,
        entity_type,
        old_id,
        kind::SUPERSEDE,
        Some("cli"),
        Some(&format!("superseded by {new_id}")),
        fields_json.as_deref(),
        Some(&meta),
    )?;
    Ok(deleted_at)
}

/// Append create audit for a row that supersedes another (meta.supersedes).
pub fn append_supersede_create(
    conn: &Connection,
    entity_type: &str,
    new_id: i64,
    supersedes_id: i64,
    reason: &str,
    fields: Option<&[FieldChange]>,
) -> Result<i64> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(anyhow!("supersede requires a non-empty --reason"));
    }
    let fields_json = fields.and_then(|fs| {
        let changed: Vec<&FieldChange> = fs.iter().filter(|f| !f.is_noop()).collect();
        if changed.is_empty() {
            None
        } else {
            Some(
                JsonValue::Array(
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
                )
                .to_string(),
            )
        }
    });
    let meta = serde_json::json!({
        "reason": reason,
        "supersedes": supersedes_id,
    })
    .to_string();
    append(
        conn,
        entity_type,
        new_id,
        kind::CREATE,
        Some("cli"),
        Some(&format!("created (supersedes {supersedes_id})")),
        fields_json.as_deref(),
        Some(&meta),
    )
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
    let n = append_guard::with_write_allow(conn, write_op::SOFT_DELETE, |conn| {
        let n = conn.execute(
            &format!(
                "UPDATE {table} SET deleted_at = ?1, delete_reason = ?2 WHERE id = ?3 AND deleted_at IS NULL"
            ),
            params![deleted_at, reason, id],
        )?;
        Ok(n)
    })?;
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
    // CASCADE child DELETEs (sets, trackpoints, imports, set_order_revisions) need
    // the same connection-local purge allow.
    let n = append_guard::with_write_allow(conn, write_op::PURGE, |conn| {
        let n = conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
        Ok(n)
    })?;
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

/// Recent audit rows across entities (newest first). Filters by storage time `at`.
///
/// `entity_types` empty = all types. `since_at` / `until_at` are inclusive RFC3339 Z
/// bounds compared as text (canonical `…Z` sorts lexicographically with time).
pub fn list_recent(
    conn: &Connection,
    since_at: &str,
    until_at: &str,
    entity_types: &[String],
    limit: i64,
) -> Result<Vec<JsonValue>> {
    let lim = limit.max(1);

    // Bound parameters first; optional IN-list follows; LIMIT last.
    let mut sql = String::from(
        "SELECT id, entity_type, entity_id, at, kind, actor, summary, fields_json, meta_json
         FROM entity_audit
         WHERE at >= ?1 AND at <= ?2",
    );
    if !entity_types.is_empty() {
        sql.push_str(" AND entity_type IN (");
        for i in 0..entity_types.len() {
            if i > 0 {
                sql.push_str(", ");
            }
            // 1-based: ?3, ?4, … after since/until
            sql.push_str(&format!("?{}", i + 3));
        }
        sql.push(')');
    }
    let limit_ph = if entity_types.is_empty() {
        3
    } else {
        entity_types.len() + 3
    };
    sql.push_str(&format!(" ORDER BY at DESC, id DESC LIMIT ?{limit_ph}"));

    let mut stmt = conn.prepare(&sql)?;

    // Collect owned params then borrow for query_map.
    let mut owned: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(entity_types.len() + 3);
    owned.push(Box::new(since_at.to_string()));
    owned.push(Box::new(until_at.to_string()));
    for t in entity_types {
        owned.push(Box::new(t.clone()));
    }
    owned.push(Box::new(lim));
    let param_refs: Vec<&dyn rusqlite::ToSql> = owned.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |r| {
            let fields_raw: Option<String> = r.get(7)?;
            let meta_raw: Option<String> = r.get(8)?;
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
                "entity_type": r.get::<_, String>(1)?,
                "entity_id": r.get::<_, i64>(2)?,
                "at": r.get::<_, String>(3)?,
                "kind": r.get::<_, String>(4)?,
                "actor": r.get::<_, Option<String>>(5)?,
                "summary": r.get::<_, Option<String>>(6)?,
                "fields": fields,
                "meta": meta,
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Enrich optional audit `current` with live `superseded_by` (event entities only).
pub fn enrich_current_supersede(
    conn: &Connection,
    entity_type: &str,
    mut current: Option<JsonValue>,
) -> Result<Option<JsonValue>> {
    if let Some(ref mut c) = current {
        attach_superseded_by(conn, entity_type, c)?;
    }
    Ok(current)
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
    } else {
        let cur = &resp["current"];
        if let Some(del) = cur["deleted_at"].as_str() {
            print!("  current: soft-deleted at {del}");
            if let Some(reason) = cur["delete_reason"].as_str() {
                if !reason.is_empty() {
                    print!(" ({reason})");
                }
            }
            println!();
        } else {
            println!("  current: present");
        }
        if let Some(sid) = cur["supersedes_id"].as_i64() {
            println!("  supersedes_id: {sid}");
        }
        if let Some(by) = cur["superseded_by"].as_i64() {
            println!("  superseded_by: {by}");
        }
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

/// One-line human dump for `audit recent` entries.
pub fn print_recent_human(entries: &[JsonValue], quiet: bool) {
    if !quiet {
        if entries.is_empty() {
            println!("audit recent: (none)");
            return;
        }
        println!("audit recent ({} entries, newest first)", entries.len());
    }
    for e in entries {
        let at = e["at"].as_str().unwrap_or("?");
        let et = e["entity_type"].as_str().unwrap_or("?");
        let eid = e["entity_id"].as_i64().unwrap_or(0);
        let kind = e["kind"].as_str().unwrap_or("?");
        let summary = e["summary"].as_str().unwrap_or("");
        if summary.is_empty() {
            println!("[{at}] {et} {eid}  {kind}");
        } else {
            println!("[{at}] {et} {eid}  {kind}  {summary}");
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
