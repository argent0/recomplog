//! Classify nutrient names as macronutrients vs micronutrients.
//!
//! Primary and extended macros live as columns on `product_nutritions`.
//! The `micronutrients` catalog must not include these names.

/// CLI flag to use when a name is a macronutrient (for error messages).
pub fn macro_flag_hint(name: &str) -> Option<&'static str> {
    match normalize(name).as_str() {
        "protein" => Some("--protein-g"),
        "carbohydrates" | "carbs" | "carbohydrate" => Some("--carbohydrates-g"),
        "fat" => Some("--fat-g"),
        "fiber" | "fibre" => Some("--fiber-g"),
        "sugars" | "sugar" => Some("--sugars-g"),
        "energy" | "calories" | "energy_kcal" | "kcal" => Some("--energy-kcal"),
        "saturated fat" | "saturated_fat" | "saturatedfat" => Some("--saturated-fat-g"),
        "trans fat" | "trans_fat" | "transfat" => Some("--trans-fat-g"),
        "monounsaturated fat" | "monounsaturated_fat" | "mufa" => Some("--monounsaturated-fat-g"),
        "polyunsaturated fat" | "polyunsaturated_fat" | "pufa" => Some("--polyunsaturated-fat-g"),
        "cholesterol" => Some("--cholesterol-mg"),
        "added sugars" | "added_sugars" | "added sugar" => Some("--added-sugars-g"),
        _ => None,
    }
}

/// True when `name` is a known macronutrient (not allowed in the micronutrient catalog).
pub fn is_macronutrient_name(name: &str) -> bool {
    macro_flag_hint(name).is_some()
}

/// Extended macros that may exist as legacy `product_micronutrients` rows.
/// Maps normalized name → `product_nutritions` column.
pub fn extended_macro_column(name: &str) -> Option<&'static str> {
    match normalize(name).as_str() {
        "saturated fat" | "saturated_fat" | "saturatedfat" => Some("saturated_fat_g"),
        "trans fat" | "trans_fat" | "transfat" => Some("trans_fat_g"),
        "monounsaturated fat" | "monounsaturated_fat" | "mufa" => Some("monounsaturated_fat_g"),
        "polyunsaturated fat" | "polyunsaturated_fat" | "pufa" => Some("polyunsaturated_fat_g"),
        "cholesterol" => Some("cholesterol_mg"),
        "added sugars" | "added_sugars" | "added sugar" => Some("added_sugars_g"),
        _ => None,
    }
}

/// All names treated as macros for catalog deletion (case-insensitive exact match).
pub const MACRO_CATALOG_NAMES: &[&str] = &[
    "Protein",
    "Carbohydrates",
    "Fat",
    "Fiber",
    "Sugars",
    "Saturated Fat",
    "Trans Fat",
    "Monounsaturated Fat",
    "Polyunsaturated Fat",
    "Cholesterol",
    "Added Sugars",
];

/// Extended macro columns added in schema v6.
pub const EXTENDED_MACRO_COLUMNS: &[&str] = &[
    "saturated_fat_g",
    "trans_fat_g",
    "monounsaturated_fat_g",
    "polyunsaturated_fat_g",
    "cholesterol_mg",
    "added_sugars_g",
];

fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_primary_and_extended() {
        assert!(is_macronutrient_name("Protein"));
        assert!(is_macronutrient_name("saturated fat"));
        assert!(is_macronutrient_name("Cholesterol"));
        assert!(!is_macronutrient_name("Vitamin C"));
        assert!(!is_macronutrient_name("Magnesium"));
        assert_eq!(
            extended_macro_column("Saturated Fat"),
            Some("saturated_fat_g")
        );
        assert_eq!(macro_flag_hint("Protein"), Some("--protein-g"));
    }
}
