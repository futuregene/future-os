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

use std::future::Future;
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
    /// The endpoint exists but connection/readiness did not complete. An owner
    /// may repair its service after a grace window; attach-only clients retry.
    Unresponsive(String),
    /// A refusal no amount of waiting repairs; the message is user-facing.
    Blocked(String),
}

/// How many times one readiness probe is attempted before a transient
/// "could not verify" answer is reported as final.
///
/// One attempt is not enough on a loaded or network-backed host. The process can
/// be descheduled for longer than the handshake timeout (a CPU quota on a login
/// node, a page-fault storm, a synchronous write to a file system that stalls),
/// and when it resumes the first probe's deadline has already expired even
/// though the Agent answered meanwhile. A retry starts a fresh deadline on a
/// fresh connection, so a stall costs one attempt instead of the whole entry.
pub const READINESS_PROBE_ATTEMPTS: u32 = 3;

/// First backoff between readiness attempts; each further attempt doubles it.
const READINESS_RETRY_BASE: Duration = Duration::from_millis(250);

/// Upper bound on one readiness backoff, so a probe never waits unboundedly.
const READINESS_RETRY_CAP: Duration = Duration::from_secs(2);

/// Probe the local Agent and classify the result.
///
/// A successful selection is dropped immediately: this only classifies. An
/// answer that a later attempt could change (`Unverified`: the readiness
/// handshake timed out, or the RPC failed) is retried with exponential backoff
/// before it is classified as `Blocked`.
pub async fn probe_local_agent(
    configured: &str,
    connect_timeout: Duration,
    handshake_timeout: Duration,
) -> LocalAgentState {
    let outcome = with_readiness_retries(READINESS_PROBE_ATTEMPTS, READINESS_RETRY_BASE, || {
        connection::select_agent(
            configured,
            connection::EXPECTED_AGENT_VERSION,
            connect_timeout,
            handshake_timeout,
        )
    })
    .await;
    match outcome {
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
        Err(error @ (ConnectionError::Transport(_) | ConnectionError::Unverified(_))) => {
            LocalAgentState::Unresponsive(error.to_string())
        }
        Err(error) => LocalAgentState::Blocked(error.to_string()),
    }
}

/// A slow probe must not cancel healthy work. Recovery needs both repeated
/// failures and elapsed time; a successful probe resets the entire window.
#[derive(Default)]
pub struct ReadinessLoss {
    since: Option<Instant>,
    failures: u32,
}
impl ReadinessLoss {
    pub fn reset(&mut self) {
        self.since = None;
        self.failures = 0;
    }

    pub fn confirmed(&mut self, now: Instant) -> bool {
        let since = *self.since.get_or_insert(now);
        self.failures = self.failures.saturating_add(1);
        self.failures >= 3 && now.saturating_duration_since(since) >= Duration::from_secs(15)
    }
}

/// Run one probe, retrying only the answers that another attempt could change.
async fn with_readiness_retries<T, F, Fut>(
    attempts: u32,
    retry_base: Duration,
    mut probe: F,
) -> Result<T, ConnectionError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, ConnectionError>>,
{
    let attempts = attempts.max(1);
    let mut attempt = 1;
    loop {
        match probe().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                if !readiness_retryable(&error) || attempt >= attempts {
                    return Err(error);
                }
                tokio::time::sleep(readiness_retry_delay(retry_base, attempt)).await;
                attempt += 1;
            }
        }
    }
}

/// Whether another readiness attempt could answer differently.
///
/// `Unverified` is "the Agent did not answer in time" (or answered with an RPC
/// error): a momentarily frozen process, a stalled socket or a transport burst
/// all produce it, and the next attempt starts a fresh deadline on a fresh
/// connection. Every other variant is a definite answer — a version/protocol
/// mismatch, a changed instance identity, a refused start — and retrying would
/// only repeat it.
fn readiness_retryable(error: &ConnectionError) -> bool {
    matches!(error, ConnectionError::Unverified(_))
}

/// Backoff before readiness attempt `attempt` (1-based): `base × 2^(n-1)`,
/// capped so one probe cannot grow without bound.
fn readiness_retry_delay(base: Duration, attempt: u32) -> Duration {
    base.saturating_mul(1u32 << attempt.saturating_sub(1).min(5))
        .min(READINESS_RETRY_CAP)
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
    fn recovery_needs_repeated_loss_and_a_success_starts_a_new_window() {
        let mut loss = ReadinessLoss::default();
        let now = Instant::now();
        assert!(!loss.confirmed(now));
        assert!(!loss.confirmed(now + Duration::from_secs(20)));
        assert!(loss.confirmed(now + Duration::from_secs(21)));
        loss.reset();
        assert!(!loss.confirmed(now + Duration::from_secs(30)));
        assert!(!loss.confirmed(now + Duration::from_secs(31)));
        assert!(!loss.confirmed(now + Duration::from_secs(32)));
        assert!(loss.confirmed(now + Duration::from_secs(45)));
    }

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

    #[test]
    fn readiness_backoff_doubles_and_caps() {
        let base = Duration::from_millis(250);
        assert_eq!(readiness_retry_delay(base, 1), Duration::from_millis(250));
        assert_eq!(readiness_retry_delay(base, 2), Duration::from_millis(500));
        assert_eq!(readiness_retry_delay(base, 3), Duration::from_millis(1000));
        // The fourth rung would be 2s, which the cap already allows; later
        // attempts stay there instead of growing without bound.
        assert_eq!(readiness_retry_delay(base, 4), READINESS_RETRY_CAP);
        assert_eq!(readiness_retry_delay(base, 99), READINESS_RETRY_CAP);
    }

    #[test]
    fn only_unverified_readiness_answers_are_retried() {
        assert!(readiness_retryable(&ConnectionError::Unverified(
            "handshake timed out".into()
        )));
        assert!(!readiness_retryable(&ConnectionError::Incompatible {
            agent_version: None,
            reason: "mismatch".into(),
        }));
        assert!(!readiness_retryable(&ConnectionError::InstanceChanged));
        assert!(!readiness_retryable(&ConnectionError::StartupBlocked(
            "refused".into()
        )));
        assert!(!readiness_retryable(&ConnectionError::InstanceBusy(
            "held".into()
        )));
    }

    #[tokio::test]
    async fn a_readiness_timeout_is_retried_until_the_probe_gives_up() {
        let mut calls = 0;
        let outcome: Result<(), ConnectionError> =
            with_readiness_retries(READINESS_PROBE_ATTEMPTS, Duration::from_millis(1), || {
                calls += 1;
                std::future::ready(Err(ConnectionError::Unverified(
                    "handshake timed out".into(),
                )))
            })
            .await;
        assert!(matches!(outcome, Err(ConnectionError::Unverified(_))));
        assert_eq!(calls, READINESS_PROBE_ATTEMPTS as usize);
    }

    #[tokio::test]
    async fn a_definite_readiness_answer_is_not_retried() {
        let mut calls = 0;
        let outcome: Result<(), ConnectionError> =
            with_readiness_retries(5, Duration::from_millis(1), || {
                calls += 1;
                std::future::ready(Err(ConnectionError::Incompatible {
                    agent_version: None,
                    reason: "mismatch".into(),
                }))
            })
            .await;
        assert!(matches!(outcome, Err(ConnectionError::Incompatible { .. })));
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn a_later_readiness_attempt_can_succeed() {
        let mut calls = 0;
        let outcome: Result<&'static str, ConnectionError> =
            with_readiness_retries(3, Duration::from_millis(1), || {
                calls += 1;
                std::future::ready(if calls < 3 {
                    Err(ConnectionError::Unverified("handshake timed out".into()))
                } else {
                    Ok("attached")
                })
            })
            .await;
        assert_eq!(outcome.unwrap(), "attached");
        assert_eq!(calls, 3);
    }
}
