//! Interruptible waits and the stream-retry budget.
//!
//! Both waits poll the shared interrupt flag every 50ms, so an `abort()` that
//! only sets the flag — without a channel send — is still caught promptly.

use std::time::Duration;
use tokio::time::sleep;

use super::super::Loop;

// Separate from HTTP request retries: five additional attempts per model step.
pub(super) const MAX_STREAM_RETRIES: usize = 5;

impl Loop {
    /// Sleep for `dur`, returning early with `true` if an interrupt arrives —
    /// either signalled on `interrupt_rx` or via the shared interrupt flag.
    /// Returns `false` if the full duration elapsed without interruption.
    ///
    /// The flag is polled every 50ms (matching the shell tool) so an `abort()`
    /// that only sets the flag — without a channel send — is still caught
    /// promptly.
    pub(super) async fn sleep_or_interrupt(
        &self,
        dur: Duration,
        mut interrupt_rx: Option<&mut tokio::sync::mpsc::Receiver<()>>,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + dur;
        let poll = Duration::from_millis(50);
        loop {
            if self.is_interrupted() {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let step = poll.min(deadline - now);
            match interrupt_rx {
                Some(ref mut rx) => {
                    tokio::select! {
                        _ = sleep(step) => {}
                        _ = rx.recv() => return true,
                    }
                }
                None => sleep(step).await,
            }
        }
    }

    /// Await `fut`, bailing out early with `None` if an interrupt arrives on
    /// `interrupt_rx` or via the shared interrupt flag; returns `Some(output)`
    /// if the future completed first. The flag is polled every 50ms as a
    /// fallback for aborts that only set the flag without a channel send.
    pub(super) async fn await_or_interrupt<F, T>(
        &self,
        fut: F,
        mut interrupt_rx: Option<&mut tokio::sync::mpsc::Receiver<()>>,
    ) -> Option<T>
    where
        F: std::future::Future<Output = T>,
    {
        tokio::pin!(fut);
        let poll = Duration::from_millis(50);
        loop {
            if self.is_interrupted() {
                return None;
            }
            match interrupt_rx {
                Some(ref mut rx) => {
                    tokio::select! {
                        out = &mut fut => return Some(out),
                        _ = rx.recv() => return None,
                        _ = sleep(poll) => {}
                    }
                }
                None => {
                    tokio::select! {
                        out = &mut fut => return Some(out),
                        _ = sleep(poll) => {}
                    }
                }
            }
        }
    }
}

/// Returns true when the LLM error was caused by request body exceeding
/// the provider's limit (context window or proxy body-size cap).  The LLM
/// client annotates these errors with a `[CTX_LIMIT]` prefix so we can
/// detect them reliably without fragile keyword matching.
pub(super) fn is_retryable_size_error(err_msg: &str) -> bool {
    err_msg.starts_with("[CTX_LIMIT]")
}
