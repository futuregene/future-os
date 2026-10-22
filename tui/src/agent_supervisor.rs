//! TUI Agent lifecycle: attach, host in-process, and keep watching.
//!
//! The TUI runs the Agent in its own process, the same way `future host` and
//! the desktop do. Only the unified `future` binary may do so (see
//! [`crate::agent_host::hostable`]); the standalone `future-tui` binary attaches to
//! an Agent somebody else started and reports why it cannot host one.
//!
//! What this module owns is the *policy*: when to attach, when to wait for another
//! process, when to start, and when to stop trying. The decisions themselves come
//! from `future_rpc::supervision`, so all three entry points answer "who owns the
//! Agent" the same way.

use std::time::{Duration, Instant};

use future_rpc::supervision::{
    probe_local_agent, restart_delay, Attached, LocalAgentState, RestartBudget, WAIT_FOR_OWNER,
    WAIT_NOTICE_AFTER, WAIT_PROBE_INTERVAL,
};

/// Per-attempt probe budget. Local IPC answers in milliseconds when it answers.
const PROBE_CONNECT: Duration = Duration::from_millis(400);
const PROBE_HANDSHAKE: Duration = Duration::from_secs(2);

/// How long a clean stop may take before the exit path gives up on it and says so
/// rather than keeping the terminal waiting.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// How often an attached Agent is re-probed: long enough to stay out of the way,
/// short enough that a death is noticed before the user blames the TUI.
const PROBE_INTERVAL: Duration = Duration::from_secs(3);

/// Attach to a serving Agent, wait for one another process is starting, or host
/// one here.
///
/// Losing the instance-lock race is not a reason to refuse to start: another
/// process is bringing the Agent up, and the only thing left to do is wait for
/// it. Waiting is bounded ([`WAIT_FOR_OWNER`]) and says so while it happens — a
/// client that waits silently and then fails is worse than one that fails at
/// once, because the user cannot tell the two apart.
///
/// A refusal no wait repairs (an unreachable explicit endpoint, a version
/// mismatch, a stop in progress) is reported immediately.
pub async fn ensure_agent_running(configured: &str) -> Result<(), String> {
    let started = Instant::now();
    let mut last_notice: Option<Instant> = None;
    loop {
        match probe_local_agent(configured, PROBE_CONNECT, PROBE_HANDSHAKE).await {
            LocalAgentState::Attached => return Ok(()),
            LocalAgentState::Blocked(reason) => return Err(reason),
            LocalAgentState::Busy(detail) => {
                let waited = started.elapsed();
                if waited >= WAIT_FOR_OWNER {
                    return Err(format!(
                        "another process has held this data root's Agent lock for {WAIT_FOR_OWNER:?} \
                         without serving; it is not starting. Stop it, then start the TUI again. {detail}"
                    ));
                }
                let due = last_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER);
                if due && waited >= WAIT_NOTICE_AFTER {
                    eprintln!(
                        "future-tui: waiting for the Agent another process is starting \
                         ({waited:?} of {WAIT_FOR_OWNER:?}). {detail}"
                    );
                    last_notice = Some(Instant::now());
                }
                tokio::time::sleep(WAIT_PROBE_INTERVAL).await;
            }
            LocalAgentState::Startable => {
                // The permit from the probe is long gone; the start below takes
                // its own and fails cleanly if another process won the race.
                match crate::agent_host::start_in_process(configured.to_string()).await {
                    Ok(()) => return Ok(()),
                    // Another process took the lock between the probe and the
                    // start: that is a yield, not a failure.
                    Err(error) if is_busy(&error) => {
                        let waited = started.elapsed();
                        let due = last_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER);
                        if due && waited >= WAIT_NOTICE_AFTER {
                            eprintln!(
                                "future-tui: another process took the Agent lock while starting; \
                                 waiting for it ({waited:?} of {WAIT_FOR_OWNER:?}). {error}"
                            );
                            last_notice = Some(Instant::now());
                        }
                        if waited >= WAIT_FOR_OWNER {
                            return Err(format!(
                                "another process holds this data root's Agent lock and has not \
                                 served for {WAIT_FOR_OWNER:?}: {error}"
                            ));
                        }
                        tokio::time::sleep(WAIT_PROBE_INTERVAL).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
}

/// Whether a start failed because another process owns the instance lock.
///
/// `future-host` maps that case to its own error variant, so this reads the type
/// rather than the message.
fn is_busy(error: &str) -> bool {
    error.contains("Another process owns this data root")
}

/// Keep watching the Agent for as long as the TUI runs.
///
/// The launch happens once, which is why an Agent that dies later would leave the
/// terminal reconnecting forever. This task closes that: it notices the absence,
/// starts a replacement when nobody holds the instance lock, waits for the process
/// that does, and rate-limits its own restarts so an Agent the host keeps killing
/// cannot become a loop.
pub fn spawn_agent_supervision(handle: &tokio::runtime::Handle, configured: String) {
    handle.spawn(async move {
        let mut budget = RestartBudget::new();
        let mut waiting_since: Option<Instant> = None;
        let mut lost_notice: Option<Instant> = None;
        let mut reachable_since: Option<Instant> = None;
        loop {
            tokio::time::sleep(PROBE_INTERVAL).await;
            match probe_local_agent(&configured, PROBE_CONNECT, PROBE_HANDSHAKE).await {
                LocalAgentState::Attached => {
                    if waiting_since.take().is_some() {
                        eprintln!("future-tui: the Agent is serving again.");
                    }
                    lost_notice = None;
                    // Only a lasting recovery is a success: an Agent the host
                    // keeps killing becomes ready on every lap, and clearing the
                    // budget on "attached" would reset the breaker each time.
                    let now = Instant::now();
                    let since = *reachable_since.get_or_insert(now);
                    if Attached::of(since, now) == Attached::Stable {
                        budget.reset();
                    }
                }
                LocalAgentState::Busy(detail) => {
                    let now = Instant::now();
                    let since = *waiting_since.get_or_insert(now);
                    let waited = now.duration_since(since);
                    let due = lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER)
                        && waited >= WAIT_NOTICE_AFTER;
                    if due {
                        eprintln!(
                            "future-tui: waiting for the Agent another process is starting \
                             ({waited:?} of {WAIT_FOR_OWNER:?}). {detail}"
                        );
                        lost_notice = Some(now);
                    }
                    if waited >= WAIT_FOR_OWNER {
                        eprintln!(
                            "future-tui: no Agent has served this data root for {WAIT_FOR_OWNER:?}; \
                             still waiting. The TUI keeps reconnecting, and a new attempt starts \
                             when the lock is released."
                        );
                        waiting_since = None;
                        lost_notice = Some(now);
                    }
                }
                LocalAgentState::Blocked(reason) => {
                    // Not repairable by waiting (version mismatch, explicit
                    // endpoint). Keep the connection banner and look again later.
                    if lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER) {
                        eprintln!("future-tui: cannot use the Agent: {reason}");
                        lost_notice = Some(Instant::now());
                    }
                }
                LocalAgentState::Startable => {
                    waiting_since = None;
                    reachable_since = None;
                    let now = Instant::now();
                    if !budget.allow(now) {
                        if lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER) {
                            eprintln!(
                                "future-tui: the Agent keeps exiting right after it starts; \
                                 not restarting it for {:?}. Check the host log: {}",
                                budget.open_for(now).unwrap_or_default(),
                                future_host::environment::log_file_path().display()
                            );
                            lost_notice = Some(now);
                        }
                        continue;
                    }
                    // A host that only attached to the dead Agent must let go
                    // first, or the stale `Ready` makes the start a no-op.
                    crate::agent_host::detach_attached();
                    match crate::agent_host::start_in_process(configured.clone()).await {
                        Ok(()) => {
                            eprintln!("future-tui: the Agent was gone; started a new one.");
                            reachable_since = None;
                            lost_notice = None;
                        }
                        Err(error) => {
                            if lost_notice.is_none_or(|at| at.elapsed() >= WAIT_NOTICE_AFTER) {
                                eprintln!("future-tui: could not start an Agent: {error}");
                                lost_notice = Some(Instant::now());
                            }
                            tokio::time::sleep(restart_delay(2)).await;
                        }
                    }
                }
            }
        }
    });
}

/// Stop an Agent this TUI owns, on the exit path.
///
/// An owned Agent holds the instance lock and the data-directory database, so
/// leaving it to process teardown would make the next start report a lock it
/// cannot explain. Bounded: a stop that cannot finish is reported, not retried
/// forever while the terminal waits.
pub fn stop_owned_agent() {
    if !crate::agent_host::owns_agent() {
        return;
    }
    if let Err(error) = crate::agent_host::stop(STOP_TIMEOUT) {
        eprintln!("future-tui: the Agent did not stop cleanly: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The busy signal has to be recognized, or a lost lock race would be
    /// reported as a startup failure — the exact behaviour this replaced.
    #[test]
    fn a_lost_lock_race_is_recognized_as_busy() {
        assert!(is_busy(
            "Another process owns this data root; waiting for it instead of starting a second Agent"
        ));
        assert!(!is_busy("Incompatible Future Agent: protocol 2"));
        assert!(!is_busy("configured external endpoint is unavailable"));
    }
}
