//! Body-mass exercise load resolution.

use crate::error::{RecomplogError, Result};
use crate::load_type;

pub const NO_WEIGHT_WARNING: &str = "Warning: body weight not recorded for this set. Volume stats and load history will exclude it. Prefer --weight <kg> with your body mass in kg.";

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

pub fn resolve_bodyweight_load(
    exercise_name: &str,
    load_type: &str,
    weight: Option<f64>,
    external_load: Option<f64>,
    no_weight_recorded: bool,
    requires_body_weight: bool,
) -> Result<(Option<f64>, Option<f64>)> {
    validate_external_load(load_type, external_load)?;

    if no_weight_recorded && weight.is_some() {
        return Err(RecomplogError::InvalidInput(
            "Cannot use --weight together with --no-weight-recorded.".into(),
        ));
    }

    if requires_body_weight && load_type::is_body_mass(load_type) {
        if !no_weight_recorded && weight.is_none() {
            return Err(RecomplogError::InvalidInput(format!(
                "Exercise '{exercise_name}' (load_type=body_mass) requires --weight <kg> (your body mass) \
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_type::BODY_MASS;

    #[test]
    fn requires_weight_for_body_mass() {
        let err = resolve_bodyweight_load("pull up", BODY_MASS, None, None, false, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("requires --weight"));
    }

    #[test]
    fn total_load_includes_external() {
        assert_eq!(total_load_kg(BODY_MASS, Some(80.0), Some(5.0)), Some(85.0));
    }
}
