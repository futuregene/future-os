//! Step C: command subscription + routing. Mobile commands arrive on
//! `p.{pairId}.cmd.>`; reads go straight to the store, prompts go through
//! `agent_bridge::headless` so the persist/finalize contract is shared with
//! the rest of the backend.

use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

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

// SECURITY (remote feature is still dev-gated — non-release builds only,
// enforced backend-side in `super::start`): these commands have NO
// authentication. The only isolation is the NATS subject prefix
// `p.{pairId}.cmd.>`, the default pair id is a constant, and the connection
// requires no TLS/credentials. `prompt` drives the local agent (read/write files,
// run bash) — i.e. equivalent to RCE for anyone who can publish on that subject.
// Before this feature is un-gated for release, this MUST gain: a random pair id,
// connection credentials or per-message signing, and subject ACLs.
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
            // Resolve the thread and persist user message + run synchronously,
            // so the accept-ack can carry the identifiers the events will be
            // published under. This matters when `session_id` is unknown/stale:
            // a NEW thread gets a freshly generated agent_session_id, and a
            // client still keyed to the session it sent would otherwise never
            // find the event subject for the run it just started.
            match prepare_remote_prompt(&cmd.session_id, cmd.message.clone()) {
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
        agent_session_id: None,
    }
}

/// Find the thread for `session_id` (create a new chat thread when unknown —
/// remote policy), then persist user message + run via `agent_bridge::headless`.
fn prepare_remote_prompt(
    session_id: &str,
    message: String,
) -> Result<crate::agent_bridge::PreparedPrompt, crate::AppError> {
    let thread = match crate::store::find_thread_by_agent_session(session_id)? {
        Some(thread) => thread,
        None => crate::store::create_thread(new_chat_thread_input())?,
    };
    let prepared = crate::agent_bridge::prepare_prompt_persisted(&thread, message)?;
    // Notify frontend: new thread/run appeared (trigger list refresh).
    crate::emit_remote_activity(&thread.id);
    Ok(prepared)
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
