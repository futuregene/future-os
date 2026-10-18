use super::*;

pub async fn unpair() -> Result<RemoteStatus, crate::AppError> {
    let notices = mobile_unpair_notices();
    let result = stop_with(|| {
        for creds in pairing::load_slots() {
            pairing::queue_revoke(&creds)?;
        }
        pairing::clear_creds()
    });
    for (client, subject, payload) in notices {
        crate::runtime::spawn(send_mobile_disconnect_notice(client, subject, payload));
    }
    // Stop is already effective even if local persistence fails. Keep the
    // credential on disk when queueing fails so a user retry can still revoke.
    result?;
    transfer::clear_preview_cache();
    spawn_revoke_cleanup();
    Ok(empty())
}

/// Unpair one device and leave the others paired.
///
/// When the device being removed is the one in control, the pointer is cleared
/// rather than handed to another device: choosing a new controller is the
/// user's decision, and until they make it nobody commands this machine.
pub async fn unpair_slot(pair_id: &str) -> Result<RemoteStatus, crate::AppError> {
    let notice = mobile_disconnect_notice_for(pair_id, "unpaired");
    let creds = pairing::load_slots()
        .into_iter()
        .find(|slot| slot.pair_id == pair_id);
    let result = (|| -> Result<(), crate::AppError> {
        if let Some(creds) = &creds {
            pairing::queue_revoke(creds)?;
        }
        pairing::remove_slot(pair_id)?;
        Ok(())
    })();
    let released = SUPERVISOR
        .active
        .lock()
        .unwrap()
        .as_deref()
        .is_some_and(|active| active == pair_id);
    if released {
        SUPERVISOR.release_control(pair_id);
    }
    stop_slot(pair_id);
    super::promote_a_lone_device();
    if let Some((client, subject, payload)) = notice {
        crate::runtime::spawn(send_mobile_disconnect_notice(client, subject, payload));
    }
    result?;
    spawn_revoke_cleanup();
    Ok(status())
}

#[cfg(test)]
pub(in crate::remote) async fn notify_mobile_unpair() {
    for (client, subject, payload) in mobile_unpair_notices() {
        send_mobile_disconnect_notice(client, subject, payload).await;
    }
}

pub(in crate::remote) fn mobile_unpair_notices() -> Vec<(async_nats::Client, String, Vec<u8>)> {
    let guard = SUPERVISOR.state.lock().unwrap();
    guard
        .values()
        .filter_map(|state| {
            let payload = serde_json::to_vec(&json!({
                "online": false, "unpaired": true, "pairId": state.pair_id,
                "bridgeInstanceId": state.bridge_instance_id, "lastHeartbeatTs": unix_timestamp(),
            }))
            .ok()?;
            let subject = format!("p.{}.presence", state.pair_id);
            let payload = state.security.seal(&subject, &payload).ok()??;
            Some((state.client.clone(), subject, payload))
        })
        .collect()
}

/// A notice for one device, sealed on that device's own channel.
pub(in crate::remote) fn mobile_disconnect_notice_for(
    pair_id: &str,
    reason: &str,
) -> Option<(async_nats::Client, String, Vec<u8>)> {
    let guard = SUPERVISOR.state.lock().unwrap();
    let state = guard.get(pair_id)?;
    let subject = format!("p.{pair_id}.presence");
    let plaintext = serde_json::to_vec(&json!({
        "online": false,
        "disconnected": true,
        "unpaired": reason == "unpaired",
        "reason": reason,
        "pairId": pair_id,
        "bridgeInstanceId": state.bridge_instance_id,
        "lastHeartbeatTs": unix_timestamp(),
    }))
    .ok()?;
    let payload = state.security.seal(&subject, &plaintext).ok()??;
    Some((state.client.clone(), subject, payload))
}

pub fn stop() -> RemoteStatus {
    stop_with(|| ());
    empty()
}

pub(in crate::remote) fn stop_with<T>(cleanup: impl FnOnce() -> T) -> T {
    SUPERVISOR.access.invalidate(|| {
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        SUPERVISOR.suspended.store(false, Ordering::Release);
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        SUPERVISOR
            .runtime_failure_window_started
            .store(0, Ordering::Release);
        SUPERVISOR
            .web_reconnect_attempts
            .store(0, Ordering::Release);
        SUPERVISOR.cancel_tasks();
        stop_runtime();
        disable_handshake();
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
        // A stopped machine serves no device, so it holds no device set and no
        // controller. The book on disk is untouched and is read again on the
        // next start; keeping the in-memory answers here would describe a
        // machine that is not running.
        SUPERVISOR.reset_devices();
        cleanup()
    })
}

pub(in crate::remote) fn disable_handshake() {
    let guard = SUPERVISOR.bridge_shared.lock().unwrap();
    for shared in guard.values() {
        let mut handshake = shared.handshake.lock().unwrap();
        if let Some(state) = handshake.take() {
            state.active_flag().store(false, Ordering::Release);
            state.secure.clear();
        }
    }
}

/// Notify an online mobile client before an intentional desktop disconnect.
/// The mobile client treats this as immediate offline state; heartbeat expiry
/// remains the fallback for crashes, forced power-off, and any lost packet.
pub async fn stop_gracefully(reason: &str) -> RemoteStatus {
    // Cancelling the bridge must not wait on a broker that is already down or
    // reconnecting. Capture the best-effort presence notification while the
    // generation still owns its client, then stop synchronously so it also
    // cancels any automatic reconnect work immediately.
    let notices = mobile_disconnect_notices(reason);
    let status = stop();
    for (client, subject, payload) in notices {
        if crate::desktop_shutdown::is_stopping() {
            // Registration is frozen at process exit. Still deliver the bounded
            // presence notice before the cleanup runtime goes away.
            send_mobile_disconnect_notice(client, subject, payload).await;
        } else {
            crate::runtime::spawn(async move {
                send_mobile_disconnect_notice(client, subject, payload).await;
            });
        }
    }
    status
}

pub async fn notify_mobile_disconnect(reason: &str) {
    for (client, subject, payload) in mobile_disconnect_notices(reason) {
        send_mobile_disconnect_notice(client, subject, payload).await;
    }
}

/// A notice for every connected device, each sealed on its own channel.
pub(in crate::remote) fn mobile_disconnect_notices(
    reason: &str,
) -> Vec<(async_nats::Client, String, Vec<u8>)> {
    let pair_ids: Vec<String> = SUPERVISOR.state.lock().unwrap().keys().cloned().collect();
    pair_ids
        .into_iter()
        .filter_map(|pair_id| mobile_disconnect_notice(&pair_id, reason))
        .collect()
}

pub(in crate::remote) fn mobile_disconnect_notice(
    pair_id: &str,
    reason: &str,
) -> Option<(async_nats::Client, String, Vec<u8>)> {
    SUPERVISOR
        .state
        .lock()
        .unwrap()
        .get(pair_id)
        .and_then(|state| {
            let bridge_instance_id = state.bridge_instance_id.clone();
            let subject = format!("p.{pair_id}.presence");
            let plaintext = serde_json::to_vec(&json!({
                "online": false,
                "disconnected": true,
                "reason": reason,
                "pairId": pair_id,
                "bridgeInstanceId": bridge_instance_id,
                "lastHeartbeatTs": unix_timestamp(),
            }))
            .ok()?;
            let payload = state.security.seal(&subject, &plaintext).ok()??;
            Some((state.client.clone(), subject, payload))
        })
}

pub(in crate::remote) async fn send_mobile_disconnect_notice(
    client: async_nats::Client,
    subject: String,
    payload: Vec<u8>,
) {
    let send = async {
        // Flushing after a failed publish is a no-op (nothing was queued), but
        // keeping it unconditional avoids a conditionally-executed flush arm.
        let _ = client.publish(subject, payload.into()).await;
        let _ = client.flush().await;
    };
    let _ = tokio::time::timeout(DISCONNECT_NOTICE_TIMEOUT, send).await;
}

/// Platform adapters report only these facts. Keeping their policy pure makes
/// suspend/resume behavior testable without macOS/Windows/Linux notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::remote) enum PowerEvent {
    Suspend,
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::remote) struct PowerTransition {
    pub(in crate::remote) stop_generation: bool,
    pub(in crate::remote) start_generation: bool,
    pub(in crate::remote) rotate_epoch: bool,
}

pub(in crate::remote) fn power_transition(
    event: PowerEvent,
    desired_running: bool,
) -> PowerTransition {
    match (event, desired_running) {
        (PowerEvent::Suspend, true) => PowerTransition {
            stop_generation: true,
            start_generation: false,
            rotate_epoch: false,
        },
        (PowerEvent::Resume, true) => PowerTransition {
            stop_generation: false,
            start_generation: true,
            rotate_epoch: true,
        },
        _ => PowerTransition {
            stop_generation: false,
            start_generation: false,
            rotate_epoch: false,
        },
    }
}

/// OS power lifecycle adapter. Suspend tears down only generation-local
/// resources; pairing, reply deduplication, drop episodes, and desired-running
/// intent remain available for resume.
pub async fn handle_system_suspend() {
    let notices = mobile_disconnect_notices("system_sleep");
    SUPERVISOR.access.invalidate(|| {
        SUPERVISOR.suspended.store(true, Ordering::Release);
        SUPERVISOR.cancel_tasks();
        let transition = power_transition(
            PowerEvent::Suspend,
            SUPERVISOR.start_requested.load(Ordering::Acquire),
        );
        if transition.stop_generation {
            *SUPERVISOR.last_error_code.lock().unwrap() = Some("system_sleep".to_string());
            SUPERVISOR
                .credential_refreshing
                .store(false, Ordering::Release);
            disable_handshake();
            stop_runtime();
        }
    });
    for (client, subject, payload) in notices {
        send_mobile_disconnect_notice(client, subject, payload).await;
    }
}

/// Resume with a fresh bridge epoch so the phone cannot mistake a half-open
/// pre-sleep socket for the current generation. `establish()` refreshes the
/// JWT before the NATS connection is built.
pub fn handle_system_resume() {
    SUPERVISOR.suspended.store(false, Ordering::Release);
    let transition = power_transition(
        PowerEvent::Resume,
        SUPERVISOR.start_requested.load(Ordering::Acquire),
    );
    if !transition.start_generation {
        return;
    }
    if transition.rotate_epoch {
        for shared in SUPERVISOR.bridge_shared.lock().unwrap().values_mut() {
            shared.bridge_instance_id =
                format!("bridge_{}", nkeys::KeyPair::new_user().public_key());
        }
    }
    *SUPERVISOR.last_error_code.lock().unwrap() = Some("system_sleep".to_string());
    SUPERVISOR
        .resume_recovery_running
        .store(true, Ordering::Release);
    SUPERVISOR.spawn(async {
        match start_once(false).await {
            Ok(status) if retryable_start_status(&status) => {
                spawn_start_retry();
                SUPERVISOR
                    .resume_recovery_running
                    .store(false, Ordering::Release);
            }
            Ok(_) => {
                SUPERVISOR
                    .resume_recovery_running
                    .store(false, Ordering::Release);
            }
            Err(error) => {
                SUPERVISOR
                    .resume_recovery_running
                    .store(false, Ordering::Release);
                eprintln!("remote: resume recovery failed [PW001]: {error}");
                *SUPERVISOR.last_error_code.lock().unwrap() =
                    Some("reconnect_required".to_string());
            }
        }
    });
}

pub(in crate::remote) fn abort_generation(state: RemoteState) {
    debug_assert!(state.generation_id > 0, "generation IDs start at one");
    state.event_task.abort();
    state.cmd_task.abort();
    state.transfer_task.abort();
    state.heartbeat_task.abort();
    state.refresh_task.abort();
    if let Some(web_task) = state.web_task {
        web_task.abort();
    }
    // Transfers belong to the access epoch, not the replaced socket.
}

pub(in crate::remote) fn stop_runtime() -> RemoteStatus {
    transfer::clear_transfers();
    let states: Vec<RemoteState> = SUPERVISOR
        .state
        .lock()
        .unwrap()
        .drain()
        .map(|(_, state)| state)
        .collect();
    for state in states {
        announce_offline(&state);
        abort_generation(state);
    }
    *SUPERVISOR.active.lock().unwrap() = None;
    empty()
}

/// Stop one device's bridge and leave the others serving.
///
/// The offline notice goes out on a plain task, not through the process-lifetime
/// registry: this is called per device (a removal, a rebuild) and registering
/// each one there made teardown wait on a publish — bounded by
/// [`DISCONNECT_NOTICE_TIMEOUT`] itself, so nothing is gained by also making it
/// the caller's problem.
pub(in crate::remote) fn stop_slot(pair_id: &str) {
    let state = SUPERVISOR.state.lock().unwrap().remove(pair_id);
    if let Some(state) = state {
        announce_offline_untracked(&state);
        abort_generation(state);
    }
}

/// Best-effort "this device is now offline", on a task the process registry
/// does not own.
fn announce_offline_untracked(state: &RemoteState) {
    let Some((client, subject, payload)) = offline_notice(state) else {
        return;
    };
    tokio::spawn(send_mobile_disconnect_notice(client, subject, payload));
}

fn offline_notice(state: &RemoteState) -> Option<(async_nats::Client, String, Vec<u8>)> {
    let pair_id = state.pair_id.clone();
    let subject = format!("p.{pair_id}.presence");
    let plaintext = serde_json::to_vec(&json!({
        "online": false, "pairId": pair_id,
        "bridgeInstanceId": state.bridge_instance_id, "lastHeartbeatTs": unix_timestamp(),
    }))
    .unwrap_or_default();
    let payload = state.security.seal(&subject, &plaintext).ok()??;
    Some((state.client.clone(), subject, payload))
}

/// Best-effort "this device is now offline" on its own presence subject.
fn announce_offline(state: &RemoteState) {
    let pair_id = state.pair_id.clone();
    let subject = format!("p.{pair_id}.presence");
    let plaintext = serde_json::to_vec(&json!({
        "online": false, "pairId": pair_id,
        "bridgeInstanceId": state.bridge_instance_id, "lastHeartbeatTs": unix_timestamp(),
    }))
    .unwrap_or_default();
    if let Ok(Some(payload)) = state.security.seal(&subject, &plaintext) {
        crate::runtime::spawn(send_mobile_disconnect_notice(
            state.client.clone(),
            subject,
            payload,
        ));
    }
}
