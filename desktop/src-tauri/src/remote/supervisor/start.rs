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
        // Ask the runtime map, not the projection: a start retry runs with nothing
        // installed and `status()` reporting that retry itself as `Reconnecting`.
        if runtime_active(&current) && !SUPERVISOR.state.lock().unwrap().is_empty() {
            return Ok(current);
        }
    }
    *SUPERVISOR.last_error_code.lock().unwrap() = Some("connecting".to_string());

    // The persisted pointer is the authority when the process starts; an
    // in-memory pointer that disagrees can only be older than the file, because
    // every claim writes through. Never clear it here: `None` in the book means
    // no device has claimed control, not that the current one lost it.
    SUPERVISOR.restore_active(pairing::active_slot());

    // A remote/server failure here (offline, revoked, HTTP error) is not a
    // program fault — surface it as a localized, not-running status instead of
    // throwing a raw transport string at the UI. Local failures (NKey, disk)
    // keep propagating as `Err`.
    let plans = match establish_slots().await {
        Ok(plans) => plans,
        Err(error) => return start_failure(error),
    };
    if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
        return Ok(empty());
    }

    // One device needs no arbitration: it is the machine's controller by
    // definition, and asking its user to "take over" from nobody would be
    // theatre. Several devices do — then only an explicit claim moves the
    // pointer, and a reconnect never does.
    if SUPERVISOR.active_pair().is_none() && plans.len() == 1 && plans[0].pairing_code.is_none() {
        SUPERVISOR.claim_control(&plans[0].creds.pair_id);
    }

    // The device whose invitation this start minted, if any: the answer must
    // describe *that* device, because its QR code is what the user is looking
    // at. Without it the answer would name whichever device happens to hold the
    // control pointer — and anything that pairs what a start returned (the
    // host-bridge fixture does exactly that) would point at the wrong pairing.
    let minted = plans
        .iter()
        .find(|plan| plan.pairing_code.is_some())
        .map(|plan| plan.creds.pair_id.clone());

    // One device failing to come up must not take the others down: each slot
    // has its own connection and its own credential. The start only reports a
    // failure when *no* device could be started, which is the case a user can
    // act on.
    let mut started = 0usize;
    let mut failure = None;
    for (index, plan) in plans.into_iter().enumerate() {
        // The local browser validation surface is one listener for the whole
        // process, so exactly one slot owns it.
        match start_slot(plan, epoch, replace_existing, index == 0).await {
            Ok(()) => started += 1,
            Err(error) => failure = Some(error),
        }
        if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
            return Ok(empty());
        }
    }
    match (started, failure) {
        (0, Some(error)) => start_failure(error),
        (0, None) => Ok(empty()),
        _ => {
            // The bind result is a fact of this start, not something a later
            // poll can reconstruct: the first status must carry it so the page
            // can say why the browser surface is missing. Later polls report
            // only what the retry budget has settled into.
            let web_bind_failed = web_client_enabled()
                && !SUPERVISOR
                    .state
                    .lock()
                    .unwrap()
                    .values()
                    .any(|state| state.web_task.is_some());
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
                })
                .unwrap_or(());
            let mut status = status_focused(minted.as_deref());
            if web_bind_failed {
                status.warning_code = Some("web_bind".to_string());
            }
            Ok(status)
        }
    }
}

/// One device's plan for this start: its credentials, plus the invitation the
/// user still has to scan when this device has not been claimed yet.
pub(in crate::remote) struct SlotPlan {
    pub(in crate::remote) creds: pairing::PairingCreds,
    pub(in crate::remote) pairing_code: Option<String>,
    pub(in crate::remote) pairing_code_expires_at: Option<i64>,
}

/// Pair one more device, then connect it so the code on screen works at once.
///
/// Only the new slot is started: the devices already serving are not touched,
/// because pairing a device has nothing to do with their connections. The
/// platform owns the cap, so `pairing_limit_reached` arrives here as an error
/// and is surfaced unchanged.
pub async fn add_device() -> Result<RemoteStatus, crate::AppError> {
    spawn_revoke_cleanup();
    let (creds, code, expires_at) = pairing::create_pairing(true).await?;
    let plan = SlotPlan {
        creds,
        pairing_code: Some(code),
        pairing_code_expires_at: expires_at,
    };
    if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
        SUPERVISOR.start_requested.store(true, Ordering::Release);
    }
    let epoch = SUPERVISOR.access.current();
    let added = plan.creds.pair_id.clone();
    let _start_guard = SUPERVISOR.start_lock.lock().await;
    start_slot(plan, epoch, true, false).await?;
    Ok(status_focused(Some(&added)))
}

/// Mint an invitation for a device that is not paired yet.
///
/// This is what `future remote devices add` asks for. Adding beside existing
/// devices is the ordinary case; with none left the machine is unpaired, and
/// *replace* is then the same request — plus the cleanup it needs.
///
/// That cleanup is not cosmetic. The pairing book can be emptied from another
/// process (`future remote devices remove --all`), while this one still holds the
/// bridges of the devices that were just removed. `create_pairing` gives a known
/// desktop back the *same* pair id, so minting the replacement without stopping
/// them first would leave two bridges on one pair's subjects, with the stale one
/// holding a credential the platform has already refused. `re_pair` — unpair,
/// settle the queued revokes, start afresh — is exactly that cleanup, and with an
/// empty book its revocation step has nothing left to revoke.
pub async fn pair_new_device() -> Result<RemoteStatus, crate::AppError> {
    if pairing::load_slots().is_empty() {
        return re_pair().await;
    }
    add_device().await
}

/// Resolve every paired device into a plan, dropping the ones the platform has
/// retired and minting an invitation when the machine has no device left.
pub(in crate::remote) async fn establish_slots() -> Result<Vec<SlotPlan>, crate::AppError> {
    let mut plans = Vec::new();
    for creds in pairing::load_slots() {
        let pair_id = creds.pair_id.clone();
        if creds.handshake_version != 2 || creds.secure.is_none() {
            // Credentials created before the signed mutual handshake have no
            // QR-bound peer identity. They cannot be upgraded safely in place.
            eprintln!("remote: replacing legacy pairing [PA003]");
            forget_slot(&pair_id)?;
            continue;
        }
        match pairing::refresh_bridge_jwt(creds).await {
            Ok(creds) => plans.push(SlotPlan {
                creds,
                pairing_code: None,
                pairing_code_expires_at: None,
            }),
            Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                // The platform no longer honours this device's credential —
                // someone revoked it. It is not this machine's device any more,
                // and leaving it on disk would make every start retry a
                // credential that can never succeed.
                eprintln!("remote: a pairing was revoked [PA001]; forgetting that device");
                forget_slot(&pair_id)?;
            }
            Err(error) => return Err(error),
        }
    }
    if plans.is_empty() {
        // Nothing left to serve: mint one invitation, exactly as a
        // single-device install always did. Replacing is right here because
        // there is nothing to keep.
        let (creds, code, exp) = pairing::create_pairing(false).await?;
        plans.push(SlotPlan {
            creds,
            pairing_code: Some(code),
            pairing_code_expires_at: exp,
        });
    }
    Ok(plans)
}

/// Drop a device the platform no longer honours, from disk and from memory.
pub(in crate::remote) fn forget_slot(pair_id: &str) -> Result<(), crate::AppError> {
    // Local disk faults propagate: a credential that cannot be forgotten will
    // be retried on every start, and hiding that behind a log line turns a
    // read-only home directory into a machine that silently never recovers.
    pairing::remove_slot(pair_id)?;
    // The pointer must not keep naming a device this machine no longer has, or
    // the UI would report "in control" for a credential that cannot connect.
    SUPERVISOR.release_control(pair_id);
    promote_a_lone_device();
    stop_slot(pair_id);
    Ok(())
}

/// After a removal: if exactly one device is left, it controls.
///
/// Not a promotion rule for several devices — that stays the user's call — but
/// the single-device case can only have one answer, and leaving it unclaimed
/// would ask the remaining device to take over from itself.
pub(in crate::remote) fn promote_a_lone_device() {
    if SUPERVISOR.active_pair().is_some() {
        return;
    }
    let slots = pairing::load_slots();
    if let [only] = slots.as_slice() {
        SUPERVISOR.claim_control(&only.pair_id);
    }
}

/// Start (or replace) one device's bridge.
///
/// `owns_local_web` marks the slot that binds the process-wide browser
/// validation listener; every other slot must not try, because the port is
/// singular and a second bind would fail and be reported as a bridge warning.
pub(in crate::remote) async fn start_slot(
    plan: SlotPlan,
    epoch: u64,
    replace_existing: bool,
    owns_local_web: bool,
) -> Result<(), crate::AppError> {
    let SlotPlan {
        creds,
        pairing_code,
        pairing_code_expires_at,
    } = plan;
    let pair_id = creds.pair_id.clone();
    if !replace_existing && SUPERVISOR.state.lock().unwrap().contains_key(&pair_id) {
        return Ok(());
    }
    let connected_nats = match connect_nats(&creds, true).await {
        Ok(connection) => connection,
        Err(error) => return Err(error),
    };
    let client = connected_nats.client;
    let nats_health = connected_nats.health;
    if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
        return Ok(());
    }
    let Some(shared) = SUPERVISOR
        .access
        .commit(epoch, || -> Result<_, crate::AppError> {
            let shared = shared_runtime(&creds, pairing_code.is_none(), false);
            if shared.pairing_confirmed.load(Ordering::Acquire) {
                pairing::save_creds(&creds)?;
            }
            Ok(shared)
        })
    else {
        return Ok(());
    };
    let shared = shared?;
    let pairing_confirmed = shared.pairing_confirmed.clone();
    let desktop_public_key = pairing::public_key(&creds)?;
    let bridge_instance_id = shared.bridge_instance_id.clone();
    let generation_id = shared.next_generation_id.fetch_add(1, Ordering::AcqRel);

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
            Err(error) => return Err(error),
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
    let web_enabled = web_client_enabled() && owns_local_web;
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

    // Keep the previous generation serving until every readiness prerequisite
    // above is complete. Replacing the map entry is the hand-off point; only
    // after it do we cancel the old generation for *this* device.
    let previous = SUPERVISOR.access.commit(epoch, || {
        SUPERVISOR.state.lock().unwrap().insert(
            pair_id.clone(),
            RemoteState {
                security: handshake_state.secure.clone(),
                generation_id,
                client,
                nats_health,
                nats_url: creds.nats_url,
                pair_id: pair_id.clone(),
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
            },
        )
    });
    SUPERVISOR.remember_device(&pair_id);
    let Some(previous) = previous else {
        return Ok(());
    };
    candidate_tasks.installed();
    if let Some(previous) = previous {
        abort_generation(previous);
    }
    spawn_runtime_supervisor(pair_id.clone(), generation_id);
    if web_bind_failed {
        spawn_web_reconnect(pair_id);
    }
    Ok(())
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

/// Rebuild one device's bridge generation when a subscription task itself dies.
/// Ordinary NATS disconnects are handled inside the task; reaching this path
/// means the task exited or panicked and cannot resubscribe on its own.
///
/// Scoped to one `pair_id` on purpose: the other devices on this machine have
/// their own healthy connections, and rebuilding them would blink every client
/// because one of them stumbled.
#[cfg_attr(test, allow(dead_code))]
pub(in crate::remote) fn spawn_runtime_reconnect(pair_id: String) {
    #[cfg(test)]
    {
        let _ = pair_id;
    }

    #[cfg(not(test))]
    {
        let attempt = record_runtime_failure(unix_millis());
        eprintln!(
            "remote: critical task stopped [RT001]; rebuilding generation ({attempt}/{MAX_RUNTIME_RECONNECT_ATTEMPTS})"
        );
        if attempt > MAX_RUNTIME_RECONNECT_ATTEMPTS {
            *SUPERVISOR.last_error_code.lock().unwrap() = Some("reconnect_required".to_string());
            return;
        }
        SUPERVISOR.spawn(async move {
            let _start_guard = SUPERVISOR.start_lock.lock().await;
            match restart_slot(&pair_id).await {
                // A retryable refusal is not a dead end: hand it to the start
                // retry worker, exactly as a failed `start_once` does. Its own
                // attempt is what records the episode and backs off.
                Ok(status) if retryable_start_status(&status) => spawn_start_retry(),
                Ok(_) => {}
                Err(error) => {
                    eprintln!("remote: automatic bridge reconnect failed [RT001]: {error}");
                    *SUPERVISOR.last_error_code.lock().unwrap() =
                        Some("reconnect_required".to_string());
                }
            }
        });
    }
}

/// Refresh and reinstall one device's runtime, without touching the others.
///
/// Errors are funneled through `start_failure` — the same conversion a fresh
/// `start_once` applies — so a refusal keeps its classification (episode,
/// `last_error_code`, retryability) instead of turning into a bare string. A
/// reconnect that the platform now refuses is not a different kind of failure
/// just because this device was already up.
#[cfg(not(test))]
async fn restart_slot(pair_id: &str) -> Result<RemoteStatus, crate::AppError> {
    let Some(creds) = pairing::load_slots()
        .into_iter()
        .find(|slot| slot.pair_id == pair_id)
    else {
        // The device is gone; the runtime must not outlive its credential.
        stop_slot(pair_id);
        return Ok(empty());
    };
    if creds.handshake_version != 2 || creds.secure.is_none() {
        forget_slot(pair_id)?;
        return Ok(empty());
    }
    let creds = match pairing::refresh_bridge_jwt(creds).await {
        Ok(creds) => creds,
        Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
            forget_slot(pair_id)?;
            return Ok(empty());
        }
        Err(error) => return start_failure(error),
    };
    let plan = SlotPlan {
        creds,
        pairing_code: None,
        pairing_code_expires_at: None,
    };
    let epoch = SUPERVISOR.access.current();
    match start_slot(plan, epoch, true, false).await {
        Ok(()) => Ok(status_focused(Some(pair_id))),
        Err(error) => start_failure(error),
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

/// Watch one device's bridge from the process runtime, independently of any UI
/// status polling. This is essential for Remote clients: a phone must recover
/// even when the desktop window is hidden or no frontend has mounted yet.
#[cfg(not(test))]
pub(in crate::remote) struct GenerationWatch {
    pair_id: String,
    generation_id: u64,
}

#[cfg(not(test))]
impl GenerationWatch {
    async fn run(self) {
        let pair_id = self.pair_id;
        let generation_id = self.generation_id;
        let mut healthy_secs = 0u8;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if !SUPERVISOR.start_requested.load(Ordering::Acquire) {
                return;
            }
            let unhealthy = {
                let guard = SUPERVISOR.state.lock().unwrap();
                let Some(state) = guard
                    .get(&pair_id)
                    .filter(|state| state.generation_id == generation_id)
                else {
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
            if SUPERVISOR
                .runtime_reconnect_attempts
                .load(Ordering::Acquire)
                >= MAX_RUNTIME_RECONNECT_ATTEMPTS
            {
                *SUPERVISOR.last_error_code.lock().unwrap() =
                    Some("reconnect_required".to_string());
                return;
            }
            spawn_runtime_reconnect(pair_id.clone());
            return;
        }
    }
}

pub(in crate::remote) fn spawn_runtime_supervisor(pair_id: String, generation_id: u64) {
    #[cfg(test)]
    {
        let _ = (pair_id, generation_id);
    }

    #[cfg(not(test))]
    SUPERVISOR.spawn(
        GenerationWatch {
            pair_id,
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
                    if let Some(state) = guard.get_mut(&pair_id) {
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
    // Load the platform trust store here, once, rather than letting async-nats
    // do it on every connection: that read fails intermittently on macOS and
    // takes the whole connection (and therefore the credential rotation) down
    // with it. A failed read now degrades to the last snapshot; `None` keeps
    // async-nats' own platform load.
    let options = match trust::root_bundle() {
        Some(bundle) => options.add_root_certificates(bundle),
        None => options,
    };
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
