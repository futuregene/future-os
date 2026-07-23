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

/// Command-id → in-flight/completed response cache (single-flight). Created
/// once per bridge start and SHARED across command loops: credential refresh
/// swaps the loop every JWT TTL, and a cache local to the loop would be wiped
/// on every swap — a client retrying right after a swap would re-execute a
/// command the old loop had already run (for `prompt`, a duplicated message).
pub(super) type ReplySlots = Arc<Mutex<HashMap<String, ReplySlot>>>;

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
            offset: 0,
            limit: 0,
            model_id: String::new(),
            level: String::new(),
            name: String::new(),
        }
    }
}

pub(super) async fn command_loop(
    client: async_nats::Client,
    pair_id: String,
    reply_slots: ReplySlots,
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

// SECURITY: NATS admits this bridge with a short-lived user JWT whose server-
// enforced ACL is scoped to this pair. Session/approval ownership is still
// checked in the command handlers because subject isolation and application
// authorization are separate boundaries.
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
            let messages =
                match crate::agent_bridge::get_session_messages(cmd.session_id.clone()).await {
                    Ok(data) => messages_vec(data),
                    Err(agent_err) => {
                        let fallback = (|| -> Result<Vec<Value>, crate::AppError> {
                            match crate::store::find_thread_by_agent_session(&cmd.session_id)? {
                                Some(thread) => {
                                    let rows = crate::store::list_messages(&thread.id)?;
                                    Ok(serde_json::to_value(&rows)
                                        .ok()
                                        .and_then(|value| value.as_array().cloned())
                                        .unwrap_or_default())
                                }
                                None => Ok(Vec::new()),
                            }
                        })();
                        match fallback {
                            Ok(messages) => messages,
                            Err(e) => {
                                reply(
                                    client,
                                    &msg,
                                    false,
                                    Value::Null,
                                    Some(&format!("{agent_err}; store fallback also failed: {e}")),
                                )
                                .await;
                                return;
                            }
                        }
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
    // Reject a prompt for a session that is already running BEFORE persisting
    // anything (matches GUI semantics: no steer/follow-up/queue). The agent
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

/// Page a full message list into a reply that fits the NATS payload cap.
///
/// Each message is content-capped first (so no single message is huge), then
/// messages are accumulated from `offset` until the serialized page would
/// exceed [`MESSAGES_PAGE_BYTES`] or `limit` is reached (always at least one
/// message — it's already capped). Returns the page plus cursor fields the
/// client uses to fetch the remainder.
fn paginate_messages(mut messages: Vec<Value>, offset: usize, limit: usize) -> Value {
    for message in messages.iter_mut() {
        truncate_message_content(message, MESSAGE_CONTENT_CAP_BYTES);
    }
    let total = messages.len();
    let start = offset.min(total);
    let mut end = start;
    let mut bytes = 0usize;
    for (index, message) in messages.iter().skip(start).enumerate() {
        let size = serde_json::to_vec(message)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        if index > 0 && (index >= limit || bytes + size > MESSAGES_PAGE_BYTES) {
            break;
        }
        bytes += size;
        end += 1;
    }
    let page: Vec<Value> = messages.drain(start..end).collect();
    json!({
        "messages": page,
        "offset": start,
        "nextOffset": end,
        "total": total,
        "hasMore": end < total,
    })
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
