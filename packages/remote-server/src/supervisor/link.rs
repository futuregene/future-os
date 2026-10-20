//! Whether the paired device's client is actually at the other end.
//!
//! Every other thing this machine reports about a device describes *this*
//! machine: the bridge's NATS connection, its tasks, its credential refresh.
//! None of them says whether anyone is sitting on the other side. A phone that
//! was force-quit and a desktop client whose user pressed "disconnect" both
//! leave the bridge serving an empty room — and the page, reading only the
//! bridge, went on saying "in control" about a device that had left.
//!
//! So presence is tracked from the client's side of the wire:
//!
//! - **Speaking proves presence.** Every authenticated command the client sends
//!   (the handshake included) is a proof, which is why an idle client needs no
//!   new traffic to be believed.
//! - **Leaving is said out loud.** A client that stops deliberately sends
//!   `client_disconnect`, and this machine believes it at once.
//! - **A silent client is presumed gone, but only if it promised to speak.**
//!   A force-quit or a crash cannot say goodbye, so a client that declared
//!   [`CLIENT_PRESENCE_FEATURE`] also sends `client_presence` beats while it is
//!   there; missing them for [`CLIENT_PRESENCE_LEASE`] means it is not.
//!   A client that never declared the capability is *not* presumed gone: it
//!   sends no beats, and "silent" would be indistinguishable from "idle", so
//!   the honest answer is the old one (present once it has spoken at all).
//!
//! A client that has not spoken since the bridge came up is not present. That
//! is the truthful reading of an empty room, and it is what makes the page
//! correct in the first place rather than only after somebody leaves.

use super::*;
use std::time::Duration;

/// Capability a client declares on `secure_ready` when it will send
/// `client_presence` beats and a `client_disconnect` on the way out.
pub use future_remote_protocol::CLIENT_PRESENCE_FEATURE;

/// How long a declared heartbeat keeps the client "present".
///
/// Four times [`CLIENT_PRESENCE_BEAT`], so a beat lost to a network hiccup or a
/// briefly stalled event loop is not a departure, while a client that is
/// actually gone stops looking present within a poll or two of a minute.
pub const CLIENT_PRESENCE_LEASE: Duration = Duration::from_secs(60);

/// How often a client that declared [`CLIENT_PRESENCE_FEATURE`] sends a beat.
///
/// The client owns this interval (it is the one that has to keep the promise);
/// it is named here so the lease above and the client's timer are read together
/// and cannot drift apart silently. `packages/remote-client/src/client.ts`
/// sends at this interval.
pub const CLIENT_PRESENCE_BEAT: Duration = Duration::from_secs(15);

#[derive(Default)]
struct LinkState {
    /// When the client last proved it was there, unix milliseconds. `0` = it has
    /// not spoken since this pairing's bridge came up.
    seen_ms: u64,
    /// What the client declared on `secure_ready`. Only a client that took on
    /// the heartbeat promise is presumed gone when the beats stop.
    declared: bool,
    /// The client said it was leaving. Cleared by the next proof.
    departed: bool,
}

/// One pairing's view of its client. Shared across bridge generations: a
/// credential refresh is the same client reconnecting, not a new one.
#[derive(Default)]
pub struct ClientLink {
    state: Mutex<LinkState>,
}

impl ClientLink {
    /// The client did something authenticated: it is here.
    pub fn observe(&self) {
        let now = unix_timestamp_ms();
        let mut state = self.state.lock().unwrap();
        state.seen_ms = now;
        state.departed = false;
    }

    /// Record what the connection declared. A client that stops declaring the
    /// capability stops being presumed gone — the declaration is authoritative,
    /// exactly like the other per-connection capabilities.
    pub fn declare(&self, declared: bool) {
        self.state.lock().unwrap().declared = declared;
    }

    /// The client said it is leaving (see `client_disconnect`).
    pub fn depart(&self) {
        let mut state = self.state.lock().unwrap();
        state.departed = true;
    }

    /// Whether the client is, as far as this machine can tell, there.
    pub fn present(&self) -> bool {
        self.present_at(unix_timestamp_ms())
    }

    /// [`ClientLink::present`] with the clock supplied, so the lease is testable
    /// without sleeping for a minute.
    pub fn present_at(&self, now_ms: u64) -> bool {
        let state = self.state.lock().unwrap();
        if state.departed || state.seen_ms == 0 {
            return false;
        }
        if !state.declared {
            // An older client keeps the old reading: it spoke, so it is here.
            // It never promised beats, so silence says nothing about it.
            return true;
        }
        now_ms.saturating_sub(state.seen_ms) <= CLIENT_PRESENCE_LEASE.as_millis() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bridge_whose_client_never_spoke_serves_an_empty_room() {
        let link = ClientLink::default();
        assert!(!link.present_at(1_000));
    }

    #[test]
    fn speaking_is_presence_and_leaving_is_not() {
        let link = ClientLink::default();
        link.observe();
        assert!(link.present());
        link.depart();
        assert!(!link.present());
        // The next authenticated message is a new arrival, not a departure.
        link.observe();
        assert!(link.present());
    }

    #[test]
    fn a_declared_client_lapses_after_the_lease_and_a_legacy_one_never_does() {
        let lease = CLIENT_PRESENCE_LEASE.as_millis() as u64;
        let link = ClientLink::default();
        link.declare(true);
        link.observe();
        link.state.lock().unwrap().seen_ms = 1_000;
        assert!(link.present_at(1_000 + lease));
        assert!(!link.present_at(1_001 + lease));
        // A beat extends it.
        link.observe();
        link.state.lock().unwrap().seen_ms = 2_000;
        assert!(link.present_at(2_000 + lease));

        // The same silence from a client that promised nothing says nothing:
        // it never claimed to beat, so only its own goodbye can retire it.
        let legacy = ClientLink::default();
        legacy.observe();
        legacy.state.lock().unwrap().seen_ms = 1_000;
        assert!(legacy.present_at(1_000 + 100 * lease));
        legacy.depart();
        assert!(!legacy.present_at(1_000 + 100 * lease));
    }
}
