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
#[allow(unused_imports)]
pub use self::publisher::*;
use self::supervisor::*;
#[allow(unused_imports)]
pub use self::supervisor::*;
#[cfg(test)]
use self::transport::*;
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

/// Remember the remote desktop switch across restarts.
///
/// The switch on the Remote page *is* the preference: "remote desktop on" and
/// "come back on at launch" are one state, not two, so turning it on or off
/// stores itself. Only the user's own switch writes here — the shutdown,
/// suspend and crash paths stop the bridge without touching this value, so
/// quitting the app is not the same as turning the remote desktop off.
///
/// Shared with the control channel, where `future remote enable|disable` is the
/// same switch reached from a terminal: two writers of one preference that
/// disagreed would make the window and the CLI describe different machines.
///
/// A failure is reported and swallowed: the bridge is already running (or
/// already stopped), which is what the switch renders. Failing the command over
/// an unwritten preference would leave the UI describing a state it cannot
/// show.
pub(crate) fn remember_switch(on: bool) {
    if let Err(error) = crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
        auto_connect_remote: Some(on),
        ..Default::default()
    }) {
        eprintln!("remote: could not store the switch state: {error}");
    }
}

/// Whether a phase describes a machine that is serving (and so is "on").
///
/// The same set the Remote page's switch renders as on: stopped, failed and
/// revoked are the three states that read as off. Everything else — including
/// a start that is retrying its connection — is the machine still trying to
/// serve, which is what the user asked for by turning it on.
pub(crate) fn is_on(phase: RemotePhase) -> bool {
    !matches!(
        phase,
        RemotePhase::Stopped | RemotePhase::Failed | RemotePhase::Revoked
    )
}

fn host() -> &'static dyn future_remote_server::services::RemoteHost {
    crate::remote_host::host()
}

#[cfg(test)]
pub(crate) fn reset_after_shutdown_for_test() {
    SUPERVISOR.reset_after_shutdown_for_test();
}
