use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum RecomplogError {
    #[error("measurement not found: {0}")]
    MeasurementNotFound(i64),

    #[error("measurement not found for date: {0}")]
    MeasurementNotFoundForDate(String),

    #[error(
        "measurement already exists for date {0}; use `body measurement update --date {0}` to modify"
    )]
    MeasurementExistsForDate(String),

    #[error(
        "sleep entry already exists for date {0}; use `body sleep update --date {0}` to modify"
    )]
    SleepExistsForDate(String),

    #[error("sleep entry not found: {0}")]
    SleepNotFound(i64),

    #[error("sleep entry not found for date: {0}")]
    SleepNotFoundForDate(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid date: {0}")]
    InvalidDate(String),

    #[error("invalid measurement: {0}")]
    InvalidMeasurement(String),

    #[error("invalid sleep: {0}")]
    InvalidSleep(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("invalid profile data: {0}")]
    InvalidProfile(String),

    #[error("invalid duration: {0}")]
    InvalidDuration(String),

    #[error("import error: {0}")]
    Import(String),

    #[error("sanity check failed: {0}")]
    Sanity(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, RecomplogError>;

impl From<anyhow::Error> for RecomplogError {
    fn from(e: anyhow::Error) -> Self {
        RecomplogError::Other(e.to_string())
    }
}
