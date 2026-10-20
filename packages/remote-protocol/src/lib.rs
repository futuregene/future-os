//! Application-independent Remote wire contract.

mod command_generated;
pub mod files;
pub mod handshake;
pub mod invitation;
pub mod json;
pub mod jwt;
pub use command_generated::{IncomingCmd, UploadReference};
pub use future_remote_crypto as crypto;

/// Clients that declare this capability implement lean event/history reads.
pub const LEAN_EVENTS_FEATURE: &str = "lean_events_v1";

pub const EVENT_COALESCING_FEATURE: &str = "event_coalescing_v1";
pub const CATALOG_ACTIVITY_FEATURE: &str = "catalog_activity_v1";
pub const REPLY_GZIP_FEATURE: &str = "reply_gzip_v1";

/// Clients that declare this capability keep proving they are still there:
/// a `client_presence` beat while they are, and `client_disconnect` when they
/// leave. A client that does not declare it is never presumed gone for being
/// silent — it never promised to speak.
pub const CLIENT_PRESENCE_FEATURE: &str = "client_presence_v1";
