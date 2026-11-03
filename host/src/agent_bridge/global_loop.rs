//! A process-global agent loop that re-attaches a subscription forever.
//!
//! `config_observer` and `session_events` both own one: a `started` guard, a
//! detached task, and a reconnect loop that re-attaches a plain `stream_events`
//! subscription after every disconnect.
//!
//! In tests that is a hazard, not just bookkeeping. The mock agent hands each
//! scripted stream to **whoever attaches first** — the queue has no notion of
//! whose script it is — so a loop that outlives the test that spawned it takes
//! the stream the *next* test scripted, and that test then waits on the mock's
//! unscripted stream, which is `futures::stream::pending()` forever while it
//! holds the mock's process-wide lock. Measured 2026-11-01: one such steal hung
//! the whole `future-host --lib` suite with no `test result` line at all.
//!
//! So this type keeps the task in test builds, stops it on request with a
//! *proof of completion* (the future is dropped, not merely aborted) and re-arms
//! the guard, which is what a fixture that owns the script queue needs.

use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct GlobalLoop {
    started: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    task: std::sync::Mutex<Option<LiveTask>>,
}

impl GlobalLoop {
    pub(crate) const fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-support"))]
            task: std::sync::Mutex::new(None),
        }
    }

    /// Start the loop unless this process already did: one Agent stream per
    /// process, which is the production rule the `started` flag exists for.
    pub(crate) fn start(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        self.spawn(future);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl GlobalLoop {
    fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        let (done, finished) = std::sync::mpsc::channel();
        // Own the completion signal before the first poll: building it *inside*
        // the async block leaves an aborted-before-first-poll task holding
        // nothing to drop, and the waiter then reads `Disconnected` instead of
        // "the future is gone" — measured in a full-suite run.
        let completion = SignalOnDrop(done);
        let handle = crate::runtime::spawn(async move {
            let _completion = completion;
            future.await;
        });
        let previous = self
            .task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(LiveTask { handle, finished });
        if let Some(previous) = previous {
            previous.stop();
        }
    }

    /// Stop the loop, wait until its future is dropped, and re-arm the guard.
    ///
    /// Nothing process-global may outlive the fixture that started it: any RPC
    /// this future issues after the next test began lands in that test's script
    /// queue. Stopping is also what makes the once-only rule testable again.
    pub(crate) fn stop_for_test(&self) {
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.started.store(false, Ordering::Release);
        if let Some(task) = task {
            task.stop();
        }
    }

    pub(crate) fn is_running_for_test(&self) -> bool {
        self.task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

#[cfg(not(any(test, feature = "test-support")))]
impl GlobalLoop {
    fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        crate::runtime::spawn(future);
    }
}

/// The one live task, and the signal that its future is gone.
#[cfg(any(test, feature = "test-support"))]
struct LiveTask {
    handle: crate::runtime::TaskHandle,
    finished: std::sync::mpsc::Receiver<()>,
}

#[cfg(any(test, feature = "test-support"))]
impl LiveTask {
    fn stop(self) {
        self.handle.abort();
        // An aborted task still has to be driven to an await point before its
        // future drops, and "aborted" is not the fact the caller needs: it is
        // that no further RPC of this future can land in the next test's
        // script queue.
        match self
            .finished
            .recv_timeout(std::time::Duration::from_secs(5))
        {
            Ok(()) => {}
            // A dropped sender without a send means the future — and with it
            // the signal — is already gone, which is the same fact.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("a process-global agent loop did not stop before the next test scripted")
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
struct SignalOnDrop(std::sync::mpsc::Sender<()>);

#[cfg(any(test, feature = "test-support"))]
impl Drop for SignalOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stopping a loop whose task never ran must return, not wait: an abort can
    /// land between `spawn` and the task's first poll, and the future then drops
    /// without having built its signal. A full-suite run hit exactly that (the
    /// waiter read `Disconnected` and panicked), so a dropped signal counts as
    /// completion.
    #[test]
    fn stopping_a_loop_that_never_polled_returns() {
        let (done, finished) = std::sync::mpsc::channel();
        drop(done);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a private runtime");
        let handle = runtime.spawn(std::future::pending::<()>());
        LiveTask { handle, finished }.stop();
    }

    #[test]
    fn the_guard_is_rearmed_by_stopping() {
        let loop_ = GlobalLoop::new();
        loop_.start(std::future::pending::<()>());
        assert!(loop_.is_running_for_test());
        loop_.stop_for_test();
        assert!(!loop_.is_running_for_test());
        // Re-armed: a second start really starts again.
        loop_.start(std::future::pending::<()>());
        assert!(loop_.is_running_for_test());
        loop_.stop_for_test();
    }
}
