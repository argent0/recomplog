//! Absolute range validation (hard-fail) and delta-vs-previous warnings for measurements.
//! Absolute range validation for sleep (hard-fail only — no variation checks).
//!
//! Absolute checks reject physical impossibilities. Delta checks only warn so legitimate
//! corrections, scale changes, and bulk imports are not blocked.
//!
//! Limits come from the application config file (see `app_config`).

use crate::config::{
    AbsoluteLimits, MetricLimits, SanityLimits, SleepSanityLimits, WorkoutSanityLimits,
};
use crate::error::{RecomplogError, Result as RResult};
use crate::models::{HeartRateZones, Lap};
use chrono::NaiveDate;
use serde::Serialize;

/// Metrics proposed on create/update (only fields that were supplied).
#[derive(Debug, Clone, Default)]
pub struct ProposedMetrics {
    pub weight_kg: Option<f64>,
    pub body_fat_pct: Option<f64>,
    pub skeletal_muscle_pct: Option<f64>,
    pub visceral_fat_level: Option<i64>,
    pub bmi: Option<f64>,
    pub resting_metabolism_kcal: Option<i64>,
}

/// Sleep metrics proposed on create/update (only fields that were supplied).
#[derive(Debug, Clone, Default)]
pub struct ProposedSleepMetrics {
    pub time_in_bed_minutes: Option<i64>,
    pub total_sleep_minutes: Option<i64>,
    pub rem_minutes: Option<i64>,
    pub deep_minutes: Option<i64>,
    pub light_minutes: Option<i64>,
    pub awake_minutes: Option<i64>,
    pub sleep_efficiency_pct: Option<f64>,
    pub sleep_score: Option<i64>,
    pub subjective_quality: Option<i64>,
    pub awakenings: Option<i64>,
    pub heart_rate_bpm: Option<f64>,
    pub hypopnea_per_hr: Option<f64>,
    pub respiratory_rate: Option<f64>,
}

/// Most recent prior non-null value for each field, with the date it came from.
#[derive(Debug, Clone, Default)]
pub struct PreviousMetrics {
    pub weight_kg: Option<(String, f64)>,
    pub body_fat_pct: Option<(String, f64)>,
    pub skeletal_muscle_pct: Option<(String, f64)>,
    pub visceral_fat_level: Option<(String, f64)>,
    pub bmi: Option<(String, f64)>,
    pub resting_metabolism_kcal: Option<(String, f64)>,
}

/// A non-fatal sanity warning (delta outlier).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SanityWarning {
    pub field: String,
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_delta: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_gap: Option<i64>,
}

/// Hard-fail absolute impossibilities using configured limits.
pub fn validate_absolute(new: &ProposedMetrics, limits: &SanityLimits) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    check_f64_range(
        "weight_kg",
        new.weight_kg,
        &limits.weight_kg.absolute(),
        &mut errors,
    );
    check_f64_range(
        "body_fat_pct",
        new.body_fat_pct,
        &limits.body_fat_pct.absolute(),
        &mut errors,
    );
    check_f64_range(
        "skeletal_muscle_pct",
        new.skeletal_muscle_pct,
        &limits.skeletal_muscle_pct.absolute(),
        &mut errors,
    );
    check_i64_range(
        "visceral_fat_level",
        new.visceral_fat_level,
        &limits.visceral_fat_level.absolute(),
        &mut errors,
    );
    check_f64_range("bmi", new.bmi, &limits.bmi.absolute(), &mut errors);
    check_i64_range(
        "resting_metabolism_kcal",
        new.resting_metabolism_kcal,
        &limits.resting_metabolism_kcal.absolute(),
        &mut errors,
    );

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Hard-fail absolute impossibilities for sleep fields (no variation / delta checks).
pub fn validate_sleep_absolute(
    new: &ProposedSleepMetrics,
    limits: &SleepSanityLimits,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    check_i64_range(
        "time_in_bed_minutes",
        new.time_in_bed_minutes,
        &limits.time_in_bed_minutes,
        &mut errors,
    );
    check_i64_range(
        "total_sleep_minutes",
        new.total_sleep_minutes,
        &limits.total_sleep_minutes,
        &mut errors,
    );
    check_i64_range(
        "rem_minutes",
        new.rem_minutes,
        &limits.rem_minutes,
        &mut errors,
    );
    check_i64_range(
        "deep_minutes",
        new.deep_minutes,
        &limits.deep_minutes,
        &mut errors,
    );
    check_i64_range(
        "light_minutes",
        new.light_minutes,
        &limits.light_minutes,
        &mut errors,
    );
    check_i64_range(
        "awake_minutes",
        new.awake_minutes,
        &limits.awake_minutes,
        &mut errors,
    );
    check_f64_range(
        "sleep_efficiency_pct",
        new.sleep_efficiency_pct,
        &limits.sleep_efficiency_pct,
        &mut errors,
    );
    check_i64_range(
        "sleep_score",
        new.sleep_score,
        &limits.sleep_score,
        &mut errors,
    );
    check_i64_range(
        "subjective_quality",
        new.subjective_quality,
        &limits.subjective_quality,
        &mut errors,
    );
    check_i64_range(
        "awakenings",
        new.awakenings,
        &limits.awakenings,
        &mut errors,
    );
    check_f64_range(
        "heart_rate_bpm",
        new.heart_rate_bpm,
        &limits.heart_rate_bpm,
        &mut errors,
    );
    check_f64_range(
        "hypopnea_per_hr",
        new.hypopnea_per_hr,
        &limits.hypopnea_per_hr,
        &mut errors,
    );
    check_f64_range(
        "respiratory_rate",
        new.respiratory_rate,
        &limits.respiratory_rate,
        &mut errors,
    );

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn check_f64_range(
    field: &str,
    value: Option<f64>,
    limits: &AbsoluteLimits,
    errors: &mut Vec<String>,
) {
    let Some(v) = value else {
        return;
    };
    if !v.is_finite() {
        errors.push(format!("{} must be a finite number", field));
        return;
    }
    if v < limits.min || v > limits.max {
        errors.push(format!(
            "{} {} is outside allowed range {}–{}",
            field,
            format_num(v),
            format_num(limits.min),
            format_num(limits.max)
        ));
    }
}

fn check_i64_range(
    field: &str,
    value: Option<i64>,
    limits: &AbsoluteLimits,
    errors: &mut Vec<String>,
) {
    let Some(v) = value else {
        return;
    };
    // Compare as f64 against config limits (stored uniformly as REAL-like numbers).
    let vf = v as f64;
    if vf < limits.min || vf > limits.max {
        errors.push(format!(
            "{} {} is outside allowed range {}–{}",
            field,
            v,
            format_num(limits.min),
            format_num(limits.max)
        ));
    }
}

fn check_i32_range(
    field: &str,
    value: Option<i32>,
    limits: &AbsoluteLimits,
    errors: &mut Vec<String>,
) {
    check_i64_range(field, value.map(i64::from), limits, errors);
}

/// Soft delta warnings vs previous values using configured thresholds.
pub fn check_deltas(
    new: &ProposedMetrics,
    previous: &PreviousMetrics,
    new_date: &str,
    limits: &SanityLimits,
) -> Vec<SanityWarning> {
    let mut warnings = Vec::new();

    check_f64_delta(
        "weight_kg",
        new.weight_kg,
        previous.weight_kg.as_ref(),
        new_date,
        &limits.weight_kg,
        &mut warnings,
    );
    check_f64_delta(
        "body_fat_pct",
        new.body_fat_pct,
        previous.body_fat_pct.as_ref(),
        new_date,
        &limits.body_fat_pct,
        &mut warnings,
    );
    check_f64_delta(
        "skeletal_muscle_pct",
        new.skeletal_muscle_pct,
        previous.skeletal_muscle_pct.as_ref(),
        new_date,
        &limits.skeletal_muscle_pct,
        &mut warnings,
    );
    check_f64_delta(
        "visceral_fat_level",
        new.visceral_fat_level.map(|v| v as f64),
        previous.visceral_fat_level.as_ref(),
        new_date,
        &limits.visceral_fat_level,
        &mut warnings,
    );
    check_f64_delta(
        "bmi",
        new.bmi,
        previous.bmi.as_ref(),
        new_date,
        &limits.bmi,
        &mut warnings,
    );
    check_f64_delta(
        "resting_metabolism_kcal",
        new.resting_metabolism_kcal.map(|v| v as f64),
        previous.resting_metabolism_kcal.as_ref(),
        new_date,
        &limits.resting_metabolism_kcal,
        &mut warnings,
    );

    warnings
}

fn check_f64_delta(
    field: &str,
    new_val: Option<f64>,
    prev: Option<&(String, f64)>,
    new_date: &str,
    limits: &MetricLimits,
    warnings: &mut Vec<SanityWarning>,
) {
    let (Some(new_v), Some((prev_date, prev_v))) = (new_val, prev) else {
        return;
    };

    let days = days_gap(prev_date, new_date).unwrap_or(1).max(1);
    let allowed = limits.delta_base + limits.delta_per_day * (days - 1) as f64;
    let delta = (new_v - prev_v).abs();
    if delta > allowed {
        let message = format!(
            "{} {} differs by {} from previous {} on {} (allowed ±{} for {} day gap)",
            field,
            format_num(new_v),
            format_num(delta),
            format_num(*prev_v),
            prev_date,
            format_num(allowed),
            days
        );
        warnings.push(SanityWarning {
            field: field.to_string(),
            kind: "delta".to_string(),
            message,
            previous_value: Some(*prev_v),
            previous_date: Some(prev_date.clone()),
            new_value: Some(new_v),
            delta: Some(delta),
            allowed_delta: Some(allowed),
            days_gap: Some(days),
        });
    }
}

fn days_gap(prev_date: &str, new_date: &str) -> Option<i64> {
    let prev = NaiveDate::parse_from_str(prev_date, "%Y-%m-%d").ok()?;
    let new = NaiveDate::parse_from_str(new_date, "%Y-%m-%d").ok()?;
    Some((new - prev).num_days())
}

fn format_num(v: f64) -> String {
    // Prefer compact integers when whole; otherwise one decimal is usually enough.
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{:.1}", v)
    }
}

// ---------- Workout set absolute validation ----------

/// Metrics proposed on set create/update (only fields that were supplied).
#[derive(Debug, Clone, Default)]
pub struct ProposedSetMetrics {
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub external_load_kg: Option<f64>,
    pub distance_km: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub rpe: Option<f64>,
    pub rir: Option<f64>,
    pub effective_reps: Option<i32>,
    pub rest_seconds: Option<i32>,
    pub avg_heart_rate_bpm: Option<f64>,
    pub max_heart_rate_bpm: Option<f64>,
    pub avg_pace_min_per_km: Option<f64>,
    pub calories_burned: Option<i32>,
    pub avg_cadence_spm: Option<f64>,
    pub total_ascent_m: Option<f64>,
    pub total_descent_m: Option<f64>,
    pub heart_rate_zones: Option<HeartRateZones>,
    pub laps: Option<Vec<Lap>>,
}

/// Hard-fail absolute impossibilities for set metrics.
pub fn validate_set_metrics(new: &ProposedSetMetrics, limits: &WorkoutSanityLimits) -> RResult<()> {
    let mut errors = Vec::new();

    check_i32_range("reps", new.reps, &limits.reps, &mut errors);
    check_f64_range("weight_kg", new.weight_kg, &limits.weight_kg, &mut errors);
    check_f64_range(
        "external_load_kg",
        new.external_load_kg,
        &limits.external_load_kg,
        &mut errors,
    );
    check_i32_range(
        "duration_seconds",
        new.duration_seconds,
        &limits.duration_seconds,
        &mut errors,
    );
    check_f64_range(
        "distance_km",
        new.distance_km,
        &limits.distance_km,
        &mut errors,
    );
    check_f64_range("rpe", new.rpe, &limits.rpe, &mut errors);
    check_f64_range("rir", new.rir, &limits.rir, &mut errors);
    check_i32_range(
        "effective_reps",
        new.effective_reps,
        &limits.effective_reps,
        &mut errors,
    );
    check_i32_range(
        "rest_seconds",
        new.rest_seconds,
        &limits.rest_seconds,
        &mut errors,
    );
    check_f64_range(
        "avg_heart_rate_bpm",
        new.avg_heart_rate_bpm,
        &limits.heart_rate_bpm,
        &mut errors,
    );
    check_f64_range(
        "max_heart_rate_bpm",
        new.max_heart_rate_bpm,
        &limits.heart_rate_bpm,
        &mut errors,
    );
    check_f64_range(
        "avg_pace_min_per_km",
        new.avg_pace_min_per_km,
        &limits.pace_min_per_km,
        &mut errors,
    );
    check_i32_range(
        "calories_burned",
        new.calories_burned,
        &limits.calories_burned,
        &mut errors,
    );
    check_f64_range(
        "avg_cadence_spm",
        new.avg_cadence_spm,
        &limits.cadence_spm,
        &mut errors,
    );
    check_f64_range(
        "total_ascent_m",
        new.total_ascent_m,
        &limits.elevation_m,
        &mut errors,
    );
    check_f64_range(
        "total_descent_m",
        new.total_descent_m,
        &limits.elevation_m,
        &mut errors,
    );

    if let (Some(avg), Some(max)) = (new.avg_heart_rate_bpm, new.max_heart_rate_bpm) {
        if avg.is_finite() && max.is_finite() && avg > max {
            errors.push(format!(
                "avg_heart_rate_bpm ({}) must be <= max_heart_rate_bpm ({})",
                format_num(avg),
                format_num(max)
            ));
        }
    }

    if let (Some(reps), Some(eff)) = (new.reps, new.effective_reps) {
        if eff > reps {
            errors.push(format!(
                "effective_reps ({}) must be <= reps ({})",
                eff, reps
            ));
        }
    }

    if let Some(ref zones) = new.heart_rate_zones {
        for (name, val) in [
            ("z1_seconds", zones.z1_seconds),
            ("z2_seconds", zones.z2_seconds),
            ("z3_seconds", zones.z3_seconds),
            ("z4_seconds", zones.z4_seconds),
            ("z5_seconds", zones.z5_seconds),
        ] {
            check_i32_range(name, Some(val as i32), &limits.hr_zone_seconds, &mut errors);
        }
        if let Some(dur) = new.duration_seconds {
            let sum = zones.z1_seconds as u64
                + zones.z2_seconds as u64
                + zones.z3_seconds as u64
                + zones.z4_seconds as u64
                + zones.z5_seconds as u64;
            let cap = ((dur as f64) * 1.1).ceil() as u64;
            if sum > cap {
                errors.push(format!(
                    "heart_rate_zones sum ({} s) exceeds duration_seconds * 1.1 ({} s)",
                    sum, cap
                ));
            }
        }
    }

    if let Some(ref laps) = new.laps {
        for lap in laps {
            let prefix = format!("lap {}", lap.lap_number);
            check_f64_range(
                &format!("{prefix} distance_km"),
                Some(lap.distance_km),
                &limits.distance_km,
                &mut errors,
            );
            check_i32_range(
                &format!("{prefix} duration_seconds"),
                Some(lap.duration_seconds as i32),
                &limits.duration_seconds,
                &mut errors,
            );
            check_f64_range(
                &format!("{prefix} pace_min_per_km"),
                Some(lap.pace_min_per_km),
                &limits.pace_min_per_km,
                &mut errors,
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(RecomplogError::Sanity(errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> SanityLimits {
        SanityLimits::default()
    }

    fn proposed_weight(w: f64) -> ProposedMetrics {
        ProposedMetrics {
            weight_kg: Some(w),
            ..Default::default()
        }
    }

    #[test]
    fn absolute_weight_bounds() {
        let lim = limits();
        assert!(validate_absolute(&proposed_weight(20.0), &lim).is_ok());
        assert!(validate_absolute(&proposed_weight(300.0), &lim).is_ok());
        assert!(validate_absolute(&proposed_weight(19.9), &lim).is_err());
        assert!(validate_absolute(&proposed_weight(300.1), &lim).is_err());
        assert!(validate_absolute(&proposed_weight(f64::NAN), &lim).is_err());
        assert!(validate_absolute(&proposed_weight(f64::INFINITY), &lim).is_err());
    }

    #[test]
    fn absolute_body_fat_out_of_range() {
        let m = ProposedMetrics {
            body_fat_pct: Some(150.0),
            ..Default::default()
        };
        let err = validate_absolute(&m, &limits()).unwrap_err();
        assert!(err.iter().any(|e| e.contains("body_fat_pct")));
    }

    #[test]
    fn absolute_collects_multiple_errors() {
        let m = ProposedMetrics {
            weight_kg: Some(5.0),
            body_fat_pct: Some(150.0),
            ..Default::default()
        };
        let err = validate_absolute(&m, &limits()).unwrap_err();
        assert_eq!(err.len(), 2);
    }

    #[test]
    fn absolute_skips_absent_fields() {
        assert!(validate_absolute(&ProposedMetrics::default(), &limits()).is_ok());
    }

    #[test]
    fn absolute_uses_custom_limits() {
        let mut lim = limits();
        lim.weight_kg.min = 50.0;
        lim.weight_kg.max = 90.0;
        assert!(validate_absolute(&proposed_weight(49.0), &lim).is_err());
        assert!(validate_absolute(&proposed_weight(82.0), &lim).is_ok());
    }

    #[test]
    fn delta_within_one_day_limit() {
        let prev = PreviousMetrics {
            weight_kg: Some(("2026-06-07".into(), 82.0)),
            ..Default::default()
        };
        let lim = limits();
        // exactly base (3.0) should not warn
        let warnings = check_deltas(&proposed_weight(85.0), &prev, "2026-06-08", &lim);
        assert!(warnings.is_empty());
        // just over
        let warnings = check_deltas(&proposed_weight(85.1), &prev, "2026-06-08", &lim);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "weight_kg");
        assert_eq!(warnings[0].kind, "delta");
        assert_eq!(warnings[0].days_gap, Some(1));
    }

    #[test]
    fn delta_gap_scaling() {
        let prev = PreviousMetrics {
            weight_kg: Some(("2026-06-01".into(), 80.0)),
            ..Default::default()
        };
        let lim = limits();
        // 7 day gap: allowed = 3.0 + 6*0.5 = 6.0
        let warnings = check_deltas(&proposed_weight(86.0), &prev, "2026-06-08", &lim);
        assert!(warnings.is_empty());
        let warnings = check_deltas(&proposed_weight(86.1), &prev, "2026-06-08", &lim);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].allowed_delta, Some(6.0));
        assert_eq!(warnings[0].days_gap, Some(7));
    }

    #[test]
    fn delta_no_previous_no_warning() {
        let warnings = check_deltas(
            &proposed_weight(95.0),
            &PreviousMetrics::default(),
            "2026-06-08",
            &limits(),
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn delta_only_checks_provided_fields() {
        let prev = PreviousMetrics {
            weight_kg: Some(("2026-06-07".into(), 82.0)),
            body_fat_pct: Some(("2026-06-07".into(), 21.0)),
            ..Default::default()
        };
        let warnings = check_deltas(&proposed_weight(82.5), &prev, "2026-06-08", &limits());
        assert!(warnings.is_empty());
    }

    #[test]
    fn visceral_and_rmr_absolute() {
        let lim = limits();
        let ok = ProposedMetrics {
            visceral_fat_level: Some(10),
            resting_metabolism_kcal: Some(1800),
            ..Default::default()
        };
        assert!(validate_absolute(&ok, &lim).is_ok());

        let bad = ProposedMetrics {
            visceral_fat_level: Some(0),
            resting_metabolism_kcal: Some(100),
            ..Default::default()
        };
        assert_eq!(validate_absolute(&bad, &lim).unwrap_err().len(), 2);
    }

    #[test]
    fn sleep_absolute_bounds() {
        let lim = SleepSanityLimits::default();
        let ok = ProposedSleepMetrics {
            total_sleep_minutes: Some(420),
            heart_rate_bpm: Some(52.0),
            hypopnea_per_hr: Some(1.6),
            respiratory_rate: Some(14.0),
            subjective_quality: Some(7),
            sleep_score: Some(80),
            sleep_efficiency_pct: Some(85.0),
            ..Default::default()
        };
        assert!(validate_sleep_absolute(&ok, &lim).is_ok());

        let bad_hr = ProposedSleepMetrics {
            heart_rate_bpm: Some(10.0),
            ..Default::default()
        };
        let err = validate_sleep_absolute(&bad_hr, &lim).unwrap_err();
        assert!(err.iter().any(|e| e.contains("heart_rate_bpm")));

        let bad_quality = ProposedSleepMetrics {
            subjective_quality: Some(0),
            ..Default::default()
        };
        assert!(validate_sleep_absolute(&bad_quality, &lim).is_err());

        let bad_eff = ProposedSleepMetrics {
            sleep_efficiency_pct: Some(150.0),
            ..Default::default()
        };
        assert!(validate_sleep_absolute(&bad_eff, &lim).is_err());
    }

    #[test]
    fn sleep_absolute_skips_absent() {
        assert!(validate_sleep_absolute(
            &ProposedSleepMetrics::default(),
            &SleepSanityLimits::default()
        )
        .is_ok());
    }

    #[test]
    fn sleep_absolute_uses_custom_limits() {
        let mut lim = SleepSanityLimits::default();
        lim.heart_rate_bpm.min = 40.0;
        lim.heart_rate_bpm.max = 70.0;
        let ok = ProposedSleepMetrics {
            heart_rate_bpm: Some(55.0),
            ..Default::default()
        };
        assert!(validate_sleep_absolute(&ok, &lim).is_ok());
        let bad = ProposedSleepMetrics {
            heart_rate_bpm: Some(80.0),
            ..Default::default()
        };
        assert!(validate_sleep_absolute(&bad, &lim).is_err());
    }
}
