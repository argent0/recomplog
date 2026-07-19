//! Connection-local write allow for append-only event tables (F3b).
//!
//! Event row `UPDATE`/`DELETE` is denied by SQLite triggers unless the same
//! database has a row in `_recomplog_write_allow`. INSERT paths stay open.
//! Catalog tables are not guarded.
//!
//! The allow table is permanent (not TEMP): SQLite binds trigger table names to
//! `main` at CREATE time, so a TEMP allow table is not visible to triggers.
//! recomplog is single-user/single-connection for mutations; helpers always
//! clear their allow row on exit.
//!
//! See `reports/append/F3-event-tables-not-append-constrained.md`.

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::entity_audit::UpdateClass;

/// Permanent write-allow table (main schema). Empty = deny event UPDATE/DELETE.
pub const WRITE_ALLOW_TABLE: &str = "_recomplog_write_allow";

/// Well-known write-allow ops (must match trigger `IN (...)` lists).
pub mod op {
    pub const SOFT_DELETE: &str = "soft_delete";
    pub const SUPERSEDE: &str = "supersede";
    pub const LIFECYCLE: &str = "lifecycle";
    pub const CORRECT: &str = "correct";
    pub const PURGE: &str = "purge";
    /// Reserved for migration blocks that must UPDATE/DELETE event rows after v13.
    #[allow(dead_code)]
    pub const MIGRATE: &str = "migrate";
}

/// Map in-place update class to a write-allow op.
pub fn op_for_update_class(class: UpdateClass) -> &'static str {
    match class {
        UpdateClass::Lifecycle => op::LIFECYCLE,
        UpdateClass::Correction => op::CORRECT,
    }
}

/// Ensure the write-allow table exists (empty = deny).
pub fn ensure_write_allow_table(conn: &Connection) -> Result<()> {
    conn.execute(
        &format!(
            "CREATE TABLE IF NOT EXISTS {WRITE_ALLOW_TABLE} (
                op TEXT NOT NULL
            )"
        ),
        [],
    )
    .with_context(|| format!("failed to create {WRITE_ALLOW_TABLE}"))?;
    Ok(())
}

/// Run `f` while `op` is allowed for event UPDATE/DELETE triggers.
///
/// Clears this allow row on exit (success or error). Nested calls stack as
/// separate rows; each call removes its own rowid.
pub fn with_write_allow<T, F>(conn: &Connection, op: &str, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T>,
{
    ensure_write_allow_table(conn)?;
    conn.execute(
        &format!("INSERT INTO {WRITE_ALLOW_TABLE} (op) VALUES (?1)"),
        [op],
    )
    .with_context(|| format!("failed to set write allow op={op}"))?;

    let result = f(conn);

    // Pop this allow entry even if `f` failed.
    let _ = conn.execute(
        &format!(
            "DELETE FROM {WRITE_ALLOW_TABLE}
             WHERE rowid = (
                 SELECT MAX(rowid) FROM {WRITE_ALLOW_TABLE} WHERE op = ?1
             )"
        ),
        [op],
    );

    result
}

/// Event / trail tables that receive append-only UPDATE/DELETE guards when present.
pub const GUARDED_EVENT_TABLES: &[&str] = &[
    "workouts",
    "exercise_sets",
    "measurements",
    "sleep",
    "consumptions",
    "purchases",
    "activity_imports",
    "activity_trackpoints",
];

/// Install allow table + triggers for tables that exist on this connection.
///
/// Skips missing tables so partial upgrade fixtures (body-only DBs) still migrate.
pub fn install_append_only_triggers(conn: &Connection) -> Result<()> {
    ensure_write_allow_table(conn)?;

    let update_ops = "'soft_delete','supersede','lifecycle','correct','migrate'";
    let delete_ops = "'purge','migrate'";
    let migrate_only = "'migrate'";
    let purge_or_migrate = "'purge','migrate'";

    if table_exists(conn, "entity_audit")? {
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS ao_entity_audit_no_update;
             CREATE TRIGGER ao_entity_audit_no_update
             BEFORE UPDATE ON entity_audit
             FOR EACH ROW
             WHEN NOT EXISTS (
               SELECT 1 FROM {WRITE_ALLOW_TABLE} WHERE op IN ({migrate_only})
             )
             BEGIN
               SELECT RAISE(ABORT, 'append-only: entity_audit is insert-only');
             END;
             DROP TRIGGER IF EXISTS ao_entity_audit_no_delete;
             CREATE TRIGGER ao_entity_audit_no_delete
             BEFORE DELETE ON entity_audit
             FOR EACH ROW
             WHEN NOT EXISTS (
               SELECT 1 FROM {WRITE_ALLOW_TABLE} WHERE op IN ({migrate_only})
             )
             BEGIN
               SELECT RAISE(ABORT, 'append-only: entity_audit is insert-only');
             END;"
        ))?;
    }

    if table_exists(conn, "set_order_revisions")? {
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS ao_set_order_revisions_no_update;
             CREATE TRIGGER ao_set_order_revisions_no_update
             BEFORE UPDATE ON set_order_revisions
             FOR EACH ROW
             WHEN NOT EXISTS (
               SELECT 1 FROM {WRITE_ALLOW_TABLE} WHERE op IN ({migrate_only})
             )
             BEGIN
               SELECT RAISE(ABORT, 'append-only: set_order_revisions is insert-only');
             END;
             DROP TRIGGER IF EXISTS ao_set_order_revisions_delete_guard;
             CREATE TRIGGER ao_set_order_revisions_delete_guard
             BEFORE DELETE ON set_order_revisions
             FOR EACH ROW
             WHEN NOT EXISTS (
               SELECT 1 FROM {WRITE_ALLOW_TABLE} WHERE op IN ({purge_or_migrate})
             )
             BEGIN
               SELECT RAISE(ABORT, 'append-only: set_order_revisions DELETE requires write allow');
             END;"
        ))?;
    }

    for table in GUARDED_EVENT_TABLES {
        if !table_exists(conn, table)? {
            continue;
        }
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS ao_{table}_update_guard;
             CREATE TRIGGER ao_{table}_update_guard
             BEFORE UPDATE ON {table}
             FOR EACH ROW
             WHEN NOT EXISTS (
               SELECT 1 FROM {WRITE_ALLOW_TABLE} WHERE op IN ({update_ops})
             )
             BEGIN
               SELECT RAISE(ABORT, 'append-only: {table} UPDATE requires write allow');
             END;
             DROP TRIGGER IF EXISTS ao_{table}_delete_guard;
             CREATE TRIGGER ao_{table}_delete_guard
             BEFORE DELETE ON {table}
             FOR EACH ROW
             WHEN NOT EXISTS (
               SELECT 1 FROM {WRITE_ALLOW_TABLE} WHERE op IN ({delete_ops})
             )
             BEGIN
               SELECT RAISE(ABORT, 'append-only: {table} DELETE requires write allow (purge)');
             END;"
        ))?;
    }

    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Triggers expected for tables present on this connection (F3b / `db check append`).
pub fn required_triggers_for_conn(conn: &Connection) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if table_exists(conn, "entity_audit")? {
        out.push("ao_entity_audit_no_update".into());
        out.push("ao_entity_audit_no_delete".into());
    }
    if table_exists(conn, "set_order_revisions")? {
        out.push("ao_set_order_revisions_no_update".into());
        out.push("ao_set_order_revisions_delete_guard".into());
    }
    for table in GUARDED_EVENT_TABLES {
        if table_exists(conn, table)? {
            out.push(format!("ao_{table}_update_guard"));
            out.push(format!("ao_{table}_delete_guard"));
        }
    }
    Ok(out)
}
