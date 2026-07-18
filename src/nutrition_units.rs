//! Explicit nutrition units for products and consumptions.
//!
//! Three kinds only (stored in canonical form):
//! - **mass** — `g` (grams); aliases: gram, kg, mg, oz, …
//! - **volume** — `ml`; aliases: l, tsp, tbsp, …
//! - **package** — `unit` (one package / item / serving as labeled);
//!   aliases: units, package, bar, cup, capsule, serving, …
//!
//! Product nutrition is always stored as `g`, `ml`, or `unit`.
//! Consumptions must use a unit of the **same kind** as the product.
//!
//! **Append-only:** after insert, `consumptions.quantity` / `consumptions.unit`
//! are treated as immutable event payload. Normalize aliases at write time
//! (`normalize_to_canonical` / CLI create). Do not bulk-rewrite settled rows
//! on open or import. One-time schema migrations that rewrote units (v3/v4)
//! are gated by `user_version` and must not re-run.

use anyhow::{anyhow, Result};

/// How a product’s reference serving is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    /// Continuous mass; canonical storage `g`.
    Mass,
    /// Continuous volume; canonical storage `ml`.
    Volume,
    /// Count of whole packages/items; canonical storage `unit`.
    Package,
}

impl UnitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mass => "mass",
            Self::Volume => "volume",
            Self::Package => "package",
        }
    }

    /// Canonical unit string stored in the DB.
    pub fn canonical_unit(self) -> &'static str {
        match self {
            Self::Mass => "g",
            Self::Volume => "ml",
            Self::Package => "unit",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mass => "mass (g)",
            Self::Volume => "volume (ml)",
            Self::Package => "package (unit)",
        }
    }
}

/// A parsed unit with kind and the factor to convert quantity → base units
/// (grams for mass, millilitres for volume, count for package).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedUnit {
    pub kind: UnitKind,
    /// Multiply raw quantity by this to get base amount (g, ml, or package count).
    pub to_base: f64,
}

impl ParsedUnit {
    pub fn canonical(self) -> &'static str {
        self.kind.canonical_unit()
    }
}

/// Parse a free-form unit string into a kind + conversion factor.
pub fn parse_unit(raw: &str) -> Result<ParsedUnit> {
    let u = raw.trim().to_lowercase();
    if u.is_empty() {
        return Err(anyhow!(
            "unit is required; use g (mass), ml (volume), or unit (package)"
        ));
    }

    let (kind, to_base) = match u.as_str() {
        // --- mass → grams ---
        "g" | "gram" | "grams" => (UnitKind::Mass, 1.0),
        "kg" | "kilogram" | "kilograms" => (UnitKind::Mass, 1000.0),
        "mg" | "milligram" | "milligrams" => (UnitKind::Mass, 0.001),
        "oz" | "ounce" | "ounces" => (UnitKind::Mass, 28.349_523_125),
        "lb" | "lbs" | "pound" | "pounds" => (UnitKind::Mass, 453.592_37),

        // --- volume → millilitres ---
        "ml" | "milliliter" | "milliliters" | "millilitre" | "millilitres" => {
            (UnitKind::Volume, 1.0)
        }
        "l" | "liter" | "liters" | "litre" | "litres" => (UnitKind::Volume, 1000.0),
        "cl" => (UnitKind::Volume, 10.0),
        "dl" => (UnitKind::Volume, 100.0),
        "tsp" | "teaspoon" | "teaspoons" => (UnitKind::Volume, 5.0),
        "tbsp" | "tablespoon" | "tablespoons" => (UnitKind::Volume, 15.0),
        "floz" | "fl-oz" | "fl_oz" | "fluidounce" | "fluid-ounce" => {
            (UnitKind::Volume, 29.573_529_562_5)
        }

        // --- package / count ---
        "unit" | "units" | "package" | "packages" | "pack" | "packs" | "packet" | "packets"
        | "serving" | "servings" | "portion" | "portions" | "bar" | "bars" | "cup" | "cups"
        | "capsule" | "capsules" | "cap" | "caps" | "tablet" | "tablets" | "pill" | "pills"
        | "scoop" | "scoops" | "piece" | "pieces" | "item" | "items" | "bottle" | "bottles"
        | "can" | "cans" | "slice" | "slices" | "drink" | "drinks" | "spoon" | "spoons" => {
            (UnitKind::Package, 1.0)
        }

        other => {
            return Err(anyhow!(
                "unknown unit '{other}'. Use g (mass), ml (volume), or unit (package). \
                 Package aliases: bar, cup, capsule, serving, scoop, …"
            ));
        }
    };

    Ok(ParsedUnit { kind, to_base })
}

/// Normalize quantity+unit into base units (e.g. `0.1 kg` → `(100.0, "g")`).
pub fn normalize_to_canonical(quantity: f64, unit: &str) -> Result<(f64, String)> {
    let p = parse_unit(unit)?;
    let qty = if p.kind == UnitKind::Package {
        quantity
    } else {
        quantity * p.to_base
    };
    Ok((qty, p.canonical().to_string()))
}

/// Human-readable “per …” label for product nutrition.
pub fn format_reference_serving(quantity: f64, unit: &str) -> String {
    let kind = parse_unit(unit).map(|p| p.kind).unwrap_or(UnitKind::Mass);
    match kind {
        UnitKind::Package => {
            if (quantity - 1.0).abs() < 1e-9 {
                "per 1 unit (package)".to_string()
            } else {
                format!("per {quantity} unit (package)")
            }
        }
        UnitKind::Mass => format!("per {quantity} g"),
        UnitKind::Volume => format!("per {quantity} ml"),
    }
}

/// Validate product nutrition reference fields; returns base quantity + canonical unit.
pub fn validate_product_reference(quantity: f64, unit: &str) -> Result<(f64, String)> {
    if !quantity.is_finite() || quantity <= 0.0 {
        return Err(anyhow!(
            "reference quantity must be a finite number > 0 (got {quantity})"
        ));
    }
    let (qty, canonical) = normalize_to_canonical(quantity, unit)?;
    // Package products: strongly prefer 1 unit = one package. Allow other counts
    // (e.g. multipack) but reject obvious mistakes.
    if canonical == "unit" && qty > 1000.0 {
        return Err(anyhow!(
            "reference quantity {qty} is unusually large for unit (package); \
             did you mean reference-unit g with that many grams?"
        ));
    }
    Ok((qty, canonical))
}

/// Resolved consumption ready to store and scale.
#[derive(Debug, Clone)]
pub struct ResolvedConsumption {
    /// Quantity in the product’s canonical unit (`g` / `ml` / `unit`).
    pub quantity: f64,
    /// Always `g`, `ml`, or `unit`.
    pub unit: String,
    /// Multiplier for per-reference macros.
    pub scale: f64,
}

/// Resolve consumption unit against a product’s reference unit.
///
/// - If `consumption_unit` is `None`, defaults to the product’s reference unit.
/// - Units must be the **same kind** (mass↔mass, volume↔volume, package↔package).
/// - Quantity is converted into the canonical unit of that kind (e.g. `0.1 kg` → `100 g`).
pub fn resolve_consumption(
    quantity: f64,
    consumption_unit: Option<&str>,
    reference_quantity: f64,
    reference_unit: &str,
) -> Result<ResolvedConsumption> {
    if !quantity.is_finite() || quantity <= 0.0 {
        return Err(anyhow!(
            "consumption quantity must be a finite number > 0 (got {quantity})"
        ));
    }
    if !reference_quantity.is_finite() || reference_quantity <= 0.0 {
        return Err(anyhow!(
            "product reference quantity is invalid ({reference_quantity}); fix product nutrition"
        ));
    }

    let ref_parsed = parse_unit(reference_unit)
        .map_err(|e| anyhow!("product has invalid reference unit '{reference_unit}': {e}"))?;

    let cons_raw = match consumption_unit {
        Some(u) if !u.trim().is_empty() => u.to_string(),
        _ => ref_parsed.canonical().to_string(),
    };
    let cons_parsed = parse_unit(&cons_raw)?;

    if cons_parsed.kind != ref_parsed.kind {
        return Err(anyhow!(
            "unit mismatch: product is {} (reference {} {}), but consumption uses {} ({}). \
             Log in the same kind — e.g. --unit {} for this product.",
            ref_parsed.kind.label(),
            reference_quantity,
            ref_parsed.canonical(),
            cons_parsed.kind.label(),
            cons_raw.trim(),
            ref_parsed.canonical(),
        ));
    }

    // Store quantity in base units matching the canonical unit string.
    let qty_base = quantity * cons_parsed.to_base;
    let ref_base = reference_quantity * ref_parsed.to_base;
    if ref_base <= 0.0 {
        return Err(anyhow!("product reference base amount is zero"));
    }
    let scale = qty_base / ref_base;
    Ok(ResolvedConsumption {
        quantity: qty_base,
        unit: cons_parsed.canonical().to_string(),
        scale,
    })
}

/// Back-compat wrapper used by report aggregations.
pub fn resolve_consumption_scale(
    quantity: f64,
    consumption_unit: Option<&str>,
    reference_quantity: f64,
    reference_unit: &str,
) -> Result<(String, f64)> {
    let r = resolve_consumption(
        quantity,
        consumption_unit,
        reference_quantity,
        reference_unit,
    )?;
    Ok((r.unit, r.scale))
}

/// Scale factor only (for reports). Returns 0 for invalid inputs so historical
/// bad rows do not panic reports; prefer `resolve_consumption_scale` on write.
pub fn consumption_scale(
    quantity: f64,
    reference_quantity: f64,
    consumption_unit: Option<&str>,
    reference_unit: &str,
) -> f64 {
    resolve_consumption_scale(
        quantity,
        consumption_unit,
        reference_quantity,
        reference_unit,
    )
    .map(|(_, s)| s)
    .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_aliases_to_canonical() {
        assert_eq!(normalize_to_canonical(1.0, "grams").unwrap().1, "g");
        assert_eq!(normalize_to_canonical(1.0, "KG").unwrap().1, "g");
        assert_eq!(normalize_to_canonical(1.0, "millilitre").unwrap().1, "ml");
        assert_eq!(normalize_to_canonical(1.0, "bar").unwrap().1, "unit");
        assert_eq!(normalize_to_canonical(1.0, "capsule").unwrap().1, "unit");
        assert_eq!(normalize_to_canonical(1.0, "serving").unwrap().1, "unit");
        assert_eq!(normalize_to_canonical(1.0, "units").unwrap().1, "unit");
    }

    #[test]
    fn rejects_unknown_unit() {
        assert!(parse_unit("furlong").is_err());
        assert!(parse_unit("").is_err());
    }

    #[test]
    fn mass_scale() {
        let r = resolve_consumption(100.0, Some("g"), 46.0, "g").unwrap();
        assert_eq!(r.unit, "g");
        assert!((r.quantity - 100.0).abs() < 1e-9);
        assert!((r.scale - 100.0 / 46.0).abs() < 1e-9);
    }

    #[test]
    fn package_scale() {
        let r = resolve_consumption(1.0, Some("bar"), 1.0, "unit").unwrap();
        assert_eq!(r.unit, "unit");
        assert!((r.quantity - 1.0).abs() < 1e-9);
        assert!((r.scale - 1.0).abs() < 1e-9);
        let r2 = resolve_consumption(2.0, Some("unit"), 1.0, "unit").unwrap();
        assert!((r2.scale - 2.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_bar_against_grams() {
        let err = resolve_consumption(1.0, Some("bar"), 46.0, "g").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unit mismatch"), "{msg}");
        assert!(msg.contains("mass"), "{msg}");
        assert!(msg.contains("package"), "{msg}");
    }

    #[test]
    fn rejects_grams_against_package() {
        let err = resolve_consumption(46.0, Some("g"), 1.0, "unit").unwrap_err();
        assert!(err.to_string().contains("unit mismatch"));
    }

    #[test]
    fn defaults_unit_to_product_reference() {
        let r = resolve_consumption(50.0, None, 100.0, "g").unwrap();
        assert_eq!(r.unit, "g");
        assert!((r.scale - 0.5).abs() < 1e-9);
    }

    #[test]
    fn kg_converts_within_mass() {
        let r = resolve_consumption(0.1, Some("kg"), 100.0, "g").unwrap();
        assert_eq!(r.unit, "g");
        assert!((r.quantity - 100.0).abs() < 1e-9);
        assert!((r.scale - 1.0).abs() < 1e-9);
    }

    #[test]
    fn volume_tsp_to_ml() {
        let r = resolve_consumption(6.0, Some("teaspoon"), 30.0, "ml").unwrap();
        assert_eq!(r.unit, "ml");
        assert!((r.quantity - 30.0).abs() < 1e-9);
        assert!((r.scale - 1.0).abs() < 1e-9);
    }

    #[test]
    fn format_serving_labels() {
        assert_eq!(format_reference_serving(100.0, "g"), "per 100 g");
        assert_eq!(
            format_reference_serving(1.0, "unit"),
            "per 1 unit (package)"
        );
        assert_eq!(format_reference_serving(200.0, "ml"), "per 200 ml");
    }

    #[test]
    fn validate_product_reference_normalizes() {
        let (q, u) = validate_product_reference(46.0, "grams").unwrap();
        assert_eq!(q, 46.0);
        assert_eq!(u, "g");
        let (_, u2) = validate_product_reference(1.0, "bar").unwrap();
        assert_eq!(u2, "unit");
    }

    #[test]
    fn iron_bar_as_package_is_full_macros() {
        // Product: 1 unit = 180 kcal
        let (_, s) = resolve_consumption_scale(1.0, Some("unit"), 1.0, "unit").unwrap();
        assert!((180.0 * s - 180.0).abs() < 1e-9);
    }
}
