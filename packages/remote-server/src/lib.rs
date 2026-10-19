//! Application-independent Remote controlled endpoint.
//! Security, transport loops and wire policy are implemented; role supervision is migrating.
pub mod commands;
mod credentials;
pub mod diagnostics;
mod error;
pub mod events;
pub mod health;
pub mod lean;
pub mod lifecycle;
pub mod reply;
pub mod secure;
pub mod services;
pub mod supervisor;
mod tasks;
pub mod transfer;
pub mod types;
pub mod wire_limits;
pub use credentials::PairingCreds;
pub use error::Error;
