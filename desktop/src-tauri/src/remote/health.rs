use super::*;

/// Generation-local health reported by async-nats. Keeping this beside the
/// client prevents a late callback from an old credential generation from
/// poisoning the currently active bridge.
#[derive(Default)]
pub(super) struct NatsHealth {
    pub(super) reconnect_required: AtomicBool,
    pub(super) service_config_error: AtomicBool,
    /// This belongs to one NATS generation, never the process. A late event
    /// from an old socket must not decide whether a newer JWT is terminal.
    pub(super) credential_was_refreshed: AtomicBool,
    pub(super) credential_expires_at: AtomicU64,
    pub(super) authorization_rejection_logged: AtomicBool,
    pub(super) event_episode: FailureEpisode,
}

impl NatsHealth {
    pub(super) fn handle_event(&self, event: &async_nats::Event) {
        use async_nats::{ClientError, Event, ServerError};
        match event {
            Event::Connected => {
                self.reconnect_required.store(false, Ordering::Release);
                self.authorization_rejection_logged
                    .store(false, Ordering::Release);
                if let Some(line) = self.event_episode.recovered() {
                    eprintln!("{line}");
                }
            }
            Event::Disconnected => {
                // Routine and self-healing: async-nats reconnects automatically,
                // so a drop is expected noise. A real outage still surfaces via
                // the ClientError/ServerError events fired on each failed
                // reconnect attempt, so don't open a failure episode here.
            }
            Event::ServerError(ServerError::AuthorizationViolation) => {
                self.mark_authorization_rejected();
            }
            // async-nats reports an authorization rejection that occurs while
            // reconnecting as `ClientError::Other`, not `ServerError`. Treat
            // this transient-client shape as a failed generation: the runtime
            // supervisor replaces it with a fresh short-lived bridge JWT.
            Event::ClientError(ClientError::Other(error)) if is_authorization_violation(error) => {
                self.mark_authorization_rejected();
            }
            Event::ClientError(ClientError::MaxReconnects) => {
                self.reconnect_required.store(true, Ordering::Release);
                if let Some(line) = self
                    .event_episode
                    .record("network", "NATS reconnect budget exhausted")
                {
                    eprintln!("{line}");
                }
            }
            Event::Closed => {
                self.reconnect_required.store(true, Ordering::Release);
                // Routine: the socket closed (credential-refresh swap, app
                // shutdown, or a terminal reconnect failure already surfaced via
                // ClientError/ServerError). A real outage still logs, so this
                // stays silent instead of opening a failure episode.
            }
            Event::SlowConsumer(subscription) => {
                if let Some(line) = self.event_episode.record("slow_consumer", subscription) {
                    eprintln!("{line}");
                }
            }
            Event::ServerError(ServerError::SlowConsumer(subscription)) => {
                if let Some(line) = self.event_episode.record("slow_consumer", subscription) {
                    eprintln!("{line}");
                }
            }
            Event::ServerError(error) => {
                if let Some(line) = self.event_episode.record("remote_server", error) {
                    eprintln!("{line}");
                }
            }
            Event::ClientError(error) => {
                if let Some(line) = self.event_episode.record("network", error) {
                    eprintln!("{line}");
                }
            }
            _ => {}
        }
    }

    pub(super) fn needs_reconnect(&self) -> bool {
        self.reconnect_required.load(Ordering::Acquire)
    }

    pub(super) fn is_terminal(&self) -> bool {
        self.service_config_error.load(Ordering::Acquire)
    }

    pub(super) fn mark_authorization_rejected(&self) {
        // The reconnect loop can deliver the same error indefinitely. One
        // state transition is enough to make the supervisor refresh the JWT;
        // suppress the duplicate events so a sleeping laptop cannot flood its
        // console while that handoff is in progress.
        if self.credential_was_refreshed.load(Ordering::Acquire)
            && self.credential_expires_at.load(Ordering::Acquire) > unix_timestamp()
        {
            self.service_config_error.store(true, Ordering::Release);
            if !self
                .authorization_rejection_logged
                .swap(true, Ordering::AcqRel)
            {
                eprintln!(
                    "remote: freshly refreshed NATS credential was rejected [AU001]; entering service_authorization"
                );
            }
            return;
        }
        self.reconnect_required.store(true, Ordering::Release);
        if !self
            .authorization_rejection_logged
            .swap(true, Ordering::AcqRel)
        {
            eprintln!("remote: NATS authorization rejected [AU002]; refreshing bridge credentials");
        }
    }
}

pub(super) fn is_authorization_violation(error: &str) -> bool {
    error.trim().eq_ignore_ascii_case("authorization violation")
}

pub(super) fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn reconnect_delay(attempt: usize, random: f64) -> std::time::Duration {
    let exponent = attempt.min(5) as u32;
    let base_secs = (1u64 << exponent).min(30);
    std::time::Duration::from_secs_f64((base_secs as f64 * (0.8 + random * 0.4)).min(30.0))
}
