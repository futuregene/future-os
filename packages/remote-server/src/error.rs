//! Errors retain transport/platform categories without importing an application type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Host(Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{message}")]
    Remote {
        status: u16,
        code: Option<String>,
        message: String,
    },
    #[error("{0}")]
    RemoteTransport(String),
    #[error("{0}")]
    RemoteAuthorization(String),
    #[error(transparent)]
    AccountAuthorization(Box<dyn std::error::Error + Send + Sync>),
    #[error("{0}")]
    Message(String),
}
impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}
