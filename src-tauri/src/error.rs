use serde::{Serialize, Serializer};
use thiserror::Error;

/// Every error that can cross the IPC boundary.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Config(String),

    #[error("{0}")]
    Agent(String),

    #[error("No provider is configured. Open settings to add an API key.")]
    NoProvider,

    #[error("Provider '{0}' is not known")]
    UnknownProvider(String),

    #[error("'{command}' was not found on your PATH. {hint}")]
    MissingBinary { command: String, hint: String },

    #[error("{0}")]
    Keyring(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

impl From<keyring::Error> for AppError {
    fn from(value: keyring::Error) -> Self {
        AppError::Keyring(value.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
