//! Startup phases and a watchdog that keeps a stalled launch diagnosable.
//!
//! A launch that blocks inside the kernel — a dead network home, a filesystem
//! filter driver, a wedged child-process spawn — cannot be interrupted from
//! Rust, and pretending otherwise would be worse than saying so. What a host
//! *can* guarantee is that the stall is attributable: every step records its
//! name and start time, and a watchdog reports the step that stopped moving
//! into the host log while the window is still blank. Without that, the only
//! evidence of a failed launch is the user's "it never came up".
//!
//! The tracker is a plain value type (cloned through an `Arc`) so the phase
//! rules are testable without the process-global instance the entry points use.

use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a launch may run before the watchdog calls it stalled.
pub const STALL_AFTER: Duration = Duration::from_secs(20);

/// How often an already-stalled launch re-reports where it is stuck. Long
/// enough to stay readable in the log, short enough to see the same step across
/// a user's repeated attempts.
pub const STALL_REPEAT: Duration = Duration::from_secs(30);

/// Where a startup is right now: the whole launch and the current step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Where {
    pub step: &'static str,
    /// Since the launch began.
    pub total: Duration,
    /// Since `step` began.
    pub step_elapsed: Duration,
}

#[derive(Clone)]
pub struct Tracker(Arc<Shared>);

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

struct State {
    step: &'static str,
    started: Instant,
    step_started: Instant,
    finished: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker {
    pub fn new() -> Self {
        let now = Instant::now();
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                step: "process start",
                started: now,
                step_started: now,
                finished: false,
            }),
            changed: Condvar::new(),
        }))
    }

    /// Record the step that is running now.
    pub fn enter(&self, step: &'static str) {
        let mut state = self.lock();
        if state.finished {
            return;
        }
        state.step = step;
        state.step_started = Instant::now();
        drop(state);
        self.0.changed.notify_all();
    }

    /// Startup completed; stop the watchdog. Idempotent.
    pub fn ready(&self) {
        self.finish();
    }

    /// Startup ended in a failure whose reason is reported elsewhere; stop the
    /// watchdog rather than describing a launch that gave up.
    pub fn failed(&self) {
        self.finish();
    }

    fn finish(&self) {
        self.lock().finished = true;
        self.0.changed.notify_all();
    }

    /// Where the launch is, or `None` once it finished.
    pub fn current(&self) -> Option<Where> {
        let state = self.lock();
        (!state.finished).then(|| Where {
            step: state.step,
            total: state.started.elapsed(),
            step_elapsed: state.step_started.elapsed(),
        })
    }

    /// Whether a still-running launch has exceeded `after` in total.
    pub fn stalled(&self, after: Duration) -> Option<Where> {
        self.current().filter(|at| at.total >= after)
    }

    /// Report a stalled launch into `report`: the first message once the budget
    /// expires, then again every `repeat` or whenever the step changes. The
    /// watchdog never holds up the launch, and stops as soon as
    /// [`Tracker::ready`] or [`Tracker::failed`] runs.
    pub fn arm(
        &self,
        report: impl Fn(&str) + Send + Sync + 'static,
        after: Duration,
        repeat: Duration,
    ) {
        // Shared so the spawn-failure path below can still report; the closure
        // owns one handle and the fallback the other.
        let report = Arc::new(report);
        let for_thread = Arc::clone(&report);
        let tracker = self.clone();
        let spawned = std::thread::Builder::new()
            .name("startup-watchdog".into())
            .spawn(move || {
                let mut reported: Option<(&'static str, Instant)> = None;
                loop {
                    // Wait (without holding the lock) until the launch budget
                    // expires, waking at least once a second so a `ready()` that
                    // lands between steps is noticed promptly.
                    let at = {
                        let mut state = tracker.lock();
                        loop {
                            if state.finished {
                                return;
                            }
                            if state.started.elapsed() >= after {
                                break Where {
                                    step: state.step,
                                    total: state.started.elapsed(),
                                    step_elapsed: state.step_started.elapsed(),
                                };
                            }
                            let budget = after
                                .saturating_sub(state.started.elapsed())
                                .min(Duration::from_secs(1));
                            state = tracker
                                .0
                                .changed
                                .wait_timeout(state, budget)
                                .unwrap_or_else(|poison| poison.into_inner())
                                .0;
                        }
                    };
                    let due = match reported {
                        Some((last, reported_at)) => {
                            last != at.step || reported_at.elapsed() >= repeat
                        }
                        None => true,
                    };
                    if due {
                        reported = Some((at.step, Instant::now()));
                        (*for_thread)(&format!(
                            "startup has not finished after {}s; it is at step '{}' ({}s)",
                            at.total.as_secs(),
                            at.step,
                            at.step_elapsed.as_secs()
                        ));
                    }
                    // Sleep until the step changes or the repeat interval is due.
                    let deadline = Instant::now() + repeat;
                    let mut state = tracker.lock();
                    while !state.finished && state.step == at.step && Instant::now() < deadline {
                        let budget = deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_secs(1));
                        state = tracker
                            .0
                            .changed
                            .wait_timeout(state, budget)
                            .unwrap_or_else(|poison| poison.into_inner())
                            .0;
                    }
                }
            });
        if let Err(error) = spawned {
            (*report)(&format!("startup watchdog could not start: {error}"));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

static GLOBAL: LazyLock<Tracker> = LazyLock::new(Tracker::new);

/// The process-wide tracker used by the entry points.
pub fn global() -> &'static Tracker {
    &GLOBAL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_the_current_step_and_how_long_it_has_run() {
        let tracker = Tracker::new();
        tracker.enter("app store");
        let at = tracker.current().expect("a running launch");
        assert_eq!(at.step, "app store");
        assert!(at.step_elapsed < Duration::from_secs(5));
    }

    #[test]
    fn a_finished_tracker_reports_no_running_step() {
        let tracker = Tracker::new();
        tracker.ready();
        assert!(tracker.current().is_none());
        // Late transitions cannot resurrect a finished launch.
        tracker.enter("app store");
        assert!(tracker.current().is_none());
    }

    #[test]
    fn stalled_needs_the_launch_budget_to_expire() {
        let tracker = Tracker::new();
        tracker.enter("window build");
        assert!(tracker.stalled(Duration::from_secs(60)).is_none());
        assert_eq!(
            tracker.stalled(Duration::ZERO).map(|at| at.step),
            Some("window build")
        );
    }

    #[test]
    fn the_watchdog_reports_a_stalled_launch_and_stops_on_ready() {
        let tracker = Tracker::new();
        tracker.enter("window build");
        let (tx, rx) = std::sync::mpsc::channel();
        tracker.arm(
            move |message| {
                let _ = tx.send(message.to_string());
            },
            Duration::from_millis(20),
            Duration::from_millis(20),
        );
        let first = rx.recv_timeout(Duration::from_secs(5)).expect("a report");
        assert!(first.contains("window build"), "{first}");
        assert!(first.contains("startup"), "{first}");
        tracker.ready();
        // The watchdog stops within a scheduling slice, and a finished launch
        // never reports again.
        let late = rx.recv_timeout(Duration::from_millis(500));
        assert!(late.is_err(), "a ready launch must not keep reporting");
    }

    #[test]
    fn a_new_step_is_reported_even_after_stalling_in_the_previous_one() {
        let tracker = Tracker::new();
        tracker.enter("app store");
        let (tx, rx) = std::sync::mpsc::channel();
        tracker.arm(
            move |message| {
                let _ = tx.send(message.to_string());
            },
            Duration::from_millis(10),
            Duration::from_secs(30),
        );
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
        tracker.enter("agent");
        let second = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the new step");
        assert!(second.contains("agent"), "{second}");
        tracker.failed();
    }

    #[test]
    fn the_watchdog_does_not_outlive_a_failed_launch() {
        let tracker = Tracker::new();
        let (tx, rx) = std::sync::mpsc::channel();
        tracker.arm(
            move |message| {
                let _ = tx.send(message.to_string());
            },
            Duration::from_millis(10),
            Duration::from_millis(10),
        );
        tracker.failed();
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "a failed launch stops the watchdog before it reports"
        );
    }
}
