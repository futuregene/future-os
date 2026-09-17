use super::*;

pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    spawn_revoke_cleanup();
    // An explicit user reconnect gets fresh automatic-reconnect budgets.
    SUPERVISOR
        .runtime_reconnect_attempts
        .store(0, Ordering::Release);
    SUPERVISOR
        .runtime_failure_window_started
        .store(0, Ordering::Release);
    SUPERVISOR
        .web_reconnect_attempts
        .store(0, Ordering::Release);
    SUPERVISOR
        .credential_refreshing
        .store(false, Ordering::Release);
    SUPERVISOR.start_retry_attempts.store(0, Ordering::Release);
    SUPERVISOR.start_retry_since.store(0, Ordering::Release);
    SUPERVISOR.start_retry_next_at.store(0, Ordering::Release);
    SUPERVISOR.start_requested.store(true, Ordering::Release);
    let result = start_once(true).await;
    if result.as_ref().is_ok_and(retryable_start_status) {
        spawn_start_retry();
    }
    result
}

pub(in crate::remote) async fn start_once(
    replace_existing: bool,
) -> Result<RemoteStatus, crate::AppError> {
    let epoch = SUPERVISOR.access.current();
    tokio::select! {
        biased;
        _ = SUPERVISOR.access.cancelled(epoch) => Ok(empty()),
        result = start_generation(replace_existing, epoch) => result,
    }
}

pub(in crate::remote) async fn start_generation(
    replace_existing: bool,
    epoch: u64,
) -> Result<RemoteStatus, crate::AppError> {
    crate::remote_host::attach_events();
    let _start_guard = SUPERVISOR.start_lock.lock().await;
    if !SUPERVISOR.start_requested.load(Ordering::Acquire)
        || SUPERVISOR.suspended.load(Ordering::Acquire)
    {
        return Ok(empty());
    }
    if !replace_existing {
        let current = status();
        if runtime_active(&current) {
            return Ok(current);
        }
    }
    *SUPERVISOR.last_error_code.lock().unwrap() = Some("connecting".to_string());

    // A remote/server failure here (offline, revoked, HTTP error) is not a
    // program fault — surface it as a localized, not-running status instead of
    // throwing a raw transport string at the UI. Local failures (NKey, disk)
    // keep propagating as `Err`.
    let (creds, pairing_code, pairing_code_expires_at) = match establish().await {
        Ok(value) => value,
        Err(error) => {
            return start_failure(error);
        }
    };
    if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
        return Ok(empty());
    }
    let connected_nats = match connect_nats(&creds, true).await {
        Ok(connection) => connection,
        Err(error) => {
            return start_failure(error);
        }
    };
    let client = connected_nats.client;
    let nats_health = connected_nats.health;
    if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
        return Ok(empty());
    }
    let Some(shared) = SUPERVISOR
        .access
        .commit(epoch, || -> Result<_, crate::AppError> {
            let shared = shared_runtime(&creds.pair_id, pairing_code.is_none(), false);
            if shared.pairing_confirmed.load(Ordering::Acquire) {
                pairing::save_creds(&creds)?;
            }
            Ok(shared)
        })
    else {
        return Ok(empty());
    };
    let shared = shared?;
    let pairing_confirmed = shared.pairing_confirmed.clone();
    let desktop_public_key = pairing::public_key(&creds)?;
    let bridge_instance_id = shared.bridge_instance_id.clone();
    let generation_id = shared.next_generation_id.fetch_add(1, Ordering::AcqRel);
    let pair_id = creds.pair_id.clone();

    // Command-id dedup cache lives OUTSIDE the command loop: credential
    // refresh swaps the loop every JWT TTL, and a cache tied to the loop would
    // be wiped each swap — retrying clients would re-execute commands (a
    // retried prompt = a duplicated user message + run).
    let reply_slots = shared.reply_slots.clone();
    let handshake_state = shared
        .handshake
        .lock()
        .unwrap()
        .get_or_insert_with(|| {
            commands::HandshakeState::new(
                creds.clone(),
                pairing_confirmed.clone(),
                bridge_instance_id.clone(),
            )
            .with_access(epoch)
        })
        .clone();

    let transport =
        match build_transport(&client, &pair_id, &handshake_state, reply_slots.clone()).await {
            Ok(transport) => transport,
            Err(error) => return start_failure(error),
        };
    let TransportTasks {
        event_tx,
        event_task,
        cmd_task,
        transfer_task,
        heartbeat_task,
        mut candidate_tasks,
    } = transport;
    let refresh_task = spawn_credential_refresh(
        pair_id.clone(),
        reply_slots,
        pairing_confirmed.clone(),
        handshake_state.clone(),
    );
    candidate_tasks.track(&refresh_task);
    // The browser client is a test-environment-only validation surface. Keep
    // the NATS/mobile bridge available in every environment, but never expose
    // the unauthenticated local HTTP listener in production or custom envs.
    let web_enabled = web_client_enabled();
    let (web_task, web_url, web_lan_url) = if web_enabled {
        match bind_web_listener().await {
            Ok(listener) => (
                Some(spawn_web_server(listener)),
                Some(format!("http://localhost:{WEB_PORT}")),
                lan_ip().map(|ip| format!("http://{ip}:{WEB_PORT}")),
            ),
            Err(error) => {
                eprintln!("remote: web client bind failed [LC002]: {error}");
                (None, None, None)
            }
        }
    } else {
        (None, None, None)
    };
    // A failed web bind is non-fatal (the bridge still runs) and is retried
    // silently before the UI is asked to intervene.
    let web_bind_failed = web_enabled && web_task.is_none();
    if let Some(task) = &web_task {
        candidate_tasks.track(task);
    }

    let status = RemoteStatus {
        phase: RemotePhase::Ready,
        reason: None,
        recovery: None,
        nats_url: creds.nats_url.clone(),
        pair_id: pair_id.clone(),
        pairing_code: pairing_code.clone(),
        pairing_code_expires_at,
        desktop_id: creds.desktop_id.clone(),
        desktop_public_key: desktop_public_key.clone(),
        web_url: web_url.clone(),
        web_lan_url: web_lan_url.clone(),
        agent_available: host().agent_available(),
        warning_code: web_bind_failed.then(|| "web_bind".to_string()),
    };
    // Keep the previous generation serving until every readiness prerequisite
    // above is complete. Replacing the pointer is the hand-off point; only
    // after it do we cancel the old generation.
    let previous = SUPERVISOR.access.commit(epoch, || {
        SUPERVISOR.state.lock().unwrap().replace(RemoteState {
            security: handshake_state.secure.clone(),
            generation_id,
            client,
            nats_health,
            nats_url: creds.nats_url,
            pair_id,
            desktop_id: creds.desktop_id,
            desktop_public_key,
            bridge_instance_id: bridge_instance_id.clone(),
            event_tx,
            drop_counters: shared.drop_counters.clone(),
            event_task,
            cmd_task,
            transfer_task,
            heartbeat_task,
            refresh_task,
            web_task,
            web_url,
            web_lan_url,
            pairing_code,
            pairing_code_expires_at,
            pairing_confirmed,
        })
    });
    let Some(previous) = previous else {
        return Ok(empty());
    };
    candidate_tasks.installed();
    if let Some(previous) = previous {
        abort_generation(previous);
    }
    SUPERVISOR
        .access
        .commit(epoch, || {
            *SUPERVISOR.last_error_code.lock().unwrap() = None;
            if let Some(line) = START_EPISODE.recovered() {
                eprintln!("{line}");
            }
            SUPERVISOR.start_retry_attempts.store(0, Ordering::Release);
            SUPERVISOR.start_retry_since.store(0, Ordering::Release);
            SUPERVISOR.start_retry_next_at.store(0, Ordering::Release);
            spawn_runtime_supervisor(bridge_instance_id, generation_id);
            if web_bind_failed {
                spawn_web_reconnect(status.pair_id.clone());
            }
            Ok(status)
        })
        .unwrap_or_else(|| Ok(empty()))
}

/// Keep retrying a categorized startup failure on Tauri's process-lifetime
/// runtime. This covers launching while offline and a network transition during
/// the first connect. Runtime disconnects after a successful start continue to
/// use async-nats plus the credential-refresh health swap below.
pub(in crate::remote) fn spawn_start_retry() {
    #[cfg(test)]
    return;

    #[cfg(not(test))]
    {
        if SUPERVISOR.start_retry_running.swap(true, Ordering::AcqRel) {
            return;
        }
        SUPERVISOR.spawn(async {
            let mut delay = std::time::Duration::from_secs(1);
            SUPERVISOR
                .start_retry_since
                .compare_exchange(0, unix_millis(), Ordering::AcqRel, Ordering::Acquire)
                .ok();
            loop {
                let jitter = 0.8 + rand::random::<f64>() * 0.4;
                let jittered =
                    std::time::Duration::from_secs_f64((delay.as_secs_f64() * jitter).min(30.0));
                SUPERVISOR.start_retry_next_at.store(
                    unix_millis().saturating_add(jittered.as_millis() as u64),
                    Ordering::Release,
                );
                tokio::time::sleep(jittered).await;
                SUPERVISOR
                    .start_retry_attempts
                    .fetch_add(1, Ordering::AcqRel);
                if !SUPERVISOR.start_requested.load(Ordering::Acquire)
                    || matches!(status().phase, RemotePhase::Ready)
                {
                    break;
                }
                match start_once(false).await {
                    Ok(status) if matches!(status.phase, RemotePhase::Ready) => break,
                    Ok(status) if retryable_start_status(&status) => {
                        delay = delay
                            .saturating_mul(2)
                            .min(std::time::Duration::from_secs(30));
                    }
                    Ok(_) | Err(_) => break,
                }
            }
            SUPERVISOR.start_retry_next_at.store(0, Ordering::Release);
            SUPERVISOR
                .start_retry_running
                .store(false, Ordering::Release);
            // Close the tiny stop→start race: a new start may have requested
            // recovery after this worker decided to exit but before it released
            // the singleton flag. Re-arm from the latest status in that case.
            let latest = status();
            if SUPERVISOR.start_requested.load(Ordering::Acquire) && retryable_start_status(&latest)
            {
                spawn_start_retry();
            }
        });
    }
}

/// Rebuild the whole bridge generation when a subscription task itself dies.
/// Ordinary NATS disconnects are handled inside the task; reaching this path
/// means the task exited or panicked and cannot resubscribe on its own.
#[cfg_attr(test, allow(dead_code))]
pub(in crate::remote) fn spawn_runtime_reconnect() {
    #[cfg(test)]
    return;

    #[cfg(not(test))]
    {
        if SUPERVISOR
            .runtime_reconnect_running
            .swap(true, Ordering::AcqRel)
        {
            return;
        }
        let attempt = record_runtime_failure(unix_millis());
        eprintln!(
            "remote: critical task stopped [RT001]; rebuilding generation ({attempt}/{MAX_RUNTIME_RECONNECT_ATTEMPTS})"
        );
        SUPERVISOR.spawn(async move {
            let result = start_once(true).await;
            match &result {
                Ok(status) if retryable_start_status(status) => spawn_start_retry(),
                Err(error) => {
                    eprintln!("remote: automatic bridge reconnect failed [RT001]: {error}");
                    *SUPERVISOR.last_error_code.lock().unwrap() =
                        Some("reconnect_required".to_string());
                }
                _ => {}
            }
            SUPERVISOR
                .runtime_reconnect_running
                .store(false, Ordering::Release);
        });
    }
}

pub(in crate::remote) fn record_runtime_failure(now: u64) -> u8 {
    let started = SUPERVISOR
        .runtime_failure_window_started
        .load(Ordering::Acquire);
    if started == 0 || now.saturating_sub(started) > RUNTIME_FAILURE_WINDOW_MS {
        SUPERVISOR
            .runtime_failure_window_started
            .store(now, Ordering::Release);
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
    }
    SUPERVISOR
        .runtime_reconnect_attempts
        .fetch_add(1, Ordering::AcqRel)
        + 1
}

/// Watch the active bridge from the process runtime, independently of any UI
/// status polling. This is essential for Remote clients: a phone must recover
/// even when the desktop window is hidden or no frontend has mounted yet.
#[cfg(not(test))]
pub(in crate::remote) struct GenerationWatch {
    bridge_instance_id: String,
    generation_id: u64,
}

#[cfg(not(test))]
impl GenerationWatch {
    async fn run(self) {
        let bridge_instance_id = self.bridge_instance_id;
        let generation_id = self.generation_id;
        let mut healthy_secs = 0u8;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
                return;
            }
            let unhealthy = {
                let guard = SUPERVISOR.state.lock().unwrap();
                let Some(state) = guard.as_ref().filter(|state| {
                    state.bridge_instance_id == bridge_instance_id
                        && state.generation_id == generation_id
                }) else {
                    return;
                };
                if state.nats_health.is_terminal() {
                    return;
                }
                state.nats_health.needs_reconnect()
                    || state.cmd_task.is_finished()
                    || state.transfer_task.is_finished()
                    || state.event_task.is_finished()
                    || state.heartbeat_task.is_finished()
                    || state.refresh_task.is_finished()
            };
            if !unhealthy {
                healthy_secs = healthy_secs.saturating_add(1);
                if healthy_secs >= RUNTIME_HEALTHY_RESET_SECS {
                    SUPERVISOR
                        .runtime_reconnect_attempts
                        .store(0, Ordering::Release);
                    SUPERVISOR
                        .runtime_failure_window_started
                        .store(0, Ordering::Release);
                }
                continue;
            }
            if SUPERVISOR.runtime_reconnect_running.load(Ordering::Acquire) {
                continue;
            }
            if SUPERVISOR
                .runtime_reconnect_attempts
                .load(Ordering::Acquire)
                >= MAX_RUNTIME_RECONNECT_ATTEMPTS
            {
                *SUPERVISOR.last_error_code.lock().unwrap() =
                    Some("reconnect_required".to_string());
                return;
            }
            spawn_runtime_reconnect();
            return;
        }
    }
}

pub(in crate::remote) fn spawn_runtime_supervisor(bridge_instance_id: String, generation_id: u64) {
    #[cfg(test)]
    {
        let _ = (bridge_instance_id, generation_id);
    }

    #[cfg(not(test))]
    SUPERVISOR.spawn(
        GenerationWatch {
            bridge_instance_id,
            generation_id,
        }
        .run(),
    );
}

/// Retry only the optional local Web listener. A busy port must not tear down
/// the healthy phone/NATS bridge; after a bounded retry budget the UI offers a
/// full reconnect button, which retries the listener with a fresh generation.
pub(in crate::remote) fn spawn_web_reconnect(pair_id: String) {
    #[cfg(test)]
    {
        let _ = pair_id;
    }

    #[cfg(not(test))]
    {
        if SUPERVISOR
            .web_reconnect_running
            .swap(true, Ordering::AcqRel)
        {
            return;
        }
        let attempt = SUPERVISOR
            .web_reconnect_attempts
            .fetch_add(1, Ordering::AcqRel)
            + 1;
        eprintln!(
            "remote: local web listener unavailable; retrying ({attempt}/{MAX_WEB_RECONNECT_ATTEMPTS})"
        );
        SUPERVISOR.spawn(async move {
            match bind_web_listener().await {
                Ok(listener) => {
                    let web_url = Some(format!("http://localhost:{WEB_PORT}"));
                    let web_lan_url = lan_ip().map(|ip| format!("http://{ip}:{WEB_PORT}"));
                    let web_task = spawn_web_server(listener);
                    let mut guard = SUPERVISOR.state.lock().unwrap();
                    if let Some(state) = guard.as_mut().filter(|state| state.pair_id == pair_id) {
                        if let Some(previous) = state.web_task.replace(web_task) {
                            previous.abort();
                        }
                        state.web_url = web_url;
                        state.web_lan_url = web_lan_url;
                        SUPERVISOR
                            .web_reconnect_attempts
                            .store(0, Ordering::Release);
                        eprintln!("remote: local web listener reconnected [LC002]");
                    } else {
                        web_task.abort();
                    }
                }
                Err(error) => {
                    eprintln!("remote: local web listener reconnect failed [LC002]: {error}")
                }
            }
            SUPERVISOR
                .web_reconnect_running
                .store(false, Ordering::Release);
        });
    }
}

/// Resolve a usable credential: refresh the persisted pairing, or — if it was
/// revoked server-side — drop it and mint a fresh pairing code. Pure control
/// plane; the NATS connect happens in [`start`] so its failure can be reported
/// the same way.
pub(in crate::remote) async fn establish(
) -> Result<(pairing::PairingCreds, Option<String>, Option<i64>), crate::AppError> {
    match pairing::load_creds() {
        Some(creds) if creds.handshake_version != 2 || creds.secure.is_none() => {
            // Credentials created before the signed mutual handshake have no
            // QR-bound peer identity. They cannot be upgraded safely in place.
            eprintln!("remote: replacing legacy pairing [PA003]");
            pairing::clear_creds()?;
            let (creds, code, exp) = pairing::create_pairing().await?;
            Ok((creds, Some(code), exp))
        }
        Some(creds) => match pairing::refresh_bridge_jwt(creds).await {
            Ok(creds) => Ok((creds, None, None)),
            Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                // A web client can revoke this pairing. Forget its unusable
                // local credential and immediately issue a replacement code
                // instead of leaving the GUI permanently stuck on startup.
                eprintln!("remote: persisted pairing was revoked [PA001]; creating a new pairing");
                pairing::clear_creds()?;
                let (creds, code, exp) = pairing::create_pairing().await?;
                Ok((creds, Some(code), exp))
            }
            Err(error) => Err(error),
        },
        None => {
            let (creds, code, exp) = pairing::create_pairing().await?;
            Ok((creds, Some(code), exp))
        }
    }
}

/// Turn a start-time failure into either a localized not-running status (when
/// the cause is a categorized remote/network/server error) or an `Err` for an
/// uncategorized local fault. Records the code so a later [`status`] poll stays
/// consistent with the value returned here.
pub(in crate::remote) fn start_failure(
    error: crate::AppError,
) -> Result<RemoteStatus, crate::AppError> {
    match pairing::error_code(&error) {
        Some(code) => {
            if let Some(line) = START_EPISODE.record(code, &error) {
                eprintln!("{line}");
            }
            *SUPERVISOR.last_error_code.lock().unwrap() = Some(code.to_string());
            Ok(RemoteStatus {
                phase: match code {
                    "network" | "server" => RemotePhase::Reconnecting,
                    "revoked" => RemotePhase::Revoked,
                    _ => RemotePhase::Failed,
                },
                reason: Some(match code {
                    "network" => RemoteFailureReason::Network,
                    "revoked" => RemoteFailureReason::CredentialRevoked,
                    "account_authorization" => RemoteFailureReason::AccountAuthorization,
                    "service_authorization" => RemoteFailureReason::ServiceAuthorization,
                    "server" => RemoteFailureReason::RemoteServer,
                    _ => RemoteFailureReason::Local,
                }),
                recovery: matches!(code, "network" | "server").then(|| RecoveryProgress {
                    attempt: SUPERVISOR.start_retry_attempts.load(Ordering::Acquire),
                    max_attempts: None,
                    since: SUPERVISOR.start_retry_since.load(Ordering::Acquire),
                    next_retry_at: None,
                }),
                ..empty()
            })
        }
        None => Err(error),
    }
}

/// NATS remains a direct TCP connection, but production requires verified TLS
/// even when the endpoint uses the nats:// scheme. E2EE protects content from
/// the broker; TLS independently protects credentials and transport metadata.
pub(in crate::remote) async fn connect_nats(
    creds: &pairing::PairingCreds,
    credential_was_refreshed: bool,
) -> Result<ConnectedNats, crate::AppError> {
    let key_pair = std::sync::Arc::new(
        nkeys::KeyPair::from_seed(&creds.nkey_seed)
            .map_err(|error| crate::AppError::Message(format!("Invalid desktop NKey: {error}")))?,
    );
    let health = Arc::new(NatsHealth::default());
    health
        .credential_was_refreshed
        .store(credential_was_refreshed, Ordering::Release);
    health
        .credential_expires_at
        .store(creds.jwt_expires_at.max(0) as u64, Ordering::Release);
    let event_health = health.clone();
    let options = async_nats::ConnectOptions::with_jwt(creds.user_jwt.clone(), move |nonce| {
        let key_pair = key_pair.clone();
        async move { key_pair.sign(&nonce).map_err(async_nats::AuthError::new) }
    })
    .custom_inbox_prefix(format!("p.{}.rep.{}", creds.pair_id, creds.desktop_id))
    .reconnect_delay_callback(|attempt| reconnect_delay(attempt, rand::random::<f64>()))
    .event_callback(move |event| {
        let health = event_health.clone();
        async move { health.handle_event(&event) }
    });
    // Unit tests use an in-process, plaintext fake broker. There is no runtime
    // configuration switch that permits a production TLS downgrade.
    #[cfg(not(test))]
    let options = options.require_tls(true);
    let client = options.connect(&creds.nats_url).await.map_err(|error| {
        let message = format!("Failed to connect to NATS: {error}");
        classify_nats_connect_error(error.kind(), message)
    })?;
    Ok(ConnectedNats { client, health })
}

pub(in crate::remote) fn classify_nats_connect_error(
    kind: async_nats::ConnectErrorKind,
    message: String,
) -> crate::AppError {
    match kind {
        async_nats::ConnectErrorKind::Authentication
        | async_nats::ConnectErrorKind::AuthorizationViolation => {
            crate::AppError::RemoteAuthorization(message)
        }
        async_nats::ConnectErrorKind::Dns
        | async_nats::ConnectErrorKind::TimedOut
        | async_nats::ConnectErrorKind::Io
        | async_nats::ConnectErrorKind::Tls
        | async_nats::ConnectErrorKind::MaxReconnects => crate::AppError::RemoteTransport(message),
        async_nats::ConnectErrorKind::ServerParse => crate::AppError::Message(message),
    }
}

/// Drop the persisted pairing and stop the bridge (the desktop "unpair").
pub(in crate::remote) fn spawn_revoke_cleanup() {
    #[cfg(not(test))]
    {
        // Compensation is independent of remote access and never opens a socket.
        // One Desktop-process worker retries persisted network failures.
        static RUNNING: AtomicBool = AtomicBool::new(false);
        if RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
        crate::runtime::spawn(async {
            loop {
                if let Err(error) = pairing::retry_pending_revokes().await {
                    if let Some(line) = START_EPISODE.record("local", error) {
                        eprintln!("{line}");
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    }
}
