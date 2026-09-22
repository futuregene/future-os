//! Duplicate and stale inbound filtering.
//!
//! Two failure modes are common to every long-polling or websocket platform:
//! the same event is delivered twice (an at-least-once delivery guarantee, a
//! reconnect replay), and a delivery arrives long after it was sent (a socket
//! that was down for minutes replaying its backlog). The first would make the
//! agent answer twice; the second would make it answer a question the user
//! already moved on from.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

/// Default age past which a replayed message is ignored.
pub const DEFAULT_STALE_AFTER_MS: i64 = 60_000;

/// Default number of recent message ids remembered per channel.
pub const DEFAULT_CAPACITY: usize = 1024;

/// A bounded, insertion-ordered set of recently seen message ids.
///
/// Bounded because a bot runs for weeks: an unbounded set is a slow leak. The
/// oldest id is evicted first, which is the right trade — the platform replays
/// recent messages, not ancient ones.
pub struct Dedup {
    state: Mutex<State>,
    capacity: usize,
}

struct State {
    order: VecDeque<String>,
    seen: HashSet<String>,
}

impl Dedup {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            state: Mutex::new(State {
                order: VecDeque::new(),
                seen: HashSet::new(),
            }),
            capacity,
        }
    }

    /// Record `message_id`; `true` when it had already been recorded.
    pub fn check_and_insert(&self, message_id: &str) -> bool {
        if message_id.is_empty() {
            // Without an id we cannot dedup; answer rather than swallow.
            return false;
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.seen.contains(message_id) {
            return true;
        }
        state.seen.insert(message_id.to_string());
        state.order.push_back(message_id.to_string());
        while state.order.len() > self.capacity {
            if let Some(evicted) = state.order.pop_front() {
                state.seen.remove(&evicted);
            }
        }
        false
    }

    /// Number of ids currently remembered (tests and diagnostics).
    pub fn len(&self) -> usize {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Whether a message is too old to answer.
///
/// Returns `false` when the platform gave no timestamp: guessing would drop
/// legitimate messages on platforms that omit it.
pub fn is_stale(created_at_ms: Option<i64>, now_ms: i64, max_age_ms: i64) -> bool {
    match created_at_ms {
        Some(created) => now_ms.saturating_sub(created) > max_age_ms,
        None => false,
    }
}

/// Current Unix time in milliseconds.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_delivery_is_not_a_duplicate() {
        let dedup = Dedup::new(8);
        assert!(!dedup.check_and_insert("m1"));
        assert!(dedup.check_and_insert("m1"));
        assert!(!dedup.check_and_insert("m2"));
    }

    #[test]
    fn a_message_without_an_id_is_never_suppressed() {
        let dedup = Dedup::new(8);
        assert!(!dedup.check_and_insert(""));
        assert!(!dedup.check_and_insert(""));
        assert!(dedup.is_empty());
    }

    #[test]
    fn the_set_stays_bounded_and_evicts_oldest_first() {
        let dedup = Dedup::new(4);
        for index in 0..10 {
            dedup.check_and_insert(&format!("m{index}"));
        }
        assert_eq!(dedup.len(), 4);
        // The oldest ids were evicted, so a very old replay is answered again —
        // an accepted trade for a bounded set.
        assert!(!dedup.check_and_insert("m0"));
        // Recent ids are still remembered.
        assert!(dedup.check_and_insert("m9"));
    }

    #[test]
    fn a_zero_capacity_is_treated_as_one() {
        let dedup = Dedup::new(0);
        assert!(!dedup.check_and_insert("m1"));
        assert_eq!(dedup.len(), 1);
    }

    #[test]
    fn messages_without_timestamps_are_never_stale() {
        assert!(!is_stale(None, 1_000_000, DEFAULT_STALE_AFTER_MS));
    }

    #[test]
    fn an_old_replay_is_stale_but_a_recent_one_is_not() {
        let now = 1_000_000;
        assert!(!is_stale(Some(now - 1_000), now, DEFAULT_STALE_AFTER_MS));
        assert!(is_stale(Some(now - 61_000), now, DEFAULT_STALE_AFTER_MS));
        // Exactly at the boundary is still fresh.
        assert!(!is_stale(Some(now - 60_000), now, DEFAULT_STALE_AFTER_MS));
    }

    #[test]
    fn a_future_timestamp_is_not_stale() {
        // Clock skew between the platform and us must not drop messages.
        assert!(!is_stale(
            Some(1_000_500),
            1_000_000,
            DEFAULT_STALE_AFTER_MS
        ));
    }

    #[test]
    fn now_ms_is_a_plausible_unix_timestamp() {
        assert!(now_ms() > 1_700_000_000_000);
    }
}
