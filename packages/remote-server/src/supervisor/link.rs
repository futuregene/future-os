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
    /// The last lapse of the lease has already been reported. Presence is read
    /// on every poll, and a departure is news once: without this flag a poll
    /// would print it every few seconds. A new proof clears it.
    lapse_reported: bool,
}

/// Whether the state describes a client that is there at `now_ms`.
///
/// One definition for the accessor, the poll that reports departures and the
/// arrival test in [`ClientLink::observe`], so "present" cannot mean three
/// slightly different things.
fn present_of(state: &LinkState, now_ms: u64) -> bool {
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

/// One pairing's view of its client. Shared across bridge generations: a
/// credential refresh is the same client reconnecting, not a new one.
#[derive(Default)]
pub struct ClientLink {
    state: Mutex<LinkState>,
    /// The pairing this link belongs to, so a departure line names it.
    pair_id: String,
}

impl ClientLink {
    /// A link that can name its pairing in what it reports.
    pub fn with_pair(pair_id: &str) -> Self {
        Self {
            state: Mutex::new(LinkState::default()),
            pair_id: pair_id.to_string(),
        }
    }

    /// The client did something authenticated: it is here.
    ///
    /// Answers whether this observation is an *arrival* — the first proof after
    /// the client was not here — because that is the one moment worth a log
    /// line. A `client_presence` beat every 15 seconds is not an arrival.
    pub fn observe(&self) -> bool {
        let now = unix_timestamp_ms();
        let mut state = self.state.lock().unwrap();
        let before = present_of(&state, now);
        state.seen_ms = now;
        state.departed = false;
        state.lapse_reported = false;
        !before
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
        present_of(&self.state.lock().unwrap(), now_ms)
    }

    /// [`ClientLink::present`], and the departure line the first time a declared
    /// client's beats lapse.
    ///
    /// Presence is read on every poll, so a reader is the only thing that runs
    /// when a force-quit client disappears — nothing is left to notice it. The
    /// announcement therefore lives here and is bounded by [`LinkState
    /// ::lapse_reported`]: one line per lapse, however often it is polled, and
    /// the client's next proof re-arms it.
    pub fn present_and_report(&self) -> bool {
        self.present_and_report_at(unix_timestamp_ms())
    }

    /// [`ClientLink::present_and_report`] with the clock supplied, so the
    /// once-per-lapse announcement is testable without waiting a minute.
    pub fn present_and_report_at(&self, now_ms: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        let present = present_of(&state, now_ms);
        let lapsed = !present
            && state.declared
            && !state.departed
            && state.seen_ms != 0
            && !state.lapse_reported;
        if lapsed {
            state.lapse_reported = true;
            future_remote_protocol::remote_log!(
                "remote: inbound device left (pair={}, reason=presence_lease_expired, direction=inbound)",
                self.pair_id
            );
        }
        present
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

    /// An arrival is the first proof after an absence, and nothing else.
    ///
    /// The log line this drives has to appear once per arrival: `client_presence`
    /// beats while the client sits there, so treating a beat as an arrival would
    /// put a line in `desktop.log` every 15 seconds.
    #[test]
    fn only_the_first_proof_after_an_absence_is_an_arrival() {
        let link = ClientLink::default();
        assert!(link.observe(), "a client that never spoke arrives once");
        assert!(!link.observe(), "a following beat is not an arrival");
        assert!(!link.observe());
        link.depart();
        assert!(
            link.observe(),
            "after a goodbye, speaking is an arrival again"
        );
        // A lease that runs out is also an absence: the client's next proof is
        // an arrival, not a continuation.
        link.declare(true);
        link.state.lock().unwrap().seen_ms = 1_000;
        assert!(!link.present_at(1_000 + CLIENT_PRESENCE_LEASE.as_millis() as u64 + 1));
        assert!(link.observe());
    }

    /// A lapse is reported once however often it is polled, and re-armed by the
    /// next proof.
    #[test]
    fn a_lapse_is_announced_once_and_rearmed_by_the_next_proof() {
        let link = ClientLink::with_pair("pair_fixture");
        link.declare(true);
        link.observe();
        link.state.lock().unwrap().seen_ms = 1_000;
        let expired = 1_000 + CLIENT_PRESENCE_LEASE.as_millis() as u64 + 1;
        assert!(
            !link.present_and_report_at(expired),
            "the lease has run out"
        );
        assert!(
            link.state.lock().unwrap().lapse_reported,
            "the first poll announces the departure"
        );
        assert!(
            !link.present_and_report_at(expired + 5_000),
            "polling again reports the same absence"
        );
        // A proof re-arms it: the next lapse is news again.
        link.observe();
        assert!(!link.state.lock().unwrap().lapse_reported);
        // A client that never declared beats is never presumed gone, so it has
        // no lapse to announce.
        let legacy = ClientLink::with_pair("pair_legacy");
        legacy.observe();
        legacy.state.lock().unwrap().seen_ms = 1_000;
        assert!(
            legacy.present_and_report_at(1_000 + 100 * CLIENT_PRESENCE_LEASE.as_millis() as u64)
        );
        assert!(!legacy.state.lock().unwrap().lapse_reported);
    }
}
