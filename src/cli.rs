use clap::{Args, Parser, Subcommand};

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
        action: WorkoutAction,
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

    /// Audit database contents against configured sanity limits.
    Check(CheckArgs),

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
        /// Start time (YYYY-MM-DD HH:MM:SS or flexible)
        #[arg(long)]
        started_at: Option<String>,
        /// e.g. Push, Pull, Run, Full Body
        #[arg(long = "type")]
        workout_type: Option<String>,
        #[arg(long)]
        notes: Option<String>,
    },
    /// List recent workouts.
    List {
        #[arg(long)]
        days: Option<i64>,
    },
    /// Show a workout with its exercises and sets.
    Show { id: i64 },
    /// Delete a workout (cascades exercises/sets).
    Delete { id: i64 },

    /// Exercise catalog operations (under the workout group).
    Exercise {
        #[command(subcommand)]
        action: ExerciseAction,
    },

    /// Set logging operations (under the workout group).
    Set {
        #[command(subcommand)]
        action: SetAction,
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
        /// Target workout id
        #[arg(long)]
        workout: i64,
        /// Exercise name or id
        #[arg(long)]
        exercise: String,
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
        #[arg(long, default_value = "working")]
        phase: String,
    },
    /// Add a cardio-focused set.
    #[command(name = "add-cardio")]
    AddCardio {
        #[arg(long)]
        workout: i64,
        #[arg(long)]
        exercise: String,
        #[arg(long)]
        distance: Option<f64>,
        #[arg(long)]
        duration: Option<i32>,
        #[arg(long = "avg-heart-rate")]
        avg_heart_rate: Option<f64>,
        #[arg(long = "max-heart-rate")]
        max_heart_rate: Option<f64>,
        #[arg(long)]
        pace: Option<f64>,
        #[arg(long)]
        calories: Option<i32>,
        #[arg(long)]
        notes: Option<String>,
    },
    /// Delete a set by id.
    Delete { id: i64 },
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
    // product-tag, store etc. can be added here later
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
        new_name: String,
    },
    Delete {
        id: i64,
    },
    /// Set macro nutrition per reference quantity.
    #[command(name = "set")]
    Set {
        id: i64,
        #[arg(long, default_value = "100")]
        reference_quantity: f64,
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
    /// Add a tag to a product.
    #[command(name = "tag-add")]
    TagAdd {
        id: i64,
        tag: String,
    },
    /// Remove a tag from a product.
    #[command(name = "tag-remove")]
    TagRemove {
        id: i64,
        tag: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PurchaseAction {
    Create {
        #[arg(long)]
        product: i64,
        #[arg(long)]
        quantity: f64,
        #[arg(long)]
        price: Option<String>,
        #[arg(long)]
        store: Option<i64>,
    },
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConsumptionAction {
    Create {
        #[arg(long)]
        product: i64,
        #[arg(long)]
        quantity: f64,
        #[arg(long)]
        date: Option<String>,
    },
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum NutrientAction {
    List,
    Create {
        name: String,
        unit: String,
        #[arg(long)]
        recommended_intake: Option<f64>,
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

#[derive(Subcommand, Debug, Clone)]
pub enum NutritionReportAction {
    /// Daily / period nutrition totals.
    List {
        #[arg(long)]
        days: Option<u32>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
    },
    /// Spending totals over a period.
    Spending {
        #[arg(long)]
        days: Option<u32>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ImportAction {
    /// Import a FIT file (Garmin/Zepp/Amazfit running or other activities).
    Fit {
        path: String,
        #[arg(long)]
        exercise: Option<String>,
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
