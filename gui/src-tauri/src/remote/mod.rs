//! Remote control runtime (embedded bridge) — connection lifecycle and event
//! mirroring. Command routing lives in [`commands`]; the prompt persist/finalize
//! contract lives in `agent_bridge::headless` (shared with any future headless
//! caller, so it can't drift from the frontend semantics).
//!
//! Design: see repo-root `docs/remote-control-*.md`. Currently implemented:
//!  - Step A: connect NATS, hold client, report status.
//!  - Step B: `publish_event` — mirror events to mobile at the `agent_bridge::stream` consumption point.
//!  - Step C (`commands.rs`): subscribe to `p.{pairId}.cmd.>`, route mobile commands into the GUI's persistence path.

mod commands;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Mutex;

/// Active remote connection. Holds async-nats client + JetStream context + command subscription task;
/// on stop, aborts the task and drops the client.
struct RemoteState {
    /// Raw client, kept to derive real connection state for [`status`].
    client: async_nats::Client,
    /// JetStream context: events are published through it (with `Nats-Msg-Id` idempotent dedup + written to EVT_* stream for reconnect replay).
    /// When no stream exists, publish still delivers messages to the subject; real-time subscribers still receive them (only persistence is lost), so graceful degradation.
    /// Internally holds a clone of the NATS client to keep the connection alive; dropped with RemoteState on stop.
    js: async_nats::jetstream::Context,
    nats_url: String,
    pair_id: String,
    cmd_task: tokio::task::JoinHandle<()>,
}

static STATE: Mutex<Option<RemoteState>> = Mutex::new(None);

/// Serializes concurrent `start()` calls: `STATE` can't be held across the
/// connect `await`, so without this two racing starts both pass `stop()`, both
/// spawn a command loop, and the loser's task is never aborted — its NATS
/// queue-group membership then silently steals a share of incoming commands.
static START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    // SECURITY: the command surface has no authentication (see the note above
    // `commands::handle_command`), which is only acceptable while the feature
    // is dev-gated. Enforce that premise here in the backend — hiding the nav
    // entry in the frontend is cosmetics, not a gate.
    if crate::build_info::is_release() {
        return Err(crate::AppError::Message(
            "Remote control is not available in release builds.".to_string(),
        ));
    }

    let _start_guard = START_LOCK.lock().await;

    // Stop any previous connection first (idempotent: aborts the old subscription task).
    let _ = stop();

    let client = async_nats::connect(&input.nats_url)
        .await
        .map_err(|e| crate::AppError::Message(format!("Failed to connect to NATS: {e}")))?;
    let js = async_nats::jetstream::new(client.clone());

    // Start the command subscription task (Step C).
    let cmd_task = tokio::spawn(commands::command_loop(
        client.clone(),
        input.pair_id.clone(),
    ));

    let status = RemoteStatus {
        running: true,
        connected: true,
        nats_url: input.nats_url.clone(),
        pair_id: input.pair_id.clone(),
        error: None,
    };
    *STATE.lock().unwrap() = Some(RemoteState {
        client,
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
        Some(s) => {
            // Derive real health instead of reporting `connected: true` for as
            // long as STATE is occupied: the NATS client reconnects with state
            // transitions, and the command loop can die independently (failed
            // subscribe / stream end) — a dead loop processes nothing and must
            // not present as a healthy bridge.
            let loop_dead = s.cmd_task.is_finished();
            let connected = !loop_dead
                && s.client.connection_state() == async_nats::connection::State::Connected;
            RemoteStatus {
                running: true,
                connected,
                nats_url: s.nats_url.clone(),
                pair_id: s.pair_id.clone(),
                error: loop_dead.then(|| {
                    "Command subscription stopped; restart the remote bridge.".to_string()
                }),
            }
        }
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
