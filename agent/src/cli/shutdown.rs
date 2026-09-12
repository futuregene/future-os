//! Keep Ctrl-C handling alive through runtime teardown and sandbox cleanup.
//!
//! Tokio's default runtime drop waits indefinitely for blocking tasks. Listening
//! on the agent runtime stops working just when those tasks (or a session/log
//! lock) can stall shutdown. A dedicated thread/runtime has no agent work or
//! logging on it, so a second interrupt or the grace deadline can always exit.

use futures_util::{Stream, StreamExt};
use std::io;
use std::time::Duration;
use tokio::sync::oneshot;

const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub(super) type ShutdownRequest = oneshot::Receiver<io::Result<()>>;

pub(super) struct ShutdownGuard {
    finished: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ShutdownGuard {
    pub(super) fn start() -> io::Result<(Self, ShutdownRequest)> {
        let signals =
            futures_util::stream::repeat_with(tokio::signal::ctrl_c).then(|signal| signal);
        Self::with_signals(signals, SHUTDOWN_GRACE)
    }

    fn with_signals(
        signals: impl Stream<Item = io::Result<()>> + Send + 'static,
        grace: Duration,
    ) -> io::Result<(Self, ShutdownRequest)> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (requested_tx, requested_rx) = oneshot::channel();
        let (finished_tx, finished_rx) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("agent-shutdown".into())
            .spawn(move || {
                let force_exit = runtime.block_on(async {
                    tokio::select! {
                        biased;
                        _ = finished_rx => false,
                        force = watch_interrupts(signals, requested_tx, grace) => force,
                    }
                });
                if force_exit {
                    // Do not log here: stdout/stderr or the log-file mutex may
                    // be what blocked shutdown. Forced exit skips destructors;
                    // Windows Job handles close on process exit and stale ACEs
                    // are recovered by the next agent's startup cleanup.
                    std::process::exit(130);
                }
            })?;
        Ok((
            Self {
                finished: Some(finished_tx),
                thread: Some(thread),
            },
            requested_rx,
        ))
    }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        if let Some(finished) = self.finished.take() {
            let _ = finished.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Return true only after a successful first interrupt, followed by another
/// interrupt or the deadline. Signal registration errors use the normal error
/// path instead of silently masquerading as Ctrl-C.
async fn watch_interrupts(
    signals: impl Stream<Item = io::Result<()>>,
    requested: oneshot::Sender<io::Result<()>>,
    grace: Duration,
) -> bool {
    tokio::pin!(signals);
    let first = signals
        .next()
        .await
        .unwrap_or_else(|| Err(io::Error::other("Ctrl-C listener ended")));
    let received = first.is_ok();
    // Even if the server already returned (and dropped its receiver), arm the
    // deadline: runtime destruction or sandbox cleanup may still be stuck.
    let _ = requested.send(first);
    if !received {
        return false;
    }
    let deadline = tokio::time::sleep(grace);
    tokio::pin!(deadline);
    tokio::select! {
        _ = &mut deadline => {},
        second = signals.next() => {
            if !matches!(second, Some(Ok(()))) {
                // A broken listener must not disable the already-armed timer.
                deadline.await;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    #[tokio::test(start_paused = true)]
    async fn first_interrupt_requests_cleanup_then_deadline_forces_exit() {
        let (signals, rx) = tokio::sync::mpsc::unbounded_channel();
        let (requested, request) = oneshot::channel();
        let watcher = tokio::spawn(watch_interrupts(
            UnboundedReceiverStream::new(rx),
            requested,
            SHUTDOWN_GRACE,
        ));
        // Merely running for a long time must not start the shutdown deadline.
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(!watcher.is_finished());
        signals.send(Ok(())).unwrap();
        request.await.unwrap().unwrap();
        tokio::time::advance(SHUTDOWN_GRACE - Duration::from_millis(1)).await;
        assert!(!watcher.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(watcher.await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn second_interrupt_forces_exit_without_waiting_for_deadline() {
        let (signals, rx) = tokio::sync::mpsc::unbounded_channel();
        let (requested, request) = oneshot::channel();
        let watcher = tokio::spawn(watch_interrupts(
            UnboundedReceiverStream::new(rx),
            requested,
            SHUTDOWN_GRACE,
        ));
        signals.send(Ok(())).unwrap();
        request.await.unwrap().unwrap();
        let start = tokio::time::Instant::now();
        signals.send(Ok(())).unwrap();
        assert!(watcher.await.unwrap());
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_still_applies_after_server_and_signal_stream_end() {
        let (requested, request) = oneshot::channel();
        drop(request);
        let start = tokio::time::Instant::now();
        assert!(
            watch_interrupts(
                futures_util::stream::iter([Ok(())]),
                requested,
                SHUTDOWN_GRACE,
            )
            .await
        );
        assert_eq!(start.elapsed(), SHUTDOWN_GRACE);
    }

    #[tokio::test]
    async fn signal_registration_failure_is_returned_without_forcing_exit() {
        let (requested, request) = oneshot::channel();
        assert!(
            !watch_interrupts(
                futures_util::stream::iter([Err(io::Error::other("registration failed"))]),
                requested,
                SHUTDOWN_GRACE,
            )
            .await
        );
        assert_eq!(
            request.await.unwrap().unwrap_err().to_string(),
            "registration failed"
        );
    }

    // Run the force-exit path in a child test process: never send a console
    // interrupt to the developer's running agent or to the test runner. The
    // child exercises the real watchdog thread and process::exit on every OS.
    #[test]
    fn stalled_shutdown_subprocess() {
        const CHILD_MODE: &str = "FUTURE_TEST_STALLED_SHUTDOWN";
        if let Ok(mode) = std::env::var(CHILD_MODE) {
            let (signals, rx) = tokio::sync::mpsc::unbounded_channel();
            let grace = if mode == "second-interrupt" {
                Duration::from_secs(60)
            } else {
                Duration::from_millis(100)
            };
            let (_guard, request) =
                ShutdownGuard::with_signals(UnboundedReceiverStream::new(rx), grace).unwrap();
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            let (started_tx, started_rx) = oneshot::channel();
            runtime.spawn_blocking(move || {
                started_tx.send(()).unwrap();
                loop {
                    std::thread::park();
                }
            });
            runtime.block_on(async {
                started_rx.await.unwrap();
                signals.send(Ok(())).unwrap();
                request.await.unwrap().unwrap();
            });
            if mode == "second-interrupt" {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(100));
                    signals.send(Ok(())).unwrap();
                });
            }
            if mode == "cleanup-lock" {
                // Simulate a synchronous session/log/cleanup lock blocking the
                // main thread before it even reaches runtime destruction.
                let lock = std::sync::Mutex::new(());
                let _held = lock.lock().unwrap();
                let _blocked = lock.lock().unwrap();
            }
            drop(runtime); // Default drop cannot finish the blocking task.
            panic!("stalled runtime unexpectedly returned");
        }

        for mode in ["runtime-drop", "cleanup-lock", "second-interrupt"] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "cli::shutdown::tests::stalled_shutdown_subprocess",
                ])
                // The child exits the whole process, so its test-harness output
                // is noise; keep stderr to surface a real panic.
                .stdout(std::process::Stdio::null())
                .env(CHILD_MODE, mode)
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("watchdog did not terminate child: {mode}");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(status.code(), Some(130), "{mode}");
        }
    }

    #[test]
    fn completed_cleanup_disarms_deadline_and_joins_signal_thread() {
        let (signals, rx) = tokio::sync::mpsc::unbounded_channel();
        let (guard, request) =
            ShutdownGuard::with_signals(UnboundedReceiverStream::new(rx), SHUTDOWN_GRACE).unwrap();
        signals.send(Ok(())).unwrap();
        request.blocking_recv().unwrap().unwrap();
        // Drop must cancel the already-armed timer, not wait for it to fire.
        drop(guard);
    }

    #[test]
    fn normal_exit_stops_and_joins_signal_thread() {
        let (guard, request) = ShutdownGuard::start().unwrap();
        drop(guard);
        assert!(request.blocking_recv().is_err());
    }
}
