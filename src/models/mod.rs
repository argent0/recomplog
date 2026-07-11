use serde::{Deserialize, Serialize};

use crate::sanity::SanityWarning;
use crate::utils::TimestampInfo;

// Common success envelope for mutating operations when --json
#[derive(Serialize, Debug)]
pub struct Success {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_id: Option<i64>,
    /// Non-fatal sanity warnings (e.g. large delta vs previous measurement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<SanityWarning>>,
}

impl Success {
    pub fn created(id: i64, date: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            success: true,
            id: Some(id),
            date: Some(date.into()),
            message: Some(msg.into()),
            deleted_id: None,
            warnings: None,
        }
    }

    pub fn created_with_warnings(
        id: i64,
        date: impl Into<String>,
        msg: impl Into<String>,
        warnings: Vec<SanityWarning>,
    ) -> Self {
        let mut s = Self::created(id, date, msg);
        if !warnings.is_empty() {
            s.warnings = Some(warnings);
        }
        s
    }

    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            id: None,
            date: None,
            message: Some(msg.into()),
            deleted_id: None,
            warnings: None,
        }
    }
    pub fn deleted(id: i64) -> Self {
        Self {
            success: true,
            id: None,
            date: None,
            message: None,
            deleted_id: Some(id),
            warnings: None,
        }
    }
}

// Core domain model
#[derive(Serialize, Debug, Clone)]
pub struct Measurement {
    pub id: i64,
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight_kg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_fat_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skeletal_muscle_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visceral_fat_level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmi: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resting_metabolism_kcal: Option<i64>,
    pub created_at: TimestampInfo,
    pub updated_at: TimestampInfo,
}

// For reports: a point in a series
#[derive(Serialize, Debug, Clone)]
pub struct MeasurementPoint {
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight_kg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_fat_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skeletal_muscle_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visceral_fat_level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmi: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resting_metabolism_kcal: Option<i64>,
}

// Stats for a single metric series
#[derive(Serialize, Debug, Clone, Default)]
pub struct MetricStats {
    pub count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<String>, // "up", "down", "stable", "insufficient_data"
}

// Report for a single metric (e.g. weight)
#[derive(Serialize, Debug)]
pub struct MetricReport {
    pub metric: String,
    pub period: Period,
    pub stats: MetricStats,
    pub series: Vec<MeasurementPoint>,
}

// Summary report across all metrics
#[derive(Serialize, Debug)]
pub struct SummaryReport {
    pub period: Period,
    pub weight: Option<MetricStats>,
    pub body_fat: Option<MetricStats>,
    pub skeletal_muscle: Option<MetricStats>,
    pub visceral_fat: Option<MetricStats>,
    pub bmi: Option<MetricStats>,
    pub resting_metabolism: Option<MetricStats>,
    pub measurement_count: i64,
    /// Sleep summary for the period (present only if sleep data exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep: Option<SleepSummary>,
}

#[derive(Serialize, Debug, Clone)]
pub struct Period {
    pub since: Option<String>,
    pub until: Option<String>,
    /// Present when the period was specified as `--days N`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
}

// ---------- Nutrition report models (nutlog parity) ----------

#[derive(Serialize, Debug)]
pub struct NutritionReport {
    pub period: Period,
    pub total_consumed_items: i64,
    pub totals: MacroTotals,
    pub micronutrients: Vec<MicroTotal>,
}

#[derive(Serialize, Debug)]
pub struct NutritionDailyReport {
    pub period: Period,
    pub value: String,
    pub days: Vec<DailyNutritionEntry>,
}

#[derive(Serialize, Debug)]
pub struct DailyNutritionEntry {
    pub date: String,
    pub total_consumed_items: i64,
    pub totals: MacroTotals,
}

#[derive(Serialize, Debug, Default, Clone)]
pub struct MacroTotals {
    pub energy_kcal: Option<f64>,
    pub protein_g: Option<f64>,
    pub carbohydrates_g: Option<f64>,
    pub fat_g: Option<f64>,
    pub fiber_g: Option<f64>,
    pub sugars_g: Option<f64>,
}

#[derive(Serialize, Debug)]
pub struct MicroTotal {
    pub nutrient_id: i64,
    pub name: String,
    pub unit: String,
    pub total_amount: f64,
}

#[derive(Serialize, Debug)]
pub struct SpendingReport {
    pub period: Period,
    pub total_cents: i64,
    pub total: String,
    pub by_store: Vec<StoreSpending>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_product: Option<Vec<ProductSpending>>,
}

#[derive(Serialize, Debug)]
pub struct StoreSpending {
    pub store_id: Option<i64>,
    pub store_name: String,
    pub cents: i64,
    pub amount: String,
    pub purchase_count: i64,
}

#[derive(Serialize, Debug)]
pub struct ProductSpending {
    pub product_id: i64,
    pub product_name: String,
    pub cents: i64,
    pub amount: String,
    pub purchase_count: i64,
}

// ---------- Brief report (multi-section daily dump) ----------

/// One consumption line in `report brief` (matches consumption list JSON).
#[derive(Serialize, Debug, Clone)]
pub struct BriefConsumption {
    pub id: i64,
    pub product_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    pub quantity: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub consumed_at: String,
}

/// Compact workout row for brief report sections.
#[derive(Serialize, Debug, Clone)]
pub struct BriefWorkout {
    pub id: i64,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workout_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_feeling: Option<i64>,
}

/// Aggregate training stats for a period (HTML overview parity).
#[derive(Serialize, Debug, Clone)]
pub struct WorkoutPeriodOverview {
    pub period: Period,
    pub workout_count: i64,
    pub days_trained: i64,
    pub set_count: i64,
    pub total_volume: f64,
    /// Workouts in this period (newest first).
    pub workouts: Vec<BriefWorkout>,
}

/// Today + previous lookback for training in `report brief`.
#[derive(Serialize, Debug, Clone)]
pub struct BriefWorkouts {
    pub today: Vec<BriefWorkout>,
    /// Overview of the N calendar days before today (same N as `--days`).
    pub previous: WorkoutPeriodOverview,
}

/// Combined multi-section brief: today logs + N-day lookback lists.
#[derive(Serialize, Debug)]
pub struct BriefReport {
    /// Lookback window for nutrition / measurements / sleep (includes today).
    pub period: Period,
    pub consumption_today: Vec<BriefConsumption>,
    pub nutrition_daily: NutritionDailyReport,
    pub measurements: Vec<Measurement>,
    pub sleep: Vec<Sleep>,
    pub workouts: BriefWorkouts,
}

/// User-level profile / settings (singleton). Managed via `recomplog config ...`.
#[derive(Serialize, Debug, Clone)]
pub struct UserProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_cm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_birth: Option<String>, // stored as YYYY-MM-DD
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<TimestampInfo>,
}

// ---------- Sleep models (spec/02-sleep-logging.md) ----------

/// A sleep session record. `date` is the wake-up date (standard sleep tracker convention).
#[derive(Serialize, Debug, Clone)]
pub struct Sleep {
    pub id: i64,
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bedtime: Option<String>, // HH:MM local
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wake_time: Option<String>, // HH:MM local
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_bed_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_sleep_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rem_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awake_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_efficiency_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_score: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subjective_quality: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awakenings: Option<i64>,
    /// Average heart rate during sleep (bpm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heart_rate_bpm: Option<f64>,
    /// Hypopnea rate in events per hour (times/hr).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hypopnea_per_hr: Option<f64>,
    /// Average respiratory rate (breaths per minute).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respiratory_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub created_at: TimestampInfo,
    pub updated_at: TimestampInfo,
}

/// Lightweight point for sleep series in some reports (optional future use).
#[derive(Serialize, Debug, Clone)]
#[allow(dead_code)]
pub struct SleepPoint {
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_sleep_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_efficiency_pct: Option<f64>,
}

/// Summary averages for sleep over a period.
#[derive(Serialize, Debug, Clone, Default)]
pub struct SleepAverages {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_sleep_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rem_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awake_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efficiency_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heart_rate_bpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hypopnea_per_hr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respiratory_rate: Option<f64>,
}

/// Extreme values (best/worst) for sleep reports.
#[derive(Serialize, Debug, Clone)]
pub struct SleepExtreme {
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pct: Option<f64>,
}

/// Extremes section for sleep report.
#[derive(Serialize, Debug, Clone)]
pub struct SleepExtremes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_total_sleep: Option<SleepExtreme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worst_total_sleep: Option<SleepExtreme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_efficiency: Option<SleepExtreme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worst_efficiency: Option<SleepExtreme>,
}

/// Full sleep report returned by `recomplog report sleep`.
#[derive(Serialize, Debug)]
pub struct SleepReport {
    pub period: Period,
    pub nights_logged: i64,
    pub averages: SleepAverages,
    pub extremes: SleepExtremes,
    /// "improving" | "declining" | "stable" | "insufficient_data" etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<String>,
    /// Full records for the period (newest first or as queried; agents get rich data).
    pub nights: Vec<Sleep>,
}

/// Compact sleep summary for embedding in `report summary`.
#[derive(Serialize, Debug, Clone)]
pub struct SleepSummary {
    pub nights_logged: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_total_sleep_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_rem_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_efficiency_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_quality: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<String>,
}

// ---------- Workout models ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeartRateZones {
    #[serde(default)]
    pub z1_seconds: u32,
    #[serde(default)]
    pub z2_seconds: u32,
    #[serde(default)]
    pub z3_seconds: u32,
    #[serde(default)]
    pub z4_seconds: u32,
    #[serde(default)]
    pub z5_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lap {
    pub lap_number: u16,
    pub distance_km: f64,
    pub duration_seconds: u32,
    pub pace_min_per_km: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_heart_rate_bpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_heart_rate_bpm: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Laps(pub Vec<Lap>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exercise {
    pub id: i64,
    pub name: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muscle_groups: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equipment: Option<String>,
    pub load_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_custom: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trackpoint {
    pub recorded_at: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude_m: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    pub cadence_spm: Option<f64>,
    pub distance_km: Option<f64>,
    pub speed_m_s: Option<f64>,
}

/// Optional profile for deriving HR zones from DOB + resting HR (local DB).
#[derive(Debug, Clone)]
pub struct HrZoneProfile {
    pub date_of_birth: String,
    pub resting_hr_bpm: Option<f64>,
    pub bounds: [f64; 5],
    pub method: String,
}
