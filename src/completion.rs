//! Shell completion candidates for clap_complete dynamic completion.
//!
//! Static lists stay aligned with runtime validators (`phase`, `load_type`,
//! `nutrition_units`). Completers must never write to stdout and must fail soft.

use std::ffi::OsStr;

use clap_complete::CompletionCandidate;

/// Canonical + common alias phase values for `--phase`.
pub const PHASES: &[&str] = &["full", "eccentric", "concentric", "ecc", "conc"];

/// `--side` values (matches `value_parser` on set commands).
pub const SIDES: &[&str] = &["left", "right", "both"];

/// Canonical load types for `--load-type`.
pub const LOAD_TYPES: &[&str] = &["body_mass", "external", "none"];

/// Canonical nutrition units (`g` / `ml` / `unit`).
pub const NUTRITION_UNITS: &[&str] = &["g", "ml", "unit"];

/// Flexible calendar-day shortcuts offered alongside free-form dates.
pub const DATE_SHORTCUTS: &[&str] = &["today", "yesterday"];

/// Legacy import `--domain` values.
pub const IMPORT_DOMAINS: &[&str] = &["workout", "body", "nutrition"];

/// Filter static candidates by case-insensitive prefix of `current`.
pub fn filter_prefix(current: &OsStr, options: &[&str]) -> Vec<CompletionCandidate> {
    let Some(cur) = current.to_str() else {
        return Vec::new();
    };
    let cur_lower = cur.to_ascii_lowercase();
    options
        .iter()
        .filter(|opt| opt.to_ascii_lowercase().starts_with(cur_lower.as_str()))
        .map(|opt| CompletionCandidate::new(*opt))
        .collect()
}

pub fn complete_phase(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, PHASES)
}

pub fn complete_side(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, SIDES)
}

pub fn complete_load_type(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, LOAD_TYPES)
}

pub fn complete_nutrition_unit(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, NUTRITION_UNITS)
}

pub fn complete_date_shortcut(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, DATE_SHORTCUTS)
}

pub fn complete_import_domain(current: &OsStr) -> Vec<CompletionCandidate> {
    filter_prefix(current, IMPORT_DOMAINS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn values(cands: &[CompletionCandidate]) -> Vec<String> {
        cands
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn phase_prefix_filters() {
        let c = complete_phase(&OsString::from("ecc"));
        let v = values(&c);
        assert!(v.contains(&"eccentric".to_string()) || v.contains(&"ecc".to_string()));
        assert!(!v.iter().any(|s| s == "full"));
    }

    #[test]
    fn empty_prefix_returns_all_phases() {
        let c = complete_phase(&OsString::from(""));
        assert_eq!(values(&c).len(), PHASES.len());
    }

    #[test]
    fn unit_prefix_g() {
        let c = complete_nutrition_unit(&OsString::from("g"));
        assert_eq!(values(&c), vec!["g".to_string()]);
    }

    #[test]
    fn date_shortcuts() {
        let c = complete_date_shortcut(&OsString::from("tod"));
        assert_eq!(values(&c), vec!["today".to_string()]);
    }

    #[test]
    fn import_domain() {
        let c = complete_import_domain(&OsString::from("work"));
        assert_eq!(values(&c), vec!["workout".to_string()]);
    }
}
