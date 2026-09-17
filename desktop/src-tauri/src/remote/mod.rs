//! Remote control runtime (embedded bridge) — connection lifecycle and event
//! mirroring. Command routing lives in [`commands`]; the prompt persist/finalize
//! contract lives in `agent_bridge::headless` (shared with any future headless
//! caller, so it can't drift from the frontend semantics).
//!
//! Design: see `docs/internals/desktop/CONNECTION.md`. The embedded bridge connects
//! with a short-lived, pair-scoped NATS user JWT, mirrors agent events, routes
//! Web/App commands through the GUI persistence path, publishes presence, and
//! refreshes credentials before expiry.

mod commands;
mod diagnostics;
mod health;
mod lifecycle;
pub(crate) mod pairing;
mod presence;
pub(crate) mod protocol;
mod publisher;
pub(crate) mod secure;
pub(crate) mod services;
mod supervisor;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
pub(crate) mod transfer;
mod transport;
mod types;
mod web_server;

use self::diagnostics::*;
use self::health::*;
use self::presence::*;
use self::publisher::*;
#[allow(unused_imports)]
pub use self::publisher::*;
use self::supervisor::*;
#[allow(unused_imports)]
pub use self::supervisor::*;
use self::transport::*;
use self::types::*;
#[allow(unused_imports)]
pub use self::types::*;
use self::web_server::*;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    Arc, Mutex,
};
use std::{collections::HashMap, sync::LazyLock};

fn host() -> &'static dyn services::RemoteHost {
    crate::remote_host::host()
}
