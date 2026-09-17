use super::*;

#[derive(Default)]
pub(super) struct FailureEpisodeState {
    category: String,
    started_at: u64,
    attempts: u64,
    reports: u8,
    last_error: String,
}

#[derive(Default)]
pub(super) struct FailureEpisode(pub(super) Mutex<FailureEpisodeState>);

#[derive(Default)]
pub(super) struct LogQuota {
    window_started_at: u64,
    emitted: u8,
}

/// This is intentionally process-wide, rather than an episode field. A
/// broken broker can alternate network/auth/task symptoms and would otherwise
/// reset each individual episode's counter forever. Support logs remain useful
/// without letting a 24-hour outage fill the disk.
pub(super) static FAILURE_LOG_QUOTAS: LazyLock<Mutex<HashMap<String, LogQuota>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) const FAILURE_LOG_WINDOW_MS: u64 = 24 * 60 * 60 * 1_000;
pub(super) const MAX_FAILURE_LOGS_PER_CATEGORY: u8 = 16;

pub(super) fn permit_failure_log(category: &str) -> bool {
    let now = unix_millis();
    let mut quotas = FAILURE_LOG_QUOTAS.lock().unwrap();
    let quota = quotas.entry(category.to_string()).or_default();
    if now.saturating_sub(quota.window_started_at) >= FAILURE_LOG_WINDOW_MS {
        quota.window_started_at = now;
        quota.emitted = 0;
    }
    if quota.emitted >= MAX_FAILURE_LOGS_PER_CATEGORY {
        return false;
    }
    quota.emitted += 1;
    true
}

impl FailureEpisode {
    pub(super) fn record(&self, category: &str, error: impl std::fmt::Display) -> Option<String> {
        let mut episode = self.0.lock().unwrap();
        let error = error.to_string();
        if episode.category != category {
            episode.category = category.to_string();
            episode.started_at = unix_millis();
            episode.attempts = 1;
            episode.reports = 0;
            episode.last_error = error;
            if permit_failure_log(category) {
                episode.reports = 1;
                return Some(format!(
                    "remote: {category} failure episode started [{}]: {}",
                    support_code_for_category(category),
                    episode.last_error
                ));
            }
            return None;
        }
        episode.attempts = episode.attempts.saturating_add(1);
        episode.last_error = error;
        if episode.reports < 15
            && episode.attempts.is_power_of_two()
            && permit_failure_log(category)
        {
            episode.reports += 1;
            return Some(format!(
                "remote: {category} failure persists [{}] (attempt {}): {}",
                support_code_for_category(category),
                episode.attempts,
                episode.last_error
            ));
        }
        None
    }

    pub(super) fn recovered(&self) -> Option<String> {
        let mut episode = self.0.lock().unwrap();
        if episode.category.is_empty() {
            return None;
        }
        let line = permit_failure_log(&episode.category).then(|| {
            format!(
                "remote: {} failure recovered [{}] after {} attempts and {}ms",
                episode.category,
                support_code_for_category(&episode.category),
                episode.attempts,
                unix_millis().saturating_sub(episode.started_at)
            )
        });
        *episode = FailureEpisodeState::default();
        line
    }
}

pub(super) fn support_code_for_category(category: &str) -> &'static str {
    match category {
        "network" | "credential_network" => "NW001",
        "remote_server" => "SV001",
        "service_authorization" => "AU001",
        "account_authorization" => "AU003",
        "credential_expired" | "credential_connect" => "AU002",
        "slow_consumer" => "RT002",
        "command_subscription" => "RT003",
        "transfer_subscription" => "RT004",
        "event_publish" => "RT005",
        "heartbeat_publish" | "state_publish" => "RT006",
        "web_bind" => "LC002",
        "revoked" => "PA001",
        "local" => "LC001",
        _ => "LC999",
    }
}

pub(super) static EVENT_PUBLISH_EPISODE: FailureEpisode =
    FailureEpisode(Mutex::new(FailureEpisodeState {
        category: String::new(),
        started_at: 0,
        attempts: 0,
        reports: 0,
        last_error: String::new(),
    }));
pub(super) static START_EPISODE: FailureEpisode = FailureEpisode(Mutex::new(FailureEpisodeState {
    category: String::new(),
    started_at: 0,
    attempts: 0,
    reports: 0,
    last_error: String::new(),
}));
pub(super) static CREDENTIAL_EPISODE: FailureEpisode =
    FailureEpisode(Mutex::new(FailureEpisodeState {
        category: String::new(),
        started_at: 0,
        attempts: 0,
        reports: 0,
        last_error: String::new(),
    }));
pub(super) static HEARTBEAT_PUBLISH_EPISODE: FailureEpisode =
    FailureEpisode(Mutex::new(FailureEpisodeState {
        category: String::new(),
        started_at: 0,
        attempts: 0,
        reports: 0,
        last_error: String::new(),
    }));

/// Port for the embedded web client HTTP server.
pub(super) const WEB_PORT: u16 = 8022;
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
