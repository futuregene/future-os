//! Durable outbound delivery.
//!
//! A reply that belongs to a turn is sent while the turn is alive, and a
//! failure is visible to the user as a missing answer. Proactive sends are
//! different: nobody is waiting, so a rate limit or a momentary outage would
//! silently lose the message. This queue is for those — an outbound note
//! survives a restart, is retried with backoff, and stops retrying the moment
//! the failure is one that cannot succeed (a banned bot, a deleted channel).
//!
//! Retrying is only safe for a message that can be sent twice without harm, so
//! callers pass an explicit durability: best-effort messages are dropped after
//! the first permanent error, required ones are kept in the `failed` list for
//! inspection.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::bridge::ConversationRef;

/// Attempts before a message is parked as failed.
pub const MAX_ATTEMPTS: u32 = 5;
/// Backoff schedule in milliseconds, indexed by attempt (1-based).
const BACKOFF_MS: [u64; 5] = [0, 5_000, 25_000, 120_000, 600_000];
/// Upper bound on the persisted queue.
pub const MAX_ENTRIES: usize = 2_000;

/// Where a queued message is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    Sent,
    Failed,
}

/// One queued outbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedDelivery {
    pub id: String,
    pub channel: String,
    pub conversation: ConversationRef,
    pub text: String,
    pub attempts: u32,
    pub state: DeliveryState,
    pub last_error: Option<String>,
    /// Unix milliseconds of the last attempt (absent before the first one).
    pub last_attempt_unix_ms: Option<i64>,
    pub enqueued_unix_ms: i64,
}

impl QueuedDelivery {
    /// Whether a retry is due at `now_ms`.
    pub fn is_due(&self, now_ms: i64) -> bool {
        if self.state != DeliveryState::Pending {
            return false;
        }
        let base = self.last_attempt_unix_ms.unwrap_or(self.enqueued_unix_ms);
        now_ms >= base.saturating_add(backoff_ms(self.attempts) as i64)
    }
}

/// Delay before the attempt that follows `attempts` completed attempts.
pub fn backoff_ms(attempts: u32) -> u64 {
    BACKOFF_MS
        .get(attempts as usize)
        .copied()
        .unwrap_or(600_000)
}

/// Failures that will never succeed, however many times we try.
///
/// Matched case-insensitively against the platform's error text. Wrongly
/// classifying a retryable error as permanent loses one message; wrongly
/// classifying a permanent one as retryable means retrying forever, so the list
/// is deliberately conservative.
const PERMANENT_PATTERNS: &[&str] = &[
    "chat not found",
    "channel not found",
    "user not found",
    "recipient not found",
    "bot was blocked",
    "bot is not a member",
    "not a member of",
    "kicked",
    "forbidden",
    "unauthorized",
    "invalid token",
    "invalid_auth",
    "token expired",
    "no conversation reference",
    "recipient is not a valid",
    "channel is archived",
    "message too long",
    "not configured",
];

/// Whether an error message describes a permanent failure.
pub fn is_permanent_error(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    PERMANENT_PATTERNS
        .iter()
        .any(|pattern| lowered.contains(pattern))
}

/// A file-backed queue of outbound messages.
pub struct DeliveryQueue {
    path: PathBuf,
    entries: Mutex<Vec<QueuedDelivery>>,
    /// Set when the file existed but could not be read; persisting then would
    /// destroy messages we failed to parse.
    load_error: Mutex<Option<String>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    deliveries: Vec<QueuedDelivery>,
}

impl DeliveryQueue {
    /// Default location: `~/.future/channels/deliveries.json`.
    pub fn default_path() -> PathBuf {
        crate::config::home_dir()
            .join(".future")
            .join("channels")
            .join("deliveries.json")
    }

    /// Load a queue from disk, tolerating an absent file.
    pub fn load(path: PathBuf) -> Self {
        let mut entries = Vec::new();
        let mut load_error = None;
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Store>(&content) {
                Ok(store) => entries = store.deliveries,
                Err(error) => {
                    load_error = Some(format!("cannot parse {}: {error}", path.display()));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                load_error = Some(format!("cannot read {}: {error}", path.display()));
            }
        }
        Self {
            path,
            entries: Mutex::new(entries),
            load_error: Mutex::new(load_error),
        }
    }

    /// Add a message to the queue and persist it.
    pub fn enqueue(
        &self,
        channel: &str,
        conversation: ConversationRef,
        text: &str,
    ) -> Result<String> {
        let id = format!("dlv_{}", uuid::Uuid::new_v4().simple());
        {
            let mut entries = self.lock();
            entries.push(QueuedDelivery {
                id: id.clone(),
                channel: channel.to_string(),
                conversation,
                text: text.to_string(),
                attempts: 0,
                state: DeliveryState::Pending,
                last_error: None,
                last_attempt_unix_ms: None,
                enqueued_unix_ms: now_ms(),
            });
            // Bound the file: drop the oldest finished entries first.
            if entries.len() > MAX_ENTRIES {
                let excess = entries.len() - MAX_ENTRIES;
                let mut dropped = 0;
                entries.retain(|entry| {
                    if dropped < excess && entry.state != DeliveryState::Pending {
                        dropped += 1;
                        false
                    } else {
                        true
                    }
                });
            }
        }
        self.save()?;
        Ok(id)
    }

    /// Messages waiting for delivery, oldest first.
    pub fn pending(&self) -> Vec<QueuedDelivery> {
        let mut entries: Vec<QueuedDelivery> = self
            .lock()
            .iter()
            .filter(|entry| entry.state == DeliveryState::Pending)
            .cloned()
            .collect();
        entries.sort_by_key(|entry| entry.enqueued_unix_ms);
        entries
    }

    /// Pending messages whose backoff has elapsed.
    pub fn due(&self, now: i64) -> Vec<QueuedDelivery> {
        self.pending()
            .into_iter()
            .filter(|entry| entry.is_due(now))
            .collect()
    }

    /// Everything currently held (diagnostics).
    pub fn all(&self) -> Vec<QueuedDelivery> {
        self.lock().clone()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mark a message delivered.
    pub fn record_success(&self, id: &str) -> Result<()> {
        {
            let mut entries = self.lock();
            if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) {
                entry.state = DeliveryState::Sent;
                entry.last_error = None;
            }
        }
        self.save()
    }

    /// Record a failed attempt, deciding whether to retry.
    pub fn record_failure(&self, id: &str, error: &str) -> DeliveryState {
        let state = {
            let mut entries = self.lock();
            match entries.iter_mut().find(|entry| entry.id == id) {
                Some(entry) => {
                    entry.attempts += 1;
                    entry.last_attempt_unix_ms = Some(now_ms());
                    entry.last_error = Some(error.to_string());
                    if is_permanent_error(error) || entry.attempts >= MAX_ATTEMPTS {
                        entry.state = DeliveryState::Failed;
                    }
                    entry.state
                }
                None => DeliveryState::Failed,
            }
        };
        let _ = self.save();
        state
    }

    /// Forget everything that finished, and persist.
    pub fn prune_finished(&self) -> Result<usize> {
        let removed = {
            let mut entries = self.lock();
            let before = entries.len();
            entries.retain(|entry| entry.state == DeliveryState::Pending);
            before - entries.len()
        };
        self.save()?;
        Ok(removed)
    }

    fn save(&self) -> Result<()> {
        if let Some(error) = self
            .load_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            anyhow::bail!("refusing to overwrite an unreadable delivery queue: {error}");
        }
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let payload = Store {
            deliveries: self.lock().clone(),
        };
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_string_pretty(&payload)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<QueuedDelivery>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// The recorded load error, if the file could not be read.
    pub fn load_error(&self) -> Option<String> {
        self.load_error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

/// Current Unix time in milliseconds.
fn now_ms() -> i64 {
    crate::bridge::dedup::now_ms()
}

/// A path that is safe to hand to [`DeliveryQueue::load`].
pub fn queue_path(root: &Path) -> PathBuf {
    root.join("deliveries.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(label: &str) -> (DeliveryQueue, PathBuf) {
        let dir = crate::test_support::temp_dir(label);
        let path = dir.join("deliveries.json");
        (DeliveryQueue::load(path.clone()), path)
    }

    fn conversation() -> ConversationRef {
        ConversationRef {
            id: "c1".into(),
            thread_id: None,
            kind: crate::bridge::ChatKind::Direct,
        }
    }

    #[test]
    fn enqueueing_persists_and_survives_a_reload() {
        let (queue, path) = queue("delivery-persist");
        let id = queue.enqueue("telegram", conversation(), "hello").unwrap();
        assert_eq!(queue.pending().len(), 1);

        let reloaded = DeliveryQueue::load(path);
        let pending = reloaded.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].text, "hello");
        assert_eq!(pending[0].channel, "telegram");
        assert_eq!(pending[0].attempts, 0);
    }

    #[test]
    fn a_missing_file_is_an_empty_queue() {
        let queue = DeliveryQueue::load(PathBuf::from("/nonexistent/deliveries.json"));
        assert!(queue.is_empty());
        assert!(queue.load_error().is_none());
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten() {
        let dir = crate::test_support::temp_dir("delivery-corrupt");
        let path = dir.join("deliveries.json");
        std::fs::write(&path, "{not json").unwrap();
        let queue = DeliveryQueue::load(path.clone());
        assert!(queue.load_error().is_some());
        let error = queue
            .enqueue("telegram", conversation(), "hello")
            .expect_err("must refuse");
        assert!(error.to_string().contains("refusing"), "{error}");
        // The original bytes are untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");
    }

    #[test]
    fn a_fresh_message_goes_out_immediately() {
        let (queue, _path) = queue("delivery-immediate");
        queue.enqueue("telegram", conversation(), "hello").unwrap();
        assert_eq!(queue.due(now_ms()).len(), 1);
    }

    #[test]
    fn a_failed_attempt_is_retried_after_a_backoff() {
        let (queue, _path) = queue("delivery-backoff");
        let id = queue.enqueue("telegram", conversation(), "hello").unwrap();
        let now = now_ms();
        let state = queue.record_failure(&id, "gateway timeout");
        assert_eq!(state, DeliveryState::Pending);
        // Not due yet, due after the first backoff interval.
        assert!(queue.due(now).is_empty());
        assert_eq!(queue.due(now + backoff_ms(1) as i64 + 1).len(), 1);
    }

    #[test]
    fn a_permanent_error_ends_the_retries_immediately() {
        let (queue, _path) = queue("delivery-permanent");
        let id = queue.enqueue("telegram", conversation(), "hello").unwrap();
        let state = queue.record_failure(&id, "Forbidden: bot was blocked by the user");
        assert_eq!(state, DeliveryState::Failed);
        assert!(queue.due(now_ms() + 10_000_000).is_empty());
    }

    #[test]
    fn attempts_are_capped() {
        let (queue, _path) = queue("delivery-attempts");
        let id = queue.enqueue("telegram", conversation(), "hello").unwrap();
        let mut state = DeliveryState::Pending;
        for _ in 0..MAX_ATTEMPTS {
            state = queue.record_failure(&id, "gateway timeout");
        }
        assert_eq!(state, DeliveryState::Failed);
        assert_eq!(queue.all()[0].attempts, MAX_ATTEMPTS);
    }

    #[test]
    fn a_success_stops_the_retries() {
        let (queue, _path) = queue("delivery-success");
        let id = queue.enqueue("telegram", conversation(), "hello").unwrap();
        queue.record_success(&id).unwrap();
        assert!(queue.pending().is_empty());
        assert!(queue.due(now_ms() + 10_000_000).is_empty());
        assert_eq!(queue.all().len(), 1, "the entry is kept for inspection");
    }

    #[test]
    fn success_clears_a_previous_error() {
        let (queue, _path) = queue("delivery-clear-error");
        let id = queue.enqueue("telegram", conversation(), "hello").unwrap();
        queue.record_failure(&id, "gateway timeout");
        queue.record_success(&id).unwrap();
        let entry = queue.all().into_iter().next().unwrap();
        assert_eq!(entry.last_error, None);
        assert_eq!(entry.state, DeliveryState::Sent);
    }

    #[test]
    fn finishing_an_unknown_id_is_harmless() {
        let (queue, _path) = queue("delivery-unknown");
        assert_eq!(
            queue.record_failure("nope", "gateway timeout"),
            DeliveryState::Failed
        );
        queue.record_success("nope").unwrap();
    }

    #[test]
    fn pruning_keeps_only_pending_messages() {
        let (queue, _path) = queue("delivery-prune");
        let sent = queue.enqueue("telegram", conversation(), "one").unwrap();
        queue.enqueue("telegram", conversation(), "two").unwrap();
        queue.record_success(&sent).unwrap();
        assert_eq!(queue.prune_finished().unwrap(), 1);
        assert_eq!(queue.all().len(), 1);
        assert_eq!(queue.pending()[0].text, "two");
    }

    #[test]
    fn a_finished_delivery_is_never_due_again() {
        let (queue, _path) = queue("delivery-not-due");
        let sent = queue.enqueue("telegram", conversation(), "one").unwrap();
        queue.record_success(&sent).unwrap();
        let sent = queue.all().into_iter().next().unwrap();
        assert!(!sent.is_due(now_ms() + 10_000_000));

        let failed = queue.enqueue("telegram", conversation(), "two").unwrap();
        queue.record_failure(&failed, "chat not found");
        let failed = queue
            .all()
            .into_iter()
            .find(|entry| entry.id == failed)
            .unwrap();
        assert_eq!(failed.state, DeliveryState::Failed);
        assert!(!failed.is_due(now_ms() + 10_000_000));
    }

    #[test]
    fn a_read_error_other_than_missing_is_reported() {
        // A directory where the file should be: reading fails with a real error
        // that must be surfaced rather than silently treated as an empty queue.
        let dir = crate::test_support::temp_dir("delivery-dir-as-file");
        let path = dir.join("deliveries.json");
        std::fs::create_dir_all(&path).unwrap();
        let queue = DeliveryQueue::load(path.clone());
        let error = queue.load_error().expect("load error");
        assert!(error.contains("cannot read"), "{error}");
        assert!(queue
            .enqueue("telegram", conversation(), "hello")
            .is_err());
    }

    #[test]
    fn the_queue_is_bounded_and_drops_finished_entries_first() {
        let (queue, _path) = queue("delivery-bounded");
        // Fill past the cap with finished entries plus one pending.
        for index in 0..MAX_ENTRIES + 5 {
            let id = queue
                .enqueue("telegram", conversation(), &format!("m{index}"))
                .unwrap();
            if index != MAX_ENTRIES + 4 {
                queue.record_success(&id).unwrap();
            }
        }
        assert!(queue.len() <= MAX_ENTRIES + 1, "{}", queue.len());
        let pending = queue.pending();
        assert_eq!(pending.len(), 1, "the pending message is never dropped");
        assert_eq!(pending[0].text, format!("m{}", MAX_ENTRIES + 4));
    }

    #[test]
    fn permanent_patterns_are_matched_case_insensitively() {
        for message in [
            "CHAT NOT FOUND",
            "Forbidden: bot was blocked by the user",
            "invalid_auth",
            "Unauthorized",
            "recipient is not a valid user",
        ] {
            assert!(is_permanent_error(message), "{message}");
        }
        for message in [
            "gateway timeout",
            "429 Too Many Requests",
            "connection reset by peer",
            "internal server error",
        ] {
            assert!(!is_permanent_error(message), "{message}");
        }
    }

    #[test]
    fn backoff_grows_then_saturates() {
        assert_eq!(backoff_ms(0), 0);
        assert_eq!(backoff_ms(1), 5_000);
        assert_eq!(backoff_ms(2), 25_000);
        assert_eq!(backoff_ms(3), 120_000);
        assert_eq!(backoff_ms(4), 600_000);
        assert_eq!(backoff_ms(99), 600_000);
    }

    #[test]
    fn the_default_path_lives_under_the_channel_directory() {
        let path = DeliveryQueue::default_path();
        assert!(path.to_string_lossy().contains("channels"));
        assert_eq!(path.file_name().unwrap(), "deliveries.json");
    }

    #[test]
    fn a_custom_root_produces_a_queue_path() {
        let path = queue_path(Path::new("/tmp/root"));
        assert_eq!(path, PathBuf::from("/tmp/root/deliveries.json"));
    }
}
