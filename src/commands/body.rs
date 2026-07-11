use crate::cli::{
    BodyReportAction, CheckArgs, CreateMeasurementArgs, DeleteArgs, ListArgs, ProfileAction,
    ProfileSetArgs, ReportRangeArgs, ShowArgs, SleepAction, SleepCreateArgs, SleepUpdateArgs,
    SummaryArgs, UpdateMeasurementArgs,
};
use crate::config::SanityLimits;
use crate::error::{RecomplogError, Result};
use crate::models::{
    HeartRateZones, Lap, Measurement, MeasurementPoint, MetricReport, MetricStats, Period, Sleep,
    SleepAverages, SleepExtreme, SleepExtremes, SleepReport, SleepSummary, Success, SummaryReport,
};
use crate::repository::body::SetAuditRow;
use crate::repository::BodyRepository as Repository;
use crate::sanity::{
    check_deltas, collect_set_metric_errors, validate_absolute, validate_sleep_absolute,
    PreviousMetrics, ProposedMetrics, ProposedSetMetrics, ProposedSleepMetrics, SanityWarning,
};
use crate::utils::{format_local, parse_date_to_ymd, resolve_date_range};
use crate::utils::{format_minutes, parse_duration_to_minutes, print_table};
use serde::Serialize;

/// Print pretty JSON (for --json paths).
fn print_json<T: serde::Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

fn print_error_json(err: &str) {
    #[derive(serde::Serialize)]
    struct ErrOut {
        success: bool,
        error: String,
    }
    print_json(&ErrOut {
        success: false,
        error: err.to_string(),
    });
}

fn quiet_print(msg: &str, quiet: bool) {
    if !quiet {
        println!("{}", msg);
    }
}

/// Resolve a Show/Update/Delete identifier (id or --date) into a usable form.
/// Returns (Some(id), None) or (None, Some(ymd)).
fn resolve_identifier(
    id: Option<i64>,
    date: Option<String>,
) -> Result<(Option<i64>, Option<String>)> {
    match (id, date) {
        (Some(i), None) => Ok((Some(i), None)),
        (None, Some(d)) => {
            let ymd =
                parse_date_to_ymd(&d).map_err(|e| RecomplogError::InvalidDate(e.to_string()))?;
            Ok((None, Some(ymd)))
        }
        (Some(_), Some(_)) => Err(RecomplogError::Other(
            "provide either an ID or --date, not both".to_string(),
        )),
        (None, None) => Err(RecomplogError::Other(
            "provide a measurement ID or --date".to_string(),
        )),
    }
}

// ---------- MEASUREMENT HANDLERS ----------

pub fn handle_measurement(
    repo: &mut Repository,
    action: crate::cli::MeasurementAction,
    limits: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        crate::cli::MeasurementAction::Create(args) => {
            handle_create(repo, args, limits, json, quiet)
        }
        crate::cli::MeasurementAction::List(args) => handle_list(repo, args, json, quiet),
        crate::cli::MeasurementAction::Show(args) => handle_show(repo, args, json),
        crate::cli::MeasurementAction::Update(args) => {
            handle_update(repo, args, limits, json, quiet)
        }
        crate::cli::MeasurementAction::Delete(args) => handle_delete(repo, args, json, quiet),
    }
}

fn proposed_from_args(
    weight_kg: Option<f64>,
    body_fat_pct: Option<f64>,
    skeletal_muscle_pct: Option<f64>,
    visceral_fat_level: Option<i64>,
    bmi: Option<f64>,
    resting_metabolism_kcal: Option<i64>,
) -> ProposedMetrics {
    ProposedMetrics {
        weight_kg,
        body_fat_pct,
        skeletal_muscle_pct,
        visceral_fat_level,
        bmi,
        resting_metabolism_kcal,
    }
}

/// Absolute hard-fail, then optional delta warnings. Returns warnings (possibly empty).
fn run_measurement_sanity(
    repo: &Repository,
    date: &str,
    proposed: &ProposedMetrics,
    limits: &SanityLimits,
    skip_delta: bool,
) -> Result<Vec<SanityWarning>> {
    if let Err(errors) = validate_absolute(proposed, limits) {
        return Err(RecomplogError::InvalidMeasurement(errors.join("; ")));
    }

    if skip_delta {
        return Ok(Vec::new());
    }

    let previous = repo.get_previous_metric_values(date)?;
    Ok(check_deltas(proposed, &previous, date, limits))
}

fn emit_sanity_warnings(warnings: &[SanityWarning]) {
    for w in warnings {
        eprintln!("Warning: {}", w.message);
    }
}

// ---------- CHECK (scan DB for limit / variation violations) ----------

/// One finding from `recomplog check`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckViolation {
    /// "measurement", "sleep", or "set".
    pub entity: String,
    /// "absolute" (hard limit) or "delta" (variation; measurements only).
    pub kind: String,
    pub id: i64,
    pub date: String,
    pub field: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_delta: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_gap: Option<i64>,
    /// Exercise name for set violations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exercise: Option<String>,
}

/// Report returned by `recomplog check` (JSON and human summary source).
#[derive(Debug, Serialize)]
pub struct CheckReport {
    /// True when no violations of the enabled checks were found.
    pub ok: bool,
    pub measurement_count: i64,
    pub sleep_count: i64,
    /// Number of exercise sets scanned in the date window.
    pub set_count: i64,
    /// Whether measurement variation checks were run (`--variations`).
    /// Sleep and sets are absolute-only; this flag does not apply to them.
    pub checked_variations: bool,
    pub hard_violation_count: i64,
    pub variation_violation_count: i64,
    pub violations: Vec<CheckViolation>,
}

fn proposed_from_measurement(m: &Measurement) -> ProposedMetrics {
    ProposedMetrics {
        weight_kg: m.weight_kg,
        body_fat_pct: m.body_fat_pct,
        skeletal_muscle_pct: m.skeletal_muscle_pct,
        visceral_fat_level: m.visceral_fat_level,
        bmi: m.bmi,
        resting_metabolism_kcal: m.resting_metabolism_kcal,
    }
}

fn absorb_into_previous(prev: &mut PreviousMetrics, m: &Measurement) {
    if let Some(v) = m.weight_kg {
        prev.weight_kg = Some((m.date.clone(), v));
    }
    if let Some(v) = m.body_fat_pct {
        prev.body_fat_pct = Some((m.date.clone(), v));
    }
    if let Some(v) = m.skeletal_muscle_pct {
        prev.skeletal_muscle_pct = Some((m.date.clone(), v));
    }
    if let Some(v) = m.visceral_fat_level {
        prev.visceral_fat_level = Some((m.date.clone(), v as f64));
    }
    if let Some(v) = m.bmi {
        prev.bmi = Some((m.date.clone(), v));
    }
    if let Some(v) = m.resting_metabolism_kcal {
        prev.resting_metabolism_kcal = Some((m.date.clone(), v as f64));
    }
}

fn field_value(m: &Measurement, field: &str) -> Option<f64> {
    match field {
        "weight_kg" => m.weight_kg,
        "body_fat_pct" => m.body_fat_pct,
        "skeletal_muscle_pct" => m.skeletal_muscle_pct,
        "visceral_fat_level" => m.visceral_fat_level.map(|v| v as f64),
        "bmi" => m.bmi,
        "resting_metabolism_kcal" => m.resting_metabolism_kcal.map(|v| v as f64),
        _ => None,
    }
}

fn proposed_from_sleep(s: &Sleep) -> ProposedSleepMetrics {
    ProposedSleepMetrics {
        time_in_bed_minutes: s.time_in_bed_minutes,
        total_sleep_minutes: s.total_sleep_minutes,
        rem_minutes: s.rem_minutes,
        deep_minutes: s.deep_minutes,
        light_minutes: s.light_minutes,
        awake_minutes: s.awake_minutes,
        sleep_efficiency_pct: s.sleep_efficiency_pct,
        sleep_score: s.sleep_score,
        subjective_quality: s.subjective_quality,
        awakenings: s.awakenings,
        heart_rate_bpm: s.heart_rate_bpm,
        hypopnea_per_hr: s.hypopnea_per_hr,
        respiratory_rate: s.respiratory_rate,
    }
}

fn sleep_field_value(s: &Sleep, field: &str) -> Option<f64> {
    match field {
        "time_in_bed_minutes" => s.time_in_bed_minutes.map(|v| v as f64),
        "total_sleep_minutes" => s.total_sleep_minutes.map(|v| v as f64),
        "rem_minutes" => s.rem_minutes.map(|v| v as f64),
        "deep_minutes" => s.deep_minutes.map(|v| v as f64),
        "light_minutes" => s.light_minutes.map(|v| v as f64),
        "awake_minutes" => s.awake_minutes.map(|v| v as f64),
        "sleep_efficiency_pct" => s.sleep_efficiency_pct,
        "sleep_score" => s.sleep_score.map(|v| v as f64),
        "subjective_quality" => s.subjective_quality.map(|v| v as f64),
        "awakenings" => s.awakenings.map(|v| v as f64),
        "heart_rate_bpm" => s.heart_rate_bpm,
        "hypopnea_per_hr" => s.hypopnea_per_hr,
        "respiratory_rate" => s.respiratory_rate,
        _ => None,
    }
}

/// Map a stored set row into write-path proposed metrics.
/// Invalid heart_rate_zones / laps JSON is skipped so numeric outliers still surface.
fn proposed_from_set_row(row: &SetAuditRow) -> ProposedSetMetrics {
    let heart_rate_zones = row
        .heart_rate_zones
        .as_deref()
        .and_then(|s| serde_json::from_str::<HeartRateZones>(s).ok());
    let laps = row
        .laps
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<Lap>>(s).ok());
    ProposedSetMetrics {
        reps: row.reps,
        weight_kg: row.weight_kg,
        external_load_kg: row.external_load_kg,
        distance_km: row.distance_km,
        duration_seconds: row.duration_seconds,
        rpe: row.rpe,
        rir: row.rir,
        effective_reps: row.effective_reps,
        rest_seconds: row.rest_seconds,
        avg_heart_rate_bpm: row.avg_heart_rate_bpm,
        max_heart_rate_bpm: row.max_heart_rate_bpm,
        avg_pace_min_per_km: row.avg_pace_min_per_km,
        calories_burned: row.calories_burned,
        avg_cadence_spm: row.avg_cadence_spm,
        total_ascent_m: row.total_ascent_m,
        total_descent_m: row.total_descent_m,
        heart_rate_zones,
        laps,
    }
}

fn set_field_value(row: &SetAuditRow, field: &str) -> Option<f64> {
    match field {
        "reps" => row.reps.map(|v| v as f64),
        "weight_kg" => row.weight_kg,
        "external_load_kg" => row.external_load_kg,
        "distance_km" => row.distance_km,
        "duration_seconds" => row.duration_seconds.map(|v| v as f64),
        "rpe" => row.rpe,
        "rir" => row.rir,
        "effective_reps" => row.effective_reps.map(|v| v as f64),
        "rest_seconds" => row.rest_seconds.map(|v| v as f64),
        "avg_heart_rate_bpm" => row.avg_heart_rate_bpm,
        "max_heart_rate_bpm" => row.max_heart_rate_bpm,
        "avg_pace_min_per_km" => row.avg_pace_min_per_km,
        "calories_burned" => row.calories_burned.map(|v| v as f64),
        "avg_cadence_spm" => row.avg_cadence_spm,
        "total_ascent_m" => row.total_ascent_m,
        "total_descent_m" => row.total_descent_m,
        _ => None,
    }
}

/// Parse absolute-range error messages like `weight_kg 825 is outside allowed range 20–300`
/// into a field name when possible.
fn field_from_absolute_message(msg: &str) -> String {
    msg.split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_string()
}

pub fn handle_check(
    repo: &mut Repository,
    args: CheckArgs,
    limits: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let (since, until) =
        resolve_date_range(args.since.as_deref(), args.until.as_deref(), args.days)
            .map_err(|e| RecomplogError::InvalidDate(e.to_string()))?;

    let mut measurements = repo.list_measurements(since.as_deref(), until.as_deref())?;
    // Walk oldest → newest so variation checks see true chronological previous values.
    measurements.sort_by(|a, b| a.date.cmp(&b.date));

    let mut violations: Vec<CheckViolation> = Vec::new();
    let mut previous = PreviousMetrics::default();

    for m in &measurements {
        let proposed = proposed_from_measurement(m);

        if let Err(msgs) = validate_absolute(&proposed, limits) {
            for msg in msgs {
                let field = field_from_absolute_message(&msg);
                violations.push(CheckViolation {
                    entity: "measurement".to_string(),
                    kind: "absolute".to_string(),
                    id: m.id,
                    date: m.date.clone(),
                    field: field.clone(),
                    message: msg,
                    value: field_value(m, &field),
                    previous_value: None,
                    previous_date: None,
                    allowed_delta: None,
                    days_gap: None,
                    exercise: None,
                });
            }
        }

        if args.variations {
            let warnings = check_deltas(&proposed, &previous, &m.date, limits);
            for w in warnings {
                violations.push(CheckViolation {
                    entity: "measurement".to_string(),
                    kind: "delta".to_string(),
                    id: m.id,
                    date: m.date.clone(),
                    field: w.field,
                    message: w.message,
                    value: w.new_value,
                    previous_value: w.previous_value,
                    previous_date: w.previous_date,
                    allowed_delta: w.allowed_delta,
                    days_gap: w.days_gap,
                    exercise: None,
                });
            }
        }

        absorb_into_previous(&mut previous, m);
    }

    // Sleep: absolute limits only (no variation checks by design).
    let sleeps = repo.list_sleeps(since.as_deref(), until.as_deref())?;
    for s in &sleeps {
        let proposed = proposed_from_sleep(s);
        if let Err(msgs) = validate_sleep_absolute(&proposed, &limits.sleep) {
            for msg in msgs {
                let field = field_from_absolute_message(&msg);
                violations.push(CheckViolation {
                    entity: "sleep".to_string(),
                    kind: "absolute".to_string(),
                    id: s.id,
                    date: s.date.clone(),
                    field: field.clone(),
                    message: msg,
                    value: sleep_field_value(s, &field),
                    previous_value: None,
                    previous_date: None,
                    allowed_delta: None,
                    days_gap: None,
                    exercise: None,
                });
            }
        }
    }

    // Exercise sets: absolute limits only (no variation checks by design).
    // Date window is the parent workout session day, not set created_at.
    let sets = repo.list_exercise_sets_for_check(since.as_deref(), until.as_deref())?;
    for row in &sets {
        let proposed = proposed_from_set_row(row);
        let msgs = collect_set_metric_errors(&proposed, &limits.workout);
        for msg in msgs {
            let field = field_from_absolute_message(&msg);
            violations.push(CheckViolation {
                entity: "set".to_string(),
                kind: "absolute".to_string(),
                id: row.id,
                date: row.workout_date.clone(),
                field: field.clone(),
                message: msg,
                value: set_field_value(row, &field),
                previous_value: None,
                previous_date: None,
                allowed_delta: None,
                days_gap: None,
                exercise: Some(row.exercise_name.clone()),
            });
        }
    }

    let hard_violation_count = violations.iter().filter(|v| v.kind == "absolute").count() as i64;
    let variation_violation_count = violations.iter().filter(|v| v.kind == "delta").count() as i64;
    let report = CheckReport {
        ok: violations.is_empty(),
        measurement_count: measurements.len() as i64,
        sleep_count: sleeps.len() as i64,
        set_count: sets.len() as i64,
        checked_variations: args.variations,
        hard_violation_count,
        variation_violation_count,
        violations,
    };

    if json {
        print_json(&report);
    } else if !quiet {
        print_check_human(&report);
    } else {
        // quiet: one line status
        if report.ok {
            println!("ok");
        } else {
            println!(
                "fail hard={} variations={}",
                report.hard_violation_count, report.variation_violation_count
            );
        }
    }

    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn print_check_human(report: &CheckReport) {
    println!(
        "Checked {} measurement(s), {} sleep entr(y/ies), and {} set(s).{}",
        report.measurement_count,
        report.sleep_count,
        report.set_count,
        if report.checked_variations {
            " (measurement variations enabled; sleep and sets are absolute-only)"
        } else {
            " (hard limits only; pass --variations for measurement deltas)"
        }
    );
    if report.ok {
        println!("OK — no violations found.");
        return;
    }

    println!(
        "Found {} hard-limit violation(s), {} variation violation(s):",
        report.hard_violation_count, report.variation_violation_count
    );
    for v in &report.violations {
        if let Some(ref ex) = v.exercise {
            println!(
                "  [{}] {} {} (id {}, {}) {}: {}",
                v.kind, v.entity, v.date, v.id, ex, v.field, v.message
            );
        } else {
            println!(
                "  [{}] {} {} (id {}) {}: {}",
                v.kind, v.entity, v.date, v.id, v.field, v.message
            );
        }
    }
}

fn handle_create(
    repo: &mut Repository,
    args: CreateMeasurementArgs,
    limits: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let date =
        parse_date_to_ymd(&args.date).map_err(|e| RecomplogError::InvalidDate(e.to_string()))?;

    // Warn (but do not hard fail) if no metrics provided.
    let has_any = args.weight_kg.is_some()
        || args.body_fat_pct.is_some()
        || args.skeletal_muscle_pct.is_some()
        || args.visceral_fat_level.is_some()
        || args.bmi.is_some()
        || args.resting_metabolism_kcal.is_some();

    if !has_any {
        // Per spec: warn but do not hard-fail. Still allow the (mostly-empty) record.
        if json {
            // For JSON we still proceed; the success message will be returned.
        } else {
            eprintln!(
                "Warning: no metrics provided. Logging an empty measurement for {}.",
                date
            );
        }
    }

    let proposed = proposed_from_args(
        args.weight_kg,
        args.body_fat_pct,
        args.skeletal_muscle_pct,
        args.visceral_fat_level,
        args.bmi,
        args.resting_metabolism_kcal,
    );

    let warnings =
        match run_measurement_sanity(repo, &date, &proposed, limits, args.no_sanity_check) {
            Ok(w) => w,
            Err(e) => {
                if json {
                    print_error_json(&e.to_string());
                    std::process::exit(1);
                } else {
                    return Err(e);
                }
            }
        };

    match repo.create_measurement(
        &date,
        args.weight_kg,
        args.body_fat_pct,
        args.skeletal_muscle_pct,
        args.visceral_fat_level,
        args.bmi,
        args.resting_metabolism_kcal,
    ) {
        Ok(id) => {
            emit_sanity_warnings(&warnings);
            let msg = format!("Measurement logged for {}", date);
            if json {
                print_json(&Success::created_with_warnings(id, date, msg, warnings));
            } else {
                quiet_print(&msg, quiet);
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_error_json(&e.to_string());
                std::process::exit(1);
            } else {
                // Let the error bubble for consistent top-level "Error: ..." + non-zero
                Err(e)
            }
        }
    }
}

fn handle_list(repo: &mut Repository, args: ListArgs, json: bool, quiet: bool) -> Result<()> {
    let (since, until) =
        resolve_date_range(args.since.as_deref(), args.until.as_deref(), args.days)
            .map_err(|e| RecomplogError::InvalidDate(e.to_string()))?;
    let measurements = repo.list_measurements(since.as_deref(), until.as_deref())?;

    if json {
        print_json(&measurements);
        return Ok(());
    }

    if quiet {
        for m in &measurements {
            // Minimal machine-friendly: id|date|weight|bf|sm|vf|bmi|rm
            println!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                m.id,
                m.date,
                m.weight_kg.map(|v| v.to_string()).unwrap_or_default(),
                m.body_fat_pct.map(|v| v.to_string()).unwrap_or_default(),
                m.skeletal_muscle_pct
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                m.visceral_fat_level
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                m.bmi.map(|v| v.to_string()).unwrap_or_default(),
                m.resting_metabolism_kcal
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            );
        }
        return Ok(());
    }

    // Human table
    if measurements.is_empty() {
        println!("(no measurements)");
        return Ok(());
    }

    let rows: Vec<Vec<String>> = measurements
        .iter()
        .map(|m| {
            vec![
                m.id.to_string(),
                m.date.clone(),
                opt_f64(m.weight_kg),
                opt_f64(m.body_fat_pct),
                opt_f64(m.skeletal_muscle_pct),
                opt_i64(m.visceral_fat_level),
                opt_f64(m.bmi),
                opt_i64(m.resting_metabolism_kcal),
            ]
        })
        .collect();
    print_table(
        vec![
            "ID",
            "Date",
            "Weight (kg)",
            "Body Fat %",
            "Muscle %",
            "Visceral",
            "BMI",
            "RMR (kcal)",
        ],
        rows,
    );
    Ok(())
}

fn opt_f64(v: Option<f64>) -> String {
    v.map(|x| format!("{:.1}", x)).unwrap_or_default()
}
fn opt_i64(v: Option<i64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

fn handle_show(repo: &mut Repository, args: ShowArgs, json: bool) -> Result<()> {
    let (id, date) = resolve_identifier(args.id, args.date)?;

    let m: Measurement = if let Some(i) = id {
        repo.get_measurement(i)?
    } else if let Some(d) = date {
        repo.get_measurement_by_date(&d)?
    } else {
        unreachable!()
    };

    if json {
        print_json(&m);
    } else {
        println!("Measurement {} ({})", m.id, m.date);
        if let Some(v) = m.weight_kg {
            println!("  weight_kg: {}", v);
        }
        if let Some(v) = m.body_fat_pct {
            println!("  body_fat_pct: {}", v);
        }
        if let Some(v) = m.skeletal_muscle_pct {
            println!("  skeletal_muscle_pct: {}", v);
        }
        if let Some(v) = m.visceral_fat_level {
            println!("  visceral_fat_level: {}", v);
        }
        if let Some(v) = m.bmi {
            println!("  bmi: {}", v);
        }
        if let Some(v) = m.resting_metabolism_kcal {
            println!("  resting_metabolism_kcal: {}", v);
        }
        println!(
            "  created: {}",
            crate::utils::format_local(&m.created_at.utc)
        );
        println!(
            "  updated: {}",
            crate::utils::format_local(&m.updated_at.utc)
        );
    }
    Ok(())
}

fn handle_update(
    repo: &mut Repository,
    args: UpdateMeasurementArgs,
    limits: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let (id, date) = resolve_identifier(args.id, args.date)?;

    let has_any = args.weight_kg.is_some()
        || args.body_fat_pct.is_some()
        || args.skeletal_muscle_pct.is_some()
        || args.visceral_fat_level.is_some()
        || args.bmi.is_some()
        || args.resting_metabolism_kcal.is_some();

    if !has_any {
        let msg = "no fields to update (provide at least one metric flag)";
        if json {
            print_error_json(msg);
        } else {
            eprintln!("{}", msg);
        }
        std::process::exit(1);
    }

    // Resolve target date for sanity checks before writing.
    let target_date = if let Some(i) = id {
        repo.get_measurement(i)?.date
    } else if let Some(ref d) = date {
        d.clone()
    } else {
        unreachable!()
    };

    let proposed = proposed_from_args(
        args.weight_kg,
        args.body_fat_pct,
        args.skeletal_muscle_pct,
        args.visceral_fat_level,
        args.bmi,
        args.resting_metabolism_kcal,
    );

    let warnings =
        match run_measurement_sanity(repo, &target_date, &proposed, limits, args.no_sanity_check) {
            Ok(w) => w,
            Err(e) => {
                if json {
                    print_error_json(&e.to_string());
                    std::process::exit(1);
                } else {
                    return Err(e);
                }
            }
        };

    let updated_id = if let Some(i) = id {
        repo.update_measurement(
            i,
            args.weight_kg,
            args.body_fat_pct,
            args.skeletal_muscle_pct,
            args.visceral_fat_level,
            args.bmi,
            args.resting_metabolism_kcal,
        )?;
        i
    } else if let Some(d) = date {
        repo.update_measurement_by_date(
            &d,
            args.weight_kg,
            args.body_fat_pct,
            args.skeletal_muscle_pct,
            args.visceral_fat_level,
            args.bmi,
            args.resting_metabolism_kcal,
        )?
    } else {
        unreachable!()
    };

    // Fetch the (now updated) record to know the canonical date for the response
    let m = repo.get_measurement(updated_id)?;
    let msg = format!("Updated measurement {} ({})", updated_id, m.date);

    emit_sanity_warnings(&warnings);

    if json {
        print_json(&Success::created_with_warnings(
            updated_id, m.date, msg, warnings,
        ));
    } else {
        quiet_print(&msg, quiet);
    }
    Ok(())
}

fn handle_delete(repo: &mut Repository, args: DeleteArgs, json: bool, quiet: bool) -> Result<()> {
    let (id, date) = resolve_identifier(args.id, args.date)?;

    let deleted_id = if let Some(i) = id {
        repo.delete_measurement(i)?
    } else if let Some(d) = date {
        repo.delete_measurement_by_date(&d)?
    } else {
        unreachable!()
    };

    if json {
        print_json(&Success::deleted(deleted_id));
    } else {
        let msg = format!("Deleted measurement {}", deleted_id);
        quiet_print(&msg, quiet);
    }
    Ok(())
}

// ---------- REPORT HANDLERS ----------

pub fn handle_body_report(
    repo: &mut Repository,
    action: BodyReportAction,
    json: bool,
    _quiet: bool,
) -> Result<()> {
    match action {
        BodyReportAction::Weight(args) => {
            handle_metric_report(repo, "weight_kg", "weight", args, json)
        }
        BodyReportAction::BodyFat(args) => {
            handle_metric_report(repo, "body_fat_pct", "body_fat", args, json)
        }
        BodyReportAction::Muscle(args) => {
            handle_metric_report(repo, "skeletal_muscle_pct", "skeletal_muscle", args, json)
        }
        BodyReportAction::VisceralFat(args) => {
            handle_metric_report(repo, "visceral_fat_level", "visceral_fat", args, json)
        }
        BodyReportAction::Bmi(args) => handle_metric_report(repo, "bmi", "bmi", args, json),
        BodyReportAction::RestingMetabolism(args) => handle_metric_report(
            repo,
            "resting_metabolism_kcal",
            "resting_metabolism",
            args,
            json,
        ),
        BodyReportAction::Summary(args) | BodyReportAction::List(args) => {
            handle_summary(repo, args, json)
        }
    }
}

pub fn handle_sleep_report_cmd(
    repo: &mut Repository,
    args: ReportRangeArgs,
    json: bool,
) -> Result<()> {
    handle_sleep_report(repo, args, json)
}

fn handle_metric_report(
    repo: &mut Repository,
    field: &str,
    metric_name: &str,
    args: ReportRangeArgs,
    json: bool,
) -> Result<()> {
    let (since, until) = resolve_report_range(&args)?;
    let series = repo.get_measurements_for_report(since.as_deref(), until.as_deref())?;

    // Filter to points that have the metric, and build stats
    let points: Vec<MeasurementPoint> = series; // already in ASC date order
    let values: Vec<(String, f64)> = points
        .iter()
        .filter_map(|p| {
            let v = match field {
                "weight_kg" => p.weight_kg,
                "body_fat_pct" => p.body_fat_pct,
                "skeletal_muscle_pct" => p.skeletal_muscle_pct,
                "visceral_fat_level" => p.visceral_fat_level.map(|x| x as f64),
                "bmi" => p.bmi,
                "resting_metabolism_kcal" => p.resting_metabolism_kcal.map(|x| x as f64),
                _ => None,
            };
            v.map(|val| (p.date.clone(), val))
        })
        .collect();

    let stats = compute_stats(&values);
    let report = MetricReport {
        metric: metric_name.to_string(),
        period: Period { since, until },
        stats,
        series: points,
    };

    if json {
        print_json(&report);
    } else {
        print_metric_report_human(&report);
    }
    Ok(())
}

fn handle_summary(repo: &mut Repository, args: SummaryArgs, json: bool) -> Result<()> {
    let (since, until) = resolve_summary_range(&args)?;
    let series = repo.get_measurements_for_report(since.as_deref(), until.as_deref())?;

    let measurement_count = series.len() as i64;

    let weight = compute_metric_stats(&series, |p| p.weight_kg);
    let body_fat = compute_metric_stats(&series, |p| p.body_fat_pct);
    let skeletal_muscle = compute_metric_stats(&series, |p| p.skeletal_muscle_pct);
    let visceral_fat = compute_metric_stats(&series, |p| p.visceral_fat_level.map(|x| x as f64));
    let bmi = compute_metric_stats(&series, |p| p.bmi);
    let resting_metabolism =
        compute_metric_stats(&series, |p| p.resting_metabolism_kcal.map(|x| x as f64));

    // Sleep integration for summary (per spec/02)
    let sleep_nights = repo.get_sleeps_for_report(since.as_deref(), until.as_deref())?;
    let sleep_summary = if sleep_nights.is_empty() {
        None
    } else {
        let n = sleep_nights.len() as i64;
        let avg_total = {
            let vals: Vec<i64> = sleep_nights
                .iter()
                .filter_map(|s| s.total_sleep_minutes)
                .collect();
            if vals.is_empty() {
                None
            } else {
                Some(vals.iter().sum::<i64>() as f64 / vals.len() as f64)
            }
        };
        let avg_rem = {
            let vals: Vec<i64> = sleep_nights.iter().filter_map(|s| s.rem_minutes).collect();
            if vals.is_empty() {
                None
            } else {
                Some(vals.iter().sum::<i64>() as f64 / vals.len() as f64)
            }
        };
        let avg_eff = {
            let vals: Vec<f64> = sleep_nights
                .iter()
                .filter_map(|s| s.sleep_efficiency_pct)
                .collect();
            if vals.is_empty() {
                None
            } else {
                Some(vals.iter().sum::<f64>() / vals.len() as f64)
            }
        };
        let avg_qual = {
            let vals: Vec<i64> = sleep_nights
                .iter()
                .filter_map(|s| s.subjective_quality)
                .collect();
            if vals.is_empty() {
                None
            } else {
                Some(vals.iter().sum::<i64>() as f64 / vals.len() as f64)
            }
        };

        let trend = if n < 2 {
            Some("insufficient_data".to_string())
        } else {
            let first = sleep_nights.first().and_then(|s| s.total_sleep_minutes);
            let last = sleep_nights.last().and_then(|s| s.total_sleep_minutes);
            match (first, last) {
                (Some(f), Some(l)) if l > f => Some("improving".to_string()),
                (Some(f), Some(l)) if l < f => Some("declining".to_string()),
                (Some(_), Some(_)) => Some("stable".to_string()),
                _ => Some("insufficient_data".to_string()),
            }
        };

        Some(SleepSummary {
            nights_logged: n,
            avg_total_sleep_minutes: avg_total,
            avg_rem_minutes: avg_rem,
            avg_efficiency_pct: avg_eff,
            avg_quality: avg_qual,
            trend,
        })
    };

    let report = SummaryReport {
        period: Period { since, until },
        weight,
        body_fat,
        skeletal_muscle,
        visceral_fat,
        bmi,
        resting_metabolism,
        measurement_count,
        sleep: sleep_summary,
    };

    if json {
        print_json(&report);
    } else {
        print_summary_human(&report);
    }
    Ok(())
}

fn resolve_report_range(args: &ReportRangeArgs) -> Result<(Option<String>, Option<String>)> {
    resolve_date_range(args.since.as_deref(), args.until.as_deref(), args.days)
        .map_err(|e| RecomplogError::InvalidDate(e.to_string()))
}

fn resolve_summary_range(args: &SummaryArgs) -> Result<(Option<String>, Option<String>)> {
    // Support --period 30d etc as convenience for --days
    let days = if let Some(p) = &args.period {
        parse_period_to_days(p).or(args.days)
    } else {
        args.days
    };
    resolve_date_range(args.since.as_deref(), args.until.as_deref(), days)
        .map_err(|e| RecomplogError::InvalidDate(e.to_string()))
}

fn parse_period_to_days(p: &str) -> Option<i64> {
    let s = p.trim().to_lowercase();
    if let Some(num) = s.strip_suffix('d') {
        num.parse::<i64>().ok()
    } else if let Some(num) = s.strip_suffix('w') {
        num.parse::<i64>().ok().map(|w| w * 7)
    } else if let Some(num) = s.strip_suffix('m') {
        num.parse::<i64>().ok().map(|m| m * 30)
    } else {
        s.parse::<i64>().ok()
    }
}

fn compute_metric_stats<F>(series: &[MeasurementPoint], extract: F) -> Option<MetricStats>
where
    F: Fn(&MeasurementPoint) -> Option<f64>,
{
    let values: Vec<(String, f64)> = series
        .iter()
        .filter_map(|p| extract(p).map(|v| (p.date.clone(), v)))
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(compute_stats(&values))
}

fn compute_stats(values: &[(String, f64)]) -> MetricStats {
    if values.is_empty() {
        return MetricStats::default();
    }
    let nums: Vec<f64> = values.iter().map(|(_, v)| *v).collect();
    let count = nums.len() as i64;
    let min = nums.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let sum: f64 = nums.iter().sum();
    let avg = sum / (count as f64);

    let start = values.first().map(|(_, v)| *v);
    let end = values.last().map(|(_, v)| *v);
    let change = match (start, end) {
        (Some(s), Some(e)) => Some(e - s),
        _ => None,
    };

    let trend = if count < 2 {
        Some("insufficient_data".to_string())
    } else if let (Some(s), Some(e)) = (start, end) {
        if (e - s).abs() < 1e-9 {
            Some("stable".to_string())
        } else if e > s {
            Some("up".to_string())
        } else {
            Some("down".to_string())
        }
    } else {
        None
    };

    MetricStats {
        count,
        min: Some(min),
        max: Some(max),
        avg: Some(avg),
        start,
        end,
        change,
        trend,
    }
}

fn print_metric_report_human(report: &MetricReport) {
    println!("Report: {}  ({} points)", report.metric, report.stats.count);
    if let (Some(s), Some(u)) = (&report.period.since, &report.period.until) {
        println!("Period: {} → {}", s, u);
    } else if let Some(s) = &report.period.since {
        println!("Since: {}", s);
    } else if let Some(u) = &report.period.until {
        println!("Until: {}", u);
    }

    let st = &report.stats;
    println!(
        "min: {}  max: {}  avg: {}",
        st.min.map(round1).unwrap_or_default(),
        st.max.map(round1).unwrap_or_default(),
        st.avg.map(round1).unwrap_or_default()
    );
    if let (Some(start), Some(end), Some(ch)) = (st.start, st.end, st.change) {
        let dir = st.trend.as_deref().unwrap_or("?");
        println!(
            "start→end: {} → {}  (change {:+.1})  trend: {}",
            round1(start),
            round1(end),
            ch,
            dir
        );
    }

    // Simple table of the series (only the relevant column)
    if !report.series.is_empty() {
        let rows: Vec<Vec<String>> = report
            .series
            .iter()
            .map(|p| {
                let val = match report.metric.as_str() {
                    "weight" => p.weight_kg,
                    "body_fat" => p.body_fat_pct,
                    "skeletal_muscle" => p.skeletal_muscle_pct,
                    "visceral_fat" => p.visceral_fat_level.map(|x| x as f64),
                    "bmi" => p.bmi,
                    "resting_metabolism" => p.resting_metabolism_kcal.map(|x| x as f64),
                    _ => None,
                };
                vec![p.date.clone(), val.map(round1).unwrap_or_default()]
            })
            .collect();
        print_table(vec!["Date", &report.metric], rows);
    }
}

fn round1(v: f64) -> String {
    format!("{:.1}", v)
}

fn print_summary_human(report: &SummaryReport) {
    println!("Body composition summary");
    if let (Some(s), Some(u)) = (&report.period.since, &report.period.until) {
        println!(
            "Period: {} → {}  ({} measurements)",
            s, u, report.measurement_count
        );
    } else {
        println!("({} measurements)", report.measurement_count);
    }
    println!();

    print_summary_metric_line("weight (kg)", &report.weight);
    print_summary_metric_line("body fat %", &report.body_fat);
    print_summary_metric_line("muscle %", &report.skeletal_muscle);
    print_summary_metric_line("visceral fat", &report.visceral_fat);
    print_summary_metric_line("bmi", &report.bmi);
    print_summary_metric_line("resting metabolism (kcal)", &report.resting_metabolism);

    // Sleep section (only if data present in period)
    if let Some(ss) = &report.sleep {
        println!();
        println!("Sleep ({} nights)", ss.nights_logged);
        if let Some(v) = ss.avg_total_sleep_minutes {
            println!("  Avg total sleep: {}", format_minutes(v.round() as i64));
        }
        if let Some(v) = ss.avg_rem_minutes {
            // REM % of total is nice-to-have but requires avg total; keep simple
            println!("  Avg REM: {}", format_minutes(v.round() as i64));
        }
        if let Some(v) = ss.avg_efficiency_pct {
            println!("  Avg efficiency: {:.1}%", v);
        }
        if let Some(v) = ss.avg_quality {
            println!("  Avg quality: {:.1}", v);
        }
        if let Some(t) = &ss.trend {
            println!("  Trend: {}", t);
        }
    }
}

fn print_summary_metric_line(name: &str, stats: &Option<MetricStats>) {
    match stats {
        Some(st) if st.count > 0 => {
            let ch = st
                .change
                .map(|c| format!("{:>+6.1}", c))
                .unwrap_or_default();
            let tr = st.trend.as_deref().unwrap_or("");
            println!(
                "  {:<22} n={:<3}  min={:<6} max={:<6} avg={:<6}  start→end {:>6} → {:<6} {:>6}  {}",
                name,
                st.count,
                st.min.map(round1).unwrap_or_default(),
                st.max.map(round1).unwrap_or_default(),
                st.avg.map(round1).unwrap_or_default(),
                st.start.map(round1).unwrap_or_default(),
                st.end.map(round1).unwrap_or_default(),
                ch,
                tr
            );
        }
        _ => {
            println!("  {:<22} (no data)", name);
        }
    }
}

// ---------- CONFIG (user profile) HANDLER ----------

pub fn handle_profile(
    repo: &mut Repository,
    action: ProfileAction,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        ProfileAction::Set(args) => handle_profile_set(repo, args, json, quiet),
        ProfileAction::Show => handle_profile_show(repo, json),
    }
}

// ---------- SLEEP HANDLER ----------

pub fn handle_sleep(
    repo: &mut Repository,
    action: SleepAction,
    limits: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        SleepAction::Create(args) => handle_sleep_create(repo, args, limits, json, quiet),
        SleepAction::List(args) => handle_sleep_list(repo, args, json, quiet),
        SleepAction::Show(args) => handle_sleep_show(repo, args, json),
        SleepAction::Update(args) => handle_sleep_update(repo, args, limits, json, quiet),
        SleepAction::Delete(args) => handle_sleep_delete(repo, args, json, quiet),
    }
}

fn handle_profile_set(
    repo: &mut Repository,
    args: ProfileSetArgs,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if args.height_cm.is_none() && args.date_of_birth.is_none() {
        let msg = "no profile fields provided; use --height-cm and/or --date-of-birth".to_string();
        if json {
            print_error_json(&msg);
        } else {
            eprintln!("{}", msg);
        }
        std::process::exit(1);
    }

    let dob_ymd = match &args.date_of_birth {
        Some(raw) => {
            Some(parse_date_to_ymd(raw).map_err(|e| RecomplogError::InvalidDate(e.to_string()))?)
        }
        None => None,
    };

    // height validation happens inside repository (on provided + merged)
    repo.set_profile(args.height_cm, dob_ymd)?;

    let msg = "User profile updated".to_string();
    if json {
        print_json(&Success::ok(msg));
    } else {
        quiet_print(&msg, quiet);
    }
    Ok(())
}

fn handle_profile_show(repo: &mut Repository, json: bool) -> Result<()> {
    let profile = repo.get_profile()?;

    if json {
        print_json(&profile);
        return Ok(());
    }

    println!("User profile:");
    match profile.height_cm {
        Some(h) => println!("  height_cm: {} cm", h),
        None => println!("  height_cm: (not set)"),
    }
    match profile.date_of_birth {
        Some(d) => println!("  date_of_birth: {}", d),
        None => println!("  date_of_birth: (not set)"),
    }
    if let Some(ts) = &profile.updated_at {
        println!("  updated: {}", format_local(&ts.utc));
    } else {
        println!("  updated: (never)");
    }
    Ok(())
}

// ---------- SLEEP IMPLEMENTATION ----------

fn parse_opt_duration(s: &Option<String>) -> Result<Option<i64>> {
    match s {
        Some(raw) => {
            let mins = parse_duration_to_minutes(raw)?;
            Ok(Some(mins))
        }
        None => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn proposed_sleep_metrics(
    time_in_bed: Option<i64>,
    total_sleep: Option<i64>,
    rem: Option<i64>,
    deep: Option<i64>,
    light: Option<i64>,
    awake: Option<i64>,
    efficiency: Option<f64>,
    score: Option<i64>,
    quality: Option<i64>,
    awakenings: Option<i64>,
    heart_rate_bpm: Option<f64>,
    hypopnea_per_hr: Option<f64>,
    respiratory_rate: Option<f64>,
) -> ProposedSleepMetrics {
    ProposedSleepMetrics {
        time_in_bed_minutes: time_in_bed,
        total_sleep_minutes: total_sleep,
        rem_minutes: rem,
        deep_minutes: deep,
        light_minutes: light,
        awake_minutes: awake,
        sleep_efficiency_pct: efficiency,
        sleep_score: score,
        subjective_quality: quality,
        awakenings,
        heart_rate_bpm,
        hypopnea_per_hr,
        respiratory_rate,
    }
}

/// Absolute hard-fail for sleep fields using config limits (no variation checks).
fn run_sleep_sanity(proposed: &ProposedSleepMetrics, limits: &SanityLimits) -> Result<()> {
    if let Err(errors) = validate_sleep_absolute(proposed, &limits.sleep) {
        return Err(RecomplogError::InvalidSleep(errors.join("; ")));
    }
    Ok(())
}

/// If efficiency not provided, and both bed and total are Some and >0, compute it.
fn auto_efficiency(
    provided: Option<f64>,
    time_in_bed: Option<i64>,
    total_sleep: Option<i64>,
) -> Option<f64> {
    if provided.is_some() {
        return provided;
    }
    match (time_in_bed, total_sleep) {
        (Some(bed), Some(sleep)) if bed > 0 => {
            let eff = (sleep as f64 * 100.0) / (bed as f64);
            // clamp to sane range
            Some(eff.clamp(0.0, 100.0))
        }
        _ => provided,
    }
}

/// Warn (to stderr) if sum of stages exceeds time in bed (when known). Non-fatal.
fn warn_if_stages_exceed_bed(
    time_in_bed: Option<i64>,
    rem: Option<i64>,
    deep: Option<i64>,
    light: Option<i64>,
    awake: Option<i64>,
) {
    if let Some(bed) = time_in_bed {
        if bed <= 0 {
            return;
        }
        let sum: i64 =
            rem.unwrap_or(0) + deep.unwrap_or(0) + light.unwrap_or(0) + awake.unwrap_or(0);
        if sum > bed {
            eprintln!(
                "Warning: sum of stage minutes ({} ) exceeds time in bed ({}).",
                sum, bed
            );
        }
    }
}

fn handle_sleep_create(
    repo: &mut Repository,
    args: SleepCreateArgs,
    limits: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let date =
        parse_date_to_ymd(&args.date).map_err(|e| RecomplogError::InvalidDate(e.to_string()))?;

    // Parse durations
    let time_in_bed = parse_opt_duration(&args.time_in_bed)?;
    let total_sleep = parse_opt_duration(&args.total_sleep)?;
    let rem = parse_opt_duration(&args.rem)?;
    let deep = parse_opt_duration(&args.deep)?;
    let light = parse_opt_duration(&args.light)?;
    let awake = parse_opt_duration(&args.awake)?;

    // Auto efficiency
    let efficiency = auto_efficiency(args.sleep_efficiency, time_in_bed, total_sleep);

    // Config-backed absolute range checks (no variation / delta checks for sleep)
    let proposed = proposed_sleep_metrics(
        time_in_bed,
        total_sleep,
        rem,
        deep,
        light,
        awake,
        efficiency,
        args.sleep_score,
        args.quality,
        args.awakenings,
        args.heart_rate,
        args.hypopnea,
        args.respiratory_rate,
    );
    run_sleep_sanity(&proposed, limits)?;

    warn_if_stages_exceed_bed(time_in_bed, rem, deep, light, awake);

    // Notes: take ownership
    let notes = args.notes.clone();

    match repo.create_sleep(
        &date,
        args.bedtime.as_deref(),
        args.wake_time.as_deref(),
        time_in_bed,
        total_sleep,
        rem,
        deep,
        light,
        awake,
        efficiency,
        args.sleep_score,
        args.quality,
        args.awakenings,
        args.heart_rate,
        args.hypopnea,
        args.respiratory_rate,
        notes.as_deref(),
    ) {
        Ok(id) => {
            let msg = format!("Sleep entry created for {}", date);
            if json {
                print_json(&Success::created(id, date, msg));
            } else {
                quiet_print(&msg, quiet);
                // Print a small human summary of what was logged
                if !quiet {
                    print_sleep_summary_human(
                        id,
                        &date,
                        args.bedtime.as_deref(),
                        args.wake_time.as_deref(),
                        time_in_bed,
                        total_sleep,
                        rem,
                        deep,
                        light,
                        awake,
                        efficiency,
                        args.sleep_score,
                        args.quality,
                        args.awakenings,
                        args.heart_rate,
                        args.hypopnea,
                        args.respiratory_rate,
                        notes.as_deref(),
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            if json {
                // For duplicate date, include a suggestion per spec
                if matches!(e, RecomplogError::SleepExistsForDate(_)) {
                    #[derive(serde::Serialize)]
                    struct ErrWithSuggestion {
                        success: bool,
                        error: String,
                        suggestion: String,
                    }
                    print_json(&ErrWithSuggestion {
                        success: false,
                        error: e.to_string(),
                        suggestion: format!("recomplog sleep update --date {}", date),
                    });
                    std::process::exit(1);
                } else {
                    print_error_json(&e.to_string());
                    std::process::exit(1);
                }
            } else {
                Err(e)
            }
        }
    }
}

fn handle_sleep_list(repo: &mut Repository, args: ListArgs, json: bool, quiet: bool) -> Result<()> {
    let (since, until) =
        resolve_date_range(args.since.as_deref(), args.until.as_deref(), args.days)
            .map_err(|e| RecomplogError::InvalidDate(e.to_string()))?;
    let sleeps = repo.list_sleeps(since.as_deref(), until.as_deref())?;

    if json {
        print_json(&sleeps);
        return Ok(());
    }

    if quiet {
        for s in &sleeps {
            println!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                s.id,
                s.date,
                s.bedtime.as_deref().unwrap_or(""),
                s.wake_time.as_deref().unwrap_or(""),
                s.time_in_bed_minutes.unwrap_or(0),
                s.total_sleep_minutes.unwrap_or(0),
                s.rem_minutes.unwrap_or(0),
                s.deep_minutes.unwrap_or(0),
                s.light_minutes.unwrap_or(0),
                s.awake_minutes.unwrap_or(0),
                s.sleep_efficiency_pct
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
                s.sleep_score.unwrap_or(0),
                s.subjective_quality.unwrap_or(0),
                s.heart_rate_bpm
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
                s.hypopnea_per_hr
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
                s.respiratory_rate
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
            );
        }
        return Ok(());
    }

    if sleeps.is_empty() {
        println!("(no sleep entries)");
        return Ok(());
    }

    let rows: Vec<Vec<String>> = sleeps
        .iter()
        .map(|s| {
            vec![
                s.id.to_string(),
                s.date.clone(),
                s.bedtime.clone().unwrap_or_default(),
                s.wake_time.clone().unwrap_or_default(),
                opt_minutes(s.time_in_bed_minutes),
                opt_minutes(s.total_sleep_minutes),
                opt_minutes(s.rem_minutes),
                opt_minutes(s.deep_minutes),
                opt_minutes(s.light_minutes),
                opt_minutes(s.awake_minutes),
                s.sleep_efficiency_pct
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
                opt_i64(s.sleep_score),
                opt_i64(s.subjective_quality),
                s.heart_rate_bpm
                    .map(|v| format!("{:.0}", v))
                    .unwrap_or_default(),
                s.hypopnea_per_hr
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
                s.respiratory_rate
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
            ]
        })
        .collect();
    print_table(
        vec![
            "ID",
            "Date",
            "Bedtime",
            "Wake",
            "Time in Bed",
            "Total Sleep",
            "REM",
            "Deep",
            "Light",
            "Awake",
            "Eff%",
            "Score",
            "Quality",
            "HR",
            "Hyp/hr",
            "Resp",
        ],
        rows,
    );
    Ok(())
}

fn opt_minutes(v: Option<i64>) -> String {
    v.map(format_minutes).unwrap_or_default()
}

fn handle_sleep_show(repo: &mut Repository, args: ShowArgs, json: bool) -> Result<()> {
    let (id, date) = resolve_identifier(args.id, args.date)?;

    let s: Sleep = if let Some(i) = id {
        repo.get_sleep(i)?
    } else if let Some(d) = date {
        repo.get_sleep_by_date(&d)?
    } else {
        unreachable!()
    };

    if json {
        print_json(&s);
    } else {
        println!("Sleep {} ({})", s.id, s.date);
        if let Some(v) = &s.bedtime {
            println!("  bedtime: {}", v);
        }
        if let Some(v) = &s.wake_time {
            println!("  wake_time: {}", v);
        }
        if let Some(v) = s.time_in_bed_minutes {
            println!("  time_in_bed_minutes: {} ({})", v, format_minutes(v));
        }
        if let Some(v) = s.total_sleep_minutes {
            println!("  total_sleep_minutes: {} ({})", v, format_minutes(v));
        }
        if let Some(v) = s.rem_minutes {
            println!("  rem_minutes: {} ({})", v, format_minutes(v));
        }
        if let Some(v) = s.deep_minutes {
            println!("  deep_minutes: {} ({})", v, format_minutes(v));
        }
        if let Some(v) = s.light_minutes {
            println!("  light_minutes: {} ({})", v, format_minutes(v));
        }
        if let Some(v) = s.awake_minutes {
            println!("  awake_minutes: {} ({})", v, format_minutes(v));
        }
        if let Some(v) = s.sleep_efficiency_pct {
            println!("  sleep_efficiency_pct: {:.2}", v);
        }
        if let Some(v) = s.sleep_score {
            println!("  sleep_score: {}", v);
        }
        if let Some(v) = s.subjective_quality {
            println!("  subjective_quality: {}", v);
        }
        if let Some(v) = s.awakenings {
            println!("  awakenings: {}", v);
        }
        if let Some(v) = s.heart_rate_bpm {
            println!("  heart_rate_bpm: {:.1}", v);
        }
        if let Some(v) = s.hypopnea_per_hr {
            println!("  hypopnea_per_hr: {:.1}/hr", v);
        }
        if let Some(v) = s.respiratory_rate {
            println!("  respiratory_rate: {:.1}", v);
        }
        if let Some(v) = &s.notes {
            println!("  notes: {}", v);
        }
        println!(
            "  created: {}",
            crate::utils::format_local(&s.created_at.utc)
        );
        println!(
            "  updated: {}",
            crate::utils::format_local(&s.updated_at.utc)
        );
    }
    Ok(())
}

fn handle_sleep_update(
    repo: &mut Repository,
    args: SleepUpdateArgs,
    limits: &SanityLimits,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let (id, date) = resolve_identifier(args.id, args.date)?;

    // Parse any provided durations
    let time_in_bed = parse_opt_duration(&args.time_in_bed)?;
    let total_sleep = parse_opt_duration(&args.total_sleep)?;
    let rem = parse_opt_duration(&args.rem)?;
    let deep = parse_opt_duration(&args.deep)?;
    let light = parse_opt_duration(&args.light)?;
    let awake = parse_opt_duration(&args.awake)?;

    // For update we do NOT auto-calc efficiency unless the caller provides the two times
    // and omits efficiency (per open question in spec: "on create only, or also on update?").
    // We choose: also on update for convenience if the two times are being set in this update.
    // But to keep simple and predictable: only auto if BOTH times are provided in *this* call
    // (i.e. not relying on prior stored values here). We can fetch and merge, but that adds complexity.
    // Per pragmatic choice: compute if the user supplied (or is supplying) both in this update
    // and did not supply efficiency. For full merge we'd need the existing record.
    // Simplest that is still useful: if efficiency omitted and *both* time_in_bed and total_sleep
    // are Some in this payload, compute.
    let efficiency = auto_efficiency(args.sleep_efficiency, time_in_bed, total_sleep);

    let proposed = proposed_sleep_metrics(
        time_in_bed,
        total_sleep,
        rem,
        deep,
        light,
        awake,
        efficiency,
        args.sleep_score,
        args.quality,
        args.awakenings,
        args.heart_rate,
        args.hypopnea,
        args.respiratory_rate,
    );
    run_sleep_sanity(&proposed, limits)?;

    // For stage sum warning we can only check what is being provided; if partial we skip or fetch.
    // For UX, if any stage provided and time_in_bed in this payload, warn using that.
    if time_in_bed.is_some() {
        warn_if_stages_exceed_bed(time_in_bed, rem, deep, light, awake);
    }

    let updated_id = if let Some(i) = id {
        repo.update_sleep(
            i,
            args.bedtime.as_deref(),
            args.wake_time.as_deref(),
            time_in_bed,
            total_sleep,
            rem,
            deep,
            light,
            awake,
            efficiency,
            args.sleep_score,
            args.quality,
            args.awakenings,
            args.heart_rate,
            args.hypopnea,
            args.respiratory_rate,
            args.notes.as_deref(),
        )?;
        i
    } else if let Some(d) = date {
        repo.update_sleep_by_date(
            &d,
            args.bedtime.as_deref(),
            args.wake_time.as_deref(),
            time_in_bed,
            total_sleep,
            rem,
            deep,
            light,
            awake,
            efficiency,
            args.sleep_score,
            args.quality,
            args.awakenings,
            args.heart_rate,
            args.hypopnea,
            args.respiratory_rate,
            args.notes.as_deref(),
        )?
    } else {
        unreachable!()
    };

    let s = repo.get_sleep(updated_id)?;
    let msg = format!("Updated sleep entry {} ({})", updated_id, s.date);

    if json {
        print_json(&Success::created(updated_id, s.date, msg));
    } else {
        quiet_print(&msg, quiet);
    }
    Ok(())
}

fn handle_sleep_delete(
    repo: &mut Repository,
    args: DeleteArgs,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let (id, date) = resolve_identifier(args.id, args.date)?;

    let deleted_id = if let Some(i) = id {
        // capture date for nicer JSON
        let s = repo.get_sleep(i)?;
        let did = repo.delete_sleep(i)?;
        if json {
            print_json(&SleepDeleteSuccess {
                success: true,
                deleted_id: did,
                date: Some(s.date),
            });
        } else {
            let msg = format!("Deleted sleep entry {}", did);
            quiet_print(&msg, quiet);
        }
        did
    } else if let Some(d) = date {
        let did = repo.delete_sleep_by_date(&d)?;
        if json {
            print_json(&SleepDeleteSuccess {
                success: true,
                deleted_id: did,
                date: Some(d.clone()),
            });
        } else {
            let msg = format!("Deleted sleep entry {}", did);
            quiet_print(&msg, quiet);
        }
        did
    } else {
        unreachable!()
    };

    // For non-json path we already printed; for id path without date we still have deleted_id above.
    if !json {
        // already handled
    }
    let _ = deleted_id;
    Ok(())
}

#[derive(serde::Serialize)]
struct SleepDeleteSuccess {
    success: bool,
    deleted_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn print_sleep_summary_human(
    id: i64,
    _date: &str,
    bedtime: Option<&str>,
    wake_time: Option<&str>,
    time_in_bed: Option<i64>,
    total_sleep: Option<i64>,
    rem: Option<i64>,
    deep: Option<i64>,
    light: Option<i64>,
    awake: Option<i64>,
    efficiency: Option<f64>,
    score: Option<i64>,
    quality: Option<i64>,
    awakenings: Option<i64>,
    heart_rate_bpm: Option<f64>,
    hypopnea_per_hr: Option<f64>,
    respiratory_rate: Option<f64>,
    notes: Option<&str>,
) {
    println!("  id: {}", id);
    if let Some(b) = bedtime {
        println!("  bedtime: {}", b);
    }
    if let Some(w) = wake_time {
        println!("  wake_time: {}", w);
    }
    if let Some(v) = time_in_bed {
        println!("  time_in_bed: {} ({})", v, format_minutes(v));
    }
    if let Some(v) = total_sleep {
        println!("  total_sleep: {} ({})", v, format_minutes(v));
    }
    if let Some(v) = rem {
        println!("  rem: {} ({})", v, format_minutes(v));
    }
    if let Some(v) = deep {
        println!("  deep: {} ({})", v, format_minutes(v));
    }
    if let Some(v) = light {
        println!("  light: {} ({})", v, format_minutes(v));
    }
    if let Some(v) = awake {
        println!("  awake: {} ({})", v, format_minutes(v));
    }
    if let Some(v) = efficiency {
        println!("  efficiency: {:.1}%", v);
    }
    if let Some(v) = score {
        println!("  score: {}", v);
    }
    if let Some(v) = quality {
        println!("  quality: {}", v);
    }
    if let Some(v) = awakenings {
        println!("  awakenings: {}", v);
    }
    if let Some(v) = heart_rate_bpm {
        println!("  heart_rate_bpm: {:.1}", v);
    }
    if let Some(v) = hypopnea_per_hr {
        println!("  hypopnea_per_hr: {:.1}/hr", v);
    }
    if let Some(v) = respiratory_rate {
        println!("  respiratory_rate: {:.1}", v);
    }
    if let Some(n) = notes {
        println!("  notes: {}", n);
    }
}

// ---------- SLEEP REPORT + SUMMARY INTEGRATION ----------

fn handle_sleep_report(repo: &mut Repository, args: ReportRangeArgs, json: bool) -> Result<()> {
    let (since, until) = resolve_report_range(&args)?;
    let nights = repo.get_sleeps_for_report(since.as_deref(), until.as_deref())?;

    let nights_logged = nights.len() as i64;

    // Compute averages (only over records that have the value)
    let avg = |extract: fn(&Sleep) -> Option<i64>| -> Option<f64> {
        let vals: Vec<i64> = nights.iter().filter_map(extract).collect();
        if vals.is_empty() {
            return None;
        }
        Some(vals.iter().sum::<i64>() as f64 / vals.len() as f64)
    };
    let avg_f = |extract: fn(&Sleep) -> Option<f64>| -> Option<f64> {
        let vals: Vec<f64> = nights.iter().filter_map(extract).collect();
        if vals.is_empty() {
            return None;
        }
        Some(vals.iter().sum::<f64>() / vals.len() as f64)
    };

    let averages = SleepAverages {
        total_sleep_minutes: avg(|s| s.total_sleep_minutes),
        rem_minutes: avg(|s| s.rem_minutes),
        deep_minutes: avg(|s| s.deep_minutes),
        light_minutes: avg(|s| s.light_minutes),
        awake_minutes: avg(|s| s.awake_minutes),
        efficiency_pct: avg_f(|s| s.sleep_efficiency_pct),
        score: avg_f(|s| s.sleep_score.map(|x| x as f64)),
        quality: avg_f(|s| s.subjective_quality.map(|x| x as f64)),
        heart_rate_bpm: avg_f(|s| s.heart_rate_bpm),
        hypopnea_per_hr: avg_f(|s| s.hypopnea_per_hr),
        respiratory_rate: avg_f(|s| s.respiratory_rate),
    };

    // Extremes
    let best_total = nights
        .iter()
        .filter_map(|s| s.total_sleep_minutes.map(|m| (s.date.clone(), m)))
        .max_by_key(|(_, m)| *m)
        .map(|(d, m)| SleepExtreme {
            date: d,
            minutes: Some(m),
            pct: None,
        });
    let worst_total = nights
        .iter()
        .filter_map(|s| s.total_sleep_minutes.map(|m| (s.date.clone(), m)))
        .min_by_key(|(_, m)| *m)
        .map(|(d, m)| SleepExtreme {
            date: d,
            minutes: Some(m),
            pct: None,
        });
    let best_eff = nights
        .iter()
        .filter_map(|s| s.sleep_efficiency_pct.map(|e| (s.date.clone(), e)))
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(d, e)| SleepExtreme {
            date: d,
            minutes: None,
            pct: Some(e),
        });
    let worst_eff = nights
        .iter()
        .filter_map(|s| s.sleep_efficiency_pct.map(|e| (s.date.clone(), e)))
        .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(d, e)| SleepExtreme {
            date: d,
            minutes: None,
            pct: Some(e),
        });

    let extremes = SleepExtremes {
        best_total_sleep: best_total,
        worst_total_sleep: worst_total,
        best_efficiency: best_eff,
        worst_efficiency: worst_eff,
    };

    // Simple trend: compare first vs last total_sleep (chronological order in get_ is ASC)
    let trend = if nights_logged < 2 {
        Some("insufficient_data".to_string())
    } else {
        // Use the loaded order (ASC by date from get_sleeps_for_report)
        let first = nights.first().and_then(|s| s.total_sleep_minutes);
        let last = nights.last().and_then(|s| s.total_sleep_minutes);
        match (first, last) {
            (Some(f), Some(l)) if l > f => Some("improving".to_string()),
            (Some(f), Some(l)) if l < f => Some("declining".to_string()),
            (Some(_), Some(_)) => Some("stable".to_string()),
            _ => Some("insufficient_data".to_string()),
        }
    };

    let report = SleepReport {
        period: Period { since, until },
        nights_logged,
        averages,
        extremes,
        trend,
        nights,
    };

    if json {
        print_json(&report);
    } else {
        print_sleep_report_human(&report);
    }
    Ok(())
}

fn print_sleep_report_human(report: &SleepReport) {
    println!("Sleep report");
    if let (Some(s), Some(u)) = (&report.period.since, &report.period.until) {
        println!("Period: {} → {}  ({} nights)", s, u, report.nights_logged);
    } else {
        println!("({} nights logged)", report.nights_logged);
    }
    println!();

    let a = &report.averages;
    if let Some(v) = a.total_sleep_minutes {
        println!("  avg total sleep: {}", format_minutes(v.round() as i64));
    }
    if let Some(v) = a.rem_minutes {
        println!("  avg REM: {}", format_minutes(v.round() as i64));
    }
    if let Some(v) = a.deep_minutes {
        println!("  avg deep: {}", format_minutes(v.round() as i64));
    }
    if let Some(v) = a.light_minutes {
        println!("  avg light: {}", format_minutes(v.round() as i64));
    }
    if let Some(v) = a.awake_minutes {
        println!("  avg awake: {}", format_minutes(v.round() as i64));
    }
    if let Some(v) = a.efficiency_pct {
        println!("  avg efficiency: {:.1}%", v);
    }
    if let Some(v) = a.score {
        println!("  avg score: {:.1}", v);
    }
    if let Some(v) = a.quality {
        println!("  avg quality: {:.1}", v);
    }
    if let Some(v) = a.heart_rate_bpm {
        println!("  avg heart rate: {:.1} bpm", v);
    }
    if let Some(v) = a.hypopnea_per_hr {
        println!("  avg hypopnea: {:.1}/hr", v);
    }
    if let Some(v) = a.respiratory_rate {
        println!("  avg respiratory rate: {:.1}/min", v);
    }

    // Extremes
    if let Some(e) = &report.extremes.best_total_sleep {
        if let Some(m) = e.minutes {
            println!("  best total sleep: {} on {}", format_minutes(m), e.date);
        }
    }
    if let Some(e) = &report.extremes.worst_total_sleep {
        if let Some(m) = e.minutes {
            println!("  worst total sleep: {} on {}", format_minutes(m), e.date);
        }
    }
    if let Some(e) = &report.extremes.best_efficiency {
        if let Some(p) = e.pct {
            println!("  best efficiency: {:.1}% on {}", p, e.date);
        }
    }
    if let Some(e) = &report.extremes.worst_efficiency {
        if let Some(p) = e.pct {
            println!("  worst efficiency: {:.1}% on {}", p, e.date);
        }
    }

    if let Some(t) = &report.trend {
        println!("  trend: {}", t);
    }

    // Optional mini table of recent nights (all in period for small sets)
    if !report.nights.is_empty() {
        println!();
        // show up to 14 newest-ish (they are ASC, rev for recent first)
        let rows: Vec<Vec<String>> = report
            .nights
            .iter()
            .rev()
            .take(14)
            .map(|n| {
                vec![
                    n.date.clone(),
                    n.total_sleep_minutes
                        .map(format_minutes)
                        .unwrap_or_default(),
                    n.rem_minutes.map(format_minutes).unwrap_or_default(),
                    n.deep_minutes.map(format_minutes).unwrap_or_default(),
                    n.light_minutes.map(format_minutes).unwrap_or_default(),
                    n.awake_minutes.map(format_minutes).unwrap_or_default(),
                    n.sleep_efficiency_pct
                        .map(|v| format!("{:.1}", v))
                        .unwrap_or_default(),
                    opt_i64(n.subjective_quality),
                ]
            })
            .collect();
        print_table(
            vec![
                "Date",
                "Total Sleep",
                "REM",
                "Deep",
                "Light",
                "Awake",
                "Eff%",
                "Qual",
            ],
            rows,
        );
    }
}
