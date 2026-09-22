//! Reconnecting WebSocket plumbing.
//!
//! Long-lived sockets (a gateway, a socket-mode app) drop for ordinary reasons —
//! a network blip, a deploy on the platform side, an idle timeout. Every
//! provider needs the same answer: reconnect with exponential backoff plus
//! jitter so a fleet of bridges does not synchronize its retries, and never
//! reconnect after shutdown or a fatal handshake failure.

use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// A connected socket, TLS already negotiated.
pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Open a WebSocket, attaching extra headers (authorization, app id) to the
/// handshake.
pub async fn connect(url: &str, headers: &[(&str, &str)]) -> Result<Socket> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let mut request = url.into_client_request()?;
    for (name, value) in headers {
        request.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(name.as_bytes())?,
            tokio_tungstenite::tungstenite::http::HeaderValue::from_str(value)?,
        );
    }
    let (socket, _response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| anyhow!("websocket connect to {url} failed: {error}"))?;
    Ok(socket)
}

/// Exponential backoff with full jitter.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
    multiplier: u32,
    attempt: u32,
}

impl Backoff {
    pub fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            multiplier: 2,
            attempt: 0,
        }
    }

    /// A backoff suitable for a gateway reconnect (1s doubling to 60s).
    pub fn gateway() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(60))
    }

    /// Forget accumulated failures — call this after a connection stayed up
    /// long enough to count as healthy.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// The next wait, in `[0, ceiling]`, where `ceiling` doubles per failure.
    pub fn next_delay(&mut self) -> Duration {
        let ceiling = self
            .initial
            .saturating_mul(self.multiplier.saturating_pow(self.attempt.min(10)))
            .min(self.max);
        self.attempt = self.attempt.saturating_add(1);
        jitter(ceiling)
    }
}

/// Uniform in `[0, ceiling)`. A cheap xorshift seeded from the clock and a
/// process-wide counter: the goal is un-synchronized retries, not randomness.
fn jitter(ceiling: Duration) -> Duration {
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = ceiling.as_nanos().min(u64::MAX as u128) as u64;
    if nanos == 0 {
        return Duration::ZERO;
    }
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    let mut state = clock
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15);
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    Duration::from_nanos(state % nanos)
}

/// Run `attempt` forever, reconnecting with backoff until shutdown.
///
/// `attempt` resolves when its connection ends: `Ok(())` for a clean close,
/// `Err` for a failure. Either way the loop reconnects — only shutdown stops
/// it. A connection that stayed up for at least [`HEALTHY_AFTER`] resets the
/// backoff, so one good session does not leave the next retry sluggish.
pub async fn supervise<F, Fut>(
    name: &str,
    shutdown: &Notify,
    mut backoff: Backoff,
    mut attempt: F,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    const HEALTHY_AFTER: Duration = Duration::from_secs(60);
    loop {
        let started = Instant::now();
        match attempt().await {
            Ok(()) => tracing::info!(channel = name, "connection closed; reconnecting"),
            Err(error) => tracing::warn!(channel = name, %error, "connection failed; reconnecting"),
        }
        if started.elapsed() >= HEALTHY_AFTER {
            backoff.reset();
        }
        let delay = backoff.next_delay();
        tracing::debug!(channel = name, ?delay, "waiting before reconnect");
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = shutdown.notified() => {
                tracing::info!(channel = name, "reconnect loop stopped");
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_never_exceeds_its_ceiling() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(4));
        for _ in 0..40 {
            let delay = backoff.next_delay();
            assert!(delay <= Duration::from_secs(4), "{delay:?}");
        }
    }

    #[test]
    fn backoff_ceiling_grows_before_it_caps() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(10));
        // Early attempts stay inside the first ceiling, later ones inside the cap.
        for _ in 0..64 {
            assert!(backoff.next_delay() <= Duration::from_secs(10));
        }
    }

    #[test]
    fn reset_returns_to_the_first_ceiling() {
        let mut backoff = Backoff::new(Duration::from_millis(50), Duration::from_secs(5));
        for _ in 0..8 {
            backoff.next_delay();
        }
        backoff.reset();
        // After a reset the delay is again bounded by the initial ceiling.
        assert!(backoff.next_delay() <= Duration::from_millis(50));
    }

    #[test]
    fn jitter_is_bounded_and_varies() {
        let ceiling = Duration::from_millis(100);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..32 {
            let delay = jitter(ceiling);
            assert!(delay < ceiling || ceiling.is_zero());
            seen.insert(delay.as_nanos());
        }
        assert!(seen.len() > 1, "jitter should not be constant");
    }

    #[test]
    fn zero_ceiling_jitters_to_zero() {
        assert_eq!(jitter(Duration::ZERO), Duration::ZERO);
    }

    #[tokio::test]
    async fn supervise_stops_on_shutdown() {
        let shutdown = Notify::new();
        let notify = std::sync::Arc::new(shutdown);
        let handle = {
            let shutdown = notify.clone();
            tokio::spawn(async move {
                supervise(
                    "test",
                    &shutdown,
                    Backoff::new(Duration::from_millis(1), Duration::from_millis(5)),
                    || async { Err(anyhow!("boom")) },
                )
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        notify.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "supervise must stop after shutdown");
        assert!(result.unwrap().unwrap().is_ok());
    }
}
