use crate::error::{RecomplogError, Result};
use crate::models::Trackpoint;
use crate::utils::DATETIME_FMT;
use chrono::{DateTime, Local, NaiveDateTime};
use fitparser::profile::MesgNum;
use fitparser::Value;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Parsed FIT activity focused on running metrics.
#[derive(Debug, Clone)]
pub struct FitActivity {
    pub started_at: String,
    pub sport: Option<String>,
    pub sport_id: Option<i64>,
    pub sub_sport_id: Option<i64>,
    pub distance_km: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub avg_heart_rate_bpm: Option<f64>,
    pub max_heart_rate_bpm: Option<f64>,
    pub calories_burned: Option<i32>,
    pub avg_cadence_spm: Option<f64>,
    pub total_ascent_m: Option<f64>,
    pub total_descent_m: Option<f64>,
    pub device_name: Option<String>,
    pub manufacturer_id: Option<i64>,
    pub product_id: Option<i64>,
    pub laps: Vec<FitLap>,
    pub records: Vec<FitRecordPoint>,
    /// HR zone times from FIT time_in_zone if present (seconds per zone 1-5).
    pub hr_zone_seconds: Option<[u32; 5]>,
}

#[derive(Debug, Clone)]
pub struct FitLap {
    pub distance_km: Option<f64>,
    pub duration_seconds: Option<u32>,
    pub avg_heart_rate_bpm: Option<f64>,
    pub max_heart_rate_bpm: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct FitRecordPoint {
    pub timestamp: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude_m: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    pub cadence_spm: Option<f64>,
    pub distance_km: Option<f64>,
    pub speed_m_s: Option<f64>,
}

impl FitRecordPoint {
    pub fn to_trackpoint(&self) -> Option<Trackpoint> {
        let recorded_at = self.timestamp.clone()?;
        Some(Trackpoint {
            recorded_at,
            latitude: self.latitude,
            longitude: self.longitude,
            altitude_m: self.altitude_m,
            heart_rate_bpm: self.heart_rate_bpm,
            cadence_spm: self.cadence_spm,
            distance_km: self.distance_km,
            speed_m_s: self.speed_m_s,
        })
    }
}

pub fn parse_fit_path(path: &Path) -> Result<FitActivity> {
    let mut file = File::open(path).map_err(|e| {
        RecomplogError::Import(format!(
            "Failed to open FIT file '{}': {}",
            path.display(),
            e
        ))
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| {
        RecomplogError::Import(format!(
            "Failed to read FIT file '{}': {}",
            path.display(),
            e
        ))
    })?;
    parse_fit_bytes(&bytes)
}

pub fn parse_fit_bytes(bytes: &[u8]) -> Result<FitActivity> {
    let records = fitparser::from_bytes(bytes)
        .map_err(|e| RecomplogError::Import(format!("Failed to parse FIT file: {}", e)))?;

    let mut activity = FitActivity {
        started_at: String::new(),
        sport: None,
        sport_id: None,
        sub_sport_id: None,
        distance_km: None,
        duration_seconds: None,
        avg_heart_rate_bpm: None,
        max_heart_rate_bpm: None,
        calories_burned: None,
        avg_cadence_spm: None,
        total_ascent_m: None,
        total_descent_m: None,
        device_name: None,
        manufacturer_id: None,
        product_id: None,
        laps: Vec::new(),
        records: Vec::new(),
        hr_zone_seconds: None,
    };

    let mut local_timestamp: Option<String> = None;
    let mut session_start: Option<String> = None;

    for rec in records {
        match rec.kind() {
            MesgNum::FileId => {
                for f in rec.fields() {
                    match f.name() {
                        "product_name" => {
                            if let Some(s) = value_as_string(f.value()) {
                                activity.device_name = Some(s);
                            }
                        }
                        "manufacturer" => {
                            activity.manufacturer_id = value_as_i64(f.value());
                        }
                        "product" | "garmin_product" if activity.product_id.is_none() => {
                            activity.product_id = value_as_i64(f.value());
                        }
                        _ => {}
                    }
                }
            }
            MesgNum::Session => {
                for f in rec.fields() {
                    match f.name() {
                        "start_time" => {
                            if let Some(ts) = value_as_datetime_string(f.value()) {
                                session_start = Some(ts);
                            }
                        }
                        "sport" => {
                            activity.sport = value_as_string(f.value());
                            activity.sport_id = value_as_i64(f.value());
                        }
                        "sub_sport" => {
                            activity.sub_sport_id = value_as_i64(f.value());
                        }
                        "total_distance" => {
                            // Profile gives meters
                            if let Some(m) = value_as_f64(f.value()) {
                                activity.distance_km = Some(m / 1000.0);
                            }
                        }
                        "total_timer_time" | "total_elapsed_time" => {
                            // Prefer total_timer_time when both appear
                            if f.name() == "total_timer_time" || activity.duration_seconds.is_none()
                            {
                                if let Some(secs) = value_as_f64(f.value()) {
                                    activity.duration_seconds = Some(secs.round() as i32);
                                }
                            }
                        }
                        "avg_heart_rate" => {
                            activity.avg_heart_rate_bpm = value_as_f64(f.value());
                        }
                        "max_heart_rate" => {
                            activity.max_heart_rate_bpm = value_as_f64(f.value());
                        }
                        "total_calories" => {
                            if let Some(c) = value_as_f64(f.value()) {
                                activity.calories_burned = Some(c.round() as i32);
                            }
                        }
                        "avg_running_cadence" | "avg_cadence" => {
                            // Prefer running-specific cadence when both exist
                            if f.name() == "avg_running_cadence"
                                || activity.avg_cadence_spm.is_none()
                            {
                                activity.avg_cadence_spm = value_as_f64(f.value());
                            }
                        }
                        "total_ascent" => {
                            activity.total_ascent_m = value_as_f64(f.value());
                        }
                        "total_descent" => {
                            activity.total_descent_m = value_as_f64(f.value());
                        }
                        _ => {}
                    }
                }
            }
            MesgNum::Activity => {
                for f in rec.fields() {
                    // Prefer activity.timestamp (UTC instant in local TZ) over
                    // local_timestamp — the latter is inconsistently decoded across
                    // devices/parsers. session.start_time is still preferred overall.
                    if f.name() == "timestamp" {
                        if let Some(ts) = value_as_datetime_string(f.value()) {
                            local_timestamp = Some(ts);
                        }
                    }
                }
            }
            MesgNum::Lap => {
                let mut lap = FitLap {
                    distance_km: None,
                    duration_seconds: None,
                    avg_heart_rate_bpm: None,
                    max_heart_rate_bpm: None,
                };
                for f in rec.fields() {
                    match f.name() {
                        "total_distance" => {
                            if let Some(m) = value_as_f64(f.value()) {
                                lap.distance_km = Some(m / 1000.0);
                            }
                        }
                        "total_timer_time" | "total_elapsed_time" => {
                            if lap.duration_seconds.is_none() || f.name() == "total_timer_time" {
                                if let Some(secs) = value_as_f64(f.value()) {
                                    lap.duration_seconds = Some(secs.round() as u32);
                                }
                            }
                        }
                        "avg_heart_rate" => {
                            lap.avg_heart_rate_bpm = value_as_f64(f.value());
                        }
                        "max_heart_rate" => {
                            lap.max_heart_rate_bpm = value_as_f64(f.value());
                        }
                        _ => {}
                    }
                }
                activity.laps.push(lap);
            }
            MesgNum::Record => {
                let mut pt = FitRecordPoint {
                    timestamp: None,
                    latitude: None,
                    longitude: None,
                    altitude_m: None,
                    heart_rate_bpm: None,
                    cadence_spm: None,
                    distance_km: None,
                    speed_m_s: None,
                };
                for f in rec.fields() {
                    match f.name() {
                        "timestamp" => {
                            pt.timestamp = value_as_datetime_string(f.value());
                        }
                        "position_lat" => {
                            // fitparser may already convert semicircles to degrees via units
                            pt.latitude = lat_lon_degrees(f.value(), f.units());
                        }
                        "position_long" => {
                            pt.longitude = lat_lon_degrees(f.value(), f.units());
                        }
                        "altitude" | "enhanced_altitude" => {
                            if pt.altitude_m.is_none() {
                                pt.altitude_m = value_as_f64(f.value());
                            }
                        }
                        "heart_rate" => {
                            pt.heart_rate_bpm = value_as_f64(f.value());
                        }
                        "cadence" => {
                            pt.cadence_spm = value_as_f64(f.value());
                        }
                        "distance" => {
                            if let Some(m) = value_as_f64(f.value()) {
                                // distance field on records is meters
                                pt.distance_km = Some(if f.units().contains("km") {
                                    m
                                } else {
                                    m / 1000.0
                                });
                            }
                        }
                        "speed" | "enhanced_speed" if pt.speed_m_s.is_none() => {
                            pt.speed_m_s = value_as_f64(f.value());
                        }
                        _ => {}
                    }
                }
                activity.records.push(pt);
            }
            MesgNum::TimeInZone => {
                // Optional: time_in_hr_zone as array of seconds
                for f in rec.fields() {
                    if f.name() == "time_in_hr_zone" {
                        if let Some(zones) = value_as_zone_array(f.value()) {
                            activity.hr_zone_seconds = Some(zones);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Prefer session.start_time (true activity start in local TZ). Fall back to
    // activity.timestamp. Both format as naive local wall clock via DATETIME_FMT.
    activity.started_at = session_start.or(local_timestamp).ok_or_else(|| {
        RecomplogError::Import("FIT file has no session.start_time or activity.timestamp".into())
    })?;

    if activity.distance_km.is_none() && activity.duration_seconds.is_none() {
        return Err(RecomplogError::Import(
            "FIT file has no session distance or duration".into(),
        ));
    }

    Ok(activity)
}

fn value_as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Invalid => None,
        Value::Float32(x) => Some(*x as f64),
        Value::Float64(x) => Some(*x),
        Value::UInt8(x) => Some(*x as f64),
        Value::UInt16(x) => Some(*x as f64),
        Value::UInt32(x) => Some(*x as f64),
        Value::SInt8(x) => Some(*x as f64),
        Value::SInt16(x) => Some(*x as f64),
        Value::SInt32(x) => Some(*x as f64),
        Value::SInt64(x) => Some(*x as f64),
        Value::UInt64(x) => Some(*x as f64),
        Value::UInt8z(x) => Some(*x as f64),
        Value::UInt16z(x) => Some(*x as f64),
        Value::UInt32z(x) => Some(*x as f64),
        Value::UInt64z(x) => Some(*x as f64),
        Value::Byte(x) => Some(*x as f64),
        Value::Enum(x) => Some(*x as f64),
        Value::Timestamp(dt) => Some(dt.timestamp() as f64),
        Value::String(s) => s.parse().ok(),
        Value::Array(_) => None,
    }
}

fn value_as_i64(v: &Value) -> Option<i64> {
    value_as_f64(v).map(|x| x as i64)
}

fn value_as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Enum(e) => Some(e.to_string()),
        Value::Invalid => None,
        other => Some(format!("{}", other)),
    }
}

fn value_as_datetime_string(v: &Value) -> Option<String> {
    match v {
        Value::Timestamp(dt) => Some(format_local_dt(dt)),
        Value::String(s) => {
            // Try common formats
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Local).format(DATETIME_FMT).to_string());
            }
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, DATETIME_FMT) {
                return Some(ndt.format(DATETIME_FMT).to_string());
            }
            Some(s.clone())
        }
        _ => {
            // Numeric FIT epoch seconds? Unlikely after profile decode.
            None
        }
    }
}

fn format_local_dt(dt: &DateTime<Local>) -> String {
    dt.format(DATETIME_FMT).to_string()
}

fn lat_lon_degrees(v: &Value, units: &str) -> Option<f64> {
    let raw = value_as_f64(v)?;
    if units.contains("degrees") || units.contains("deg") || raw.abs() <= 180.0 {
        // Already degrees (or already converted)
        if raw.abs() > 180.0 {
            // semicircles
            Some(raw * (180.0 / (i32::MAX as f64 + 1.0)))
        } else {
            Some(raw)
        }
    } else {
        // semicircles
        Some(raw * (180.0 / (i32::MAX as f64 + 1.0)))
    }
}

fn value_as_zone_array(v: &Value) -> Option<[u32; 5]> {
    match v {
        Value::Array(items) => {
            let mut out = [0u32; 5];
            // FIT often includes zone 0 (below zone 1); map first 5 positive zones
            let secs: Vec<u32> = items
                .iter()
                .filter_map(|item| value_as_f64(item).map(|s| s.round() as u32))
                .collect();
            if secs.is_empty() {
                return None;
            }
            // Prefer skipping index 0 if length is 6 (zones 0-5)
            let slice = if secs.len() >= 6 {
                &secs[1..6]
            } else {
                &secs[..secs.len().min(5)]
            };
            for (i, s) in slice.iter().enumerate() {
                out[i] = *s;
            }
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_fit() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Zepp20260710164935.fit")
    }

    #[test]
    fn parse_zepp_sample_run() {
        let path = sample_fit();
        assert!(
            path.exists(),
            "fixture missing: {} (canonical path is tests/fixtures/)",
            path.display()
        );
        let act = parse_fit_path(&path).expect("parse sample");
        assert!(
            act.started_at.starts_with("2026-07-10"),
            "started_at={}",
            act.started_at
        );
        // Session start is 2026-07-10 19:49:35 UTC → 16:49:35 in UTC-3 (filename)
        assert!(
            act.started_at.contains("16:49:35")
                || act.started_at.contains("19:49:35")
                || act.started_at.contains("15:49:35")
                || act.started_at.contains("18:49:35"),
            "unexpected started_at={}",
            act.started_at
        );
        let dist = act.distance_km.expect("distance");
        assert!((dist - 8.027).abs() < 0.02, "distance={}", dist);
        assert_eq!(act.duration_seconds, Some(2808));
        assert_eq!(act.avg_heart_rate_bpm, Some(156.0));
        assert_eq!(act.max_heart_rate_bpm, Some(175.0));
        assert_eq!(act.calories_burned, Some(597));
        assert_eq!(act.avg_cadence_spm, Some(77.0));
        assert!(act.records.len() > 1000);
        let sport = act.sport.as_deref().unwrap_or("").to_lowercase();
        assert!(
            sport.contains("run") || act.sport_id == Some(1),
            "sport={:?} id={:?}",
            act.sport,
            act.sport_id
        );
    }
}
