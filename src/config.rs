//! Application config file (sanity limits).
//!
//! Default location (XDG): `$XDG_CONFIG_HOME/recomplog/config.toml`
//! (usually `~/.config/recomplog/config.toml`).
//! Override with global `--config PATH`.
//!
//! - Measurement metrics use absolute ranges + optional gap-aware delta thresholds.
//! - Sleep metrics use absolute ranges only (no variation / delta checks).

use crate::error::{RecomplogError, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Full on-disk application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AppConfig {
    /// Absolute ranges (and measurement delta thresholds) for sanity checks.
    #[serde(default)]
    pub sanity: SanityLimits,
}

/// Absolute min/max range only (used for sleep fields — no variation checks).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AbsoluteLimits {
    /// Inclusive minimum for absolute hard-fail.
    pub min: f64,
    /// Inclusive maximum for absolute hard-fail.
    pub max: f64,
}

impl AbsoluteLimits {
    pub const fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    /// Structural + physical checks. Appends messages to `errors` (does not short-circuit).
    fn collect_errors(&self, field: &str, meta: MetaBounds, errors: &mut Vec<String>) {
        if !self.min.is_finite() || !self.max.is_finite() {
            errors.push(format!("{field}: min and max must be finite"));
            return; // further range checks are meaningless
        }

        if self.min > self.max {
            errors.push(format!(
                "{field}: min ({}) must be <= max ({})",
                self.min, self.max
            ));
        }

        if meta.floor_exclusive {
            if self.min <= meta.floor {
                errors.push(format!(
                    "{field}: min ({}) must be > {}",
                    self.min, meta.floor
                ));
            }
            if self.max <= meta.floor {
                errors.push(format!(
                    "{field}: max ({}) must be > {}",
                    self.max, meta.floor
                ));
            }
        } else {
            if self.min < meta.floor {
                errors.push(format!(
                    "{field}: min ({}) must be >= {}",
                    self.min, meta.floor
                ));
            }
            if self.max < meta.floor {
                errors.push(format!(
                    "{field}: max ({}) must be >= {}",
                    self.max, meta.floor
                ));
            }
        }

        if self.min > meta.ceiling {
            errors.push(format!(
                "{field}: min ({}) must be <= {}",
                self.min, meta.ceiling
            ));
        }
        if self.max > meta.ceiling {
            errors.push(format!(
                "{field}: max ({}) must be <= {}",
                self.max, meta.ceiling
            ));
        }
    }
}

/// Per-metric absolute range + gap-aware delta allowance (measurements only).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricLimits {
    /// Inclusive minimum for absolute hard-fail.
    pub min: f64,
    /// Inclusive maximum for absolute hard-fail.
    pub max: f64,
    /// Allowed absolute delta for a 1-day gap (warn if exceeded).
    pub delta_base: f64,
    /// Extra allowed delta per additional day between samples.
    pub delta_per_day: f64,
}

impl MetricLimits {
    pub const fn new(min: f64, max: f64, delta_base: f64, delta_per_day: f64) -> Self {
        Self {
            min,
            max,
            delta_base,
            delta_per_day,
        }
    }

    /// Absolute portion of these limits (for shared range checks).
    pub fn absolute(&self) -> AbsoluteLimits {
        AbsoluteLimits {
            min: self.min,
            max: self.max,
        }
    }

    /// Structural + physical checks. Appends messages to `errors` (does not short-circuit).
    fn collect_errors(&self, field: &str, meta: MetaBounds, errors: &mut Vec<String>) {
        self.absolute().collect_errors(field, meta, errors);

        if !self.delta_base.is_finite()
            || !self.delta_per_day.is_finite()
            || self.delta_base < 0.0
            || self.delta_per_day < 0.0
        {
            errors.push(format!(
                "{field}: delta_base and delta_per_day must be finite and >= 0"
            ));
        } else if self.min.is_finite() && self.max.is_finite() && self.min <= self.max {
            let span = self.max - self.min;
            if self.delta_base > span {
                errors.push(format!(
                    "{field}: delta_base ({}) must be <= (max - min) ({})",
                    self.delta_base, span
                ));
            }
        }
    }
}

/// Physical envelope for a metric's config min/max (not user-editable).
#[derive(Clone, Copy)]
struct MetaBounds {
    /// Lowest allowed value for `min` (and for `max`).
    floor: f64,
    /// If true, `min` and `max` must be strictly greater than `floor`.
    floor_exclusive: bool,
    /// Highest allowed value for `max` (and for `min`).
    ceiling: f64,
}

/// Absolute-only sanity limits for all numeric sleep fields (no deltas / variations).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SleepSanityLimits {
    #[serde(default = "default_sleep_time_in_bed")]
    pub time_in_bed_minutes: AbsoluteLimits,
    #[serde(default = "default_sleep_total_sleep")]
    pub total_sleep_minutes: AbsoluteLimits,
    #[serde(default = "default_sleep_rem")]
    pub rem_minutes: AbsoluteLimits,
    #[serde(default = "default_sleep_deep")]
    pub deep_minutes: AbsoluteLimits,
    #[serde(default = "default_sleep_light")]
    pub light_minutes: AbsoluteLimits,
    #[serde(default = "default_sleep_awake")]
    pub awake_minutes: AbsoluteLimits,
    #[serde(default = "default_sleep_efficiency")]
    pub sleep_efficiency_pct: AbsoluteLimits,
    #[serde(default = "default_sleep_score")]
    pub sleep_score: AbsoluteLimits,
    #[serde(default = "default_sleep_quality")]
    pub subjective_quality: AbsoluteLimits,
    #[serde(default = "default_sleep_awakenings")]
    pub awakenings: AbsoluteLimits,
    #[serde(default = "default_sleep_heart_rate")]
    pub heart_rate_bpm: AbsoluteLimits,
    #[serde(default = "default_sleep_hypopnea")]
    pub hypopnea_per_hr: AbsoluteLimits,
    #[serde(default = "default_sleep_respiratory_rate")]
    pub respiratory_rate: AbsoluteLimits,
}

fn default_sleep_time_in_bed() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 1440.0)
}
fn default_sleep_total_sleep() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 1440.0)
}
fn default_sleep_rem() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 720.0)
}
fn default_sleep_deep() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 720.0)
}
fn default_sleep_light() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 1440.0)
}
fn default_sleep_awake() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 1440.0)
}
fn default_sleep_efficiency() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 100.0)
}
fn default_sleep_score() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 100.0)
}
fn default_sleep_quality() -> AbsoluteLimits {
    AbsoluteLimits::new(1.0, 10.0)
}
fn default_sleep_awakenings() -> AbsoluteLimits {
    AbsoluteLimits::new(0.0, 100.0)
}
fn default_sleep_heart_rate() -> AbsoluteLimits {
    AbsoluteLimits::new(20.0, 250.0)
}
fn default_sleep_hypopnea() -> AbsoluteLimits {
    // times/hr (AHI-style); allow high values for severe OSA device readings
    AbsoluteLimits::new(0.0, 120.0)
}
fn default_sleep_respiratory_rate() -> AbsoluteLimits {
    AbsoluteLimits::new(1.0, 60.0)
}

impl Default for SleepSanityLimits {
    fn default() -> Self {
        Self {
            time_in_bed_minutes: default_sleep_time_in_bed(),
            total_sleep_minutes: default_sleep_total_sleep(),
            rem_minutes: default_sleep_rem(),
            deep_minutes: default_sleep_deep(),
            light_minutes: default_sleep_light(),
            awake_minutes: default_sleep_awake(),
            sleep_efficiency_pct: default_sleep_efficiency(),
            sleep_score: default_sleep_score(),
            subjective_quality: default_sleep_quality(),
            awakenings: default_sleep_awakenings(),
            heart_rate_bpm: default_sleep_heart_rate(),
            hypopnea_per_hr: default_sleep_hypopnea(),
            respiratory_rate: default_sleep_respiratory_rate(),
        }
    }
}

impl SleepSanityLimits {
    fn validate(&self, errors: &mut Vec<String>) {
        // Durations: 0..=7 days (generous ceiling so users can widen day-max if needed)
        let duration_meta = MetaBounds {
            floor: 0.0,
            floor_exclusive: false,
            ceiling: 10080.0,
        };
        self.time_in_bed_minutes.collect_errors(
            "sanity.sleep.time_in_bed_minutes",
            duration_meta,
            errors,
        );
        self.total_sleep_minutes.collect_errors(
            "sanity.sleep.total_sleep_minutes",
            duration_meta,
            errors,
        );
        self.rem_minutes
            .collect_errors("sanity.sleep.rem_minutes", duration_meta, errors);
        self.deep_minutes
            .collect_errors("sanity.sleep.deep_minutes", duration_meta, errors);
        self.light_minutes
            .collect_errors("sanity.sleep.light_minutes", duration_meta, errors);
        self.awake_minutes
            .collect_errors("sanity.sleep.awake_minutes", duration_meta, errors);

        self.sleep_efficiency_pct.collect_errors(
            "sanity.sleep.sleep_efficiency_pct",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 100.0,
            },
            errors,
        );
        self.sleep_score.collect_errors(
            "sanity.sleep.sleep_score",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 100.0,
            },
            errors,
        );
        self.subjective_quality.collect_errors(
            "sanity.sleep.subjective_quality",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 10.0,
            },
            errors,
        );
        self.awakenings.collect_errors(
            "sanity.sleep.awakenings",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 500.0,
            },
            errors,
        );
        self.heart_rate_bpm.collect_errors(
            "sanity.sleep.heart_rate_bpm",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: true,
                ceiling: 300.0,
            },
            errors,
        );
        self.hypopnea_per_hr.collect_errors(
            "sanity.sleep.hypopnea_per_hr",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 200.0,
            },
            errors,
        );
        self.respiratory_rate.collect_errors(
            "sanity.sleep.respiratory_rate",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: true,
                ceiling: 100.0,
            },
            errors,
        );
    }
}

/// All measurement + sleep sanity limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SanityLimits {
    #[serde(default = "default_weight_kg")]
    pub weight_kg: MetricLimits,
    #[serde(default = "default_body_fat_pct")]
    pub body_fat_pct: MetricLimits,
    #[serde(default = "default_skeletal_muscle_pct")]
    pub skeletal_muscle_pct: MetricLimits,
    #[serde(default = "default_visceral_fat_level")]
    pub visceral_fat_level: MetricLimits,
    #[serde(default = "default_bmi")]
    pub bmi: MetricLimits,
    #[serde(default = "default_resting_metabolism_kcal")]
    pub resting_metabolism_kcal: MetricLimits,
    /// Absolute-only sleep field limits (no delta / variation thresholds).
    #[serde(default)]
    pub sleep: SleepSanityLimits,
}

fn default_weight_kg() -> MetricLimits {
    MetricLimits::new(20.0, 300.0, 3.0, 0.5)
}
fn default_body_fat_pct() -> MetricLimits {
    MetricLimits::new(2.0, 70.0, 2.0, 0.3)
}
fn default_skeletal_muscle_pct() -> MetricLimits {
    MetricLimits::new(10.0, 60.0, 2.0, 0.3)
}
fn default_visceral_fat_level() -> MetricLimits {
    MetricLimits::new(1.0, 30.0, 2.0, 0.5)
}
fn default_bmi() -> MetricLimits {
    MetricLimits::new(10.0, 60.0, 1.5, 0.25)
}
fn default_resting_metabolism_kcal() -> MetricLimits {
    MetricLimits::new(800.0, 4000.0, 150.0, 20.0)
}

impl Default for SanityLimits {
    fn default() -> Self {
        Self {
            weight_kg: default_weight_kg(),
            body_fat_pct: default_body_fat_pct(),
            skeletal_muscle_pct: default_skeletal_muscle_pct(),
            visceral_fat_level: default_visceral_fat_level(),
            bmi: default_bmi(),
            resting_metabolism_kcal: default_resting_metabolism_kcal(),
            sleep: SleepSanityLimits::default(),
        }
    }
}

impl SanityLimits {
    /// Ensure each metric's limits are structurally valid and physically possible.
    pub fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();

        // weight > 0, max <= 500
        self.weight_kg.collect_errors(
            "sanity.weight_kg",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: true,
                ceiling: 500.0,
            },
            &mut errors,
        );
        // percentages 0..=100
        self.body_fat_pct.collect_errors(
            "sanity.body_fat_pct",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 100.0,
            },
            &mut errors,
        );
        self.skeletal_muscle_pct.collect_errors(
            "sanity.skeletal_muscle_pct",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 100.0,
            },
            &mut errors,
        );
        // visceral fat level 0..=59
        self.visceral_fat_level.collect_errors(
            "sanity.visceral_fat_level",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: false,
                ceiling: 59.0,
            },
            &mut errors,
        );
        // bmi > 0, max <= 100
        self.bmi.collect_errors(
            "sanity.bmi",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: true,
                ceiling: 100.0,
            },
            &mut errors,
        );
        // RMR > 0, max <= 10000
        self.resting_metabolism_kcal.collect_errors(
            "sanity.resting_metabolism_kcal",
            MetaBounds {
                floor: 0.0,
                floor_exclusive: true,
                ceiling: 10000.0,
            },
            &mut errors,
        );

        self.sleep.validate(&mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(RecomplogError::InvalidConfig(errors.join("; ")))
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        self.sanity.validate()
    }
}

/// Result of loading (or creating) the config file.
#[derive(Debug)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub path: PathBuf,
    /// True when the file did not exist and was written with defaults.
    pub created: bool,
}

/// Default config file path: `$XDG_CONFIG_HOME/recomplog/config.toml`
/// (typically `~/.config/recomplog/config.toml`).
pub fn default_config_path() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("com", "recomplog", "recomplog") {
        proj_dirs.config_dir().join("config.toml")
    } else {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        home.join(".config/recomplog/config.toml")
    }
}

/// Resolve config path from optional `--config` override.
pub fn resolve_config_path(override_path: Option<&str>) -> PathBuf {
    match override_path {
        Some(p) => PathBuf::from(p),
        None => default_config_path(),
    }
}

/// Load config from disk, or create a default file if missing.
///
/// When created, `LoadedConfig::created` is true so callers can notify the user/agent.
pub fn load_or_create(override_path: Option<&str>) -> Result<LoadedConfig> {
    let path = resolve_config_path(override_path);

    if path.exists() {
        let config = load_from_path(&path)?;
        config.validate()?;
        return Ok(LoadedConfig {
            config,
            path,
            created: false,
        });
    }

    let config = AppConfig::default();
    write_default(&path, &config)?;
    Ok(LoadedConfig {
        config,
        path,
        created: true,
    })
}

fn load_from_path(path: &Path) -> Result<AppConfig> {
    let raw = fs::read_to_string(path).map_err(|e| {
        RecomplogError::InvalidConfig(format!(
            "failed to read config at {}: {}",
            path.display(),
            e
        ))
    })?;
    toml::from_str(&raw).map_err(|e| {
        RecomplogError::InvalidConfig(format!(
            "failed to parse config at {}: {}",
            path.display(),
            e
        ))
    })
}

fn write_default(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            RecomplogError::InvalidConfig(format!(
                "failed to create config directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }
    let pretty = toml::to_string_pretty(config).map_err(|e| {
        RecomplogError::InvalidConfig(format!("failed to serialize default config: {}", e))
    })?;
    fs::write(path, pretty.as_bytes()).map_err(|e| {
        RecomplogError::InvalidConfig(format!(
            "failed to write default config at {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_or_create_writes_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let loaded = load_or_create(Some(path.to_str().unwrap())).unwrap();
        assert!(loaded.created);
        assert!(path.exists());
        assert_eq!(loaded.config, AppConfig::default());

        let loaded2 = load_or_create(Some(path.to_str().unwrap())).unwrap();
        assert!(!loaded2.created);
        assert_eq!(loaded2.config.sanity.weight_kg.min, 20.0);
    }

    #[test]
    fn load_respects_custom_limits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = AppConfig::default();
        cfg.sanity.weight_kg.min = 40.0;
        cfg.sanity.weight_kg.max = 120.0;
        write_default(&path, &cfg).unwrap();

        let loaded = load_or_create(Some(path.to_str().unwrap())).unwrap();
        assert!(!loaded.created);
        assert_eq!(loaded.config.sanity.weight_kg.min, 40.0);
        assert_eq!(loaded.config.sanity.weight_kg.max, 120.0);
    }

    #[test]
    fn invalid_min_max_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[sanity.weight_kg]
min = 100
max = 50
delta_base = 3
delta_per_day = 0.5
"#,
        )
        .unwrap();
        let err = load_or_create(Some(path.to_str().unwrap())).unwrap_err();
        assert!(err.to_string().contains("min"));
    }

    #[test]
    fn default_config_is_physically_valid() {
        AppConfig::default().validate().unwrap();
    }

    #[test]
    fn negative_weight_min_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = AppConfig::default();
        cfg.sanity.weight_kg.min = -5.0;
        write_default(&path, &cfg).unwrap();
        let err = load_or_create(Some(path.to_str().unwrap())).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("weight_kg") && msg.contains("must be > 0"),
            "got: {msg}"
        );
    }

    #[test]
    fn zero_weight_min_rejected() {
        let mut cfg = AppConfig::default();
        cfg.sanity.weight_kg.min = 0.0;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("must be > 0"));
    }

    #[test]
    fn body_fat_max_over_100_rejected() {
        let mut cfg = AppConfig::default();
        cfg.sanity.body_fat_pct.max = 150.0;
        let err = cfg.validate().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("body_fat_pct") && msg.contains("must be <= 100"),
            "got: {msg}"
        );
    }

    #[test]
    fn negative_body_fat_min_rejected() {
        let mut cfg = AppConfig::default();
        cfg.sanity.body_fat_pct.min = -1.0;
        let err = cfg.validate().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("body_fat_pct") && msg.contains("must be >= 0"),
            "got: {msg}"
        );
    }

    #[test]
    fn tight_valid_custom_range_ok() {
        let mut cfg = AppConfig::default();
        cfg.sanity.weight_kg.min = 40.0;
        cfg.sanity.weight_kg.max = 120.0;
        cfg.sanity.weight_kg.delta_base = 3.0;
        cfg.validate().unwrap();
    }

    #[test]
    fn collects_multiple_physical_errors() {
        let mut cfg = AppConfig::default();
        cfg.sanity.weight_kg.min = -1.0;
        cfg.sanity.body_fat_pct.max = 150.0;
        let err = cfg.validate().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("weight_kg"), "got: {msg}");
        assert!(msg.contains("body_fat_pct"), "got: {msg}");
    }

    #[test]
    fn delta_base_larger_than_span_rejected() {
        let mut cfg = AppConfig::default();
        cfg.sanity.weight_kg.min = 80.0;
        cfg.sanity.weight_kg.max = 82.0;
        cfg.sanity.weight_kg.delta_base = 5.0; // > span of 2
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("delta_base"), "got: {}", err);
    }

    #[test]
    fn sleep_defaults_present_and_valid() {
        let cfg = AppConfig::default();
        cfg.validate().unwrap();
        assert_eq!(cfg.sanity.sleep.heart_rate_bpm.min, 20.0);
        assert_eq!(cfg.sanity.sleep.heart_rate_bpm.max, 250.0);
        assert_eq!(cfg.sanity.sleep.subjective_quality.min, 1.0);
        assert_eq!(cfg.sanity.sleep.subjective_quality.max, 10.0);
    }

    #[test]
    fn sleep_invalid_range_rejected() {
        let mut cfg = AppConfig::default();
        cfg.sanity.sleep.heart_rate_bpm.min = 100.0;
        cfg.sanity.sleep.heart_rate_bpm.max = 50.0;
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("heart_rate_bpm") && err.to_string().contains("min"),
            "got: {}",
            err
        );
    }

    #[test]
    fn sleep_section_loaded_from_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[sanity.weight_kg]
min = 20
max = 300
delta_base = 3
delta_per_day = 0.5

[sanity.sleep.heart_rate_bpm]
min = 40
max = 90

[sanity.sleep.hypopnea_per_hr]
min = 0
max = 50
"#,
        )
        .unwrap();
        let loaded = load_or_create(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(loaded.config.sanity.sleep.heart_rate_bpm.min, 40.0);
        assert_eq!(loaded.config.sanity.sleep.heart_rate_bpm.max, 90.0);
        assert_eq!(loaded.config.sanity.sleep.hypopnea_per_hr.max, 50.0);
        // unspecified sleep fields still get defaults
        assert_eq!(loaded.config.sanity.sleep.respiratory_rate.min, 1.0);
    }
}
