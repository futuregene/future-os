use super::*;

/// Events requested per get_events_since page. A long run's journal far
/// exceeds the gRPC message cap when returned whole (every event crosses the
/// wire about three times under the typed dual-write), so full-tail reads
/// page through it. The server additionally bounds a page by a
/// serialized-size budget, keeping pages safe even for runs with multi-MB
/// tool outputs.
pub(super) const EVENTS_PAGE_SIZE: i64 = 50_000;

/// Cursor advance for the get_events_since page loop: `Some(next)` to keep
/// paging from `next`, `None` when the tail is complete. Terminates on a
/// malformed has_more page (no advancing idx) instead of re-requesting the
/// same cursor forever.
pub(super) fn next_events_cursor(
    page: &future_rpc::payloads::EventsSincePayload,
    cursor: i64,
) -> Option<i64> {
    if !page.has_more {
        return None;
    }
    page.events
        .last()
        .map(|event| event.idx)
        .filter(|idx| *idx > cursor)
}

/// Fetch the agent's buffered events for a session's current run (P1c backfill).
/// `since_idx = -1` returns the requested run's retained prefix. A stale or
/// unknown `run_id` is an explicit error and never realigns to another run.
/// Returns the canonical typed replay payload. Typed protobuf and legacy JSON
/// responses are normalized once in `future-rpc`; callers never inspect raw
/// response JSON or oneof fields themselves.
///
/// Paged under the hood (`max_events`): the returned envelope holds the
/// complete tail regardless of journal size.
pub async fn get_events_since_payload(
    session_id: String,
    run_id: String,
    since_idx: i64,
) -> Result<future_rpc::payloads::EventsSincePayload, crate::AppError> {
    read_events_since(session_id, run_id, since_idx, None).await
}

pub(super) async fn read_events_since(
    session_id: String,
    run_id: String,
    since_idx: i64,
    page_limit: Option<i64>,
) -> Result<future_rpc::payloads::EventsSincePayload, crate::AppError> {
    let mut client = connect_agent().await?;
    let mut cursor = since_idx;
    let mut merged: Option<future_rpc::payloads::EventsSincePayload> = None;
    loop {
        let command = crate::agent_proto::RpcCommand {
            run_id: run_id.clone(),
            since_idx: cursor,
            max_events: page_limit
                .unwrap_or(EVENTS_PAGE_SIZE)
                .clamp(1, EVENTS_PAGE_SIZE),
            ..base_command("get_events_since", session_id.clone())
        };
        let response = client
            .execute_command(command)
            .await
            .map_err(|status| {
                // OutOfRange here is the agent rejecting its own oversized
                // response at the 32 MiB encode cap: it serialized the whole
                // tail even though this client always requests paged reads, so
                // the running agent almost certainly predates the
                // get_events_since paging protocol (proto max_events). Paging
                // is server-side — only an agent restart on a current build
                // fixes it. (A single event over ~10 MiB would trip the same
                // cap even on a current agent.)
                if status.code() == tonic::Code::OutOfRange {
                    format!(
                        "get_events_since failed: {status} — the agent exceeded the 32 MiB gRPC cap on a paged read, so the running agent likely predates get_events_since paging (max_events); rebuild and restart the agent"
                    )
                } else {
                    format!("get_events_since failed: {status}")
                }
            })?
            .into_inner()
            .ok_or_rpc_error("get_events_since returned an error")?;
        let mut page = future_rpc::decode::decode_events_since(&response).unwrap_or_else(|| {
            future_rpc::payloads::EventsSincePayload {
                run_id: run_id.clone(),
                events: Vec::new(),
                truncated: false,
                projection: None,
                has_more: false,
            }
        });
        if !page.run_id.is_empty() && page.run_id != run_id {
            return Err(format!(
                "get_events_since returned run {}, expected {run_id}",
                page.run_id
            )
            .into());
        }
        if page.run_id.is_empty() {
            page.run_id.clone_from(&run_id);
        }
        let next = if page_limit.is_some() {
            None
        } else {
            next_events_cursor(&page, cursor)
        };
        match &mut merged {
            None => merged = Some(page),
            Some(total) => {
                total.truncated |= page.truncated;
                if total.projection.is_none() {
                    total.projection = page.projection.take();
                }
                total.events.append(&mut page.events);
            }
        }
        match next {
            Some(next_cursor) => cursor = next_cursor,
            None => break,
        }
    }
    let mut result = merged.unwrap_or(future_rpc::payloads::EventsSincePayload {
        run_id,
        events: Vec::new(),
        truncated: false,
        projection: None,
        has_more: false,
    });
    // Only the full-tail API has drained all Agent pages. A bounded remote
    // read must preserve has_more so the phone continues from its cursor.
    if page_limit.is_none() {
        result.has_more = false;
    }
    Ok(result)
}

/// JSON compatibility wrapper for remote/web consumers. Native Desktop code
/// uses [`get_events_since_payload`] directly and therefore stays strongly
/// typed across the RPC boundary.
pub async fn get_events_since(
    session_id: String,
    run_id: String,
    since_idx: i64,
) -> Result<serde_json::Value, crate::AppError> {
    serde_json::to_value(get_events_since_payload(session_id, run_id, since_idx).await?)
        .map_err(|error| format!("Could not serialize get_events_since response: {error}").into())
}

/// Optional semantic bootstrap. Only explicit legacy/size responses fall back
/// to raw replay; transport errors and corrupt snapshots must remain failures.
pub(crate) async fn get_run_snapshot(
    session_id: String,
    run_id: String,
) -> Result<Option<serde_json::Value>, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(crate::agent_proto::RpcCommand {
            run_id: run_id.clone(),
            ..base_command("get_run_snapshot", session_id)
        })
        .await
        .map_err(|status| format!("get_run_snapshot failed: {status}"))?
        .into_inner();
    if !response.success
        && (response.error == "unknown command: get_run_snapshot"
            || matches!(
                response.error_code.as_str(),
                "run_snapshot_too_large" | "run_snapshot_unavailable"
            ))
    {
        return Ok(None);
    }
    let response = response.ok_or_rpc_error("get_run_snapshot returned an error")?;
    let data = future_rpc::decode::response_data(&response);
    let projection = &data["projection"];
    let cursor = projection["cursor"].as_i64();
    if data["runSnapshot"] != true
        || projection["runId"] != run_id
        || cursor.is_none_or(|cursor| cursor < 0)
        || data["watermark"].as_i64() != cursor
        || !projection["events"]
            .as_array()
            .is_some_and(|events| !events.is_empty())
    {
        return Err("get_run_snapshot returned an invalid projection".into());
    }
    Ok(Some(data))
}

/// One bounded Agent page for a remote continuation with a pinned watermark.
/// Unlike the native full-tail API, this must not drain subsequent pages.
pub(crate) async fn get_events_since_page(
    session_id: String,
    run_id: String,
    since_idx: i64,
    limit: usize,
) -> Result<serde_json::Value, crate::AppError> {
    let limit = limit.min(EVENTS_PAGE_SIZE as usize) as i64;
    serde_json::to_value(read_events_since(session_id, run_id, since_idx, Some(limit)).await?)
        .map_err(|error| format!("Could not serialize get_events_since response: {error}").into())
}

/// Fetch a session's full message history from the agent (LLM Message shape:
/// `{role, content, tool_calls?}` where `content` is a string or an array of
/// content blocks). The agent's JSONL is the source of truth for ALL sessions —
/// including TUI/CLI sessions the GUI store only holds as imported thread stubs
/// with no message rows — so the remote bridge serves history from here rather
/// than from the store.
pub async fn get_session_messages(
    session_id: String,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("get_messages", session_id))
        .await
        .map_err(|status| format!("get_messages failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_messages returned an error")?;
    if response.data.is_empty() {
        Ok(serde_json::json!({ "messages": [] }))
    } else {
        Ok(future_rpc::decode::response_data(&response))
    }
}

/// Fetch a session's display entries (user/assistant/tool + session_info) from
/// the agent. Unlike `get_session_messages` (LLM wire shape), entries are
/// display-shaped — plain-text content plus per-entry `meta` (user attachments
/// with cached thumbnails), which is how the GUI rebuilds attachment chips.
pub async fn get_session_entries(session_id: String) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let entries = fetch_all_session_entries_with_client(&mut client, &session_id).await?;
    Ok(serde_json::json!({ "entries": entries }))
}

pub(crate) async fn query_tools(
    session_id: String,
    run_id: String,
    tool_call_id: Option<String>,
    offset: i64,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let command = if tool_call_id.is_some() {
        "get_tool_output"
    } else {
        "list_tool_calls"
    };
    let response = client
        .execute_command(future_rpc::proto::RpcCommand {
            run_id,
            tool_call_id,
            offset: Some(offset),
            limit: Some(100),
            ..base_command(command, session_id)
        })
        .await
        .map_err(|error| format!("tool query failed: {error}"))?
        .into_inner()
        .ok_or_rpc_error("tool query rejected")?;
    Ok(future_rpc::decode::response_data(&response))
}

/// Fetch one backward, user-exchange-bounded history page from the Agent. This
/// keeps remote mobile paging end-to-end: the desktop no longer downloads the
/// complete Agent history again for every NATS page and then slices it locally.
pub async fn get_session_entries_before(
    session_id: String,
    before: i64,
    user_exchange_limit: i64,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_session_entries_before_command(
            session_id,
            before,
            user_exchange_limit,
        ))
        .await
        .map_err(|status| format!("get_session_entries failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_session_entries returned an error")?;
    let page = future_rpc::decode::decode_session_entries_page(&response)
        .ok_or_else(|| "get_session_entries typed page is missing or invalid".to_string())?;
    Ok(serde_json::json!({
        "entries": page.entries,
        "hasMore": page.has_more,
        "nextOffset": page.next_offset,
    }))
}

pub(super) async fn fetch_all_session_entries_with_client(
    client: &mut crate::agent_proto::FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
) -> Result<Vec<future_rpc::payloads::SessionEntryPayload>, crate::AppError> {
    const PAGE_SIZE: i64 = 250;
    let mut offset = 0_i64;
    let mut entries = Vec::new();
    loop {
        let response = client
            .execute_command(get_session_entries_page_command(
                session_id.to_string(),
                offset,
                PAGE_SIZE,
            ))
            .await
            .map_err(|status| format!("get_session_entries failed: {status}"))?
            .into_inner()
            .ok_or_rpc_error("get_session_entries returned an error")?;
        let page = future_rpc::decode::decode_session_entries_page(&response)
            .ok_or_else(|| "get_session_entries typed page is missing or invalid".to_string())?;
        entries.extend(page.entries);
        if !page.has_more {
            return Ok(entries);
        }
        if page.next_offset <= offset {
            return Err(format!(
                "get_session_entries returned a non-advancing offset: {} after {offset}",
                page.next_offset
            )
            .into());
        }
        offset = page.next_offset;
    }
}

/// Fetch the session's current state (model, thinkingLevel, isStreaming, etc.)
/// from the agent. Used by the remote bridge to populate the web client's
/// model/thinking selectors.
pub async fn get_session_state(session_id: String) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_state_command(session_id))
        .await
        .map_err(|status| format!("get_state failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_state returned an error")?;
    let value = future_rpc::decode::response_data(&response);
    Ok(if value.is_null() {
        serde_json::json!({})
    } else {
        value
    })
}

/// Fetch the available model list from the agent (for the web client's model selector).
pub async fn get_available_models() -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_available_models_command())
        .await
        .map_err(|status| format!("get_available_models failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("get_available_models returned an error")?;
    let value = future_rpc::decode::response_data(&response);
    Ok(if value.is_null() {
        serde_json::json!({ "models": [] })
    } else {
        value
    })
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProbeResult {
    pub available: bool,
    pub code: String,
    pub backend: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub capabilities: Option<serde_json::Value>,
}

pub async fn probe_sandbox() -> Result<SandboxProbeResult, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("probe_sandbox", String::new()))
        .await
        .map_err(|status| format!("probe_sandbox failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent could not check sandbox availability.")?;
    serde_json::from_value(future_rpc::decode::response_data(&response))
        .map_err(|error| format!("Invalid sandbox probe response: {error}").into())
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsSandboxProbeResult {
    pub available: bool,
    pub code: String,
}

pub async fn probe_windows_sandbox() -> Result<WindowsSandboxProbeResult, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("probe_windows_sandbox", String::new()))
        .await
        .map_err(|status| format!("probe_windows_sandbox failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent could not check Windows write protection.")?;
    serde_json::from_value(future_rpc::decode::response_data(&response))
        .map_err(|error| format!("Invalid Windows sandbox probe response: {error}").into())
}

pub async fn reset_windows_sandbox() -> Result<usize, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("reset_windows_sandbox", String::new()))
        .await
        .map_err(|status| format!("reset_windows_sandbox failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent could not reset Windows write protection.")?;
    let data = future_rpc::decode::response_data(&response);
    data.get("removedCapabilities")
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| "Invalid Windows sandbox reset response.".into())
}

/// Set the model on a live agent session (remote bridge).
pub async fn set_session_model(
    session_id: String,
    model_id: String,
) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_model_command(model_id, session_id))
        .await
        .map_err(|status| format!("set_model failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_model returned an error")?;
    Ok(())
}

/// Persist the onboarding model-picker's choice as the agent's global default
/// model (sessionless `set_default_model` RPC → settings.json `defaultModel`).
pub async fn set_default_model(model_id: String) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_default_model_command(model_id))
        .await
        .map_err(|status| format!("set_default_model failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_default_model returned an error")?;
    Ok(())
}

/// Set the thinking level on a live agent session (remote bridge).
pub async fn set_session_thinking_level(
    session_id: String,
    level: String,
) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_thinking_level_command(level, session_id))
        .await
        .map_err(|status| format!("set_thinking_level failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_thinking_level returned an error")?;
    Ok(())
}

/// Generate a suggestion only; callers decide whether and when to save it.
pub async fn generate_session_title(
    session_id: String,
    language: String,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let command = crate::agent_proto::RpcCommand {
        mode: language,
        ..base_command("generate_session_title", session_id)
    };
    let response = client
        .execute_command(command)
        .await
        .map_err(|status| map_rpc_error("Title generation failed", status))?
        .into_inner()
        .ok_or_rpc_error("Title generation failed")?;
    let data = future_rpc::decode::response_data(&response);
    if data["title"]
        .as_str()
        .is_none_or(|title| title.trim().is_empty())
    {
        return Err("Model returned an empty title".into());
    }
    Ok(data)
}

/// Rename a session: update the agent's session name, then mirror to the GUI store.
pub async fn rename_session(session_id: String, name: String) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    client
        .execute_command(set_session_name_command(name.clone(), session_id.clone()))
        .await
        .map_err(|status| format!("set_session_name failed: {status}"))?
        .into_inner()
        .ok_or_rpc_error("set_session_name returned an error")?;
    // Mirror to GUI store so the sidebar title stays in sync.
    if let Ok(Some(thread)) = crate::store::find_thread_by_agent_session(&session_id) {
        let _ = crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id,
            title: name,
        });
    }
    Ok(())
}

/// Create a fresh agent session for a just-created thread and persist the
/// agent-generated session id back onto the thread. Used by the remote
/// `new_session` command so the client receives the *real* agent session id up
/// front. If we instead handed the client the thread id, the agent would run
/// the subsequent prompt under a different (agent-generated) id and every
/// event subject / history lookup on the client would mismatch — events get
/// filtered out and `get_messages` finds nothing.
pub(crate) async fn provision_agent_session(
    thread_id: &str,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<String, crate::AppError> {
    let cwd = workspace_path_for_thread(thread_id)?;
    let mut client = connect_agent().await?;
    // Empty stored id → the agent generates a real session id, seeded with the
    // caller's model / thinking selections (matches the GUI new-chat draft).
    let ensured = ensure_agent_session(
        &mut client,
        "",
        &cwd,
        model_id.as_deref(),
        thinking_level.as_deref(),
    )
    .await?;
    let session_id = ensured.session_id;
    set_agent_permission_level(&mut client, &session_id, "workspace").await?;
    set_agent_sandbox_policy(&mut client, &session_id, thread_id).await?;
    crate::store::update_thread_session_id(thread_id, &session_id)?;
    Ok(session_id)
}

/// Tell the running agent to re-read `auth.json` and refresh every live
/// session's in-memory API key.
///
/// Since audit item 2 this is the FALLBACK-ONLY refresher: the primary config
/// writes go through `agent_bridge::config` (set_auth / upsert_provider /
/// delete_provider), and the agent refreshes its own live sessions inline. This
/// `reload_auth` round-trip is still sent after a LOCAL file write — used when
/// the agent is unreachable or pre-item-2 — because the agent caches the
/// resolved key inside each session's provider and the prompt path never
/// re-reads `auth.json`, so without it a session keeps serving prompts with a
/// stale key (e.g. still answering after logout) while the model list — which
/// does re-read disk — already shows logged-out.
///
/// Best-effort: if the agent isn't running there's no in-memory state to
/// refresh, so an unavailable agent is treated as success.
#[cfg(test)]
pub async fn reload_agent_credentials() -> Result<(), crate::AppError> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        // connect_agent's only error kind is AgentUnavailable (every failure
        // it constructs is one), and a down agent has no in-memory state to
        // refresh — treated as success.
        Err(_) => return Ok(()),
    };
    client
        .execute_command(base_command("reload_auth", String::new()))
        .await
        // Transport-level Unavailable → AgentUnavailable → treated as success
        // above ("no in-memory state to refresh on a down agent").
        .map_err(|status| map_rpc_error("Unable to refresh Future Agent credentials", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the credential refresh.")?;
    Ok(())
}

/// Result of [`sync_future_models`]: whether the platform fetch populated the
/// agent's model cache, and the number of Future models in that cache.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFutureModelsResult {
    pub synced: bool,
    pub model_count: usize,
    // Older Agents predate the typed-RPC revision field. Preserve the legacy
    // JSON fallback by treating its absence as the initial revision.
    #[serde(default)]
    pub revision: i64,
}

/// Ask the agent to synchronously fetch the Future provider's models (warming
/// its cache) and rebuild its model registry, so the next [`list_agent_models`]
/// returns a complete list. Used by post-login setup, manual refresh, and the
/// desktop's low-frequency maintenance schedule.
/// Best-effort like [`reload_agent_credentials`]: an unavailable agent yields a
/// zeroed result rather than an error.
pub async fn sync_future_models() -> Result<SyncFutureModelsResult, crate::AppError> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        // connect_agent's only error kind is AgentUnavailable: an unavailable
        // agent yields a zeroed result rather than an error.
        Err(_) => {
            return Ok(SyncFutureModelsResult {
                model_count: 0,
                synced: false,
                revision: 0,
            });
        }
    };
    let response = client
        .execute_command(base_command("sync_future_models", String::new()))
        .await
        .map_err(|status| map_rpc_error("Unable to sync Future Agent models", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the model sync.")?;
    serde_json::from_value::<SyncFutureModelsResult>(future_rpc::decode::response_data(&response))
        .map_err(|error| format!("Future Agent returned invalid sync result: {error}").into())
}
