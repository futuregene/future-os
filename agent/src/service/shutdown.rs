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

    fn drain(&self) -> Vec<String> {
        let mut errors = Vec::new();
        self.state
            .approval_gate
            .cancel_all("Agent is shutting down.");
        // Abort approvals/shells before waiting for blocking RPC handlers: a
        // shell command or a tool awaiting approval is itself admitted work.
        let sessions = self
            .state
            .sessions
            .read()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in &sessions {
            let session = session.read();
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
        if !self.wait("RPC listener and streams", || {
            !self.shutdown.server_running.load(Ordering::Acquire)
        }) {
            return errors;
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
