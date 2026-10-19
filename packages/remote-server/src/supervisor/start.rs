//! Startup and recovery of independently paired controlled-endpoint slots.
use super::transport::TransportTasks;
use super::*;
use crate::{health::unix_millis, pairing, types::*};
pub struct SlotPlan {
    pub creds: PairingCreds,
    pub pairing_code: Option<String>,
    pub pairing_code_expires_at: Option<i64>,
}
impl Supervisor {
    pub async fn start(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(crate::Error::Message("remote_server_stopped".into()));
        }
        self.spawn_revoke_cleanup();
        let epoch = self.access.current();
        let admitted = self.access.commit(epoch, || {
            if self.closed.load(Ordering::Acquire) {
                return false;
            }
            // An explicit user reconnect gets fresh automatic-reconnect budgets.
            self.runtime_reconnect_attempts.store(0, Ordering::Release);
            self.runtime_failure_window_started
                .store(0, Ordering::Release);
            self.web_reconnect_attempts.store(0, Ordering::Release);
            self.credential_refreshing.store(false, Ordering::Release);
            self.start_retry_attempts.store(0, Ordering::Release);
            self.start_retry_since.store(0, Ordering::Release);
            self.start_retry_next_at.store(0, Ordering::Release);
            self.start_requested.store(true, Ordering::Release);
            true
        });
        if admitted != Some(true) {
            return Ok(self.empty());
        }
        let result = self.start_once(true).await;
        if result.as_ref().is_ok_and(retryable_start_status) {
            self.spawn_start_retry();
        }
        result
    }

    pub async fn start_once(
        self: &Arc<Self>,
        replace_existing: bool,
    ) -> Result<RemoteStatus, crate::Error> {
        let epoch = self.access.current();
        tokio::select! {
            biased;
            _ = self.access.cancelled(epoch) => Ok(self.empty()),
            result = self.start_generation(replace_existing, epoch) => result,
        }
    }

    pub async fn start_generation(
        self: &Arc<Self>,
        replace_existing: bool,
        epoch: u64,
    ) -> Result<RemoteStatus, crate::Error> {
        self.executor.attach_events();
        let _start_guard = self.start_lock.lock().await;
        if !self.start_requested.load(Ordering::Acquire) || self.suspended.load(Ordering::Acquire) {
            return Ok(self.empty());
        }
        if !replace_existing {
            let current = self.status();
            // Ask the runtime map, not the projection: a start retry runs with nothing
            // installed and `self.status()` reporting that retry itself as `Reconnecting`.
            if runtime_active(&current) && !self.state.lock().unwrap().is_empty() {
                return Ok(current);
            }
        }
        *self.last_error_code.lock().unwrap() = Some("connecting".to_string());

        // The persisted pointer is the authority when the process starts; an
        // in-memory pointer that disagrees can only be older than the file, because
        // every claim writes through. Never clear it here: `None` in the book means
        // no device has claimed control, not that the current one lost it.
        self.restore_active(self.host.active_slot());

        // A remote/server failure here (offline, revoked, HTTP error) is not a
        // program fault — surface it as a localized, not-running status instead of
        // throwing a raw transport string at the UI. Local failures (NKey, disk)
        // keep propagating as `Err`.
        let plans = match self.establish_slots().await {
            Ok(plans) => plans,
            Err(error) => return self.start_failure(error),
        };
        if !self.start_requested.load(Ordering::Acquire) {
            return Ok(self.empty());
        }

        // One device needs no arbitration: it is the machine's controller by
        // definition, and asking its user to "take over" from nobody would be
        // theatre. Several devices do — then only an explicit claim moves the
        // pointer, and a reconnect never does.
        if self.active_pair().is_none() && plans.len() == 1 && plans[0].pairing_code.is_none() {
            self.claim_control(&plans[0].creds.pair_id);
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
            match self
                .start_slot(plan, epoch, replace_existing, index == 0)
                .await
            {
                Ok(()) => started += 1,
                Err(error) => failure = Some(error),
            }
            if !self.start_requested.load(Ordering::Acquire) {
                return Ok(self.empty());
            }
        }
        match (started, failure) {
            (0, Some(error)) => self.start_failure(error),
            (0, None) => Ok(self.empty()),
            _ => {
                // The bind result is a fact of this start, not something a later
                // poll can reconstruct: the first status must carry it so the page
                // can say why the browser surface is missing. Later polls report
                // only what the retry budget has settled into.
                let web_bind_failed = self.executor.validation_enabled()
                    && !self
                        .state
                        .lock()
                        .unwrap()
                        .values()
                        .any(|state| state.web_task.is_some());
                self.access
                    .commit(epoch, || {
                        *self.last_error_code.lock().unwrap() = None;
                        if let Some(line) = self.start_failures.recovered() {
                            eprintln!("{line}");
                        }
                        self.start_retry_attempts.store(0, Ordering::Release);
                        self.start_retry_since.store(0, Ordering::Release);
                        self.start_retry_next_at.store(0, Ordering::Release);
                    })
                    .unwrap_or(());
                let mut status = self.status_focused(minted.as_deref());
                if web_bind_failed {
                    status.warning_code = Some("web_bind".to_string());
                }
                Ok(status)
            }
        }
    }

    pub async fn add_device(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(crate::Error::Message("remote_server_stopped".into()));
        }
        let epoch = self.access.current();
        tokio::select! {
            biased;
            _ = self.access.cancelled(epoch) => Ok(self.empty()),
            result = self.add_device_generation(epoch) => result,
        }
    }
    async fn add_device_generation(
        self: &Arc<Self>,
        epoch: u64,
    ) -> Result<RemoteStatus, crate::Error> {
        self.spawn_revoke_cleanup();
        let (creds, code, expires_at) = self.host.invite(true).await?;
        let plan = SlotPlan {
            creds,
            pairing_code: Some(code),
            pairing_code_expires_at: expires_at,
        };
        if self
            .access
            .commit(epoch, || {
                self.start_requested.store(true, Ordering::Release)
            })
            .is_none()
        {
            return Ok(self.empty());
        }
        let added = plan.creds.pair_id.clone();
        let _start_guard = self.start_lock.lock().await;
        self.start_slot(plan, epoch, true, false).await?;
        Ok(self.status_focused(Some(&added)))
    }

    pub async fn pair_new_device(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        if self.host.load_slots().is_empty() {
            return self.re_pair().await;
        }
        self.add_device().await
    }

    pub async fn establish_slots(self: &Arc<Self>) -> Result<Vec<SlotPlan>, crate::Error> {
        let mut plans = Vec::new();
        for creds in self.host.load_slots() {
            let pair_id = creds.pair_id.clone();
            if creds.handshake_version != 2 || creds.secure.is_none() {
                // Credentials created before the signed mutual handshake have no
                // QR-bound peer identity. They cannot be upgraded safely in place.
                eprintln!("remote: replacing legacy pairing [PA003]");
                self.forget_slot(&pair_id)?;
                continue;
            }
            match self.host.refresh(creds).await {
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
                    self.forget_slot(&pair_id)?;
                }
                Err(error) => return Err(error),
            }
        }
        if plans.is_empty() {
            // Nothing left to serve: mint one invitation, exactly as a
            // single-device install always did. Replacing is right here because
            // there is nothing to keep.
            let (creds, code, exp) = self.host.invite(false).await?;
            plans.push(SlotPlan {
                creds,
                pairing_code: Some(code),
                pairing_code_expires_at: exp,
            });
        }
        Ok(plans)
    }

    pub fn forget_slot(self: &Arc<Self>, pair_id: &str) -> Result<(), crate::Error> {
        // Local disk faults propagate: a credential that cannot be forgotten will
        // be retried on every start, and hiding that behind a log line turns a
        // read-only home directory into a machine that silently never recovers.
        self.host.remove_slot(pair_id)?;
        // The pointer must not keep naming a device this machine no longer has, or
        // the UI would report "in control" for a credential that cannot connect.
        self.release_control(pair_id);
        self.promote_a_lone_device();
        self.stop_slot(pair_id);
        Ok(())
    }

    pub fn promote_a_lone_device(self: &Arc<Self>) {
        if self.active_pair().is_some() {
            return;
        }
        let slots = self.host.load_slots();
        if let [only] = slots.as_slice() {
            self.claim_control(&only.pair_id);
        }
    }

    pub async fn start_slot(
        self: &Arc<Self>,
        plan: SlotPlan,
        epoch: u64,
        replace_existing: bool,
        owns_local_web: bool,
    ) -> Result<(), crate::Error> {
        let SlotPlan {
            creds,
            pairing_code,
            pairing_code_expires_at,
        } = plan;
        let pair_id = creds.pair_id.clone();
        if !replace_existing && self.state.lock().unwrap().contains_key(&pair_id) {
            return Ok(());
        }
        let connected_nats = match self.connect_nats(&creds, true).await {
            Ok(connection) => connection,
            Err(error) => return Err(error),
        };
        let client = connected_nats.client;
        let nats_health = connected_nats.health;
        if !self.start_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(shared) = self.access.commit(epoch, || -> Result<_, crate::Error> {
            let shared = self.shared_runtime(&creds, pairing_code.is_none(), false);
            if shared.pairing_confirmed.load(Ordering::Acquire) {
                self.host.save(&creds)?;
            }
            Ok(shared)
        }) else {
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
                self.new_handshake_state(
                    creds.clone(),
                    pairing_confirmed.clone(),
                    bridge_instance_id.clone(),
                )
                .with_access(epoch)
            })
            .clone();

        let transport = match self
            .build_transport(&client, &pair_id, &handshake_state, reply_slots.clone())
            .await
        {
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
        let refresh_task = self.spawn_credential_refresh(
            pair_id.clone(),
            reply_slots,
            pairing_confirmed.clone(),
            handshake_state.clone(),
        );
        candidate_tasks.track_owned(&refresh_task);
        // The browser client is a test-environment-only validation surface. Keep
        // the NATS/mobile bridge available in every environment, but never expose
        // the unauthenticated local HTTP listener in production or custom envs.
        let web_enabled = self.executor.validation_enabled() && owns_local_web;
        let (web_task, web_url, web_lan_url) = if web_enabled {
            match self.executor.bind_validation().await {
                Ok(surface) => (
                    Some(self.spawn_generation_for(epoch, surface.task)),
                    Some(surface.url),
                    surface.lan_url,
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
            candidate_tasks.track_owned(task);
        }

        // Keep the previous generation serving until every readiness prerequisite
        // above is complete. Replacing the map entry is the hand-off point; only
        // after it do we cancel the old generation for *this* device.
        let previous = self.access.commit(epoch, || {
            self.remember_device(&pair_id);
            self.state.lock().unwrap().insert(
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
        let Some(previous) = previous else {
            return Ok(());
        };
        candidate_tasks.installed();
        if let Some(previous) = previous {
            abort_generation(previous);
        }
        self.spawn_runtime_supervisor(pair_id.clone(), generation_id);
        if web_bind_failed {
            self.spawn_web_reconnect(pair_id);
        }
        Ok(())
    }

    pub fn spawn_start_retry(self: &Arc<Self>) {
        if !self.executor.options().automatic_recovery {
            return;
        }

        if self.start_retry_running.swap(true, Ordering::AcqRel) {
            return;
        }
        self.spawn({
            let server = self.clone();
            async move {
                let mut delay = std::time::Duration::from_secs(1);
                server
                    .start_retry_since
                    .compare_exchange(0, unix_millis(), Ordering::AcqRel, Ordering::Acquire)
                    .ok();
                loop {
                    let jitter = 0.8 + rand::random::<f64>() * 0.4;
                    let jittered = std::time::Duration::from_secs_f64(
                        (delay.as_secs_f64() * jitter).min(30.0),
                    );
                    server.start_retry_next_at.store(
                        unix_millis().saturating_add(jittered.as_millis() as u64),
                        Ordering::Release,
                    );
                    tokio::time::sleep(jittered).await;
                    server.start_retry_attempts.fetch_add(1, Ordering::AcqRel);
                    if !server.start_requested.load(Ordering::Acquire)
                        || matches!(server.status().phase, RemotePhase::Ready)
                    {
                        break;
                    }
                    match server.start_once(false).await {
                        Ok(status) if matches!(status.phase, RemotePhase::Ready) => break,
                        Ok(status) if retryable_start_status(&status) => {
                            delay = delay
                                .saturating_mul(2)
                                .min(std::time::Duration::from_secs(30));
                        }
                        Ok(_) | Err(_) => break,
                    }
                }
                server.start_retry_next_at.store(0, Ordering::Release);
                server.start_retry_running.store(false, Ordering::Release);
                // Close the tiny stop→start race: a new start may have requested
                // recovery after this worker decided to exit but before it released
                // the singleton flag. Re-arm from the latest status in that case.
                let latest = server.status();
                if server.start_requested.load(Ordering::Acquire) && retryable_start_status(&latest)
                {
                    server.spawn_start_retry();
                }
            }
        });
    }

    pub fn spawn_runtime_reconnect(self: &Arc<Self>, pair_id: String) {
        if !self.executor.options().automatic_recovery {
            return;
        }

        let attempt = self.record_runtime_failure(unix_millis());
        eprintln!(
            "remote: critical task stopped [RT001]; rebuilding generation ({attempt}/{MAX_RUNTIME_RECONNECT_ATTEMPTS})"
        );
        if attempt > MAX_RUNTIME_RECONNECT_ATTEMPTS {
            *self.last_error_code.lock().unwrap() = Some("reconnect_required".to_string());
            return;
        }
        self.spawn({
            let server = self.clone();
            async move {
                let _start_guard = server.start_lock.lock().await;
                match server.restart_slot(&pair_id).await {
                    // A retryable refusal is not a dead end: hand it to the start
                    // retry worker, exactly as a failed `start_once` does. Its own
                    // attempt is what records the episode and backs off.
                    Ok(status) if retryable_start_status(&status) => server.spawn_start_retry(),
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("remote: automatic bridge reconnect failed [RT001]: {error}");
                        *server.last_error_code.lock().unwrap() =
                            Some("reconnect_required".to_string());
                    }
                }
            }
        });
    }

    async fn restart_slot(self: &Arc<Self>, pair_id: &str) -> Result<RemoteStatus, crate::Error> {
        let Some(creds) = self
            .host
            .load_slots()
            .into_iter()
            .find(|slot| slot.pair_id == pair_id)
        else {
            // The device is gone; the runtime must not outlive its credential.
            self.stop_slot(pair_id);
            return Ok(self.empty());
        };
        if creds.handshake_version != 2 || creds.secure.is_none() {
            self.forget_slot(pair_id)?;
            return Ok(self.empty());
        }
        let creds = match self.host.refresh(creds).await {
            Ok(creds) => creds,
            Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                self.forget_slot(pair_id)?;
                return Ok(self.empty());
            }
            Err(error) => return self.start_failure(error),
        };
        let plan = SlotPlan {
            creds,
            pairing_code: None,
            pairing_code_expires_at: None,
        };
        let epoch = self.access.current();
        match self.start_slot(plan, epoch, true, false).await {
            Ok(()) => Ok(self.status_focused(Some(pair_id))),
            Err(error) => self.start_failure(error),
        }
    }

    pub fn record_runtime_failure(self: &Arc<Self>, now: u64) -> u8 {
        let started = self.runtime_failure_window_started.load(Ordering::Acquire);
        if started == 0 || now.saturating_sub(started) > RUNTIME_FAILURE_WINDOW_MS {
            self.runtime_failure_window_started
                .store(now, Ordering::Release);
            self.runtime_reconnect_attempts.store(0, Ordering::Release);
        }
        self.runtime_reconnect_attempts
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }

    pub fn spawn_runtime_supervisor(self: &Arc<Self>, pair_id: String, generation_id: u64) {
        if !self.executor.options().automatic_recovery {
            return;
        }

        let server = self.clone();
        self.spawn(async move {
            server.watch_generation(pair_id, generation_id).await;
        });
    }

    pub fn spawn_web_reconnect(self: &Arc<Self>, pair_id: String) {
        if !self.executor.options().automatic_recovery {
            return;
        }

        if self.web_reconnect_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let epoch = self.access.current();
        let attempt = self.web_reconnect_attempts.fetch_add(1, Ordering::AcqRel) + 1;
        eprintln!(
            "remote: local web listener unavailable; retrying ({attempt}/{MAX_WEB_RECONNECT_ATTEMPTS})"
        );
        self.spawn({
            let server = self.clone();
            async move {
                match server.executor.bind_validation().await {
                    Ok(surface) => {
                        let web_url = Some(surface.url);
                        let web_lan_url = surface.lan_url;
                        let web_task = server.spawn_generation_for(epoch, surface.task);
                        let mut candidate = lifecycle::CandidateTasks::default();
                        candidate.track_owned(&web_task);
                        let installed = server.access.commit(epoch, || {
                            let mut guard = server.state.lock().unwrap();
                            let Some(state) = guard.get_mut(&pair_id) else {
                                return false;
                            };
                            if let Some(previous) = state.web_task.replace(web_task) {
                                previous.abort();
                            }
                            state.web_url = web_url;
                            state.web_lan_url = web_lan_url;
                            server.web_reconnect_attempts.store(0, Ordering::Release);
                            eprintln!("remote: local web listener reconnected [LC002]");
                            true
                        });
                        if installed == Some(true) {
                            candidate.installed();
                        }
                    }
                    Err(error) => {
                        eprintln!("remote: local web listener reconnect failed [LC002]: {error}")
                    }
                }
                let _ = server.access.commit(epoch, || {
                    server.web_reconnect_running.store(false, Ordering::Release)
                });
            }
        });
    }

    pub fn start_failure(
        self: &Arc<Self>,
        error: crate::Error,
    ) -> Result<RemoteStatus, crate::Error> {
        match pairing::error_code(&error) {
            Some(code) => {
                if let Some(line) = self.start_failures.record(code, &error) {
                    eprintln!("{line}");
                }
                *self.last_error_code.lock().unwrap() = Some(code.to_string());
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
                        attempt: self.start_retry_attempts.load(Ordering::Acquire),
                        max_attempts: None,
                        since: self.start_retry_since.load(Ordering::Acquire),
                        next_retry_at: None,
                    }),
                    ..self.empty()
                })
            }
            None => Err(error),
        }
    }

    async fn watch_generation(self: Arc<Self>, pair_id: String, generation_id: u64) {
        let mut healthy_secs = 0u8;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if !self.start_requested.load(Ordering::Acquire) {
                return;
            }
            let unhealthy = {
                let guard = self.state.lock().unwrap();
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
                    self.runtime_reconnect_attempts.store(0, Ordering::Release);
                    self.runtime_failure_window_started
                        .store(0, Ordering::Release);
                }
                continue;
            }
            if self.runtime_reconnect_attempts.load(Ordering::Acquire)
                >= MAX_RUNTIME_RECONNECT_ATTEMPTS
            {
                *self.last_error_code.lock().unwrap() = Some("reconnect_required".to_string());
                return;
            }
            self.spawn_runtime_reconnect(pair_id.clone());
            return;
        }
    }
}
