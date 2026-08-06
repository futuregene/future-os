//! Encode side of the typed-RPC wire contract: JSON payload `Value` →
//! typed proto payload. Called at the agent's gRPC boundary to populate
//! `RpcResponse.payload` / `StreamEvent.payload` alongside the dual-written
//! JSON `data` string.
//!
//! Defensive by contract: unknown commands, unexpected shapes, or parse
//! failures return `None` — the client then falls back to `data`. A malformed
//! typed payload must never reach the wire, and nothing here panics.

use crate::payloads::{
    strip_legacy_aliases, EventsSincePayload, GetStatePayload, ProjectionPayload,
    ReplayEventPayload, SessionEntryPayload, SessionSummaryPayload, GET_STATE_ALIASES,
    SESSION_SUMMARY_ALIASES, TERMINAL_ACK_ALIASES,
};
use crate::proto;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Encode a unary response's JSON payload into its typed wire form. Returns
/// `None` for commands that are not typed yet or when the shape does not
/// match (clients fall back to the JSON `data` string).
pub fn response_payload(command: &str, data: &Value) -> Option<proto::ResponsePayload> {
    use proto::response_payload::Kind;
    let kind = match command {
        "get_state" => get_state(data).map(Kind::GetState),
        "list_sessions" => list_sessions(data).map(Kind::ListSessions),
        "get_session_entries" => get_session_entries(data).map(Kind::GetSessionEntries),
        "get_events_since" => get_events_since(data).map(Kind::GetEventsSince),
        _ => None,
    };
    kind.map(|kind| proto::ResponsePayload { kind: Some(kind) })
}

/// Deserialize a (dual-casing) wire JSON value into a payload struct: drop the
/// legacy duplicate keys first (serde rejects duplicate field names), then let
/// the serde aliases pick up legacy-only spellings from pre-migration agents.
fn from_wire_value<T: DeserializeOwned>(mut value: Value, aliases: &[(&str, &str)]) -> Option<T> {
    strip_legacy_aliases(&mut value, aliases);
    serde_json::from_value(value).ok()
}

// ── get_state ────────────────────────────────────────────────────────────────

fn get_state(data: &Value) -> Option<proto::SessionState> {
    // The wire JSON carries legacy duplicates at the top level and inside each
    // recentTerminalAcks entry; strip both before deserializing.
    let mut value = data.clone();
    strip_legacy_aliases(&mut value, GET_STATE_ALIASES);
    if let Some(acks) = value
        .get_mut("recentTerminalAcks")
        .and_then(Value::as_array_mut)
    {
        for ack in acks {
            strip_legacy_aliases(ack, TERMINAL_ACK_ALIASES);
        }
    }
    let payload: GetStatePayload = serde_json::from_value(value).ok()?;
    Some(get_state_to_proto(&payload))
}

pub(crate) fn get_state_to_proto(p: &GetStatePayload) -> proto::SessionState {
    proto::SessionState {
        model: p.model.clone(),
        thinking_level: p.thinking_level.clone(),
        is_streaming: p.is_streaming,
        is_compacting: p.is_compacting,
        session_file: p.session_file.clone(),
        session_id: p.session_id.clone(),
        session_name: p.session_name.clone(),
        explicit_session: p.explicit_session,
        auto_compaction_enabled: p.auto_compaction_enabled,
        query_count: p.query_count as i32,
        // `pending_message_count` is the legacy /status field for the queued
        // count; the payload carries it as `queued_count`.
        pending_message_count: p.queued_count as i32,
        version: p.version.clone(),
        cwd: p.cwd.clone(),
        skills: p.skills.clone(),
        context_files: p.context_files.clone(),
        extensions: p.extensions.clone().unwrap_or_default(),
        context_tokens: p.context_tokens,
        context_window: p.context_window,
        context_percent: p.context_percent,
        tokens_in: p.tokens_in,
        tokens_out: p.tokens_out,
        total_cost: p.total_cost,
        image_support: p.image_support,
        tokens_cache_r: p.tokens_cache_r,
        tokens_cache_w: p.tokens_cache_w,
        permission_level: p.permission_level.clone(),
        agent_instance_id: p.agent_instance_id.clone(),
        parent_session_id: p.parent_session_id.clone(),
        created_by: p.created_by.clone(),
        // Free-form metadata travels as a serialized JSON value (string,
        // object or null); decoders re-inflate it.
        source_meta: serde_json::to_string(&p.source_meta).unwrap_or_else(|_| "null".to_string()),
        active_run: p.active_run.as_ref().map(run_state_snapshot_to_proto),
        queued_runs: p.queued_runs.iter().map(queued_run_to_proto).collect(),
        queued_count: p.queued_count as i32,
        interrupted_run: p.interrupted_run.as_ref().map(run_state_snapshot_to_proto),
        requested_run: p.requested_run.as_ref().and_then(run_terminal_to_proto),
        recent_terminal_acks: p
            .recent_terminal_acks
            .iter()
            .map(terminal_ack_to_proto)
            .collect(),
        pending_approvals: p
            .pending_approvals
            .iter()
            .filter_map(approval_card_to_proto)
            .collect(),
    }
}

fn run_state_snapshot_to_proto(
    snapshot: &crate::payloads::RunStateSnapshot,
) -> proto::RunStateSnapshot {
    proto::RunStateSnapshot {
        run_id: snapshot.run_id.clone(),
        epoch: snapshot.epoch.map(|epoch| epoch as i64),
        run_sequence: snapshot.run_sequence.map(|sequence| sequence as i64),
        state: snapshot.state.clone(),
        last_event_idx: snapshot.last_event_idx,
    }
}

fn queued_run_to_proto(run: &crate::payloads::QueuedRunState) -> proto::QueuedRunState {
    proto::QueuedRunState {
        run_id: run.run_id.clone(),
        run_sequence: run.run_sequence as i64,
        client_request_id: run.client_request_id.clone(),
        state: run.state.clone(),
        queue_position: run.queue_position as i32,
        accepted_at: run.accepted_at.clone(),
        display_text: run.display_text.clone(),
    }
}

fn terminal_ack_to_proto(ack: &crate::payloads::TerminalAck) -> proto::TerminalAck {
    proto::TerminalAck {
        run_id: ack.run_id.clone(),
        run_sequence: ack.run_sequence as i64,
        client_request_id: ack.client_request_id.clone(),
        state: ack.state.clone(),
        reason: ack.reason.clone(),
    }
}

/// run_terminal marker content (snake_case JSON object) → RunTerminalInfo.
fn run_terminal_to_proto(value: &Value) -> Option<proto::RunTerminalInfo> {
    Some(proto::RunTerminalInfo {
        run_id: value.get("run_id")?.as_str()?.to_string(),
        state: value.get("state")?.as_str()?.to_string(),
        run_tokens: value.get("run_tokens")?.as_i64()?,
        run_duration_ms: value.get("run_duration_ms")?.as_i64()?,
        error: value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// One pending approval card → ApprovalRequestInfo. Tool-specific fields that
/// are not modelled explicitly travel in `extras` as a JSON object string.
fn approval_card_to_proto(card: &Value) -> Option<proto::ApprovalRequestInfo> {
    let object = card.as_object()?;
    let string_field = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let json_field = |key: &str| {
        object
            .get(key)
            .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()))
            .unwrap_or_default()
    };
    const MODELLED_KEYS: &[&str] = &[
        "approval_request_id",
        "session_id",
        "tool_id",
        "tool_name",
        "kind",
        "risk_level",
        "title",
        "summary",
        "requested_action",
        "action",
        "sandbox_boundary",
        "save_suggestion",
        "reviewer",
    ];
    let extras: serde_json::Map<String, Value> = object
        .iter()
        .filter(|(key, _)| !MODELLED_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Some(proto::ApprovalRequestInfo {
        approval_request_id: string_field("approval_request_id"),
        session_id: string_field("session_id"),
        tool_name: string_field("tool_name"),
        tool_id: string_field("tool_id"),
        kind: string_field("kind"),
        title: string_field("title"),
        action: json_field("action"),
        risk_level: string_field("risk_level"),
        summary: string_field("summary"),
        requested_action: json_field("requested_action"),
        sandbox_boundary: json_field("sandbox_boundary"),
        // save_suggestion is null on the wire for kinds without a rule
        // suggestion; keep it unset so decoders can reconstruct the null.
        save_suggestion: object
            .get("save_suggestion")
            .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())),
        reviewer: string_field("reviewer"),
        extras: if extras.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&extras).unwrap_or_default()
        },
    })
}

// ── list_sessions ────────────────────────────────────────────────────────────

fn list_sessions(data: &Value) -> Option<proto::ListSessionsResponse> {
    let rows = data.get("sessions")?.as_array()?;
    let mut sessions = Vec::with_capacity(rows.len());
    for row in rows {
        let payload: SessionSummaryPayload = from_wire_value(row.clone(), SESSION_SUMMARY_ALIASES)?;
        sessions.push(session_summary_to_proto(&payload));
    }
    Some(proto::ListSessionsResponse { sessions })
}

pub(crate) fn session_summary_to_proto(row: &SessionSummaryPayload) -> proto::SessionSummary {
    proto::SessionSummary {
        id: row.id.clone(),
        session_name: row.session_name.clone(),
        model: row.model.clone(),
        cwd: row.cwd.clone(),
        updated_at: row.updated_at.clone(),
        parent_session_id: row.parent_session_id.clone(),
        first_message: row.first_message.clone(),
        query_count: row.query_count as i32,
        is_streaming: row.is_streaming,
    }
}

// ── get_session_entries ──────────────────────────────────────────────────────

fn get_session_entries(data: &Value) -> Option<proto::SessionEntriesResponse> {
    let rows = data.get("entries")?.as_array()?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let payload: SessionEntryPayload = serde_json::from_value(row.clone()).ok()?;
        entries.push(session_entry_to_proto(&payload));
    }
    Some(proto::SessionEntriesResponse { entries })
}

pub(crate) fn session_entry_to_proto(entry: &SessionEntryPayload) -> proto::SessionEntry {
    // `content` is display text for message entries and the raw session_info
    // JSON object for the session_info entry; the discriminator lets decoders
    // re-inflate the exact original value.
    let (content, content_is_object) = match &entry.content {
        Value::String(text) => (text.clone(), false),
        other => (serde_json::to_string(other).unwrap_or_default(), true),
    };
    proto::SessionEntry {
        id: entry.id.clone(),
        role: entry.role.clone(),
        content,
        content_is_object,
        name: entry.name.clone(),
        tool_args: entry.tool_args.clone(),
        timestamp: entry.timestamp.clone(),
        thinking: entry.thinking.clone(),
        meta: entry
            .meta
            .as_ref()
            .map(|value| serde_json::to_string(value).unwrap_or_default()),
        tool_calls: entry
            .tool_calls
            .as_ref()
            .map(|value| serde_json::to_string(value).unwrap_or_default()),
        output_tokens: entry.output_tokens,
        duration_ms: entry.duration_ms,
    }
}

// ── get_events_since ─────────────────────────────────────────────────────────

fn get_events_since(data: &Value) -> Option<proto::EventsSince> {
    let payload: EventsSincePayload = serde_json::from_value(data.clone()).ok()?;
    Some(events_since_to_proto(&payload))
}

pub(crate) fn events_since_to_proto(payload: &EventsSincePayload) -> proto::EventsSince {
    proto::EventsSince {
        run_id: payload.run_id.clone(),
        events: payload.events.iter().map(replay_event_to_proto).collect(),
        truncated: payload.truncated,
        projection: payload.projection.as_ref().map(projection_to_proto),
    }
}

fn projection_to_proto(projection: &ProjectionPayload) -> proto::ProjectionSnapshot {
    proto::ProjectionSnapshot {
        run_id: projection.run_id.clone(),
        cursor: projection.cursor,
        events: projection
            .events
            .iter()
            .map(replay_event_to_proto)
            .collect(),
    }
}

fn replay_event_to_proto(event: &ReplayEventPayload) -> proto::ReplayEvent {
    proto::ReplayEvent {
        r#type: event.event_type.clone(),
        data: event.data.clone(),
        run_id: event.run_id.clone(),
        idx: event.idx,
        session_id: event.session_id.clone(),
        epoch: event.epoch,
        event_id: event.event_id.clone(),
        timestamp: event.timestamp.clone(),
        session_idx: event.session_idx,
        run_sequence: event.run_sequence,
        // Event payloads are typed in a later batch (events codec); replayed
        // events keep the JSON string until then.
        payload: None,
    }
}
