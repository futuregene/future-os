//! Errors cross the host boundary without importing an application error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    RemoteTransport(String),
}
