//! Startup and recovery of independently paired controlled-endpoint slots.
use super::transport::TransportTasks;
use super::*;
use crate::{health::unix_millis, pairing, types::*};
pub struct SlotPlan {
    pub creds: PairingCreds,
    pub pairing_code: Option<String>,
    pub pairing_code_expires_at: Option<i64>,
}

/// How many times one `add` builds the bridge for the invitation it minted.
///
/// Minting and serving are two different round trips, and the second one is the
/// slower of them: the broker has to be reachable and the credential accepted.
/// An invitation whose bridge never came up is a QR code that pairs nothing, so a
/// first attempt that failed on the network alone — a machine that just woke, a
/// relay still coming up — gets another couple of tries instead of being handed
/// back as "try again later".
pub(super) const ADD_CONNECT_ATTEMPTS: u8 = 3;
/// The pause before attempt `attempt + 1`.
///
/// Grows so a broker that is coming up is not raced by a tight loop, and stays
/// short because a user is waiting for a QR code: the whole retry budget is
/// under a second, next to the platform round trip that precedes it.
pub(super) fn add_connect_pause(attempt: u8) -> std::time::Duration {
    std::time::Duration::from_millis(250 * u64::from(attempt))
}

/// Whether a failed `start_slot` deserves another go with the same invitation.
///
/// Only a network failure does. A refused credential is refused however often it
/// is presented, and a local fault (an unreadable NKey, a full disk) is not
/// something waiting repairs — retrying those would delay the one answer the user
/// can act on by the whole retry budget.
pub(super) fn add_retryable(attempt: u8, error: &crate::Error) -> bool {
    attempt < ADD_CONNECT_ATTEMPTS && pairing::error_code(error) == Some("network")
}
/// The invitation this pair is still waiting for.
pub(super) struct CarriedInvitation {
    /// The code the desktop showed, so the rebuilt slot keeps showing it.
    pub code: String,
    pub expires_at: Option<i64>,
    /// The invitation's own credentials: its identity is what the code proves.
    pub creds: PairingCreds,
}

/// The invitation a device is still waiting for, if it is still waiting.
///
/// The one piece of a slot that is **not** in the pairing book: an unconfirmed
/// device is not a device yet, so its invitation lives only in the running
/// bridge. Everything that rebuilds a slot from the book alone therefore drops it
/// — `establish_slots` when a start or a retry re-runs, and `restart_slot` when a
/// generation dies — and brings back the identity of the *previous* pairing on the
/// same `pair_id` (the platform hands a known desktop the same one). The code on
/// screen then pairs nothing: the rebuilt identity has already pinned a peer, so
/// the still-correct invitation is refused with `invitation_already_used`.
///
/// Recovery asks this first. `None` is the ordinary case and means "nothing
/// pending": the device confirmed (the code is spent), the code expired (it would
/// be refused anyway), or no slot for it is running.
pub(super) fn carried_invitation(state: &RemoteState) -> Option<CarriedInvitation> {
    if state.pairing_confirmed.load(Ordering::Acquire) {
        return None;
    }
    let code = state.pairing_code.clone()?;
    let expires_at = state.pairing_code_expires_at;
    // `exp` is inclusive-upper, the same rule the client and the handshake use.
    if expires_at.is_some_and(|exp| crate::supervisor::unix_timestamp() >= exp.max(0) as u64) {
        return None;
    }
    Some(CarriedInvitation {
        code,
        expires_at,
        creds: state.security.pending_invitation()?,
    })
}

/// What a reconnect must do with one device, decided from the pairing book alone.
///
/// Split out of [`Supervisor::restart_slot`] so the three decisions are assertable
/// without a child process, a broker and a platform. The end-to-end tests that
/// drove them through a real Desktop could not be: a fixture's device starts with
/// an already-expired credential (the only way a live refusal is classified
/// *reconnectable* — `health.rs` treats a rejection of a still-valid, refreshed
/// credential as terminal), and an expired credential makes the device's own
/// renewal worker due immediately. Whether the run then reached `Ready` before
/// that first generation was rebuilt was a race between the fixture's timer and
/// the child's startup, which is why it failed on the CI runner and passed on a
/// laptop.
#[derive(Debug)]
pub(super) enum SlotRecovery {
    /// The book no longer holds the device: stop its runtime and keep nothing.
    Stop,
    /// A credential that cannot be upgraded in place (pre-v2, or no signed peer
    /// identity): forget the device rather than serve it forever.
    Forget,
    /// Refresh this credential and rebuild the generation from it.
    ///
    /// Boxed because the credential is ~300 bytes and the other two variants carry
    /// nothing: the decision is built once per reconnect, so the allocation is
    /// free next to the round trip it precedes, while an unboxed payload would
    /// make every `let` in this file move that much.
    Refresh(Box<PairingCreds>),
}

pub(super) fn recovery_plan(slots: &[PairingCreds], pair_id: &str) -> SlotRecovery {
    match slots.iter().find(|slot| slot.pair_id == pair_id) {
        None => SlotRecovery::Stop,
        Some(creds) if creds.handshake_version != 2 || creds.secure.is_none() => {
            SlotRecovery::Forget
        }
        Some(creds) => SlotRecovery::Refresh(Box::new(creds.clone())),
    }
}

impl Supervisor {
    pub async fn start(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        self.start_with(true).await
    }
    /// GUI switch: serve existing pairings without minting an invitation.
    pub async fn enable(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        self.start_with(false).await
    }
    async fn start_with(
        self: &Arc<Self>,
        invite_when_empty: bool,
    ) -> Result<RemoteStatus, crate::Error> {
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
            self.invite_when_empty
                .store(invite_when_empty, Ordering::Release);
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

        if plans.is_empty() {
            self.access.commit(epoch, || {
                *self.last_error_code.lock().unwrap() = None;
            });
            return Ok(self.status());
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
                            future_remote_protocol::remote_log!("{line}");
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
        // One live invitation at a time.
        //
        // An unclaimed code already belongs to a running bridge and is on this
        // machine's screen (or was, a moment ago). Minting a second one spent a
        // *second* row of the platform's per-desktop allowance — every pending
        // invitation counts against it — and answering with the newest code
        // orphaned the older ones, which nothing could claim and nothing
        // displayed. Pressing "add a device" five times therefore reached the cap
        // and refused the sixth with a limit error about a cap the user had
        // filled without ever pairing anything.
        if let Some(pair_id) = self.shown_invitations().into_iter().next() {
            return Ok(self.status_focused(Some(&pair_id)));
        }
        let keep_existing = !self.host.load_slots().is_empty();
        let Invitation {
            creds,
            code,
            expires_at,
            cleared,
        } = self
            .host
            .invite(keep_existing, &self.shown_invitations())
            .await?;
        if self
            .access
            .commit(epoch, || {
                self.start_requested.store(true, Ordering::Release)
            })
            .is_none()
        {
            return Ok(self.empty());
        }
        let added = creds.pair_id.clone();
        let _start_guard = self.start_lock.lock().await;
        // The bridge is started from the invitation that was just minted, and the
        // same one is retried: minting again would put a *second* code on the
        // platform (each is a row that counts towards the cap) while the user is
        // looking at the first, and the first would serve nobody.
        let mut attempt = 0u8;
        loop {
            attempt += 1;
            #[cfg(any(test, feature = "test-support"))]
            self.hooks.add_attempts.fetch_add(1, Ordering::AcqRel);
            let plan = SlotPlan {
                creds: creds.clone(),
                pairing_code: Some(code.clone()),
                pairing_code_expires_at: expires_at,
            };
            match self.start_slot(plan, epoch, true, false).await {
                Ok(()) => {
                    let mut status = self.status_focused(Some(&added));
                    // Carried out by this one answer: the codes that had to go so
                    // this one could exist are news, not state.
                    status.cleared_invitations = cleared;
                    return Ok(status);
                }
                Err(error) if add_retryable(attempt, &error) => {
                    // Only a broker that is not answering yet is worth the next
                    // attempt. A refused credential or a local fault (disk, NKey)
                    // fails the same way however long one waits, and waiting would
                    // only delay the answer the user can act on.
                    tokio::time::sleep(add_connect_pause(attempt)).await;
                    if !self.start_requested.load(Ordering::Acquire) {
                        return Ok(self.empty());
                    }
                }
                // Reported as it is, so the caller can name the class: a code
                // nothing serves is worse than an error that says why.
                Err(error) => {
                    // The code was never shown — this call is what would have
                    // returned it — so nobody can claim it. Give the platform's
                    // allowance back now instead of leaving the row for the
                    // sweep that only clears invitations older than three
                    // minutes: a person retrying a failing add would otherwise
                    // fill the cap with codes that were never on screen.
                    let _ = self.host.queue_revoke(&creds);
                    self.spawn_revoke_cleanup();
                    return Err(error);
                }
            }
        }
    }

    pub async fn pair_new_device(self: &Arc<Self>) -> Result<RemoteStatus, crate::Error> {
        if self.host.load_slots().is_empty() {
            return self.re_pair().await;
        }
        self.add_device().await
    }

    /// The plan for a book slot, keeping the invitation it is still waiting for.
    ///
    /// The refreshed credentials carry the current grant; the *identity* and the
    /// code come from the pending invitation, because that is what the QR on
    /// screen proves. Without this, a start or a retry re-runs `establish_slots`
    /// and the pending device silently becomes the previously paired one.
    fn with_carried_invitation(&self, pair_id: &str, creds: PairingCreds) -> SlotPlan {
        let carried = {
            let state = self.state.lock().unwrap();
            state.get(pair_id).and_then(carried_invitation)
        };
        match carried {
            Some(carried) => SlotPlan {
                creds: PairingCreds {
                    secure: carried.creds.secure,
                    ..creds
                },
                pairing_code: Some(carried.code),
                pairing_code_expires_at: carried.expires_at,
            },
            None => SlotPlan {
                creds,
                pairing_code: None,
                pairing_code_expires_at: None,
            },
        }
    }
    pub async fn establish_slots(self: &Arc<Self>) -> Result<Vec<SlotPlan>, crate::Error> {
        let mut plans = Vec::new();
        // The revocation a start *acted* on, kept as the error itself so the
        // caller's existing classification names it. It is only the start's answer
        // when it leaves nothing to serve — see the empty-book arm below.
        let mut revoked: Option<crate::Error> = None;
        for creds in self.host.load_slots() {
            let pair_id = creds.pair_id.clone();
            if creds.handshake_version != 2 || creds.secure.is_none() {
                // Credentials created before the signed mutual handshake have no
                // QR-bound peer identity. They cannot be upgraded safely in place.
                future_remote_protocol::remote_log!("remote: replacing legacy pairing [PA003]");
                self.forget_slot(&pair_id)?;
                continue;
            }
            match self.host.refresh(creds).await {
                Ok(creds) => plans.push(self.with_carried_invitation(&pair_id, creds)),
                Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                    // The platform no longer honours this device's credential —
                    // someone revoked it. It is not this machine's device any more,
                    // and leaving it on disk would make every start retry a
                    // credential that can never succeed.
                    future_remote_protocol::remote_log!("remote: a pairing was revoked [PA001 invalid_remote_credential]; forgetting that device");
                    self.forget_slot(&pair_id)?;
                    // Only the first one is kept: one reason names the event, and
                    // a machine whose book held several revoked devices has the
                    // same thing to do about it.
                    revoked.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }
        if plans.is_empty() {
            if self.invite_when_empty.load(Ordering::Acquire) {
                // Nothing left to serve: mint one invitation, exactly as a
                // single-device install always did. Replacing is right here because
                // there is nothing to keep.
                let Invitation {
                    creds,
                    code,
                    expires_at,
                    ..
                // Nothing is on screen yet: this invitation is the one starting.
                } = self.host.invite(false, &[]).await?;
                plans.push(SlotPlan {
                    creds,
                    pairing_code: Some(code),
                    pairing_code_expires_at: expires_at,
                });
            } else if let Some(error) = revoked {
                // This start mints no invitation, so a revoked device leaves it
                // with nothing at all: no bridge, no book, no code. Answering
                // "on, serving nothing" there would report the same state as an
                // unpaired machine while the reason the book is empty — a
                // credential the platform refused — is a fault the operator has to
                // handle, and the entry points stop on it (`Revoked` is a terminal
                // phase). So it is the start's failure, which projects exactly the
                // phase a revocation noticed mid-run produces; the two readings of
                // one event must not disagree. A book emptied by the user's own
                // `remove` never takes this arm: nothing was revoked, so the
                // emptied book stays "on, serving nothing" and the entry keeps
                // running, ready to be asked for a new device.
                return Err(error);
            }
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
        let (installed_tx, installed_rx) = tokio::sync::oneshot::channel();
        let refresh_task = self.spawn_credential_refresh(
            pair_id.clone(),
            reply_slots,
            pairing_confirmed.clone(),
            handshake_state.clone(),
            Some(installed_rx),
        );
        candidate_tasks.track_owned(&refresh_task);
        // The browser client is a test-environment-only validation surface. Keep
        // the NATS/mobile bridge available in every environment, but never expose
        // the unauthenticated local HTTP listener in production or custom envs.
        let web_enabled = self.executor.validation_enabled() && owns_local_web;
        let (web_task, web_url, web_lan_url) = if web_enabled {
            if self
                .access
                .commit(epoch, || {
                    *self.web_owner.lock().unwrap() = Some(pair_id.clone());
                })
                .is_none()
            {
                return Ok(());
            }
            match self.executor.bind_validation().await {
                Ok(surface) => (
                    Some(self.spawn_generation_for(epoch, surface.task)),
                    Some(surface.url),
                    surface.lan_url,
                ),
                Err(error) => {
                    future_remote_protocol::remote_log!(
                        "remote: web client bind failed [LC002]: {error}"
                    );
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
                    credential_expires_at: creds.jwt_expires_at,
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
        // The refresh loop checks its generation in `state`. Starting it before
        // publication lets an already-due credential race the web bind and exit
        // as if its generation had been replaced. A cancelled install drops the
        // sender, so its candidate cannot begin renewing a discarded generation.
        let _ = installed_tx.send(());
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

    pub fn spawn_runtime_reconnect(self: &Arc<Self>, pair_id: String, generation_id: u64) {
        if !self.executor.options().automatic_recovery {
            return;
        }
        // A generation that stopped *because the host is stopping* is not a fault:
        // teardown cancels the command/event tasks and closes the client, so the
        // health watcher sees exactly what a crash looks like. Building a
        // replacement then spends the shutdown budget on a bridge that is about to
        // be dropped again, and records an `[RT001]` episode (and a
        // `reconnect_required` code) for a stop nobody suffered.
        if self.executor.is_stopping() {
            return;
        }

        let attempt = self.record_runtime_failure(unix_millis());
        future_remote_protocol::remote_log!(
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
                    Ok(_) => server.rearm_generation_watch(&pair_id, generation_id),
                    Err(error) => {
                        future_remote_protocol::remote_log!(
                            "remote: automatic bridge reconnect failed [RT001]: {error}"
                        );
                        *server.last_error_code.lock().unwrap() =
                            Some("reconnect_required".to_string());
                        server.rearm_generation_watch(&pair_id, generation_id);
                    }
                }
            }
        });
    }

    /// Hand a generation whose rebuild failed back to the watch that owns the
    /// attempt budget.
    ///
    /// `spawn_runtime_reconnect` spends exactly one attempt, and the watch that
    /// asked for it has already returned by the time the rebuild finishes. So
    /// before this every failed rebuild was the last thing that ever ran for
    /// that device: one `[RT001]` line — or nothing at all when the rebuild
    /// answered a status that is not retryable, which is what a refused
    /// credential does — and then silence while the grant it was holding lapsed
    /// and every reconnect presented it again. The machine only came back if
    /// someone restarted the process.
    ///
    /// Re-arming returns the decision to [`Supervisor::watch_generation`], where
    /// the attempt budget and the ten-minute cool-down already live: a relay
    /// whose authorization cache is still catching up recovers within seconds,
    /// and a machine whose credentials are permanently refused spends a bounded
    /// few mints and then reports `service_authorization` between attempts
    /// rather than pretending to serve.
    fn rearm_generation_watch(self: &Arc<Self>, pair_id: &str, generation_id: u64) {
        if self.executor.is_stopping() {
            return;
        }
        // Only while the generation that failed is still the installed one: a
        // rebuild that did install a replacement must not resurrect a watch over
        // a generation that is already gone — the replacement spawned its own.
        let installed = self
            .state
            .lock()
            .unwrap()
            .get(pair_id)
            .is_some_and(|state| state.generation_id == generation_id);
        if !installed {
            return;
        }
        self.spawn_runtime_supervisor(pair_id.to_string(), generation_id);
    }

    async fn restart_slot(self: &Arc<Self>, pair_id: &str) -> Result<RemoteStatus, crate::Error> {
        // A device the desktop is still waiting for has no book entry — the
        // invitation is what will make it one — so the book-first recovery below
        // would answer `Stop` for it and take the bridge serving the code on
        // screen down with it. It also has no *stored* identity to bring back when
        // it does: the slot's own invitation identity is the one to rebuild from.
        let carried = {
            let state = self.state.lock().unwrap();
            state.get(pair_id).and_then(carried_invitation)
        };
        if let Some(carried) = carried {
            let plan = SlotPlan {
                creds: carried.creds,
                pairing_code: Some(carried.code),
                pairing_code_expires_at: carried.expires_at,
            };
            let epoch = self.access.current();
            return match self.start_slot(plan, epoch, true, false).await {
                Ok(()) => Ok(self.status_focused(Some(pair_id))),
                Err(error) => self.start_failure(error),
            };
        }
        let creds = match recovery_plan(&self.host.load_slots(), pair_id) {
            // The device is gone; the runtime must not outlive its credential.
            SlotRecovery::Stop => {
                self.stop_slot(pair_id);
                return Ok(self.empty());
            }
            SlotRecovery::Forget => {
                self.forget_slot(pair_id)?;
                return Ok(self.empty());
            }
            SlotRecovery::Refresh(creds) => *creds,
        };
        let creds = match self.host.refresh(creds).await {
            Ok(creds) => creds,
            Err(error) if pairing::is_invalid_or_revoked_error(&error) => {
                // Only the classified, fixed code is logged, never a platform
                // response body/message that could reflect credentials.
                future_remote_protocol::remote_log!(
                    "remote: invalid_remote_credential; forgetting that device"
                );
                self.forget_slot(pair_id)?;
                return Ok(self.empty());
            }
            Err(error) => return self.start_failure(error),
        };
        let plan = self.with_carried_invitation(pair_id, creds);
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
        future_remote_protocol::remote_log!(
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
                            *server.web_owner.lock().unwrap() = Some(pair_id.clone());
                            if let Some(previous) = state.web_task.replace(web_task) {
                                previous.abort();
                            }
                            state.web_url = web_url;
                            state.web_lan_url = web_lan_url;
                            server.web_reconnect_attempts.store(0, Ordering::Release);
                            future_remote_protocol::remote_log!(
                                "remote: local web listener reconnected [LC002]"
                            );
                            true
                        });
                        if installed == Some(true) {
                            candidate.installed();
                        }
                    }
                    Err(error) => {
                        future_remote_protocol::remote_log!(
                            "remote: local web listener reconnect failed [LC002]: {error}"
                        )
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
                    future_remote_protocol::remote_log!("{line}");
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
                // A credential the relay refused is a verdict on *this
                // generation*, not on the machine, and the replacement is built
                // by `recovery_plan` — which re-mints through `host.refresh`
                // instead of presenting the refused grant again. Reading it as
                // "stop watching" is what left a live bridge dead with nobody
                // left to mint: the socket reported `User Authentication
                // Expired`, the watch returned, and the device stayed offline
                // until the process was restarted.
                state.nats_health.is_terminal()
                    || state.nats_health.needs_reconnect()
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
                let since = self.runtime_failure_window_started.load(Ordering::Acquire);
                if unix_millis().saturating_sub(since) <= RUNTIME_FAILURE_WINDOW_MS {
                    // Keep the generation's existing watcher as the single
                    // retry owner. Exhaustion cools down; it must not require a
                    // user reconnect after an ordinary network outage. The next
                    // lap rechecks user intent and the installed generation.
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    continue;
                }
            }
            self.spawn_runtime_reconnect(pair_id.clone(), generation_id);
            return;
        }
    }
}
