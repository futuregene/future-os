//! Encode side of the typed-RPC wire contract: JSON payload `Value` →
//! typed proto payload. Called at the agent's gRPC boundary to populate
//! `RpcResponse.payload` / `StreamEvent.payload`. Typed commands ride the
//! typed `payload` alone; untyped commands keep the JSON `data` string.
//!
//! Defensive by contract: unknown commands, unexpected shapes, or parse
//! failures return `None` — the client then falls back to `data`. A malformed
//! typed payload must never reach the wire, and nothing here panics.

use crate::payloads::{
    EventsSincePayload, GetStatePayload, ProjectionPayload, ReplayEventPayload,
    SessionEntryPayload, SessionSummaryPayload,
};
use crate::proto;
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
        "prompt" => prompt(data).map(Kind::Prompt),
        "list_models" => list_models(data).map(Kind::ListModels),
        "get_agent_info" => get_agent_info(data).map(Kind::GetAgentInfo),
        "get_commands" => get_commands(data).map(Kind::GetCommands),
        "compact" => compact(data).map(Kind::Compact),
        "shell" => shell(data).map(Kind::Shell),
        "cycle_model" => cycle_model(data).map(Kind::CycleModel),
        "sync_future_models" => sync_future_models(data).map(Kind::SyncFutureModels),
        "refresh_skills" => refresh_skills(data).map(Kind::RefreshSkills),
        "get_session_stats" => get_session_stats(data).map(Kind::GetSessionStats),
        "get_runtime_metrics" => get_runtime_metrics(data).map(Kind::GetRuntimeMetrics),
        "get_session_events_since" => {
            get_session_events_since(data).map(Kind::GetSessionEventsSince)
        }
        _ => None,
    };
    kind.map(|kind| proto::ResponsePayload { kind: Some(kind) })
}

// ── get_state ────────────────────────────────────────────────────────────────

fn get_state(data: &Value) -> Option<proto::SessionState> {
    let payload: GetStatePayload = serde_json::from_value(data.clone()).ok()?;
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
        let payload: SessionSummaryPayload = serde_json::from_value(row.clone()).ok()?;
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
    Some(proto::SessionEntriesResponse {
        entries,
        has_more: data
            .get("hasMore")
            .or_else(|| data.get("has_more"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        next_offset: data
            .get("nextOffset")
            .or_else(|| data.get("next_offset"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
    })
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
        entry_type: entry.entry_type.clone(),
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
        input_tokens: entry.input_tokens,
        cache_read_tokens: entry.cache_read_tokens,
        checkpoint: entry
            .checkpoint
            .as_ref()
            .map(|value| serde_json::to_string(value).unwrap_or_default()),
        tool_call_id: entry.tool_call_id.clone(),
        tool_result_is_error: entry.tool_result_is_error,
        run_status: entry.run_status.clone(),
        run_error: entry.run_error.clone(),
        run_duration_ms: entry.run_duration_ms,
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
        has_more: payload.has_more,
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
        payload: event_payload(&event.event_type, &event.data),
    }
}

// ── events ───────────────────────────────────────────────────────────────────

/// Encode a stream event's JSON payload into its typed wire form. Returns
/// `None` for pass-through event types (settings changes, ping, ...) and
/// shape mismatches — those keep serving the JSON `data` string only.
pub fn event_payload(event_type: &str, data_json: &str) -> Option<proto::EventPayload> {
    use proto::event_payload::Kind;
    let mut value: Value = serde_json::from_str(data_json).ok()?;
    // The broadcast serializer injects a redundant "type" key into most
    // payloads; the envelope already carries the type, so the typed form
    // drops it.
    if let Some(object) = value.as_object_mut() {
        object.remove("type");
    }
    let kind = match event_type {
        "text_chunk" => serde_json::from_value::<crate::event_payloads::TextChunkData>(value)
            .ok()
            .map(|data| Kind::TextChunk(proto::TextChunk { text: data.text })),
        "user_message" => serde_json::from_value::<crate::event_payloads::UserMessageData>(value)
            .ok()
            .map(|data| Kind::UserMessage(proto::UserMessageEvent { text: data.text })),
        "thinking_delta" => {
            serde_json::from_value::<crate::event_payloads::ThinkingDeltaData>(value)
                .ok()
                .map(|data| Kind::ThinkingDelta(proto::ThinkingDelta { text: data.text }))
        }
        // Lifecycle markers carry no payload on the wire today.
        "thinking_start" => Some(Kind::ThinkingStart(proto::ThinkingStart {})),
        "thinking_end" => Some(Kind::ThinkingEnd(proto::ThinkingEnd {})),
        "agent_start" => serde_json::from_value::<crate::event_payloads::AgentStartData>(value)
            .ok()
            .map(|data| {
                Kind::AgentStart(proto::AgentStart {
                    started_at_ms: data.started_at_ms,
                })
            }),
        "agent_end" => serde_json::from_value::<crate::event_payloads::AgentEndData>(value)
            .ok()
            .map(|data| {
                Kind::AgentEnd(proto::AgentEnd {
                    state: data.state,
                    error: data.error,
                    duration_ms: data.duration_ms,
                    output_tokens: data.usage.map(|usage| usage.output_tokens),
                    reason: data.reason,
                })
            }),
        "tool_start" => serde_json::from_value::<crate::event_payloads::ToolStartData>(value)
            .ok()
            .map(|data| {
                Kind::ToolStart(proto::ToolStart {
                    tool_id: data.tool_id,
                    tool_name: data.tool_name,
                    tool_args: data
                        .tool_args
                        .map(|args| serde_json::to_string(&args).unwrap_or_default())
                        .unwrap_or_default(),
                })
            }),
        "tool_delta" => serde_json::from_value::<crate::event_payloads::ToolDeltaData>(value)
            .ok()
            .map(|data| {
                Kind::ToolDelta(proto::ToolDelta {
                    tool_id: data.tool_id,
                    text: data.text,
                    tc_index: data.tc_index,
                })
            }),
        "tool_end" => serde_json::from_value::<crate::event_payloads::ToolEndData>(value)
            .ok()
            .map(|data| {
                Kind::ToolEnd(proto::ToolEnd {
                    tool_id: data.tool_id,
                    tool_name: data.tool_name,
                    text: data.text,
                    error: if data.error.is_empty() {
                        None
                    } else {
                        Some(data.error)
                    },
                    exit_code: data.exit_code,
                    is_soft_fail: data.is_soft_fail,
                    target_path: data.target_path,
                })
            }),
        "approval_request" => approval_card_to_proto(&value).map(Kind::ApprovalRequest),
        "approval_decision" => {
            serde_json::from_value::<crate::event_payloads::ApprovalDecisionData>(value)
                .ok()
                .map(|data| {
                    Kind::ApprovalDecision(proto::ApprovalDecisionEvent {
                        approval_request_id: data.approval_request_id,
                        tool_id: data.tool_id,
                        status: data.status,
                        note: data.note,
                    })
                })
        }
        "usage" => serde_json::from_value::<crate::event_payloads::UsageEventData>(value)
            .ok()
            .map(|data| {
                Kind::Usage(proto::UsageEvent {
                    usage: Some(usage_to_proto(&data.usage)),
                })
            }),
        "error" => serde_json::from_value::<crate::event_payloads::ErrorEventData>(value)
            .ok()
            .map(|data| Kind::Error(proto::ErrorEvent { error: data.error })),
        _ => None,
    };
    kind.map(|kind| proto::EventPayload { kind: Some(kind) })
}

fn usage_to_proto(usage: &crate::event_payloads::UsageData) -> proto::UsageInfo {
    proto::UsageInfo {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        cache_write_tokens: usage.cache_write_tokens,
        credit_cost: usage.credit_cost,
    }
}

// ── prompt ack ───────────────────────────────────────────────────────────────

fn prompt(data: &Value) -> Option<proto::PromptAck> {
    let ack: crate::payloads_ext::RunAck = serde_json::from_value(data.clone()).ok()?;
    Some(prompt_ack_to_proto(&ack))
}

pub(crate) fn prompt_ack_to_proto(ack: &crate::payloads_ext::RunAck) -> proto::PromptAck {
    proto::PromptAck {
        run_id: ack.run_id.clone(),
        run_epoch: ack.run_epoch,
        accepted_state: ack.accepted_state.as_str().to_string(),
        run_sequence: ack.run_sequence,
        queue_position: ack.queue_position,
    }
}

// ── list_models ──────────────────────────────────────────────────────────────

fn list_models(data: &Value) -> Option<proto::ListModelsResponse> {
    let payload: crate::payloads_ext::ListModelsPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::ListModelsResponse {
        models: payload.models.iter().map(model_entry_to_proto).collect(),
        default_model: payload.default_model,
        is_scoped: payload.is_scoped,
        builtin_providers: payload
            .builtin_providers
            .unwrap_or_default()
            .into_iter()
            .map(|(id, provider)| (id, builtin_provider_to_proto(&provider)))
            .collect(),
    })
}

fn model_entry_to_proto(model: &crate::payloads_ext::ModelEntryPayload) -> proto::ModelEntry {
    proto::ModelEntry {
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

fn builtin_provider_to_proto(
    provider: &crate::payloads_ext::BuiltinProviderPayload,
) -> proto::BuiltinProvider {
    proto::BuiltinProvider {
        name: provider.name.clone(),
        model_count: provider.model_count as u64,
        base_url: provider.base_url.clone(),
    }
}

// ── get_agent_info ───────────────────────────────────────────────────────────

fn get_agent_info(data: &Value) -> Option<proto::AgentInfo> {
    let payload: crate::payloads_ext::AgentInfoPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::AgentInfo {
        version: payload.version,
        agent_instance_id: payload.agent_instance_id,
        skills_count: payload.skills_count as u64,
    })
}

// ── get_commands ─────────────────────────────────────────────────────────────

fn get_commands(data: &Value) -> Option<proto::CommandsResponse> {
    let payload: crate::payloads_ext::CommandsPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::CommandsResponse {
        commands: payload
            .commands
            .iter()
            .map(|command| proto::Command {
                name: command.name.clone(),
                description: command.description.clone(),
                name_zh: command.name_zh.clone(),
                description_zh: command.description_zh.clone(),
                source: command.source.clone(),
            })
            .collect(),
    })
}

// ── compact ──────────────────────────────────────────────────────────────────

fn compact(data: &Value) -> Option<proto::CompactResult> {
    let payload: crate::payloads_ext::CompactPayload = serde_json::from_value(data.clone()).ok()?;
    Some(proto::CompactResult {
        checkpoint_id: payload.checkpoint_id.unwrap_or_default(),
        already_compacted: payload.already_compacted.unwrap_or(false),
        tokens_before: payload.tokens_before,
        tokens_after: payload.tokens_after,
        summary: payload.summary,
        messages_removed: payload.messages_removed,
    })
}

// ── shell ────────────────────────────────────────────────────────────────────

fn shell(data: &Value) -> Option<proto::ShellResult> {
    let payload: crate::payloads_ext::ShellPayload = serde_json::from_value(data.clone()).ok()?;
    Some(proto::ShellResult {
        output: payload.output,
        exit_code: payload.exit_code,
    })
}

// ── cycle_model ──────────────────────────────────────────────────────────────

fn cycle_model(data: &Value) -> Option<proto::CycleModelResult> {
    let payload: crate::payloads_ext::CycleModelPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::CycleModelResult {
        model: payload.model,
        thinking_level: payload.thinking_level,
        is_scoped: payload.is_scoped,
    })
}

// ── sync_future_models ───────────────────────────────────────────────────────

fn sync_future_models(data: &Value) -> Option<proto::SyncFutureModelsResult> {
    let payload: crate::payloads_ext::SyncFutureModelsPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::SyncFutureModelsResult {
        synced: payload.synced,
        model_count: payload.model_count as u64,
        revision: payload.revision,
    })
}

// ── refresh_skills ───────────────────────────────────────────────────────────

fn refresh_skills(data: &Value) -> Option<proto::RefreshSkillsResult> {
    let payload: crate::payloads_ext::RefreshSkillsPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::RefreshSkillsResult {
        skills_count: payload.skills_count as u64,
        skills: payload.skills,
        refreshed: payload.refreshed,
    })
}

// ── get_session_stats ────────────────────────────────────────────────────────

fn get_session_stats(data: &Value) -> Option<proto::SessionStatsResponse> {
    let payload: crate::payloads_ext::SessionStatsPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::SessionStatsResponse {
        session_file: payload.session_file,
        session_id: payload.session_id,
        user_messages: payload.user_messages as u64,
        assistant_messages: payload.assistant_messages as u64,
        tool_calls: payload.tool_calls as u64,
        tool_results: payload.tool_results as u64,
        total_messages: payload.total_messages as u64,
        tokens: Some(proto::StatsTokens {
            input: payload.tokens.input,
            output: payload.tokens.output,
            cache_read: payload.tokens.cache_read,
            total: payload.tokens.total,
        }),
        // proto carries the number as double; the JSON-path struct keeps the
        // raw Value so the fallback round-trip loses nothing.
        cost: payload.cost.as_f64()?,
    })
}

// ── get_runtime_metrics ──────────────────────────────────────────────────────

fn get_runtime_metrics(data: &Value) -> Option<proto::RuntimeMetricsResponse> {
    let payload: crate::payloads_ext::RuntimeMetricsPayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::RuntimeMetricsResponse {
        session_id: payload.session_id,
        active_run_gauge: payload.active_run_gauge as u64,
        stale_epoch_drops: payload.stale_epoch_drops,
        persistence_degraded: payload.persistence_degraded,
        broadcast_lag: payload.broadcast_lag,
        ring_truncations: payload.ring_truncations,
        active_run_id: payload.active_run_id,
        queued_runs: payload.queued_runs as u64,
        queued_bytes: payload.queued_bytes as u64,
        event_journal_healthy: payload.event_journal_healthy,
        event_journal_error: payload.event_journal_error,
    })
}

// ── get_session_events_since ─────────────────────────────────────────────────

fn get_session_events_since(data: &Value) -> Option<proto::SessionEventsSinceResponse> {
    let payload: crate::payloads_ext::SessionEventsSincePayload =
        serde_json::from_value(data.clone()).ok()?;
    Some(proto::SessionEventsSinceResponse {
        events: payload
            .events
            .into_iter()
            .map(|event| proto::SessionEventRecord {
                r#type: event.event_type,
                data: event.data,
                session_id: event.session_id,
                session_idx: event.session_idx,
                event_id: event.event_id,
                timestamp: event.timestamp,
            })
            .collect(),
    })
}
