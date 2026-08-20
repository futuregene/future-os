//! Read-only session observability and export handlers: state, messages,
//! event replay, stats, and HTML export.

use std::sync::Arc;

use crate::rpc::{
    generate_session_html, get_state_internal, AppState, RpcCommand, RpcResponse, ServerSession,
    SseEvent,
};

/// Serialized-size budget for one paged `get_events_since` response. Every
/// event crosses the wire about three times (JSON `data` dual-write, typed
/// `ReplayEvent.data`, typed `EventPayload`), so this much journal-serialized
/// content stays well under the 32 MiB gRPC message cap.
pub(crate) const EVENTS_PAGE_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// Per-event wire size beyond the `data` payload (type, run/event ids,
/// timestamp, idx…), approximating the journal line.
pub(crate) const EVENT_WIRE_OVERHEAD: usize = 320;

/// Cut `events` to one page for a paging caller (`max_events > 0`): at most
/// `max_events` entries, and at most [`EVENTS_PAGE_BYTE_BUDGET`] of estimated
/// serialized size, whichever comes first. The first event always goes out —
/// even when it alone exceeds the budget — so the caller's cursor always
/// advances. Returns the page plus whether a tail remains. `max_events <= 0`
/// is the legacy unlimited behavior: no cut, `has_more = false`.
pub(crate) fn page_events_tail(events: Vec<SseEvent>, max_events: i64) -> (Vec<SseEvent>, bool) {
    if max_events <= 0 || events.is_empty() {
        return (events, false);
    }
    let count_cap = usize::try_from(max_events).unwrap_or(usize::MAX);
    let mut bytes = 0usize;
    let mut cut = 0usize;
    for event in &events {
        if cut >= count_cap {
            break;
        }
        let size = event.data.len() + EVENT_WIRE_OVERHEAD;
        if cut > 0 && bytes + size > EVENTS_PAGE_BYTE_BUDGET {
            break;
        }
        bytes += size;
        cut += 1;
    }
    let has_more = cut < events.len();
    let mut page = events;
    page.truncate(cut);
    (page, has_more)
}

/// Base directory for HTML exports. Always `/tmp` in production; overridable
/// in tests (the setter is `cfg(test)`-only) so the write-failure arm can be
/// reached deterministically.
static EXPORT_DIR_OVERRIDE: parking_lot::Mutex<Option<std::path::PathBuf>> =
    parking_lot::Mutex::new(None);

fn export_output_path(session_id: &str) -> std::path::PathBuf {
    let base = EXPORT_DIR_OVERRIDE
        .lock()
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join(format!(
        "future_agent_export_{}_{}.html",
        session_id,
        chrono::Local::now().format("%Y%m%d%H%M%S")
    ))
}

/// RAII guard for the test-only export-dir override. Holds the override for
/// its lifetime and restores `/tmp` on drop, so a panic can't leak a bad dir
/// into the parallel success-path export test.
#[cfg(test)]
pub(crate) struct ExportDirGuard;

#[cfg(test)]
impl ExportDirGuard {
    pub(crate) fn new(dir: std::path::PathBuf) -> Self {
        *EXPORT_DIR_OVERRIDE.lock() = Some(dir);
        ExportDirGuard
    }
}

#[cfg(test)]
impl Drop for ExportDirGuard {
    fn drop(&mut self) {
        *EXPORT_DIR_OVERRIDE.lock() = None;
    }
}

/// Serializes the two export tests, since the override is process-global.
#[cfg(test)]
pub(crate) static EXPORT_TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub(crate) fn handle_get_state(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // The session-scoped guard in the dispatcher already resolved `session`, so
    // get_state_internal's only None path (unknown session) is unreachable here.
    let state_val = get_state_internal(
        state,
        &cmd.session_id,
        (!cmd.run_id.is_empty()).then_some(cmd.run_id.as_str()),
    )
    .expect("session-scoped guard guarantees a live session");
    RpcResponse::ok(id, "get_state", state_val)
}

pub(crate) fn handle_get_messages(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let msgs = session.read().get_messages();
    RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
}

pub(crate) fn handle_get_events_since(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    // P1: backfill current-run events with idx > since_idx (Bridge reconnect).
    let replay = {
        let sess = session.read();
        sess.broadcaster.events_since(&cmd.run_id, cmd.since_idx)
    };
    let (run_id, events, _min_idx, projection) = match replay {
        Ok(replay) => replay,
        Err(error) => {
            return RpcResponse::build_fail(id, "get_events_since", &error.to_string());
        }
    };
    // A cursor older than the replay ring returns a complete compressed
    // projection instead of a knowingly incomplete event tail.
    let truncated = projection.is_some();
    // Paging (proto max_events): a long run's journal far exceeds the
    // gRPC message cap when returned whole, so a paging caller gets the
    // tail cut to its page size (bounded by a serialized-size budget)
    // and re-requests from the last idx while has_more is set.
    let (events, has_more) = page_events_tail(events, cmd.max_events);
    // Typed payload (audit item 1): ReplayEventPayload / EventsSincePayload.
    let events = events
        .iter()
        .map(crate::rpc::replay_event_payload)
        .collect::<Vec<_>>();
    let projection = projection.map(|snapshot| crate::rpc::payloads::ProjectionPayload {
        run_id: snapshot.run_id,
        cursor: snapshot.cursor,
        events: snapshot
            .events
            .iter()
            .map(crate::rpc::replay_event_payload)
            .collect(),
    });
    let payload = crate::rpc::payloads::EventsSincePayload {
        run_id,
        events,
        truncated,
        projection,
        has_more,
    };
    RpcResponse::ok(
        id,
        "get_events_since",
        serde_json::to_value(payload).unwrap_or_default(),
    )
}

pub(crate) fn handle_get_session_events_since(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let replay = session
        .read()
        .broadcaster
        .session_events_since(cmd.since_idx);
    match replay {
        Ok(events) => RpcResponse::ok(
            id,
            "get_session_events_since",
            serde_json::json!({
                "events": events.into_iter().map(|event| serde_json::json!({
                    "type": event.event_type,
                    "data": event.data,
                    "sessionId": event.session_id,
                    "sessionIdx": event.session_idx,
                    "eventId": event.event_id,
                    "timestamp": event.timestamp,
                })).collect::<Vec<_>>()
            }),
        ),
        Err(error) => RpcResponse::build_fail(id, "get_session_events_since", &error.to_string()),
    }
}

pub(crate) fn handle_get_session_stats(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let stats = session.read().get_session_stats();
    RpcResponse::ok(id, "get_session_stats", stats)
}

pub(crate) fn handle_get_runtime_metrics(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let metrics = session.read().get_runtime_metrics();
    RpcResponse::ok(id, "get_runtime_metrics", metrics)
}

pub(crate) fn handle_get_last_assistant_text(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let text = session.read().get_last_assistant_text();
    RpcResponse::ok(
        id,
        "get_last_assistant_text",
        serde_json::json!({"text": if text.is_empty() { None } else { Some(text) }}),
    )
}

pub(crate) fn handle_export_html(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Export session to HTML file
    let sess = session.read();
    let session_id = sess.session_id();
    let model = sess.model.clone();
    let cwd = sess.cwd.clone();
    let messages = sess.get_messages();
    drop(sess);

    // Generate HTML
    let html = generate_session_html(&session_id, &model, &cwd, &messages);

    // Write to a unique temp file to avoid clobbering concurrent exports.
    let output_path = export_output_path(&session_id);
    let output_path_str = output_path.to_string_lossy().to_string();
    if let Err(e) = std::fs::write(&output_path, html) {
        return RpcResponse::build_fail(id, "export_html", &format!("failed to write file: {}", e));
    }

    RpcResponse::ok(
        id,
        "export_html",
        serde_json::json!({"path": output_path_str}),
    )
}
