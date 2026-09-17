use super::*;

pub fn status() -> RemoteStatus {
    match SUPERVISOR.state.lock().unwrap().as_ref() {
        Some(s) => {
            // Derive real health instead of reporting `connected: true` for as
            // long as SUPERVISOR.state is occupied: the NATS client reconnects with state
            // transitions, and every critical background task can die
            // independently. Any finished task requires a full generation
            // reconnect; otherwise the bridge can look connected while losing
            // commands, events, presence, transfers, or credential refreshes.
            let critical_task_dead = s.cmd_task.is_finished()
                || s.transfer_task.is_finished()
                || s.event_task.is_finished()
                || s.heartbeat_task.is_finished()
                || s.refresh_task.is_finished();
            let terminal_service_error = s.nats_health.is_terminal();
            let generation_unhealthy = critical_task_dead || s.nats_health.needs_reconnect();
            let nats_reconnecting =
                s.client.connection_state() != async_nats::connection::State::Connected;
            let reconnect_in_flight = SUPERVISOR.runtime_reconnect_running.load(Ordering::Acquire);
            let can_auto_reconnect = SUPERVISOR
                .runtime_reconnect_attempts
                .load(Ordering::Acquire)
                < MAX_RUNTIME_RECONNECT_ATTEMPTS;
            let reconnecting = (generation_unhealthy || nats_reconnecting)
                && !terminal_service_error
                && SUPERVISOR.start_requested.load(Ordering::Acquire)
                && (!generation_unhealthy || reconnect_in_flight || can_auto_reconnect);
            let connected = !generation_unhealthy && !terminal_service_error && !nats_reconnecting;

            // The local Web listener is test-only and retries independently so
            // a busy port never disrupts the healthy phone bridge.
            let web_enabled = web_client_enabled();
            let web_dead = web_enabled && s.web_task.as_ref().is_none_or(|task| task.is_finished());
            if web_enabled && !web_dead {
                SUPERVISOR
                    .web_reconnect_attempts
                    .store(0, Ordering::Release);
            }
            let web_reconnect_in_flight =
                web_enabled && SUPERVISOR.web_reconnect_running.load(Ordering::Acquire);
            let can_reconnect_web = web_enabled
                && SUPERVISOR.web_reconnect_attempts.load(Ordering::Acquire)
                    < MAX_WEB_RECONNECT_ATTEMPTS;
            if web_dead && can_reconnect_web && !web_reconnect_in_flight {
                spawn_web_reconnect(s.pair_id.clone());
            }
            let web_reconnect_exhausted =
                web_dead && !web_reconnect_in_flight && !can_reconnect_web;
            // Re-expose the pairing code until it expires so the UI keeps it
            // after navigating away and back (it's no longer a show-once value).
            let confirmed = s.pairing_confirmed.load(Ordering::Acquire);
            let code_fresh = !confirmed
                && s.pairing_code.is_some()
                && s.pairing_code_expires_at
                    .is_some_and(|exp| exp > unix_timestamp() as i64);
            let (pairing_code, pairing_code_expires_at) = if code_fresh {
                (s.pairing_code.clone(), s.pairing_code_expires_at)
            } else {
                (None, None)
            };
            RemoteStatus {
                phase: if terminal_service_error {
                    RemotePhase::Failed
                } else if SUPERVISOR.credential_refreshing.load(Ordering::Acquire) {
                    RemotePhase::Refreshing
                } else if connected {
                    RemotePhase::Ready
                } else if reconnecting {
                    RemotePhase::Reconnecting
                } else {
                    RemotePhase::Failed
                },
                reason: if terminal_service_error {
                    Some(RemoteFailureReason::ServiceAuthorization)
                } else if SUPERVISOR.credential_refreshing.load(Ordering::Acquire) {
                    Some(RemoteFailureReason::CredentialExpired)
                } else if generation_unhealthy {
                    Some(RemoteFailureReason::GenerationUnhealthy)
                } else if nats_reconnecting {
                    Some(RemoteFailureReason::Network)
                } else {
                    None
                },
                recovery: reconnecting.then(|| RecoveryProgress {
                    attempt: if generation_unhealthy {
                        SUPERVISOR
                            .runtime_reconnect_attempts
                            .load(Ordering::Acquire) as u64
                    } else {
                        SUPERVISOR.start_retry_attempts.load(Ordering::Acquire)
                    },
                    max_attempts: generation_unhealthy
                        .then_some(MAX_RUNTIME_RECONNECT_ATTEMPTS as u64),
                    since: if generation_unhealthy {
                        SUPERVISOR
                            .runtime_failure_window_started
                            .load(Ordering::Acquire)
                    } else {
                        SUPERVISOR.start_retry_since.load(Ordering::Acquire)
                    },
                    next_retry_at: match SUPERVISOR.start_retry_next_at.load(Ordering::Acquire) {
                        0 => None,
                        value => Some(value),
                    },
                }),
                nats_url: s.nats_url.clone(),
                pair_id: s.pair_id.clone(),
                pairing_code,
                pairing_code_expires_at,
                desktop_id: s.desktop_id.clone(),
                desktop_public_key: s.desktop_public_key.clone(),
                web_url: s.web_url.clone(),
                web_lan_url: s.web_lan_url.clone(),
                agent_available: host().agent_available(),
                warning_code: web_reconnect_exhausted.then(|| "web_bind".to_string()),
            }
        }
        // A bridge that stopped on its own (revoked pairing) explains itself
        // through the last recorded error code instead of a bare "not running".
        // When stopped, surface the persisted pair_id so the frontend can still
        // show the paired row (disconnected state) — the authoritative pairing
        // fact is the persisted credential, not the runtime SUPERVISOR.state.
        None => {
            let error_code = SUPERVISOR.last_error_code.lock().unwrap().clone();
            // Startup retries run before a bridge instance exists, so this
            // state cannot be inferred from `SUPERVISOR.state`. Expose it explicitly so
            // the UI shows an amber reconnecting indicator instead of briefly
            // presenting the initial transient network/server error as final.
            let retrying_start = SUPERVISOR.start_requested.load(Ordering::Acquire)
                && SUPERVISOR.start_retry_running.load(Ordering::Acquire)
                && matches!(error_code.as_deref(), Some("network") | Some("server"));
            let resuming_from_sleep = SUPERVISOR.start_requested.load(Ordering::Acquire)
                && SUPERVISOR.resume_recovery_running.load(Ordering::Acquire)
                && matches!(error_code.as_deref(), Some("system_sleep"));
            let reconnecting = retrying_start || resuming_from_sleep;
            let (phase, reason) = if reconnecting {
                (
                    RemotePhase::Reconnecting,
                    Some(match error_code.as_deref() {
                        Some("server") => RemoteFailureReason::RemoteServer,
                        Some("system_sleep") => RemoteFailureReason::SystemSleep,
                        _ => RemoteFailureReason::Network,
                    }),
                )
            } else {
                match error_code.as_deref() {
                    Some("revoked") => (
                        RemotePhase::Revoked,
                        Some(RemoteFailureReason::CredentialRevoked),
                    ),
                    Some("account_authorization") => (
                        RemotePhase::Failed,
                        Some(RemoteFailureReason::AccountAuthorization),
                    ),
                    Some("service_authorization" | "service_config") => (
                        RemotePhase::Failed,
                        Some(RemoteFailureReason::ServiceAuthorization),
                    ),
                    Some("reconnect_required") => (
                        RemotePhase::Failed,
                        Some(RemoteFailureReason::GenerationUnhealthy),
                    ),
                    Some("system_sleep") => {
                        (RemotePhase::Failed, Some(RemoteFailureReason::SystemSleep))
                    }
                    Some("connecting") => (RemotePhase::Connecting, None),
                    Some("protocol") => (RemotePhase::Failed, Some(RemoteFailureReason::Protocol)),
                    Some("network") => (
                        RemotePhase::Reconnecting,
                        Some(RemoteFailureReason::Network),
                    ),
                    Some("server") => (
                        RemotePhase::Reconnecting,
                        Some(RemoteFailureReason::RemoteServer),
                    ),
                    Some(_) => (RemotePhase::Failed, Some(RemoteFailureReason::Local)),
                    None => (RemotePhase::Stopped, None),
                }
            };
            RemoteStatus {
                phase,
                reason,
                recovery: reconnecting.then(|| RecoveryProgress {
                    attempt: SUPERVISOR.start_retry_attempts.load(Ordering::Acquire),
                    max_attempts: None,
                    since: SUPERVISOR.start_retry_since.load(Ordering::Acquire),
                    next_retry_at: match SUPERVISOR.start_retry_next_at.load(Ordering::Acquire) {
                        0 => None,
                        value => Some(value),
                    },
                }),
                pair_id: pairing::load_creds().map(|c| c.pair_id).unwrap_or_default(),
                ..empty()
            }
        }
    }
}
