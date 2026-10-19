//! Remote control runtime (embedded bridge) — connection lifecycle and event
//! mirroring. Command routing lives in [`commands`]; the prompt persist/finalize
//! contract lives in `agent_bridge::headless` (shared with any future headless
//! caller, so it can't drift from the frontend semantics).
//!
//! Design: see `docs/internals/desktop/CONNECTION.md`. The embedded bridge connects
//! with a short-lived, pair-scoped NATS user JWT, mirrors agent events, routes
//! Web/App commands through the GUI persistence path, publishes presence, and
//! refreshes credentials before expiry.

pub(crate) mod commands;
mod diagnostics;
mod health;
mod lifecycle;
pub(crate) mod pairing;
mod presence;
pub(crate) mod protocol;
mod publisher;
#[cfg(test)]
mod real_network;
pub(crate) mod relay;
#[cfg(test)]
mod relay_e2e;
pub(crate) mod secure;
pub(crate) mod services;
mod supervisor;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
pub(crate) mod transfer;
mod transport;
pub(crate) mod trust;
mod types;
#[cfg(test)]
mod verify_e2e;
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

/// Record that this process has a credential for `pair_id`.
///
/// Called by the host's credential store whenever a device is saved or removed,
/// so the control rules can answer "do we have this device?" without reading the
/// file on every command.
pub(crate) fn note_known_device(pair_id: &str) {
    SUPERVISOR.remember_device(pair_id);
}

pub(crate) fn forget_known_device(pair_id: &str) {
    SUPERVISOR.forget_device(pair_id);
}

fn host() -> &'static dyn services::RemoteHost {
    crate::remote_host::host()
}
