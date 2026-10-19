//! Application-independent Remote wire contract.

mod command_generated;
pub mod invitation;
pub mod json;
pub use command_generated::{IncomingCmd, UploadReference};
pub use future_remote_crypto as crypto;
