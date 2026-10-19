//! Desktop adapters for the shared Remote Server runtime.
//! Connection supervision, command admission and transport live in
//! `future_remote_server`; this module supplies application services, event
//! attachment, operator settings, validation HTTP and GUI/headless entry points.
//! The actual prompt/persistence contract remains in the application host.

pub(crate) mod commands;
pub(crate) mod control;
mod diagnostics;
pub(crate) mod pairing;
#[cfg(test)]
mod presence;
mod publisher;
#[cfg(test)]
mod real_network;
pub(crate) mod relay;
#[cfg(test)]
mod relay_e2e;
pub(crate) mod secure;
mod supervisor;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
pub(crate) mod transfer;
#[cfg(test)]
mod transport;
pub(crate) mod trust;
mod types;
#[cfg(test)]
mod verify_e2e;
mod web_server;

use self::diagnostics::*;
#[cfg(test)]
use self::presence::*;
use self::publisher::*;
#[allow(unused_imports)]
pub use self::publisher::*;
use self::supervisor::*;
#[allow(unused_imports)]
pub use self::supervisor::*;
#[cfg(test)]
use self::transport::*;
use self::types::*;
#[allow(unused_imports)]
pub use self::types::*;
use self::web_server::*;
#[cfg(test)]
use future_remote_server::health::*;

pub(crate) use future_remote_server::supervisor::unix_timestamp;
#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;

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

fn host() -> &'static dyn future_remote_server::services::RemoteHost {
    crate::remote_host::host()
}

#[cfg(test)]
pub(crate) fn reset_after_shutdown_for_test() {
    SUPERVISOR.reset_after_shutdown_for_test();
}
