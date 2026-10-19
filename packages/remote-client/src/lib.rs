//! Reusable Remote controller connection, independent of application storage/UI.

pub mod credentials;
mod error;
pub mod session;
pub use error::Error;
pub(crate) use future_remote_protocol::json as codec;
pub use session::ConnectionOptions;

pub mod host;
pub mod runtime;
mod tasks;
pub use runtime::{Client, Emitter, PeerEvent, PeerSummary};
pub use tasks::TaskHandle;

pub mod transfer;
