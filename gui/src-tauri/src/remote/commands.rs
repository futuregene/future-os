//! Step C: command subscription + routing. Mobile commands arrive on
//! `p.{pairId}.cmd.>`; reads go straight to the store, prompts go through
//! `agent_bridge::headless` so the persist/finalize contract is shared with
//! the rest of the backend.

use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

type ReplySlot = Arc<tokio::sync::Mutex<Option<Vec<u8>>>>;
type ReplySlots = Arc<Mutex<HashMap<String, ReplySlot>>>;

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
    // set_model / set_thinking_level
    model_id: String,
    level: String,
    // set_session_name
    name: String,
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
            model_id: String::new(),
            level: String::new(),
            name: String::new(),
        }
    }
}

pub(super) async fn command_loop(client: async_nats::Client, pair_id: String) {
    let subject = format!("p.{pair_id}.cmd.>");
    let queue = format!("bridge.{pair_id}");
    let mut sub = match client.queue_subscribe(subject.clone(), queue).await {
        Ok(sub) => sub,
        Err(e) => {
            eprintln!("remote: failed to subscribe to commands {subject}: {e}");
            return;
        }
    };
    let reply_slots: ReplySlots = Arc::new(Mutex::new(HashMap::new()));
    eprintln!("remote: subscribed to commands {subject}");
    while let Some(msg) = sub.next().await {
        let client = client.clone();
        let reply_slots = reply_slots.clone();
        // Spawn per command: prevent a slow command from blocking others.
        tokio::spawn(async move {
            handle_command_singleflight(&client, msg, reply_slots).await;
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
) {
    let command_id = serde_json::from_slice::<IncomingCmd>(&msg.payload)
        .ok()
        .map(|cmd| cmd.id)
        .filter(|id| !id.is_empty());
    let Some(command_id) = command_id else {
        handle_command(client, msg).await;
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
        .scope(capture.clone(), handle_command(client, msg))
        .await;
    *cached = capture.lock().unwrap().clone();
}

// SECURITY: Phase 1 authenticates NATS admission with one shared token and
// partitions traffic with a random pairId. It does not enforce per-pair subject
// ACLs; scoped user JWTs remain required before treating the public relay as a
// hostile multi-tenant boundary.
async fn handle_command(client: &async_nats::Client, msg: async_nats::Message) {
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

    match cmd.cmd_type.as_str() {
        "list_sessions" => match crate::store::list_threads() {
            Ok(threads) => {
                let sessions: Vec<Value> = threads
                    .into_iter()
                    .filter_map(|t| {
                        t.agent_session_id.map(
                            |sid| json!({ "sessionId": sid, "title": t.title, "threadId": t.id }),
                        )
                    })
                    .collect();
                reply(client, &msg, true, json!({ "sessions": sessions }), None).await;
            }
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
        "get_messages" => {
            // Serve history from the agent (source of truth for all sessions).
            // The GUI store only has message rows for GUI-native threads —
            // TUI/CLI sessions imported as thread stubs would show empty history.
            // Fall back to the store when the agent is unreachable.
            match crate::agent_bridge::get_session_messages(cmd.session_id.clone()).await {
                Ok(data) => reply(client, &msg, true, data, None).await,
                Err(agent_err) => {
                    let fallback = (|| -> Result<Value, crate::AppError> {
                        match crate::store::find_thread_by_agent_session(&cmd.session_id)? {
                            Some(thread) => {
                                let messages = crate::store::list_messages(&thread.id)?;
                                Ok(json!({ "messages": messages }))
                            }
                            None => Ok(json!({ "messages": [] })),
                        }
                    })();
                    match fallback {
                        Ok(data) => reply(client, &msg, true, data, None).await,
                        Err(e) => {
                            reply(
                                client,
                                &msg,
                                false,
                                Value::Null,
                                Some(&format!("{agent_err}; store fallback also failed: {e}")),
                            )
                            .await;
                        }
                    }
                }
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
            let model_id = (!cmd.model_id.trim().is_empty()).then(|| cmd.model_id.clone());
            let thinking_level = (!cmd.level.trim().is_empty()).then(|| cmd.level.clone());
            match prepare_remote_prompt(
                &cmd.session_id,
                cmd.message.clone(),
                model_id,
                thinking_level,
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
                cmd.model_id.clone(),
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

/// Find the thread for `session_id` (create a new chat thread when unknown —
/// remote policy), then persist user message + run via `agent_bridge::headless`.
async fn prepare_remote_prompt(
    session_id: &str,
    message: String,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<crate::agent_bridge::PreparedPrompt, crate::AppError> {
    let thread = match crate::store::find_thread_by_agent_session(session_id)? {
        Some(thread) => thread,
        None => {
            // Lazy creation: the thread is born with the first message, titled
            // from it (mirrors the GUI new-chat draft), and immediately gets a
            // real agent session id so the ack, the event subjects, and history
            // all agree from the start (no empty row, no id drift).
            let mut input = new_chat_thread_input();
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
