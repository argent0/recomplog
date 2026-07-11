//! Shared helpers: tables, durations, flexible dates, JSON output.
pub const DATETIME_FMT: &str = "%Y-%m-%d %H:%M:%S";

use crate::error::RecomplogError;
use anyhow::{anyhow, Result as AnyResult};
use chrono::{DateTime, Datelike, Local, NaiveDate, Weekday};
use comfy_table::Table;
use serde::Serialize;

/// Header underline only — no outer borders, column dividers, or row separators.
const HEADER_ONLY_PRESET: &str = "    ──              ";

/// Normalize a stored datetime string for display (`YYYY-MM-DD HH:MM:SS`).
pub fn format_datetime(s: &str) -> String {
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, DATETIME_FMT) {
        return dt.format(DATETIME_FMT).to_string();
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
    if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
        let local = dt.with_timezone(&Local);
        local.format("%Y-%m-%d %H:%M:%S %Z").to_string()
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
    let utc = ts.to_string();
    let local = if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
        dt.with_timezone(&Local)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    } else {
        ts.to_string()
    };
    TimestampInfo { utc, local }
}

pub fn parse_date_to_ymd(input: &str) -> AnyResult<String> {
    let d = parse_flexible_date(input)?;
    Ok(d.format("%Y-%m-%d").to_string())
}

/// Parse flexible datetime for workout started_at etc.
/// Accepts date-only (assumes local midnight) or "YYYY-MM-DD HH:MM:SS".
pub fn parse_flexible_datetime(s: &str) -> AnyResult<String> {
    let s = s.trim();
    let lower = s.to_lowercase();
    if lower == "now" {
        return Ok(crate::db::now_utc());
    }
    if lower == "today"
        || lower == "yesterday"
        || lower.contains("ago")
        || lower.starts_with("last ")
    {
        let d = parse_flexible_date(s)?;
        return Ok(format!("{} 12:00:00", d.format("%Y-%m-%d")));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(format!("{} 12:00:00", d.format("%Y-%m-%d")));
    }
    // already datetime-like
    if chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").is_ok() {
        return Ok(s.to_string());
    }
    if chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok() {
        return Ok(s.replace('T', " "));
    }
    Err(anyhow!(
        "unrecognized datetime '{}'; use now, today, YYYY-MM-DD, or YYYY-MM-DD HH:MM:SS",
        s
    ))
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
    fn flexible_date_keywords() {
        let d = parse_flexible_date("today").unwrap();
        assert_eq!(d, Local::now().date_naive());
        let y = parse_flexible_date("yesterday").unwrap();
        assert_eq!(y, Local::now().date_naive() - chrono::Duration::days(1));
        assert!(parse_flexible_date("2026-07-05").is_ok());
    }
}
