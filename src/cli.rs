use clap::{Args, Parser, Subcommand, ValueEnum};

/// recomplog - unified CLI for body recomposition tracking.
///
/// Combines workout logging (repslog), body measurements + sleep (bodylog),
/// nutrition logging (nutlog), and HTML dashboard generation (bodydashboard)
/// into a single local-first, agent-friendly tool.
///
/// All data lives in one SQLite database for reliable cross-domain reports.
#[derive(Parser, Debug)]
#[command(name = "recomplog", version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Output structured JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Override default SQLite database location
    /// (XDG: ~/.local/share/recomplog/recomplog.db).
    #[arg(long, global = true, value_name = "PATH")]
    pub db: Option<String>,

    /// Override application config path
    /// (default: ~/.config/recomplog/config.toml).
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<String>,

    /// Minimal / machine-friendly human output.
    #[arg(long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Training: workouts (sessions), exercises, and sets.
    ///
    /// All training-related commands live under `workout`:
    ///   recomplog workout create ...
    ///   recomplog workout exercise list ...
    ///   recomplog workout set add ...
    Workout {
        #[command(subcommand)]
        action: Box<WorkoutAction>,
    },

    /// Body metrics and sleep (measurements, composition, rest).
    ///
    ///   recomplog body measurement create ...
    ///   recomplog body measurement list --days 30
    ///   recomplog body sleep create --date yesterday ...
    Body {
        #[command(subcommand)]
        action: BodyAction,
    },

    /// Nutrition logging: products, purchases, consumption, nutrients.
    ///
    ///   recomplog nutrition product create "Oats 500g" --tags bulk,breakfast
    ///   recomplog nutrition product list --json
    ///   recomplog nutrition consumption create --product 3 --quantity 1.5
    Nutrition {
        #[command(subcommand)]
        action: NutritionAction,
    },

    /// Generate reports (nutrition, body trends, combined, HTML dashboard).
    /// Reports are intentionally top-level for quick cross-domain access.
    Report {
        #[command(subcommand)]
        action: ReportAction,
    },

    /// Import data from external sources (FIT files, legacy tool databases).
    Import {
        #[command(subcommand)]
        action: ImportAction,
    },

    /// Configuration and sanity limit management.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Audit database contents (sanity limits) or detect missing daily logs.
    ///
    ///   recomplog check --variations --days 90
    ///   recomplog check missing --days 7 --workout-days 3
    Check(CheckCommand),

    /// One-time initialization and migration helpers.
    Init {
        #[arg(long)]
        dry_run: bool,
    },

    /// Database migration status / apply (advanced).
    Migrate {
        #[arg(short, long)]
        status: bool,
        #[arg(short, long)]
        dry_run: bool,
        #[arg(short, long)]
        force: bool,
    },

    /// Print version (also available as --version).
    Version,
}

// ---------- Grouped command actions ----------

/// Actions under `recomplog workout ...`
#[derive(Subcommand, Debug, Clone)]
pub enum WorkoutAction {
    /// Create a new workout session (container for exercises/sets).
    Create {
        /// Start time as RFC3339 (e.g. 2026-07-14T18:30:00-03:00). Default: now (UTC Z).
        #[arg(long)]
        started_at: Option<String>,
        /// End time as RFC3339 (e.g. 2026-07-14T19:45:00-03:00). Stored as UTC Z.
        #[arg(long = "finished-at")]
        finished_at: Option<String>,
        /// e.g. Push, Pull, Run, Full Body
        #[arg(long = "type")]
        workout_type: Option<String>,
        #[arg(long)]
        notes: Option<String>,
        /// Validate and show what would be created without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// List recent workouts.
    List {
        #[arg(long)]
        days: Option<i64>,
        #[arg(long, default_value_t = 30)]
        limit: i64,
    },
    /// Show a workout with its exercises and sets.
    Show { id: i64 },
    /// Update workout fields (partial).
    Update {
        id: i64,
        #[arg(long = "type")]
        workout_type: Option<String>,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long)]
        duration: Option<i32>,
        #[arg(long)]
        feeling: Option<i32>,
        /// Start time as RFC3339 (stored as UTC Z).
        #[arg(long)]
        started_at: Option<String>,
        /// End time as RFC3339 (stored as UTC Z).
        #[arg(long = "finished-at")]
        finished_at: Option<String>,
        /// Validate and show what would be updated without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a workout (cascades exercises/sets).
    Delete {
        id: i64,
        /// Show what would be deleted without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Workout analysis: volume, PRs, history, load progression.
    ///
    /// Bare `workout stats --days N` is an alias for `workout stats volume --days N`.
    Stats {
        #[command(subcommand)]
        action: Option<WorkoutStatsAction>,
        /// Window in days when no subcommand is given (volume alias).
        #[arg(long, default_value_t = 30)]
        days: i64,
    },

    /// Exercise catalog operations (under the workout group).
    Exercise {
        #[command(subcommand)]
        action: ExerciseAction,
    },

    /// Set logging operations (under the workout group).
    Set {
        #[command(subcommand)]
        action: Box<SetAction>,
    },
}

/// Actions under `recomplog workout stats ...`
#[derive(Subcommand, Debug, Clone)]
pub enum WorkoutStatsAction {
    /// Personal records: max weight (body-mass aware) and max reps per exercise.
    Prs {
        #[arg(short, long)]
        exercise: Option<String>,
    },
    /// Training volume (body-mass aware kg·reps + effective reps).
    Volume {
        #[arg(short, long)]
        exercise: Option<String>,
        /// Period string: `30d`, `90d`, or `1y` (mutually exclusive with `--days`).
        #[arg(short, long)]
        period: Option<String>,
        #[arg(long)]
        days: Option<i64>,
    },
    /// Session frequency / duration summary for a day window.
    Summary {
        #[arg(short, long, default_value_t = 30)]
        days: i64,
    },
    /// Per-set history for an exercise across workouts in a date range.
    History {
        #[arg(short, long)]
        exercise: String,
        #[arg(short, long, default_value_t = 30)]
        days: i64,
    },
    /// Load progression for an exercise (sets with recorded weight only).
    Weight {
        #[arg(short, long)]
        exercise: String,
    },
}

/// Exercise actions: `recomplog workout exercise <action>`
#[derive(Subcommand, Debug, Clone)]
pub enum ExerciseAction {
    List {
        #[arg(short, long)]
        search: Option<String>,
        #[arg(short, long)]
        category: Option<String>,
    },
    Create {
        /// Exercise name (lowercase, singular recommended)
        name: String,
        #[arg(short, long)]
        category: String,
        #[arg(short, long)]
        equipment: Option<String>,
        #[arg(long = "load-type")]
        load_type: Option<String>,
        #[arg(short, long)]
        muscles: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
        #[arg(long = "allow-phase-in-name")]
        allow_phase_in_name: bool,
        /// Validate and show what would be created without writing.
        #[arg(long)]
        dry_run: bool,
    },
    Update {
        /// Exercise id or name
        exercise: String,
        #[arg(short, long)]
        category: Option<String>,
        #[arg(short, long)]
        equipment: Option<String>,
        #[arg(long = "clear-equipment")]
        clear_equipment: bool,
        #[arg(long = "load-type")]
        load_type: Option<String>,
        #[arg(short, long)]
        muscles: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
        /// Validate and show what would be updated without writing.
        #[arg(long)]
        dry_run: bool,
    },
    Search {
        term: String,
    },
}

/// Set actions: `recomplog workout set <action>`
#[derive(Subcommand, Debug, Clone)]
pub enum SetAction {
    /// Add a strength (or general) set.
    Add {
        #[arg(long)]
        workout: Option<i64>,
        #[arg(long)]
        exercise: Option<String>,
        /// Direct workout_exercise id (alternative to --workout + --exercise)
        #[arg(long = "workout-exercise")]
        workout_exercise: Option<i64>,
        #[arg(long)]
        reps: Option<i32>,
        #[arg(long)]
        weight: Option<f64>,
        #[arg(long = "external-load")]
        external_load: Option<f64>,
        #[arg(long = "no-weight-recorded")]
        no_weight_recorded: bool,
        #[arg(long)]
        duration: Option<i32>,
        #[arg(long)]
        distance: Option<f64>,
        #[arg(long)]
        rpe: Option<f64>,
        #[arg(long)]
        rir: Option<f64>,
        #[arg(long = "effective-reps")]
        effective_reps: Option<i32>,
        #[arg(long = "rest")]
        rest_seconds: Option<i32>,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long, value_parser = ["left", "right", "both"])]
        side: Option<String>,
        #[arg(long, default_value = "full")]
        phase: String,
        #[arg(long = "avg-heart-rate")]
        avg_heart_rate: Option<f64>,
        #[arg(long = "max-heart-rate")]
        max_heart_rate: Option<f64>,
        #[arg(long = "hr-zones")]
        hr_zones: Option<String>,
        #[arg(long)]
        pace: Option<f64>,
        #[arg(long)]
        calories: Option<i32>,
        #[arg(long)]
        laps: Option<String>,
        /// Average cadence (steps per minute)
        #[arg(long)]
        cadence: Option<f64>,
        /// Total ascent in meters
        #[arg(long)]
        ascent: Option<f64>,
        /// Total descent in meters
        #[arg(long)]
        descent: Option<f64>,
        /// Validate and show resolved payload without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add a cardio-focused set (distance, duration, HR, pace).
    #[command(name = "add-cardio")]
    AddCardio {
        #[arg(long)]
        workout: Option<i64>,
        #[arg(long)]
        exercise: Option<String>,
        #[arg(long = "workout-exercise")]
        workout_exercise: Option<i64>,
        #[arg(long)]
        distance: f64,
        #[arg(long)]
        duration: i32,
        #[arg(long = "avg-heart-rate")]
        avg_heart_rate: f64,
        #[arg(long = "max-heart-rate")]
        max_heart_rate: f64,
        #[arg(long)]
        pace: f64,
        #[arg(long)]
        calories: i32,
        #[arg(long = "hr-zones")]
        hr_zones: Option<String>,
        #[arg(long)]
        laps: Option<String>,
        /// Average cadence (steps per minute)
        #[arg(long)]
        cadence: Option<f64>,
        /// Total ascent in meters
        #[arg(long)]
        ascent: Option<f64>,
        /// Total descent in meters
        #[arg(long)]
        descent: Option<f64>,
        /// Require --hr-zones and --laps (repslog-style strict cardio)
        #[arg(long = "require-zones-laps")]
        require_zones_laps: bool,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long, default_value = "full")]
        phase: String,
        /// Validate and show resolved payload without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add a rest-pause/cluster sequence (comma-separated reps/rir/effective-reps).
    #[command(name = "add-cluster")]
    AddCluster {
        #[arg(long)]
        workout: Option<i64>,
        #[arg(long)]
        exercise: Option<String>,
        #[arg(long = "workout-exercise")]
        workout_exercise: Option<i64>,
        #[arg(long)]
        weight: Option<f64>,
        #[arg(long = "external-load")]
        external_load: Option<f64>,
        #[arg(long = "no-weight-recorded")]
        no_weight_recorded: bool,
        /// Comma-separated reps e.g. "10,5,5"
        #[arg(long)]
        reps: String,
        /// Comma-separated RIR e.g. "0,0,1"
        #[arg(long)]
        rir: String,
        /// Comma-separated effective reps e.g. "6,4,3"
        #[arg(long = "effective-reps")]
        effective_reps: String,
        #[arg(long = "rest")]
        rest_seconds: i32,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long, value_parser = ["left", "right", "both"])]
        side: Option<String>,
        #[arg(long, default_value = "full")]
        phase: String,
        /// Validate and show resolved payload without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add left/right (or both) sets for unilateral work.
    #[command(name = "add-unilateral")]
    AddUnilateral {
        #[arg(long)]
        workout: Option<i64>,
        #[arg(long)]
        exercise: Option<String>,
        #[arg(long = "workout-exercise")]
        workout_exercise: Option<i64>,
        /// Comma-separated reps
        #[arg(long)]
        reps: String,
        #[arg(long)]
        weight: Option<f64>,
        #[arg(long = "external-load")]
        external_load: Option<f64>,
        #[arg(long = "no-weight-recorded")]
        no_weight_recorded: bool,
        #[arg(long)]
        rir: Option<String>,
        #[arg(long = "effective-reps")]
        effective_reps: Option<String>,
        #[arg(long = "rest")]
        rest_seconds: Option<i32>,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long, value_parser = ["left", "right", "both"], default_value = "both")]
        side: String,
        #[arg(long, default_value = "full")]
        phase: String,
        /// Validate and show resolved payload without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// List sets for a workout_exercise id.
    List {
        #[arg(long = "workout-exercise")]
        workout_exercise: i64,
    },
    /// Add exercise to workout and optionally log the first set.
    Quick {
        #[arg(long)]
        workout: i64,
        #[arg(long)]
        exercise: String,
        #[arg(long)]
        reps: Option<i32>,
        #[arg(long)]
        weight: Option<f64>,
        #[arg(long = "external-load")]
        external_load: Option<f64>,
        #[arg(long = "no-weight-recorded")]
        no_weight_recorded: bool,
        #[arg(long)]
        duration: Option<i32>,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long)]
        phase: Option<String>,
        /// Validate and show resolved payload without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Update fields on an existing set.
    Update {
        id: i64,
        #[arg(long)]
        reps: Option<i32>,
        #[arg(long)]
        weight: Option<f64>,
        #[arg(long = "external-load")]
        external_load: Option<f64>,
        #[arg(long)]
        duration: Option<i32>,
        #[arg(long)]
        distance: Option<f64>,
        #[arg(long)]
        rpe: Option<f64>,
        #[arg(long)]
        rir: Option<f64>,
        #[arg(long = "effective-reps")]
        effective_reps: Option<i32>,
        #[arg(long = "rest")]
        rest_seconds: Option<i32>,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long, value_parser = ["left", "right", "both"])]
        side: Option<String>,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long = "avg-heart-rate")]
        avg_heart_rate: Option<f64>,
        #[arg(long = "max-heart-rate")]
        max_heart_rate: Option<f64>,
        #[arg(long)]
        pace: Option<f64>,
        #[arg(long)]
        calories: Option<i32>,
        /// Average cadence (steps per minute)
        #[arg(long)]
        cadence: Option<f64>,
        /// Total ascent in meters
        #[arg(long)]
        ascent: Option<f64>,
        /// Total descent in meters
        #[arg(long)]
        descent: Option<f64>,
        #[arg(long = "hr-zones")]
        hr_zones: Option<String>,
        #[arg(long)]
        laps: Option<String>,
        /// Validate and show resolved payload without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Reorder a set within its workout-exercise.
    Move {
        id: i64,
        /// Target 1-based position
        #[arg(long)]
        to: i32,
        /// Show what would be moved without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a set by id.
    Delete {
        id: i64,
        /// Show what would be deleted without writing.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Actions under `recomplog body ...`
#[derive(Subcommand, Debug, Clone)]
pub enum BodyAction {
    /// Daily body composition measurements.
    Measurement {
        #[command(subcommand)]
        action: MeasurementAction,
    },
    /// Sleep sessions.
    Sleep {
        #[command(subcommand)]
        action: Box<SleepAction>,
    },
    /// User profile (height, date of birth) used for derived metrics.
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProfileAction {
    /// Set or update profile fields.
    Set(ProfileSetArgs),
    /// Show current profile.
    Show,
}

#[derive(Args, Debug, Clone)]
pub struct ProfileSetArgs {
    #[arg(long, value_name = "CM")]
    pub height_cm: Option<f64>,
    #[arg(long, value_name = "DATE")]
    pub date_of_birth: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MeasurementAction {
    /// Create / log a new daily measurement (one per date).
    Create(CreateMeasurementArgs),
    /// List measurements.
    List(ListArgs),
    /// Rolling medians over a trailing calendar-day window.
    Medians(MediansArgs),
    /// Show a single measurement.
    Show(ShowArgs),
    /// Update fields on an existing measurement.
    Update(UpdateMeasurementArgs),
    /// Delete a measurement.
    Delete(DeleteArgs),
}

#[derive(Args, Debug, Clone)]
pub struct CreateMeasurementArgs {
    #[arg(long, default_value = "today")]
    pub date: String,
    #[arg(long)]
    pub weight_kg: Option<f64>,
    #[arg(long)]
    pub body_fat_pct: Option<f64>,
    #[arg(long)]
    pub skeletal_muscle_pct: Option<f64>,
    #[arg(long)]
    pub visceral_fat_level: Option<i64>,
    #[arg(long)]
    pub bmi: Option<f64>,
    #[arg(long)]
    pub resting_metabolism_kcal: Option<i64>,
    /// Skip delta-vs-previous warnings.
    #[arg(long)]
    pub no_sanity_check: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ListArgs {
    #[arg(long)]
    pub days: Option<i64>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub until: Option<String>,
}

/// Args for `body measurement medians`.
#[derive(Args, Debug, Clone)]
pub struct MediansArgs {
    /// Trailing calendar-day window length (inclusive of the row date).
    #[arg(long, value_name = "DAYS", default_value = "7")]
    pub window: i64,
    #[arg(long)]
    pub days: Option<i64>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub until: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ShowArgs {
    #[arg(long)]
    pub id: Option<i64>,
    #[arg(long)]
    pub date: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct UpdateMeasurementArgs {
    #[arg(long)]
    pub id: Option<i64>,
    #[arg(long)]
    pub date: Option<String>,
    #[arg(long)]
    pub weight_kg: Option<f64>,
    #[arg(long)]
    pub body_fat_pct: Option<f64>,
    #[arg(long)]
    pub skeletal_muscle_pct: Option<f64>,
    #[arg(long)]
    pub visceral_fat_level: Option<i64>,
    #[arg(long)]
    pub bmi: Option<f64>,
    #[arg(long)]
    pub resting_metabolism_kcal: Option<i64>,
    #[arg(long)]
    pub no_sanity_check: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DeleteArgs {
    #[arg(long)]
    pub id: Option<i64>,
    #[arg(long)]
    pub date: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SleepAction {
    /// Create / log a sleep entry (one per wake-up date).
    Create(SleepCreateArgs),
    /// List sleep entries (newest first).
    List(ListArgs),
    /// Show a single sleep entry by id or --date.
    Show(ShowArgs),
    /// Update fields on an existing sleep entry.
    Update(SleepUpdateArgs),
    /// Delete a sleep entry.
    Delete(DeleteArgs),
}

#[derive(Args, Debug, Clone)]
pub struct SleepCreateArgs {
    /// Wake-up date (local calendar day).
    #[arg(long, default_value = "today")]
    pub date: String,
    #[arg(long)]
    pub bedtime: Option<String>,
    #[arg(long)]
    pub wake_time: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub time_in_bed: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub total_sleep: Option<String>,
    #[arg(long, value_name = "DURATION", alias = "rem-minutes")]
    pub rem: Option<String>,
    #[arg(long, value_name = "DURATION", alias = "deep-minutes")]
    pub deep: Option<String>,
    #[arg(long, value_name = "DURATION", alias = "light-minutes")]
    pub light: Option<String>,
    #[arg(long, value_name = "DURATION", alias = "awake-minutes")]
    pub awake: Option<String>,
    #[arg(long)]
    pub sleep_efficiency: Option<f64>,
    #[arg(long)]
    pub sleep_score: Option<i64>,
    #[arg(long)]
    pub quality: Option<i64>,
    #[arg(long)]
    pub awakenings: Option<i64>,
    #[arg(long, alias = "heart-rate-bpm")]
    pub heart_rate: Option<f64>,
    #[arg(long, alias = "hypopnea-per-hr")]
    pub hypopnea: Option<f64>,
    #[arg(long)]
    pub respiratory_rate: Option<f64>,
    #[arg(long)]
    pub notes: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct SleepUpdateArgs {
    pub id: Option<i64>,
    #[arg(long)]
    pub date: Option<String>,
    #[arg(long)]
    pub bedtime: Option<String>,
    #[arg(long)]
    pub wake_time: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub time_in_bed: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub total_sleep: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub rem: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub deep: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub light: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub awake: Option<String>,
    #[arg(long)]
    pub sleep_efficiency: Option<f64>,
    #[arg(long)]
    pub sleep_score: Option<i64>,
    #[arg(long)]
    pub quality: Option<i64>,
    #[arg(long)]
    pub awakenings: Option<i64>,
    #[arg(long, alias = "heart-rate-bpm")]
    pub heart_rate: Option<f64>,
    #[arg(long, alias = "hypopnea-per-hr")]
    pub hypopnea: Option<f64>,
    #[arg(long)]
    pub respiratory_rate: Option<f64>,
    #[arg(long)]
    pub notes: Option<String>,
}

/// Actions under `recomplog nutrition ...`
#[derive(Subcommand, Debug, Clone)]
pub enum NutritionAction {
    Product {
        #[command(subcommand)]
        action: ProductAction,
    },
    Purchase {
        #[command(subcommand)]
        action: PurchaseAction,
    },
    Consumption {
        #[command(subcommand)]
        action: ConsumptionAction,
    },
    Nutrient {
        #[command(subcommand)]
        action: NutrientAction,
    },
    #[command(name = "product-tag")]
    ProductTag {
        #[command(subcommand)]
        action: TaxonomyAction,
    },
    Store {
        #[command(subcommand)]
        action: StoreAction,
    },
    #[command(name = "store-tag")]
    StoreTag {
        #[command(subcommand)]
        action: TaxonomyAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProductAction {
    Create {
        name: String,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
    },
    List,
    Search {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        tag: Option<String>,
    },
    Show {
        id: i64,
    },
    Rename {
        id: i64,
        #[arg(long)]
        name: String,
    },
    Delete {
        id: i64,
        #[arg(long)]
        force: bool,
    },
    /// Set macros + micronutrients for a product.
    Nutrition {
        #[command(subcommand)]
        action: ProductNutritionAction,
    },
    Tag {
        #[command(subcommand)]
        action: TagModifyAction,
    },
    /// Back-compat aliases
    #[command(name = "set")]
    SetLegacy {
        id: i64,
        #[arg(long, default_value = "100")]
        reference_quantity: f64,
        /// `g` (mass), `ml` (volume), or `unit` (package).
        #[arg(long, default_value = "g")]
        reference_unit: String,
        #[arg(long)]
        energy_kcal: Option<f64>,
        #[arg(long)]
        protein_g: Option<f64>,
        #[arg(long)]
        carbohydrates_g: Option<f64>,
        #[arg(long)]
        fat_g: Option<f64>,
        #[arg(long)]
        fiber_g: Option<f64>,
        #[arg(long)]
        sugars_g: Option<f64>,
    },
    #[command(name = "tag-add")]
    TagAdd {
        id: i64,
        tag: String,
    },
    #[command(name = "tag-remove")]
    TagRemove {
        id: i64,
        tag: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProductNutritionAction {
    Set {
        id: i64,
        /// Amount of food the macros describe (e.g. 100 for 100 g, 1 for one package).
        #[arg(long)]
        reference_quantity: Option<f64>,
        /// How the reference amount is measured: `g` (mass), `ml` (volume),
        /// or `unit` (one package/item). Aliases like `bar`/`capsule` → `unit`.
        #[arg(long)]
        reference_unit: Option<String>,
        #[arg(long)]
        energy_kcal: Option<f64>,
        #[arg(long)]
        protein_g: Option<f64>,
        #[arg(long)]
        carbohydrates_g: Option<f64>,
        #[arg(long)]
        fat_g: Option<f64>,
        #[arg(long)]
        fiber_g: Option<f64>,
        #[arg(long)]
        sugars_g: Option<f64>,
        /// Repeatable: --micronutrient NAME AMOUNT UNIT
        #[arg(long, value_names = ["NAME", "AMOUNT", "UNIT"], num_args = 3, action = clap::ArgAction::Append)]
        micronutrient: Vec<String>,
        #[arg(long = "json-file")]
        json_file: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum TagModifyAction {
    Add {
        id: i64,
        #[arg(long)]
        tag: String,
    },
    Remove {
        id: i64,
        #[arg(long)]
        tag: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PurchaseAction {
    Create {
        #[arg(long)]
        product: i64,
        #[arg(long, default_value_t = 1.0)]
        quantity: f64,
        #[arg(long)]
        price: Option<String>,
        #[arg(long)]
        store: Option<i64>,
        /// Purchase instant as RFC3339 (e.g. 2026-07-14T18:30:00-03:00). Required.
        #[arg(long)]
        date: String,
    },
    List {
        /// Flexible calendar day: today, yesterday, YYYY-MM-DD, …
        #[arg(long)]
        since: Option<String>,
        /// Flexible calendar day: today, yesterday, YYYY-MM-DD, …
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        product: Option<i64>,
        #[arg(long)]
        store: Option<i64>,
    },
    Show {
        id: i64,
    },
    Delete {
        id: i64,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConsumptionAction {
    Create {
        #[arg(long)]
        product: i64,
        /// Amount consumed, in `--unit` (defaults to the product’s reference unit).
        #[arg(long)]
        quantity: f64,
        /// Must match the product unit kind: `g`/`ml`/`unit` (package).
        /// Omit to use the product’s reference unit. Aliases: bar, cup, capsule → unit.
        #[arg(long)]
        unit: Option<String>,
        /// Consumption instant as RFC3339 (e.g. 2026-07-14T13:45:00-03:00). Required.
        #[arg(long)]
        date: String,
        /// Allow logging at local midnight (discouraged; usually a missing time-of-day).
        #[arg(long = "allow-midnight")]
        allow_midnight: bool,
    },
    List {
        /// Flexible calendar day: today, yesterday, YYYY-MM-DD, …
        #[arg(long)]
        since: Option<String>,
        /// Flexible calendar day: today, yesterday, YYYY-MM-DD, …
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        product: Option<i64>,
    },
    Delete {
        id: i64,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum NutrientAction {
    List,
    Create {
        name: String,
        #[arg(long)]
        unit: String,
        #[arg(long)]
        recommended_intake: Option<f64>,
    },
    Show {
        id: i64,
    },
    Search {
        query: String,
    },
    Delete {
        id: i64,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum TaxonomyAction {
    Create { name: String },
    List,
    Search { query: String },
    Show { id: i64 },
    Delete { id: i64 },
}

#[derive(Subcommand, Debug, Clone)]
pub enum StoreAction {
    Create {
        name: String,
    },
    List,
    Show {
        id: i64,
    },
    Rename {
        id: i64,
        #[arg(long)]
        name: String,
    },
    Delete {
        id: i64,
    },
    Tag {
        #[command(subcommand)]
        action: TagModifyAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ReportAction {
    /// Nutrition summary over a period.
    Nutrition {
        #[command(subcommand)]
        action: NutritionReportAction,
    },
    /// Body measurement trends + stats.
    Body {
        #[command(subcommand)]
        action: BodyReportAction,
    },
    /// Sleep trends and averages.
    Sleep(ReportRangeArgs),
    /// Combined recomposition dashboard data (JSON for agents).
    Summary(SummaryArgs),
    /// Multi-section terminal brief: focal-day consumption + full workout detail,
    /// then N-day nutrition / body / sleep / previous workouts overview.
    ///
    /// Replaces the multi-tool shell habit of listing consumption, nutrition,
    /// measurements, and sleep in one shot. Focal-day workouts use the same
    /// detail as `workout show` (exercises + sets). Use `--date` to anchor the
    /// brief on a day other than today.
    Brief {
        /// Lookback for nutrition, measurements, sleep, and previous workouts.
        /// Focal-day consumption and workouts use `--date` (default: today).
        #[arg(short, long, default_value_t = 7)]
        days: u32,
        /// Anchor day for the brief (consumption + workouts that day; lookback ends here).
        /// Flexible: today, yesterday, YYYY-MM-DD, last monday, …
        #[arg(long)]
        date: Option<String>,
    },
    /// Generate a self-contained mobile-friendly HTML dashboard report.
    Html {
        #[arg(short, long, default_value = "7")]
        days: u32,
        #[arg(short, long, default_value = ".")]
        output_dir: String,
        #[arg(long, default_value = "index.html")]
        name: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum BodyReportAction {
    /// Compact summary across all key metrics.
    Summary(SummaryArgs),
    Weight(ReportRangeArgs),
    #[command(name = "body-fat")]
    BodyFat(ReportRangeArgs),
    Muscle(ReportRangeArgs),
    #[command(name = "visceral-fat")]
    VisceralFat(ReportRangeArgs),
    Bmi(ReportRangeArgs),
    #[command(name = "resting-metabolism")]
    RestingMetabolism(ReportRangeArgs),
    /// Alias: same as summary for `report body --days N` style via default
    #[command(name = "list")]
    List(SummaryArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ReportRangeArgs {
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub until: Option<String>,
    #[arg(long)]
    pub days: Option<i64>,
}

#[derive(Args, Debug, Clone)]
pub struct SummaryArgs {
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub until: Option<String>,
    #[arg(long)]
    pub days: Option<i64>,
    #[arg(long)]
    pub period: Option<String>,
}

/// Date range flags shared by nutrition report subcommands.
#[derive(Args, Debug, Clone)]
pub struct NutritionPeriodArgs {
    /// Start of period (inclusive). Flexible: today, yesterday, 2026-05-01, etc.
    #[arg(long)]
    pub since: Option<String>,
    /// End of period (inclusive). Same flexible date formats as --since.
    #[arg(long)]
    pub until: Option<String>,
    /// Last N calendar days inclusive of today. Cannot be combined with --since/--until.
    #[arg(long, conflicts_with_all = ["since", "until"])]
    pub days: Option<u32>,
}

/// Which macro nutrient(s) to show in per-day nutrition list output.
#[derive(Clone, Copy, Debug, ValueEnum, Eq, PartialEq)]
pub enum NutritionReportValue {
    /// All tracked macros (energy, protein, carbohydrates, fat, fiber, sugars).
    Macronutrients,
    /// Energy only (kcal).
    Calories,
    /// Protein only (g).
    Protein,
    /// Carbohydrates only (g).
    Carbohydrates,
    /// Fat only (g).
    Fat,
    /// Fiber only (g).
    Fiber,
    /// Sugars only (g).
    Sugars,
}

impl NutritionReportValue {
    pub fn label(self) -> &'static str {
        match self {
            Self::Macronutrients => "macronutrients",
            Self::Calories => "calories",
            Self::Protein => "protein",
            Self::Carbohydrates => "carbohydrates",
            Self::Fat => "fat",
            Self::Fiber => "fiber",
            Self::Sugars => "sugars",
        }
    }
}

/// Spending report grouping mode.
#[derive(Clone, Copy, Debug, ValueEnum, Eq, PartialEq, Default)]
pub enum SpendingBy {
    /// Total only (by_store breakdown still included in JSON).
    #[default]
    Total,
    /// Emphasize store breakdown (same data as total; human output identical).
    Store,
    /// Also break down by product.
    Product,
}

#[derive(Subcommand, Debug, Clone)]
pub enum NutritionReportAction {
    /// Aggregate nutrition totals for a period (macros + micronutrients).
    Summary(NutritionPeriodArgs),
    /// Per-day nutrition breakdown (not per consumption line).
    List {
        #[command(flatten)]
        period: NutritionPeriodArgs,
        /// Which macro value(s) to show per day.
        #[arg(long, value_enum, default_value_t = NutritionReportValue::Macronutrients)]
        value: NutritionReportValue,
    },
    /// Spending totals over a period, optionally grouped by product.
    Spending {
        #[command(flatten)]
        period: NutritionPeriodArgs,
        /// Group by: total, store, or product (by_store always present in JSON).
        #[arg(long, value_enum, default_value_t = SpendingBy::Total)]
        by: SpendingBy,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ImportAction {
    /// Import a FIT file (Garmin/Zepp/Amazfit running or other activities).
    Fit {
        /// Path to the .fit file
        path: String,
        /// Override exercise name (default: FIT session.sport). Must exist in catalog.
        #[arg(long)]
        exercise: Option<String>,
        /// Workout type label (default: Run)
        #[arg(long = "type")]
        workout_type: Option<String>,
        #[arg(short, long)]
        notes: Option<String>,
        /// Allow re-import of a previously imported file
        #[arg(long)]
        force: bool,
        /// HR zone upper bounds bpm zones 1-5 (comma-separated)
        #[arg(long = "hr-zone-bounds")]
        hr_zone_bounds: Option<String>,
        /// Skip deriving zones from user profile / sleep
        #[arg(long = "no-profile-hr")]
        no_profile_hr: bool,
        /// Show what would be imported
        #[arg(long)]
        dry_run: bool,
    },
    /// Import data from a legacy tool database (repslog.db, bodylog.db, or nutlog.db).
    ///
    /// This is the recommended way to migrate from the previous separate tools.
    /// The importer detects the source schema and copies relevant rows.
    Legacy {
        /// Path to the old database file.
        #[arg(long, value_name = "PATH")]
        from_db: String,
        /// Only import this domain (workout|body|nutrition). Default: all that are detected.
        #[arg(long)]
        domain: Option<String>,
        /// Dry-run: show what would be imported without writing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigAction {
    /// Show or edit config (sanity limits etc.).
    Show,
    /// Generate a default config file.
    Generate {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        force: bool,
    },
    Path,
}

/// Top-level `check` command: optional subcommand, else sanity-limit audit.
#[derive(Args, Debug, Clone)]
pub struct CheckCommand {
    #[command(subcommand)]
    pub action: Option<CheckAction>,
    /// Sanity-limit audit flags (used when no subcommand is given).
    #[command(flatten)]
    pub audit: CheckArgs,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CheckAction {
    /// Detect missing daily logs (measurement, sleep, nutrition) and workout inactivity.
    Missing(CheckMissingArgs),
}

/// Args for `recomplog check missing`.
#[derive(Args, Debug, Clone)]
pub struct CheckMissingArgs {
    /// Calendar days to scan for measurement / sleep / nutrition (includes today).
    #[arg(long, default_value_t = 7)]
    pub days: u32,
    /// Fail if no workout session falls in this many calendar days (includes today).
    #[arg(long = "workout-days", default_value_t = 3)]
    pub workout_days: u32,
}

/// Args for bare `recomplog check` (sanity-limit audit).
#[derive(Args, Debug, Clone)]
pub struct CheckArgs {
    #[arg(long, alias = "deltas")]
    pub variations: bool,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub until: Option<String>,
    #[arg(long)]
    pub days: Option<i64>,
}
