//! Readiness handoff and credential rotation for one controlled endpoint.
use super::*;
use crate::pairing;
pub struct TransportTasks {
    pub event_tx: tokio::sync::mpsc::Sender<EventPublish>,
    pub event_task: TaskHandle,
    pub cmd_task: TaskHandle,
    pub transfer_task: TaskHandle,
    pub heartbeat_task: TaskHandle,
    pub candidate_tasks: lifecycle::CandidateTasks,
}
impl Supervisor {
    pub async fn build_transport(
        self: &Arc<Self>,
        client: &async_nats::Client,
        pair_id: &str,
        handshake: &commands::HandshakeState,
        reply_slots: commands::ReplySlots,
    ) -> Result<TransportTasks, crate::Error> {
        #[cfg(any(test, feature = "test-support"))]
        if self.hooks.inject_build_failure.swap(0, Ordering::SeqCst) > 0 {
            return Err(crate::Error::RemoteTransport(
                "injected: the secure handshake was refused".to_string(),
            ));
        }
        let epoch = handshake
            .access_epoch()
            .unwrap_or_else(|| self.access.current());
        let mut candidate_tasks = lifecycle::CandidateTasks::default();
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(4096);
        let coalesce = self.coalesce_events(pair_id);
        // A live-lane capability belongs to the connection that declared it. Reset
        // it here so a client that never declares it — an older build on the same
        // pairing — is served the legacy lane instead of inheriting the previous
        // client's request.
        coalesce.store(false, Ordering::Release);
        // The catalog's recency packets are the same kind of permission: they
        // replace a whole session list with a few dozen bytes, which only a client
        // that applies packets can be served.
        self.catalog_activity(pair_id)
            .store(false, Ordering::Release);
        // Same rule for the reply encoding: an older client has no gzip magic-byte
        // detection and would fail to parse a compressed reply, so the permission
        // must not outlive the connection that asked for it.
        handshake.gzip_replies.store(false, Ordering::Release);
        handshake.lean_events.store(false, Ordering::Release);
        self.lean_events(pair_id).store(false, Ordering::Release);
        let event_task = self.spawn_generation_for(
            epoch,
            crate::events::run_event_publisher(
                client.clone(),
                event_rx,
                handshake.secure.clone(),
                coalesce,
            ),
        );
        candidate_tasks.track_owned(&event_task);
        let (command_ready_tx, command_ready_rx) = tokio::sync::oneshot::channel();
        let cmd_task = self.spawn_generation_for(
            epoch,
            commands::command_loop_with_ready(
                client.clone(),
                pair_id.into(),
                reply_slots,
                handshake.clone(),
                Some(command_ready_tx),
            ),
        );
        candidate_tasks.track_owned(&cmd_task);
        let (transfer_ready_tx, transfer_ready_rx) = tokio::sync::oneshot::channel();
        let transfer = crate::transfer::TransferLoop::new(
            self.host.clone(),
            handshake.secure.clone(),
            handshake.active_flag(),
            self.executor.options().transfer_cleanup,
        );
        let transfer_task = self.spawn_generation_for(
            epoch,
            transfer.run(client.clone(), pair_id.into(), Some(transfer_ready_tx)),
        );
        candidate_tasks.track_owned(&transfer_task);
        #[cfg(any(test, feature = "test-support"))]
        {
            let pause = self.hooks.readiness_pause.lock().unwrap().take();
            if let Some((entered, release)) = pause {
                let _ = entered.send(());
                let _ = release.await;
            }
        }
        let readiness = async {
            command_ready_rx
                .await
                .map_err(|_| crate::Error::RemoteTransport("command readiness ended".into()))?;
            transfer_ready_rx
                .await
                .map_err(|_| crate::Error::RemoteTransport("transfer readiness ended".into()))?;
            secure::publish(
                client,
                &handshake.secure,
                format!("p.{pair_id}.presence"),
                serde_json::to_vec(
                    &self.light_presence_payload(pair_id, handshake.bridge_instance_id()),
                )?,
            )
            .await?;
            client
                .flush()
                .await
                .map_err(|error| crate::Error::RemoteTransport(error.to_string()))
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), readiness)
            .await
            .map_err(|_| crate::Error::RemoteTransport("Remote readiness timed out".into()))??;
        let heartbeat_task = self.spawn_generation_for(
            epoch,
            self.run_presence_heartbeat(
                client.clone(),
                pair_id.into(),
                handshake.bridge_instance_id().into(),
                handshake.secure.clone(),
                self.executor.options().publisher,
            ),
        );
        candidate_tasks.track_owned(&heartbeat_task);
        Ok(TransportTasks {
            event_tx,
            event_task,
            cmd_task,
            transfer_task,
            heartbeat_task,
            candidate_tasks,
        })
    }
    fn renewal_retry_delay(&self, seconds_until_expiry: i64) -> std::time::Duration {
        if seconds_until_expiry <= 0 {
            self.executor.options().expired_retry
        } else {
            self.executor.options().refresh_tick
        }
    }
    pub fn spawn_credential_refresh(
        self: &Arc<Self>,
        pair_id: String,
        reply_slots: commands::ReplySlots,
        pairing_confirmed: Arc<AtomicBool>,
        handshake_state: commands::HandshakeState,
    ) -> TaskHandle {
        {
            let epoch = handshake_state
                .access_epoch()
                .unwrap_or_else(|| self.access.current());
            let server = self.clone();
            self.spawn_generation_for(epoch, async move {
                loop {
                    while !pairing_confirmed.load(Ordering::Acquire) {
                        tokio::time::sleep(server.executor.options().publisher.presence).await;
                    }
                    let Some(creds) = server.host.load().filter(|creds| creds.pair_id == pair_id)
                    else {
                        return;
                    };
                    // Credential scheduling owns only credential expiry. Critical task
                    // and transport health are exclusively decided by
                    // RemoteSupervisor, preventing two loops from replacing the same
                    // generation concurrently.
                    loop {
                        // Check at the tick cadence — so a replaced generation stops
                        // promptly — but wake exactly when the renewal is due. Waiting
                        // out a fixed tick and *then* asking "is it due yet" made the
                        // margin a function of the tick rather than of the work the
                        // renewal has to do.
                        let wait = pairing::refresh_delay(&creds)
                            .min(server.executor.options().refresh_tick);
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                        let generation_active = server
                            .state
                            .lock()
                            .unwrap()
                            .get(&pair_id)
                            .is_some_and(|state| {
                                Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
                            });
                        if !generation_active {
                            return;
                        }
                        if server
                            .state
                            .lock()
                            .unwrap()
                            .get(&pair_id)
                            .is_some_and(|state| state.nats_health.is_terminal())
                        {
                            // A deterministic authorization/configuration failure does
                            // not improve by rotating the same credentials forever.
                            return;
                        }
                        // Renew once the lead time is reached; the delay above means
                        // the loop is already awake at that moment.
                        let refresh_due = pairing::refresh_delay(&creds).is_zero();
                        if refresh_due {
                            server.credential_refreshing.store(true, Ordering::Release);
                            break;
                        }
                    }
                    let _start_guard = server.start_lock.lock().await;
                    let until_expiry = pairing::seconds_until_expiry(&creds);
                    let refreshed = match server.host.refresh(creds).await {
                        Ok(creds) => creds,
                        Err(error)
                            if pairing::is_invalid_or_revoked_error(&error)
                                || pairing::is_account_authorization_error(&error) =>
                        {
                            // The pairing or its platform account authorization was revoked (web-side unpair, or this desktop
                            // re-paired elsewhere). Retrying forever would keep a
                            // zombie bridge that can never work again while the GUI
                            // shows "running": drop the dead credential, record why,
                            // and stop the bridge. `server.stop()` aborts this very task, but
                            // abort only lands at the next await and we return here.
                            let generation_active = server
                                .state
                                .lock()
                                .unwrap()
                                .get(&pair_id)
                                .is_some_and(|state| {
                                    Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
                                });
                            if !generation_active {
                                return;
                            }
                            let revoked = pairing::is_invalid_or_revoked_error(&error);
                            eprintln!(
                                "remote: platform rejected account authorization; stopping bridge"
                            );
                            *server.last_error_code.lock().unwrap() = Some(
                                if revoked {
                                    "revoked"
                                } else {
                                    "account_authorization"
                                }
                                .to_string(),
                            );
                            server.credential_refreshing.store(false, Ordering::Release);
                            if revoked {
                                // Only this device's credential is dead. The others on
                                // this machine keep serving, and the control pointer is
                                // cleared if it named the device that just died.
                                let _ = server.forget_slot(&pair_id);
                            } else {
                                // Account-level authorization is not a device fact: the
                                // whole machine cannot authenticate, so every bridge
                                // stops.
                                let _ = server.stop();
                            }
                            return;
                        }
                        Err(error) => {
                            if let Some(line) = server
                                .credential_failures
                                .record("credential_network", error)
                            {
                                eprintln!("{line}");
                            }
                            server.credential_refreshing.store(false, Ordering::Release);
                            tokio::time::sleep(server.renewal_retry_delay(until_expiry)).await;
                            continue;
                        }
                    };
                    let connected_nats = match server.connect_nats(&refreshed, true).await {
                        Ok(connection) => connection,
                        Err(crate::Error::RemoteAuthorization(error)) => {
                            eprintln!(
                                "remote: refreshed NATS credential rejected [AU001]: {error}"
                            );
                            if let Some(state) = server.state.lock().unwrap().get(&pair_id) {
                                state
                                    .nats_health
                                    .service_config_error
                                    .store(true, Ordering::Release);
                            }
                            *server.last_error_code.lock().unwrap() =
                                Some("service_authorization".to_string());
                            server.credential_refreshing.store(false, Ordering::Release);
                            return;
                        }
                        Err(error) => {
                            if let Some(line) = server
                                .credential_failures
                                .record("credential_connect", error)
                            {
                                eprintln!("{line}");
                            }
                            server.credential_refreshing.store(false, Ordering::Release);
                            tokio::time::sleep(
                                server
                                    .renewal_retry_delay(pairing::seconds_until_expiry(&refreshed)),
                            )
                            .await;
                            continue;
                        }
                    };
                    let client = connected_nats.client;
                    let nats_health = connected_nats.health;
                    // Serialize refresh installation with explicit start/recovery. A
                    // refresh scheduler belongs to its current runtime and is aborted
                    // when that runtime is replaced.
                    let transport = match server
                        .build_transport(&client, &pair_id, &handshake_state, reply_slots.clone())
                        .await
                    {
                        Ok(transport) => transport,
                        Err(error) => {
                            if let Some(line) = server
                                .credential_failures
                                .record("credential_connect", error)
                            {
                                eprintln!("{line}");
                            }
                            server.credential_refreshing.store(false, Ordering::Release);
                            tokio::time::sleep(
                                server
                                    .renewal_retry_delay(pairing::seconds_until_expiry(&refreshed)),
                            )
                            .await;
                            continue;
                        }
                    };
                    let TransportTasks {
                        event_tx,
                        event_task: new_event,
                        cmd_task: new_cmd,
                        transfer_task: new_transfer,
                        heartbeat_task: new_heartbeat,
                        mut candidate_tasks,
                    } = transport;
                    // Hold the server.state lock across the generation check AND the creds
                    // save: saving outside the lock raced `unpair()` (stop → clear
                    // creds) and could resurrect a just-revoked credential file.
                    let mut guard = server.state.lock().unwrap();
                    let Some(state) = guard
                        .get_mut(&pair_id)
                        .filter(|state| Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed))
                    else {
                        new_event.abort();
                        new_cmd.abort();
                        new_transfer.abort();
                        new_heartbeat.abort();
                        server.credential_refreshing.store(false, Ordering::Release);
                        return;
                    };
                    if let Err(error) = server.host.save(&refreshed) {
                        eprintln!("remote: save refreshed credential failed [LC004]: {error}");
                        server.credential_refreshing.store(false, Ordering::Release);
                        continue;
                    }
                    candidate_tasks.installed();
                    let old_cmd = std::mem::replace(&mut state.cmd_task, new_cmd);
                    let old_transfer = std::mem::replace(&mut state.transfer_task, new_transfer);
                    let old_heartbeat = std::mem::replace(&mut state.heartbeat_task, new_heartbeat);
                    let old_event = std::mem::replace(&mut state.event_task, new_event);
                    state.event_tx = event_tx;
                    state.client = client;
                    state.nats_health = nats_health;
                    state.nats_url = refreshed.nats_url;
                    old_cmd.abort();
                    old_transfer.abort();
                    old_heartbeat.abort();
                    // The old event drain is deliberately NOT aborted: dropping the
                    // handle detaches it, and it exits on its own after flushing its
                    // backlog — no event gap at the swap point.
                    drop(old_event);
                    if let Some(line) = server.credential_failures.recovered() {
                        eprintln!("{line}");
                    }
                    server.credential_refreshing.store(false, Ordering::Release);
                }
            })
        }
    }
}

#[cfg(test)]
mod policy_tests {
    #[test]
    fn a_failed_renewal_retries_fast_only_once_the_credential_expired() {
        let supervisor = crate::supervisor::test_support::isolated();
        // Still usable: the ordinary cadence, so a broker or platform outage is
        // not hammered.
        assert_eq!(
            supervisor.renewal_retry_delay(600),
            supervisor.executor.options().refresh_tick
        );
        assert_eq!(
            supervisor.renewal_retry_delay(1),
            supervisor.executor.options().refresh_tick
        );
        // Expired: nothing can be published, so waiting a full tick is dropped
        // events — retry faster than the ordinary cadence.
        assert_eq!(
            supervisor.renewal_retry_delay(0),
            supervisor.executor.options().expired_retry
        );
        assert_eq!(
            supervisor.renewal_retry_delay(-30),
            supervisor.executor.options().expired_retry
        );
        assert!(
            supervisor.executor.options().expired_retry
                < supervisor.executor.options().refresh_tick
        );
    }
}
