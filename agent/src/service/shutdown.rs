//! A native supervisor drains work; a separate native deadline watcher never
//! depends on a Tokio worker, session lock, logger or SQLite acknowledgement.
use super::AgentHandle;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShutdownStatus {
    Stopped,
    FailedStopping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownReport {
    pub status: ShutdownStatus,
    pub errors: Vec<String>,
}

/// Sub-step names for the admission phase of a drain.
///
/// A timeout report prints whichever stage name was current when the budget ran
/// out, so the single name "admission" could not say whether the time went on the
/// session-map snapshot or on one session's lock — the difference between "the
/// service is wedged" and "one busy session is wedged". These are set *before*
/// the step that may block, which is what makes them work at all: a drain stuck
/// past its deadline never reaches its own "finished" line, and the deadline
/// watcher reads only this name.
const STAGE_SESSION_SNAPSHOT: &str = "admission: session snapshot";
const STAGE_INTERRUPTING_SESSIONS: &str = "admission: interrupting sessions";

/// The drain's last wait before storage, named once so the report and the log
/// line cannot drift apart.
const STAGE_LISTENER: &str = "RPC listener and streams";
/// The same stage after the graceful drain was abandoned. A report naming this
/// says the server was dropped, not that a client was still being waited for.
const STAGE_LISTENER_ABANDONED: &str = "RPC listener and streams (abandoned)";

/// How much of a stop's five seconds the listener's graceful drain may use.
///
/// Long enough for the streams `close_streams` just ended to reach their peers
/// and for a client to close its connection (milliseconds on the local endpoint;
/// a stop with a silent peer measured ~35 ms of real work), short enough that a
/// peer which never finishes costs half a second instead of the stop's budget.
const LISTENER_GRACE: Duration = Duration::from_millis(500);

/// How long one shutdown sub-step may take before it is worth a warning.
///
/// Measurement only: nothing here changes what the drain does or how long it is
/// allowed to take. The whole stop budget is five seconds, so a quarter of a
/// second inside a single step is already the interesting case, and several such
/// steps together are what actually exhausts the budget.
const SLOW_SHUTDOWN_STEP: Duration = Duration::from_millis(250);

pub(super) struct ShutdownShared {
    started: AtomicBool,
    pub(super) server_running: AtomicBool,
    stage: Mutex<&'static str>,
    report: watch::Sender<Option<ShutdownReport>>,
}

impl Default for ShutdownShared {
    fn default() -> Self {
        Self {
            started: AtomicBool::new(false),
            server_running: AtomicBool::new(false),
            stage: Mutex::new("admission"),
            report: watch::channel(None).0,
        }
    }
}

impl ShutdownShared {
    fn finish(&self, errors: Vec<String>) {
        // Reports are terminal and idempotent: a late worker completion must
        // never turn an already reported timeout into permission to restart.
        self.report.send_if_modified(|report| {
            if report.is_some() {
                return false;
            }
            *report = Some(ShutdownReport {
                status: if errors.is_empty() {
                    ShutdownStatus::Stopped
                } else {
                    ShutdownStatus::FailedStopping
                },
                errors,
            });
            true
        });
    }

    pub(super) fn failed(&self, message: String) {
        self.finish(vec![message]);
    }
    pub(super) fn report(&self) -> Option<ShutdownReport> {
        self.report.borrow().clone()
    }
}

impl AgentHandle {
    /// The first request freezes admission synchronously. Repeated callers
    /// observe the same terminal report and never create another supervisor.
    pub fn request_shutdown(&self, deadline: Duration) {
        self.state.service_work.close();
        self.state.shutting_down.store(true, Ordering::Release);
        if self.shutdown.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(expires) = Instant::now().checked_add(deadline) else {
            self.shutdown
                .failed("invalid Agent shutdown deadline".into());
            return;
        };
        let shared = self.shutdown.clone();
        let watcher = std::thread::Builder::new().name("agent-shutdown-deadline".into()).spawn(move || {
            while shared.report().is_none() {
                if Instant::now() >= expires {
                    let stage = *shared.stage.lock();
                    shared.failed(format!("Agent shutdown deadline exceeded during {stage}; instance ownership retained"));
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        if let Err(error) = watcher {
            self.shutdown.failed(format!(
                "could not start shutdown deadline watcher: {error}"
            ));
            return;
        }
        let handle = self.clone();
        let worker = std::thread::Builder::new()
            .name("agent-shutdown".into())
            .spawn(move || {
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle.drain()));
                match outcome {
                    Ok(errors) => handle.shutdown.finish(errors),
                    Err(_) => handle
                        .shutdown
                        .failed("Agent shutdown supervisor panicked".into()),
                }
            });
        if let Err(error) = worker {
            self.shutdown
                .failed(format!("could not start shutdown supervisor: {error}"));
        }
    }

    pub async fn shutdown(&self, deadline: Duration) -> ShutdownReport {
        self.request_shutdown(deadline);
        self.wait_for_shutdown().await
    }

    /// Observe termination without initiating it. Hosts must poll this alongside
    /// `serve` so a failed drain cannot leave their native worker waiting forever.
    pub async fn wait_for_shutdown(&self) -> ShutdownReport {
        let mut report = self.shutdown.report.subscribe();
        loop {
            if let Some(report) = report.borrow_and_update().clone() {
                return report;
            }
            if report.changed().await.is_err() {
                return ShutdownReport {
                    status: ShutdownStatus::FailedStopping,
                    errors: vec!["shutdown reporter unavailable".into()],
                };
            }
        }
    }

    pub(super) fn shutdown_blocking(&self, deadline: Duration) -> ShutdownReport {
        self.request_shutdown(deadline);
        loop {
            if let Some(report) = self.shutdown.report() {
                return report;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait(&self, stage: &'static str, mut settled: impl FnMut() -> bool) -> bool {
        *self.shutdown.stage.lock() = stage;
        while !settled() {
            if self.shutdown.report().is_some() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }

    /// [`Self::wait`] with a budget of its own: one step may not hold the whole
    /// stop, so past `budget` this returns `false` and the caller decides what to
    /// do instead of waiting.
    fn wait_within(
        &self,
        budget: Duration,
        stage: &'static str,
        mut settled: impl FnMut() -> bool,
    ) -> bool {
        *self.shutdown.stage.lock() = stage;
        let expires = Instant::now() + budget;
        while !settled() {
            if self.shutdown.report().is_some() || Instant::now() >= expires {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }

    /// Name the sub-step a timeout report should show while it runs.
    ///
    /// Set before the work, never after it: a step that blocks past the deadline
    /// never gets to report its own completion, and the deadline watcher reads
    /// only this name to say where the drain was when its budget ran out.
    fn set_stage(&self, stage: &'static str) {
        *self.shutdown.stage.lock() = stage;
    }

    fn drain(&self) -> Vec<String> {
        let mut errors = Vec::new();
        self.state
            .approval_gate
            .cancel_all("Agent is shutting down.");
        // Abort approvals/shells before waiting for blocking RPC handlers: a
        // shell command or a tool awaiting approval is itself admitted work.
        self.set_stage(STAGE_SESSION_SNAPSHOT);
        let started = Instant::now();
        let sessions = self
            .state
            .sessions
            .read()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let snapshot_waited = started.elapsed();
        if snapshot_waited > SLOW_SHUTDOWN_STEP {
            tracing::warn!(
                step = STAGE_SESSION_SNAPSHOT,
                waited_ms = snapshot_waited.as_millis() as u64,
                sessions = sessions.len(),
                "Agent shutdown step blocked"
            );
        }
        // Per-session interruption, timed per session so a single held lock is
        // named rather than averaged into the whole phase. The session id comes
        // from the guard, not from the map: the snapshot keeps the same shape it
        // always had, and the id is only needed once the lock was released.
        self.set_stage(STAGE_INTERRUPTING_SESSIONS);
        for handle in &sessions {
            let started = Instant::now();
            let session = handle.read();
            let waited = started.elapsed();
            if waited > SLOW_SHUTDOWN_STEP {
                // Safe to read the session's own state here: the guard is ours,
                // so what made the wait long is already gone and the report is
                // about what this session was doing when it was released.
                tracing::warn!(
                    step = STAGE_INTERRUPTING_SESSIONS,
                    session_id = %session.session_id,
                    waited_ms = waited.as_millis() as u64,
                    sessions = sessions.len(),
                    active_run = session.runtime.snapshot().is_some(),
                    "Agent shutdown step blocked"
                );
            }
            let _ = session.interrupt_for_shutdown();
            session
                .approval_gate
                .cancel_session(&session.session_id, "Agent is shutting down.");
            if let Ok(loop_) = session.agent_loop.try_read() {
                loop_.interrupt_flag.store(true, Ordering::Release);
            };
        }
        if !self.wait("admitted commands", || {
            self.state.service_work.counts().0 == 0
        }) {
            return errors;
        }
        // Include sessions whose creation/hydration was already admitted at
        // the boundary, then cancel queued acknowledgements durably.
        let sessions = self
            .state
            .sessions
            .read()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        *self.shutdown.stage.lock() = "queues and cancellation";
        let mut cancelled_queues = Vec::new();
        for session in &sessions {
            let mut session = session.write();
            let _ = session.interrupt_for_shutdown();
            session
                .approval_gate
                .cancel_session(&session.session_id, "Agent is shutting down.");
            if let Ok(loop_) = session.agent_loop.try_read() {
                loop_.interrupt_flag.store(true, Ordering::Release);
            }
            session.stop_scheduler();
            session.runtime.stop_cancellation_timer();
            let cancelled =
                session.cancel_all_queued_runs(crate::runtime::QueuedCancellationReason::Cancelled);
            cancelled_queues.push(cancelled);
        }
        if !self.wait("runs and native background work", || {
            // A run admitted before the boundary can register an approval
            // while it unwinds. Release late registrations as well.
            self.state
                .approval_gate
                .cancel_all("Agent is shutting down.");
            self.state.service_work.counts() == (0, 0)
                && sessions.iter().all(|session| {
                    session.try_read().is_some_and(|session| {
                        !session.runtime.has_owned_task()
                            && !session.compaction_in_progress.load(Ordering::Acquire)
                    })
                })
                // Read after run ownership too: a run may register a child
                // finalizer between the first count read and its own exit.
                && self.state.service_work.counts() == (0, 0)
        }) {
            return errors;
        }
        for (session, cancelled) in sessions.iter().zip(cancelled_queues) {
            let session = session.read();
            if session.ephemeral {
                continue;
            }
            for request in cancelled {
                let mut terminal = crate::session::SessionEntry::run_terminal(
                    &request.run_id,
                    crate::session::RUN_STATE_CANCELLED,
                    0,
                    0,
                    Some("Agent shut down before queued execution"),
                );
                if let Some(content) = terminal.content.as_mut() {
                    content["run_sequence"] = request.run_sequence.into();
                }
                if let Err(error) = session
                    .session_manager
                    .append_entries_synced(&session.session_id, &[terminal])
                {
                    errors.push(format!(
                        "queued run {} terminal persistence: {error:#}",
                        request.run_id
                    ));
                }
            }
        }
        for session in &sessions {
            let session = session.read();
            session.stop_scheduler();
            session.runtime.stop_cancellation_timer();
        }
        if !self.wait("scheduler and cancellation monitors", || {
            sessions.iter().all(|session| {
                session.try_read().is_some_and(|session| {
                    session.scheduler_settled() && session.runtime.auxiliary_settled()
                })
            })
        }) {
            return errors;
        }
        *self.shutdown.stage.lock() = "session writers";
        for session in &sessions {
            let session = session.read();
            session.stop_scheduler();
            session.runtime.stop_cancellation_timer();
            if let Some(active) = session.runtime.snapshot() {
                errors.push(format!(
                    "run {} has unresolved {:?} state",
                    active.run_id, active.phase
                ));
            }
            if let Err(error) = session.persistence.close() {
                errors.push(format!("session writer: {error:#}"));
            }
            if let Some(error) = session.persistence.last_error() {
                errors.push(error);
            }
            if let Err(error) = session.broadcaster.shutdown_journal() {
                errors.push(format!("event journal: {error:#}"));
            }
        }
        // Close subscriptions only after terminal writes. Graceful Tonic
        // shutdown then stops accepting connections and drains active RPCs.
        self.state.service_work.close_streams();
        self.state.service_work.stop_listener();
        // The listener's share of the stop is bounded, and the clients are not
        // the ones who set it. Measured 2026-10-28: one connected peer that
        // never finished its request kept Tonic's graceful shutdown pending
        // until the deadline, which reported "RPC listener and streams" and
        // retained instance ownership for the life of the Desktop process — no
        // Agent could start for ~40 minutes. A peer is not worth the socket, the
        // endpoint and the lock: past this grace the server is dropped.
        if !self.wait_within(LISTENER_GRACE, STAGE_LISTENER, || {
            !self.shutdown.server_running.load(Ordering::Acquire)
        }) {
            if self.shutdown.report().is_none() {
                tracing::warn!(
                    stage = STAGE_LISTENER,
                    grace_ms = LISTENER_GRACE.as_millis() as u64,
                    "Agent shutdown: abandoning the graceful RPC drain"
                );
                self.state.service_work.abandon_listener();
            }
            if !self.wait(STAGE_LISTENER_ABANDONED, || {
                !self.shutdown.server_running.load(Ordering::Acquire)
            }) {
                return errors;
            }
        }
        *self.shutdown.stage.lock() = "journal compactor and SQLite";
        if let Err(error) = self.state.session_manager.shutdown() {
            errors.push(format!("storage shutdown: {error:#}"));
        }
        *self.shutdown.stage.lock() = "Windows sandbox permissions";
        #[cfg(windows)]
        if let Err(error) = crate::sandbox::reset_windows_sandbox_capabilities() {
            errors.push(format!("Windows sandbox cleanup: {error:#}"));
        }
        errors
    }
}

/// Tests live here, not next to `service::tests`, so they can drive `drain` on
/// the test thread and observe a thread-local subscriber.
#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
