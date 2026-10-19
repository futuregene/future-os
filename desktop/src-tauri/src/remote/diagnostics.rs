use super::*;
pub(super) use future_remote_server::diagnostics::FailureEpisode;

pub(super) static START_EPISODE: LazyLock<FailureEpisode> = LazyLock::new(Default::default);
pub(super) static CREDENTIAL_EPISODE: LazyLock<FailureEpisode> = LazyLock::new(Default::default);
pub(super) static HEARTBEAT_PUBLISH_EPISODE: LazyLock<FailureEpisode> =
    LazyLock::new(Default::default);

/// Port for the embedded web client HTTP server.
pub(crate) const WEB_PORT: u16 = 8022;
/// The remote browser client is a single self-contained HTML file. Embed it so
/// release bundles do not depend on the build machine's source checkout.
pub(super) const EMBEDDED_WEB_INDEX: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/index.html"));
/// A shutdown notification is advisory: never delay closing the desktop for a
/// slow or unreachable broker, but give a healthy connection a short window to
/// flush the packet before its tasks are torn down.
pub(super) const DISCONNECT_NOTICE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(750);

/// Bound on the event publish queue; on overflow the newest event is dropped
/// (logged) rather than blocking the agent event loop. The client recovers the
/// gap via `get_events_since` backfill on its next reattach.
pub(super) const EVENT_QUEUE_CAPACITY: usize = 4096;

/// Rate-limited drop reporting for the remote event mirror: one line when a
/// drop episode starts, one per 10s while it persists, one on recovery — instead
/// of the per-event line that flooded the terminal the moment the queue
/// saturated (e.g. NATS offline). A dropped event is never data loss; the
/// client heals the gap via `get_events_since` backfill.
pub(super) struct DropCounters {
    /// An episode (queue full / NATS offline) is active until a successful enqueue.
    pub(super) dropping: AtomicBool,
    /// Cumulative drops since the episode started; reset on recovery.
    pub(super) dropped: AtomicU64,
    /// Number of emitted lines in this episode. Capped so a prolonged outage
    /// can never fill the console or a redirected log file.
    pub(super) reports: AtomicU8,
}

impl DropCounters {
    pub(super) const fn new() -> Self {
        Self {
            dropping: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            reports: AtomicU8::new(0),
        }
    }

    /// Returns the line to print for a dropped event, or `None` when the episode
    /// is already being reported at full cadence. Event-driven: called on the
    /// hot path every event passes through, so it keeps reporting even while
    /// the drain task is blocked on `publish().await` and can't reach its own
    /// timer. `now` is injected so tests can advance time deterministically.
    pub(super) fn record_drop(
        &self,
        why: &str,
        event_type: &str,
        session_id: &str,
        now: u64,
    ) -> Option<String> {
        let first = !self.dropping.swap(true, Ordering::Relaxed);
        let total = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = now;
        if first {
            self.reports.store(1, Ordering::Relaxed);
            Some(format!(
                "remote: {why}; dropping {event_type} for {session_id} (backfill on reconnect heals the gap)"
            ))
        } else if total.is_power_of_two() && self.reports.load(Ordering::Relaxed) < 15 {
            self.reports.fetch_add(1, Ordering::Relaxed);
            Some(format!(
                "remote: {why}; dropped {total} events so far for {session_id}"
            ))
        } else {
            None
        }
    }

    /// One-shot recovery line when a drop episode ends (the first event that
    /// enqueues successfully). Returns `None` when no episode was active, and
    /// resets the episode counters so the next episode starts fresh.
    pub(super) fn report_recovery(&self) -> Option<String> {
        if self.dropping.swap(false, Ordering::Relaxed) {
            let dropped = self.dropped.swap(0, Ordering::Relaxed);
            self.reports.store(0, Ordering::Relaxed);
            Some(format!(
                "remote: event publish recovered; dropped {dropped} events during the backlog"
            ))
        } else {
            None
        }
    }
}
