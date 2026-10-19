//! Application-independent Remote controlled endpoint.
//! Explicit host services, role supervision, secure transport and bounded shutdown.
pub mod commands;
pub mod connection;
mod credentials;
pub mod diagnostics;
mod error;
pub mod events;
pub mod health;
pub mod lean;
pub mod lifecycle;
pub mod pairing;
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
