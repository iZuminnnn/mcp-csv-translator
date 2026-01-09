use thiserror::Error;

#[derive(Error, Debug)]
pub enum CsvTranslatorError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("CSV error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Token limit exceeded: current {current} > max {max}")]
    TokenLimitExceeded { current: usize, max: usize },

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Checkpoint error: {0}")]
    CheckpointError(String),

    #[error("Row count mismatch: expected {expected}, got {got}")]
    RowCountMismatch { expected: usize, got: usize },

    #[error("Resequencer buffer overflow: waiting for chunk {waiting_for}")]
    ResequencerOverflow { waiting_for: usize },

    #[error("Channel closed unexpectedly")]
    ChannelClosed,

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Translation failed after retries: {0}")]
    TranslationFailed(String),
}

pub type Result<T> = std::result::Result<T, CsvTranslatorError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryStrategy {
    ExpandContext,
    ReduceChunkSize,
    SkipRow,
    LowerTemperature,
}

impl RetryStrategy {
    pub fn next(&self) -> Option<Self> {
        match self {
            RetryStrategy::ExpandContext => Some(RetryStrategy::LowerTemperature),
            RetryStrategy::LowerTemperature => Some(RetryStrategy::ReduceChunkSize),
            RetryStrategy::ReduceChunkSize => Some(RetryStrategy::SkipRow),
            RetryStrategy::SkipRow => None,
        }
    }
}
