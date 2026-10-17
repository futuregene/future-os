//! A cheap bounded RPC probe. It observes the existing Agent and never starts one.
//!
//! The probe's answer is published to the phone as `agentAvailable`, and the
//! phone renders `agentAvailable: false` as "the desktop is preparing" — so a
//! probe that fails while nothing is wrong reads to the user as a dropped
//! remote connection. A local RPC is not a health assertion when the machine is
//! running a build: the previous 3-second deadline failed 33 times in this
//! machine's own log, and refusing to distinguish "slow" from "absent" turned
//! load into a reported outage.
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long one probe may take before it counts as a failure. Generous on
/// purpose: a *successful* probe is the only thing that proves the Agent is
/// there, and waiting longer costs nothing but a later correction.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Consecutive failures before the bridge tells the phone the Agent is gone.
/// One unlucky probe is not evidence.
const FAILURES_BEFORE_UNAVAILABLE: u32 = 2;

/// How old the last successful probe may be and still count as evidence. Must
/// exceed the worst-case gap between successes (a full timeout plus the sleep
/// below), or a slow-but-alive Agent would still flap the phone.
const FRESHNESS: Duration = Duration::from_secs(30);

/// Pause between probes.
const INTERVAL: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Availability(Mutex<Option<Observations>>);

#[derive(Default)]
struct Observations {
    last_success: Option<Instant>,
    consecutive_failures: u32,
}

impl Availability {
    fn record_success(&self, now: Instant) {
        *self.0.lock().unwrap() = Some(Observations {
            last_success: Some(now),
            consecutive_failures: 0,
        });
    }

    fn record_failure(&self) {
        let mut guard = self.0.lock().unwrap();
        let state = guard.get_or_insert_with(Observations::default);
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    }

    fn available(&self, now: Instant) -> bool {
        self.0.lock().unwrap().as_ref().is_some_and(|state| {
            state.consecutive_failures < FAILURES_BEFORE_UNAVAILABLE
                && state
                    .last_success
                    .is_some_and(|at| now.saturating_duration_since(at) <= FRESHNESS)
        })
    }
}
static HEALTH: Availability = Availability(Mutex::new(None));
pub(crate) fn available() -> bool {
    HEALTH.available(Instant::now())
}
pub(crate) async fn monitor() {
    let mut previous = None;
    let mut last_warning: Option<Instant> = None;
    loop {
        let result = tokio::time::timeout(PROBE_TIMEOUT, async {
            let mut client = crate::agent_bridge::connect_agent().await?;
            use crate::agent_bridge::RpcResponseExt;
            client
                .execute_command(crate::agent_bridge::list_streaming_sessions_command())
                .await
                .map_err(|e| crate::AppError::AgentUnavailable(e.to_string()))?
                .into_inner()
                .ok_or_rpc_error("Agent health check failed")?;
            Ok::<_, crate::AppError>(())
        })
        .await;
        let available = matches!(result, Ok(Ok(())));
        let now = Instant::now();
        if available {
            HEALTH.record_success(now);
        } else {
            HEALTH.record_failure();
        }
        let reported = HEALTH.available(now);
        if previous != Some(reported) {
            if !reported
                && last_warning
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(300))
            {
                eprintln!("remote: Agent availability probe failed [LC003]: {result:?}");
                last_warning = Some(now);
            }
            previous = Some(reported);
        }
        tokio::time::sleep(INTERVAL).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_requires_a_recent_success() {
        let health = Availability::default();
        let now = Instant::now();
        assert!(!health.available(now));

        health.record_success(now);
        assert!(health.available(now + Duration::from_secs(9)));
        assert!(!health.available(now + FRESHNESS + Duration::from_secs(1)));
    }

    #[test]
    fn one_slow_probe_does_not_report_the_agent_unavailable() {
        let health = Availability::default();
        let now = Instant::now();
        health.record_success(now);

        // A single probe hit its deadline while the machine was busy; the Agent
        // is still the one we answered with a moment ago.
        health.record_failure();
        assert!(health.available(now + Duration::from_secs(1)));

        let next = now + Duration::from_secs(4);
        health.record_success(next);
        health.record_failure();
        assert!(health.available(next));
    }

    #[test]
    fn repeated_failures_report_it_and_a_success_recovers() {
        let health = Availability::default();
        let now = Instant::now();
        health.record_success(now);
        health.record_failure();
        health.record_failure();
        assert!(!health.available(now));

        health.record_success(now);
        assert!(health.available(now));
    }

    #[test]
    fn failures_after_expiry_cannot_resurrect_an_old_success() {
        let health = Availability::default();
        let now = Instant::now();
        health.record_success(now);
        // A probe that never recovered: the last success ages out on its own.
        assert!(!health.available(now + FRESHNESS + Duration::from_secs(1)));
    }
}
