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
}

pub(super) fn new_reply_slots() -> ReplySlots {
    Arc::new(Mutex::new(HashMap::new()))
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
    // set_session_name
    name: String,
    // prompt creation mode / existing workspace selection
    workspace_id: String,
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
            name: String::new(),
            workspace_id: String::new(),
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
    let mut sub = match client.queue_subscribe(subject.clone(), queue).await {
        Ok(sub) => sub,
        Err(e) => {
            eprintln!("remote: failed to subscribe to commands {subject}: {e}");
            return;
        }
    };
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
            tokio::time::sleep(Duration::from_secs(600)).await;
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
                                "title": t.title,
                                "threadId": t.id,
                                "mode": t.mode,
                                "workspaceId": t.workspace_id,
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
            match crate::agent_bridge::get_events_since(
                cmd.session_id.clone(),
                cmd.run_id.clone(),
                cmd.since_idx,
            )
            .await
            {
                Ok(data) => reply(client, &msg, true, data, None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
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
        "abort" => match crate::agent_bridge::abort_session(&cmd.session_id).await {
            Ok(()) => reply(client, &msg, true, json!({}), None).await,
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
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
            match crate::agent_bridge::decide_approval(input).await {
                Ok(_) => reply(client, &msg, true, json!({}), None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
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
            match crate::agent_bridge::set_session_model(
                cmd.session_id.clone(),
                qualified_model_id(&cmd.model_id, &cmd.provider_id).unwrap_or_default(),
            )
            .await
            {
                Ok(()) => reply(client, &msg, true, json!({}), None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_thinking_level" => {
            match crate::agent_bridge::set_session_thinking_level(
                cmd.session_id.clone(),
                cmd.level.clone(),
            )
            .await
            {
                Ok(()) => reply(client, &msg, true, json!({}), None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "set_session_name" => {
            match crate::agent_bridge::rename_session(cmd.session_id.clone(), cmd.name.clone())
                .await
            {
                Ok(()) => reply(client, &msg, true, json!({}), None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
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
    state.active.store(false, Ordering::Release);
    let desktop_public_key = match crate::remote::pairing::public_key(&state.creds) {
        Ok(key) => key,
        Err(error) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
    };
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
    let key_pair = match nkeys::KeyPair::from_seed(&state.creds.nkey_seed) {
        Ok(key_pair) => key_pair,
        Err(error) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
    };
    let signature = match key_pair.sign(transcript.as_bytes()) {
        Ok(signature) => URL_SAFE_NO_PAD.encode(signature),
        Err(error) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
    };
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

/// Find the thread for `session_id` (create a new chat thread when unknown —
/// remote policy), then persist user message + run via `agent_bridge::headless`.
async fn prepare_remote_prompt(
    session_id: &str,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
    mode: String,
    workspace_id: String,
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
    let prepared =
        crate::agent_bridge::prepare_prompt_persisted(&thread, message, model_id, thinking_level)?;
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
    let Some(content) = message.get_mut("content") else {
        return;
    };
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
    if let Ok(payload) = serde_json::to_vec(&body) {
        let _ = REPLY_CAPTURE.try_with(|capture| {
            *capture.lock().unwrap() = Some(payload.clone());
        });
        publish_reply_payload(client, msg, payload).await;
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
        assert!(
            arr.len() < 6,
            "expected byte budget to cap the page, got {}",
            arr.len()
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
    fn byte_cut_is_char_boundary_safe() {
        let s = "中文内容"; // multi-byte chars
        let (end, truncated) = byte_cut(s, 4);
        assert!(s.is_char_boundary(end));
        assert!(truncated);
        let (end, truncated) = byte_cut(s, 1024);
        assert_eq!(end, s.len());
        assert!(!truncated);
    }
}
