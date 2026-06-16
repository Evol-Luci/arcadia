use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("could not resolve XDG base directories")]
    NoBaseDirs,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Unreachable(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("adapter error: {0}")]
    Adapter(#[from] AdapterError),
}

/// Adapters are fallible and capability-reporting; a misbehaving emulator
/// surfaces an `AdapterError` rather than panicking the engine.
#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("no adapter registered for id `{0}`")]
    UnknownAdapter(String),

    #[error("emulator executable not found: {0}")]
    ExecutableMissing(String),

    #[error("capability `{0}` not supported by this adapter")]
    Unsupported(String),

    #[error("launch failed: {0}")]
    Launch(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, EngineError>;
