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
            // `sessions` was just built above, so the key is always an array;
            // iterate infallibly rather than an if-let.
            for row in value["sessions"].as_array_mut().into_iter().flatten() {
                inject_legacy_aliases(row, SESSION_SUMMARY_ALIASES);
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
        Kind::Prompt(ack) => serde_json::to_value(prompt_ack_from_proto(ack)).ok(),
        Kind::ListModels(response) => serde_json::to_value(list_models_from_proto(response)).ok(),
        Kind::GetAgentInfo(info) => serde_json::to_value(agent_info_from_proto(info)).ok(),
        Kind::GetCommands(response) => serde_json::to_value(commands_from_proto(response)).ok(),
        Kind::Compact(result) => serde_json::to_value(compact_from_proto(result)).ok(),
        Kind::Shell(result) => serde_json::to_value(shell_from_proto(result)).ok(),
        Kind::CycleModel(result) => serde_json::to_value(cycle_model_from_proto(result)).ok(),
        Kind::SyncFutureModels(result) => {
            serde_json::to_value(sync_future_models_from_proto(result)).ok()
        }
        Kind::RefreshSkills(result) => serde_json::to_value(refresh_skills_from_proto(result)).ok(),
        Kind::GetSessionStats(response) => {
            serde_json::to_value(session_stats_from_proto(response)).ok()
        }
        Kind::GetRuntimeMetrics(response) => {
            serde_json::to_value(runtime_metrics_from_proto(response)).ok()
        }
        Kind::GetSessionEventsSince(response) => {
            serde_json::to_value(session_events_since_from_proto(response)).ok()
        } // Exhaustive on purpose: a new oneof member fails the build here
          // until it gets a decoder (or an explicit `=> None` to keep the JSON
          // `data` fallback).
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
pub(crate) fn inflate_json_value(raw: &str) -> Value {
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
        has_more: events.has_more,
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

// ── prompt ack ───────────────────────────────────────────────────────────────

/// Prompt acknowledgement, decoded from the typed wire form when present.
pub fn decode_prompt_ack(resp: &proto::RpcResponse) -> Option<crate::payloads_ext::RunAck> {
    if let Some(Kind::Prompt(ack)) = resp.payload.as_ref().and_then(|p| p.kind.as_ref()) {
        return Some(prompt_ack_from_proto(ack));
    }
    fallback_value(resp).and_then(|value| serde_json::from_value(value).ok())
}

pub(crate) fn prompt_ack_from_proto(ack: &proto::PromptAck) -> crate::payloads_ext::RunAck {
    crate::payloads_ext::RunAck {
        run_id: ack.run_id.clone(),
        run_epoch: ack.run_epoch,
        accepted_state: crate::payloads_ext::RunAcceptedState::parse(&ack.accepted_state)
            .unwrap_or(crate::payloads_ext::RunAcceptedState::Running),
        run_sequence: ack.run_sequence,
        queue_position: ack.queue_position,
    }
}

// ── list_models ──────────────────────────────────────────────────────────────

/// list_models payload, decoded from the typed wire form when present.
pub fn decode_list_models(
    resp: &proto::RpcResponse,
) -> Option<crate::payloads_ext::ListModelsPayload> {
    if let Some(Kind::ListModels(response)) = resp.payload.as_ref().and_then(|p| p.kind.as_ref()) {
        return Some(list_models_from_proto(response));
    }
    fallback_value(resp).and_then(|value| serde_json::from_value(value).ok())
}

fn list_models_from_proto(
    response: &proto::ListModelsResponse,
) -> crate::payloads_ext::ListModelsPayload {
    crate::payloads_ext::ListModelsPayload {
        models: response.models.iter().map(model_entry_from_proto).collect(),
        default_model: response.default_model.clone(),
        is_scoped: response.is_scoped,
        builtin_providers: if response.builtin_providers.is_empty() {
            None
        } else {
            Some(
                response
                    .builtin_providers
                    .iter()
                    .map(|(id, provider)| {
                        (
                            id.clone(),
                            crate::payloads_ext::BuiltinProviderPayload {
                                name: provider.name.clone(),
                                model_count: provider.model_count as usize,
                                base_url: provider.base_url.clone(),
                            },
                        )
                    })
                    .collect(),
            )
        },
    }
}

fn model_entry_from_proto(model: &proto::ModelEntry) -> crate::payloads_ext::ModelEntryPayload {
    crate::payloads_ext::ModelEntryPayload {
        id: model.id.clone(),
        label: model.label.clone(),
        provider: model.provider.clone(),
        supports_images: model.supports_images,
        thinking_level: model.thinking_level.clone(),
        context_window: model.context_window,
        is_default: model.is_default,
        description: model.description.clone(),
        description_en: model.description_en.clone(),
        recommended: model.recommended,
    }
}

// ── get_agent_info ───────────────────────────────────────────────────────────

fn agent_info_from_proto(info: &proto::AgentInfo) -> crate::payloads_ext::AgentInfoPayload {
    crate::payloads_ext::AgentInfoPayload {
        version: info.version.clone(),
        agent_instance_id: info.agent_instance_id.clone(),
        skills_count: info.skills_count as usize,
    }
}

// ── get_commands ─────────────────────────────────────────────────────────────

fn commands_from_proto(response: &proto::CommandsResponse) -> crate::payloads_ext::CommandsPayload {
    crate::payloads_ext::CommandsPayload {
        commands: response
            .commands
            .iter()
            .map(|command| crate::payloads_ext::CommandPayload {
                name: command.name.clone(),
                description: command.description.clone(),
                name_zh: command.name_zh.clone(),
                description_zh: command.description_zh.clone(),
                source: command.source.clone(),
            })
            .collect(),
    }
}

// ── compact / shell / cycle_model / sync_future_models / refresh_skills ─────

fn compact_from_proto(result: &proto::CompactResult) -> crate::payloads_ext::CompactPayload {
    crate::payloads_ext::CompactPayload {
        tokens_before: result.tokens_before,
        tokens_after: result.tokens_after,
        summary: result.summary.clone(),
        messages_removed: result.messages_removed,
    }
}

fn shell_from_proto(result: &proto::ShellResult) -> crate::payloads_ext::ShellPayload {
    crate::payloads_ext::ShellPayload {
        output: result.output.clone(),
        exit_code: result.exit_code,
    }
}

fn cycle_model_from_proto(
    result: &proto::CycleModelResult,
) -> crate::payloads_ext::CycleModelPayload {
    crate::payloads_ext::CycleModelPayload {
        model: result.model.clone(),
        thinking_level: result.thinking_level.clone(),
        is_scoped: result.is_scoped,
    }
}

fn sync_future_models_from_proto(
    result: &proto::SyncFutureModelsResult,
) -> crate::payloads_ext::SyncFutureModelsPayload {
    crate::payloads_ext::SyncFutureModelsPayload {
        synced: result.synced,
        model_count: result.model_count as usize,
    }
}

fn refresh_skills_from_proto(
    result: &proto::RefreshSkillsResult,
) -> crate::payloads_ext::RefreshSkillsPayload {
    crate::payloads_ext::RefreshSkillsPayload {
        skills_count: result.skills_count as usize,
        skills: result.skills.clone(),
        refreshed: result.refreshed,
    }
}

// ── get_session_stats / get_runtime_metrics / get_session_events_since ──────

fn session_stats_from_proto(
    response: &proto::SessionStatsResponse,
) -> crate::payloads_ext::SessionStatsPayload {
    let tokens = response.tokens.unwrap_or_default();
    crate::payloads_ext::SessionStatsPayload {
        session_file: response.session_file.clone(),
        session_id: response.session_id.clone(),
        user_messages: response.user_messages as usize,
        assistant_messages: response.assistant_messages as usize,
        tool_calls: response.tool_calls as usize,
        tool_results: response.tool_results as usize,
        total_messages: response.total_messages as usize,
        tokens: crate::payloads_ext::StatsTokensPayload {
            input: tokens.input,
            output: tokens.output,
            cache_read: tokens.cache_read,
            total: tokens.total,
        },
        cost: serde_json::Value::from(response.cost),
    }
}

fn runtime_metrics_from_proto(
    response: &proto::RuntimeMetricsResponse,
) -> crate::payloads_ext::RuntimeMetricsPayload {
    crate::payloads_ext::RuntimeMetricsPayload {
        session_id: response.session_id.clone(),
        active_run_gauge: response.active_run_gauge as usize,
        stale_epoch_drops: response.stale_epoch_drops,
        persistence_degraded: response.persistence_degraded,
        broadcast_lag: response.broadcast_lag,
        ring_truncations: response.ring_truncations,
        active_run_id: response.active_run_id.clone(),
        queued_runs: response.queued_runs as usize,
        queued_bytes: response.queued_bytes as usize,
        event_journal_healthy: response.event_journal_healthy,
        event_journal_error: response.event_journal_error.clone(),
    }
}

fn session_events_since_from_proto(
    response: &proto::SessionEventsSinceResponse,
) -> crate::payloads_ext::SessionEventsSincePayload {
    crate::payloads_ext::SessionEventsSincePayload {
        events: response
            .events
            .iter()
            .map(|event| crate::payloads_ext::SessionEventRecordPayload {
                event_type: event.r#type.clone(),
                data: event.data.clone(),
                session_id: event.session_id.clone(),
                session_idx: event.session_idx,
                event_id: event.event_id.clone(),
                timestamp: event.timestamp.clone(),
            })
            .collect(),
    }
}

// ── events ───────────────────────────────────────────────────────────────────

/// The event payload as a JSON value. During the migration window the
/// original `data` string wins (byte-stable for journal/NATS consumers);
/// the typed reconstruction takes over when `data` is absent (the future
/// state once dual-write ends).
pub fn event_data(event: &proto::StreamEvent) -> Value {
    if !event.data.is_empty() {
        return serde_json::from_str(&event.data).unwrap_or(Value::Null);
    }
    typed_event_value(event.payload.as_ref().and_then(|p| p.kind.as_ref()))
}

/// Canonical JSON text of [`event_data`]: the original `data` string
/// verbatim when present (byte-identical for persistence/republish
/// consumers), else the typed reconstruction.
pub fn event_data_json(event: &proto::StreamEvent) -> String {
    if !event.data.is_empty() {
        return event.data.clone();
    }
    typed_event_json(event.payload.as_ref().and_then(|p| p.kind.as_ref()))
}

/// [`event_data`] for projection-snapshot events.
pub fn projected_event_data(event: &proto::ProjectedRunEvent) -> Value {
    if !event.data.is_empty() {
        return serde_json::from_str(&event.data).unwrap_or(Value::Null);
    }
    typed_event_value(event.payload.as_ref().and_then(|p| p.kind.as_ref()))
}

/// [`event_data_json`] for projection-snapshot events.
pub fn projected_event_data_json(event: &proto::ProjectedRunEvent) -> String {
    if !event.data.is_empty() {
        return event.data.clone();
    }
    typed_event_json(event.payload.as_ref().and_then(|p| p.kind.as_ref()))
}

/// [`event_data`] for replayed events (get_events_since).
pub fn replay_event_data(event: &proto::ReplayEvent) -> Value {
    if !event.data.is_empty() {
        return serde_json::from_str(&event.data).unwrap_or(Value::Null);
    }
    typed_event_value(event.payload.as_ref().and_then(|p| p.kind.as_ref()))
}

/// [`event_data_json`] for replayed events (get_events_since).
pub fn replay_event_data_json(event: &proto::ReplayEvent) -> String {
    if !event.data.is_empty() {
        return event.data.clone();
    }
    typed_event_json(event.payload.as_ref().and_then(|p| p.kind.as_ref()))
}

fn typed_event_value(kind: Option<&proto::event_payload::Kind>) -> Value {
    let Some(kind) = kind else {
        return Value::Null;
    };
    typed_event_json_inner(kind).unwrap_or(Value::Null)
}

fn typed_event_json(kind: Option<&proto::event_payload::Kind>) -> String {
    let Some(kind) = kind else {
        return String::new();
    };
    typed_event_json_inner(kind)
        .and_then(|value| serde_json::to_string(&value).ok())
        .unwrap_or_default()
}

/// Reconstruct the canonical payload shape from the typed event form. The
/// redundant `type` key the JSON serializer injects is NOT re-added — every
/// consumer keys off the envelope type. Field presence mirrors the broadcast
/// serializer: empty strings and absent optionals stay omitted.
fn typed_event_json_inner(kind: &proto::event_payload::Kind) -> Option<serde_json::Value> {
    use crate::event_payloads as ev;
    use proto::event_payload::Kind as K;
    match kind {
        K::TextChunk(data) => serde_json::to_value(ev::TextChunkData {
            text: data.text.clone(),
        })
        .ok(),
        K::UserMessage(data) => serde_json::to_value(ev::UserMessageData {
            text: data.text.clone(),
        })
        .ok(),
        K::ThinkingDelta(data) => serde_json::to_value(ev::ThinkingDeltaData {
            text: data.text.clone(),
        })
        .ok(),
        K::ThinkingStart(_) => serde_json::to_value(ev::ThinkingMarkerData::default()).ok(),
        K::ThinkingEnd(_) => serde_json::to_value(ev::ThinkingMarkerData::default()).ok(),
        K::AgentStart(data) => serde_json::to_value(ev::AgentStartData {
            started_at_ms: data.started_at_ms,
        })
        .ok(),
        K::AgentEnd(data) => serde_json::to_value(ev::AgentEndData {
            state: data.state.clone(),
            error: data.error.clone(),
            duration_ms: data.duration_ms,
            usage: data.output_tokens.map(|tokens| ev::AgentEndUsage {
                output_tokens: tokens,
            }),
            reason: data.reason.clone(),
        })
        .ok(),
        K::ToolStart(data) => serde_json::to_value(ev::ToolStartData {
            tool_id: data.tool_id.clone(),
            tool_name: data.tool_name.clone(),
            tool_args: inflate_optional_json(&data.tool_args),
            tc_index: None,
        })
        .ok(),
        K::ToolDelta(data) => serde_json::to_value(ev::ToolDeltaData {
            tool_id: data.tool_id.clone(),
            text: data.text.clone(),
            tc_index: data.tc_index,
        })
        .ok(),
        K::ToolEnd(data) => serde_json::to_value(ev::ToolEndData {
            tool_id: data.tool_id.clone(),
            tool_name: data.tool_name.clone(),
            text: data.text.clone(),
            error: data.error.clone().unwrap_or_default(),
            exit_code: data.exit_code,
            is_soft_fail: data.is_soft_fail,
            target_path: data.target_path.clone(),
        })
        .ok(),
        K::ApprovalRequest(info) => Some(approval_card_from_proto(info)),
        K::ApprovalDecision(data) => serde_json::to_value(ev::ApprovalDecisionData {
            approval_request_id: data.approval_request_id.clone(),
            tool_id: data.tool_id.clone(),
            status: data.status.clone(),
            note: data.note.clone(),
        })
        .ok(),
        K::Usage(data) => {
            let usage = data.usage.unwrap_or_default();
            serde_json::to_value(ev::UsageEventData {
                usage: ev::UsageData {
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                    total_tokens: usage.total_tokens,
                    cache_read_tokens: usage.cache_read_tokens,
                    cache_write_tokens: usage.cache_write_tokens,
                    credit_cost: usage.credit_cost,
                },
            })
            .ok()
        }
        K::Error(data) => serde_json::to_value(ev::ErrorEventData {
            error: data.error.clone(),
        })
        .ok(),
    }
}

/// Re-inflate a serialized-JSON carrier that may be empty (absent on the
/// wire) into the original JSON value.
fn inflate_optional_json(raw: &str) -> Option<Value> {
    if raw.is_empty() {
        return None;
    }
    Some(serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::response_payload::Kind;

    fn resp_with_payload(kind: Kind) -> proto::RpcResponse {
        proto::RpcResponse {
            payload: Some(proto::ResponsePayload { kind: Some(kind) }),
            ..Default::default()
        }
    }

    fn resp_with_data(data: &str) -> proto::RpcResponse {
        proto::RpcResponse {
            data: data.to_string(),
            ..Default::default()
        }
    }

    fn typed_text_chunk(text: &str) -> Option<proto::EventPayload> {
        Some(proto::EventPayload {
            kind: Some(proto::event_payload::Kind::TextChunk(proto::TextChunk {
                text: text.to_string(),
            })),
        })
    }

    #[test]
    fn response_data_str_serializes_non_null_payloads() {
        let resp = resp_with_data(r#"{"a":1}"#);
        assert_eq!(response_data_str(&resp), r#"{"a":1}"#);
        assert_eq!(response_data_str(&proto::RpcResponse::default()), "");
    }

    #[test]
    fn typed_list_sessions_injects_legacy_aliases() {
        let resp = resp_with_payload(Kind::ListSessions(proto::ListSessionsResponse {
            sessions: vec![proto::SessionSummary {
                id: "s1".to_string(),
                session_name: Some("Demo".to_string()),
                ..Default::default()
            }],
        }));
        let value = response_data(&resp);
        assert_eq!(value["sessions"][0]["sessionName"], json!("Demo"));
        assert_eq!(value["sessions"][0]["session_name"], json!("Demo"));

        let rows = decode_list_sessions(&resp).unwrap();
        assert_eq!(rows[0].id, "s1");
        assert_eq!(rows[0].session_name.as_deref(), Some("Demo"));
    }

    #[test]
    fn typed_get_state_injects_aliases_and_decodes_subobjects() {
        let state = proto::SessionState {
            session_id: Some("s1".to_string()),
            extensions: vec!["ext-a".to_string()],
            recent_terminal_acks: vec![proto::TerminalAck {
                run_id: "r0".to_string(),
                run_sequence: 3,
                client_request_id: "c0".to_string(),
                state: "cancelled".to_string(),
                reason: "superseded".to_string(),
            }],
            requested_run: Some(proto::RunTerminalInfo {
                run_id: "r9".to_string(),
                state: "failed".to_string(),
                run_tokens: 10,
                run_duration_ms: 20,
                error: Some("boom".to_string()),
            }),
            ..Default::default()
        };
        let resp = resp_with_payload(Kind::GetState(state));

        let value = response_data(&resp);
        assert_eq!(value["sessionId"], json!("s1"));
        assert_eq!(value["extensions"], json!(["ext-a"]));
        let ack = &value["recentTerminalAcks"][0];
        assert_eq!(ack["runId"], json!("r0"));
        assert_eq!(ack["run_id"], json!("r0"));
        assert_eq!(ack["run_sequence"], json!(3));
        assert_eq!(value["requestedRun"]["error"], json!("boom"));

        let payload = decode_get_state(&resp).unwrap();
        assert_eq!(payload.session_id.as_deref(), Some("s1"));
        assert_eq!(payload.extensions, Some(vec!["ext-a".to_string()]));
    }

    /// The alias-injection helper must no-op cleanly when the get_state value
    /// carries no recentTerminalAcks array.
    #[test]
    fn inject_get_state_aliases_without_acks() {
        let mut value = json!({"sessionName": "Demo"});
        inject_get_state_aliases(&mut value);
        assert_eq!(value["session_name"], json!("Demo"));
        assert!(value.get("recentTerminalAcks").is_none());
    }

    /// The JSON fallback path strips the legacy duplicates before
    /// deserializing — top-level aliases and per-ack snake_case keys.
    #[test]
    fn decode_get_state_fallback_strips_legacy_aliases() {
        let resp = resp_with_data(
            r#"{
                "agentInstanceId": "a1", "model": "m", "imageSupport": false,
                "thinkingLevel": "off", "isStreaming": false, "isCompacting": false,
                "explicitSession": true, "autoCompactionEnabled": true,
                "queryCount": 2, "version": "1", "cwd": "/w",
                "skills": [], "contextFiles": [], "contextWindow": 1000,
                "contextTokens": 10, "contextPercent": 1.0, "tokensIn": 1,
                "tokensOut": 2, "tokensCacheR": 0, "tokensCacheW": 0,
                "totalCost": 0.0, "permissionLevel": "all", "createdBy": "tui",
                "sourceMeta": null, "queuedRuns": [], "queuedCount": 0,
                "pendingApprovals": [],
                "sessionName": "Demo", "session_name": "Demo",
                "recentTerminalAcks": [{
                    "runId": "r0", "run_id": "r0",
                    "runSequence": 4, "run_sequence": 4,
                    "clientRequestId": "c0", "client_request_id": "c0",
                    "state": "cancelled", "reason": "superseded"
                }]
            }"#,
        );
        let payload = decode_get_state(&resp).unwrap();
        assert_eq!(payload.session_name.as_deref(), Some("Demo"));
        assert_eq!(payload.recent_terminal_acks[0].run_id, "r0");
        assert_eq!(payload.recent_terminal_acks[0].run_sequence, 4);

        // Absent-acks edge of the strip helper.
        let mut bare = json!({"sessionName": "Demo", "session_name": "Demo"});
        strip_for_get_state(&mut bare);
        assert!(bare.get("session_name").is_none());
        assert_eq!(bare["sessionName"], json!("Demo"));
    }

    #[test]
    fn decode_session_entries_typed_fallback_and_missing() {
        let resp = resp_with_payload(Kind::GetSessionEntries(proto::SessionEntriesResponse {
            entries: vec![proto::SessionEntry {
                id: "e1".to_string(),
                role: "session_info".to_string(),
                content: r#"{"schema":1}"#.to_string(),
                content_is_object: true,
                meta: Some(r#"{"m":2}"#.to_string()),
                tool_calls: Some("[]".to_string()),
                output_tokens: Some(9),
                duration_ms: Some(11),
                ..Default::default()
            }],
        }));
        let entries = decode_session_entries(&resp).unwrap();
        assert_eq!(entries[0].content, json!({"schema": 1}));
        assert_eq!(entries[0].meta, Some(json!({"m": 2})));
        assert_eq!(entries[0].output_tokens, Some(9));
        assert_eq!(entries[0].duration_ms, Some(11));

        let resp = resp_with_data(
            r#"{"entries":[{"id":"e2","role":"user","content":"hi","name":"","tool_args":"","timestamp":"t"}]}"#,
        );
        let entries = decode_session_entries(&resp).unwrap();
        assert_eq!(entries[0].content, Value::String("hi".to_string()));

        // `entries` absent / no data at all → None.
        assert!(decode_session_entries(&resp_with_data("{}")).is_none());
        assert!(decode_session_entries(&proto::RpcResponse::default()).is_none());
    }

    #[test]
    fn decode_events_since_typed_fallback_and_empty() {
        let resp = resp_with_payload(Kind::GetEventsSince(proto::EventsSince {
            run_id: "r1".to_string(),
            events: vec![proto::ReplayEvent {
                r#type: "text_chunk".to_string(),
                data: "{}".to_string(),
                ..Default::default()
            }],
            truncated: true,
            projection: None,
            has_more: true,
        }));
        let payload = decode_events_since(&resp).unwrap();
        assert_eq!(payload.run_id, "r1");
        assert!(payload.truncated);
        assert!(payload.has_more);
        assert_eq!(payload.events.len(), 1);

        let resp = resp_with_data(r#"{"runId":"r2","events":[],"truncated":false}"#);
        assert_eq!(decode_events_since(&resp).unwrap().run_id, "r2");

        // No typed payload and no data → None.
        assert!(decode_events_since(&proto::RpcResponse::default()).is_none());
    }

    #[test]
    fn inflate_json_value_handles_empty_invalid_and_valid() {
        assert_eq!(inflate_json_value(""), Value::Null);
        assert_eq!(
            inflate_json_value("nope"),
            Value::String("nope".to_string())
        );
        assert_eq!(inflate_json_value(r#"{"a":1}"#), json!({"a": 1}));

        assert_eq!(inflate_optional_json(""), None);
        assert_eq!(
            inflate_optional_json("raw"),
            Some(Value::String("raw".to_string()))
        );
        assert_eq!(inflate_optional_json("1"), Some(json!(1)));
    }

    #[test]
    fn run_terminal_includes_error_only_on_failure() {
        let info = proto::RunTerminalInfo {
            run_id: "r1".to_string(),
            state: "failed".to_string(),
            run_tokens: 12,
            run_duration_ms: 34,
            error: Some("boom".to_string()),
        };
        let content = run_terminal_from_proto(&info);
        assert_eq!(content["error"], json!("boom"));
        assert_eq!(content["run_tokens"], json!(12));

        let ok = proto::RunTerminalInfo::default();
        assert!(run_terminal_from_proto(&ok).get("error").is_none());
    }

    #[test]
    fn approval_card_null_save_suggestion_and_extras_merge() {
        let info = proto::ApprovalRequestInfo {
            approval_request_id: "ap1".to_string(),
            title: "Modelled".to_string(),
            requested_action: r#"{"command":"ls"}"#.to_string(),
            save_suggestion: None,
            extras: r#"{"type":"tool_call","title":"FromExtras"}"#.to_string(),
            ..Default::default()
        };
        let card = approval_card_from_proto(&info);
        // Unset save_suggestion reconstructs the wire's explicit null.
        assert_eq!(card["save_suggestion"], Value::Null);
        assert_eq!(card["requested_action"], json!({"command": "ls"}));
        // Modelled keys win over extras; other extras merge in.
        assert_eq!(card["title"], json!("Modelled"));
        assert_eq!(card["type"], json!("tool_call"));
    }

    #[test]
    fn approval_card_tolerates_unparseable_extras() {
        let info = proto::ApprovalRequestInfo {
            approval_request_id: "ap2".to_string(),
            extras: "not json".to_string(),
            ..Default::default()
        };
        let card = approval_card_from_proto(&info);
        assert_eq!(card["approval_request_id"], json!("ap2"));
        assert!(card.get("type").is_none());
    }

    #[test]
    fn decode_prompt_ack_typed_fallback_and_unknown_state() {
        let resp = resp_with_payload(Kind::Prompt(proto::PromptAck {
            run_id: "r1".to_string(),
            run_epoch: 7,
            accepted_state: "queued".to_string(),
            run_sequence: Some(3),
            queue_position: Some(0),
        }));
        let ack = decode_prompt_ack(&resp).unwrap();
        assert_eq!(
            ack.accepted_state,
            crate::payloads_ext::RunAcceptedState::Queued
        );
        assert_eq!(ack.run_sequence, Some(3));

        let resp = resp_with_data(r#"{"run_id":"r2","run_epoch":1,"accepted_state":"existing"}"#);
        let ack = decode_prompt_ack(&resp).unwrap();
        assert_eq!(
            ack.accepted_state,
            crate::payloads_ext::RunAcceptedState::Existing
        );

        // An unrecognized wire state degrades to Running.
        let resp = resp_with_payload(Kind::Prompt(proto::PromptAck {
            accepted_state: "weird".to_string(),
            ..Default::default()
        }));
        assert_eq!(
            decode_prompt_ack(&resp).unwrap().accepted_state,
            crate::payloads_ext::RunAcceptedState::Running
        );
    }

    #[test]
    fn decode_list_models_typed_and_fallback() {
        let resp = resp_with_payload(Kind::ListModels(proto::ListModelsResponse {
            models: vec![proto::ModelEntry {
                id: "m1".to_string(),
                is_default: true,
                ..Default::default()
            }],
            default_model: "m1".to_string(),
            is_scoped: true,
            builtin_providers: std::collections::HashMap::from([(
                "future".to_string(),
                proto::BuiltinProvider {
                    name: "Future".to_string(),
                    model_count: 2,
                    base_url: "https://example".to_string(),
                },
            )]),
        }));
        let payload = decode_list_models(&resp).unwrap();
        assert_eq!(payload.models[0].id, "m1");
        assert!(payload.is_scoped);
        assert_eq!(payload.builtin_providers.unwrap()["future"].model_count, 2);

        let resp = resp_with_data(
            r#"{
                "models": [{
                    "id": "m2", "label": "M2", "provider": "p",
                    "supportsImages": false, "thinkingLevel": "off",
                    "contextWindow": 8, "isDefault": false, "recommended": false
                }],
                "defaultModel": "m2", "isScoped": false
            }"#,
        );
        let payload = decode_list_models(&resp).unwrap();
        assert_eq!(payload.models[0].id, "m2");
        assert!(payload.builtin_providers.is_none());
    }

    #[test]
    fn event_data_json_typed_only_and_absent() {
        let event = proto::StreamEvent {
            r#type: "text_chunk".to_string(),
            payload: typed_text_chunk("hi"),
            ..Default::default()
        };
        assert_eq!(event_data_json(&event), r#"{"text":"hi"}"#);
        assert_eq!(event_data(&event), json!({"text": "hi"}));

        // Neither data nor payload: Null / empty string.
        let bare = proto::StreamEvent::default();
        assert_eq!(event_data(&bare), Value::Null);
        assert_eq!(event_data_json(&bare), "");
    }

    #[test]
    fn projected_event_data_variants() {
        let with_data = proto::ProjectedRunEvent {
            r#type: "text_chunk".to_string(),
            data: r#"{"text":"a"}"#.to_string(),
            ..Default::default()
        };
        assert_eq!(projected_event_data(&with_data), json!({"text": "a"}));
        assert_eq!(projected_event_data_json(&with_data), r#"{"text":"a"}"#);

        let typed_only = proto::ProjectedRunEvent {
            payload: typed_text_chunk("b"),
            ..Default::default()
        };
        assert_eq!(projected_event_data(&typed_only), json!({"text": "b"}));
        assert_eq!(projected_event_data_json(&typed_only), r#"{"text":"b"}"#);

        let bare = proto::ProjectedRunEvent::default();
        assert_eq!(projected_event_data(&bare), Value::Null);
        assert_eq!(projected_event_data_json(&bare), "");
    }

    #[test]
    fn replay_event_data_variants() {
        let with_data = proto::ReplayEvent {
            r#type: "text_chunk".to_string(),
            data: r#"{"text":"a"}"#.to_string(),
            ..Default::default()
        };
        assert_eq!(replay_event_data(&with_data), json!({"text": "a"}));
        assert_eq!(replay_event_data_json(&with_data), r#"{"text":"a"}"#);

        let typed_only = proto::ReplayEvent {
            payload: typed_text_chunk("b"),
            ..Default::default()
        };
        assert_eq!(replay_event_data(&typed_only), json!({"text": "b"}));
        assert_eq!(replay_event_data_json(&typed_only), r#"{"text":"b"}"#);

        let bare = proto::ReplayEvent::default();
        assert_eq!(replay_event_data(&bare), Value::Null);
        assert_eq!(replay_event_data_json(&bare), "");
    }
}
