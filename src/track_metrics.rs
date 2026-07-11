//! Derive runner metrics from activity trackpoints (compute-on-read for `workout show`).
//!
//! ## Constants / thresholds
//! - Stop: speed &lt; 0.5 m/s (~20 min/km)
//! - Pause gap: Δt &gt; 45 s counts as stopped
//! - Pace sample clip for stats: 2.0–20.0 min/km
//! - Elevation noise floor: |Δalt| ≥ 1.5 m
//! - GPS glitch jump: &gt; 200 m between samples ignored for path length
//! - Loop: start–end gap &lt; max(80 m, 2% of path length)
//!
//! ## Grade-adjusted pace (GAP)
//! Per-segment factor (Strava-like, positive grades):
//! `1 + 0.03·g + 1.5e-4·g²` where `g` is grade percent. Adjusted speed = raw / factor;
//! distance-weighted average yields GAP in min/km.
//!
//! Cadence is reported in **device units as stored** (FIT often uses steps/min per foot).

use crate::hr_zones;
use crate::models::{HeartRateZones, Trackpoint};
use crate::utils::DATETIME_FMT;
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

const STOP_SPEED_M_S: f64 = 0.5;
const PAUSE_GAP_S: i64 = 45;
const PACE_MIN_CLIP: f64 = 2.0;
const PACE_MAX_CLIP: f64 = 20.0;
const ELEV_NOISE_M: f64 = 1.5;
const GPS_GLITCH_M: f64 = 200.0;
const LOOP_MIN_GAP_M: f64 = 80.0;
const PARTIAL_KM_MIN: f64 = 0.2;
const DRIFT_MIN_HALF_S: u32 = 30;
const VAM_MIN_ASCENT_M: f64 = 10.0;
const EARTH_RADIUS_M: f64 = 6_371_000.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Distribution {
    pub min: f64,
    pub median: f64,
    pub max: f64,
    pub mean: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KmSplit {
    /// 1-based full km index, or last partial (same index as next would-be km).
    pub km_index: u16,
    /// True when this split is a partial final km (&lt; 1.0 km).
    pub partial: bool,
    pub distance_km: f64,
    pub duration_seconds: u32,
    pub pace_min_per_km: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_hr: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BestEffort {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pace_min_per_km: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteKind {
    Loop,
    PointToPoint,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteSummary {
    pub kind: RouteKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gps_distance_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crow_flies_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_end_gap_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat_span_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lon_span_deg: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackMetrics {
    pub sample_count: usize,
    pub elapsed_seconds: u32,
    pub moving_seconds: u32,
    pub stopped_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moving_pace_min_per_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pace: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pace_cv: Option<f64>,
    pub synthetic_km_splits: Vec<KmSplit>,
    pub best_efforts: Vec<BestEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hr_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hr_drift_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hr_zones_recomputed: Option<HeartRateZones>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cadence: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cadence_cv: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_stride_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elev_min_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elev_max_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elev_net_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ascent_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descent_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_adj_pace_min_per_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vam_m_per_hour: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteSummary>,
}

/// Optional context for zone recompute (from set snapshots + workout date).
#[derive(Debug, Clone, Default)]
pub struct ZoneRecomputeContext {
    pub date_of_birth: Option<String>,
    pub resting_hr_bpm: Option<f64>,
    /// Activity calendar date `YYYY-MM-DD` (from workout `started_at`).
    pub activity_date: Option<String>,
}

/// Fallback distance (km) when track has no cumulative distance series.
pub fn compute(points: &[Trackpoint], fallback_distance_km: Option<f64>) -> Option<TrackMetrics> {
    compute_with_zones(
        points,
        fallback_distance_km,
        &ZoneRecomputeContext::default(),
    )
}

pub fn compute_with_zones(
    points: &[Trackpoint],
    fallback_distance_km: Option<f64>,
    zone_ctx: &ZoneRecomputeContext,
) -> Option<TrackMetrics> {
    if points.is_empty() {
        return None;
    }

    let times: Vec<Option<i64>> = points
        .iter()
        .map(|p| parse_ts_unix(&p.recorded_at))
        .collect();

    let first_t = times.iter().flatten().next().copied();
    let last_t = times.iter().rev().flatten().next().copied();
    let elapsed_seconds = match (first_t, last_t) {
        (Some(a), Some(b)) if b >= a => (b - a) as u32,
        _ => 0,
    };

    // Per-point resolved speed (m/s), using device speed or Δdistance/Δt.
    let mut speeds: Vec<Option<f64>> = vec![None; points.len()];
    for i in 0..points.len() {
        if let Some(s) = points[i].speed_m_s.filter(|s| *s > 0.0 && s.is_finite()) {
            speeds[i] = Some(s);
            continue;
        }
        if i == 0 {
            continue;
        }
        let dt = delta_t(times[i - 1], times[i]);
        let d0 = points[i - 1].distance_km;
        let d1 = points[i].distance_km;
        if let (Some(dt), Some(a), Some(b)) = (dt, d0, d1) {
            if dt > 0 && b >= a {
                let m_s = (b - a) * 1000.0 / dt as f64;
                if m_s.is_finite() && m_s > 0.0 {
                    speeds[i] = Some(m_s);
                }
            }
        }
    }

    let mut moving_seconds: u32 = 0;
    let mut stopped_seconds: u32 = 0;
    let mut pace_samples: Vec<f64> = Vec::new();
    let mut cadence_samples: Vec<f64> = Vec::new();
    let mut stride_samples: Vec<f64> = Vec::new();
    let mut hr_values: Vec<f64> = Vec::new();

    // For drift: accumulate speed*dt and HR*dt in each half (moving only).
    let mid_t = first_t.zip(last_t).map(|(a, b)| a + (b - a) / 2);

    let mut half1_speed_dt = 0.0;
    let mut half1_hr_dt = 0.0;
    let mut half1_dt: u32 = 0;
    let mut half2_speed_dt = 0.0;
    let mut half2_hr_dt = 0.0;
    let mut half2_dt: u32 = 0;

    // Elevation
    let mut elev_min: Option<f64> = None;
    let mut elev_max: Option<f64> = None;
    let mut first_elev: Option<f64> = None;
    let mut last_elev: Option<f64> = None;
    let mut ascent = 0.0;
    let mut descent = 0.0;
    let mut prev_elev_for_climb: Option<f64> = None;

    // GAP: distance-weighted adjusted speed
    let mut gap_speed_dist = 0.0; // sum(adj_speed * dist_m)
    let mut gap_dist_m = 0.0;

    // Cumulative series for splits / best efforts
    let mut cum_dist_m: Vec<f64> = Vec::with_capacity(points.len());
    let mut cum_time_s: Vec<i64> = Vec::with_capacity(points.len());
    let mut cum_hr_sum: Vec<f64> = Vec::with_capacity(points.len()); // for avg HR in windows
    let mut cum_hr_n: Vec<u32> = Vec::with_capacity(points.len());

    let mut running_dist_m = 0.0;
    let mut running_hr_sum = 0.0;
    let mut running_hr_n = 0u32;
    let t0 = first_t.unwrap_or(0);

    for (i, p) in points.iter().enumerate() {
        if let Some(alt) = p.altitude_m.filter(|a| a.is_finite()) {
            elev_min = Some(elev_min.map_or(alt, |m| m.min(alt)));
            elev_max = Some(elev_max.map_or(alt, |m| m.max(alt)));
            if first_elev.is_none() {
                first_elev = Some(alt);
            }
            last_elev = Some(alt);

            if let Some(prev) = prev_elev_for_climb {
                let d = alt - prev;
                if d.abs() >= ELEV_NOISE_M {
                    if d > 0.0 {
                        ascent += d;
                    } else {
                        descent += -d;
                    }
                    prev_elev_for_climb = Some(alt);
                }
            } else {
                prev_elev_for_climb = Some(alt);
            }
        }

        if let Some(hr) = p.heart_rate_bpm.filter(|h| *h > 0.0 && h.is_finite()) {
            hr_values.push(hr);
            running_hr_sum += hr;
            running_hr_n += 1;
        }

        // Distance series: prefer device cumulative; else integrate speed.
        if let Some(d) = p.distance_km.filter(|d| d.is_finite() && *d >= 0.0) {
            running_dist_m = d * 1000.0;
        } else if i > 0 {
            if let (Some(dt), Some(sp)) = (delta_t(times[i - 1], times[i]), speeds[i]) {
                if dt > 0 && dt <= PAUSE_GAP_S && sp >= STOP_SPEED_M_S {
                    running_dist_m += sp * dt as f64;
                }
            }
        }

        let t_rel = times[i].map(|t| t - t0).unwrap_or(0).max(0);
        cum_dist_m.push(running_dist_m);
        cum_time_s.push(t_rel);
        cum_hr_sum.push(running_hr_sum);
        cum_hr_n.push(running_hr_n);

        if i == 0 {
            continue;
        }

        let dt_opt = delta_t(times[i - 1], times[i]);
        let Some(dt) = dt_opt else { continue };
        if dt <= 0 {
            continue;
        }
        let dt_u = dt as u32;

        if dt > PAUSE_GAP_S {
            stopped_seconds = stopped_seconds.saturating_add(dt_u);
            continue;
        }

        let sp = speeds[i].or(speeds[i - 1]);
        let moving = sp.map(|s| s >= STOP_SPEED_M_S).unwrap_or(false);

        if moving {
            moving_seconds = moving_seconds.saturating_add(dt_u);
            if let Some(s) = sp {
                let pace = speed_to_pace(s);
                if (PACE_MIN_CLIP..=PACE_MAX_CLIP).contains(&pace) {
                    pace_samples.push(pace);
                }

                // Cadence / stride on moving samples at current point
                if let Some(c) = points[i].cadence_spm.filter(|c| *c > 0.0 && c.is_finite()) {
                    cadence_samples.push(c);
                    let steps_per_s = c / 60.0;
                    if steps_per_s > 0.0 {
                        stride_samples.push(s / steps_per_s);
                    }
                }

                // GAP for this segment
                let horiz_m = s * dt as f64;
                if horiz_m > 0.0 {
                    let mut grade_pct = 0.0;
                    if let (Some(a0), Some(a1)) = (
                        points[i - 1].altitude_m.filter(|a| a.is_finite()),
                        points[i].altitude_m.filter(|a| a.is_finite()),
                    ) {
                        grade_pct = ((a1 - a0) / horiz_m) * 100.0;
                    }
                    let factor = gap_factor(grade_pct);
                    let adj_speed = s / factor;
                    if adj_speed.is_finite() && adj_speed > 0.0 {
                        gap_speed_dist += adj_speed * horiz_m;
                        gap_dist_m += horiz_m;
                    }
                }

                // Drift halves
                if let Some(mid) = mid_t {
                    if let (Some(t_prev), Some(hr)) = (
                        times[i],
                        points[i]
                            .heart_rate_bpm
                            .filter(|h| *h > 0.0 && h.is_finite()),
                    ) {
                        let speed_dt = s * dt as f64;
                        let hr_dt = hr * dt as f64;
                        if t_prev <= mid {
                            half1_speed_dt += speed_dt;
                            half1_hr_dt += hr_dt;
                            half1_dt = half1_dt.saturating_add(dt_u);
                        } else {
                            half2_speed_dt += speed_dt;
                            half2_hr_dt += hr_dt;
                            half2_dt = half2_dt.saturating_add(dt_u);
                        }
                    }
                }
            }
        } else {
            stopped_seconds = stopped_seconds.saturating_add(dt_u);
        }
    }

    // If elapsed known but moving+stopped under-count (missing timestamps), pad stopped.
    let accounted = moving_seconds.saturating_add(stopped_seconds);
    if elapsed_seconds > accounted {
        stopped_seconds = stopped_seconds.saturating_add(elapsed_seconds - accounted);
    }

    let track_dist_km = distance_span_km(points).or_else(|| {
        if running_dist_m > 0.0 {
            Some(running_dist_m / 1000.0)
        } else {
            None
        }
    });
    // Prefer device/session distance for moving pace (official length); track span is
    // used only when no set-level distance was provided.
    let dist_km = fallback_distance_km.or(track_dist_km);

    let moving_pace_min_per_km = match (dist_km, moving_seconds) {
        (Some(d), m) if d > 0.0 && m > 0 => Some((m as f64 / 60.0) / d),
        _ => None,
    };

    let pace = distribution_of(&pace_samples);
    let pace_cv = cv_of(&pace_samples);

    let synthetic_km_splits = synthetic_km_splits(&cum_dist_m, &cum_time_s, &cum_hr_sum, &cum_hr_n);
    let best_efforts = best_efforts(&cum_dist_m, &cum_time_s);

    let hr_min = hr_values
        .iter()
        .copied()
        .filter(|h| *h > 0.0)
        .reduce(f64::min);

    let hr_drift_pct = compute_drift(
        half1_speed_dt,
        half1_hr_dt,
        half1_dt,
        half2_speed_dt,
        half2_hr_dt,
        half2_dt,
    );

    let hr_zones_recomputed = recompute_zones(points, &times, zone_ctx);

    let cadence = distribution_of(&cadence_samples);
    let cadence_cv = cv_of(&cadence_samples);
    let avg_stride_m = if stride_samples.is_empty() {
        None
    } else {
        Some(stride_samples.iter().sum::<f64>() / stride_samples.len() as f64)
    };

    let elev_net_m = match (first_elev, last_elev) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    let (ascent_m, descent_m) = if elev_min.is_some() {
        Some((ascent, descent))
    } else {
        None
    }
    .map(|(a, d)| (Some(a), Some(d)))
    .unwrap_or((None, None));

    let grade_adj_pace_min_per_km = if gap_dist_m > 0.0 {
        let mean_adj = gap_speed_dist / gap_dist_m;
        if mean_adj > 0.0 {
            Some(speed_to_pace(mean_adj))
        } else {
            None
        }
    } else {
        None
    };

    let vam_m_per_hour = match (ascent_m, moving_seconds) {
        (Some(a), m) if a >= VAM_MIN_ASCENT_M && m > 0 => Some(a / (m as f64 / 3600.0)),
        _ => None,
    };

    let route = route_summary(points);

    Some(TrackMetrics {
        sample_count: points.len(),
        elapsed_seconds,
        moving_seconds,
        stopped_seconds,
        moving_pace_min_per_km,
        pace,
        pace_cv,
        synthetic_km_splits,
        best_efforts,
        hr_min,
        hr_drift_pct,
        hr_zones_recomputed,
        cadence,
        cadence_cv,
        avg_stride_m,
        elev_min_m: elev_min,
        elev_max_m: elev_max,
        elev_net_m,
        ascent_m,
        descent_m,
        grade_adj_pace_min_per_km,
        vam_m_per_hour,
        route,
    })
}

fn parse_ts_unix(s: &str) -> Option<i64> {
    NaiveDateTime::parse_from_str(s, DATETIME_FMT)
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

fn delta_t(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    }
}

fn speed_to_pace(speed_m_s: f64) -> f64 {
    // min/km = (1000/speed) / 60
    (1000.0 / speed_m_s) / 60.0
}

fn gap_factor(grade_pct: f64) -> f64 {
    // Positive grade costs more; mild downhill slightly easier (clamp factor).
    let g = grade_pct;
    let f = if g >= 0.0 {
        1.0 + 0.03 * g + 1.5e-4 * g * g
    } else {
        // mild benefit downhill, avoid extreme
        (1.0 + 0.02 * g).max(0.7)
    };
    f.max(0.5)
}

fn distance_span_km(points: &[Trackpoint]) -> Option<f64> {
    let first = points
        .iter()
        .find_map(|p| p.distance_km.filter(|d| d.is_finite()))?;
    let last = points
        .iter()
        .rev()
        .find_map(|p| p.distance_km.filter(|d| d.is_finite()))?;
    let d = last - first;
    if d > 0.0 {
        Some(d)
    } else if last > 0.0 {
        Some(last)
    } else {
        None
    }
}

fn distribution_of(samples: &[f64]) -> Option<Distribution> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = samples.iter().copied().filter(|x| x.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let min = sorted[0];
    let max = sorted[n - 1];
    let mean = sorted.iter().sum::<f64>() / n as f64;
    let median = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    };
    Some(Distribution {
        min,
        median,
        max,
        mean,
    })
}

fn cv_of(samples: &[f64]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    if mean.abs() < f64::EPSILON {
        return None;
    }
    let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
    Some(var.sqrt() / mean)
}

fn compute_drift(
    half1_speed_dt: f64,
    half1_hr_dt: f64,
    half1_dt: u32,
    half2_speed_dt: f64,
    half2_hr_dt: f64,
    half2_dt: u32,
) -> Option<f64> {
    if half1_dt < DRIFT_MIN_HALF_S || half2_dt < DRIFT_MIN_HALF_S {
        return None;
    }
    if half1_hr_dt <= 0.0 || half2_hr_dt <= 0.0 {
        return None;
    }
    // Efficiency = mean_speed / mean_HR
    let mean_sp1 = half1_speed_dt / half1_dt as f64;
    let mean_hr1 = half1_hr_dt / half1_dt as f64;
    let mean_sp2 = half2_speed_dt / half2_dt as f64;
    let mean_hr2 = half2_hr_dt / half2_dt as f64;
    if mean_hr1 <= 0.0 || mean_hr2 <= 0.0 {
        return None;
    }
    let ef1 = mean_sp1 / mean_hr1;
    let ef2 = mean_sp2 / mean_hr2;
    if ef1 <= 0.0 {
        return None;
    }
    // Positive = second half less efficient (more strain) — invert EF ratio convention:
    // plan: drift_pct = 100 * (EF2/EF1 - 1). If EF drops, drift is negative...
    // Plan said "positive = more strain second half". Strain increases when EF decreases,
    // so use 100 * (EF1/EF2 - 1) OR 100 * (1 - EF2/EF1). Use 100 * (EF1/EF2 - 1) for positive strain.
    // Re-read plan: "drift_pct = 100 * (EF2/EF1 - 1) (positive = more strain second half)"
    // That's wrong if EF2 < EF1 (more strain) → negative. We'll use 100 * (EF1/EF2 - 1)
    // so higher HR for same speed in H2 → positive drift.
    Some(100.0 * (ef1 / ef2 - 1.0))
}

fn recompute_zones(
    points: &[Trackpoint],
    times: &[Option<i64>],
    zone_ctx: &ZoneRecomputeContext,
) -> Option<HeartRateZones> {
    let dob = zone_ctx.date_of_birth.as_deref()?;
    let date_str = zone_ctx.activity_date.as_deref()?;
    let on_date = NaiveDate::parse_from_str(date_str.trim(), "%Y-%m-%d").ok()?;
    let sleep = zone_ctx.resting_hr_bpm.map(|r| vec![r]).unwrap_or_default();
    let profile = hr_zones::resolve_auto_bounds(dob, on_date, &sleep)?;
    let bounds = profile.bounds;

    let mut zones = HeartRateZones::default();
    let mut prev_t: Option<i64> = None;
    for (i, p) in points.iter().enumerate() {
        let hr = match p.heart_rate_bpm {
            Some(h) if h > 0.0 && h.is_finite() => h,
            _ => {
                prev_t = times[i];
                continue;
            }
        };
        let dt = match (prev_t, times[i]) {
            (Some(a), Some(b)) => (b - a).max(0) as u32,
            _ => 1,
        };
        prev_t = times[i];
        // Cap single-step contribution (avoid pause gaps inflating a zone)
        let secs = dt.min(PAUSE_GAP_S as u32).max(1);
        if hr <= bounds[0] {
            zones.z1_seconds = zones.z1_seconds.saturating_add(secs);
        } else if hr <= bounds[1] {
            zones.z2_seconds = zones.z2_seconds.saturating_add(secs);
        } else if hr <= bounds[2] {
            zones.z3_seconds = zones.z3_seconds.saturating_add(secs);
        } else if hr <= bounds[3] {
            zones.z4_seconds = zones.z4_seconds.saturating_add(secs);
        } else {
            zones.z5_seconds = zones.z5_seconds.saturating_add(secs);
        }
    }
    let total = zones.z1_seconds
        + zones.z2_seconds
        + zones.z3_seconds
        + zones.z4_seconds
        + zones.z5_seconds;
    if total == 0 {
        None
    } else {
        Some(zones)
    }
}

fn synthetic_km_splits(
    cum_dist_m: &[f64],
    cum_time_s: &[i64],
    cum_hr_sum: &[f64],
    cum_hr_n: &[u32],
) -> Vec<KmSplit> {
    if cum_dist_m.is_empty() {
        return Vec::new();
    }
    let mut splits = Vec::new();
    let mut next_km = 1u16;
    let mut start_idx = 0usize;

    while next_km < 500 {
        let target = next_km as f64 * 1000.0;
        let end_idx = cum_dist_m.iter().position(|&d| d >= target);
        let Some(end_idx) = end_idx else {
            break;
        };
        let t0 = cum_time_s[start_idx];
        let t1 = cum_time_s[end_idx];
        let dur = (t1 - t0).max(0) as u32;
        if dur == 0 {
            start_idx = end_idx;
            next_km += 1;
            continue;
        }
        let dist_km = 1.0;
        let pace = (dur as f64 / 60.0) / dist_km;
        let avg_hr = avg_hr_window(cum_hr_sum, cum_hr_n, start_idx, end_idx);
        splits.push(KmSplit {
            km_index: next_km,
            partial: false,
            distance_km: dist_km,
            duration_seconds: dur,
            pace_min_per_km: pace,
            avg_hr,
        });
        start_idx = end_idx;
        next_km += 1;
    }

    // Partial final km
    let total_m = *cum_dist_m.last().unwrap_or(&0.0);
    let full_m = ((next_km - 1) as f64) * 1000.0;
    let rem_m = total_m - full_m;
    if rem_m >= PARTIAL_KM_MIN * 1000.0 {
        let t0 = cum_time_s[start_idx];
        let t1 = *cum_time_s.last().unwrap_or(&t0);
        let dur = (t1 - t0).max(0) as u32;
        let dist_km = rem_m / 1000.0;
        if dur > 0 && dist_km > 0.0 {
            let pace = (dur as f64 / 60.0) / dist_km;
            let avg_hr = avg_hr_window(cum_hr_sum, cum_hr_n, start_idx, cum_dist_m.len() - 1);
            splits.push(KmSplit {
                km_index: next_km,
                partial: true,
                distance_km: dist_km,
                duration_seconds: dur,
                pace_min_per_km: pace,
                avg_hr,
            });
        }
    }

    splits
}

fn avg_hr_window(
    cum_hr_sum: &[f64],
    cum_hr_n: &[u32],
    start_idx: usize,
    end_idx: usize,
) -> Option<f64> {
    if end_idx >= cum_hr_sum.len() || start_idx >= cum_hr_sum.len() {
        return None;
    }
    let (sum0, n0) = if start_idx == 0 {
        (0.0, 0u32)
    } else {
        (cum_hr_sum[start_idx], cum_hr_n[start_idx])
    };
    let sum1 = cum_hr_sum[end_idx];
    let n1 = cum_hr_n[end_idx];
    if n1 <= n0 {
        return None;
    }
    Some((sum1 - sum0) / (n1 - n0) as f64)
}

fn best_efforts(cum_dist_m: &[f64], cum_time_s: &[i64]) -> Vec<BestEffort> {
    let mut out = Vec::new();
    if cum_dist_m.len() < 2 {
        return out;
    }
    let total_m = *cum_dist_m.last().unwrap_or(&0.0);
    let total_s = *cum_time_s.last().unwrap_or(&0);

    for &(label, dist_m) in &[("400 m", 400.0), ("1 km", 1000.0), ("1 mi", 1609.34)] {
        if total_m + 1.0 < dist_m {
            continue;
        }
        if let Some((dur, pace)) = fastest_time_for_distance(cum_dist_m, cum_time_s, dist_m) {
            out.push(BestEffort {
                label: label.to_string(),
                duration_seconds: Some(dur),
                distance_km: Some(dist_m / 1000.0),
                pace_min_per_km: Some(pace),
            });
        }
    }

    for &(label, window_s) in &[("5 min", 300i64), ("10 min", 600i64)] {
        if total_s < window_s {
            continue;
        }
        if let Some(dist_m) = farthest_distance_in_time(cum_dist_m, cum_time_s, window_s) {
            let pace = if dist_m > 0.0 {
                Some((window_s as f64 / 60.0) / (dist_m / 1000.0))
            } else {
                None
            };
            out.push(BestEffort {
                label: label.to_string(),
                duration_seconds: Some(window_s as u32),
                distance_km: Some(dist_m / 1000.0),
                pace_min_per_km: pace,
            });
        }
    }

    out
}

/// Two-pointer: minimum time to cover `target_m`.
fn fastest_time_for_distance(
    cum_dist_m: &[f64],
    cum_time_s: &[i64],
    target_m: f64,
) -> Option<(u32, f64)> {
    let n = cum_dist_m.len();
    let mut best: Option<u32> = None;
    let mut j = 0usize;
    for i in 0..n {
        while j < n && cum_dist_m[j] - cum_dist_m[i] < target_m {
            j += 1;
        }
        if j >= n {
            break;
        }
        let dt = (cum_time_s[j] - cum_time_s[i]).max(0) as u32;
        if dt == 0 {
            continue;
        }
        best = Some(best.map_or(dt, |b| b.min(dt)));
    }
    best.map(|dur| {
        let pace = (dur as f64 / 60.0) / (target_m / 1000.0);
        (dur, pace)
    })
}

fn farthest_distance_in_time(cum_dist_m: &[f64], cum_time_s: &[i64], window_s: i64) -> Option<f64> {
    let n = cum_dist_m.len();
    let mut best = 0.0;
    let mut j = 0usize;
    for i in 0..n {
        while j + 1 < n && cum_time_s[j + 1] - cum_time_s[i] <= window_s {
            j += 1;
        }
        if cum_time_s[j] - cum_time_s[i] <= window_s {
            let d = cum_dist_m[j] - cum_dist_m[i];
            if d > best {
                best = d;
            }
        }
    }
    if best > 0.0 {
        Some(best)
    } else {
        None
    }
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let to_rad = std::f64::consts::PI / 180.0;
    let dlat = (lat2 - lat1) * to_rad;
    let dlon = (lon2 - lon1) * to_rad;
    let a = (dlat / 2.0).sin().powi(2)
        + (lat1 * to_rad).cos() * (lat2 * to_rad).cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    EARTH_RADIUS_M * c
}

fn route_summary(points: &[Trackpoint]) -> Option<RouteSummary> {
    // Keep timestamps so glitch filter can use implied speed (sparse samples may jump >200 m).
    let gps: Vec<(f64, f64, Option<i64>)> = points
        .iter()
        .filter_map(|p| match (p.latitude, p.longitude) {
            (Some(lat), Some(lon)) if lat.is_finite() && lon.is_finite() => {
                Some((lat, lon, parse_ts_unix(&p.recorded_at)))
            }
            _ => None,
        })
        .collect();

    if gps.len() < 2 {
        return None;
    }

    let mut path_m = 0.0;
    for w in gps.windows(2) {
        let d = haversine_m(w[0].0, w[0].1, w[1].0, w[1].1);
        if !d.is_finite() || d <= 0.0 {
            continue;
        }
        // Drop teleport glitches: huge jump in a short time, or absurd implied speed.
        let dt = match (w[0].2, w[1].2) {
            (Some(a), Some(b)) if b > a => Some(b - a),
            _ => None,
        };
        let is_glitch = match dt {
            Some(dt) if dt <= 5 && d > GPS_GLITCH_M => true,
            Some(dt) if dt > 0 && d / dt as f64 > 55.0 => true, // > ~200 km/h
            None if d > GPS_GLITCH_M * 5.0 => true,             // no time: allow larger jumps
            _ => false,
        };
        if !is_glitch {
            path_m += d;
        }
    }

    let start = (gps[0].0, gps[0].1);
    let end = (gps[gps.len() - 1].0, gps[gps.len() - 1].1);
    let gap_m = haversine_m(start.0, start.1, end.0, end.1);
    let crow = gap_m / 1000.0;

    let lat_min = gps.iter().map(|g| g.0).fold(f64::INFINITY, f64::min);
    let lat_max = gps.iter().map(|g| g.0).fold(f64::NEG_INFINITY, f64::max);
    let lon_min = gps.iter().map(|g| g.1).fold(f64::INFINITY, f64::min);
    let lon_max = gps.iter().map(|g| g.1).fold(f64::NEG_INFINITY, f64::max);

    let loop_thresh = LOOP_MIN_GAP_M.max(0.02 * path_m.max(gap_m));
    let kind = if gap_m.is_finite() {
        if gap_m < loop_thresh {
            RouteKind::Loop
        } else {
            RouteKind::PointToPoint
        }
    } else {
        RouteKind::Unknown
    };

    Some(RouteSummary {
        kind,
        gps_distance_km: if path_m > 0.0 {
            Some(path_m / 1000.0)
        } else {
            None
        },
        crow_flies_km: if crow.is_finite() { Some(crow) } else { None },
        start_end_gap_m: if gap_m.is_finite() { Some(gap_m) } else { None },
        lat_span_deg: Some(lat_max - lat_min),
        lon_span_deg: Some(lon_max - lon_min),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tp(t: &str, dist_km: f64, speed: f64, hr: f64) -> Trackpoint {
        Trackpoint {
            recorded_at: t.into(),
            latitude: None,
            longitude: None,
            altitude_m: None,
            heart_rate_bpm: Some(hr),
            cadence_spm: None,
            distance_km: Some(dist_km),
            speed_m_s: Some(speed),
        }
    }

    fn steady_run(seconds: i64, speed_m_s: f64, hr: f64) -> Vec<Trackpoint> {
        let mut pts = Vec::new();
        let base = NaiveDateTime::parse_from_str("2026-07-10 10:00:00", DATETIME_FMT).unwrap();
        for s in 0..=seconds {
            let t = (base + chrono::Duration::seconds(s))
                .format(DATETIME_FMT)
                .to_string();
            let dist_km = (speed_m_s * s as f64) / 1000.0;
            let mut p = tp(&t, dist_km, speed_m_s, hr);
            p.cadence_spm = Some(80.0);
            p.altitude_m = Some(100.0);
            pts.push(p);
        }
        pts
    }

    #[test]
    fn constant_speed_moving_equals_elapsed() {
        // 3 m/s for 1000 s → 3 km, pace = 1000/3/60 ≈ 5.555 min/km
        let pts = steady_run(1000, 3.0, 150.0);
        let m = compute(&pts, None).unwrap();
        assert_eq!(m.sample_count, 1001);
        assert_eq!(m.elapsed_seconds, 1000);
        assert!(m.moving_seconds >= 990 && m.moving_seconds <= 1000);
        assert!(m.stopped_seconds < 20);
        let pace = m.moving_pace_min_per_km.unwrap();
        assert!((pace - 5.555).abs() < 0.05, "pace={}", pace);
        assert!(m.synthetic_km_splits.iter().filter(|s| !s.partial).count() >= 2);
        assert!(m.best_efforts.iter().any(|b| b.label == "1 km"));
    }

    #[test]
    fn stop_in_middle_counts_stopped() {
        let base = NaiveDateTime::parse_from_str("2026-07-10 10:00:00", DATETIME_FMT).unwrap();
        let mut pts = Vec::new();
        // 100 s moving at 3 m/s
        for s in 0..=100 {
            let t = (base + chrono::Duration::seconds(s))
                .format(DATETIME_FMT)
                .to_string();
            pts.push(tp(&t, 3.0 * s as f64 / 1000.0, 3.0, 140.0));
        }
        // 60 s stopped
        for s in 101..=160 {
            let t = (base + chrono::Duration::seconds(s))
                .format(DATETIME_FMT)
                .to_string();
            pts.push(tp(&t, 0.3, 0.0, 120.0));
        }
        // 100 s moving again
        for s in 161..=260 {
            let t = (base + chrono::Duration::seconds(s))
                .format(DATETIME_FMT)
                .to_string();
            let dist = 0.3 + 3.0 * (s - 160) as f64 / 1000.0;
            pts.push(tp(&t, dist, 3.0, 145.0));
        }
        let m = compute(&pts, None).unwrap();
        assert!(m.stopped_seconds >= 50, "stopped={}", m.stopped_seconds);
        assert!(m.moving_seconds < m.elapsed_seconds);
        let mp = m.moving_pace_min_per_km.unwrap();
        // distance ~0.6 km, moving ~200 s → pace ~5.55
        assert!((mp - 5.55).abs() < 0.5, "moving pace={}", mp);
    }

    #[test]
    fn gps_square_is_loop() {
        // Rough ~1 km sides near equator: 1/111 deg ≈ 0.009 deg lat
        let step = 0.009;
        let base = NaiveDateTime::parse_from_str("2026-07-10 10:00:00", DATETIME_FMT).unwrap();
        let corners = [
            (0.0, 0.0),
            (step, 0.0),
            (step, step),
            (0.0, step),
            (0.0, 0.0),
        ];
        let mut pts = Vec::new();
        for (i, (lat, lon)) in corners.iter().enumerate() {
            let t = (base + chrono::Duration::seconds(i as i64 * 300))
                .format(DATETIME_FMT)
                .to_string();
            let mut p = tp(&t, i as f64, 3.0, 140.0);
            p.latitude = Some(*lat);
            p.longitude = Some(*lon);
            pts.push(p);
        }
        let m = compute(&pts, None).unwrap();
        let route = m.route.unwrap();
        assert_eq!(route.kind, RouteKind::Loop);
        let gps = route.gps_distance_km.unwrap();
        assert!(gps > 3.5 && gps < 4.5, "gps path km={}", gps);
    }

    #[test]
    fn hr_drift_positive_when_second_half_harder() {
        let base = NaiveDateTime::parse_from_str("2026-07-10 10:00:00", DATETIME_FMT).unwrap();
        let mut pts = Vec::new();
        // 200 s at speed 3, HR 140 then 200 s speed 3, HR 170
        for s in 0..=400 {
            let t = (base + chrono::Duration::seconds(s))
                .format(DATETIME_FMT)
                .to_string();
            let hr = if s <= 200 { 140.0 } else { 170.0 };
            pts.push(tp(&t, 3.0 * s as f64 / 1000.0, 3.0, hr));
        }
        let m = compute(&pts, None).unwrap();
        let drift = m.hr_drift_pct.unwrap();
        assert!(drift > 5.0, "expected positive drift, got {}", drift);
    }

    #[test]
    fn elevation_smoothed_ignores_noise() {
        let base = NaiveDateTime::parse_from_str("2026-07-10 10:00:00", DATETIME_FMT).unwrap();
        let alts = [100.0, 100.5, 101.0, 110.0, 110.3, 120.0]; // +10 +10 real after noise floor
        let mut pts = Vec::new();
        for (i, alt) in alts.iter().enumerate() {
            let t = (base + chrono::Duration::seconds(i as i64 * 10))
                .format(DATETIME_FMT)
                .to_string();
            let mut p = tp(&t, i as f64 * 0.05, 3.0, 140.0);
            p.altitude_m = Some(*alt);
            pts.push(p);
        }
        let m = compute(&pts, None).unwrap();
        assert_eq!(m.elev_min_m, Some(100.0));
        assert_eq!(m.elev_max_m, Some(120.0));
        let ascent = m.ascent_m.unwrap();
        assert!((18.0..=22.0).contains(&ascent), "ascent={}", ascent);
    }

    #[test]
    fn stride_from_cadence_and_speed() {
        // speed 3 m/s, cadence 180 spm → steps/s = 3 → stride = 1.0 m
        let pts = {
            let mut p = steady_run(60, 3.0, 140.0);
            for x in &mut p {
                x.cadence_spm = Some(180.0);
            }
            p
        };
        let m = compute(&pts, None).unwrap();
        let stride = m.avg_stride_m.unwrap();
        assert!((stride - 1.0).abs() < 0.05, "stride={}", stride);
    }

    #[test]
    fn zone_recompute_with_dob() {
        let pts = steady_run(120, 3.0, 150.0);
        let ctx = ZoneRecomputeContext {
            date_of_birth: Some("1983-07-21".into()),
            resting_hr_bpm: Some(52.0),
            activity_date: Some("2026-07-10".into()),
        };
        let m = compute_with_zones(&pts, None, &ctx).unwrap();
        assert!(m.hr_zones_recomputed.is_some());
        let z = m.hr_zones_recomputed.unwrap();
        let total = z.z1_seconds + z.z2_seconds + z.z3_seconds + z.z4_seconds + z.z5_seconds;
        assert!(total > 0);
    }
}
