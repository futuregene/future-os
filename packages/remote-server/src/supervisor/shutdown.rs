//! Role stop, device removal and OS lifecycle; no process-global exit hooks.
use super::*;
use crate::types::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    Suspend,
    Resume,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerTransition {
    pub stop_generation: bool,
    pub start_generation: bool,
    pub rotate_epoch: bool,
}
pub async fn send_mobile_disconnect_notice(
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
    let _ = tokio::time::timeout(std::time::Duration::from_millis(750), send).await;
}

pub fn power_transition(event: PowerEvent, desired_running: bool) -> PowerTransition {
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

pub fn abort_generation(state: RemoteState) {
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
impl Supervisor {
    pub async fn unpair(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        let notices = self.mobile_unpair_notices();
        let result = self.stop_with(|| {
            for creds in self.host.load_slots() {
                self.host.queue_revoke(&creds)?;
            }
            self.host.clear()
        });
        for (client, subject, payload) in notices {
            self.spawn_notice(send_mobile_disconnect_notice(client, subject, payload));
        }
        // Stop is already effective even if local persistence fails. Keep the
        // credential on disk when queueing fails so a user retry can still revoke.
        result?;
        self.host.clear_preview_cache();
        self.spawn_revoke_cleanup();
        Ok(self.empty())
    }

    pub async fn unpair_slot(
        self: &Arc<Self>,
        pair_id: &str,
    ) -> Result<RemoteStatus, crate::Error> {
        let notice = self.mobile_disconnect_notice_for(pair_id, "unpaired");
        let creds = self
            .host
            .load_slots()
            .into_iter()
            .find(|slot| slot.pair_id == pair_id);
        let result = (|| -> Result<(), crate::Error> {
            if let Some(creds) = &creds {
                self.host.queue_revoke(creds)?;
            }
            self.host.remove_slot(pair_id)?;
            Ok(())
        })();
        let released = self
            .active
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|active| active == pair_id);
        if released {
            self.release_control(pair_id);
        }
        self.stop_slot(pair_id);
        self.promote_a_lone_device();
        if let Some((client, subject, payload)) = notice {
            self.spawn_notice(send_mobile_disconnect_notice(client, subject, payload));
        }
        result?;
        self.spawn_revoke_cleanup();
        Ok(self.status())
    }

    pub async fn disconnect_device(
        self: &Arc<Self>,
        pair_id: &str,
    ) -> Result<RemoteStatus, crate::Error> {
        if !self.state.lock().unwrap().contains_key(pair_id) {
            // No runtime means the device is already not connected; saying so is
            // more useful than a silent success that would hide a typo.
            return Err(crate::Error::Message("device_offline".to_string()));
        }
        let notice = self.mobile_disconnect_notice_for(pair_id, "disconnected");
        let released = self
            .active
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|active| active == pair_id);
        if released {
            self.release_control(pair_id);
        }
        self.stop_slot(pair_id);
        // Deliberately no `promote_a_lone_device` here, unlike unpairing: the
        // device stays paired, and promoting it would report a device this machine
        // is no longer serving as its controller. Control stays with nobody until a
        // device connects and claims it (the persisted pointer is untouched, so the
        // same device takes it back when it reconnects).
        if let Some((client, subject, payload)) = notice {
            self.spawn_notice(send_mobile_disconnect_notice(client, subject, payload));
        }
        Ok(self.status())
    }

    pub fn mobile_unpair_notices(self: &Arc<Self>) -> Vec<(async_nats::Client, String, Vec<u8>)> {
        let guard = self.state.lock().unwrap();
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

    pub fn mobile_disconnect_notice_for(
        self: &Arc<Self>,
        pair_id: &str,
        reason: &str,
    ) -> Option<(async_nats::Client, String, Vec<u8>)> {
        let guard = self.state.lock().unwrap();
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

    pub fn stop(self: &Arc<Self>) -> RemoteStatus {
        self.stop_with(|| ());
        self.empty()
    }

    pub fn disable_handshake(self: &Arc<Self>) {
        let guard = self.bridge_shared.lock().unwrap();
        for shared in guard.values() {
            let mut handshake = shared.handshake.lock().unwrap();
            if let Some(state) = handshake.take() {
                state.active_flag().store(false, Ordering::Release);
                state.secure.clear();
            }
        }
    }

    pub async fn stop_gracefully(self: &Arc<Self>, reason: &str) -> RemoteStatus {
        // Cancelling the bridge must not wait on a broker that is already down or
        // reconnecting. Capture the best-effort presence notification while the
        // generation still owns its client, then stop synchronously so it also
        // cancels any automatic reconnect work immediately.
        let notices = self.mobile_disconnect_notices(reason);
        let status = self.stop();
        for (client, subject, payload) in notices {
            if self.executor.is_stopping() {
                // Registration is frozen at process exit. Still deliver the bounded
                // presence notice before the cleanup runtime goes away.
                send_mobile_disconnect_notice(client, subject, payload).await;
            } else {
                self.spawn_notice(async move {
                    send_mobile_disconnect_notice(client, subject, payload).await;
                });
            }
        }
        status
    }

    pub async fn notify_mobile_disconnect(self: &Arc<Self>, reason: &str) {
        for (client, subject, payload) in self.mobile_disconnect_notices(reason) {
            send_mobile_disconnect_notice(client, subject, payload).await;
        }
    }

    pub fn mobile_disconnect_notices(
        self: &Arc<Self>,
        reason: &str,
    ) -> Vec<(async_nats::Client, String, Vec<u8>)> {
        let pair_ids: Vec<String> = self.state.lock().unwrap().keys().cloned().collect();
        pair_ids
            .into_iter()
            .filter_map(|pair_id| self.mobile_disconnect_notice(&pair_id, reason))
            .collect()
    }

    pub fn mobile_disconnect_notice(
        self: &Arc<Self>,
        pair_id: &str,
        reason: &str,
    ) -> Option<(async_nats::Client, String, Vec<u8>)> {
        self.state.lock().unwrap().get(pair_id).and_then(|state| {
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

    pub async fn handle_system_suspend(self: &Arc<Self>) {
        let notices = self.mobile_disconnect_notices("system_sleep");
        self.access.invalidate(|| {
            self.suspended.store(true, Ordering::Release);
            self.cancel_tasks();
            let transition = power_transition(
                PowerEvent::Suspend,
                self.start_requested.load(Ordering::Acquire),
            );
            if transition.stop_generation {
                *self.last_error_code.lock().unwrap() = Some("system_sleep".to_string());
                self.credential_refreshing.store(false, Ordering::Release);
                self.disable_handshake();
                self.stop_runtime();
            }
        });
        for (client, subject, payload) in notices {
            send_mobile_disconnect_notice(client, subject, payload).await;
        }
    }

    pub fn handle_system_resume(self: &Arc<Self>) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        self.suspended.store(false, Ordering::Release);
        let transition = power_transition(
            PowerEvent::Resume,
            self.start_requested.load(Ordering::Acquire),
        );
        if !transition.start_generation {
            return;
        }
        if transition.rotate_epoch {
            for shared in self.bridge_shared.lock().unwrap().values_mut() {
                shared.bridge_instance_id =
                    format!("bridge_{}", nkeys::KeyPair::new_user().public_key());
            }
        }
        *self.last_error_code.lock().unwrap() = Some("system_sleep".to_string());
        self.resume_recovery_running.store(true, Ordering::Release);
        self.spawn({
            let server = self.clone();
            async move {
                match server.start_once(false).await {
                    Ok(status) if retryable_start_status(&status) => {
                        server.spawn_start_retry();
                        server
                            .resume_recovery_running
                            .store(false, Ordering::Release);
                    }
                    Ok(_) => {
                        server
                            .resume_recovery_running
                            .store(false, Ordering::Release);
                    }
                    Err(error) => {
                        server
                            .resume_recovery_running
                            .store(false, Ordering::Release);
                        eprintln!("remote: resume recovery failed [PW001]: {error}");
                        *server.last_error_code.lock().unwrap() =
                            Some("reconnect_required".to_string());
                    }
                }
            }
        });
    }

    pub fn stop_runtime(self: &Arc<Self>) -> RemoteStatus {
        self.host.clear_transfers();
        let states: Vec<RemoteState> = self
            .state
            .lock()
            .unwrap()
            .drain()
            .map(|(_, state)| state)
            .collect();
        for state in states {
            self.announce_offline(&state);
            abort_generation(state);
        }
        self.cancel_generations();
        *self.active.lock().unwrap() = None;
        *self.web_owner.lock().unwrap() = None;
        self.empty()
    }

    pub fn stop_slot(self: &Arc<Self>, pair_id: &str) {
        let state = self.state.lock().unwrap().remove(pair_id);
        if let Some(state) = state {
            self.announce_slot_offline(&state);
            abort_generation(state);
        }
        let mut owner = self.web_owner.lock().unwrap();
        if owner.as_deref() == Some(pair_id) {
            *owner = None;
        }
    }

    fn announce_slot_offline(self: &Arc<Self>, state: &RemoteState) {
        let Some((client, subject, payload)) = offline_notice(state) else {
            return;
        };
        self.spawn_notice(send_mobile_disconnect_notice(client, subject, payload));
    }

    fn announce_offline(self: &Arc<Self>, state: &RemoteState) {
        let pair_id = state.pair_id.clone();
        let subject = format!("p.{pair_id}.presence");
        let plaintext = serde_json::to_vec(&json!({
            "online": false, "pairId": pair_id,
            "bridgeInstanceId": state.bridge_instance_id, "lastHeartbeatTs": unix_timestamp(),
        }))
        .unwrap_or_default();
        if let Ok(Some(payload)) = state.security.seal(&subject, &plaintext) {
            self.spawn_notice(send_mobile_disconnect_notice(
                state.client.clone(),
                subject,
                payload,
            ));
        }
    }
    pub async fn re_pair(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        self.unpair().await?;
        // `unpair` only queues the revoke: it is also the Remote page's button and
        // must not wait on HTTP. Here the queue cannot be left for the compensation
        // loop, because the platform gives a desktop it already knows the *same*
        // `pair_id` for the replacement invitation — a revoke delivered after it
        // would cancel the very code this command is about to print. Deliver it now.
        // A failure here is not fatal: creating the invitation replaces the old
        // binding anyway, and `create_pairing` retires whatever revoke is left.
        if let Err(error) = self.host.retry_revokes().await {
            eprintln!("remote: re-pair could not settle the saved pairing's revoke: {error}");
        }
        match self.add_device().await {
            Ok(status) => Ok(status),
            Err(error) => self.start_failure(error),
        }
    }
    pub fn stop_with<T>(self: &Arc<Self>, cleanup: impl FnOnce() -> T) -> T {
        self.access.invalidate(|| {
            self.start_requested.store(false, Ordering::Release);
            self.suspended.store(false, Ordering::Release);
            self.runtime_reconnect_attempts.store(0, Ordering::Release);
            self.runtime_failure_window_started
                .store(0, Ordering::Release);
            self.web_reconnect_attempts.store(0, Ordering::Release);
            self.cancel_tasks();
            self.stop_runtime();
            self.disable_handshake();
            self.bridge_shared.lock().unwrap().clear();
            // A stopped machine serves no device, so it holds no device set and no
            // controller. The book on disk is untouched and is read again on the
            // next start; keeping the in-memory answers here would describe a
            // machine that is not running.
            self.reset_devices();
            cleanup()
        })
    }
    pub async fn takeover(self: &Arc<Self>, pair_id: &str) -> Result<RemoteStatus, crate::Error> {
        if !self.state.lock().unwrap().contains_key(pair_id) {
            return Err(crate::Error::Message("device_offline".to_string()));
        }
        let previous = self.claim_control_soon(pair_id);
        if let Some(previous) = previous {
            self.publish_standby(&previous);
        }
        self.publish_control(pair_id, "active");
        // The windowed desktop raises its main window so the person taking
        // control sees the machine they now own; a headless host no-ops.
        self.host.on_control_claimed();
        Ok(self.status())
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn power_events_preserve_intent_and_only_resume_rebuilds_a_generation() {
        assert_eq!(
            power_transition(PowerEvent::Suspend, true),
            PowerTransition {
                stop_generation: true,
                start_generation: false,
                rotate_epoch: false,
            }
        );
        assert_eq!(
            power_transition(PowerEvent::Resume, true),
            PowerTransition {
                stop_generation: false,
                start_generation: true,
                rotate_epoch: true,
            }
        );
        for event in [PowerEvent::Suspend, PowerEvent::Resume] {
            assert_eq!(
                power_transition(event, false),
                PowerTransition {
                    stop_generation: false,
                    start_generation: false,
                    rotate_epoch: false,
                }
            );
        }
    }
    #[test]
    fn runtime_failure_budget_uses_a_ten_minute_window() {
        let supervisor = crate::supervisor::test_support::isolated();
        supervisor
            .runtime_failure_window_started
            .store(0, Ordering::Release);
        supervisor
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        assert_eq!(supervisor.record_runtime_failure(1_000), 1);
        assert_eq!(supervisor.record_runtime_failure(2_000), 2);
        assert_eq!(supervisor.record_runtime_failure(3_000), 3);
        assert_eq!(supervisor.record_runtime_failure(4_000), 4);
        assert_eq!(
            supervisor.record_runtime_failure(1_000 + RUNTIME_FAILURE_WINDOW_MS + 1),
            1
        );
        supervisor
            .runtime_failure_window_started
            .store(0, Ordering::Release);
        supervisor
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
    }
}
