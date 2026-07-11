use thiserror::Error;

/// Domain errors for recomplog (used by library-style code as it is developed).
/// The binary layer currently uses `anyhow` for simplicity during the merge.
#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum RecomplogError {
    #[error("database error: {0}")]
    Database(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("import error: {0}")]
    Import(String),
}

#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, RecomplogError>;
