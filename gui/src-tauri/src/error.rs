//! Unified application error.
//!
//! Wraps the common fallible sources (SQLite, IO) so `?` converts them
//! automatically — replacing the previous `.map_err(|e| e.to_string())` noise —
//! and carries free-form messages for everything else (gRPC/transport errors,
//! validation, not-found lookups). It serializes to a plain string, so the
//! Tauri command error contract seen by the frontend (`Result<T, string>`)
//! is unchanged.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Message(String),
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        AppError::Message(message)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        AppError::Message(message.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
