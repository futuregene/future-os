//! Decode side of the typed-RPC wire contract: typed proto payload →
//! canonical JSON `Value` (with the migration-window legacy aliases
//! re-injected, so downstream consumers see exactly what the dual-written
//! `data` string carries), plus typed getters for clients that want the
//! payload structs directly.
//!
//! Every entry point falls back to the JSON `data` string when the typed
//! payload is absent (old agent, or a command that is not typed yet).

use crate::payloads::{
    inject_legacy_aliases, EventsSincePayload, GetStatePayload, ProjectionPayload,
    ReplayEventPayload, SessionEntryPayload, SessionSummaryPayload, GET_STATE_ALIASES,
    SESSION_SUMMARY_ALIASES, TERMINAL_ACK_ALIASES,
};
use crate::proto;
use proto::response_payload::Kind;
use serde_json::{json, Value};

/// The response payload as a JSON value. Typed payload first (reconstructed
/// into the canonical dual-casing shape), JSON `data` fallback second.
/// Empty `data` without a typed payload yields `Value::Null`.
pub fn response_data(resp: &proto::RpcResponse) -> Value {
    if let Some(value) = typed_response_data(resp) {
        return value;
    }
    if resp.data.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(&resp.data).unwrap_or(Value::Null)
}

/// Canonical JSON text of [`response_data`] — for consumers that persist or
/// republish the payload string (GUI event journal, NATS bridge).
pub fn response_data_str(resp: &proto::RpcResponse) -> String {
    match response_data(resp) {
        Value::Null => String::new(),
        value => serde_json::to_string(&value).unwrap_or_default(),
    }
}

fn typed_response_data(resp: &proto::RpcResponse) -> Option<Value> {
    let kind = resp.payload.as_ref()?.kind.as_ref()?;
    match kind {
        Kind::GetState(state) => {
            let payload = session_state_from_proto(state);
            let mut value = serde_json::to_value(&payload).ok()?;
            inject_get_state_aliases(&mut value);
            Some(value)
        }
        Kind::ListSessions(response) => {
            let rows: Vec<Value> = response
                .sessions
                .iter()
                .map(session_summary_from_proto)
                .map(|row| serde_json::to_value(&row))
                .collect::<Result<_, _>>()
                .ok()?;
            let mut value = json!({ "sessions": rows });
            if let Some(rows) = value.get_mut("sessions").and_then(Value::as_array_mut) {
                for row in rows {
                    inject_legacy_aliases(row, SESSION_SUMMARY_ALIASES);
                }
            }
            Some(value)
        }
        Kind::GetSessionEntries(response) => {
            let entries: Vec<Value> = response
                .entries
                .iter()
                .map(session_entry_from_proto)
                .map(|entry| serde_json::to_value(&entry))
                .collect::<Result<_, _>>()
                .ok()?;
            Some(json!({ "entries": entries }))
        }
        Kind::GetEventsSince(events) => {
            let payload = events_since_from_proto(events);
            serde_json::to_value(&payload).ok()
        }
        _ => None,
    }
}

/// Mirror of the agent's get_state alias injection (canonical sessionName
/// plus snake_case TerminalAck keys) so typed-decoded values match the
/// dual-written JSON exactly.
fn inject_get_state_aliases(value: &mut Value) {
    inject_legacy_aliases(value, GET_STATE_ALIASES);
    if let Some(acks) = value
        .get_mut("recentTerminalAcks")
        .and_then(Value::as_array_mut)
    {
        for ack in acks {
            inject_legacy_aliases(ack, TERMINAL_ACK_ALIASES);
        }
    }
}

// ── Typed getters (typed-first, JSON fallback) ──────────────────────────────

/// get_state payload, decoded from the typed wire form when present.
pub fn decode_get_state(resp: &proto::RpcResponse) -> Option<GetStatePayload> {
    if let Some(Kind::GetState(state)) = resp.payload.as_ref().and_then(|p| p.kind.as_ref()) {
        return Some(session_state_from_proto(state));
    }
    fallback_value(resp).and_then(|mut value| {
        strip_for_get_state(&mut value);
        serde_json::from_value(value).ok()
    })
}

/// list_sessions rows, decoded from the typed wire form when present.
pub fn decode_list_sessions(resp: &proto::RpcResponse) -> Option<Vec<SessionSummaryPayload>> {
    if let Some(Kind::ListSessions(response)) = resp.payload.as_ref().and_then(|p| p.kind.as_ref())
    {
        return Some(
            response
                .sessions
                .iter()
                .map(session_summary_from_proto)
                .collect(),
        );
    }
    fallback_value(resp).and_then(|value| {
        let rows = value.get("sessions")?.as_array()?.clone();
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut row = row;
            crate::payloads::strip_legacy_aliases(&mut row, SESSION_SUMMARY_ALIASES);
            out.push(serde_json::from_value(row).ok()?);
        }
        Some(out)
    })
}

/// get_session_entries rows, decoded from the typed wire form when present.
pub fn decode_session_entries(resp: &proto::RpcResponse) -> Option<Vec<SessionEntryPayload>> {
    if let Some(Kind::GetSessionEntries(response)) =
        resp.payload.as_ref().and_then(|p| p.kind.as_ref())
    {
        return Some(
            response
                .entries
                .iter()
                .map(session_entry_from_proto)
                .collect(),
        );
    }
    fallback_value(resp).and_then(|value| {
        let rows = value.get("entries")?.as_array()?.clone();
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(serde_json::from_value(row).ok()?);
        }
        Some(out)
    })
}

/// get_events_since payload, decoded from the typed wire form when present.
pub fn decode_events_since(resp: &proto::RpcResponse) -> Option<EventsSincePayload> {
    if let Some(Kind::GetEventsSince(events)) = resp.payload.as_ref().and_then(|p| p.kind.as_ref())
    {
        return Some(events_since_from_proto(events));
    }
    fallback_value(resp).and_then(|value| serde_json::from_value(value).ok())
}

fn fallback_value(resp: &proto::RpcResponse) -> Option<Value> {
    if resp.data.is_empty() {
        return None;
    }
    serde_json::from_str(&resp.data).ok()
}

fn strip_for_get_state(value: &mut Value) {
    crate::payloads::strip_legacy_aliases(value, GET_STATE_ALIASES);
    if let Some(acks) = value
        .get_mut("recentTerminalAcks")
        .and_then(Value::as_array_mut)
    {
        for ack in acks {
            crate::payloads::strip_legacy_aliases(ack, TERMINAL_ACK_ALIASES);
        }
    }
}

// ── proto → payload struct conversions ──────────────────────────────────────

pub(crate) fn session_state_from_proto(state: &proto::SessionState) -> GetStatePayload {
    GetStatePayload {
        agent_instance_id: state.agent_instance_id.clone(),
        model: state.model.clone(),
        image_support: state.image_support,
        thinking_level: state.thinking_level.clone(),
        is_streaming: state.is_streaming,
        is_compacting: state.is_compacting,
        session_file: state.session_file.clone(),
        session_id: state.session_id.clone(),
        session_name: state.session_name.clone(),
        explicit_session: state.explicit_session,
        auto_compaction_enabled: state.auto_compaction_enabled,
        query_count: state.query_count.max(0) as usize,
        version: state.version.clone(),
        cwd: state.cwd.clone(),
        skills: state.skills.clone(),
        context_files: state.context_files.clone(),
        extensions: if state.extensions.is_empty() {
            None
        } else {
            Some(state.extensions.clone())
        },
        context_window: state.context_window,
        context_tokens: state.context_tokens,
        context_percent: state.context_percent,
        tokens_in: state.tokens_in,
        tokens_out: state.tokens_out,
        tokens_cache_r: state.tokens_cache_r,
        tokens_cache_w: state.tokens_cache_w,
        total_cost: state.total_cost,
        permission_level: state.permission_level.clone(),
        parent_session_id: state.parent_session_id.clone(),
        created_by: state.created_by.clone(),
        // Serialized JSON value → re-inflated (null when empty/unparseable).
        source_meta: inflate_json_value(&state.source_meta),
        active_run: state.active_run.as_ref().map(run_state_snapshot_from_proto),
        queued_runs: state
            .queued_runs
            .iter()
            .map(queued_run_from_proto)
            .collect(),
        recent_terminal_acks: state
            .recent_terminal_acks
            .iter()
            .map(terminal_ack_from_proto)
            .collect(),
        queued_count: state.queued_count.max(0) as usize,
        interrupted_run: state
            .interrupted_run
            .as_ref()
            .map(run_state_snapshot_from_proto),
        requested_run: state.requested_run.as_ref().map(run_terminal_from_proto),
        pending_approvals: state
            .pending_approvals
            .iter()
            .map(approval_card_from_proto)
            .collect(),
    }
}

/// Re-inflate a serialized-JSON carrier field into the original JSON value.
/// Empty strings decode to `Value::Null` (the wire uses them for null/absent).
fn inflate_json_value(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn run_state_snapshot_from_proto(
    snapshot: &proto::RunStateSnapshot,
) -> crate::payloads::RunStateSnapshot {
    crate::payloads::RunStateSnapshot {
        run_id: snapshot.run_id.clone(),
        epoch: snapshot.epoch.map(|epoch| epoch as u64),
        run_sequence: snapshot.run_sequence.map(|sequence| sequence as u64),
        state: snapshot.state.clone(),
        last_event_idx: snapshot.last_event_idx,
    }
}

fn queued_run_from_proto(run: &proto::QueuedRunState) -> crate::payloads::QueuedRunState {
    crate::payloads::QueuedRunState {
        run_id: run.run_id.clone(),
        run_sequence: run.run_sequence as u64,
        client_request_id: run.client_request_id.clone(),
        state: run.state.clone(),
        queue_position: run.queue_position.max(0) as usize,
        accepted_at: run.accepted_at.clone(),
        display_text: run.display_text.clone(),
    }
}

fn terminal_ack_from_proto(ack: &proto::TerminalAck) -> crate::payloads::TerminalAck {
    crate::payloads::TerminalAck {
        run_id: ack.run_id.clone(),
        run_sequence: ack.run_sequence as u64,
        client_request_id: ack.client_request_id.clone(),
        state: ack.state.clone(),
        reason: ack.reason.clone(),
    }
}

/// RunTerminalInfo → run_terminal marker content (snake_case JSON object).
fn run_terminal_from_proto(info: &proto::RunTerminalInfo) -> Value {
    let mut content = json!({
        "run_id": info.run_id,
        "state": info.state,
        "run_tokens": info.run_tokens,
        "run_duration_ms": info.run_duration_ms,
    });
    if let Some(error) = &info.error {
        content["error"] = Value::String(error.clone());
    }
    content
}

/// ApprovalRequestInfo → the full approval card object. Modelled fields come
/// first; the `extras` JSON object (including the card's `type` key) merges
/// back in afterwards.
fn approval_card_from_proto(info: &proto::ApprovalRequestInfo) -> Value {
    let mut card = serde_json::Map::new();
    // `type` travels in extras; insert the modelled keys first so extras can
    // never clobber them.
    card.insert(
        "approval_request_id".to_string(),
        Value::String(info.approval_request_id.clone()),
    );
    card.insert(
        "session_id".to_string(),
        Value::String(info.session_id.clone()),
    );
    card.insert("tool_id".to_string(), Value::String(info.tool_id.clone()));
    card.insert(
        "tool_name".to_string(),
        Value::String(info.tool_name.clone()),
    );
    card.insert("kind".to_string(), Value::String(info.kind.clone()));
    card.insert(
        "risk_level".to_string(),
        Value::String(info.risk_level.clone()),
    );
    card.insert("title".to_string(), Value::String(info.title.clone()));
    card.insert("summary".to_string(), Value::String(info.summary.clone()));
    card.insert(
        "requested_action".to_string(),
        inflate_json_value(&info.requested_action),
    );
    card.insert("action".to_string(), inflate_json_value(&info.action));
    card.insert(
        "sandbox_boundary".to_string(),
        inflate_json_value(&info.sandbox_boundary),
    );
    // Unset save_suggestion reconstructs the wire's explicit null.
    card.insert(
        "save_suggestion".to_string(),
        match &info.save_suggestion {
            Some(raw) => inflate_json_value(raw),
            None => Value::Null,
        },
    );
    card.insert("reviewer".to_string(), Value::String(info.reviewer.clone()));
    if !info.extras.is_empty() {
        if let Ok(Value::Object(extras)) = serde_json::from_str::<Value>(&info.extras) {
            for (key, value) in extras {
                card.entry(key).or_insert(value);
            }
        }
    }
    Value::Object(card)
}

pub(crate) fn session_summary_from_proto(row: &proto::SessionSummary) -> SessionSummaryPayload {
    SessionSummaryPayload {
        id: row.id.clone(),
        session_name: row.session_name.clone(),
        model: row.model.clone(),
        cwd: row.cwd.clone(),
        updated_at: row.updated_at.clone(),
        parent_session_id: row.parent_session_id.clone(),
        first_message: row.first_message.clone(),
        query_count: row.query_count.max(0) as usize,
        is_streaming: row.is_streaming,
    }
}

pub(crate) fn session_entry_from_proto(entry: &proto::SessionEntry) -> SessionEntryPayload {
    SessionEntryPayload {
        id: entry.id.clone(),
        role: entry.role.clone(),
        content: if entry.content_is_object {
            inflate_json_value(&entry.content)
        } else {
            Value::String(entry.content.clone())
        },
        name: entry.name.clone(),
        tool_args: entry.tool_args.clone(),
        timestamp: entry.timestamp.clone(),
        thinking: entry.thinking.clone(),
        meta: entry.meta.as_ref().map(|raw| inflate_json_value(raw)),
        tool_calls: entry.tool_calls.as_ref().map(|raw| inflate_json_value(raw)),
        output_tokens: entry.output_tokens,
        duration_ms: entry.duration_ms,
    }
}

pub(crate) fn events_since_from_proto(events: &proto::EventsSince) -> EventsSincePayload {
    EventsSincePayload {
        run_id: events.run_id.clone(),
        events: events.events.iter().map(replay_event_from_proto).collect(),
        truncated: events.truncated,
        projection: events.projection.as_ref().map(projection_from_proto),
    }
}

fn projection_from_proto(projection: &proto::ProjectionSnapshot) -> ProjectionPayload {
    ProjectionPayload {
        run_id: projection.run_id.clone(),
        cursor: projection.cursor,
        events: projection
            .events
            .iter()
            .map(replay_event_from_proto)
            .collect(),
    }
}

fn replay_event_from_proto(event: &proto::ReplayEvent) -> ReplayEventPayload {
    ReplayEventPayload {
        event_type: event.r#type.clone(),
        data: event.data.clone(),
        run_id: event.run_id.clone(),
        idx: event.idx,
        session_id: event.session_id.clone(),
        epoch: event.epoch,
        event_id: event.event_id.clone(),
        timestamp: event.timestamp.clone(),
        session_idx: event.session_idx,
        run_sequence: event.run_sequence,
    }
}
