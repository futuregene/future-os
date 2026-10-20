//! Desktop adapters for the shared Remote Server runtime.
//! Connection supervision, command admission and transport live in
//! `future_remote_server`; this module supplies application services, event
//! attachment, operator settings, validation HTTP and GUI/headless entry points.
//! The actual prompt/persistence contract remains in the application host.

pub(crate) mod commands;
pub mod control;
mod diagnostics;
pub mod pairing;
#[cfg(test)]
mod presence;
mod publisher;
#[cfg(test)]
mod real_network;
pub mod relay;
#[cfg(test)]
mod relay_e2e;
pub(crate) mod secure;
mod supervisor;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;
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

// Cargo can unify dev features into a real product binary. Merely exposing
// fixture helpers must not change its TLS, recovery or validation behavior.
#[cfg(any(test, feature = "test-support"))]
static FIXTURE_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn enable_fixture_mode() {
    FIXTURE_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn fixture_mode() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    {
        cfg!(test) || FIXTURE_MODE.load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        false
    }
}

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

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn reset_after_shutdown_for_test() {
    SUPERVISOR.reset_after_shutdown_for_test();
}

pub fn invitation_link(status: &RemoteStatus) -> Option<String> {
    let code = status.pairing_code.as_ref()?;
    if status
        .pairing_code_expires_at
        .is_none_or(|expiry| expiry <= unix_timestamp() as i64)
    {
        return None;
    }
    // Desktop owns the complete v2 invitation, including the local-only PSK.
    // Do not wrap it in another `code` parameter or reconstruct a v1 invite.
    let url = reqwest::Url::parse(code).ok()?;
    // v3 is the same invitation, minted by a self-hosted relay instead of the
    // platform: the QR still carries the Noise static key and the one-time
    // secret, and the phone still verifies them. Only the claim endpoint moved.
    if url.scheme() != "futureos"
        || url.host_str() != Some("remote")
        || url.path() != "/pair"
        || !url
            .query_pairs()
            .any(|(key, value)| key == "v" && matches!(&*value, "2" | "3"))
    {
        return None;
    }
    Some(code.clone())
}
