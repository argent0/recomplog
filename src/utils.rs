//! Shared helpers: tables, durations, flexible dates, JSON output.
/// Legacy naive wall-clock format (`YYYY-MM-DD HH:MM:SS`) still present in old DBs / fixtures.
pub const DATETIME_FMT: &str = "%Y-%m-%d %H:%M:%S";

use crate::error::RecomplogError;
use anyhow::{anyhow, Result as AnyResult};
use chrono::{
    DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone,
    Timelike, Utc, Weekday,
};
use comfy_table::Table;
use serde::Serialize;

/// Header underline only — no outer borders, column dividers, or row separators.
const HEADER_ONLY_PRESET: &str = "    ──              ";

/// Buenos Aires / Argentina fixed offset (UTC−3, no DST). Used only to interpret
/// **legacy** naive datetimes already stored in the database.
pub fn legacy_local_tz() -> FixedOffset {
    FixedOffset::west_opt(3 * 3600).expect("UTC-3 is a valid fixed offset")
}

/// Canonical DB/API form for instants: `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_instant_utc(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Parse create/update CLI input: RFC3339 only (any offset), as UTC.
pub fn parse_rfc3339_to_utc(s: &str) -> AnyResult<DateTime<Utc>> {
    let s = s.trim();
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            anyhow!(
                "invalid instant '{}': expected RFC3339 (e.g. 2026-07-14T18:30:00-03:00 or …Z): {}",
                s,
                e
            )
        })
}

/// Parse RFC3339 create input and return canonical DB string (`…Z`).
pub fn parse_rfc3339_instant_for_db(s: &str) -> AnyResult<String> {
    Ok(format_instant_utc(parse_rfc3339_to_utc(s)?))
}

/// Accept **only** the canonical stored form `YYYY-MM-DDTHH:MM:SSZ`.
/// Use after `format_instant_utc` / `parse_rfc3339_instant_for_db` on write paths.
pub fn validate_instant_for_db(s: &str) -> AnyResult<String> {
    let s = s.trim();
    let Ok(dt) = DateTime::parse_from_rfc3339(s) else {
        return Err(anyhow!(
            "instant must be canonical UTC RFC3339 (YYYY-MM-DDTHH:MM:SSZ), got '{}'",
            s
        ));
    };
    let utc = dt.with_timezone(&Utc);
    let canonical = format_instant_utc(utc);
    if s != canonical {
        return Err(anyhow!(
            "instant must be canonical UTC RFC3339 (…Z with second precision), got '{}'; use '{}'",
            s,
            canonical
        ));
    }
    Ok(canonical)
}

/// Dual-read helper for values already in the DB (or legacy import): RFC3339, naive
/// wall clock in Buenos Aires, or date-only (legacy nutrition → BA noon).
pub fn parse_stored_instant(s: &str) -> AnyResult<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty instant"));
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    let tz = legacy_local_tz();
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, DATETIME_FMT) {
        return naive_local_to_utc(ndt, tz);
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return naive_local_to_utc(ndt, tz);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let ndt = d
            .and_hms_opt(12, 0, 0)
            .ok_or_else(|| anyhow!("invalid date '{}'", s))?;
        return naive_local_to_utc(ndt, tz);
    }
    Err(anyhow!("unrecognized stored instant '{}'", s))
}

fn naive_local_to_utc(ndt: NaiveDateTime, tz: FixedOffset) -> AnyResult<DateTime<Utc>> {
    tz.from_local_datetime(&ndt)
        .single()
        .map(|dt| dt.with_timezone(&Utc))
        .ok_or_else(|| anyhow!("ambiguous or invalid local datetime {}", ndt))
}

/// Normalize any readable stored/legacy instant to canonical `…Z` for DB write-back.
pub fn normalize_stored_instant_to_db(s: &str) -> AnyResult<String> {
    let canonical = format_instant_utc(parse_stored_instant(s)?);
    validate_instant_for_db(&canonical)
}

/// True when the instant falls on local wall-clock midnight (00:00:00).
pub fn is_local_midnight(dt: DateTime<Utc>) -> bool {
    let local = dt.with_timezone(&Local);
    local.hour() == 0 && local.minute() == 0 && local.second() == 0
}

/// Refuse consumption at local midnight unless `--allow-midnight` (discouraged).
pub fn refuse_consumption_midnight(dt: DateTime<Utc>, allow_midnight: bool) -> AnyResult<()> {
    if allow_midnight || !is_local_midnight(dt) {
        return Ok(());
    }
    Err(anyhow!(
        "refusing consumption at local midnight (often a missing time-of-day). \
         Pass a real local time as RFC3339 (e.g. 2026-07-14T13:45:00-03:00), \
         or --allow-midnight if you really mean midnight (discouraged)."
    ))
}

/// Normalize a stored datetime string for display (`YYYY-MM-DD HH:MM:SS` local).
pub fn format_datetime(s: &str) -> String {
    if let Ok(dt) = parse_stored_instant(s) {
        return dt.with_timezone(&Local).format(DATETIME_FMT).to_string();
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return format!("{} 00:00:00", d.format("%Y-%m-%d"));
    }
    s.to_string()
}

/// Print a minimal human table (header + underline, no box borders).
pub fn print_table(headers: Vec<&str>, rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        return;
    }

    let mut table = Table::new();
    table.load_preset(HEADER_ONLY_PRESET);
    table.set_header(headers);
    for row in rows {
        table.add_row(row);
    }
    for column in table.column_iter_mut() {
        column.set_padding((0, 1));
    }
    println!("{}", table.trim_fmt());
}

pub fn quiet_print(quiet: bool, msg: impl AsRef<str>) {
    if !quiet {
        println!("{}", msg.as_ref());
    }
}

pub fn print_json<T: Serialize>(v: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into())
    );
}

pub fn print_error_json(err: &str) {
    #[derive(Serialize)]
    struct ErrOut {
        success: bool,
        error: String,
    }
    print_json(&ErrOut {
        success: false,
        error: err.to_string(),
    });
}

/// Formats a duration in seconds as `H:MM:SS` or `M:SS`.
pub fn format_duration(seconds: u32) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Formats pace as `M'SS"/km` (repslog-compatible human style).
///
/// Uses floor-minutes + round-seconds (same as repslog). Seconds of 60 carry into
/// minutes so values near a whole minute show as `6'00"/km` rather than `5'60"/km`.
/// Zero is formatted as `0'00"/km` (empty/invalid → `--`).
pub fn format_pace(min_per_km: f64) -> String {
    if !min_per_km.is_finite() || min_per_km < 0.0 {
        return "--".into();
    }
    let mut mins = min_per_km.floor() as u32;
    let mut secs = ((min_per_km - f64::from(mins)) * 60.0).round() as u32;
    if secs >= 60 {
        mins = mins.saturating_add(secs / 60);
        secs %= 60;
    }
    format!("{mins}'{secs:02}\"/km")
}

/// Text bar + zone percentages for HR zone time distribution.
pub fn format_hr_zones_bar(zones: &crate::models::HeartRateZones) -> String {
    let total_secs: u32 = zones.z1_seconds
        + zones.z2_seconds
        + zones.z3_seconds
        + zones.z4_seconds
        + zones.z5_seconds;
    if total_secs == 0 {
        return "No HR data".to_string();
    }

    let width: usize = 20;
    let z1_p = zones.z1_seconds as f64 / total_secs as f64;
    let z2_p = zones.z2_seconds as f64 / total_secs as f64;
    let z3_p = zones.z3_seconds as f64 / total_secs as f64;
    let z4_p = zones.z4_seconds as f64 / total_secs as f64;
    let z5_p = zones.z5_seconds as f64 / total_secs as f64;

    let z1_w = (z1_p * width as f64).round() as usize;
    let z2_w = (z2_p * width as f64).round() as usize;
    let z3_w = (z3_p * width as f64).round() as usize;
    let z4_w = (z4_p * width as f64).round() as usize;
    let z5_w = width.saturating_sub(z1_w + z2_w + z3_w + z4_w);

    let bar = format!(
        "{}{}{}{}{}",
        "█".repeat(z1_w),
        "█".repeat(z2_w),
        "█".repeat(z3_w),
        "█".repeat(z4_w),
        "█".repeat(z5_w)
    );

    format!(
        "{bar} (Z1:{:.0}% Z2:{:.0}% Z3:{:.0}% Z4:{:.0}% Z5:{:.0}%)",
        z1_p * 100.0,
        z2_p * 100.0,
        z3_p * 100.0,
        z4_p * 100.0,
        z5_p * 100.0
    )
}

/// Formats minutes as a compact human string.
pub fn format_minutes(minutes: i64) -> String {
    if minutes < 0 {
        return format!("-{} m", -minutes);
    }
    if minutes == 0 {
        return "0 m".to_string();
    }
    let h = minutes / 60;
    let m = minutes % 60;
    if h > 0 && m > 0 {
        format!("{} h {} m", h, m)
    } else if h > 0 {
        format!("{} h 0 m", h)
    } else {
        format!("{} m", m)
    }
}

/// Parses flexible human duration strings into whole minutes (i64).
pub fn parse_duration_to_minutes(s: &str) -> Result<i64, RecomplogError> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return Err(RecomplogError::InvalidDuration(
            "empty duration string".to_string(),
        ));
    }

    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 || parts.len() == 3 {
            let hours: i64 = parts[0].trim().parse().map_err(|_| {
                RecomplogError::InvalidDuration(format!("invalid hours in '{}'", s))
            })?;
            let minutes: i64 = parts[1].trim().parse().map_err(|_| {
                RecomplogError::InvalidDuration(format!("invalid minutes in '{}'", s))
            })?;
            if hours < 0 || minutes < 0 {
                return Err(RecomplogError::InvalidDuration(
                    "durations cannot be negative".to_string(),
                ));
            }
            return Ok(hours * 60 + minutes);
        }
        return Err(RecomplogError::InvalidDuration(format!(
            "unrecognized colon duration format: '{}'. Use H:M or H:M:S",
            s
        )));
    }

    if let Ok(n) = s.parse::<i64>() {
        if n < 0 {
            return Err(RecomplogError::InvalidDuration(
                "durations cannot be negative".to_string(),
            ));
        }
        return Ok(n);
    }

    let mut normalized = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_ascii_digit() || ch == '.' {
            while i < chars.len() {
                let c = chars[i];
                if c.is_ascii_digit() || c == '.' || c == '-' {
                    normalized.push(c);
                    i += 1;
                } else {
                    break;
                }
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            while i < chars.len() {
                let c = chars[i];
                if c.is_ascii_alphabetic() {
                    normalized.push(c);
                    i += 1;
                } else {
                    break;
                }
            }
            normalized.push(' ');
            continue;
        }
        normalized.push(ch);
        i += 1;
    }

    let mut total_minutes: i64 = 0;
    let cleaned = normalized.replace([',', ';'], " ");
    for token in cleaned.split_whitespace() {
        let t = token.trim();
        if t.is_empty() {
            continue;
        }

        let tchars: Vec<char> = t.chars().collect();
        let mut j = 0;
        while j < tchars.len() {
            let mut num_str = String::new();
            while j < tchars.len() {
                let c = tchars[j];
                if c.is_ascii_digit() || c == '.' || c == '-' {
                    num_str.push(c);
                    j += 1;
                } else {
                    break;
                }
            }
            if num_str.is_empty() {
                return Err(RecomplogError::InvalidDuration(format!(
                    "invalid duration token: '{}'",
                    t
                )));
            }
            let num: f64 = num_str.parse().map_err(|_| {
                RecomplogError::InvalidDuration(format!("invalid number in token '{}'", t))
            })?;
            if num < 0.0 {
                return Err(RecomplogError::InvalidDuration(
                    "durations cannot be negative".to_string(),
                ));
            }

            let mut unit_str = String::new();
            while j < tchars.len() {
                let c = tchars[j];
                if c.is_ascii_alphabetic() {
                    unit_str.push(c);
                    j += 1;
                } else {
                    break;
                }
            }

            let unit = unit_str.trim().to_lowercase();
            let minutes_for_token: i64 = if unit.is_empty() {
                num.round() as i64
            } else if matches!(unit.as_str(), "h" | "hr" | "hrs" | "hour" | "hours") {
                (num * 60.0).round() as i64
            } else if matches!(unit.as_str(), "m" | "min" | "mins" | "minute" | "minutes") {
                num.round() as i64
            } else {
                return Err(RecomplogError::InvalidDuration(format!(
                    "unrecognized unit '{}' in '{}'. Use h, hr, m, min, etc.",
                    unit, t
                )));
            };
            total_minutes += minutes_for_token;
        }
    }

    if total_minutes < 0 {
        return Err(RecomplogError::InvalidDuration(
            "durations cannot be negative".to_string(),
        ));
    }

    Ok(total_minutes)
}

// ---------- Flexible dates ----------

pub fn parse_flexible_date(s: &str) -> AnyResult<NaiveDate> {
    let s = s.trim().to_lowercase();
    let now = Local::now();
    let today = now.date_naive();

    if s == "today" {
        return Ok(today);
    }
    if s == "yesterday" {
        return Ok(today - chrono::Duration::days(1));
    }
    if s == "tomorrow" {
        return Ok(today + chrono::Duration::days(1));
    }

    if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return Ok(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%m-%d-%Y") {
        return Ok(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%d-%m-%Y") {
        return Ok(d);
    }

    if s.ends_with(" days ago") || s.ends_with(" day ago") {
        if let Some(num_str) = s.split_whitespace().next() {
            if let Ok(n) = num_str.parse::<i64>() {
                return Ok(today - chrono::Duration::days(n));
            }
        }
        return Err(anyhow!("unrecognized date: {}", s));
    }

    if s == "last week" {
        return Ok(today - chrono::Duration::days(7));
    }
    if s == "last month" {
        return Ok(today - chrono::Duration::days(30));
    }

    if let Some(wd) = parse_weekday(&s) {
        return Ok(most_recent_weekday(today, wd, false));
    }
    if let Some(rest) = s.strip_prefix("last ") {
        if let Some(wd) = parse_weekday(rest) {
            return Ok(most_recent_weekday(today, wd, true));
        }
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Ok(dt.date_naive());
    }

    Err(anyhow!(
        "unrecognized date format: '{}'. Use today, yesterday, 2026-06-05, last monday, 3 days ago, etc.",
        s
    ))
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    let t = s.trim().to_lowercase();
    match t.as_str() {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" | "tues" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" | "thur" | "thurs" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn most_recent_weekday(from: NaiveDate, target: Weekday, strict_previous: bool) -> NaiveDate {
    let mut d = from;
    if !strict_previous && d.weekday() == target {
        return d;
    }
    for _ in 0..7 {
        d -= chrono::Duration::days(1);
        if d.weekday() == target {
            return d;
        }
    }
    from - chrono::Duration::days(7)
}

pub fn format_local(ts: &str) -> String {
    if let Ok(dt) = parse_stored_instant(ts) {
        dt.with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S %Z")
            .to_string()
    } else {
        ts.to_string()
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct TimestampInfo {
    pub utc: String,
    pub local: String,
}

pub fn make_timestamp_info(ts: &str) -> TimestampInfo {
    if let Ok(dt) = parse_stored_instant(ts) {
        TimestampInfo {
            utc: format_instant_utc(dt),
            local: dt
                .with_timezone(&Local)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        }
    } else {
        TimestampInfo {
            utc: ts.to_string(),
            local: ts.to_string(),
        }
    }
}

pub fn parse_date_to_ymd(input: &str) -> AnyResult<String> {
    let d = parse_flexible_date(input)?;
    validate_date_ymd(&d.format("%Y-%m-%d").to_string())
}

/// Strict calendar-day form for DB columns (`YYYY-MM-DD`).
pub fn validate_date_ymd(s: &str) -> AnyResult<String> {
    let s = s.trim();
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow!("date must be YYYY-MM-DD, got '{}'", s))?;
    Ok(d.format("%Y-%m-%d").to_string())
}

/// Resolve since/until/days into optional YYYY-MM-DD bounds (inclusive).
pub fn resolve_date_range(
    since: Option<&str>,
    until: Option<&str>,
    days: Option<i64>,
) -> AnyResult<(Option<String>, Option<String>)> {
    match (since, until, days) {
        (None, None, None) => Ok((None, None)),
        (s, u, None) => {
            let since_ymd = match s {
                Some(v) => Some(parse_date_to_ymd(v)?),
                None => None,
            };
            let until_ymd = match u {
                Some(v) => Some(parse_date_to_ymd(v)?),
                None => None,
            };
            Ok((since_ymd, until_ymd))
        }
        (None, None, Some(n)) if n > 0 => {
            let today = Local::now().date_naive();
            let since_date = today - chrono::Duration::days(n - 1);
            Ok((
                Some(since_date.format("%Y-%m-%d").to_string()),
                Some(today.format("%Y-%m-%d").to_string()),
            ))
        }
        _ => {
            if let Some(n) = days {
                if n > 0 {
                    let today = Local::now().date_naive();
                    let since_date = today - chrono::Duration::days(n - 1);
                    return Ok((
                        Some(since_date.format("%Y-%m-%d").to_string()),
                        Some(today.format("%Y-%m-%d").to_string()),
                    ));
                }
            }
            let since_ymd = match since {
                Some(v) => Some(parse_date_to_ymd(v)?),
                None => None,
            };
            let until_ymd = match until {
                Some(v) => Some(parse_date_to_ymd(v)?),
                None => None,
            };
            Ok((since_ymd, until_ymd))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_various_formats() {
        assert_eq!(parse_duration_to_minutes("1 h 9 m").unwrap(), 69);
        assert_eq!(parse_duration_to_minutes("1h9m").unwrap(), 69);
        assert_eq!(parse_duration_to_minutes("53 m").unwrap(), 53);
        assert_eq!(parse_duration_to_minutes("5h 25m").unwrap(), 325);
        assert_eq!(parse_duration_to_minutes("325").unwrap(), 325);
        assert_eq!(parse_duration_to_minutes("8:15").unwrap(), 495);
        assert_eq!(parse_duration_to_minutes("1 hr 30 min").unwrap(), 90);
        assert_eq!(parse_duration_to_minutes("2 hours 5 minutes").unwrap(), 125);
        assert_eq!(parse_duration_to_minutes("90min").unwrap(), 90);
        assert_eq!(parse_duration_to_minutes("  2H  30M  ").unwrap(), 150);
        assert_eq!(parse_duration_to_minutes("0").unwrap(), 0);
        assert_eq!(parse_duration_to_minutes("1h").unwrap(), 60);
        assert_eq!(parse_duration_to_minutes("1:00").unwrap(), 60);
    }

    #[test]
    fn parse_edge_and_invalid() {
        assert!(parse_duration_to_minutes("").is_err());
        assert!(parse_duration_to_minutes("-5").is_err());
        assert!(parse_duration_to_minutes("-1 h").is_err());
        assert!(parse_duration_to_minutes("abc").is_err());
        assert!(parse_duration_to_minutes("5x").is_err());
    }

    #[test]
    fn format_minutes_basic() {
        assert_eq!(format_minutes(489), "8 h 9 m");
        assert_eq!(format_minutes(53), "53 m");
        assert_eq!(format_minutes(0), "0 m");
        assert_eq!(format_minutes(60), "1 h 0 m");
    }

    #[test]
    fn format_pace_repslog_style() {
        assert_eq!(format_pace(5.833333), "5'50\"/km");
        assert_eq!(format_pace(0.0), "0'00\"/km");
        // Near whole minute: carry rounded 60s → next minute (not 5'60").
        assert_eq!(format_pace(5.9967), "6'00\"/km");
        assert_eq!(format_pace(f64::NAN), "--");
        assert_eq!(format_pace(-1.0), "--");
    }

    #[test]
    fn flexible_date_keywords() {
        let d = parse_flexible_date("today").unwrap();
        assert_eq!(d, Local::now().date_naive());
        let y = parse_flexible_date("yesterday").unwrap();
        assert_eq!(y, Local::now().date_naive() - chrono::Duration::days(1));
        assert!(parse_flexible_date("2026-07-05").is_ok());
    }

    #[test]
    fn format_instant_utc_canonical_z() {
        let dt = DateTime::parse_from_rfc3339("2026-07-14T18:30:00-03:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(format_instant_utc(dt), "2026-07-14T21:30:00Z");
    }

    #[test]
    fn validate_instant_for_db_accepts_only_canonical() {
        assert_eq!(
            validate_instant_for_db("2026-07-14T21:30:00Z").unwrap(),
            "2026-07-14T21:30:00Z"
        );
        assert!(validate_instant_for_db("2026-07-14 21:30:00").is_err());
        assert!(validate_instant_for_db("2026-07-14").is_err());
        assert!(validate_instant_for_db("2026-07-14T21:30:00+00:00").is_err());
        assert!(validate_instant_for_db("2026-07-14T21:30:00.000Z").is_err());
    }

    #[test]
    fn parse_rfc3339_normalizes_to_z() {
        assert_eq!(
            parse_rfc3339_instant_for_db("2026-07-14T18:30:00-03:00").unwrap(),
            "2026-07-14T21:30:00Z"
        );
    }

    #[test]
    fn parse_stored_instant_ba_legacy_naive() {
        // 18:00 Buenos Aires (UTC-3) → 21:00Z
        let dt = parse_stored_instant("2020-06-14 18:00:00").unwrap();
        assert_eq!(format_instant_utc(dt), "2020-06-14T21:00:00Z");
    }

    #[test]
    fn parse_stored_instant_date_only_ba_noon() {
        // 12:00 BA → 15:00Z
        let dt = parse_stored_instant("2020-06-14").unwrap();
        assert_eq!(format_instant_utc(dt), "2020-06-14T15:00:00Z");
    }

    #[test]
    fn refuse_consumption_midnight_guard() {
        // Construct UTC that is local midnight for the running machine.
        let local_midnight = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
        let utc = local_midnight
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert!(refuse_consumption_midnight(utc, false).is_err());
        assert!(refuse_consumption_midnight(utc, true).is_ok());
        let noon_local = Local::now()
            .date_naive()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert!(refuse_consumption_midnight(noon_local, false).is_ok());
    }
}
