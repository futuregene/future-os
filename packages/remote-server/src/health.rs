use crate::{diagnostics::FailureEpisode, supervisor::unix_timestamp};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

/// Consecutive failed mints that stop being "slow" and start being "stuck".
///
/// Three, because the renewal has 90 s of lead and a 15 s tick: three misses is
/// nearly a whole lead window spent without a replacement grant, and the reader
/// deserves to be told before the grant runs out.
pub const MINT_FAILURES_BEFORE_STUCK: u8 = 3;

/// Generation-local health reported by async-nats. Keeping this beside the
/// client prevents a late callback from an old credential generation from
/// poisoning the currently active bridge.
#[derive(Default)]
pub struct NatsHealth {
    pub reconnect_required: AtomicBool,
    pub service_config_error: AtomicBool,
    /// This belongs to one NATS generation, never the process. A late event
    /// from an old socket must not decide whether a newer JWT is terminal.
    pub credential_was_refreshed: AtomicBool,
    pub credential_expires_at: AtomicU64,
    pub authorization_rejection_logged: AtomicBool,
    pub event_episode: FailureEpisode,
    /// Whether this generation's inbound command lane is subscribed.
    ///
    /// A bridge with a live socket and no command lane answers nothing: the
    /// phone's requests reach a subject nobody listens on and time out behind a
    /// whole request deadline. The lane is therefore health, not a detail of the
    /// command loop, and [`Self::command_lane_down_since`] is what makes "why was
    /// this device unreachable for minutes" answerable.
    command_lane_down: AtomicBool,
    /// When the lane went down, unix milliseconds. 0 = it is up.
    command_lane_down_since: AtomicU64,
    /// Consecutive failed credential mints in this generation.
    ///
    /// Three in a row is where a renewal stops being merely slow: the user is
    /// told, rather than reading a generic "offline" while nothing renews.
    mint_failures: AtomicU8,
}

impl NatsHealth {
    /// The command lane subscribed: requests will be answered from here on.
    pub fn command_lane_up(&self) {
        self.command_lane_down_since.store(0, Ordering::Release);
        self.command_lane_down.store(false, Ordering::Release);
    }

    /// The command lane lost its subscription. Keeps the *first* moment it went
    /// down, so a retry ladder reports how long the device has been deaf rather
    /// than how long the current attempt has been running.
    pub fn command_lane_down(&self) {
        let _ = self.command_lane_down_since.compare_exchange(
            0,
            unix_millis(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.command_lane_down.store(true, Ordering::Release);
    }

    /// Whether the lane is down right now.
    pub fn command_lane_is_down(&self) -> bool {
        self.command_lane_down.load(Ordering::Acquire)
    }

    /// Since when the lane has been down, unix milliseconds.
    pub fn command_lane_down_since(&self) -> Option<u64> {
        let since = self.command_lane_down_since.load(Ordering::Acquire);
        (since > 0).then_some(since)
    }

    /// Count one failed mint; returns the new consecutive count.
    pub fn note_mint_failure(&self) -> u8 {
        let previous = self
            .mint_failures
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                Some(count.saturating_add(1))
            })
            .unwrap_or(u8::MAX);
        previous.saturating_add(1)
    }

    /// A mint succeeded: the consecutive count starts over.
    pub fn clear_mint_failures(&self) {
        self.mint_failures.store(0, Ordering::Release);
    }

    /// How many mints have failed in a row in this generation.
    pub fn mint_failures(&self) -> u8 {
        self.mint_failures.load(Ordering::Acquire)
    }

    pub fn handle_event(&self, event: &async_nats::Event) {
        use async_nats::{ClientError, Event, ServerError};
        match event {
            Event::Connected => {
                self.reconnect_required.store(false, Ordering::Release);
                self.authorization_rejection_logged
                    .store(false, Ordering::Release);
                if let Some(line) = self.event_episode.recovered() {
                    future_remote_protocol::remote_log!("{line}");
                }
            }
            // The server is retiring this connection: a relay rolling restart,
            // or a node scaled down behind the same address. `async-nats` can
            // migrate to another address in the pool, but this client holds one
            // and its reconnect budget is a single attempt, so in practice a
            // drain is followed by the socket going away.
            //
            // Acting here rather than waiting for the close is what keeps a
            // drain from costing a whole credential lifetime: the drain usually
            // arrives just after a rotation, so the grant already in hand is the
            // one the retiring node stopped honouring, and the replacement — a
            // new mint — is exactly what a rebuild produces. Silence here was
            // measured: two drains in one `desktop.log`, each followed fifteen
            // minutes later by `User Authentication Expired` with nobody left
            // watching to mint a replacement.
            Event::Draining | Event::LameDuckMode => {
                self.reconnect_required.store(true, Ordering::Release);
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
                note_link_issue();
                if let Some(line) = self
                    .event_episode
                    .record("network", "NATS reconnect budget exhausted")
                {
                    future_remote_protocol::remote_log!("{line}");
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
                    future_remote_protocol::remote_log!("{line}");
                }
            }
            Event::ServerError(ServerError::SlowConsumer(subscription)) => {
                if let Some(line) = self.event_episode.record("slow_consumer", subscription) {
                    future_remote_protocol::remote_log!("{line}");
                }
            }
            Event::ServerError(error) => {
                if let Some(line) = self.event_episode.record("remote_server", error) {
                    future_remote_protocol::remote_log!("{line}");
                }
            }
            Event::ClientError(error) => {
                if let Some(line) = self.event_episode.record("network", error) {
                    future_remote_protocol::remote_log!("{line}");
                }
            }
        }
    }

    pub fn needs_reconnect(&self) -> bool {
        self.reconnect_required.load(Ordering::Acquire)
    }

    pub fn is_terminal(&self) -> bool {
        self.service_config_error.load(Ordering::Acquire)
    }

    /// A credential this generation presented was refused.
    ///
    /// Both shapes end the *generation* — the supervisor rebuilds it from a
    /// fresh mint (`recovery_plan` → `Refresh`) instead of presenting the same
    /// grant again. What they separate is the reason shown while that happens:
    /// a refusal of a credential that was already renewed and still has life
    /// left (`AU001`) is a machine whose credentials the relay will not honour,
    /// so the status names `service_authorization`; everything else (`AU002`) is
    /// an expired or superseded grant, which the next mint replaces.
    ///
    /// Neither verdict may be read as "stop trying": the reconnect loop can
    /// deliver the same error indefinitely, so one state transition is enough
    /// per generation, but the generation itself is disposable by design.
    pub fn mark_authorization_rejected(&self) {
        note_link_issue();
        if self.credential_was_refreshed.load(Ordering::Acquire)
            && self.credential_expires_at.load(Ordering::Acquire) > unix_timestamp()
        {
            self.service_config_error.store(true, Ordering::Release);
            if !self
                .authorization_rejection_logged
                .swap(true, Ordering::AcqRel)
            {
                future_remote_protocol::remote_log!(
                    "remote: freshly refreshed NATS credential was rejected [AU001]; rebuilding the generation with a new mint"
                );
            }
            return;
        }
        self.reconnect_required.store(true, Ordering::Release);
        if !self
            .authorization_rejection_logged
            .swap(true, Ordering::AcqRel)
        {
            future_remote_protocol::remote_log!(
                "remote: NATS authorization rejected [AU002]; rebuilding the generation with a renewed credential"
            );
        }
    }
}

pub fn is_authorization_violation(error: &str) -> bool {
    error.trim().eq_ignore_ascii_case("authorization violation")
}

pub fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// When any generation of this process's relay link last looked unhealthy: an
/// authentication rejection, or an exhausted reconnect budget. Unix
/// milliseconds; 0 = never.
///
/// Process-global because the reader of this fact is usually not the generation
/// that failed — the status projection and a measurement of a slow send both ask
/// "was the link just failing?" after the generation that answers them may
/// already have been replaced.
static LAST_LINK_ISSUE_MS: AtomicU64 = AtomicU64::new(0);

/// Record that the link just failed in a way a reader would notice.
pub fn note_link_issue() {
    LAST_LINK_ISSUE_MS.store(unix_millis(), Ordering::Release);
}

/// How long ago the link last failed, or `None` if it never has.
pub fn link_issue_age() -> Option<std::time::Duration> {
    let at = LAST_LINK_ISSUE_MS.load(Ordering::Acquire);
    (at > 0).then(|| std::time::Duration::from_millis(unix_millis().saturating_sub(at)))
}

pub fn reconnect_delay(attempt: usize, random: f64) -> std::time::Duration {
    let exponent = attempt.min(5) as u32;
    let base_secs = (1u64 << exponent).min(30);
    std::time::Duration::from_secs_f64((base_secs as f64 * (0.8 + random * 0.4)).min(30.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn nats_health_handles_remaining_event_variants() {
        let health = NatsHealth::default();

        // A routine drop: self-healing reconnect, no failure episode.
        health.handle_event(&async_nats::Event::Disconnected);
        assert!(!health.needs_reconnect());

        // Slow-consumer and generic server errors each open an episode.
        health.handle_event(&async_nats::Event::SlowConsumer(7));
        health.handle_event(&async_nats::Event::ServerError(
            async_nats::ServerError::SlowConsumer(8),
        ));
        health.handle_event(&async_nats::Event::ServerError(
            async_nats::ServerError::Other("permissions".to_string()),
        ));

        // Both shapes of "this server is retiring the connection" end the
        // generation: a drain means the node that issued (or was honouring)
        // this grant is going away, so the bridge asks to be rebuilt on a fresh
        // mint instead of waiting for a close that may be fifteen minutes away.
        for event in [async_nats::Event::LameDuckMode, async_nats::Event::Draining] {
            let health = NatsHealth::default();
            health.handle_event(&event);
            assert!(
                health.needs_reconnect(),
                "{event:?} must ask for a replacement generation"
            );
            // A migration that does succeed (another address in the pool)
            // withdraws the request rather than spending a rebuild on it.
            health.handle_event(&async_nats::Event::Connected);
            assert!(!health.needs_reconnect(), "{event:?} migration recovers");
        }
    }
    #[test]
    fn nats_health_classifies_recoverable_and_terminal_events() {
        let health = NatsHealth::default();
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::MaxReconnects,
        ));
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());

        health.handle_event(&async_nats::Event::Connected);
        assert!(!health.needs_reconnect());

        // async-nats wraps reconnect-time authorization errors in the client
        // event rather than forwarding the server event. This must trigger a
        // generation refresh instead of leaving an apparently live bridge in
        // an unbounded reconnect/log loop.
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("authorization violation".to_string()),
        ));
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("authorization violation".to_string()),
        ));
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());
        assert!(health
            .authorization_rejection_logged
            .load(Ordering::Acquire));

        health.handle_event(&async_nats::Event::Connected);
        assert!(!health.needs_reconnect());
        assert!(!health
            .authorization_rejection_logged
            .load(Ordering::Acquire));

        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("IO error".to_string()),
        ));
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("IO error".to_string()),
        ));
        assert!(!health.needs_reconnect());

        health.handle_event(&async_nats::Event::Connected);

        health
            .credential_expires_at
            .store(unix_timestamp() + 3600, Ordering::Release);
        // The same rejection on the generation created by the reactive
        // refresh is terminal service authorization.
        health
            .credential_was_refreshed
            .store(true, Ordering::Release);
        health.handle_event(&async_nats::Event::ServerError(
            async_nats::ServerError::AuthorizationViolation,
        ));
        health.handle_event(&async_nats::Event::Connected);
        assert!(
            health.is_terminal(),
            "reconnect must not hide an auth failure"
        );
    }
    #[test]
    fn expired_rotated_credential_is_refreshable_not_a_configuration_failure() {
        let health = NatsHealth::default();
        health
            .credential_was_refreshed
            .store(true, Ordering::Release);
        health
            .credential_expires_at
            .store(unix_timestamp().saturating_sub(1), Ordering::Release);
        health.mark_authorization_rejected();
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());
    }
    #[test]
    fn desktop_reconnect_backoff_matches_the_shared_policy() {
        assert_eq!(reconnect_delay(0, 0.5), Duration::from_secs(1));
        assert_eq!(reconnect_delay(1, 0.5), Duration::from_secs(2));
        assert_eq!(reconnect_delay(2, 0.5), Duration::from_secs(4));
        assert_eq!(reconnect_delay(4, 0.5), Duration::from_secs(16));
        assert_eq!(reconnect_delay(5, 0.5), Duration::from_secs(30));
        assert!(reconnect_delay(20, 1.0) <= Duration::from_secs(30));
    }
}
