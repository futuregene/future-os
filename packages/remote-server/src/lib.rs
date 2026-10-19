//! Application-independent Remote controlled endpoint.
//! Security endpoint extraction is implemented; role supervision is migrating.
mod credentials;
mod error;
pub mod secure;
pub mod services;
pub mod types;
pub use credentials::PairingCreds;
pub use error::Error;
