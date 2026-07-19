//! Cross-domain audit inspection: `recomplog audit recent` (S7d).

use crate::cli::AuditAction;
use crate::db;
use crate::entity_audit;
use crate::utils::{format_instant_utc, print_json};
use anyhow::{anyhow, Result};
use chrono::{Duration, Local, TimeZone, Utc};

/// Dispatch `recomplog audit …`.
pub fn handle(
    action: AuditAction,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        AuditAction::Recent {
            days,
            entity,
            limit,
        } => handle_recent(db_override, days, &entity, limit, json, quiet),
    }
}

fn handle_recent(
    db_override: Option<&str>,
    days: u32,
    entity: &[String],
    limit: i64,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if days == 0 {
        return Err(anyhow!("--days must be >= 1"));
    }
    if limit < 1 {
        return Err(anyhow!("--limit must be >= 1"));
    }

    let entity_filter = entity_audit::parse_entity_filter(entity)?;
    let (since_at, until_at) = storage_window_rfc3339(days)?;

    let conn = db::open_db(db_override)?;
    let entries = entity_audit::list_recent(&conn, &since_at, &until_at, &entity_filter, limit)?;

    let entity_filter_json = if entity_filter.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::json!(entity_filter)
    };

    let resp = serde_json::json!({
        "success": true,
        "days": days,
        "since_at": since_at,
        "until_at": until_at,
        "entity_filter": entity_filter_json,
        "limit": limit,
        "count": entries.len(),
        "entries": entries,
    });

    if json {
        print_json(&resp);
    } else {
        entity_audit::print_recent_human(
            resp["entries"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or(&[]),
            quiet,
        );
    }
    Ok(())
}

/// Inclusive local-calendar window of `days` ending today → RFC3339 Z bounds on storage clock.
///
/// Lower: local midnight of (today − (days−1)), as UTC.
/// Upper: now (UTC) so in-progress day is included without clock skew to end-of-day.
fn storage_window_rfc3339(days: u32) -> Result<(String, String)> {
    let today = Local::now().date_naive();
    let since_date = today - Duration::days(i64::from(days) - 1);
    let since_local = Local
        .from_local_datetime(&since_date.and_hms_opt(0, 0, 0).expect("midnight"))
        .single()
        .ok_or_else(|| anyhow!("could not resolve local midnight for {since_date}"))?;
    let since_at = format_instant_utc(since_local.with_timezone(&Utc));
    let until_at = format_instant_utc(Utc::now());
    Ok((since_at, until_at))
}
