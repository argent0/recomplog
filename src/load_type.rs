//! Exercise load_type normalization.

use crate::error::{RecomplogError, Result};

pub const BODY_MASS: &str = "body_mass";
pub const EXTERNAL: &str = "external";
pub const NONE: &str = "none";

pub fn normalize_load_type(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "body_mass" | "body-mass" | "bodymass" => Ok(BODY_MASS),
        "external" | "external_load" | "weighted" | "weight" => Ok(EXTERNAL),
        "none" | "unweighted" => Ok(NONE),
        other => Err(RecomplogError::InvalidInput(format!(
            "Invalid load type '{other}'. Expected one of: body_mass, external, none."
        ))),
    }
}

pub fn is_body_mass(load_type: &str) -> bool {
    load_type == BODY_MASS
}

/// Resolve load_type and equipment when creating an exercise.
pub fn resolve_for_new_exercise(
    category: &str,
    equipment: Option<&str>,
    load_type: Option<&str>,
) -> Result<(String, Option<String>, bool)> {
    let mut deprecated_bodyweight_equipment = false;
    let mut resolved_equipment = equipment.map(str::to_string);

    let resolved_load_type = if let Some(lt) = load_type {
        normalize_load_type(lt)?.to_string()
    } else if equipment.is_some_and(|e| e.eq_ignore_ascii_case("bodyweight")) {
        deprecated_bodyweight_equipment = true;
        resolved_equipment = None;
        BODY_MASS.to_string()
    } else {
        match category.to_ascii_lowercase().as_str() {
            "cardio" => NONE.to_string(),
            "calisthenics" | "flexibility" => BODY_MASS.to_string(),
            _ => EXTERNAL.to_string(),
        }
    };

    Ok((
        resolved_load_type,
        resolved_equipment,
        deprecated_bodyweight_equipment,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodyweight_equipment_maps_to_body_mass() {
        let (lt, eq, dep) =
            resolve_for_new_exercise("calisthenics", Some("bodyweight"), None).unwrap();
        assert_eq!(lt, BODY_MASS);
        assert!(eq.is_none());
        assert!(dep);
    }
}
