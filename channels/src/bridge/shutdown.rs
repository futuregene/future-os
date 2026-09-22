//! The channel shutdown signal.
//!
//! A `Notify` is edge-triggered: it wakes whoever is parked at that instant and
//! stores nothing (or at most one permit) for everyone else. Channels need the
//! opposite. A provider typically has two waits in sequence — the session that
//! is streaming, then the supervisor's reconnect backoff — and the second one
//! registers *after* the first was woken by the signal. With a bare `Notify`
//! that second wait never completes, so a graceful shutdown hangs until the
//! process is killed (and tests time out).
//!
//! [`Shutdown`] keeps a flag alongside the notification, so a wait started after
//! the signal returns immediately. Providers only see the same shape they
//! already use (`shutdown().notified()`), so nothing else has to change.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// A level-triggered shutdown signal.
#[derive(Debug, Default)]
pub struct Shutdown {
    triggered: AtomicBool,
    notify: Notify,
}

/// The future returned by [`Shutdown::notified`].
///
/// A hand-written `Future` rather than an `async fn` because a channel may poll
/// it again after it has already completed (a `select!` that picked another
/// branch in the same iteration); an `async fn` panics when resumed after
/// completion, and this one simply reports readiness again.
pub struct Notified<'a> {
    shutdown: &'a Shutdown,
    inner: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>>,
}

impl std::future::Future for Notified<'_> {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let this = self.get_mut();
        if this.inner.is_none() {
            let shutdown = this.shutdown;
            this.inner = Some(Box::pin(async move {
                // Registration happens on the first poll of the inner future;
                // the flag is re-checked after pinning so a trigger that landed
                // in between is not missed (and `trigger` also leaves a permit).
                let parked = shutdown.notify.notified();
                tokio::pin!(parked);
                if shutdown.is_triggered() {
                    return;
                }
                parked.await;
            }));
        }
        let inner = this.inner.as_mut().expect("inner future");
        if inner.as_mut().poll(cx).is_pending() {
            return std::task::Poll::Pending;
        }
        // Clear only on readiness: a pending poll must keep the registration so
        // the notification can find this waiter. A later poll after readiness
        // builds a fresh inner future, whose own flag check reports readiness
        // again instead of panicking.
        this.inner = None;
        std::task::Poll::Ready(())
    }
}

impl Shutdown {
    /// A fresh signal.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Fire the signal. Safe to call more than once; later calls are no-ops for
    /// the flag and merely re-wake anyone parked on the notification.
    pub fn trigger(&self) {
        self.triggered.store(true, Ordering::SeqCst);
        // Wake everyone parked now, and leave a permit for the next waiter to
        // register (the supervisor that follows a woken session).
        self.notify.notify_waiters();
        self.notify.notify_one();
    }

    /// Whether the signal has fired. Use this for a cheap check in a loop.
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    /// Wait until the signal fires, returning immediately if it already has.
    ///
    /// Cancel-safe: dropping the returned future leaves no state behind, so it
    /// can be used directly in a `select!`.
    pub fn notified(&self) -> Notified<'_> {
        Notified {
            shutdown: self,
            inner: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_wait_before_the_signal_completes_when_it_fires() {
        let shutdown = Shutdown::new();
        let waiter = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move { shutdown.notified().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown.trigger();
        let done = tokio::time::timeout(Duration::from_secs(2), waiter).await;
        assert!(done.is_ok(), "a parked waiter must be woken");
    }

    #[tokio::test]
    async fn a_wait_started_after_the_signal_returns_immediately() {
        // This is the sequence a channel has: the session is woken first, then
        // the supervisor's reconnect backoff registers its own wait.
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let first = tokio::time::timeout(Duration::from_secs(2), shutdown.notified()).await;
        assert!(first.is_ok(), "the first waiter must return");
        let second = tokio::time::timeout(Duration::from_secs(2), shutdown.notified()).await;
        assert!(second.is_ok(), "a later waiter must return too");
        let third = tokio::time::timeout(Duration::from_secs(2), shutdown.notified()).await;
        assert!(third.is_ok(), "and so must every waiter after it");
        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn waiting_without_a_signal_keeps_waiting() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_triggered());
        let waited = tokio::time::timeout(Duration::from_millis(50), shutdown.notified()).await;
        assert!(waited.is_err(), "an unsignalled wait must not complete");
    }

    #[test]
    fn triggering_twice_is_harmless() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        shutdown.trigger();
        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn the_same_future_can_be_polled_again_after_it_completes() {
        // A channel's `select!` may poll the wait again in a later iteration
        // after it already completed (an `async fn` would panic here).
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let mut wait = shutdown.notified();
        assert!(matches!(poll_once(&mut wait), std::task::Poll::Ready(())));
        // Poll it a second time, which must also report readiness.
        assert!(matches!(poll_once(&mut wait), std::task::Poll::Ready(())));
    }

    /// Poll a future once through a no-op waker, so a test can observe the
    /// stalled/re-ready states `select!` produces.
    fn poll_once<F: std::future::Future + Unpin>(future: &mut F) -> std::task::Poll<F::Output> {
        let waker = std::task::Waker::noop().clone();
        let mut context = std::task::Context::from_waker(&waker);
        std::future::Future::poll(std::pin::Pin::new(future), &mut context)
    }

    #[tokio::test]
    async fn one_wait_does_not_consume_the_signal_for_the_next() {
        let shutdown = Shutdown::new();
        // A waiter that arrives, is woken, and a completely separate one later:
        // the flag is what makes the second one return.
        let first = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                shutdown.notified().await;
                // Do something that takes a moment before the next wait starts.
                tokio::time::sleep(Duration::from_millis(30)).await;
                shutdown.notified().await;
            })
        };
        shutdown.trigger();
        let done = tokio::time::timeout(Duration::from_secs(2), first).await;
        assert!(done.is_ok(), "the second wait in one task must also return");
    }
}
