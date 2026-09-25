use super::*;

pub(super) struct TransportTasks {
    pub(super) event_tx: tokio::sync::mpsc::Sender<EventPublish>,
    pub(super) event_task: tokio::task::JoinHandle<()>,
    pub(super) cmd_task: tokio::task::JoinHandle<()>,
    pub(super) transfer_task: tokio::task::JoinHandle<()>,
    pub(super) heartbeat_task: tokio::task::JoinHandle<()>,
    pub(super) candidate_tasks: lifecycle::CandidateTasks,
}

#[cfg(test)]
pub(super) type ReadinessPause = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);
#[cfg(test)]
pub(super) static READINESS_PAUSE: Mutex<Option<ReadinessPause>> = Mutex::new(None);

/// All connection paths use the same subscription readiness and task ownership.
pub(super) async fn build_transport(
    client: &async_nats::Client,
    pair_id: &str,
    handshake: &commands::HandshakeState,
    reply_slots: commands::ReplySlots,
) -> Result<TransportTasks, crate::AppError> {
    let mut candidate_tasks = lifecycle::CandidateTasks::default();
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
    let coalesce = SUPERVISOR.coalesce_events(pair_id);
    // A live-lane capability belongs to the connection that declared it. Reset
    // it here so a client that never declares it — an older build on the same
    // pairing — is served the legacy lane instead of inheriting the previous
    // client's request.
    coalesce.store(false, Ordering::Release);
    // Same rule for the reply encoding: an older client has no gzip magic-byte
    // detection and would fail to parse a compressed reply, so the permission
    // must not outlive the connection that asked for it.
    handshake.gzip_replies.store(false, Ordering::Release);
    crate::remote_host::lean::set_enabled(false);
    let event_task =
        spawn_secure_event_publisher(client.clone(), event_rx, handshake.secure.clone(), coalesce);
    candidate_tasks.track(&event_task);
    let (command_ready_tx, command_ready_rx) = tokio::sync::oneshot::channel();
    let cmd_task = tokio::spawn(commands::command_loop_with_ready(
        client.clone(),
        pair_id.into(),
        reply_slots,
        handshake.clone(),
        Some(command_ready_tx),
    ));
    candidate_tasks.track(&cmd_task);
    let (transfer_ready_tx, transfer_ready_rx) = tokio::sync::oneshot::channel();
    let transfer_task = transfer::spawn_transfer_loop_with_ready(
        client.clone(),
        pair_id.into(),
        handshake.active_flag(),
        handshake.secure.clone(),
        Some(transfer_ready_tx),
    );
    candidate_tasks.track(&transfer_task);
    #[cfg(test)]
    {
        let pause = READINESS_PAUSE.lock().unwrap().take();
        if let Some((entered, release)) = pause {
            let _ = entered.send(());
            let _ = release.await;
        }
    }
    let readiness = async {
        command_ready_rx
            .await
            .map_err(|_| crate::AppError::RemoteTransport("command readiness ended".into()))?;
        transfer_ready_rx
            .await
            .map_err(|_| crate::AppError::RemoteTransport("transfer readiness ended".into()))?;
        secure::publish(
            client,
            &handshake.secure,
            format!("p.{pair_id}.presence"),
            serde_json::to_vec(&light_presence_payload(
                pair_id,
                handshake.bridge_instance_id(),
            ))?,
        )
        .await?;
        client
            .flush()
            .await
            .map_err(|error| crate::AppError::RemoteTransport(error.to_string()))
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), readiness)
        .await
        .map_err(|_| crate::AppError::RemoteTransport("Remote readiness timed out".into()))??;
    let heartbeat_task = spawn_secure_presence_heartbeat(
        client.clone(),
        pair_id.into(),
        handshake.bridge_instance_id().into(),
        handshake.secure.clone(),
    );
    candidate_tasks.track(&heartbeat_task);
    Ok(TransportTasks {
        event_tx,
        event_task,
        cmd_task,
        transfer_task,
        heartbeat_task,
        candidate_tasks,
    })
}

pub(super) fn spawn_credential_refresh(
    pair_id: String,
    reply_slots: commands::ReplySlots,
    pairing_confirmed: Arc<AtomicBool>,
    handshake_state: commands::HandshakeState,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            while !pairing_confirmed.load(Ordering::Acquire) {
                tokio::time::sleep(presence_tick()).await;
            }
            let Some(creds) = pairing::load_creds().filter(|creds| creds.pair_id == pair_id) else {
                return;
            };
            // Credential scheduling owns only credential expiry. Critical task
            // and transport health are exclusively decided by
            // RemoteSupervisor, preventing two loops from replacing the same
            // generation concurrently.
            loop {
                tokio::time::sleep(refresh_tick()).await;
                let generation_active =
                    SUPERVISOR
                        .state
                        .lock()
                        .unwrap()
                        .as_ref()
                        .is_some_and(|state| {
                            state.pair_id == pair_id
                                && Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
                        });
                if !generation_active {
                    return;
                }
                if SUPERVISOR
                    .state
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|state| state.nats_health.is_terminal())
                {
                    // A deterministic authorization/configuration failure does
                    // not improve by rotating the same credentials forever.
                    return;
                }
                // Refresh this far ahead of expiry (a production tick); kept
                // independent of the test-shrunk tick so the due path stays
                // reachable under test timing.
                let refresh_due =
                    pairing::refresh_delay(&creds) < std::time::Duration::from_secs(15);
                if refresh_due {
                    SUPERVISOR
                        .credential_refreshing
                        .store(true, Ordering::Release);
                    break;
                }
            }
            let _start_guard = SUPERVISOR.start_lock.lock().await;
            let refreshed = match pairing::refresh_bridge_jwt(creds).await {
                Ok(creds) => creds,
                Err(error)
                    if pairing::is_invalid_or_revoked_error(&error)
                        || pairing::is_account_authorization_error(&error) =>
                {
                    // The pairing or its platform account authorization was revoked (web-side unpair, or this desktop
                    // re-paired elsewhere). Retrying forever would keep a
                    // zombie bridge that can never work again while the GUI
                    // shows "running": drop the dead credential, record why,
                    // and stop the bridge. `stop()` aborts this very task, but
                    // abort only lands at the next await and we return here.
                    let generation_active =
                        SUPERVISOR
                            .state
                            .lock()
                            .unwrap()
                            .as_ref()
                            .is_some_and(|state| {
                                state.pair_id == pair_id
                                    && Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
                            });
                    if !generation_active {
                        return;
                    }
                    let revoked = pairing::is_invalid_or_revoked_error(&error);
                    eprintln!("remote: platform rejected account authorization; stopping bridge");
                    *SUPERVISOR.last_error_code.lock().unwrap() = Some(
                        if revoked {
                            "revoked"
                        } else {
                            "account_authorization"
                        }
                        .to_string(),
                    );
                    SUPERVISOR
                        .credential_refreshing
                        .store(false, Ordering::Release);
                    if revoked {
                        let _ = pairing::clear_creds();
                    }
                    let _ = stop();
                    return;
                }
                Err(error) => {
                    if let Some(line) = CREDENTIAL_EPISODE.record("credential_network", error) {
                        eprintln!("{line}");
                    }
                    SUPERVISOR
                        .credential_refreshing
                        .store(false, Ordering::Release);
                    tokio::time::sleep(refresh_tick()).await;
                    continue;
                }
            };
            let connected_nats = match connect_nats(&refreshed, true).await {
                Ok(connection) => connection,
                Err(crate::AppError::RemoteAuthorization(error)) => {
                    eprintln!("remote: refreshed NATS credential rejected [AU001]: {error}");
                    if let Some(state) = SUPERVISOR.state.lock().unwrap().as_ref() {
                        state
                            .nats_health
                            .service_config_error
                            .store(true, Ordering::Release);
                    }
                    *SUPERVISOR.last_error_code.lock().unwrap() =
                        Some("service_authorization".to_string());
                    SUPERVISOR
                        .credential_refreshing
                        .store(false, Ordering::Release);
                    return;
                }
                Err(error) => {
                    if let Some(line) = CREDENTIAL_EPISODE.record("credential_connect", error) {
                        eprintln!("{line}");
                    }
                    SUPERVISOR
                        .credential_refreshing
                        .store(false, Ordering::Release);
                    tokio::time::sleep(refresh_tick()).await;
                    continue;
                }
            };
            let client = connected_nats.client;
            let nats_health = connected_nats.health;
            // Serialize refresh installation with explicit start/recovery. A
            // refresh scheduler belongs to its current runtime and is aborted
            // when that runtime is replaced.
            let transport =
                match build_transport(&client, &pair_id, &handshake_state, reply_slots.clone())
                    .await
                {
                    Ok(transport) => transport,
                    Err(error) => {
                        if let Some(line) = CREDENTIAL_EPISODE.record("credential_connect", error) {
                            eprintln!("{line}");
                        }
                        SUPERVISOR
                            .credential_refreshing
                            .store(false, Ordering::Release);
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
            // Hold the SUPERVISOR.state lock across the generation check AND the creds
            // save: saving outside the lock raced `unpair()` (stop → clear
            // creds) and could resurrect a just-revoked credential file.
            let mut guard = SUPERVISOR.state.lock().unwrap();
            let Some(state) = guard.as_mut().filter(|state| {
                state.pair_id == pair_id
                    && Arc::ptr_eq(&state.pairing_confirmed, &pairing_confirmed)
            }) else {
                new_event.abort();
                new_cmd.abort();
                new_transfer.abort();
                new_heartbeat.abort();
                SUPERVISOR
                    .credential_refreshing
                    .store(false, Ordering::Release);
                return;
            };
            if let Err(error) = pairing::save_creds(&refreshed) {
                eprintln!("remote: save refreshed credential failed [LC004]: {error}");
                SUPERVISOR
                    .credential_refreshing
                    .store(false, Ordering::Release);
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
            if let Some(line) = CREDENTIAL_EPISODE.recovered() {
                eprintln!("{line}");
            }
            SUPERVISOR
                .credential_refreshing
                .store(false, Ordering::Release);
        }
    })
}
