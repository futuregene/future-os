//! Bounded failure episodes shared across Remote transport tasks.
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default)]
struct FailureEpisodeState {
    category: String,
    started_at: u64,
    attempts: u64,
    reports: u8,
    last_error: String,
}

#[derive(Default)]
pub struct FailureEpisode(Mutex<FailureEpisodeState>);

#[derive(Default)]
struct LogQuota {
    window_started_at: u64,
    emitted: u8,
}

/// This is intentionally process-wide, rather than an episode field. A
/// broken broker can alternate network/auth/task symptoms and would otherwise
/// reset each individual episode's counter forever. Support logs remain useful
/// without letting a 24-hour outage fill the disk.
static FAILURE_LOG_QUOTAS: LazyLock<Mutex<HashMap<String, LogQuota>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const FAILURE_LOG_WINDOW_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_FAILURE_LOGS_PER_CATEGORY: u8 = 16;

fn permit_failure_log(category: &str) -> bool {
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
    pub fn record(&self, category: &str, error: impl std::fmt::Display) -> Option<String> {
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

    pub fn recovered(&self) -> Option<String> {
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

fn support_code_for_category(category: &str) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_episode_same_category_non_power_of_two_attempt_is_silent() {
        let category = "test_episode_silent";
        let episode = FailureEpisode(Mutex::new(FailureEpisodeState::default()));
        // Attempt 1 (fresh category) and attempt 2 (power of two) both report;
        // attempt 3 is not a power of two, so it falls through to the silent None.
        assert!(episode.record(category, "first").is_some());
        assert!(episode.record(category, "second").is_some());
        assert!(episode.record(category, "third").is_none());
    }
    #[test]
    fn failure_episode_category_switch_after_quota_exhaustion_reports_nothing() {
        // Exhaust the process-wide log quota for a fresh category, then start a
        // new episode with that category: the first `record` resets the episode
        // state but `permit_failure_log` refuses the line, so it returns None.
        let category = "test_episode_switch_quota";
        for _ in 0..MAX_FAILURE_LOGS_PER_CATEGORY {
            assert!(permit_failure_log(category));
        }
        assert!(!permit_failure_log(category));

        let episode = FailureEpisode(Mutex::new(FailureEpisodeState::default()));
        assert!(episode.record(category, "boom").is_none());
    }
    #[test]
    fn support_codes_cover_every_category() {
        assert_eq!(support_code_for_category("network"), "NW001");
        assert_eq!(support_code_for_category("credential_network"), "NW001");
        assert_eq!(support_code_for_category("remote_server"), "SV001");
        assert_eq!(support_code_for_category("service_authorization"), "AU001");
        assert_eq!(support_code_for_category("account_authorization"), "AU003");
        assert_eq!(support_code_for_category("credential_expired"), "AU002");
        assert_eq!(support_code_for_category("credential_connect"), "AU002");
        assert_eq!(support_code_for_category("slow_consumer"), "RT002");
        assert_eq!(support_code_for_category("command_subscription"), "RT003");
        assert_eq!(support_code_for_category("transfer_subscription"), "RT004");
        assert_eq!(support_code_for_category("event_publish"), "RT005");
        assert_eq!(support_code_for_category("heartbeat_publish"), "RT006");
        assert_eq!(support_code_for_category("state_publish"), "RT006");
        assert_eq!(support_code_for_category("web_bind"), "LC002");
        assert_eq!(support_code_for_category("revoked"), "PA001");
        assert_eq!(support_code_for_category("local"), "LC001");
        assert_eq!(support_code_for_category("mystery"), "LC999");
    }
    #[test]
    fn failure_logs_are_bounded_across_alternating_episodes() {
        // The per-episode counter alone is insufficient: alternating category
        // names used to restart it indefinitely. This unique test category
        // exercises the process-level, 24-hour quota.
        let category = "test_alternating_failure_quota";
        for _ in 0..MAX_FAILURE_LOGS_PER_CATEGORY {
            assert!(permit_failure_log(category));
        }
        assert!(!permit_failure_log(category));
    }
}

/// Rate-limited drop reporting for the remote event mirror: one line when a
/// drop episode starts, one per 10s while it persists, one on recovery — instead
/// of the per-event line that flooded the terminal the moment the queue
/// saturated (e.g. NATS offline). A dropped event is never data loss; the
/// client heals the gap via `get_events_since` backfill.
pub struct DropCounters {
    /// An episode (queue full / NATS offline) is active until a successful enqueue.
    dropping: AtomicBool,
    /// Cumulative drops since the episode started; reset on recovery.
    dropped: AtomicU64,
    /// Number of emitted lines in this episode. Capped so a prolonged outage
    /// can never fill the console or a redirected log file.
    reports: AtomicU8,
}

impl DropCounters {
    pub const fn new() -> Self {
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
    pub fn record_drop(
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
    pub fn report_recovery(&self) -> Option<String> {
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

impl Default for DropCounters {
    fn default() -> Self {
        Self::new()
    }
}
impl DropCounters {
    #[cfg(any(test, feature = "test-support"))]
    pub fn reset_for_test(&self) {
        self.dropping.store(false, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
        self.reports.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;
    #[test]
    fn drop_log_rate_limits_episodes() {
        let counters = DropCounters::new();

        // Episode progress is reported only at power-of-two attempts.
        for i in 1..=10 {
            let line = counters.record_drop("queue full", "tool_delta", "s1", 1_000 + i);
            assert_eq!(
                line.is_some(),
                i.is_power_of_two(),
                "unexpected reporting decision for drop {i}"
            );
        }
        assert_eq!(counters.reports.load(Ordering::Relaxed), 4);
    }
    #[test]
    fn drop_log_reports_recovery_and_resets() {
        let counters = DropCounters::new();

        counters.record_drop("NATS not connected", "tool_delta", "s1", 1_000);
        let line = counters
            .report_recovery()
            .expect("recovery after an active episode reports");
        assert!(line.contains("dropped 1 events"), "got: {line}");

        // No episode active: recovery is silent and counters stay zero.
        assert!(counters.report_recovery().is_none());

        // The next episode starts fresh: the first drop logs again.
        assert!(counters
            .record_drop("queue full", "tool_delta", "s1", 2_000)
            .is_some());
    }
}
