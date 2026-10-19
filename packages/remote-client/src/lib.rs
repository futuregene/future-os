//! Reusable Remote controller connection, independent of application storage/UI.

pub mod credentials;
mod error;
pub mod session;
pub use error::Error;
pub(crate) use future_remote_protocol::json as codec;
pub use session::ConnectionOptions;
