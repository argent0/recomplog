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
        // ... (cardio, rpe, etc. will be expanded)
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
        action: SleepAction,
    },
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
    Create {
        #[arg(long, default_value = "today")]
        date: String,
        #[arg(long)]
        total_sleep: Option<String>,
        #[arg(long)]
        rem: Option<String>,
        #[arg(long)]
        deep: Option<String>,
        #[arg(long)]
        light: Option<String>,
        #[arg(long)]
        awake: Option<String>,
    },
    List(ListArgs),
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
    // add more (rename, nutrition set, delete, tag) later
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
        #[arg(long)]
        days: Option<u32>,
    },
    /// Combined recomposition dashboard data (JSON for agents).
    Summary {
        #[arg(long)]
        days: Option<u32>,
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
pub enum NutritionReportAction {
    List {
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
    pub days: Option<i64>,
}
