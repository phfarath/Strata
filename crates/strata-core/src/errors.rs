use thiserror::Error;

#[derive(Error, Debug)]
pub enum StrataError {
    #[error("Database error: {0}")]
    Database(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Entity not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Tool execution error: {0}")]
    ToolError(String),

    #[error("Reasoning error: {0}")]
    Reasoning(String),

    #[error("Reasoning error: {0}")]
    ReasoningError(String),

    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("Execution failed with code {code:?}: {stderr}")]
    ExecutionFailed { code: Option<i32>, stderr: String },

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Sync error: {0}")]
    Sync(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("I/O error: {0}")]
    Io(String),
}

impl From<serde_json::Error> for StrataError {
    fn from(err: serde_json::Error) -> Self {
        StrataError::Serialization(err.to_string())
    }
}

impl From<std::io::Error> for StrataError {
    fn from(err: std::io::Error) -> Self {
        StrataError::Io(err.to_string())
    }
}
