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
) -> Result<future_rpc::payloads::EventsSincePayload, crate::Error> {
    read_events_since(session_id, run_id, since_idx, None, None).await
}

pub(super) async fn read_events_since(
    session_id: String,
    run_id: String,
    since_idx: i64,
    page_limit: Option<i64>,
    until: Option<i64>,
) -> Result<future_rpc::payloads::EventsSincePayload, crate::Error> {
    let mut client = connect_agent().await?;
    let mut cursor = since_idx;
    let mut window = until;
    let mut merged: Option<future_rpc::payloads::EventsSincePayload> = None;
    loop {
        let command = future_rpc::proto::RpcCommand {
            run_id: run_id.clone(),
            since_idx: cursor,
            replay_until_idx: window,
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
                watermark: window,
            }
        });
        if let Some(watermark) = page.watermark {
            if window.is_some_and(|window| window != watermark) {
                return Err("replay_window_changed".into());
            }
            window = Some(watermark);
        }
        if let Some(until) = window {
            page.events.retain(|event| event.idx <= until);
            if page
                .projection
                .as_ref()
                .is_some_and(|projection| projection.cursor > until)
            {
                return Err("replay_window_changed".into());
            }
            if page.events.last().is_some_and(|event| event.idx >= until) {
                page.has_more = false;
            }
        }
        if page_limit.is_none() {
            for event in page.events.iter_mut().chain(
                page.projection
                    .iter_mut()
                    .flat_map(|projection| projection.events.iter_mut()),
            ) {
                if !future_rpc::content_body::required_inline(&event.event_type)
                    || !event.data.contains("\"content_refs\"")
                {
                    continue;
                }
                let mut data = serde_json::from_str(&event.data)?;
                future_rpc::content_body::hydrate_required(
                    &mut client,
                    &session_id,
                    &run_id,
                    &event.event_type,
                    &mut data,
                )
                .await
                .map_err(crate::Error::from)?;
                event.data = data.to_string();
            }
        }
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
        if page_limit.is_none() && page.has_more && next.is_none() {
            return Err("replay page did not advance its cursor".into());
        }
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
        watermark: window,
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
) -> Result<serde_json::Value, crate::Error> {
    serde_json::to_value(get_events_since_payload(session_id, run_id, since_idx).await?)
        .map_err(|error| format!("Could not serialize get_events_since response: {error}").into())
}

/// Optional semantic bootstrap. Only explicit legacy/size responses fall back
/// to raw replay; transport errors and corrupt snapshots must remain failures.
pub async fn get_run_snapshot(
    session_id: String,
    run_id: String,
) -> Result<Option<serde_json::Value>, crate::Error> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(future_rpc::proto::RpcCommand {
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
        || projection["events"]
            .as_array()
            .is_none_or(|events| events.is_empty())
    {
        return Err("get_run_snapshot returned an invalid projection".into());
    }
    Ok(Some(data))
}

/// One bounded Agent page for a remote continuation with a pinned watermark.
/// Unlike the native full-tail API, this must not drain subsequent pages.
pub async fn get_events_since_page(
    session_id: String,
    run_id: String,
    since_idx: i64,
    limit: usize,
) -> Result<serde_json::Value, crate::Error> {
    let limit = limit.min(EVENTS_PAGE_SIZE as usize) as i64;
    serde_json::to_value(read_events_since(session_id, run_id, since_idx, Some(limit), None).await?)
        .map_err(|error| format!("Could not serialize get_events_since response: {error}").into())
}

/// One source page in an immutable replay window, including its first read.
pub async fn get_events_since_page_until(
    session_id: String,
    run_id: String,
    since_idx: i64,
    limit: usize,
    until: Option<i64>,
) -> Result<serde_json::Value, crate::Error> {
    let limit = limit.min(EVENTS_PAGE_SIZE as usize) as i64;
    serde_json::to_value(
        read_events_since(session_id, run_id, since_idx, Some(limit), until).await?,
    )
    .map_err(|error| format!("Could not serialize get_events_since response: {error}").into())
}

/// Capability preflight reads metadata only, including a missing session's
/// prospective format. It must never hydrate history or create the session.
pub async fn get_session_storage_format(
    session_id: String,
) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("get_session_storage_format", session_id))
        .await
        .map_err(|error| format!("session format preflight failed: {error}"))?
        .into_inner()
        .ok_or_rpc_error("session format preflight rejected")?;
    Ok(future_rpc::decode::response_data(&response))
}

/// Read one committed immutable event field page. The Agent checks the exact
/// session/run, database restore generation and published stream coordinates.
pub async fn get_event_content_range(
    session_id: String,
    run_id: String,
    request: future_rpc::content_range::ContentRequest,
) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
    let response = client
        .execute_command(future_rpc::proto::RpcCommand {
            run_id,
            content_request: Some(request.into()),
            ..base_command("get_event_content_range", session_id)
        })
        .await
        .map_err(|error| format!("event content read failed: {error}"))?
        .into_inner()
        .ok_or_rpc_error("event content read rejected")?;
    Ok(future_rpc::decode::response_data(&response))
}

/// Fetch a session's full message history from the agent (LLM Message shape:
/// `{role, content, tool_calls?}` where `content` is a string or an array of
/// content blocks). The agent's JSONL is the source of truth for ALL sessions —
/// including TUI/CLI sessions the GUI store only holds as imported thread stubs
/// with no message rows — so the remote bridge serves history from here rather
/// than from the store.
pub async fn get_session_messages(session_id: String) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
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
pub async fn get_session_entries(session_id: String) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
    get_session_entries_with_client(&mut client, session_id).await
}

pub async fn get_session_entries_with_client(
    client: &mut super::client::AgentClient,
    session_id: String,
) -> Result<serde_json::Value, crate::Error> {
    let entries = fetch_all_session_entries_with_client(client, &session_id).await?;
    Ok(serde_json::json!({ "entries": entries }))
}

pub async fn query_tools(
    session_id: String,
    run_id: String,
    tool_call_id: Option<String>,
    offset: i64,
) -> Result<serde_json::Value, crate::Error> {
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

/// Fetch one tool call's stored arguments from the Agent.
///
/// The lean history page (a phone that declared `lean_events_v1`) drops a shell
/// call's `arguments` whole — it is the page's most expensive payload no phone
/// surface reads — and the phone asks for it back here when the row is opened.
/// Identity-scoped by session AND run so an id repeated elsewhere cannot answer.
pub async fn get_tool_call_args(
    session_id: String,
    run_id: String,
    tool_call_id: String,
) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(future_rpc::proto::RpcCommand {
            run_id,
            tool_call_id: Some(tool_call_id),
            ..base_command("get_tool_call_args", session_id)
        })
        .await
        .map_err(|error| format!("get_tool_call_args failed: {error}"))?
        .into_inner()
        .ok_or_rpc_error("get_tool_call_args rejected")?;
    Ok(future_rpc::decode::response_data(&response))
}

/// Fetch one backward, user-exchange-bounded history page from the Agent. This
/// keeps remote mobile paging end-to-end: the desktop no longer downloads the
/// complete Agent history again for every NATS page and then slices it locally.
pub async fn get_session_entries_before(
    session_id: String,
    before: i64,
    user_exchange_limit: i64,
) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
    get_session_entries_before_with_client(&client, session_id, before, user_exchange_limit).await
}

pub async fn get_session_entries_before_with_client(
    client: &super::client::AgentClient,
    session_id: String,
    before: i64,
    user_exchange_limit: i64,
) -> Result<serde_json::Value, crate::Error> {
    let page = read_history_page_with_client(
        client,
        get_session_entries_before_command(session_id, before, user_exchange_limit),
    )
    .await?;
    Ok(serde_json::json!({
        "entries": page.entries,
        "hasMore": page.has_more,
        "nextOffset": page.next_offset,
    }))
}

/// Whether a session holds any record a reader can open.
///
/// A task run reads this when it settles: a conversation the run opened and
/// never wrote into is a shell — a name in the sidebar and nothing behind it —
/// because the prompt never reached the Agent (or the Agent refused it). Any
/// record at all keeps the conversation; the criterion is "no records", not how
/// long the run took or how often it called a model.
///
/// Every session is born with a `session_info` snapshot, so that one entry is
/// what a session with nothing in it contains — it is metadata, not a record
/// (`future session transcript` shows the same session as empty). The page also
/// reports whether it was cut short: a page with more behind it is never an
/// empty session, whatever its first entries happen to be.
///
/// `Err` means the Agent could not be asked, which callers must treat as *not*
/// empty: a read failure is not evidence, and deleting a conversation on one
/// would throw away a run's only record.
pub async fn session_has_entries(session_id: &str) -> Result<bool, crate::Error> {
    const PAGE: i64 = 8;
    let client = connect_agent().await?;
    let page = read_history_page_with_client(
        &client,
        get_session_entries_page_command(session_id.to_string(), 0, PAGE),
    )
    .await?;
    if page.has_more {
        return Ok(true);
    }
    Ok(page
        .entries
        .iter()
        .any(|entry| entry.kind != future_agent::session::ENTRY_TYPE_SESSION_INFO))
}

/// Fetch all of a session's entries, paged.
pub(super) async fn fetch_all_session_entries_with_client(
    client: &mut super::client::AgentClient,
    session_id: &str,
) -> Result<Vec<future_rpc::payloads::SessionEntryPayload>, crate::Error> {
    const PAGE_SIZE: i64 = 250;
    let mut offset = 0_i64;
    let mut entries = Vec::new();
    loop {
        let page = read_history_page_with_client(
            client,
            get_session_entries_page_command(session_id.to_string(), offset, PAGE_SIZE),
        )
        .await?;
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

async fn read_history_page_with_client(
    client: &super::client::AgentClient,
    command: future_rpc::proto::RpcCommand,
) -> Result<future_rpc::payloads::SessionEntriesPage, crate::Error> {
    future_rpc::history::read_page(command, |command| {
        // Cloning a tonic client shares the existing channel/connection.
        let mut client = client.clone();
        async move {
            let response = client
                .execute_command(command)
                .await
                .map_err(|e| format!("get_session_entries failed: {e}"))?
                .into_inner();
            if super::client::is_session_not_found(&response) {
                return Err("Conversation history is unavailable: restore its matching Agent database or legacy sessions backup.".into());
            }
            let response = response
                .ok_or_rpc_error("get_session_entries returned an error")
                .map_err(|error| error.to_string())?;
            future_rpc::decode::decode_session_entries_page(&response)
                .ok_or_else(|| "invalid history page".to_owned())
        }
    })
    .await
    .map_err(Into::into)
}

/// Fetch the session's current state (model, thinkingLevel, isStreaming, etc.)
/// from the agent. Used by the remote bridge to populate the web client's
/// model/thinking selectors.
pub async fn get_session_state(session_id: String) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
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
pub async fn get_available_models() -> Result<serde_json::Value, crate::Error> {
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

pub async fn probe_sandbox() -> Result<SandboxProbeResult, crate::Error> {
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

pub async fn probe_windows_sandbox() -> Result<WindowsSandboxProbeResult, crate::Error> {
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

pub async fn reset_windows_sandbox() -> Result<usize, crate::Error> {
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
pub async fn set_session_model(session_id: String, model_id: String) -> Result<(), crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
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
pub async fn set_default_model(model_id: String) -> Result<(), crate::Error> {
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
) -> Result<(), crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
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
) -> Result<serde_json::Value, crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
    let command = future_rpc::proto::RpcCommand {
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
pub async fn rename_session(session_id: String, name: String) -> Result<(), crate::Error> {
    let mut client = connect_agent().await?;
    super::verify_session_database(&mut client, &session_id).await?;
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
pub async fn provision_agent_session(
    thread_id: &str,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<String, crate::Error> {
    provision_agent_session_with_policy(thread_id, "", model_id, thinking_level, "workspace", None)
        .await
}

/// Provision an agent session with an explicit permission level and sandbox
/// tier. Used by tasks (full permission, no approval prompts) — the default
/// `provision_agent_session` keeps the GUI's workspace-level default.
///
/// `session_name` is the title the session is created with. Empty leaves the
/// Agent to title it from the first user message (right for a conversation a
/// person starts); a task run passes its own name, because its first message is
/// the `── future_tasks_run_envelope_v1 ──` envelope — the sidebar would
/// otherwise show that header as the conversation's name.
pub async fn provision_agent_session_with_policy(
    thread_id: &str,
    session_name: &str,
    model_id: Option<String>,
    thinking_level: Option<String>,
    permission_level: &str,
    sandbox_tier: Option<&str>,
) -> Result<String, crate::Error> {
    let cwd = workspace_path_for_thread(thread_id)?;
    let mut client = connect_agent().await?;
    let ensured = ensure_named_agent_session(
        &mut client,
        "",
        &cwd,
        session_name,
        model_id.as_deref(),
        thinking_level.as_deref(),
    )
    .await?;
    let session_id = ensured.session_id;
    crate::store::bind_thread_session_id(thread_id, &session_id)?;
    set_agent_permission_level(&mut client, &session_id, permission_level).await?;
    match sandbox_tier {
        Some(tier) => set_agent_sandbox_policy_tier(&mut client, &session_id, tier).await?,
        None => set_agent_sandbox_policy(&mut client, &session_id, thread_id).await?,
    }
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
pub async fn reload_agent_credentials() -> Result<(), crate::Error> {
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
/// Best-effort: an unavailable agent yields a
/// zeroed result rather than an error.
pub async fn sync_future_models() -> Result<SyncFutureModelsResult, crate::Error> {
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

/// Result of [`request_future_models_refresh`]: whether the agent started a
/// background catalogue refresh for this call. `false` is not an error — the
/// agent's backoff window may still be open, another refresh may be running, or
/// there may be no Future credential at all.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestFutureModelsRefreshResult {
    pub requested: bool,
}

/// Ask the agent to refresh the Future catalogue **in the background**: the call
/// returns as soon as the fetch is scheduled, and the refreshed list reaches the
/// UI later through the agent's `provider_config_changed` event. Used by the
/// model pickers, which must open immediately instead of waiting on the platform.
/// Best-effort: an unavailable agent yields `requested: false` rather than an
/// error, because a picker opening is not a place to report connection failures.
pub async fn request_future_models_refresh(
) -> Result<RequestFutureModelsRefreshResult, crate::Error> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        Err(_) => return Ok(RequestFutureModelsRefreshResult { requested: false }),
    };
    let response = client
        .execute_command(base_command("request_future_models_refresh", String::new()))
        .await
        .map_err(|status| map_rpc_error("Unable to request a Future Agent model refresh", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the model refresh request.")?;
    serde_json::from_value::<RequestFutureModelsRefreshResult>(future_rpc::decode::response_data(
        &response,
    ))
    .map_err(|error| format!("Future Agent returned an invalid refresh result: {error}").into())
}

#[cfg(test)]
mod probe_tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::test_support::agent::{mock_agent, Reply, TestHome};

    /// The sandbox probes are the Settings page's capability read: the desktop
    /// asks, the Agent answers, and a refusal is an error the page shows. Both
    /// probes and the reset are asserted through the command they issue and the
    /// shape that comes back, because a wrong field name here reads as "the
    /// sandbox is unavailable" on a machine where it works.
    #[tokio::test]
    async fn the_sandbox_probes_ask_the_agent_and_map_its_answer() {
        let _home = TestHome::new("queries-sandbox");
        let mock = mock_agent();

        mock.push_data(
            "probe_sandbox",
            serde_json::json!({
                "available": true,
                "code": "ok",
                "backend": "seatbelt",
                "path": "/usr/bin/sandbox-exec",
                "version": "1",
                "capabilities": {"network": false},
            }),
        );
        let probe = probe_sandbox().await.expect("sandbox probe");
        assert!(probe.available);
        assert_eq!(probe.backend, "seatbelt");
        assert_eq!(probe.path.as_deref(), Some("/usr/bin/sandbox-exec"));
        assert_eq!(
            probe.capabilities,
            Some(serde_json::json!({"network": false}))
        );
        assert_eq!(mock.requests_of("probe_sandbox").len(), 1);

        mock.push_data(
            "probe_windows_sandbox",
            serde_json::json!({"available": false, "code": "not_windows"}),
        );
        let windows = probe_windows_sandbox().await.expect("windows probe");
        assert!(!windows.available);
        assert_eq!(windows.code, "not_windows");

        mock.push_data(
            "reset_windows_sandbox",
            serde_json::json!({"removedCapabilities": 2}),
        );
        assert_eq!(reset_windows_sandbox().await.expect("reset"), 2);

        // A reset reply without the count is refused rather than reported as
        // "nothing was removed": the page uses the number to say what changed.
        mock.push_data("reset_windows_sandbox", serde_json::json!({}));
        let error = reset_windows_sandbox().await.expect_err("no count");
        assert!(
            error
                .to_string()
                .contains("Invalid Windows sandbox reset response"),
            "{error}"
        );

        // A rejected probe is an error, with the Agent's own sentence.
        mock.push("probe_sandbox", Reply::Reject("probe failed".into()));
        let error = probe_sandbox().await.expect_err("rejected");
        assert_eq!(
            error.to_string(),
            "probe failed",
            "the Agent's own sentence is what the page shows"
        );
    }

    /// The immutable content page the timeline asks for when a message was
    /// stored out of line: the caller passes the coordinates it was given, and
    /// the range comes back for exactly that run.
    #[tokio::test]
    async fn an_event_content_range_is_read_for_the_run_it_names() {
        let _home = TestHome::new("queries-content-range");
        let mock = mock_agent();
        mock.push_data(
            "get_event_content_range",
            serde_json::json!({"text": "hello", "nextOffset": 5, "complete": true}),
        );

        let range = get_event_content_range(
            "sess_1".into(),
            "run_1".into(),
            future_rpc::content_range::ContentRequest {
                database_id: "db".into(),
                restore_generation: "gen".into(),
                epoch: 7,
                cursor: 42,
                field: "text".into(),
                offset: 0,
                max_bytes: 1024,
            },
        )
        .await
        .expect("content range");

        assert_eq!(range["text"], serde_json::json!("hello"));
        assert_eq!(range["complete"], serde_json::json!(true));
        let requests = mock.requests_of("get_event_content_range");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].run_id, "run_1");
        let sent = requests[0]
            .content_request
            .as_ref()
            .expect("the request is forwarded");
        assert_eq!(sent.cursor, 42);
        assert_eq!(sent.field, "text");
        assert_eq!(sent.max_bytes, 1024);
    }

    /// A settled run whose projection the Agent can no longer serve is `None`,
    /// not an error: the caller falls back to the journal instead of showing a
    /// broken conversation. The two codes the Agent uses for that are the ones
    /// the fallback keys on.
    #[tokio::test]
    async fn an_unavailable_run_snapshot_falls_back_to_none() {
        let _home = TestHome::new("queries-run-snapshot");
        let mock = mock_agent();

        for code in ["run_snapshot_too_large", "run_snapshot_unavailable"] {
            mock.push(
                "get_run_snapshot",
                Reply::RejectCode(code, "the projection cannot be served".into()),
            );
            assert!(
                get_run_snapshot("sess_1".into(), "run_1".into())
                    .await
                    .expect("a missing projection is not an error")
                    .is_none(),
                "{code} must fall back"
            );
        }

        // Any other refusal is reported: it is a fault, not a projection that
        // aged out, and silently reading an empty journal would hide it.
        mock.push(
            "get_run_snapshot",
            Reply::RejectCode("internal_error", "boom".into()),
        );
        let error = get_run_snapshot("sess_1".into(), "run_1".into())
            .await
            .expect_err("a real fault");
        assert_eq!(error.to_string(), "boom", "the Agent's error is reported");
    }
}
