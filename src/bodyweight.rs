//! Body-mass exercise load resolution.

use crate::error::{RecomplogError, Result};
use crate::load_type;
use rusqlite::{params, Connection, OptionalExtension};

pub const NO_WEIGHT_WARNING: &str = "Warning: body weight not recorded for this set. Volume stats and load history will exclude it. Prefer --weight <kg> with your body mass in kg, or log a body measurement (`body measurement create --weight-kg ...`).";

/// Latest non-null body weight from `measurements`.
///
/// When `on_or_before` is `Some(YYYY-MM-DD)`, prefers the newest measurement on or before
/// that day (so historical workouts use the weight at the time). Falls back to overall latest
/// if nothing is on or before that date.
pub fn lookup_measured_body_weight(
    conn: &Connection,
    on_or_before: Option<&str>,
) -> Result<Option<(String, f64)>> {
    if let Some(date) = on_or_before {
        let row: Option<(String, f64)> = conn
            .query_row(
                "SELECT date, weight_kg FROM measurements \
                 WHERE weight_kg IS NOT NULL AND date <= ?1 \
                 ORDER BY date DESC LIMIT 1",
                params![date],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row = conn
        .query_row(
            "SELECT date, weight_kg FROM measurements \
             WHERE weight_kg IS NOT NULL \
             ORDER BY date DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row)
}

pub fn validate_external_load(load_type: &str, external_load: Option<f64>) -> Result<()> {
    if external_load.is_some() && !load_type::is_body_mass(load_type) {
        return Err(RecomplogError::InvalidInput(
            "--external-load is only valid for body-mass exercises (load_type=body_mass). \
             Use --weight for barbell/dumbbell load."
                .into(),
        ));
    }
    Ok(())
}

/// Resolve stored `weight_kg` / `external_load_kg` for a set.
///
/// For `body_mass` exercises when `--weight` is omitted:
/// 1. use `measured_body_weight` from body measurements (if provided), else
/// 2. require `--weight` or `--no-weight-recorded`.
pub fn resolve_bodyweight_load(
    exercise_name: &str,
    load_type: &str,
    weight: Option<f64>,
    external_load: Option<f64>,
    no_weight_recorded: bool,
    requires_body_weight: bool,
    measured_body_weight: Option<f64>,
) -> Result<(Option<f64>, Option<f64>)> {
    validate_external_load(load_type, external_load)?;

    if no_weight_recorded && weight.is_some() {
        return Err(RecomplogError::InvalidInput(
            "Cannot use --weight together with --no-weight-recorded.".into(),
        ));
    }

    if requires_body_weight && load_type::is_body_mass(load_type) {
        if !no_weight_recorded && weight.is_none() {
            if let Some(mw) = measured_body_weight {
                if mw <= 0.0 {
                    return Err(RecomplogError::InvalidInput(
                        "Body weight must be a positive value in kg.".into(),
                    ));
                }
                return Ok((Some(mw), external_load));
            }
            return Err(RecomplogError::InvalidInput(format!(
                "Exercise '{exercise_name}' (load_type=body_mass) requires --weight <kg> (your body mass), \
                 a body measurement with weight_kg (`body measurement create --weight-kg ...`), \
                 or --no-weight-recorded (not recommended; excludes set from volume stats)."
            )));
        }
        if let Some(w) = weight {
            if w <= 0.0 {
                return Err(RecomplogError::InvalidInput(
                    "Body weight must be a positive value in kg.".into(),
                ));
            }
        }
        if no_weight_recorded {
            eprintln!("{NO_WEIGHT_WARNING}");
            Ok((None, external_load))
        } else {
            Ok((weight, external_load))
        }
    } else {
        if no_weight_recorded {
            return Err(RecomplogError::InvalidInput(
                "--no-weight-recorded is only valid for body-mass exercises (load_type=body_mass)."
                    .into(),
            ));
        }
        Ok((weight, external_load))
    }
}

/// Total load for a set (body mass + external, or bar weight). Used by callers/tests.
#[allow(dead_code)]
pub fn total_load_kg(
    load_type: &str,
    weight_kg: Option<f64>,
    external_load_kg: Option<f64>,
) -> Option<f64> {
    match (load_type::is_body_mass(load_type), weight_kg) {
        (true, Some(w)) => Some(w + external_load_kg.unwrap_or(0.0)),
        (false, Some(w)) => Some(w),
        _ => None,
    }
}

/// Human-readable load string for tables (body-mass vs bar load).
pub fn format_load_display(
    load_type: &str,
    weight_kg: Option<f64>,
    external_load_kg: Option<f64>,
) -> String {
    if load_type::is_body_mass(load_type) {
        match weight_kg {
            Some(w) => {
                let mut s = format!("{:.1} kg BW", w);
                if let Some(ext) = external_load_kg {
                    if ext.abs() > f64::EPSILON {
                        if ext > 0.0 {
                            s.push_str(&format!(" +{:.1} kg", ext));
                        } else {
                            s.push_str(&format!(" {:.1} kg assist", ext));
                        }
                    }
                }
                s
            }
            None => "(body weight not recorded)".to_string(),
        }
    } else if let Some(w) = weight_kg {
        format!("{:.2} kg", w)
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_type::{BODY_MASS, EXTERNAL};

    #[test]
    fn requires_weight_for_body_mass_without_measurement() {
        let err = resolve_bodyweight_load("pull up", BODY_MASS, None, None, false, true, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("requires --weight") || err.contains("body measurement"));
    }

    #[test]
    fn uses_measured_body_weight_when_weight_omitted() {
        let (w, el) =
            resolve_bodyweight_load("pull up", BODY_MASS, None, None, false, true, Some(81.2))
                .unwrap();
        assert_eq!(w, Some(81.2));
        assert_eq!(el, None);
    }

    #[test]
    fn explicit_weight_overrides_measured() {
        let (w, _) = resolve_bodyweight_load(
            "pull up",
            BODY_MASS,
            Some(82.0),
            None,
            false,
            true,
            Some(81.2),
        )
        .unwrap();
        assert_eq!(w, Some(82.0));
    }

    #[test]
    fn measured_with_external_load() {
        let (w, el) = resolve_bodyweight_load(
            "pull up",
            BODY_MASS,
            None,
            Some(5.0),
            false,
            true,
            Some(80.0),
        )
        .unwrap();
        assert_eq!(w, Some(80.0));
        assert_eq!(el, Some(5.0));
    }

    #[test]
    fn total_load_includes_external() {
        assert_eq!(total_load_kg(BODY_MASS, Some(80.0), Some(5.0)), Some(85.0));
    }

    #[test]
    fn format_load_body_mass_with_external() {
        assert_eq!(
            format_load_display(BODY_MASS, Some(80.0), Some(5.0)),
            "80.0 kg BW +5.0 kg"
        );
    }

    #[test]
    fn format_load_body_mass_assist() {
        assert_eq!(
            format_load_display(BODY_MASS, Some(80.0), Some(-10.0)),
            "80.0 kg BW -10.0 kg assist"
        );
    }

    #[test]
    fn format_load_external_bar() {
        assert_eq!(
            format_load_display(EXTERNAL, Some(100.0), None),
            "100.00 kg"
        );
    }

    #[test]
    fn format_load_missing_bw() {
        assert_eq!(
            format_load_display(BODY_MASS, None, None),
            "(body weight not recorded)"
        );
    }
}
