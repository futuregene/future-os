//! Remote control runtime (embedded bridge).
//!
//! Design: see repo-root `docs/remote-control-*.md`. Currently implemented:
//!  - Step A: connect NATS, hold client, report status.
//!  - Step B: `publish_event` — mirror events to mobile at the `agent_bridge::stream` consumption point.
//!  - Step C (this file): subscribe to `p.{pairId}.cmd.>`, route mobile commands into the GUI's persistence path.
//!     - `list_sessions` / `get_messages` / `new_session` → directly read/write GUI store.
//!     - `prompt` → replicate the frontend handleSend: create thread/run + append user → `agent_prompt`
//!       (streaming → write run_events + tap mirror) → append assistant → notify frontend to refresh.

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

/// Active remote connection. Holds async-nats client + JetStream context + command subscription task;
/// on stop, aborts the task and drops the client.
struct RemoteState {
    /// JetStream context: events are published through it (with `Nats-Msg-Id` idempotent dedup + written to EVT_* stream for reconnect replay).
    /// When no stream exists, publish still delivers messages to the subject; real-time subscribers still receive them (only persistence is lost), so graceful degradation.
    /// Internally holds a clone of the NATS client to keep the connection alive; dropped with RemoteState on stop.
    js: async_nats::jetstream::Context,
    nats_url: String,
    pair_id: String,
    cmd_task: tokio::task::JoinHandle<()>,
}

static STATE: Mutex<Option<RemoteState>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {
    /// The GUI backend connects to the NATS **client port** (`nats://host:4222`), NOT the browser WebSocket port.
    pub nats_url: String,
    pub pair_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub running: bool,
    pub connected: bool,
    pub nats_url: String,
    pub pair_id: String,
    pub error: Option<String>,
}

fn empty() -> RemoteStatus {
    RemoteStatus {
        running: false,
        connected: false,
        nats_url: String::new(),
        pair_id: String::new(),
        error: None,
    }
}

pub async fn start(input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    // Stop any previous connection first (idempotent: aborts the old subscription task).
    let _ = stop();

    let client = async_nats::connect(&input.nats_url)
        .await
        .map_err(|e| crate::AppError::Message(format!("Failed to connect to NATS: {e}")))?;
    let js = async_nats::jetstream::new(client.clone());

    // Start the command subscription task (Step C).
    let cmd_task = tokio::spawn(command_loop(client.clone(), input.pair_id.clone()));

    let status = RemoteStatus {
        running: true,
        connected: true,
        nats_url: input.nats_url.clone(),
        pair_id: input.pair_id.clone(),
        error: None,
    };
    *STATE.lock().unwrap() = Some(RemoteState {
        js,
        nats_url: input.nats_url,
        pair_id: input.pair_id,
        cmd_task,
    });
    Ok(status)
}

pub fn stop() -> RemoteStatus {
    if let Some(state) = STATE.lock().unwrap().take() {
        state.cmd_task.abort();
    }
    empty()
}

pub fn status() -> RemoteStatus {
    match STATE.lock().unwrap().as_ref() {
        Some(s) => RemoteStatus {
            running: true,
            connected: true,
            nats_url: s.nats_url.clone(),
            pair_id: s.pair_id.clone(),
            error: None,
        },
        None => empty(),
    }
}

/// Event tap (Step B / P1): if remote is running, mirror an agent event to
/// `p.{pairId}.evt.{session}`. Returns immediately when not connected — does not block GUI event consumption.
///
/// Uses JetStream publish with `Nats-Msg-Id = {session}:{runId}:{idx}`:
///  - Idempotent: re-sent/replayed events deduplicated by broker via dupe-window;
///  - Durable: written to EVT_* stream, clients can replay on reconnect (see web `backfillActiveRun`);
///  - Graceful degradation: even without a stream, messages still reach the subject; real-time core subscribers still receive them (only persistence is lost).
///    We don't await the ack (to avoid per-token blocking) — the message is already sent on publish.
pub async fn publish_event(session_id: &str, event_type: &str, data: &str, run_id: &str, idx: i64) {
    let target = {
        let guard = STATE.lock().unwrap();
        guard.as_ref().map(|s| (s.js.clone(), s.pair_id.clone()))
    };
    let Some((js, pair_id)) = target else {
        return;
    };
    let subject = format!("p.{pair_id}.evt.{session_id}");
    let body = json!({ "type": event_type, "data": data, "runId": run_id, "idx": idx });
    let Ok(payload) = serde_json::to_vec(&body) else {
        return;
    };
    if run_id.is_empty() {
        // Events without a run_id (theoretically only early/edge cases) skip dedup, publish directly.
        let _ = js.publish(subject, payload.into()).await;
    } else {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(
            "Nats-Msg-Id",
            format!("{session_id}:{run_id}:{idx}").as_str(),
        );
        let _ = js
            .publish_with_headers(subject, headers, payload.into())
            .await;
    }
}

// ─── Step C: Command subscription + routing ──────────────────────────────────

/// Command sent by the client via NATS (camelCase JSON, only the fields the bridge needs).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct IncomingCmd {
    id: String,
    #[serde(rename = "type")]
    cmd_type: String,
    session_id: String,
    message: String,
    // get_events_since (P1c backfill)
    run_id: String,
    since_idx: i64,
}

impl Default for IncomingCmd {
    fn default() -> Self {
        Self {
            id: String::new(),
            cmd_type: String::new(),
            session_id: String::new(),
            message: String::new(),
            run_id: String::new(),
            since_idx: -1,
        }
    }
}

async fn command_loop(client: async_nats::Client, pair_id: String) {
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
        let pair_id = pair_id.clone();
        // Spawn per command: prevent a slow command from blocking others.
        tokio::spawn(async move {
            handle_command(&client, &pair_id, msg).await;
        });
    }
}

async fn handle_command(client: &async_nats::Client, _pair_id: &str, msg: async_nats::Message) {
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
            let result = (|| -> Result<Value, crate::AppError> {
                match crate::store::find_thread_by_agent_session(&cmd.session_id)? {
                    Some(thread) => {
                        let messages = crate::store::list_messages(&thread.id)?;
                        Ok(json!({ "messages": messages }))
                    }
                    None => Ok(json!({ "messages": [] })),
                }
            })();
            match result {
                Ok(data) => reply(client, &msg, true, data, None).await,
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
        "new_session" => match crate::store::create_thread(new_chat_thread_input()) {
            Ok(thread) => {
                crate::emit_remote_activity(&thread.id);
                let sid = thread.agent_session_id.unwrap_or(thread.id);
                reply(client, &msg, true, json!({ "sessionId": sid }), None).await;
            }
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
        "prompt" => {
            // Accept-ack immediately; actual execution runs in the background (completion visible via event stream agent_end).
            let session_id = cmd.session_id.clone();
            let message = cmd.message.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_remote_prompt(session_id, message).await {
                    eprintln!("remote: prompt processing failed: {e}");
                }
            });
            reply(client, &msg, true, json!({}), None).await;
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
        model_provider: None,
        model_id: None,
        thinking_level: None,
    }
}

/// Replicate the frontend handleSend persistence sequence: mobile prompt → write to GUI SQLite + display + tap mirror.
async fn handle_remote_prompt(session_id: String, message: String) -> Result<(), crate::AppError> {
    // (a) Find or create thread (by agent_session_id; create a new chat thread if not found).
    let thread = match crate::store::find_thread_by_agent_session(&session_id)? {
        Some(thread) => thread,
        None => crate::store::create_thread(new_chat_thread_input())?,
    };
    let agent_session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());

    // (b) append user message。
    let user_msg = crate::store::append_message(crate::store::AppendMessageInput {
        thread_id: thread.id.clone(),
        run_id: None,
        role: "user".to_string(),
        content_type: Some("markdown".to_string()),
        content: message.clone(),
        status: Some("complete".to_string()),
    })?;

    // (c) Create run.
    let run = crate::store::create_run(crate::store::CreateRunInput {
        thread_id: thread.id.clone(),
        trigger_message_id: Some(user_msg.id),
        model_provider: thread.model_provider.clone(),
        model_id: thread.model_id.clone(),
    })?;

    // Notify frontend: new thread/run appeared (trigger list refresh).
    crate::emit_remote_activity(&thread.id);

    // (d) Run agent_prompt (streaming events written to run_events by stream.rs + tapped and mirrored to mobile).
    let result = crate::agent_bridge::agent_prompt(
        message,
        None,
        thread.id.clone(),
        Some(agent_session_id),
        Some(run.id.clone()),
        thread.model_id.clone(),
        thread.thinking_level.clone(),
    )
    .await;

    // (e) Finalize run + append assistant message (content = full response text), matching the frontend.
    match result {
        // Stream closed before `agent_end`: the text is a truncated prefix, not a
        // finished answer. Persist it (so the partial isn't lost) but mark the run
        // failed rather than completed.
        Ok(response) if !response.complete => {
            let _ = crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id: run.id.clone(),
                status: "failed".to_string(),
                error_message: Some("Response interrupted before completion.".to_string()),
                error_type: Some("stream_interrupted".to_string()),
            });
            let _ = crate::store::append_message(crate::store::AppendMessageInput {
                thread_id: thread.id.clone(),
                run_id: Some(run.id.clone()),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content: response.content,
                status: Some("failed".to_string()),
            });
        }
        Ok(response) => {
            let _ = crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id: run.id.clone(),
                status: "completed".to_string(),
                error_message: None,
                error_type: None,
            });
            let content = if response.content.trim().is_empty() {
                "Future Agent completed but returned no text.".to_string()
            } else {
                response.content
            };
            let _ = crate::store::append_message(crate::store::AppendMessageInput {
                thread_id: thread.id.clone(),
                run_id: Some(run.id.clone()),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content,
                status: Some("complete".to_string()),
            });
        }
        Err(e) => {
            let _ = crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
                run_id: run.id.clone(),
                status: "failed".to_string(),
                error_message: Some(e.to_string()),
                error_type: None,
            });
            let _ = crate::store::append_message(crate::store::AppendMessageInput {
                thread_id: thread.id.clone(),
                run_id: Some(run.id.clone()),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content: format!("Future Agent error: {e}"),
                status: Some("failed".to_string()),
            });
        }
    }

    crate::emit_remote_activity(&thread.id);
    Ok(())
}

/// Send a unified request-reply response (in `RpcResponse` shape), and flush to ensure timely delivery.
async fn reply(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    success: bool,
    data: Value,
    error: Option<&str>,
) {
    let Some(reply_subject) = msg.reply.clone() else {
        return;
    };
    let body = json!({
        "type": "response",
        "success": success,
        "data": data,
        "error": error,
    });
    if let Ok(payload) = serde_json::to_vec(&body) {
        let _ = client.publish(reply_subject, payload.into()).await;
        let _ = client.flush().await;
    }
}
