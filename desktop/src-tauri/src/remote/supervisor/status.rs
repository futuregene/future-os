use super::*;

/// One device's health, derived from its own runtime.
///
/// Deliberately per slot: a phone whose bridge is rebuilding must not make the
/// desktop client on the same machine look broken, and vice versa. The single
/// reason code this returns is the *first* thing wrong with that device, chosen
/// in the order a user would want to hear it.
fn slot_health(state: &RemoteState) -> (RemotePhase, Option<RemoteFailureReason>) {
    // Derive real health instead of reporting `connected: true` for as long as
    // the map entry exists: the NATS client reconnects with state transitions,
    // and every critical background task can die independently. Any finished
    // task requires a generation reconnect; otherwise the bridge can look
    // connected while losing commands, events, presence, transfers, or
    // credential refreshes.
    let critical_task_dead = state.cmd_task.is_finished()
        || state.transfer_task.is_finished()
        || state.event_task.is_finished()
        || state.heartbeat_task.is_finished()
        || state.refresh_task.is_finished();
    let terminal_service_error = state.nats_health.is_terminal();
    let generation_unhealthy = critical_task_dead || state.nats_health.needs_reconnect();
    let nats_reconnecting =
        state.client.connection_state() != async_nats::connection::State::Connected;
    if terminal_service_error {
        return (
            RemotePhase::Failed,
            Some(RemoteFailureReason::ServiceAuthorization),
        );
    }
    if SUPERVISOR.credential_refreshing.load(Ordering::Acquire) {
        return (
            RemotePhase::Refreshing,
            Some(RemoteFailureReason::CredentialExpired),
        );
    }
    // The reconnect budget is what turns "reconnecting" into an answerable
    // failure. Reporting Reconnecting forever would describe a machine that is
    // retrying something it has already given up on.
    let requested = SUPERVISOR.start_requested.load(Ordering::Acquire);
    if generation_unhealthy {
        let can_reconnect = SUPERVISOR
            .runtime_reconnect_attempts
            .load(Ordering::Acquire)
            < MAX_RUNTIME_RECONNECT_ATTEMPTS;
        let phase = if requested && can_reconnect {
            RemotePhase::Reconnecting
        } else {
            RemotePhase::Failed
        };
        return (phase, Some(RemoteFailureReason::GenerationUnhealthy));
    }
    if nats_reconnecting {
        let phase = if requested {
            RemotePhase::Reconnecting
        } else {
            RemotePhase::Failed
        };
        return (phase, Some(RemoteFailureReason::Network));
    }
    (RemotePhase::Ready, None)
}

fn connected(state: &RemoteState) -> bool {
    state.client.connection_state() == async_nats::connection::State::Connected
        && !state.nats_health.is_terminal()
        && !state.cmd_task.is_finished()
}

/// The invitation of `pair_id`, if it is still worth showing.
///
/// Re-exposed until it expires so the UI keeps it after navigating away and
/// back (it is no longer a show-once value), and dropped the moment the device
/// claims it or the code lapses.
fn pending_invitation(state: Option<&RemoteState>, pair_id: &str) -> (Option<String>, Option<i64>) {
    let Some(state) = state.filter(|state| state.pair_id == pair_id) else {
        return (None, None);
    };
    let confirmed = state.pairing_confirmed.load(Ordering::Acquire);
    let fresh = !confirmed
        && state.pairing_code.is_some()
        && state
            .pairing_code_expires_at
            .is_some_and(|exp| exp > unix_timestamp() as i64);
    if fresh {
        (state.pairing_code.clone(), state.pairing_code_expires_at)
    } else {
        (None, None)
    }
}

fn device_row(
    pair_id: &str,
    desktop_id: String,
    state: Option<&RemoteState>,
    active: Option<&str>,
) -> RemoteDevice {
    let control = if active == Some(pair_id) {
        DeviceControl::Active
    } else {
        DeviceControl::Standby
    };
    let (phase, _) = match state {
        Some(state) => slot_health(state),
        None => (RemotePhase::Stopped, None),
    };
    let is_connected = state.is_some_and(connected);
    let client = SUPERVISOR
        .client_identity(pair_id)
        .and_then(|slot| slot.lock().unwrap().clone());
    let (pairing_code, pairing_code_expires_at) = pending_invitation(state, pair_id);
    RemoteDevice {
        pair_id: pair_id.to_string(),
        phase,
        control,
        // A device that has never been claimed is not "connected"; it is an
        // invitation on screen. Reporting otherwise would tell the user a
        // device is in the room.
        connected: is_connected && pairing_code.is_none(),
        client,
        pairing_code,
        pairing_code_expires_at,
        desktop_id,
    }
}

pub fn status() -> RemoteStatus {
    status_focused(None)
}

/// The machine's status, with `prefer` chosen as the device the answer is
/// *about*.
///
/// A start that mints an invitation, and an add-a-device, both need their
/// answer to describe the device they just created: that is the invitation
/// whose QR code is on screen. Leaving the choice to the control pointer made
/// both report some other device whenever the process also held one — and the
/// host-bridge fixture, which pairs `status().pair_id`, then pointed a client
/// at a pairing the machine was not serving.
pub(in crate::remote) fn status_focused(prefer: Option<&str>) -> RemoteStatus {
    let active = SUPERVISOR.active_pair();
    let guard = SUPERVISOR.state.lock().unwrap();
    if guard.is_empty() {
        // Nothing is running: the truth is whatever stopped it. `last_error_code`
        // is the only record of why, and it is deliberately process-lifetime.
        drop(guard);
        return stopped_status(prefer, true);
    }

    // Every paired device, in pairing order, plus any runtime that holds an
    // invitation not yet claimed — that device is not in the book until the
    // handshake confirms it, and the user needs to see its QR code meanwhile.
    let mut devices: Vec<RemoteDevice> = Vec::new();
    for creds in pairing::load_slots() {
        devices.push(device_row(
            &creds.pair_id,
            creds.desktop_id.clone(),
            guard.get(&creds.pair_id),
            active.as_deref(),
        ));
    }
    for (pair_id, state) in guard.iter() {
        if devices.iter().any(|device| &device.pair_id == pair_id) {
            continue;
        }
        devices.push(device_row(
            pair_id,
            state.desktop_id.clone(),
            Some(state),
            active.as_deref(),
        ));
    }
    // The controlling device leads: it is what every single-device field below
    // projects, and what the page shows first.
    devices.sort_by_key(|device| device.control != DeviceControl::Active);

    let focus = match prefer
        .and_then(|pair_id| devices.iter().find(|device| device.pair_id == pair_id))
        .or_else(|| {
            devices
                .iter()
                .find(|device| device.control == DeviceControl::Active)
        })
        .or_else(|| devices.first())
    {
        Some(focus) => focus.clone(),
        // `guard` is non-empty here, so a device exists and this is unreachable
        // in practice; the flag keeps the projection honest if it ever is not.
        None => return stopped_status(prefer, false),
    };
    let runtime = guard.get(&focus.pair_id);

    // A running generation can still be reported as reconnecting; the
    // supervisor's recovery budgets are process-wide, so they are projected
    // onto whichever device is being described. Which budget it is depends on
    // why: a dead generation spends the crash-loop budget, while a first
    // connect / network recovery spends the startup-retry one.
    let reason = runtime.and_then(|state| slot_health(state).1);
    let reconnecting = matches!(
        focus.phase,
        RemotePhase::Reconnecting | RemotePhase::Connecting
    );
    let generation_unhealthy = matches!(reason, Some(RemoteFailureReason::GenerationUnhealthy));
    let recovery = reconnecting.then(|| RecoveryProgress {
        attempt: if generation_unhealthy {
            SUPERVISOR
                .runtime_reconnect_attempts
                .load(Ordering::Acquire) as u64
        } else {
            SUPERVISOR.start_retry_attempts.load(Ordering::Acquire)
        },
        max_attempts: generation_unhealthy.then_some(MAX_RUNTIME_RECONNECT_ATTEMPTS as u64),
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
    });

    RemoteStatus {
        client: focus.client.clone(),
        devices,
        phase: focus.phase,
        reason,
        recovery,
        nats_url: runtime
            .map(|state| state.nats_url.clone())
            .unwrap_or_default(),
        pair_id: focus.pair_id.clone(),
        pairing_code: focus.pairing_code.clone(),
        pairing_code_expires_at: focus.pairing_code_expires_at,
        desktop_id: focus.desktop_id.clone(),
        desktop_public_key: runtime
            .map(|state| state.desktop_public_key.clone())
            .unwrap_or_default(),
        web_url: runtime.and_then(|state| state.web_url.clone()),
        web_lan_url: runtime.and_then(|state| state.web_lan_url.clone()),
        agent_available: host().agent_available(),
        warning_code: web_warning(runtime),
    }
}

/// The local browser listener is one listener for the whole process, owned by
/// one slot; a dead one is reported there and nowhere else.
fn web_warning(runtime: Option<&RemoteState>) -> Option<String> {
    let runtime = runtime?;
    if !web_client_enabled() {
        return None;
    }
    let dead = runtime
        .web_task
        .as_ref()
        .is_none_or(|task| task.is_finished());
    let can_retry =
        SUPERVISOR.web_reconnect_attempts.load(Ordering::Acquire) < MAX_WEB_RECONNECT_ATTEMPTS;
    let in_flight = SUPERVISOR.web_reconnect_running.load(Ordering::Acquire);
    if dead && !in_flight && !can_retry {
        return Some("web_bind".to_string());
    }
    if dead && can_retry && !in_flight {
        spawn_web_reconnect(runtime.pair_id.clone());
    }
    None
}

/// Nothing is running. A bridge that stopped on its own (revoked pairing)
/// explains itself through the last recorded error code instead of a bare
/// "not running".
///
/// `prefer` means the same thing as in [`status_focused`]: the caller asked
/// about one device (one it just created), and even with nothing running the
/// answer must be about that device rather than the persisted controller.
fn stopped_status(prefer: Option<&str>, no_runtime: bool) -> RemoteStatus {
    let error_code = SUPERVISOR.last_error_code.lock().unwrap().clone();
    // Startup retries run before a bridge instance exists, so this state cannot
    // be inferred from the runtime map. Expose it explicitly so the UI shows an
    // amber reconnecting indicator instead of briefly presenting the initial
    // transient network/server error as final.
    let retrying_start = SUPERVISOR.start_requested.load(Ordering::Acquire)
        && SUPERVISOR.start_retry_running.load(Ordering::Acquire)
        && matches!(error_code.as_deref(), Some("network") | Some("server"));
    let resuming_from_sleep = SUPERVISOR.start_requested.load(Ordering::Acquire)
        && SUPERVISOR.resume_recovery_running.load(Ordering::Acquire)
        && matches!(error_code.as_deref(), Some("system_sleep"));
    let reconnecting = retrying_start || resuming_from_sleep;
    // The switch is on and there is nothing to serve: every device was removed,
    // or none was ever added. Reporting `Stopped` here would flip the switch
    // back off under the user's hand, and there is no failure to report.
    let enabled_and_empty = SUPERVISOR.start_requested.load(Ordering::Acquire)
        && !reconnecting
        && error_code.is_none()
        && pairing::load_slots().is_empty()
        && no_runtime;
    let (phase, reason) = if enabled_and_empty {
        (RemotePhase::Enabled, None)
    } else if reconnecting {
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
            Some("system_sleep") => (RemotePhase::Failed, Some(RemoteFailureReason::SystemSleep)),
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
    // Even with nothing running, the paired devices are the authoritative fact:
    // the page must still list them, as offline rows.
    let active = SUPERVISOR.active_pair();
    let devices = pairing::load_slots()
        .into_iter()
        .map(|creds| {
            device_row(
                &creds.pair_id,
                creds.desktop_id.clone(),
                None,
                active.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    let preferred = prefer.and_then(|pair_id| {
        devices
            .iter()
            .find(|device| device.pair_id == pair_id)
            .map(|device| {
                (
                    device.pair_id.clone(),
                    device.pairing_code.clone(),
                    device.pairing_code_expires_at,
                )
            })
    });
    let (pair_id, pairing_code, pairing_code_expires_at) = preferred.unwrap_or_else(|| {
        (
            pairing::load_creds()
                .map(|creds| creds.pair_id)
                .unwrap_or_default(),
            None,
            None,
        )
    });
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
        devices,
        pair_id,
        pairing_code,
        pairing_code_expires_at,
        ..empty()
    }
}
