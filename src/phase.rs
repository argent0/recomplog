//! Rep phase normalization (full / eccentric / concentric).

use crate::error::{RecomplogError, Result};

pub const FULL: &str = "full";
pub const ECCENTRIC: &str = "eccentric";
pub const CONCENTRIC: &str = "concentric";

pub fn normalize_phase(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "full" | "working" => Ok(FULL),
        "eccentric" | "ecc" => Ok(ECCENTRIC),
        "concentric" | "conc" => Ok(CONCENTRIC),
        other => Err(RecomplogError::InvalidInput(format!(
            "Invalid phase '{other}'. Expected one of: full, eccentric, concentric."
        ))),
    }
}

pub fn format_phase_label(phase: &str) -> String {
    match phase {
        ECCENTRIC => "eccentric".to_string(),
        CONCENTRIC => "concentric".to_string(),
        _ => String::new(),
    }
}

pub fn format_reps_with_phase(reps: i32, phase: &str) -> String {
    match phase {
        ECCENTRIC => format!("{reps} reps (eccentric)"),
        CONCENTRIC => format!("{reps} reps (concentric)"),
        _ => format!("{reps} reps"),
    }
}

pub fn name_contains_phase_info(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains(ECCENTRIC) || lower.contains(CONCENTRIC) {
        return true;
    }
    name.split_whitespace().any(|word| {
        let token = word.trim_matches(|c: char| !c.is_alphanumeric());
        matches!(token, "ecc" | "conc")
    })
}

pub fn validate_exercise_name_phase(name: &str, allow_phase_in_name: bool) -> Result<()> {
    if allow_phase_in_name || !name_contains_phase_info(name) {
        return Ok(());
    }
    Err(RecomplogError::InvalidInput(
        "Exercise name contains rep phase information (eccentric/concentric). \
         Use one exercise per movement and tag sets with --phase full|eccentric|concentric instead. \
         Pass --allow-phase-in-name to override."
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_aliases() {
        assert_eq!(normalize_phase("ecc").unwrap(), ECCENTRIC);
        assert_eq!(normalize_phase("conc").unwrap(), CONCENTRIC);
        assert_eq!(normalize_phase("FULL").unwrap(), FULL);
        assert_eq!(normalize_phase("working").unwrap(), FULL);
    }

    #[test]
    fn rejects_unknown_phase() {
        assert!(normalize_phase("isometric").is_err());
    }

    #[test]
    fn detects_phase_words_in_exercise_names() {
        assert!(name_contains_phase_info("pistol squat (eccentric only)"));
        assert!(!name_contains_phase_info("pistol squat"));
    }
}
