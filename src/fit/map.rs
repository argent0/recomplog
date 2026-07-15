use super::parse::FitActivity;
use crate::error::{RecomplogError, Result};
use crate::models::{HeartRateZones, HrZoneProfile, Lap, Trackpoint};

/// Fully mapped import ready for repository writes.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub started_at: String,
    pub workout_type: String,
    pub notes: Option<String>,
    pub duration_minutes: Option<i32>,
    pub distance_km: f64,
    pub duration_seconds: i32,
    pub avg_heart_rate_bpm: Option<f64>,
    pub max_heart_rate_bpm: Option<f64>,
    pub calories_burned: Option<i32>,
    pub avg_pace_min_per_km: Option<f64>,
    pub avg_cadence_spm: Option<f64>,
    pub total_ascent_m: Option<f64>,
    pub total_descent_m: Option<f64>,
    pub heart_rate_zones: Option<HeartRateZones>,
    /// Snapshot from bodylog at import (YYYY-MM-DD); used to derive age at activity time.
    pub date_of_birth: Option<String>,
    /// Snapshot of median sleep HR used for Karvonen (if any).
    pub resting_hr_bpm: Option<f64>,
    pub laps: Option<Vec<Lap>>,
    pub trackpoints: Vec<Trackpoint>,
    pub device_name: Option<String>,
    pub manufacturer_id: Option<i64>,
    pub product_id: Option<i64>,
    pub fit_sport: Option<i64>,
    pub fit_sub_sport: Option<i64>,
    /// Diagnostics only (not stored): how zones were derived.
    #[allow(dead_code)]
    pub hr_zones_source: Option<String>,
}

impl ImportPlan {
    pub fn from_activity(
        activity: &FitActivity,
        workout_type: Option<&str>,
        notes: Option<&str>,
        source_filename: &str,
        hr_zone_bounds: Option<&[f64; 5]>,
        // When set, used only if FIT has no device zones and CLI bounds are absent.
        bodylog_profile: Option<&HrZoneProfile>,
    ) -> Result<Self> {
        ensure_running(activity)?;

        let distance_km = activity.distance_km.ok_or_else(|| {
            RecomplogError::Import("FIT session is missing total_distance".into())
        })?;
        let duration_seconds = activity.duration_seconds.ok_or_else(|| {
            RecomplogError::Import(
                "FIT session is missing total_timer_time / total_elapsed_time".into(),
            )
        })?;
        if distance_km <= 0.0 {
            return Err(RecomplogError::Import(
                "FIT session distance must be greater than 0".into(),
            ));
        }
        if duration_seconds <= 0 {
            return Err(RecomplogError::Import(
                "FIT session duration must be greater than 0".into(),
            ));
        }

        let avg_pace_min_per_km = Some((duration_seconds as f64 / 60.0) / distance_km);

        let mut date_of_birth = None;
        let mut resting_hr_bpm = None;
        let mut hr_zones_source = None;

        let heart_rate_zones = if let Some(zones) = activity.hr_zone_seconds {
            hr_zones_source = Some("FIT time_in_hr_zone".into());
            Some(HeartRateZones {
                z1_seconds: zones[0],
                z2_seconds: zones[1],
                z3_seconds: zones[2],
                z4_seconds: zones[3],
                z5_seconds: zones[4],
            })
        } else if let Some(bounds) = hr_zone_bounds {
            hr_zones_source = Some("CLI --hr-zone-bounds".into());
            Some(compute_hr_zones(&activity.records, bounds))
        } else if let Some(profile) = bodylog_profile {
            date_of_birth = Some(profile.date_of_birth.clone());
            resting_hr_bpm = profile.resting_hr_bpm;
            hr_zones_source = Some(profile.method.clone());
            Some(compute_hr_zones(&activity.records, &profile.bounds))
        } else {
            None
        };

        let laps = map_laps(&activity.laps, distance_km, duration_seconds as u32);

        // Always keep the record stream when present (GPS/HR samples).
        let trackpoints: Vec<Trackpoint> = activity
            .records
            .iter()
            .filter_map(|r| r.to_trackpoint())
            .collect();

        let notes = build_notes(notes, source_filename, activity.device_name.as_deref());

        let duration_minutes = Some(((duration_seconds as f64) / 60.0).round() as i32);

        Ok(ImportPlan {
            started_at: activity.started_at.clone(),
            workout_type: workout_type.unwrap_or("Run").to_string(),
            notes,
            duration_minutes,
            distance_km,
            duration_seconds,
            avg_heart_rate_bpm: activity.avg_heart_rate_bpm,
            max_heart_rate_bpm: activity.max_heart_rate_bpm,
            calories_burned: activity.calories_burned,
            avg_pace_min_per_km,
            avg_cadence_spm: activity.avg_cadence_spm,
            total_ascent_m: activity.total_ascent_m,
            total_descent_m: activity.total_descent_m,
            heart_rate_zones,
            date_of_birth,
            resting_hr_bpm,
            laps,
            trackpoints,
            device_name: activity.device_name.clone(),
            manufacturer_id: activity.manufacturer_id,
            product_id: activity.product_id,
            fit_sport: activity.sport_id,
            fit_sub_sport: activity.sub_sport_id,
            hr_zones_source,
        })
    }
}

fn ensure_running(activity: &FitActivity) -> Result<()> {
    let sport = activity.sport.as_deref().unwrap_or("").to_ascii_lowercase();
    let is_run = sport.contains("run") || activity.sport_id == Some(1);
    if !is_run {
        return Err(RecomplogError::Import(format!(
            "FIT sport is not running (sport={:?}, sport_id={:?}). Only running FIT files are supported.",
            activity.sport, activity.sport_id
        )));
    }
    Ok(())
}

fn map_laps(
    raw: &[super::parse::FitLap],
    total_distance_km: f64,
    total_duration_s: u32,
) -> Option<Vec<Lap>> {
    // Skip single full-activity lap noise
    if raw.len() < 2 {
        return None;
    }
    let mut laps = Vec::new();
    for (i, lap) in raw.iter().enumerate() {
        let distance_km = lap.distance_km.unwrap_or(0.0);
        let duration_seconds = lap.duration_seconds.unwrap_or(0);
        if distance_km <= 0.0 || duration_seconds == 0 {
            continue;
        }
        let pace_min_per_km = (duration_seconds as f64 / 60.0) / distance_km;
        laps.push(Lap {
            lap_number: (i + 1) as u16,
            distance_km,
            duration_seconds,
            pace_min_per_km,
            avg_heart_rate_bpm: lap.avg_heart_rate_bpm,
            max_heart_rate_bpm: lap.max_heart_rate_bpm,
        });
    }
    if laps.len() < 2 {
        return None;
    }
    // Light validation: sum of lap distances roughly matches session
    let sum_dist: f64 = laps.iter().map(|l| l.distance_km).sum();
    if (sum_dist - total_distance_km).abs() > total_distance_km.max(0.5) * 0.15 + 0.5 {
        // Still import laps; they are what the device reported
        let _ = total_duration_s;
    }
    Some(laps)
}

fn build_notes(
    user_notes: Option<&str>,
    source_filename: &str,
    device_name: Option<&str>,
) -> Option<String> {
    let import_tag = match device_name {
        Some(d) if !d.is_empty() => format!("imported from {} ({})", source_filename, d),
        _ => format!("imported from {}", source_filename),
    };
    match user_notes {
        Some(n) if !n.trim().is_empty() => Some(format!("{} | {}", n.trim(), import_tag)),
        _ => Some(import_tag),
    }
}

/// Compute zone times from record samples given upper bounds for zones 1–5 (bpm).
/// Zone i is (prev_bound, bound_i]; zone 1 starts above 0.
pub fn compute_hr_zones(
    records: &[super::parse::FitRecordPoint],
    bounds: &[f64; 5],
) -> HeartRateZones {
    let mut zones = HeartRateZones::default();
    // Assume ~1s sample spacing when consecutive timestamps unavailable
    let mut prev_ts: Option<&str> = None;
    for rec in records {
        let hr = match rec.heart_rate_bpm {
            Some(h) if h > 0.0 => h,
            _ => {
                prev_ts = rec.timestamp.as_deref();
                continue;
            }
        };
        let dt = match (prev_ts, rec.timestamp.as_deref()) {
            (Some(a), Some(b)) => timestamp_delta_seconds(a, b).unwrap_or(1),
            _ => 1,
        };
        prev_ts = rec.timestamp.as_deref();
        let secs = dt.max(0) as u32;
        if hr <= bounds[0] {
            zones.z1_seconds = zones.z1_seconds.saturating_add(secs);
        } else if hr <= bounds[1] {
            zones.z2_seconds = zones.z2_seconds.saturating_add(secs);
        } else if hr <= bounds[2] {
            zones.z3_seconds = zones.z3_seconds.saturating_add(secs);
        } else if hr <= bounds[3] {
            zones.z4_seconds = zones.z4_seconds.saturating_add(secs);
        } else {
            // above z4 bound: z5 if within z5 bound or above
            zones.z5_seconds = zones.z5_seconds.saturating_add(secs);
        }
    }
    let _ = bounds[4]; // z5 upper bound documented for users; we bucket all above z4 into z5
    zones
}

fn timestamp_delta_seconds(a: &str, b: &str) -> Option<i64> {
    use crate::utils::parse_stored_instant;
    let da = parse_stored_instant(a).ok()?;
    let db = parse_stored_instant(b).ok()?;
    Some((db - da).num_seconds())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit::parse::FitRecordPoint;

    #[test]
    fn compute_zones_buckets() {
        let records = vec![
            FitRecordPoint {
                timestamp: Some("2026-07-10 10:00:00".into()),
                heart_rate_bpm: Some(100.0),
                latitude: None,
                longitude: None,
                altitude_m: None,
                cadence_spm: None,
                distance_km: None,
                speed_m_s: None,
            },
            FitRecordPoint {
                timestamp: Some("2026-07-10 10:00:01".into()),
                heart_rate_bpm: Some(150.0),
                latitude: None,
                longitude: None,
                altitude_m: None,
                cadence_spm: None,
                distance_km: None,
                speed_m_s: None,
            },
            FitRecordPoint {
                timestamp: Some("2026-07-10 10:00:02".into()),
                heart_rate_bpm: Some(180.0),
                latitude: None,
                longitude: None,
                altitude_m: None,
                cadence_spm: None,
                distance_km: None,
                speed_m_s: None,
            },
        ];
        let bounds = [120.0, 140.0, 160.0, 175.0, 200.0];
        let z = compute_hr_zones(&records, &bounds);
        // first sample: dt=1 default (no prev) -> z1
        assert_eq!(z.z1_seconds, 1);
        // 150 -> z3, 180 -> z5
        assert_eq!(z.z3_seconds, 1);
        assert_eq!(z.z5_seconds, 1);
    }
}
