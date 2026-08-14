//! Step C: command subscription + routing. Mobile commands arrive on
//! `p.{pairId}.cmd.>`; reads go straight to the store, prompts go through
//! `agent_bridge::headless` so the persist/finalize contract is shared with
//! the rest of the backend.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

type ReplySlot = Arc<tokio::sync::Mutex<Option<Vec<u8>>>>;

/// Command-id → in-flight/completed response cache (single-flight). Created
/// once per bridge start and SHARED across command loops: credential refresh
/// swaps the loop every JWT TTL, and a cache local to the loop would be wiped
/// on every swap — a client retrying right after a swap would re-execute a
/// command the old loop had already run (for `prompt`, a duplicated message).
pub(super) type ReplySlots = Arc<Mutex<HashMap<String, ReplySlot>>>;

#[derive(Clone)]
pub(super) struct HandshakeState {
    creds: crate::remote::pairing::PairingCreds,
    confirmed: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    bridge_instance_id: String,
    pending: Arc<Mutex<HashMap<String, PendingHandshake>>>,
}

#[derive(Clone)]
struct PendingHandshake {
    transcript: String,
    device_id: String,
    client_public_key: String,
}

impl HandshakeState {
    pub(super) fn new(
        creds: crate::remote::pairing::PairingCreds,
        confirmed: Arc<AtomicBool>,
        bridge_instance_id: String,
    ) -> Self {
        Self {
            creds,
            confirmed,
            active: Arc::new(AtomicBool::new(false)),
            bridge_instance_id,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(super) fn bridge_instance_id(&self) -> &str {
        &self.bridge_instance_id
    }

    pub(super) fn active_flag(&self) -> Arc<AtomicBool> {
        self.active.clone()
    }
}

pub(super) fn new_reply_slots() -> ReplySlots {
    Arc::new(Mutex::new(HashMap::new()))
}

/// How long a completed single-flight response stays cached for retrying
/// clients (matches the planned NATS duplicate window). Tests shrink it to
/// milliseconds so expiry can be observed without a ten-minute wait.
fn reply_slot_ttl() -> Duration {
    #[cfg(test)]
    const TTL: Duration = Duration::from_millis(500);
    #[cfg(not(test))]
    const TTL: Duration = Duration::from_secs(600);
    TTL
}

/// First resubscribe delay after a failed subscribe / ended stream (doubles up
/// to 30s). Tests shrink it so the self-heal path runs without real waits.
fn resubscribe_backoff() -> Duration {
    #[cfg(test)]
    const BACKOFF: Duration = Duration::from_millis(10);
    #[cfg(not(test))]
    const BACKOFF: Duration = Duration::from_secs(1);
    BACKOFF
}

tokio::task_local! {
    static REPLY_CAPTURE: Arc<Mutex<Option<Vec<u8>>>>;
}

/// Command sent by the client via NATS (camelCase JSON, only the fields the bridge needs).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct IncomingCmd {
    id: String,
    #[serde(rename = "type")]
    cmd_type: String,
    session_id: String,
    message: String,
    // approval_decision
    entry_id: String,
    mode: String,
    // get_events_since (P1c backfill)
    run_id: String,
    since_idx: i64,
    // get_messages pagination (NATS payload-limit guard)
    offset: i64,
    limit: i64,
    // set_model / set_thinking_level
    model_id: String,
    provider_id: String,
    level: String,
    // set_approval_tier
    tier: String,
    // set_session_name
    name: String,
    transfer_name: String,
    // delete_session / set_session_pinned (thread-scoped, see ThreadRecord)
    thread_id: String,
    pinned: bool,
    // prompt creation mode / existing workspace selection
    workspace_id: String,
    // file transfer control + prompt attachment references
    mime_type: String,
    kind: String,
    original_size: u64,
    transfer_size: u64,
    transfer_id: String,
    file_path: String,
    attachments: Vec<super::transfer::UploadReference>,
    // signed application-level pairing handshake
    protocol_version: u32,
    pair_id: String,
    device_id: String,
    client_public_key: String,
    client_nonce: String,
    desktop_nonce: String,
    expected_desktop_id: String,
    expected_desktop_public_key: String,
    client_signature: String,
}

impl Default for IncomingCmd {
    fn default() -> Self {
        Self {
            id: String::new(),
            cmd_type: String::new(),
            session_id: String::new(),
            message: String::new(),
            entry_id: String::new(),
            mode: String::new(),
            run_id: String::new(),
            since_idx: -1,
            offset: 0,
            limit: 0,
            model_id: String::new(),
            provider_id: String::new(),
            level: String::new(),
            tier: String::new(),
            name: String::new(),
            transfer_name: String::new(),
            thread_id: String::new(),
            pinned: false,
            workspace_id: String::new(),
            mime_type: String::new(),
            kind: String::new(),
            original_size: 0,
            transfer_size: 0,
            transfer_id: String::new(),
            file_path: String::new(),
            attachments: Vec::new(),
            protocol_version: 0,
            pair_id: String::new(),
            device_id: String::new(),
            client_public_key: String::new(),
            client_nonce: String::new(),
            desktop_nonce: String::new(),
            expected_desktop_id: String::new(),
            expected_desktop_public_key: String::new(),
            client_signature: String::new(),
        }
    }
}

pub(super) async fn command_loop(
    client: async_nats::Client,
    pair_id: String,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
) {
    let subject = format!("p.{pair_id}.cmd.>");
    let queue = format!("bridge.{pair_id}");
    // Self-heal: a failed subscribe or an ended subscription stream (server-side
    // close, permission error) must not kill the loop — a dead loop leaves the
    // queue group without a live member and every command then times out until
    // the next credential-refresh swap. Retry with backoff; the task is only
    // terminated by the caller (generation swap / stop), which aborts it.
    let mut backoff = resubscribe_backoff();
    loop {
        let mut sub = match client.queue_subscribe(subject.clone(), queue.clone()).await {
            Ok(sub) => sub,
            Err(e) => {
                eprintln!(
                    "remote: failed to subscribe to commands {subject}: {e}; retrying in {backoff:?}"
                );
                tokio::time::sleep(backoff).await;
                backoff = backoff.saturating_mul(2).min(Duration::from_secs(30));
                continue;
            }
        };
        backoff = resubscribe_backoff();
        eprintln!("remote: subscribed to commands {subject}");
        while let Some(msg) = sub.next().await {
            let client = client.clone();
            let reply_slots = reply_slots.clone();
            let handshake = handshake.clone();
            // Spawn per command: prevent a slow command from blocking others.
            tokio::spawn(async move {
                handle_command_singleflight(&client, msg, reply_slots, handshake).await;
            });
        }
        eprintln!("remote: command subscription ended unexpectedly; resubscribing in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = backoff.saturating_mul(2).min(Duration::from_secs(30));
    }
}

/// Merge concurrent/retried deliveries carrying the same command id. The first
/// delivery executes the command; followers wait for and receive the exact same
/// response bytes. Completed responses stay cached for ten minutes, matching
/// the planned NATS duplicate window, then expire without blocking unrelated ids.
async fn handle_command_singleflight(
    client: &async_nats::Client,
    msg: async_nats::Message,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
) {
    let command_id = serde_json::from_slice::<IncomingCmd>(&msg.payload)
        .ok()
        .map(|cmd| cmd.id)
        .filter(|id| !id.is_empty());
    let Some(command_id) = command_id else {
        handle_command(client, msg, handshake).await;
        return;
    };

    let (slot, inserted) = {
        let mut slots = reply_slots.lock().unwrap();
        match slots.get(&command_id) {
            Some(slot) => (slot.clone(), false),
            None => {
                let slot = Arc::new(tokio::sync::Mutex::new(None));
                slots.insert(command_id.clone(), slot.clone());
                (slot, true)
            }
        }
    };

    if inserted {
        let slots = reply_slots.clone();
        let id = command_id.clone();
        let expected = slot.clone();
        tokio::spawn(async move {
            tokio::time::sleep(reply_slot_ttl()).await;
            let mut slots = slots.lock().unwrap();
            if slots
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(current, &expected))
            {
                slots.remove(&id);
            }
        });
    }

    let mut cached = slot.lock().await;
    if let Some(payload) = cached.as_ref() {
        publish_reply_payload(client, &msg, payload.clone()).await;
        return;
    }

    let capture = Arc::new(Mutex::new(None));
    REPLY_CAPTURE
        .scope(capture.clone(), handle_command(client, msg, handshake))
        .await;
    *cached = capture.lock().unwrap().clone();
}

// SECURITY: NATS admits this bridge with a short-lived user JWT whose server-
// enforced ACL is scoped to this pair. Session/approval ownership is still
// checked in the command handlers because subject isolation and application
// authorization are separate boundaries.
async fn handle_command(
    client: &async_nats::Client,
    msg: async_nats::Message,
    handshake: HandshakeState,
) {
    let cmd: IncomingCmd = match serde_json::from_slice(&msg.payload) {
        Ok(cmd) => cmd,
        Err(e) => {
            reply(
                client,
                &msg,
                false,
                Value::Null,
                Some(&format!("Failed to parse command JSON: {e}")),
            )
            .await;
            return;
        }
    };

    let handshake_command = matches!(
        cmd.cmd_type.as_str(),
        "pair_handshake" | "pair_handshake_confirm"
    );
    if !handshake_command && !handshake.active.load(Ordering::Acquire) {
        reply(
            client,
            &msg,
            false,
            Value::Null,
            Some("pairing_handshake_required"),
        )
        .await;
        return;
    }

    match cmd.cmd_type.as_str() {
        "pair_handshake" => {
            handle_pair_handshake(client, &msg, &cmd, &handshake).await;
        }
        "pair_handshake_confirm" => {
            handle_pair_handshake_confirm(client, &msg, &cmd, &handshake).await;
        }
        // Presence is normally pushed every 20 seconds. A client that subscribes
        // after the latest heartbeat would otherwise look offline until the next
        // tick because core NATS subscriptions do not replay old messages.
        "get_presence" => {
            let pair_id = msg
                .subject
                .strip_prefix("p.")
                .and_then(|subject| subject.split('.').next())
                .unwrap_or_default();
            reply(
                client,
                &msg,
                true,
                super::build_presence_payload(pair_id, &handshake.bridge_instance_id),
                None,
            )
            .await;
        }
        // A phone-initiated unpair first reaches the desktop over the existing
        // authenticated command channel. Acknowledge before stopping the
        // bridge, then perform the destructive work in a separate task so the
        // requester can clear itself without waiting for token expiry.
        "unpair" => {
            reply(client, &msg, true, json!({}), None).await;
            tauri::async_runtime::spawn(async {
                if let Err(error) = super::unpair().await {
                    eprintln!("remote: phone-initiated unpair failed: {error}");
                }
            });
        }
        "list_sessions" => match crate::store::list_threads() {
            Ok(threads) => {
                let active_sessions: Vec<String> =
                    crate::store::active_run_sessions().unwrap_or_default();
                let thread_ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
                let run_infos = crate::store::latest_run_infos(&thread_ids).unwrap_or_default();
                let run_status_by_thread: std::collections::HashMap<&str, &str> = run_infos
                    .iter()
                    .map(|info| (info.thread_id.as_str(), info.status.as_str()))
                    .collect();
                let sessions: Vec<Value> = threads
                    .into_iter()
                    .filter_map(|t| {
                        t.agent_session_id.map(|sid| {
                            let streaming = active_sessions.iter().any(|active| active == &sid);
                            let status = run_status_by_thread.get(t.id.as_str()).copied();
                            json!({
                                "sessionId": sid,
                                "threadId": t.id,
                                "title": t.title,
                                "mode": t.mode,
                                "workspaceId": t.workspace_id,
                                "pinned": t.pinned,
                                "streaming": streaming,
                                "status": status,
                            })
                        })
                    })
                    .collect();
                reply(client, &msg, true, json!({ "sessions": sessions }), None).await;
            }
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
        "list_workspaces" => match crate::store::list_workspaces() {
            Ok(workspaces) => {
                let workspaces: Vec<Value> = workspaces
                    .into_iter()
                    .filter(|workspace| workspace.kind == "user")
                    .filter_map(|workspace| serde_json::to_value(workspace).ok())
                    .collect();
                reply(
                    client,
                    &msg,
                    true,
                    json!({ "workspaces": workspaces }),
                    None,
                )
                .await;
            }
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
        "get_messages" => {
            // Serve history from the agent (source of truth for all sessions).
            // The GUI store only has message rows for GUI-native threads —
            // TUI/CLI sessions imported as thread stubs would show empty history.
            // Fall back to the store when the agent is unreachable.
            //
            // The whole history is fetched locally (gRPC/store have no payload
            // limit) then paged here, because NATS rejects any single reply over
            // the 1MB user-JWT payload cap — a long session's full history would
            // otherwise fail silently (client times out with no response).
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            let messages = match crate::agent_bridge::get_session_messages(cmd.session_id.clone())
                .await
            {
                Ok(data) => messages_vec(data),
                Err(agent_err) => {
                    reply(
                            client,
                            &msg,
                            false,
                            Value::Null,
                            Some(&format!(
                                "{agent_err}; conversation history is unavailable while the Agent is offline"
                            )),
                        )
                        .await;
                    return;
                }
            };
            reply(
                client,
                &msg,
                true,
                paginate_messages(messages, offset, limit),
                None,
            )
            .await;
        }
        "get_session_entries" => {
            // Display-shaped history (plain-text content + per-entry meta with
            // user attachments) for clients that render attachment chips.
            // Paged for the same NATS payload cap as get_messages.
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            match crate::agent_bridge::get_session_entries(cmd.session_id.clone()).await {
                Ok(data) => {
                    reply(
                        client,
                        &msg,
                        true,
                        paginate_items(entries_vec(data), offset, limit, "entries"),
                        None,
                    )
                    .await;
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "get_events_since" => {
            // P1c: replay buffered events for the current in-progress run, so late-joining clients can catch up on missed prefix events.
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            match crate::agent_bridge::get_events_since(
                cmd.session_id.clone(),
                cmd.run_id.clone(),
                cmd.since_idx,
            )
            .await
            {
                Ok(data) => {
                    reply(
                        client,
                        &msg,
                        true,
                        paginate_events(data, offset, limit),
                        None,
                    )
                    .await
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "upload_init" => {
            match super::transfer::init_upload(
                &cmd.name,
                &cmd.transfer_name,
                &cmd.mime_type,
                &cmd.kind,
                cmd.original_size,
                cmd.transfer_size,
            ) {
                Ok(data) => {
                    reply(
                        client,
                        &msg,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Err(error) => {
                    reply(client, &msg, false, Value::Null, Some(&error.to_string())).await
                }
            }
        }
        "upload_complete" => match super::transfer::complete_upload(&cmd.transfer_id) {
            Ok(data) => {
                reply(
                    client,
                    &msg,
                    true,
                    serde_json::to_value(data).unwrap_or(Value::Null),
                    None,
                )
                .await
            }
            Err(error) => reply(client, &msg, false, Value::Null, Some(&error.to_string())).await,
        },
        "upload_cancel" => {
            reply_unit(
                client,
                &msg,
                super::transfer::cancel_upload(&cmd.transfer_id),
            )
            .await
        }
        "download_prepare" => {
            match super::transfer::prepare_download(&cmd.session_id, &cmd.file_path).await {
                Ok(data) => {
                    reply(
                        client,
                        &msg,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Err(error) => {
                    reply(client, &msg, false, Value::Null, Some(&error.to_string())).await
                }
            }
        }
        "download_cancel" => {
            super::transfer::cancel_download(&cmd.transfer_id);
            reply(client, &msg, true, json!({}), None).await;
        }
        "prompt" => {
            // Lazy creation (matches the GUI new-chat flow): the web client's
            // "new" button only stages a local draft and sends the first message
            // with an empty `session_id`. Here an empty/unknown id creates the
            // thread + a real agent session on the fly, so the accept-ack can
            // carry the identifiers the events will be published under and the
            // client can latch onto the real session id. Model / thinking level
            // travel with the first prompt so the freshly-created session is
            // seeded with the user's draft selections.
            let model_id = qualified_model_id(&cmd.model_id, &cmd.provider_id);
            let thinking_level = (!cmd.level.trim().is_empty()).then(|| cmd.level.clone());
            match prepare_remote_prompt(
                &cmd.session_id,
                cmd.message.clone(),
                model_id,
                thinking_level,
                cmd.mode.clone(),
                cmd.workspace_id.clone(),
                cmd.attachments.clone(),
            )
            .await
            {
                Ok(prepared) => {
                    let ack = json!({
                        "sessionId": prepared.session_id,
                        "threadId": prepared.thread_id,
                        "runId": prepared.run_id,
                    });
                    // Actual execution runs in the background (completion visible via event stream agent_end).
                    tokio::spawn(async move {
                        let thread_id = prepared.thread_id.clone();
                        if let Err(e) = crate::agent_bridge::run_prepared_prompt(prepared).await {
                            eprintln!("remote: prompt processing failed: {e}");
                        }
                        crate::emit_remote_activity(&thread_id);
                    });
                    reply(client, &msg, true, ack, None).await;
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "abort" => {
            reply_unit(
                client,
                &msg,
                crate::agent_bridge::abort_session(&cmd.session_id).await,
            )
            .await
        }
        "continue_run" => {
            // Resume a failed run: synthesize a continue prompt from the run's
            // recent terminal events and push it through the normal prompt
            // pipeline (model/thinking default to the session's current values).
            let prompt = build_continue_prompt(&cmd.run_id);
            match prepare_remote_prompt(
                &cmd.session_id,
                prompt,
                None,
                None,
                "chat".to_string(),
                String::new(),
                Vec::new(),
            )
            .await
            {
                Ok(prepared) => {
                    let ack = json!({
                        "sessionId": prepared.session_id,
                        "threadId": prepared.thread_id,
                        "runId": prepared.run_id,
                    });
                    tokio::spawn(async move {
                        let thread_id = prepared.thread_id.clone();
                        if let Err(e) = crate::agent_bridge::run_prepared_prompt(prepared).await {
                            eprintln!("remote: continue_run failed: {e}");
                        }
                        crate::emit_remote_activity(&thread_id);
                    });
                    reply(client, &msg, true, ack, None).await;
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "approval_decision" => {
            let ownership = (|| -> Result<(), crate::AppError> {
                let approval = crate::store::get_approval_request(&cmd.entry_id)?
                    .ok_or_else(|| "Approval request could not be loaded.".to_string())?;
                let thread = crate::store::get_thread(&approval.thread_id)?
                    .ok_or_else(|| "Approval thread could not be loaded.".to_string())?;
                let owner_session_id = thread.agent_session_id.unwrap_or(thread.id);
                if cmd.session_id != owner_session_id {
                    return Err(crate::AppError::Message(
                        "Approval request does not belong to this session.".to_string(),
                    ));
                }
                Ok(())
            })();
            if let Err(error) = ownership {
                reply(client, &msg, false, Value::Null, Some(&error.to_string())).await;
                return;
            }
            let input = crate::store::DecideApprovalRequestInput {
                approval_request_id: cmd.entry_id.clone(),
                status: cmd.mode.clone(),
                decision_note: None,
            };
            reply_unit(
                client,
                &msg,
                crate::agent_bridge::decide_approval(input)
                    .await
                    .map(|_| ()),
            )
            .await;
        }
        "get_state" => match crate::agent_bridge::get_session_state(cmd.session_id.clone()).await {
            Ok(data) => reply(client, &msg, true, data, None).await,
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
        "list_models" | "get_available_models" => {
            match crate::agent_bridge::get_available_models().await {
                Ok(data) => reply(client, &msg, true, data, None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_model" => {
            reply_unit(
                client,
                &msg,
                crate::agent_bridge::set_session_model(
                    cmd.session_id.clone(),
                    qualified_model_id(&cmd.model_id, &cmd.provider_id).unwrap_or_default(),
                )
                .await,
            )
            .await;
        }
        "set_thinking_level" => {
            reply_unit(
                client,
                &msg,
                crate::agent_bridge::set_session_thinking_level(
                    cmd.session_id.clone(),
                    cmd.level.clone(),
                )
                .await,
            )
            .await;
        }
        "get_settings" => {
            match crate::store::get_app_settings() {
                Ok(settings) => {
                    reply(
                        client,
                        &msg,
                        true,
                        json!({
                            "approvalTier": settings.approval_tier,
                            // The macOS Seatbelt sandbox tier only exists on macOS;
                            // the phone gates its "sandbox" option on this flag so a
                            // Windows/Linux user never picks a tier that silently
                            // provides no isolation.
                            "sandboxAvailable": cfg!(target_os = "macos"),
                        }),
                        None,
                    )
                    .await
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_approval_tier" => {
            // The tier is a global app preference (not per-session): writing it
            // here is the same as flipping it in the desktop Settings. It takes
            // effect on the next session establishment, where the bridge pushes
            // it to the agent via `set_agent_sandbox_policy`.
            match crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                approval_tier: Some(cmd.tier.clone()),
                ..Default::default()
            }) {
                Ok(settings) => {
                    reply(
                        client,
                        &msg,
                        true,
                        json!({ "approvalTier": settings.approval_tier }),
                        None,
                    )
                    .await
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_session_name" => {
            match crate::agent_bridge::rename_session(cmd.session_id.clone(), cmd.name.clone())
                .await
            {
                Ok(()) => {
                    if let Ok(Some(thread)) =
                        crate::store::find_thread_by_agent_session(&cmd.session_id)
                    {
                        crate::emit_remote_activity(&thread.id);
                    }
                    reply(client, &msg, true, json!({}), None).await
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_session_pinned" => {
            match crate::store::pin_thread(crate::store::PinThreadInput {
                thread_id: cmd.thread_id.clone(),
                pinned: cmd.pinned,
            }) {
                Ok(_) => {
                    crate::emit_remote_activity(&cmd.thread_id);
                    reply(client, &msg, true, json!({}), None).await
                }
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "delete_session" => {
            // Matches the desktop single-thread delete: the session record is
            // removed (and, when it is the only owner, the agent session too),
            // but the temporary chat workspace files are kept (delete_files =
            // false). Only reachable with a non-empty thread id from the remote
            // client; a missing id is a malformed request, not a deletion.
            if cmd.thread_id.is_empty() {
                reply(client, &msg, false, Value::Null, Some("missing thread_id")).await;
            } else {
                match crate::store::delete_thread_with_files(&cmd.thread_id, false) {
                    Ok(_) => {
                        crate::emit_remote_activity(&cmd.thread_id);
                        reply(client, &msg, true, json!({}), None).await
                    }
                    Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
                }
            }
        }
        other => {
            reply(
                client,
                &msg,
                false,
                Value::Null,
                Some(&format!("Unsupported command: {other}")),
            )
            .await;
        }
    }
}

const HANDSHAKE_PROTOCOL_VERSION: u32 = 1;

/// Inputs bound into the pairing handshake transcript. Every field is an
/// `&str`, so a positional call could silently transpose two of them and sign
/// a different transcript — named fields make that a compile error instead.
struct HandshakeTranscript<'a> {
    pair_id: &'a str,
    desktop_id: &'a str,
    desktop_public_key: &'a str,
    bridge_instance_id: &'a str,
    device_id: &'a str,
    client_public_key: &'a str,
    client_nonce: &'a str,
    desktop_nonce: &'a str,
}

fn handshake_transcript(parts: &HandshakeTranscript<'_>) -> String {
    [
        "futureos-remote-handshake-v1",
        parts.pair_id,
        parts.desktop_id,
        parts.desktop_public_key,
        parts.bridge_instance_id,
        parts.device_id,
        parts.client_public_key,
        parts.client_nonce,
        parts.desktop_nonce,
    ]
    .join("\n")
}

async fn handle_pair_handshake(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    cmd: &IncomingCmd,
    state: &HandshakeState,
) {
    // The desktop key pair is derived once: a bad seed fails here, and a
    // successfully parsed pair always signs, so no later fallible step.
    let key_pair = match nkeys::KeyPair::from_seed(&state.creds.nkey_seed) {
        Ok(key_pair) => key_pair,
        Err(error) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
    };
    let desktop_public_key = key_pair.public_key();
    // Validate BEFORE deactivating commands: a garbage handshake must not lock
    // the bridge (active=false gates every command) — only a well-formed
    // handshake from a party that holds the pair's identity may suspend the
    // current session while it re-authenticates.
    let valid = cmd.protocol_version == HANDSHAKE_PROTOCOL_VERSION
        && cmd.pair_id == state.creds.pair_id
        && cmd.expected_desktop_id == state.creds.desktop_id
        && cmd.expected_desktop_public_key == desktop_public_key
        && cmd.device_id.starts_with("dev_")
        && cmd.client_public_key.starts_with('U')
        && (16..=256).contains(&cmd.client_nonce.len());
    if !valid {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_identity_mismatch"),
        )
        .await;
        return;
    }

    // Identity validated — the handshake may now suspend command processing
    // until the confirm round completes (see handle_pair_handshake_confirm).
    state.active.store(false, Ordering::Release);

    let desktop_nonce = nkeys::KeyPair::new_user().public_key();
    let transcript = handshake_transcript(&HandshakeTranscript {
        pair_id: &state.creds.pair_id,
        desktop_id: &state.creds.desktop_id,
        desktop_public_key: &desktop_public_key,
        bridge_instance_id: &state.bridge_instance_id,
        device_id: &cmd.device_id,
        client_public_key: &cmd.client_public_key,
        client_nonce: &cmd.client_nonce,
        desktop_nonce: &desktop_nonce,
    });
    let signature = URL_SAFE_NO_PAD.encode(
        key_pair
            .sign(transcript.as_bytes())
            .expect("a seed-derived KeyPair always signs"),
    );
    state.pending.lock().unwrap().clear();
    state.pending.lock().unwrap().insert(
        desktop_nonce.clone(),
        PendingHandshake {
            transcript,
            device_id: cmd.device_id.clone(),
            client_public_key: cmd.client_public_key.clone(),
        },
    );
    reply(
        client,
        msg,
        true,
        json!({
            "protocolVersion": HANDSHAKE_PROTOCOL_VERSION,
            "pairId": state.creds.pair_id,
            "desktopId": state.creds.desktop_id,
            "desktopPublicKey": desktop_public_key,
            "bridgeInstanceId": state.bridge_instance_id,
            "deviceId": cmd.device_id,
            "clientPublicKey": cmd.client_public_key,
            "clientNonce": cmd.client_nonce,
            "desktopNonce": desktop_nonce,
            "desktopSignature": signature,
        }),
        None,
    )
    .await;
}

async fn handle_pair_handshake_confirm(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    cmd: &IncomingCmd,
    state: &HandshakeState,
) {
    let pending = state.pending.lock().unwrap().remove(&cmd.desktop_nonce);
    let Some(pending) = pending else {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_challenge_expired"),
        )
        .await;
        return;
    };
    if cmd.device_id != pending.device_id {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_identity_mismatch"),
        )
        .await;
        return;
    }
    let signature = URL_SAFE_NO_PAD.decode(&cmd.client_signature).ok();
    let verified = signature
        .and_then(|signature| {
            nkeys::KeyPair::from_public_key(&pending.client_public_key)
                .ok()
                .map(|key| {
                    key.verify(pending.transcript.as_bytes(), &signature)
                        .is_ok()
                })
        })
        .unwrap_or(false);
    if !verified {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_signature_invalid"),
        )
        .await;
        return;
    }
    if !state.confirmed.load(Ordering::Acquire) {
        if let Err(error) = crate::remote::pairing::save_creds(&state.creds) {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
        state.confirmed.store(true, Ordering::Release);
    }
    state.active.store(true, Ordering::Release);
    reply(
        client,
        msg,
        true,
        json!({
            "confirmed": true,
            "pairId": state.creds.pair_id,
            "desktopId": state.creds.desktop_id,
            "bridgeInstanceId": state.bridge_instance_id,
            "deviceId": cmd.device_id,
            "desktopNonce": cmd.desktop_nonce,
            "features": ["file_transfer_v1", "approval_tier_v1", "continue_run_v1"],
            "presence": super::build_presence_payload(
                &state.creds.pair_id,
                &state.bridge_instance_id,
            ),
        }),
        None,
    )
    .await;
}

fn new_chat_thread_input() -> crate::store::CreateThreadInput {
    crate::store::CreateThreadInput {
        mode: "chat".to_string(),
        title: None,
        workspace_id: None,
        workspace_path: None,
        workspace_name: None,
        agent_session_id: None,
    }
}

/// Model ids from the agent catalogue are only unique inside their provider.
/// The Agent RPC accepts a single qualified `provider/model` value, so normalize
/// new mobile commands and keep legacy already-qualified callers working.
fn qualified_model_id(model_id: &str, provider_id: &str) -> Option<String> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return None;
    }
    let provider_id = provider_id.trim();
    if provider_id.is_empty() || model_id.contains('/') {
        Some(model_id.to_string())
    } else {
        Some(format!("{provider_id}/{model_id}"))
    }
}

/// One-shot injected failure for the prompt-prepare step (tests only): the
/// store write inside `prepare_prompt_persisted` cannot fail deterministically
/// from the outside (pooled WAL connections keep working across chmod/unlink),
/// so the claimed-attachment rollback path is exercised through this seam.
#[cfg(test)]
static INJECT_PREPARE_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// See [`INJECT_PREPARE_FAILURE`]; armed by tests, disarmed on use.
#[cfg(test)]
fn injected_prepare_failure() -> Result<(), crate::AppError> {
    INJECT_PREPARE_FAILURE
        .swap(false, std::sync::atomic::Ordering::Relaxed)
        .then(|| crate::AppError::Message("injected prepare failure".to_string()))
        .map_or(Ok(()), Err)
}

/// Build the "continue the previous task" prompt for a failed run. Mirrors the
/// desktop `buildContinuePrompt`/`loadRunResumeSummary` shape, but folds only
/// the run's recent terminal events (tool output detail lives in the GUI-side
/// summary; the store exposes the events, which is enough to resume). Sent to
/// the LLM, so the text is intentionally not localized.
fn build_continue_prompt(run_id: &str) -> String {
    let events = crate::store::list_run_events(run_id).unwrap_or_default();
    let mut lines = vec!["继续上一个任务。".to_string()];
    let terminal: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "error" | "agent_error" | "agent_end" | "tool_end" | "tool_result"
            )
        })
        .collect();
    if !terminal.is_empty() {
        lines.push(String::new());
        lines.push("已执行内容摘要:".to_string());
        for event in terminal.iter().rev().take(6).rev() {
            let payload = event.payload.as_deref().unwrap_or("");
            let truncated: String = payload.chars().take(360).collect();
            lines.push(format!("- {}: {}", event.event_type, truncated));
        }
    }
    lines.join("\n")
}

/// Find the thread for `session_id` (create a new chat thread when unknown —
/// remote policy), then persist user message + run via `agent_bridge::headless`.
async fn prepare_remote_prompt(
    session_id: &str,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    mode: String,
    workspace_id: String,
    upload_references: Vec<super::transfer::UploadReference>,
) -> Result<crate::agent_bridge::PreparedPrompt, crate::AppError> {
    let thread = match crate::store::find_thread_by_agent_session(session_id)? {
        Some(thread) => thread,
        None => {
            // Lazy creation: the thread is born with the first message, titled
            // from it (mirrors the GUI new-chat draft), and immediately gets a
            // real agent session id so the ack, the event subjects, and history
            // all agree from the start (no empty row, no id drift).
            let mut input = if mode == "workspace" {
                if workspace_id.trim().is_empty() {
                    return Err(crate::AppError::Message(
                        "Select a workspace before starting a workspace conversation.".to_string(),
                    ));
                }
                crate::store::CreateThreadInput {
                    mode: "workspace".to_string(),
                    title: None,
                    workspace_id: Some(workspace_id),
                    workspace_path: None,
                    workspace_name: None,
                    agent_session_id: None,
                }
            } else {
                new_chat_thread_input()
            };
            input.title = Some(derive_thread_title(&message));
            let mut thread = crate::store::create_thread(input)?;
            match crate::agent_bridge::provision_agent_session(
                &thread.id,
                model_id.clone(),
                thinking_level.clone(),
            )
            .await
            {
                Ok(sid) => thread.agent_session_id = Some(sid),
                Err(e) => {
                    // Thread exists but has no agent session → it would show as
                    // an orphan empty row in the GUI list. Remove it best-effort.
                    let _ = crate::store::delete_thread(&thread.id);
                    return Err(e);
                }
            }
            thread
        }
    };
    // Reject a prompt for a session that is already running BEFORE persisting
    // anything (matches GUI semantics: no follow-up/queue). The agent
    // refuses a concurrent prompt too, but only after the ack — checking here
    // keeps a busy session from accumulating a phantom user message, a failed
    // run, and a fake "Future Agent error" assistant reply. Residual race: two
    // clients prompting the same idle session within milliseconds can both pass
    // this check; the agent's is_streaming refusal stays as the backstop.
    let resolved_session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());
    if crate::store::active_run_sessions()?
        .iter()
        .any(|active| active == &resolved_session_id)
    {
        return Err(crate::AppError::Message(
            "This session is still running; wait for it to finish or abort it first.".to_string(),
        ));
    }
    let attachments = super::transfer::claim_uploads(&upload_references, &thread.id)?;
    let prepared = crate::agent_bridge::prepare_prompt_persisted(
        &thread,
        message,
        model_id,
        thinking_level,
        attachments.clone(),
    );
    #[cfg(test)]
    let prepared = prepared.and_then(|prepared| injected_prepare_failure().map(|()| prepared));
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            super::transfer::rollback_claimed(&attachments);
            return Err(error);
        }
    };
    // Notify frontend: new thread/run appeared (trigger list refresh).
    crate::emit_remote_activity(&thread.id);
    Ok(prepared)
}

/// Derive a thread title from the first message, matching the GUI new-chat
/// draft (`deriveThreadTitle`): collapse whitespace, take 28 chars, ellipsize.
/// Empty input falls back to the default chat title so the row isn't blank.
fn derive_thread_title(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = compact.trim();
    if compact.is_empty() {
        return "New Chat".to_string();
    }
    let chars: Vec<char> = compact.chars().collect();
    if chars.len() > 28 {
        format!("{}...", chars.into_iter().take(28).collect::<String>())
    } else {
        compact.to_string()
    }
}

/// Reply budget for a `get_messages` page: comfortably under NATS's 1MB
/// user-JWT payload limit, leaving headroom for the reply envelope.
const MESSAGES_PAGE_BYTES: usize = 512 * 1024;
/// A single persisted message can embed a huge tool result; cap its content so
/// one oversized message can't push a page past the payload limit on its own.
const MESSAGE_CONTENT_CAP_BYTES: usize = 256 * 1024;
/// Default page size when the client doesn't ask for one.
const DEFAULT_MESSAGE_PAGE_LIMIT: usize = 100;

/// Extract the `messages` array from an agent `get_messages` reply.
fn messages_vec(data: Value) -> Vec<Value> {
    data.get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Extract the `entries` array from an agent `get_session_entries` reply.
fn entries_vec(data: Value) -> Vec<Value> {
    data.get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn paginate_messages(messages: Vec<Value>, offset: usize, limit: usize) -> Value {
    paginate_items(messages, offset, limit, "messages")
}

/// Page a full item list into a reply that fits the NATS payload cap.
///
/// Each item is content-capped first (so no single item is huge), then items
/// are accumulated from `offset` until the serialized page would exceed
/// [`MESSAGES_PAGE_BYTES`] or `limit` is reached (always at least one item —
/// it's already capped). Returns the page (under `key`) plus cursor fields the
/// client uses to fetch the remainder.
fn paginate_items(mut items: Vec<Value>, offset: usize, limit: usize, key: &str) -> Value {
    for item in items.iter_mut() {
        truncate_message_content(item, MESSAGE_CONTENT_CAP_BYTES);
    }
    let total = items.len();
    let start = offset.min(total);
    let mut end = start;
    let mut bytes = 0usize;
    for (index, item) in items.iter().skip(start).enumerate() {
        let size = serde_json::to_vec(item)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        if index > 0 && (index >= limit || bytes + size > MESSAGES_PAGE_BYTES) {
            break;
        }
        bytes += size;
        end += 1;
    }
    let page: Vec<Value> = items.drain(start..end).collect();
    let mut value = json!({
        "offset": start,
        "nextOffset": end,
        "total": total,
        "hasMore": end < total,
    });
    value[key] = json!(page);
    value
}

/// Page a session's replay event tail into a reply that fits the NATS payload
/// cap, mirroring `paginate_items` (each event's `data` is capped, then events
/// accumulate until the page would exceed [`MESSAGES_PAGE_BYTES`]). The reply
/// keeps the envelope's non-event fields (`runId`, `projection`, `truncated`)
/// on every page so the client can distinguish a ring-overflow projection from
/// a plain tail replay regardless of which page it lands on.
fn paginate_events(mut data: Value, offset: usize, limit: usize) -> Value {
    let run_id = data.get("runId").cloned().unwrap_or(Value::Null);
    let projection = data.get("projection").cloned().unwrap_or(Value::Null);
    let truncated = data.get("truncated").cloned().unwrap_or(Value::Null);
    let events = data
        .get_mut("events")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();
    let mut page = paginate_items(events, offset, limit, "events");
    if !run_id.is_null() {
        page["runId"] = run_id;
    }
    if !projection.is_null() {
        page["projection"] = projection;
    }
    if !truncated.is_null() {
        page["truncated"] = truncated;
    }
    page
}

/// Cap the serialized size of a single message by truncating its `content`
/// (a string or an array of `{type:"text", text}` blocks). Non-text blocks
/// (tool_use etc.) are left intact so the shape stays renderable.
fn truncate_message_content(message: &mut Value, cap: usize) {
    let oversized = serde_json::to_vec(message)
        .map(|bytes| bytes.len() > cap)
        .unwrap_or(false);
    if !oversized {
        return;
    }
    // Replay events carry their payload in `data` (a JSON string), not
    // `content` — a single oversized event (e.g. a multi-MB tool result kept
    // verbatim in the journal) would otherwise page out whole and be silently
    // dropped by the relay, failing every reconcile on that session (H2
    // residual). Mirror the live path (`cap_event_data`): swap the oversized
    // `data` for a `_truncated` placeholder the client reducer consumes.
    if message.get("content").is_none() {
        if let Some(Value::String(data)) = message.get_mut("data") {
            if data.len() > cap {
                *data = format!(
                    r#"{{"_truncated":true,"bytes":{},"note":"event exceeded the relay payload limit and was truncated; full content is available via get_messages"}}"#,
                    data.len()
                );
            }
        }
        return;
    }
    let content = message
        .get_mut("content")
        .expect("content presence checked above");
    match content {
        Value::String(text) => {
            let (end, truncated) = byte_cut(text, cap);
            if truncated {
                let mut cut = text[..end].to_string();
                cut.push_str("\n\n[…内容过长，远程端已截断，完整内容见本机会话…]");
                *text = cut;
            }
        }
        Value::Array(blocks) => {
            let mut remaining = cap;
            for block in blocks.iter_mut() {
                if remaining == 0 {
                    break;
                }
                let is_text = block.get("type").and_then(Value::as_str) == Some("text");
                if !is_text {
                    continue;
                }
                if let Some(Value::String(text)) = block.get_mut("text") {
                    let (end, truncated) = byte_cut(text, remaining);
                    if truncated {
                        let mut cut = text[..end].to_string();
                        cut.push('…');
                        *text = cut;
                        remaining = 0;
                    } else {
                        remaining = remaining.saturating_sub(text.len());
                    }
                }
            }
        }
        _ => {}
    }
}

/// Return a byte index at a char boundary, not exceeding `max_bytes`, and
/// whether the string had to be cut.
fn byte_cut(text: &str, max_bytes: usize) -> (usize, bool) {
    if text.len() <= max_bytes {
        return (text.len(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (end, true)
}

/// Send a unified request-reply response (in `RpcResponse` shape), and flush to ensure timely delivery.
async fn reply(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    success: bool,
    data: Value,
    error: Option<&str>,
) {
    if msg.reply.is_none() {
        return;
    }
    let body = json!({
        "type": "response",
        "success": success,
        "data": data,
        "error": error,
    });
    // A serde_json::Value always serializes, so this cannot fail.
    let payload = serde_json::to_vec(&body).expect("a response Value always serializes");
    let _ = REPLY_CAPTURE.try_with(|capture| {
        *capture.lock().unwrap() = Some(payload.clone());
    });
    publish_reply_payload(client, msg, payload).await;
}

/// Reply `{}` on success or the error text on failure — the shared shape for
/// every unit-result command (abort, set_model, ...).
async fn reply_unit(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    result: Result<(), crate::AppError>,
) {
    match result {
        Ok(()) => reply(client, msg, true, json!({}), None).await,
        Err(error) => reply(client, msg, false, Value::Null, Some(&error.to_string())).await,
    }
}

async fn publish_reply_payload(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    payload: Vec<u8>,
) {
    let Some(reply_subject) = msg.reply.clone() else {
        return;
    };
    let _ = client.publish(reply_subject, payload.into()).await;
    let _ = client.flush().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_transcript_binds_both_device_identities_and_nonces() {
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: "pair_1",
            desktop_id: "desktop_1",
            desktop_public_key: "UDESKTOP",
            bridge_instance_id: "bridge_1",
            device_id: "dev_1",
            client_public_key: "UCLIENT",
            client_nonce: "client_nonce",
            desktop_nonce: "desktop_nonce",
        });
        assert_eq!(
            transcript,
            "futureos-remote-handshake-v1\npair_1\ndesktop_1\nUDESKTOP\nbridge_1\ndev_1\nUCLIENT\nclient_nonce\ndesktop_nonce"
        );
        assert_ne!(
            transcript,
            handshake_transcript(&HandshakeTranscript {
                pair_id: "pair_1",
                desktop_id: "desktop_other",
                desktop_public_key: "UDESKTOP",
                bridge_instance_id: "bridge_1",
                device_id: "dev_1",
                client_public_key: "UCLIENT",
                client_nonce: "client_nonce",
                desktop_nonce: "desktop_nonce",
            })
        );
    }

    #[test]
    fn handshake_signature_rejects_tampered_transcript() {
        let desktop = nkeys::KeyPair::new_user();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: "pair_1",
            desktop_id: "desktop_1",
            desktop_public_key: &desktop.public_key(),
            bridge_instance_id: "bridge_1",
            device_id: "dev_1",
            client_public_key: "UCLIENT",
            client_nonce: "client_nonce",
            desktop_nonce: "desktop_nonce",
        });
        let signature = desktop.sign(transcript.as_bytes()).unwrap();
        let verifier = nkeys::KeyPair::from_public_key(&desktop.public_key()).unwrap();
        assert!(verifier.verify(transcript.as_bytes(), &signature).is_ok());
        assert!(verifier
            .verify(format!("{transcript}_tampered").as_bytes(), &signature)
            .is_err());
    }

    fn text_message(text: &str) -> Value {
        json!({ "role": "assistant", "content": text })
    }

    #[test]
    fn paginate_small_list_is_one_page() {
        let messages = vec![text_message("a"), text_message("b"), text_message("c")];
        let page = paginate_messages(messages, 0, 100);
        assert_eq!(page["messages"].as_array().unwrap().len(), 3);
        assert_eq!(page["offset"], 0);
        assert_eq!(page["nextOffset"], 3);
        assert_eq!(page["total"], 3);
        assert_eq!(page["hasMore"], false);
    }

    #[test]
    fn paginate_respects_limit_and_cursors() {
        let messages = vec![text_message("a"), text_message("b"), text_message("c")];
        let first = paginate_messages(messages.clone(), 0, 2);
        assert_eq!(first["messages"].as_array().unwrap().len(), 2);
        assert_eq!(first["nextOffset"], 2);
        assert_eq!(first["hasMore"], true);
        let second = paginate_messages(messages, 2, 2);
        assert_eq!(second["messages"].as_array().unwrap().len(), 1);
        assert_eq!(second["nextOffset"], 3);
        assert_eq!(second["hasMore"], false);
    }

    #[test]
    fn paginate_bounds_by_byte_budget() {
        // ~100KB messages; a 512KB budget fits ~5 of them, forcing a second page.
        let big = "x".repeat(100 * 1024);
        let messages: Vec<Value> = (0..6).map(|_| text_message(&big)).collect();
        let page = paginate_messages(messages, 0, 100);
        let arr = page["messages"].as_array().unwrap();
        let arr_len = arr.len();
        assert!(
            arr_len < 6,
            "expected byte budget to cap the page, got {arr_len}"
        );
        assert_eq!(page["hasMore"], true);
        // The page itself stays comfortably under the 1MB NATS payload cap.
        let size = serde_json::to_vec(&page).map(|b| b.len()).unwrap();
        assert!(size < 1024 * 1024, "page too large: {size}");
    }

    #[test]
    fn paginate_caps_and_includes_oversized_message() {
        // A message larger than the page budget is content-capped (cap < budget)
        // so it fits, and the page never exceeds the payload cap.
        let huge = "y".repeat(MESSAGES_PAGE_BYTES + 1024);
        let messages = vec![text_message(&huge), text_message("small")];
        let page = paginate_messages(messages, 0, 100);
        let arr = page["messages"].as_array().unwrap();
        assert!(!arr.is_empty());
        // The oversized message's content was truncated to the cap.
        let content = arr[0]["content"].as_str().unwrap();
        assert!(content.len() <= MESSAGE_CONTENT_CAP_BYTES + 128);
        let size = serde_json::to_vec(&page).map(|b| b.len()).unwrap();
        assert!(size < 1024 * 1024, "page too large: {size}");
    }

    #[test]
    fn truncate_caps_string_content() {
        let mut message = text_message(&"z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2));
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let content = message["content"].as_str().unwrap();
        assert!(content.len() <= MESSAGE_CONTENT_CAP_BYTES + 128);
        assert!(content.contains("截断"));
    }

    #[test]
    fn truncate_caps_text_blocks_and_keeps_others() {
        let mut message = json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "a".repeat(MESSAGE_CONTENT_CAP_BYTES * 2) },
                { "type": "tool_use", "id": "t1", "name": "shell" },
            ]
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let blocks = message["content"].as_array().unwrap();
        // Tool block untouched.
        assert_eq!(blocks[1]["type"], "tool_use");
        assert_eq!(blocks[1]["name"], "shell");
        // Text block truncated.
        let text = blocks[0]["text"].as_str().unwrap();
        assert!(text.len() <= MESSAGE_CONTENT_CAP_BYTES + 8);
    }

    #[test]
    fn truncate_leaves_small_messages_alone() {
        let mut message = text_message("small");
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        assert_eq!(message["content"], "small");
    }

    #[test]
    fn truncate_skips_messages_without_a_content_field() {
        // Oversized but with no `content` key → the let-else early-returns.
        let mut message = json!({
            "role": "assistant",
            "tool_use": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2),
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        assert!(message.get("content").is_none());
    }

    #[test]
    fn truncate_skips_non_text_blocks_and_fits_small_ones() {
        // A non-text block first (continue), then a small text block that fits
        // (the remaining-subtract path), then an oversized one.
        let mut message = json!({
            "role": "assistant",
            "content": [
                { "type": "tool_use", "id": "t0", "name": "shell" },
                { "type": "text", "text": "small" },
                { "type": "text", "text": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2) },
            ]
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let blocks = message["content"].as_array().unwrap();
        assert_eq!(blocks[0]["name"], "shell"); // untouched
        assert_eq!(blocks[1]["text"], "small"); // fits, untouched
        let text = blocks[2]["text"].as_str().unwrap();
        assert!(text.len() <= MESSAGE_CONTENT_CAP_BYTES + 8);
    }

    #[test]
    fn truncate_ignores_non_string_non_array_content() {
        // Oversized but content is a scalar → the `_ => {}` arm.
        let mut message = json!({
            "role": "assistant",
            "content": 42,
            "pad": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2),
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        assert_eq!(message["content"], 42);
    }

    #[test]
    fn truncate_skips_text_blocks_without_a_string_text() {
        // A block claiming `type: "text"` but carrying a non-string `text` is
        // left intact (the text-block match's `_ => {}` arm).
        let mut message = json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": 42 },
                { "type": "text", "text": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2) },
            ]
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let blocks = message["content"].as_array().unwrap();
        assert_eq!(blocks[0]["text"], 42); // untouched
        let text = blocks[1]["text"].as_str().unwrap();
        assert!(text.len() <= MESSAGE_CONTENT_CAP_BYTES + 8);
    }

    #[test]
    fn byte_cut_is_char_boundary_safe() {
        let s = "中文内容"; // multi-byte chars
        let (end, truncated) = byte_cut(s, 4);
        assert!(s.is_char_boundary(end));
        assert!(truncated);
        let (end, truncated) = byte_cut(s, 1024);
        assert_eq!(end, s.len());
        assert!(!truncated);
    }

    #[test]
    fn paginate_events_pages_a_large_tail() {
        // A multi-MB replay tail must page instead of shipping as one reply.
        let big = "y".repeat(100 * 1024);
        let events: Vec<Value> = (0..6)
            .map(|i| json!({ "type": "text_chunk", "run_id": "run-1", "idx": i, "data": big }))
            .collect();
        let data = json!({ "runId": "run-1", "events": events });
        let first = paginate_events(data, 0, 100);
        let arr = first["events"].as_array().unwrap();
        let arr_len = arr.len();
        assert!(
            arr_len < 6,
            "byte budget should split the tail, got {arr_len}"
        );
        assert_eq!(first["runId"], "run-1");
        assert_eq!(first["hasMore"], true);
        let size = serde_json::to_vec(&first).map(|b| b.len()).unwrap();
        assert!(size < 1024 * 1024, "page too large: {size}");
    }

    #[test]
    fn paginate_events_carries_projection_on_first_page() {
        let events = vec![json!({ "type": "text_chunk", "run_id": "run-1", "idx": 0 })];
        let data = json!({
            "runId": "run-1",
            "events": events,
            "projection": { "run_id": "run-1", "cursor": 42, "events": [] },
        });
        let page = paginate_events(data, 0, 100);
        assert_eq!(page["projection"]["cursor"], 42);
        assert_eq!(page["events"].as_array().unwrap().len(), 1);
        assert_eq!(page["hasMore"], false);
    }

    #[test]
    fn paginate_events_carries_truncated_flag() {
        let events = vec![json!({ "type": "text_chunk", "run_id": "run-1", "idx": 0 })];
        let data = json!({
            "runId": "run-1",
            "events": events,
            "truncated": true,
        });
        let page = paginate_events(data, 0, 100);
        assert_eq!(page["truncated"], json!(true));
        assert_eq!(page["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn paginate_events_truncates_a_single_oversized_event_data() {
        // A single journal event larger than the relay payload cap (a multi-MB
        // tool result) must not page out whole — the "at least one item per
        // page" rule would otherwise ship it intact and the relay silently
        // drops it, failing every reconcile on that session (H2 residual).
        let huge = "z".repeat(3 * 1024 * 1024);
        let events =
            vec![json!({ "type": "tool_result", "run_id": "run-1", "idx": 0, "data": huge })];
        let data = json!({ "runId": "run-1", "events": events });
        let page = paginate_events(data, 0, 100);
        let arr = page["events"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "the single event still pages through");
        let data_str = arr[0]["data"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(data_str).unwrap();
        assert_eq!(
            parsed["_truncated"], true,
            "oversized event data must be swapped for the _truncated marker"
        );
        // The page must now fit well under the relay cap.
        let size = serde_json::to_vec(&page).map(|b| b.len()).unwrap();
        assert!(
            size < 1024 * 1024,
            "page too large after truncation: {size}"
        );
    }

    #[test]
    fn truncate_swaps_oversized_event_data_but_keeps_small() {
        let mut big = json!({ "type": "tool_result", "run_id": "r", "idx": 0, "data": "x".repeat(MESSAGE_CONTENT_CAP_BYTES + 10) });
        truncate_message_content(&mut big, MESSAGE_CONTENT_CAP_BYTES);
        let parsed: Value = serde_json::from_str(big["data"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["_truncated"], true);

        let mut small = json!({ "type": "text_chunk", "run_id": "r", "idx": 1, "data": "ok" });
        truncate_message_content(&mut small, MESSAGE_CONTENT_CAP_BYTES);
        assert_eq!(small["data"], "ok");
    }
}

#[cfg(test)]
mod bridge_tests {
    #![allow(clippy::await_holding_lock)]
    use super::super::test_support::{
        await_publish, ensure_mock_agent, init_store, jwt, mock_agent_lock, nats_connect,
        nats_connect_once, now_secs, unique, FakeNats, HomeGuard,
    };
    use super::super::transfer;
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn thread_title_derivation_matches_the_gui_draft() {
        // Whitespace-only / empty input falls back to the default chat title.
        assert_eq!(derive_thread_title(""), "New Chat");
        assert_eq!(derive_thread_title("  \n\t  "), "New Chat");
        // Whitespace collapses; 28 chars is the cut, ellipsized beyond it.
        assert_eq!(
            derive_thread_title("hello   there\nworld"),
            "hello there world"
        );
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(derive_thread_title(long), "abcdefghijklmnopqrstuvwxyz01...");
    }

    fn bridge_creds() -> crate::remote::pairing::PairingCreds {
        let key_pair = nkeys::KeyPair::new_user();
        crate::remote::pairing::PairingCreds {
            handshake_version: 1,
            pair_id: format!("pair_{}", unique("cmd")),
            desktop_id: format!("desktop_{}", unique("cmd")),
            nkey_seed: key_pair.seed().unwrap().to_string(),
            user_jwt: jwt(now_secs() + 3600),
            nats_url: "nats://127.0.0.1:1".to_string(),
            nats_ws_url: "ws://127.0.0.1:1".to_string(),
            jwt_expires_at: now_secs() + 3600,
        }
    }

    /// A running command loop against a fake NATS: returns the client handle a
    /// test drives, plus the pieces it may need to poke.
    struct Bridge {
        client: async_nats::Client,
        nats: FakeNats,
        pair_id: String,
        handshake: HandshakeState,
        loop_handle: tokio::task::JoinHandle<()>,
    }

    impl Bridge {
        async fn start() -> Self {
            let nats = FakeNats::start().await;
            let client = nats_connect(&nats).await;
            let creds = bridge_creds();
            let pair_id = creds.pair_id.clone();
            let handshake = HandshakeState::new(
                creds,
                Arc::new(AtomicBool::new(false)),
                format!("bridge_{}", unique("cmd")),
            );
            let reply_slots = new_reply_slots();
            let loop_handle = tokio::spawn(command_loop(
                client.clone(),
                pair_id.clone(),
                reply_slots.clone(),
                handshake.clone(),
            ));
            nats.wait_for_sub(&format!("p.{pair_id}.cmd.>"), Duration::from_secs(5))
                .await;
            Bridge {
                client,
                nats,
                pair_id,
                handshake,
                loop_handle,
            }
        }

        /// Activate the bridge (as a completed handshake would).
        fn activate(&self) {
            self.handshake.active_flag().store(true, Ordering::Release);
        }

        /// Send a command and await its reply envelope.
        async fn call(&self, cmd: Value) -> Value {
            let subject = format!("p.{}.cmd.rpc", self.pair_id);
            let message = self
                .client
                .request(subject, serde_json::to_vec(&cmd).unwrap().into())
                .await
                .expect("bridge reply");
            serde_json::from_slice(&message.payload).expect("reply is JSON")
        }

        fn stop(self) {
            self.loop_handle.abort();
        }
    }

    fn handshake_cmd(
        creds: &crate::remote::pairing::PairingCreds,
        client_key: &nkeys::KeyPair,
    ) -> Value {
        json!({
            "id": unique("cmd"),
            "type": "pair_handshake",
            "protocolVersion": 1,
            "pairId": creds.pair_id,
            "deviceId": "dev_test",
            "clientPublicKey": client_key.public_key(),
            "clientNonce": "nonce-0123456789abcdef",
            "expectedDesktopId": creds.desktop_id,
            "expectedDesktopPublicKey": crate::remote::pairing::public_key(creds).unwrap(),
        })
    }

    /// A corrupted desktop NKey seed fails the handshake before any signing:
    /// the client gets a failure reply and the bridge stays inactive.
    #[tokio::test]
    async fn handshake_rejects_a_bad_desktop_seed() {
        let _home = HomeGuard::new("cmd-bad-seed");
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        let mut creds = bridge_creds();
        creds.nkey_seed = "not-a-valid-seed".to_string();
        let handshake = HandshakeState::new(
            creds,
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        );
        let reply_subject = format!("rep_{}", unique("hs"));
        let mut tap = nats.tap();
        let msg = async_nats::Message {
            subject: "p.pair.cmd.pair_handshake".into(),
            reply: Some(reply_subject.clone().into()),
            payload: Vec::new().into(),
            headers: None,
            status: None,
            description: None,
            length: 0,
        };
        let cmd = IncomingCmd {
            cmd_type: "pair_handshake".to_string(),
            ..Default::default()
        };
        handle_pair_handshake(&client, &msg, &cmd, &handshake).await;
        let reply = await_publish(&mut tap, &reply_subject, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(false));
        assert!(!handshake.active_flag().load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn rejects_garbage_and_requires_handshake() {
        let _home = HomeGuard::new("cmd-gate");
        let bridge = Bridge::start().await;

        // Unparseable payload → error reply.
        let mut tap = bridge.nats.tap();
        let reply_subject = format!("rep-{}", unique("garbage"));
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            Some(&reply_subject),
            b"{not json".to_vec(),
        );
        let reply = await_publish(&mut tap, &reply_subject, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(false));
        assert!(reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("Failed to parse command JSON"));

        // A well-formed non-handshake command before activation is refused.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert_eq!(reply["error"], json!("pairing_handshake_required"));

        // A command with no reply subject is processed but not answered.
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            None,
            b"{ not json either".to_vec(),
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        bridge.stop();
    }

    #[tokio::test]
    async fn handshake_roundtrip_activates_the_bridge() {
        let _home = HomeGuard::new("cmd-handshake");
        init_store();
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.creds.clone();

        // Identity mismatch → refused.
        let mut bad = handshake_cmd(&creds, &client_key);
        bad["protocolVersion"] = json!(99);
        let reply = bridge.call(bad).await;
        assert_eq!(reply["error"], json!("pairing_identity_mismatch"));

        // Valid challenge → desktop nonce + signature.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let data = &reply["data"];
        assert_eq!(data["pairId"], json!(creds.pair_id));
        let desktop_nonce = data["desktopNonce"].as_str().unwrap().to_string();
        // The desktop signature verifies against the transcript.
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(data["desktopSignature"].as_str().unwrap())
            .unwrap();
        nkeys::KeyPair::from_public_key(&crate::remote::pairing::public_key(&creds).unwrap())
            .unwrap()
            .verify(transcript.as_bytes(), &signature)
            .unwrap();

        // Unknown challenge → expired.
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": "never-issued",
                "clientSignature": "x",
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_challenge_expired"));

        // A forged client signature → rejected.
        let forged = client_key.sign(b"a different transcript entirely").unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(forged),
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_signature_invalid"));

        // A fresh challenge + correctly signed transcript activates the bridge
        // and persists the credential.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["confirmed"], json!(true));
        assert_eq!(
            reply["data"]["features"],
            json!(["file_transfer_v1", "approval_tier_v1", "continue_run_v1"])
        );
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));
        assert!(crate::remote::pairing::load_creds().is_some());

        // Now ordinary commands pass the gate.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // A second handshake resets activity until reconfirmed — and the
        // reconfirm skips the credential save (already persisted above).
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        assert!(!bridge.handshake.active_flag().load(Ordering::Acquire));
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));
        bridge.stop();
    }

    #[tokio::test]
    async fn handshake_confirm_rejects_a_device_mismatch() {
        let _home = HomeGuard::new("cmd-device-mismatch");
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.creds.clone();
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_other",
                "desktopNonce": desktop_nonce,
                "clientSignature": "eA",
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_identity_mismatch"));
        bridge.stop();
    }

    #[tokio::test]
    async fn handshake_confirm_reports_credential_save_failures() {
        let home = HomeGuard::new("cmd-save-fail");
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.creds.clone();
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        // No HOME → the credential persist fails and the error surfaces.
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        drop(home);
        bridge.stop();
    }

    /// Activated bridge with a store and mock agent behind it.
    async fn active_bridge(label: &str) -> (HomeGuard, Bridge) {
        let home = HomeGuard::new(label);
        init_store();
        ensure_mock_agent();
        let bridge = Bridge::start().await;
        bridge.activate();
        (home, bridge)
    }

    #[tokio::test]
    async fn presence_and_catalog_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-presence").await;

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_presence" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["online"], json!(true));
        assert_eq!(reply["data"]["pairId"], json!(bridge.pair_id));

        // Empty store → empty session/workspace lists.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["data"]["sessions"], json!([]));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_workspaces" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // A thread with an agent session shows up, streaming while its run is
        // active.
        let session = unique("sess");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("From remote".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        let sessions = reply["data"]["sessions"].as_array().unwrap();
        let row = sessions
            .iter()
            .find(|row| row["sessionId"] == json!(session))
            .expect("thread listed");
        assert_eq!(row["title"], json!("From remote"));
        assert_eq!(row["streaming"], json!(true));

        bridge.stop();
    }

    #[tokio::test]
    async fn catalog_commands_report_store_failures() {
        let _lock = mock_agent_lock();
        // No init_store and no .future dir → the DB connect fails.
        let _home = HomeGuard::new("cmd-store-down");
        ensure_mock_agent();
        let bridge = Bridge::start().await;
        bridge.activate();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_workspaces" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        // Settings handlers report the same store failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        bridge.stop();
    }

    #[tokio::test]
    async fn history_commands_page_and_fail() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-history").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["total"], json!(2));
        assert_eq!(reply["data"]["hasMore"], json!(false));

        // An explicit positive limit pages instead of using the default.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session, "limit": 1 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(reply["data"]["hasMore"], json!(true));

        // Agent failure → the remote-specific error text.
        agent.script_for(
            "get_messages",
            &session,
            false,
            json!(null),
            "agent exploded",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("conversation history is unavailable"));

        // Entries page through too.
        agent.set_session_entries(
            &session,
            json!({ "entries": [{ "entryType": "user", "content": "hi" }] }),
        );
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);

        // Entries honor an explicit positive limit too.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session, "limit": 5 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);
        agent.script_for("get_session_entries", &session, false, json!(null), "nope");
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));

        // Events backfill (success and failure).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["events"], json!([]));
        // A positive limit uses the caller's page size (limit > 0 branch).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1, "limit": 5 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["events"], json!([]));
        agent.script_for(
            "get_events_since",
            &session,
            false,
            json!(null),
            "stale run",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1 }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop();
    }

    #[tokio::test]
    async fn transfer_control_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-transfer").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        // upload_init validation + success.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "a.txt", "kind": "file", "originalSize": 0, "transferSize": 0 }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "a.txt", "kind": "file", "originalSize": 4, "transferSize": 4 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        let upload_id = reply["data"]["uploadId"].as_str().unwrap().to_string();

        // upload_complete on an incomplete upload → error; after the bytes, ok.
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "upload_complete", "transferId": upload_id }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));
        transfer::write_upload_chunk(&upload_id, 0, b"data").unwrap();
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "upload_complete", "transferId": upload_id }),
            )
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["contentHash"].as_str().unwrap().len(), 64);

        // upload_cancel always succeeds.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_cancel", "transferId": upload_id }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // download_prepare failure (unknown attachment) and success.
        let session = unique("sess");
        agent.set_session_entries(&session, json!({ "entries": [] }));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_prepare", "sessionId": session, "filePath": "/tmp/none.txt" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let dir = std::env::temp_dir().join(unique("futureos-cmd-dl"));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("take.txt");
        std::fs::write(&file, b"take me").unwrap();
        agent.set_session_entries(
            &session,
            json!({"entries":[{"meta":{"attachments":[{"path": file.to_string_lossy()}]}}]}),
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_prepare", "sessionId": session, "filePath": file.to_string_lossy() }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let transfer_id = reply["data"]["transferId"].as_str().unwrap().to_string();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_cancel", "transferId": transfer_id }))
            .await;
        assert_eq!(reply["success"], json!(true));

        std::fs::remove_dir_all(dir).ok();
        bridge.stop();
    }

    #[tokio::test]
    async fn prompt_creates_threads_and_rejects_busy_sessions() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-prompt").await;

        // Workspace mode without a workspace id → validation error.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "message": "hi", "mode": "workspace" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("Select a workspace"));

        // Chat mode with an empty session id → lazy thread + agent session.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "message": "hello there", "modelId": "m1", "providerId": "p1", "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let session = reply["data"]["sessionId"].as_str().unwrap().to_string();
        assert!(session.starts_with("mock-session-"));
        let thread_id = reply["data"]["threadId"].as_str().unwrap().to_string();
        // The run the ack carried settles as failed: the mock agent's event
        // stream ends without agent_end.
        let run_id = reply["data"]["runId"].as_str().unwrap().to_string();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let run = crate::store::get_run(&run_id).unwrap().expect("run row");
            if run.status != "running" {
                assert_eq!(run.status, "failed");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "run never settled");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // A follow-up prompt on the idle session reuses the thread.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "sessionId": session, "message": "again" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["threadId"], json!(thread_id));

        // While a run for the session is active, another prompt is refused.
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread_id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "sessionId": session, "message": "too soon" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("still running"));

        bridge.stop();
    }

    #[tokio::test]
    async fn prompt_workspace_mode_and_prepare_failures() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-prompt-ws").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        // Workspace mode with a real workspace id → thread bound to it.
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-prompt"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Prompt WS".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "prompt", "message": "workspace hello",
                "mode": "workspace", "workspaceId": workspace.id,
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let ws_thread = reply["data"]["threadId"].as_str().unwrap().to_string();
        assert_eq!(
            crate::store::list_threads()
                .unwrap()
                .iter()
                .find(|t| t.id == ws_thread)
                .map(|t| t.mode.as_str()),
            Some("workspace")
        );

        // Agent-side session provisioning failure → the just-created orphan
        // thread is removed and the error surfaces.
        agent.script(
            "new_session",
            false,
            json!(null),
            "agent rejected the session",
        );
        let before = crate::store::list_threads().unwrap().len();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "message": "fail provisioning" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("rejected"));
        assert_eq!(crate::store::list_threads().unwrap().len(), before);

        // Prepare failure after attachments were claimed → the claimed copies
        // roll back (no leaked files under the thread's image dir).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "note.txt", "kind": "file", "originalSize": 3, "transferSize": 3 }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let upload_id = reply["data"]["uploadId"].as_str().unwrap().to_string();
        transfer::write_upload_chunk(&upload_id, 0, b"hey").unwrap();
        transfer::complete_upload(&upload_id).unwrap();
        INJECT_PREPARE_FAILURE.store(true, Ordering::Relaxed);
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "prompt", "message": "with attachment",
                "attachments": [{ "uploadId": upload_id }],
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("injected prepare failure"));
        let thread = crate::store::list_threads()
            .unwrap()
            .into_iter()
            .find(|t| t.title == "with attachment")
            .expect("thread for the failed prepare");
        let origin = crate::store::thread_images_dir(&thread.id)
            .unwrap()
            .join("origin");
        let leaked = origin
            .read_dir()
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
        assert!(!leaked, "claimed attachment copies must roll back");

        std::fs::remove_dir_all(&workspace_dir).ok();
        bridge.stop();
    }

    #[tokio::test]
    async fn session_control_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-session-ctl").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        // abort: success and agent failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "abort", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        agent.script_for("abort", &session, false, json!(null), "cannot abort");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "abort", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // get_state.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_state", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for("get_state", &session, false, json!(null), "gone");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_state", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // list_models / get_available_models share a handler.
        for cmd_type in ["list_models", "get_available_models"] {
            let reply = bridge
                .call(json!({ "id": unique("cmd"), "type": cmd_type }))
                .await;
            assert_eq!(reply["success"], json!(true), "{cmd_type}: {reply}");
        }
        agent.script("list_models", false, json!(null), "no models");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_models" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // set_model / set_thinking_level / set_session_name (success + failure).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_model", "sessionId": session, "modelId": "m1", "providerId": "p1" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert!(agent.served("set_model", &session));
        agent.script_for("set_model", &session, false, json!(null), "bad model");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_model", "sessionId": session, "modelId": "m1" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_thinking_level", "sessionId": session, "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for(
            "set_thinking_level",
            &session,
            false,
            json!(null),
            "bad level",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_thinking_level", "sessionId": session, "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for(
            "set_session_name",
            &session,
            false,
            json!(null),
            "no rename",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // A rename that resolves to a GUI thread emits remote activity
        // (the Ok(Some) find_thread_by_agent_session branch).
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Phone".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // set_session_pinned: success and unknown-thread failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_pinned", "threadId": thread.id, "pinned": true }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_pinned", "threadId": "missing-thread", "pinned": false }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // delete_session: missing id, success, and unknown-thread failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("missing thread_id"));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session", "threadId": thread.id }))
            .await;
        assert_eq!(reply["success"], json!(true));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session", "threadId": "missing-thread" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // Unknown command type.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "teleport", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported command"));

        bridge.stop();
    }

    #[tokio::test]
    async fn settings_commands_read_and_update() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-settings").await;

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("off"));
        assert_eq!(
            reply["data"]["sandboxAvailable"],
            json!(cfg!(target_os = "macos"))
        );

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("sandbox"));

        bridge.stop();
    }

    #[tokio::test]
    async fn continue_run_resumes_a_failed_run_and_rejects_busy_sessions() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue").await;
        let agent = ensure_mock_agent();
        let session = unique("sess");

        // A thread bound to the session + a failed run with terminal events.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Continue".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-failed".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "tool_result".to_string(),
            payload: Some(r#"{"text":"did the thing"}"#.to_string()),
            sequence: 1,
        })
        .unwrap();
        crate::store::flush_run_event_log_for_test(&run.id);
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "failed".to_string(),
            error_message: None,
            error_type: None,
        })
        .unwrap();

        // Reject the Ok arm's spawned `run_prepared_prompt` at session
        // provisioning (new_session) so it fails BEFORE spawning a session
        // observer — the ack path (prepare_remote_prompt) is store-only and
        // unaffected. This keeps the shared mock script clean for later tests
        // (no zombie) without touching the process-global agent endpoint.
        agent.script("new_session", false, json!(null), "rejected");

        // Ok arm: continue the failed run → ack carries the fresh run ids.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": run.id }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["sessionId"], json!(session));
        assert_eq!(reply["data"]["threadId"], json!(thread.id));

        // The spawned pipeline settles the new run as failed (its session
        // provisioning is rejected) — poll until the run leaves the running state.
        let continued_run = reply["data"]["runId"].as_str().unwrap().to_string();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let row = crate::store::get_run(&continued_run)
                .unwrap()
                .expect("continued run");
            if row.status != "running" {
                assert_eq!(row.status, "failed");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "continued run never settled"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        bridge.stop();
    }

    #[tokio::test]
    async fn continue_run_rejects_a_busy_session() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue-busy").await;
        let session = unique("sess");

        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Busy".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        // An active run on the session → the busy guard fires before anything
        // is persisted or spawned.
        crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-active".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": "run-active" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("still running"));

        bridge.stop();
    }

    #[tokio::test]
    async fn build_continue_prompt_folds_recent_terminal_events() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-continue-prompt");
        init_store();
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Prompt".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        // A non-terminal event is ignored; terminal events are folded newest-first.
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "text_chunk".to_string(),
            payload: Some("ignored".to_string()),
            sequence: 1,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "tool_result".to_string(),
            payload: Some("tool output".to_string()),
            sequence: 2,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "error".to_string(),
            payload: Some("boom".to_string()),
            sequence: 3,
        })
        .unwrap();
        crate::store::flush_run_event_log_for_test(&run.id);

        let prompt = build_continue_prompt(&run.id);
        assert!(prompt.contains("继续上一个任务。"), "{prompt}");
        assert!(prompt.contains("已执行内容摘要:"), "{prompt}");
        assert!(prompt.contains("error"), "{prompt}");
        assert!(prompt.contains("tool_result"), "{prompt}");
        assert!(
            !prompt.contains("text_chunk"),
            "non-terminal ignored: {prompt}"
        );

        // An unknown run (no events) still yields the default continue prompt.
        let empty = build_continue_prompt("no-such-run");
        assert_eq!(empty, "继续上一个任务。");
    }

    #[tokio::test]
    async fn approval_decision_ownership_and_outcomes() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-approval").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        // Unknown approval request.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "nope", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("could not be loaded"));

        // A real approval owned by a DIFFERENT session → ownership mismatch.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-1".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-1".to_string()),
            kind: "command".to_string(),
            title: "Run ls".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": "someone-else", "entryId": "appr-1", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("does not belong"));

        // Owning session decides → the agent is notified and the reply is ok.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-1", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(agent.served("approval_decision", &session));
        let record = crate::store::get_approval_request("appr-1")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "approved");

        // An agent-side stale approval cancels locally but still replies ok.
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-2".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-2".to_string()),
            kind: "command".to_string(),
            title: "Run pwd".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();
        agent.script_for(
            "approval_decision",
            &session,
            false,
            json!(null),
            "approval request is not pending",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-2", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let record = crate::store::get_approval_request("appr-2")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "cancelled");

        // A genuine agent rejection surfaces as an error.
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-3".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-3".to_string()),
            kind: "command".to_string(),
            title: "Run rm".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();
        agent.script_for("approval_decision", &session, false, json!(null), "boom");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-3", "mode": "denied" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop();
    }

    #[tokio::test]
    async fn duplicate_command_ids_get_one_execution_and_cached_replies() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-singleflight").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        let command_id = unique("cmdid");
        let cmd = || json!({ "id": command_id, "type": "get_state", "sessionId": session });
        // Two concurrent deliveries → a single agent call, identical replies.
        let (first, second) = tokio::join!(bridge.call(cmd()), bridge.call(cmd()));
        assert_eq!(first, second);
        let executions = agent
            .requests()
            .iter()
            .filter(|(command, sid)| command == "get_state" && sid == &session)
            .count();
        assert_eq!(executions, 1, "retried command must not re-execute");

        // After the reply-slot TTL the entry expires and the command runs again.
        tokio::time::sleep(Duration::from_millis(650)).await;
        let third = bridge.call(cmd()).await;
        assert_eq!(third["success"], json!(true));
        let executions = agent
            .requests()
            .iter()
            .filter(|(command, sid)| command == "get_state" && sid == &session)
            .count();
        assert_eq!(executions, 2, "expired cache entries re-execute");

        // A cached reply replayed to a delivery WITHOUT a reply subject is
        // simply dropped (no publish, no panic).
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            None,
            serde_json::to_vec(&cmd()).unwrap(),
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        bridge.stop();
    }

    #[tokio::test]
    async fn command_loop_resubscribes_after_the_stream_ends() {
        let _home = HomeGuard::new("cmd-self-heal");
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let creds = bridge_creds();
        let pair_id = creds.pair_id.clone();
        let handshake = HandshakeState::new(
            creds,
            Arc::new(AtomicBool::new(true)),
            format!("bridge_{}", unique("cmd")),
        );
        let handle = tokio::spawn(command_loop(
            client,
            pair_id.clone(),
            new_reply_slots(),
            handshake,
        ));
        nats.wait_for_sub(&format!("p.{pair_id}.cmd.>"), Duration::from_secs(5))
            .await;
        // Kill the server: the subscription stream ends and every resubscribe
        // fails — the loop must keep retrying instead of exiting.
        nats.kill();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!handle.is_finished(), "command loop must self-heal");
        handle.abort();
    }
}
