//! Local Agent supervision policy: probe, classify, and decide.
//!
//! Three entry points can bring an Agent up for one data root (`future agent`,
//! `future host`, `future tui`), and each of them can lose that race or
//! outlive the winner. The decisions they must all make are the same, so they
//! live here once:
//!
//! * **Attached** — a compatible Agent answers; use it.
//! * **Startable** — nothing answers and the instance lock is free; starting is
//!   ours to do.
//! * **Busy** — the lock is held but nothing answers yet: another process is
//!   starting (or serving on an endpoint this client cannot see). The only
//!   correct move is to wait; starting a second Agent is what the lock forbids.
//! * **Blocked** — a refusal no wait repairs (version/protocol mismatch, an
//!   explicit TCP endpoint that is unreachable, a shutdown in progress).
//!
//! Nothing here launches anything: it classifies, and it bounds the waiting and
//! the restart rate so the three entries cannot storm a misbehaving Agent.

use std::time::{Duration, Instant};

use crate::connection::{self, AgentSelection, ConnectionError};

/// How long a client waits for another process to finish starting the Agent.
/// Matches `AgentHost::START_TIMEOUT`, so "waiting for someone else" and
/// "waiting for ourselves" share one budget instead of stacking.
pub const WAIT_FOR_OWNER: Duration = Duration::from_secs(180);

/// How often a waiting client looks again, once past the fast phase.
pub const WAIT_PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// After this much waiting, a client says why the wait is long.
pub const WAIT_NOTICE_AFTER: Duration = Duration::from_secs(30);

/// What a client should do about the local Agent right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalAgentState {
    /// A compatible Agent answered the readiness probe.
    Attached,
    /// Nothing answered and no process holds the instance lock.
    Startable,
    /// Another process holds the lock and nothing answers yet.
    Busy(String),
    /// A refusal no amount of waiting repairs; the message is user-facing.
    Blocked(String),
}

/// Probe the local Agent and classify the result.
///
/// A successful selection is dropped immediately: this only classifies.
pub async fn probe_local_agent(
    configured: &str,
    connect_timeout: Duration,
    handshake_timeout: Duration,
) -> LocalAgentState {
    match connection::select_agent(
        configured,
        connection::EXPECTED_AGENT_VERSION,
        connect_timeout,
        handshake_timeout,
    )
    .await
    {
        Ok(AgentSelection::External(client)) => {
            drop(client);
            LocalAgentState::Attached
        }
        // `select_agent` only hands out a start permit after the lock check, so
        // reaching this arm with a permit means "nobody holds it now". The
        // permit is deliberately dropped: whoever acts consumes a fresh one.
        Ok(AgentSelection::LocalStart(permit)) => {
            drop(permit);
            LocalAgentState::Startable
        }
        Err(error) if error.is_instance_busy() => LocalAgentState::Busy(error.to_string()),
        Err(ConnectionError::Transport(_)) => LocalAgentState::Startable,
        Err(error) => LocalAgentState::Blocked(error.to_string()),
    }
}

/// How long an attached Agent must stay answering before a restart counts as a
/// success.
///
/// Without this the breaker is useless against the case it exists for: an Agent
/// that starts, runs the task, and is killed by the host (OOM, a per-user
/// quota). Every restart *does* become ready, so clearing the budget on
/// "attached" would reset it once per lap and the loop would never stop.
pub const STABLE_AFTER: Duration = Duration::from_secs(60);

/// How a client should treat a successful probe, given when the Agent last
/// became reachable again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attached {
    /// Answering, but not for long enough to call the last restart a success.
    Recently,
    /// Answering long enough that the previous failure is behind us.
    Stable,
}

impl Attached {
    pub fn of(reachable_since: Instant, now: Instant) -> Self {
        if now.duration_since(reachable_since) >= STABLE_AFTER {
            Self::Stable
        } else {
            Self::Recently
        }
    }
}

/// The delay before restart attempt `attempt` (0-based), with jitter.
///
/// `1s, 2s, 4s, …, 30s` scaled by 0.8–1.2, so two clients that saw the same
/// death do not reach for the lock in the same millisecond.
pub fn restart_delay(attempt: u32) -> Duration {
    let base = 1_000u64.saturating_mul(1u64 << attempt.min(5)).min(30_000);
    let jitter = 800 + (spread(attempt) * 400) / 1000;
    Duration::from_millis(base.saturating_mul(jitter) / 1000)
}

/// Cheap per-attempt spread in `0..1000`, seeded by the process id so two
/// clients differ without pulling in an RNG dependency.
fn spread(attempt: u32) -> u64 {
    let seed = u64::from(std::process::id())
        .wrapping_mul(6364136223846793005)
        .wrapping_add(u64::from(attempt));
    (seed >> 33) % 1000
}

/// Bounds how often one process restarts an Agent that dies immediately.
///
/// Without it, an Agent that is killed by the host (OOM, a per-user quota) turns
/// into a restart loop that spends the user's credits and buries the cause. Only
/// this process's own attempts count; another client's failures are its business.
#[derive(Debug, Clone)]
pub struct RestartBudget {
    window: Duration,
    threshold: usize,
    cooldown: Duration,
    max_cooldown: Duration,
    attempts: Vec<Instant>,
    open_until: Option<Instant>,
    cooldown_now: Duration,
}

impl Default for RestartBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl RestartBudget {
    /// Three attempts inside five minutes open a one-minute cooldown that
    /// doubles (to ten minutes) while the half-open attempt keeps failing.
    pub fn new() -> Self {
        Self {
            window: Duration::from_secs(300),
            threshold: 3,
            cooldown: Duration::from_secs(60),
            max_cooldown: Duration::from_secs(600),
            attempts: Vec::new(),
            open_until: None,
            cooldown_now: Duration::from_secs(60),
        }
    }

    /// May this process start an Agent now? `true` also means "count it".
    ///
    /// An attempt is presumed failed until [`Self::reset`] proves otherwise
    /// (the caller resets only after the Agent has stayed reachable), so the
    /// half-open attempt re-arms the breaker immediately: a start that becomes
    /// ready and is killed again must not get a fresh cooldown each lap.
    pub fn allow(&mut self, now: Instant) -> bool {
        if let Some(until) = self.open_until {
            if now < until {
                return false;
            }
            // Cooldown expired: exactly one attempt, with the doubled cooldown
            // already armed for the case where it fails too.
            self.attempts.clear();
            self.attempts.push(now);
            self.open_until = Some(now + self.cooldown_now);
            self.cooldown_now = (self.cooldown_now * 2).min(self.max_cooldown);
            return true;
        }
        self.attempts
            .retain(|at| now.duration_since(*at) < self.window);
        if self.attempts.len() < self.threshold {
            self.attempts.push(now);
            return true;
        }
        self.open_until = Some(now + self.cooldown_now);
        self.cooldown_now = (self.cooldown_now * 2).min(self.max_cooldown);
        false
    }

    /// The Agent came up: forget the failures and the escalation.
    pub fn reset(&mut self) {
        self.attempts.clear();
        self.open_until = None;
        self.cooldown_now = self.cooldown;
    }

    /// How long the breaker stays open, for the message a user sees.
    pub fn open_for(&self, now: Instant) -> Option<Duration> {
        self.open_until
            .filter(|until| *until > now)
            .map(|until| until - now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_delay_grows_and_stays_within_jitter() {
        for attempt in 0..8 {
            let delay = restart_delay(attempt);
            // The base grows to 32s but is capped at 30s, so the expectations
            // have to cap it too — otherwise the lower bound of the capped
            // attempts is wrong (24s is inside the jitter, 25.6s is not).
            let capped = (1_000u64 << attempt.min(5)).min(30_000);
            assert!(
                delay >= Duration::from_millis(capped * 8 / 10),
                "attempt {attempt}: {delay:?}"
            );
            assert!(
                delay <= Duration::from_millis(capped * 12 / 10 + 1),
                "attempt {attempt}: {delay:?}"
            );
        }
    }

    #[test]
    fn the_breaker_opens_after_three_attempts_and_reopens_after_a_half_open_failure() {
        let mut budget = RestartBudget::new();
        let start = Instant::now();
        assert!(budget.allow(start));
        assert!(budget.allow(start + Duration::from_secs(1)));
        assert!(budget.allow(start + Duration::from_secs(2)));
        // Fourth attempt inside the window is refused and opens the breaker.
        assert!(!budget.allow(start + Duration::from_secs(3)));
        assert!(budget.open_for(start + Duration::from_secs(3)).is_some());
        // Still closed inside the cooldown.
        assert!(!budget.allow(start + Duration::from_secs(30)));
        // Half-open after the cooldown: exactly one attempt...
        assert!(budget.allow(start + Duration::from_secs(64)));
        // ...and the breaker re-arms at once until a success is proven, so a
        // half-open failure does not hand out attempts on demand.
        assert!(!budget.allow(start + Duration::from_secs(65)));
        assert!(!budget.allow(start + Duration::from_secs(130)));
        // The doubled cooldown expired: one more attempt, then brakes again.
        assert!(budget.allow(start + Duration::from_secs(200)));
        assert!(!budget.allow(start + Duration::from_secs(201)));
    }

    #[test]
    fn a_proven_success_clears_the_budget() {
        let mut budget = RestartBudget::new();
        let start = Instant::now();
        assert!(budget.allow(start));
        assert!(budget.allow(start + Duration::from_secs(1)));
        assert!(budget.allow(start + Duration::from_secs(2)));
        assert!(!budget.allow(start + Duration::from_secs(3)));
        // The Agent stayed reachable long enough: clean slate.
        budget.reset();
        assert!(budget.allow(start + Duration::from_secs(200)));
        assert!(budget.allow(start + Duration::from_secs(201)));
        assert!(budget.allow(start + Duration::from_secs(202)));
        assert!(!budget.allow(start + Duration::from_secs(203)));
    }

    /// The case the breaker exists for: the Agent becomes ready, runs the task,
    /// and the host kills it. Every lap "succeeds", so stability — not mere
    /// attachment — is what may clear the budget.
    #[test]
    fn becoming_ready_again_does_not_count_as_stable() {
        let start = Instant::now();
        assert_eq!(
            Attached::of(start, start + Duration::from_secs(5)),
            Attached::Recently
        );
        assert_eq!(
            Attached::of(start, start + Duration::from_secs(59)),
            Attached::Recently
        );
        assert_eq!(Attached::of(start, start + STABLE_AFTER), Attached::Stable);
    }

    #[test]
    fn attempts_outside_the_window_do_not_count() {
        let mut budget = RestartBudget::new();
        let start = Instant::now();
        // Two attempts age out of the five-minute window before the next run.
        assert!(budget.allow(start));
        assert!(budget.allow(start + Duration::from_secs(1)));
        assert!(budget.allow(start + Duration::from_secs(400)));
        assert!(budget.allow(start + Duration::from_secs(401)));
        // Three inside the window is the threshold.
        assert!(budget.allow(start + Duration::from_secs(402)));
        assert!(!budget.allow(start + Duration::from_secs(403)));
    }
}
