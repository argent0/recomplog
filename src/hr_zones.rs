//! Pure HR zone bound helpers (DOB + optional resting HR).
//!
//! Used at FIT import and for compute-on-read zone recompute in `track_metrics`.
//! Does not call external tools — only pure math over profile inputs.

use chrono::{Datelike, NaiveDate};

use crate::models::HrZoneProfile;

/// Median of a non-empty slice of f64 values. Returns None if empty.
pub fn median_f64(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        Some(v[n / 2])
    } else {
        Some((v[n / 2 - 1] + v[n / 2]) / 2.0)
    }
}

/// Whole years of age on `on_date` given DOB `YYYY-MM-DD`.
pub fn age_years(dob: &str, on_date: NaiveDate) -> Option<u32> {
    let dob = NaiveDate::parse_from_str(dob.trim(), "%Y-%m-%d").ok()?;
    if on_date < dob {
        return None;
    }
    let mut age = (on_date.year() - dob.year()) as u32;
    let birthday_passed = (on_date.month(), on_date.day()) >= (dob.month(), dob.day());
    if !birthday_passed {
        age = age.saturating_sub(1);
    }
    Some(age)
}

/// Tanaka HRmax estimate: 208 − 0.7 × age.
pub fn hr_max_tanaka(age: u32) -> f64 {
    208.0 - 0.7 * age as f64
}

const ZONE_FRACS: [f64; 5] = [0.60, 0.70, 0.80, 0.90, 1.00];

/// Karvonen upper bounds: RHR + p × (HRmax − RHR).
pub fn zone_bounds_karvonen(hr_rest: f64, hr_max: f64) -> Option<[f64; 5]> {
    if !hr_rest.is_finite() || !hr_max.is_finite() || hr_max <= hr_rest {
        return None;
    }
    let hrr = hr_max - hr_rest;
    let mut out = [0.0; 5];
    for (i, p) in ZONE_FRACS.iter().enumerate() {
        out[i] = (hr_rest + p * hrr).round();
    }
    ensure_nondecreasing(&mut out);
    Some(out)
}

/// Percent-of-HRmax upper bounds.
pub fn zone_bounds_pct_max(hr_max: f64) -> Option<[f64; 5]> {
    if !hr_max.is_finite() || hr_max <= 0.0 {
        return None;
    }
    let mut out = [0.0; 5];
    for (i, p) in ZONE_FRACS.iter().enumerate() {
        out[i] = (p * hr_max).round();
    }
    ensure_nondecreasing(&mut out);
    Some(out)
}

fn ensure_nondecreasing(bounds: &mut [f64; 5]) {
    for i in 1..5 {
        if bounds[i] < bounds[i - 1] {
            bounds[i] = bounds[i - 1];
        }
    }
}

/// Build auto bounds from DOB + optional sleep HR samples (median RHR).
///
/// - DOB + usable median RHR → Karvonen
/// - DOB only → %HRmax
/// - Invalid age/RHR → None
pub fn resolve_auto_bounds(
    date_of_birth: &str,
    on_date: NaiveDate,
    sleep_hrs: &[f64],
) -> Option<HrZoneProfile> {
    let age = age_years(date_of_birth, on_date)?;
    if !(10..=100).contains(&age) {
        return None;
    }
    let hr_max = hr_max_tanaka(age);
    let rhr = median_f64(sleep_hrs).and_then(|m| {
        if (30.0..=100.0).contains(&m) {
            Some(m)
        } else {
            None
        }
    });

    if let Some(rest) = rhr {
        let bounds = zone_bounds_karvonen(rest, hr_max)?;
        Some(HrZoneProfile {
            date_of_birth: date_of_birth.trim().to_string(),
            resting_hr_bpm: Some(rest),
            bounds,
            method: format!(
                "Karvonen (age {}, RHR median {:.0}, HRmax {:.0})",
                age, rest, hr_max
            ),
        })
    } else {
        let bounds = zone_bounds_pct_max(hr_max)?;
        Some(HrZoneProfile {
            date_of_birth: date_of_birth.trim().to_string(),
            resting_hr_bpm: None,
            bounds,
            method: format!("%HRmax (age {}, HRmax {:.0})", age, hr_max),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd() {
        assert_eq!(median_f64(&[3.0, 1.0, 2.0]), Some(2.0));
    }

    #[test]
    fn median_even() {
        assert_eq!(median_f64(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }

    #[test]
    fn age_before_birthday() {
        let dob = "1983-07-21";
        let on = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        assert_eq!(age_years(dob, on), Some(42));
    }

    #[test]
    fn age_on_birthday() {
        let dob = "1983-07-21";
        let on = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        assert_eq!(age_years(dob, on), Some(43));
    }

    #[test]
    fn karvonen_monotonic() {
        let b = zone_bounds_karvonen(52.0, 179.0).unwrap();
        for i in 1..5 {
            assert!(b[i] >= b[i - 1], "{:?}", b);
        }
        assert!((b[4] - 179.0).abs() < 1.0);
    }

    #[test]
    fn pct_max_only() {
        let on = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        let p = resolve_auto_bounds("1983-07-21", on, &[]).unwrap();
        assert!(p.resting_hr_bpm.is_none());
        assert!(p.method.contains("%HRmax"));
        assert_eq!(p.date_of_birth, "1983-07-21");
    }

    #[test]
    fn karvonen_with_sleep() {
        let on = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        let p = resolve_auto_bounds("1983-07-21", on, &[52.0, 50.0, 54.0]).unwrap();
        assert_eq!(p.resting_hr_bpm, Some(52.0));
        assert!(p.method.contains("Karvonen"));
    }

    #[test]
    fn invalid_rhr_falls_back_to_pct() {
        let on = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        let p = resolve_auto_bounds("1983-07-21", on, &[10.0]).unwrap();
        assert!(p.resting_hr_bpm.is_none());
    }

    #[test]
    fn bad_age_none() {
        let on = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        assert!(resolve_auto_bounds("2019-01-01", on, &[52.0]).is_none());
    }
}
