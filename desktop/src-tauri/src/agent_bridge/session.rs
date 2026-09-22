//! Agent session lifecycle: ensure/create a session for a thread, set its
//! permission level, and resolve a thread's workspace path and prior-message
//! count. These back the per-prompt setup in the parent module.

use std::collections::HashMap;

use super::client::{
    fork_command, get_state_command, new_session_command, set_cwd_command,
    set_permission_level_command, set_sandbox_policy_command, RpcResponseExt,
};
use crate::store;

/// Outcome of `ensure_agent_session`.
#[derive(Debug)]
pub(super) struct EnsuredSession {
    pub session_id: String,
    /// True only when the caller supplied no identity and this function minted
    /// the first session for an unbound thread. Existing thread/session
    /// identity is immutable here: metadata may converge, identity may not.
    pub created: bool,
}

/// Ensure an agent session exists for the given thread. Returns the session
/// id (the existing one, or the newly-created one if the agent generated it).
/// `model_id` and `thinking_level` are applied to newly-created sessions so
/// the agent starts with the user's selection immediately.
pub(super) async fn ensure_agent_session(
    client: &mut super::client::AgentClient,
    session_id: &str,
    cwd: &str,
    model_id: Option<&str>,
    thinking_level: Option<&str>,
) -> Result<EnsuredSession, crate::AppError> {
    // If the thread already has a stored session id, check if it's still valid.
    if !session_id.is_empty() {
        let response = client
            .execute_command(get_state_command(session_id.to_string()))
            .await
            .map_err(|error| format!("Unable to inspect Future Agent session: {error}"))?
            .into_inner();

        if response.success {
            let value = future_rpc::decode::response_data(&response);
            let active_id = value
                .get("sessionId")
                .and_then(|id| id.as_str())
                .unwrap_or_default();
            let active_cwd = value
                .get("cwd")
                .and_then(|cwd| cwd.as_str())
                .unwrap_or_default();
            if active_id != session_id {
                return Err(format!(
                    "Future Agent returned session {active_id:?} while inspecting {session_id:?}."
                )
                .into());
            }
            if active_cwd != cwd {
                // The Desktop thread owns the workspace binding. A cwd drift is
                // metadata that can be repaired in place; it is not evidence
                // that the Agent lost the conversation. Replacing this live
                // session used to discard its complete history (most visibly
                // when returning to the parent after a fork).
                client
                    .execute_command(set_cwd_command(cwd.to_string(), session_id.to_string()))
                    .await
                    .map_err(|error| {
                        format!("Unable to restore Future Agent session workspace: {error}")
                    })?
                    .into_inner()
                    .ok_or_rpc_error("Future Agent rejected the session workspace repair.")?;
            }
            return Ok(EnsuredSession {
                session_id: session_id.to_string(),
                created: false,
            });
        } else if is_missing_session_error(&response.error) {
            return Err(format!(
                "Future Agent session {session_id:?} is missing. The conversation binding was preserved instead of replacing its history with an empty session."
            )
            .into());
        } else {
            return Err(format!(
                "Future Agent could not load the existing session: {}",
                response.error
            )
            .into());
        }
    }

    // Create a new session. Pass empty session_id to let the agent generate it.
    let resp = client
        .execute_command(new_session_command(
            String::new(),
            cwd.to_string(),
            "desktop",
            crate::device_identity::device_id_or_empty(),
            serde_json::Value::Null,
            model_id.map(str::to_string),
            thinking_level.map(str::to_string),
        ))
        .await
        .map_err(|error| format!("Unable to create Future Agent session: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session initialization.")?;

    let new_id = serde_json::from_str::<serde_json::Value>(&resp.data)
        .ok()
        .and_then(|v| v.get("sessionId").cloned())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    if new_id.is_empty() {
        return Err("Future Agent created a session without returning its identity.".into());
    }

    Ok(EnsuredSession {
        session_id: new_id,
        created: true,
    })
}

fn is_missing_session_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("session not found") || error.contains("no such session")
}

pub(super) async fn set_agent_permission_level(
    client: &mut super::client::AgentClient,
    session_id: &str,
    level: &str,
) -> Result<(), crate::AppError> {
    client
        .execute_command(set_permission_level_command(
            level.to_string(),
            session_id.to_string(),
        ))
        .await
        .map_err(|error| format!("Unable to set Future Agent permission level: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the permission level selection.")?;
    Ok(())
}

/// Push the session's approval tier to the agent. The agent reads the rule
/// files (`${WS}/.future/approval_rule.json`, `~/.future/approval_rule.json`)
/// directly — only the tier travels over the wire (docs/internals/desktop/SANDBOX/COMMON.md):
/// `"manual"` (ask), `"sandbox"` (the available OS sandbox wraps shell commands), or `"off"`
/// (fully open). The tier is a global app preference, defaulting to `"manual"`.
pub(super) async fn set_agent_sandbox_policy(
    client: &mut super::client::AgentClient,
    session_id: &str,
    _thread_id: &str,
) -> Result<(), crate::AppError> {
    let tier = match store::get_app_settings() {
        Ok(settings) => settings.approval_tier,
        Err(error) => {
            // A settings read failure must never widen access. Keep the run
            // usable behind explicit approval; the next successful settings
            // read will apply the user's persisted selection again.
            eprintln!(
                "FutureOS: approval settings unavailable; falling back to manual approval: {error}"
            );
            "manual".to_string()
        }
    };
    let policy = crate::agent_proto::SandboxPolicy { tier: tier.clone() };
    let response = client
        .execute_command(set_sandbox_policy_command(policy, session_id.to_string()))
        .await
        .map_err(|error| format!("Unable to set Future Agent sandbox policy: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the sandbox policy.")?;

    let sandbox_available = future_rpc::decode::response_data(&response)
        .get("sandboxAvailable")
        .and_then(serde_json::Value::as_bool);
    if tier == "sandbox" && sandbox_available == Some(false) {
        eprintln!("FutureOS: sandbox unavailable [SB001]; using manual approval");
        store::update_app_settings(store::UpdateAppSettingsInput {
            approval_tier: Some("manual".to_string()),
            ..Default::default()
        })?;
        let manual = crate::agent_proto::SandboxPolicy {
            tier: "manual".to_string(),
        };
        client
            .execute_command(set_sandbox_policy_command(manual, session_id.to_string()))
            .await
            .map_err(|error| format!("Unable to apply fallback sandbox policy: {error}"))?
            .into_inner()
            .ok_or_rpc_error("Future Agent rejected the fallback sandbox policy.")?;
    }
    Ok(())
}

pub(crate) fn workspace_path_for_thread(thread_id: &str) -> Result<String, crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Thread workspace could not be loaded.".to_string())?;
    Ok(workspace.path)
}

/// Fork a session at the given user message. Returns the new GUI thread id.
///
/// The Agent owns fork-point resolution and idempotent child creation. Desktop
/// projects that child through its unique session binding, so retrying any
/// post-commit failure reuses the same thread and run rows. Messages are served from the
/// agent JSONL (no SQLite `messages` table), so no message import is needed.
pub async fn fork_agent_session(
    thread_id: &str,
    source_entry_id: &str,
    request_id: &str,
) -> Result<String, crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let session_id = thread
        .agent_session_id
        .ok_or_else(|| "No agent session for this thread.".to_string())?;

    let mut client = super::client::connect_agent().await?;

    // ── call agent fork RPC ────────────────────────────────────────────

    if source_entry_id.trim().is_empty() {
        return Err("The selected message is not yet persisted.".into());
    }
    if request_id.trim().is_empty() {
        return Err("Fork request identity is missing.".into());
    }

    let fork_response = client
        .execute_command(fork_command(
            session_id.clone(),
            source_entry_id.to_string(),
            session_id.clone(),
            crate::device_identity::device_id_or_empty(),
            request_id.to_string(),
        ))
        .await
        .map_err(|error| format!("Unable to fork session: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the fork request.")?;

    let new_session_id = serde_json::from_str::<serde_json::Value>(&fork_response.data)
        .ok()
        .and_then(|v| v.get("sessionId").cloned())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();

    if new_session_id.is_empty() {
        return Err("Fork did not return a session.".into());
    }

    // ── read forked entries for metadata ───────────────────────────────

    let fork_entries: Vec<serde_json::Value> =
        super::fetch_all_session_entries_with_client(&mut client, &new_session_id)
            .await
            .map_err(|error| format!("Unable to list fork session entries: {error}"))?
            .into_iter()
            .filter_map(|entry| serde_json::to_value(entry).ok())
            .collect();

    // The agent's fork_session writes metadata into a session_info entry
    // (role = "system"); find it — get_session_entries now includes it.
    let session_info = fork_entries
        .iter()
        .find(|e| e.get("role").and_then(|r| r.as_str()) == Some("system"));
    let agent_session_name = session_info
        .and_then(|e| e.get("session"))
        .and_then(|c| c.get("sessionName"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "(fork)")
        .map(str::to_string);
    let session_name = agent_session_name.unwrap_or_else(|| {
        let parent_title = if thread.title.is_empty() {
            "Untitled"
        } else {
            &thread.title
        };
        format!("{parent_title} (fork)")
    });
    // Session-entry payloads intentionally contain conversation data only;
    // the selected model lives in the forked session's state. Read it from
    // the canonical typed response so a payload-only agent does not create
    // model-less historical runs after a fork.
    let session_model = match client
        .execute_command(get_state_command(new_session_id.clone()))
        .await
    {
        Ok(response) => future_rpc::decode::response_data(&response.into_inner())
            .get("model")
            .and_then(|model| model.as_str())
            .unwrap_or_default()
            .to_string(),
        Err(error) => {
            eprintln!("FutureOS: fork get_state failed: {error}");
            String::new()
        }
    };

    let entry_groups = group_session_entries(&fork_entries);

    // ── create workspace + thread ──────────────────────────────────────

    let (new_thread, _) =
        store::get_or_create_thread_for_agent_session(store::CreateThreadInput {
            mode: thread.mode.clone(),
            title: Some(session_name),
            workspace_id: if thread.mode == "chat" {
                None
            } else {
                Some(thread.workspace_id.clone())
            },
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(new_session_id.clone()),
        })?;

    store::sync_thread_parent_session(&new_session_id, &session_id)?;
    store::inherit_thread_asset_root(&new_thread.id, &thread.id)?;

    // Now that the thread (and its workspace) exist, set the forked
    // session's cwd to match so ensure_agent_session can find it
    // instead of creating a brand-new empty session.
    let cwd = workspace_path_for_thread(&new_thread.id)
        .expect("invariant: thread workspace exists immediately after create_thread");
    std::fs::create_dir_all(&cwd)?;
    if let Err(e) = client
        .execute_command(set_cwd_command(cwd, new_session_id.clone()))
        .await
    {
        eprintln!("FutureOS: fork set_cwd failed: {e}");
    }

    let (provider, model_id) = split_model(&session_model);
    let run_count = entry_groups.len();
    let mut run_ids: Vec<String> = Vec::with_capacity(run_count);
    for group in &entry_groups {
        let canonical_id = group
            .iter()
            .filter_map(|index| fork_entries.get(*index))
            .find_map(|entry| entry["runId"].as_str())
            .ok_or_else(|| "Fork history has no canonical run identity".to_string())?;
        let run = match store::get_run(canonical_id)? {
            Some(run) if run.thread_id == new_thread.id => run,
            Some(_) => {
                return Err(format!(
                    "Fork history run identity {canonical_id} belongs to another thread"
                )
                .into());
            }
            None => store::create_run(store::CreateRunInput {
                id: Some(canonical_id.to_owned()),
                thread_id: new_thread.id.clone(),
                trigger_message_id: None,
                model_provider: provider.clone(),
                model_id: model_id.clone(),
            })?,
        };
        let outcome = group
            .iter()
            .filter_map(|index| fork_entries.get(*index))
            .filter_map(|entry| entry.get("run"))
            .find(|run| {
                run.get("status")
                    .and_then(|status| status.as_str())
                    .is_some()
            });
        let outcome_status = outcome
            .and_then(|run| run.get("status"))
            .and_then(|status| status.as_str())
            .unwrap_or("completed");
        let inherited_error = outcome
            .and_then(|run| run.get("error"))
            .and_then(|error| error.as_str())
            .filter(|error| !error.trim().is_empty())
            .map(str::to_string);
        let (status, error_message, error_type) = match outcome_status {
            "failed" => (
                "failed",
                inherited_error.or_else(|| Some("Inherited run failed".to_string())),
                Some("model_failed".to_string()),
            ),
            "interrupted" => (
                "failed",
                inherited_error.or_else(|| Some("Inherited run was interrupted".to_string())),
                Some("unknown".to_string()),
            ),
            "cancelled" => ("cancelled", None, None),
            _ => ("completed", None, None),
        };
        let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: status.to_string(),
            error_message,
            error_type,
        });
        run_ids.push(run.id);
    }

    // Write synthetic run events so the right panel (Runs tab) shows tool calls
    // from the forked history immediately — no live stream exists for these runs.
    synthesize_run_events_from_entries(&fork_entries, &entry_groups, &run_ids);

    Ok(new_thread.id)
}

/// Write synthetic `tool_start` and `tool_end` run events from agent session
/// entries for runs that have no live event stream (forked and imported
/// sessions). The persistence pass extracts file artifacts and folds the
/// events only for local artifact extraction. Tool inspection queries the
/// Agent directly; this transient correlation cache is not its data source.
///
/// Entries are grouped by canonical `meta.run_id`, with a positional
/// user-to-next-user fallback for legacy journals. Tool result entries are
/// matched by `tool_call_id` inside their group.
pub(super) fn synthesize_run_events_from_entries(
    entries: &[serde_json::Value],
    groups: &[Vec<usize>],
    run_ids: &[String],
) {
    for (group, run_id) in groups.iter().zip(run_ids) {
        let tool_results: HashMap<&str, &serde_json::Value> = group
            .iter()
            .filter_map(|index| entries.get(*index))
            .flat_map(|entry| entry["blocks"].as_array().into_iter().flatten())
            .filter(|block| block["kind"] == "tool_result")
            .filter_map(|block| {
                let id = block["toolCallId"].as_str()?;
                (!id.is_empty()).then_some((id, block))
            })
            .collect();
        let mut seq: i64 = 0;

        for entry in group.iter().filter_map(|index| entries.get(*index)) {
            if entry.get("role").and_then(|role| role.as_str()) != Some("assistant") {
                continue;
            }
            let Some(tool_calls) = entry.get("blocks").and_then(|value| value.as_array()) else {
                continue;
            };

            for tc in tool_calls
                .iter()
                .filter(|block| block["kind"] == "tool_call")
            {
                let tc_id = tc.get("toolCallId").and_then(|v| v.as_str()).unwrap_or("");
                if tc_id.is_empty() {
                    continue;
                }
                let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = tc
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                let start_payload = serde_json::json!({
                    "tool_id": tc_id,
                    "tool_name": name,
                    "tool_args": args,
                });
                // Imported history is represented by the Agent transcript. Keep
                // only the GUI's derived tool projection; never recreate a GUI
                // raw-event JSONL from it.
                super::persist::persist_run_event(
                    Some(run_id),
                    "tool_start",
                    &start_payload.to_string(),
                    seq,
                );
                seq += 1;

                // tool_end from the matching result entry, if one exists.
                if let Some(result) = tool_results.get(tc_id) {
                    let content = result.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let is_error = result
                        .get("isError")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    let end_payload = if is_error {
                        serde_json::json!({
                            "tool_id": tc_id,
                            "text": content,
                            "error": content,
                        })
                    } else {
                        serde_json::json!({
                            "tool_id": tc_id,
                            "text": content,
                        })
                    };
                    super::persist::persist_run_event(
                        Some(run_id),
                        "tool_end",
                        &end_payload.to_string(),
                        seq,
                    );
                    seq += 1;
                }
            }
        }
    }
}

/// Group display entries into real agent runs. Canonical run IDs win; entries
/// from older journals are grouped into user-led exchanges.
pub(super) fn group_session_entries(entries: &[serde_json::Value]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut canonical: HashMap<String, usize> = HashMap::new();
    let mut current: Option<usize> = None;

    for (entry_index, entry) in entries.iter().enumerate() {
        let role = entry
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !matches!(role, "user" | "assistant" | "tool") {
            continue;
        }
        if let Some(run_id) = entry
            .get("runId")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        {
            let group_index = *canonical.entry(run_id.to_string()).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[group_index].push(entry_index);
            current = Some(group_index);
            continue;
        }

        let group_index = match (role, current) {
            ("user", _) | (_, None) => {
                groups.push(Vec::new());
                let index = groups.len() - 1;
                current = Some(index);
                index
            }
            (_, Some(index)) => index,
        };
        groups[group_index].push(entry_index);
    }
    groups
}

pub(super) fn split_model(model: &str) -> (Option<String>, Option<String>) {
    if model.is_empty() {
        return (None, None);
    }
    if let Some((provider, id)) = model.split_once('/') {
        (Some(provider.to_string()), Some(id.to_string()))
    } else {
        (None, Some(model.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{
        break_home, get_state_payload, mock_agent, restore_home, seed_run, seed_thread,
        seed_workspace, Reply, TestHome,
    };
    use super::*;

    async fn mock_client() -> (
        super::super::test_support::MockAgentGuard,
        super::super::client::AgentClient,
    ) {
        let mock = mock_agent();
        let client = super::super::client::connect_agent()
            .await
            .expect("connect to mock");
        (mock, client)
    }

    #[test]
    fn split_model_variants() {
        assert_eq!(split_model(""), (None, None));
        assert_eq!(
            split_model("future/k3"),
            (Some("future".to_string()), Some("k3".to_string()))
        );
        assert_eq!(split_model("k3"), (None, Some("k3".to_string())));
    }

    #[tokio::test]
    async fn ensure_reuses_a_matching_session() {
        let (mock, mut client) = mock_client().await;
        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-1", "cwd": "/tmp/ws"}),
        );
        let ensured = ensure_agent_session(&mut client, "sess-1", "/tmp/ws", None, None)
            .await
            .expect("ensured");
        assert_eq!(ensured.session_id, "sess-1");
        assert!(!ensured.created);
        assert!(mock.requests_of("new_session").is_empty());
    }

    #[tokio::test]
    async fn ensure_repairs_a_moved_session_without_losing_context() {
        let (mock, mut client) = mock_client().await;

        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-1", "cwd": "/elsewhere"}),
        );
        mock.push_data("set_cwd", serde_json::json!({"cwd": "/tmp/ws"}));
        let ensured = ensure_agent_session(
            &mut client,
            "sess-1",
            "/tmp/ws",
            Some("future/k3"),
            Some("high"),
        )
        .await
        .expect("ensured");
        assert_eq!(ensured.session_id, "sess-1");
        assert!(!ensured.created, "cwd repair preserves the session history");
        assert!(mock.requests_of("new_session").is_empty());
        let repaired = &mock.requests_of("set_cwd")[0];
        assert_eq!(repaired.session_id, "sess-1");
        assert_eq!(repaired.cwd, "/tmp/ws");
    }

    #[tokio::test]
    async fn ensure_preserves_the_binding_when_the_agent_lost_the_session() {
        let (mock, mut client) = mock_client().await;

        mock.push("get_state", Reply::Reject("no such session".to_string()));
        let error = ensure_agent_session(&mut client, "sess-1", "/tmp/ws", None, None)
            .await
            .expect_err("missing session must not be replaced");
        assert!(error.to_string().contains("binding was preserved"));
        assert!(mock.requests_of("new_session").is_empty());
    }

    #[tokio::test]
    async fn ensure_creates_for_an_empty_stored_id() {
        let (mock, mut client) = mock_client().await;
        mock.push_data(
            "new_session",
            serde_json::json!({"sessionId": "sess-fresh"}),
        );
        let ensured = ensure_agent_session(&mut client, "", "/tmp/ws", None, None)
            .await
            .expect("ensured");
        assert_eq!(ensured.session_id, "sess-fresh");
        assert!(ensured.created);
        assert!(
            mock.requests_of("get_state").is_empty(),
            "no probe for an empty stored id"
        );
    }

    #[tokio::test]
    async fn ensure_error_paths() {
        let (mock, mut client) = mock_client().await;

        // get_state transport failure.
        mock.push("get_state", Reply::Status(tonic::Code::Unavailable, "down"));
        let error = ensure_agent_session(&mut client, "sess-1", "/tmp/ws", None, None)
            .await
            .expect_err("transport");
        assert!(
            error
                .to_string()
                .contains("Unable to inspect Future Agent session"),
            "{error}"
        );

        // An application-level storage failure is not evidence that the
        // session is absent and must never create a replacement conversation.
        mock.push(
            "get_state",
            Reply::Reject("session storage unavailable".to_string()),
        );
        let error = ensure_agent_session(&mut client, "sess-1", "/tmp/ws", None, None)
            .await
            .expect_err("storage rejection");
        assert!(error.to_string().contains("session storage unavailable"));
        assert!(mock.requests_of("new_session").is_empty());

        // new_session transport failure.
        mock.push("new_session", Reply::Status(tonic::Code::Internal, "boom"));
        let error = ensure_agent_session(&mut client, "", "/tmp/ws", None, None)
            .await
            .expect_err("transport");
        assert!(
            error
                .to_string()
                .contains("Unable to create Future Agent session"),
            "{error}"
        );

        // new_session rejected at app level.
        mock.push("new_session", Reply::Reject("quota".to_string()));
        let error = ensure_agent_session(&mut client, "", "/tmp/ws", None, None)
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "quota");

        // An identity-less session can never be bound safely.
        mock.push_data("new_session", serde_json::json!({"ok": true}));
        let error = ensure_agent_session(&mut client, "", "/tmp/ws", None, None)
            .await
            .expect_err("missing identity");
        assert!(error.to_string().contains("without returning its identity"));
    }

    #[tokio::test]
    async fn permission_level_and_sandbox_policy_round_trip() {
        let home = TestHome::new("session-setup");
        let (mock, mut client) = mock_client().await;

        mock.push("set_permission_level", Reply::Data("{}".to_string()));
        set_agent_permission_level(&mut client, "sess-1", "workspace")
            .await
            .expect("permission level");
        let request = &mock.requests_of("set_permission_level")[0];
        assert_eq!(request.level, "workspace");

        // Default tier on a fresh store is "off".
        mock.push("set_sandbox_policy", Reply::Data("{}".to_string()));
        set_agent_sandbox_policy(&mut client, "sess-1", "thread-1")
            .await
            .expect("sandbox policy");
        let policy = mock.requests_of("set_sandbox_policy")[0]
            .sandbox_policy
            .clone()
            .expect("policy");
        assert_eq!(policy.tier, "off");

        // A configured tier is pushed verbatim.
        crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
            approval_tier: Some("sandbox".to_string()),
            hidden_models: None,
            auto_upgrade_skills: None,
            auto_connect_remote: None,
            skill_guide_dismissed: None,
            skill_intro_dismissed: None,
            bell_on_complete: None,
            auto_title_first_turn: None,
            title_language: None,
            community_edition: None,
        })
        .expect("update settings");
        mock.push("set_sandbox_policy", Reply::Data("{}".to_string()));
        set_agent_sandbox_policy(&mut client, "sess-1", "thread-1")
            .await
            .expect("sandbox policy");
        assert_eq!(
            mock.requests_of("set_sandbox_policy")[1]
                .sandbox_policy
                .clone()
                .expect("policy")
                .tier,
            "sandbox"
        );

        // If the Agent reports that the selected OS sandbox is unavailable,
        // persist the safe manual tier and apply it to the live session too.
        mock.push_data(
            "set_sandbox_policy",
            serde_json::json!({ "sandboxAvailable": false }),
        );
        mock.push_data(
            "set_sandbox_policy",
            serde_json::json!({ "sandboxAvailable": true }),
        );
        set_agent_sandbox_policy(&mut client, "sess-1", "thread-1")
            .await
            .expect("sandbox fallback");
        let policies = mock.requests_of("set_sandbox_policy");
        assert_eq!(
            policies[2].sandbox_policy.as_ref().expect("policy").tier,
            "sandbox"
        );
        assert_eq!(
            policies[3].sandbox_policy.as_ref().expect("policy").tier,
            "manual"
        );
        assert_eq!(
            crate::store::get_app_settings()
                .expect("settings after fallback")
                .approval_tier,
            "manual"
        );

        // Store unreadable → falls back to explicit manual approval.
        let prev = break_home();
        mock.push("set_sandbox_policy", Reply::Data("{}".to_string()));
        set_agent_sandbox_policy(&mut client, "sess-1", "thread-1")
            .await
            .expect("sandbox policy");
        restore_home(prev);
        assert_eq!(
            mock.requests_of("set_sandbox_policy")[4]
                .sandbox_policy
                .clone()
                .expect("policy")
                .tier,
            "manual"
        );

        // Error paths.
        mock.push(
            "set_permission_level",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = set_agent_permission_level(&mut client, "sess-1", "workspace")
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("permission level"), "{error}");
        mock.push(
            "set_permission_level",
            Reply::Reject("bad level".to_string()),
        );
        let error = set_agent_permission_level(&mut client, "sess-1", "workspace")
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad level");

        mock.push(
            "set_sandbox_policy",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = set_agent_sandbox_policy(&mut client, "sess-1", "thread-1")
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("sandbox policy"), "{error}");
        mock.push("set_sandbox_policy", Reply::Reject("bad tier".to_string()));
        let error = set_agent_sandbox_policy(&mut client, "sess-1", "thread-1")
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad tier");
        drop(home);
    }

    #[tokio::test]
    async fn workspace_path_for_thread_resolves_and_errors() {
        let home = TestHome::new("session-ws-path");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        assert_eq!(
            workspace_path_for_thread(&thread.id).expect("path"),
            workspace.path
        );

        let error = workspace_path_for_thread("no-such-thread").expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");

        // Workspace row gone (raw delete, FK off) → error.
        let conn =
            rusqlite::Connection::open(home.path().join(".future/app/app.db")).expect("open db");
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("fk off");
        conn.execute("DELETE FROM workspaces WHERE id = ?1", [&workspace.id])
            .expect("delete workspace row");
        drop(conn);
        let error = workspace_path_for_thread(&thread.id).expect_err("missing workspace");
        assert_eq!(error.to_string(), "Thread workspace could not be loaded.");
    }

    // ── fork_agent_session ──────────────────────────────────────────────

    fn entries_payload(entries: serde_json::Value) -> serde_json::Value {
        let mut entries = entries.as_array().cloned().unwrap_or_default();
        let mut current_run = "history-test".to_owned();
        for entry in &mut entries {
            if entry["role"] == "user" {
                current_run = format!("history-{}", entry["id"].as_str().unwrap());
            }
            let object = entry.as_object_mut().unwrap();
            object.entry("kind").or_insert(serde_json::Value::Null);
            if object["kind"].is_null() {
                object.insert("kind".into(), object["role"].clone());
            }
            object
                .entry("createdAtMs")
                .or_insert(serde_json::json!(1000));
            object.entry("blocks").or_insert(serde_json::json!([]));
            object
                .entry("runId")
                .or_insert(serde_json::json!(current_run));
        }
        serde_json::json!({"entries": entries})
    }

    fn forked_entries() -> serde_json::Value {
        serde_json::json!([
            {"id":"f0","role":"system","kind":"session_info","session":{"sessionName":"Forked Chat","model":"future/k3"},"blocks":[]},
            {"id":"f1","role":"user","runId":"fork-run-1","blocks":[{"kind":"text","text":"first question"}]},
            {"id":"f2","role":"assistant","runId":"fork-run-1","run":{"status":"completed"},"blocks":[{"kind":"text","text":"first answer"},{"kind":"tool_call","toolCallId":"tc-1","name":"shell","arguments":{"command":"ls"}}]},
            {"id":"f3","role":"tool","runId":"fork-run-1","blocks":[{"kind":"tool_result","toolCallId":"tc-1","text":"file.txt","isError":false}]}
        ])
    }

    #[tokio::test]
    async fn fork_by_source_identity_creates_thread_runs_and_events() {
        let home = TestHome::new("session-fork");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork"}));
        mock.push_typed_data("get_session_entries", entries_payload(forked_entries()));
        let mut fork_state = get_state_payload("sess-fork", false);
        fork_state["model"] = serde_json::json!("future/k3");
        mock.push_state_for_session("sess-fork", Reply::TypedData(fork_state));
        mock.push("set_cwd", Reply::Data("{}".to_string()));

        let new_thread_id = fork_agent_session(&thread.id, "e3", "request-1")
            .await
            .expect("fork");

        // Desktop sends the persisted user identity; the Agent owns resolving
        // the complete settled turn boundary.
        let fork_request = &mock.requests_of("fork")[0];
        assert_eq!(fork_request.entry_id, "e3");
        assert_eq!(fork_request.mode, "through_turn");
        assert_eq!(fork_request.client_request_id, "request-1");
        assert_eq!(fork_request.session_id, "sess-1");
        assert_eq!(fork_request.parent_session, "sess-1");

        let new_thread = crate::store::get_thread(&new_thread_id)
            .expect("thread")
            .expect("exists");
        assert_eq!(new_thread.title, "Forked Chat");
        assert_eq!(new_thread.agent_session_id.as_deref(), Some("sess-fork"));
        assert_eq!(new_thread.parent_session_id.as_deref(), Some("sess-1"));

        // One assistant reply in the forked history → one completed run with
        // the session model split into provider/id.
        let runs = crate::store::list_runs(&new_thread_id).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "completed");
        assert_eq!(runs[0].model_provider.as_deref(), Some("future"));
        assert_eq!(runs[0].model_id.as_deref(), Some("k3"));

        // Synthetic tool events were projected for the run panel.
        let input = crate::store::get_tool_call_input(&runs[0].id, "tc-1").expect("input");
        assert_eq!(input.as_deref(), Some(r#"{"command":"ls"}"#));

        // The forked session's cwd was aligned with the new thread workspace.
        assert_eq!(mock.requests_of("set_cwd").len(), 1);
        assert_eq!(mock.requests_of("set_cwd")[0].session_id, "sess-fork");
    }

    #[tokio::test]
    async fn fork_retry_reuses_the_same_thread_and_historical_run_projection() {
        let home = TestHome::new("session-fork-idempotent");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        for _ in 0..2 {
            mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork"}));
            mock.push_typed_data("get_session_entries", entries_payload(forked_entries()));
            let mut state = get_state_payload("sess-fork", false);
            state["model"] = serde_json::json!("future/k3");
            mock.push_state_for_session("sess-fork", Reply::TypedData(state));
            mock.push("set_cwd", Reply::Data("{}".to_string()));
        }

        let first = fork_agent_session(&thread.id, "e1", "stable-request")
            .await
            .expect("first fork projection");
        let second = fork_agent_session(&thread.id, "e1", "stable-request")
            .await
            .expect("retry projection");

        assert_eq!(first, second);
        assert_eq!(crate::store::list_runs(&first).unwrap().len(), 1);
        assert_eq!(
            crate::store::list_threads()
                .unwrap()
                .iter()
                .filter(|candidate| candidate.agent_session_id.as_deref() == Some("sess-fork"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn fork_without_session_metadata_uses_parent_title() {
        let home = TestHome::new("session-fork-content");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        // The persisted entry id is forwarded unchanged; Desktop does not
        // reinterpret the fork point from rendered content.
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-2"}));
        // No session_info entry → title defaults to "<parent> (fork)", no model.
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([
                {"id": "f1", "role": "user", "runId": "fork-run-only", "blocks": [{"kind":"text","text":"first question"}]}
            ])),
        );
        mock.push("set_cwd", Reply::Data("{}".to_string()));

        let new_thread_id = fork_agent_session(&thread.id, "e3", "request-2")
            .await
            .expect("fork");
        assert_eq!(mock.requests_of("fork")[0].entry_id, "e3");
        let new_thread = crate::store::get_thread(&new_thread_id)
            .expect("thread")
            .expect("exists");
        assert_eq!(new_thread.title, "test thread (fork)");
        let runs = crate::store::list_runs(&new_thread_id).expect("runs");
        assert_eq!(runs.len(), 1, "no assistant replies → one placeholder run");
        assert_eq!(runs[0].model_provider, None);
    }

    #[tokio::test]
    async fn fork_last_user_message_ends_at_the_tail() {
        let home = TestHome::new("session-fork-tail");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-3"}));
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([])),
        );
        mock.push("set_cwd", Reply::Data("{}".to_string()));

        fork_agent_session(&thread.id, "e5", "request-3")
            .await
            .expect("fork");
        // The last user message is the tail entry itself.
        assert_eq!(mock.requests_of("fork")[0].entry_id, "e5");
    }

    #[tokio::test]
    async fn fork_error_paths() {
        let home = TestHome::new("session-fork-errors");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        // Unknown thread.
        let error = fork_agent_session("no-such-thread", "x", "request")
            .await
            .expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");

        // Thread without an agent session.
        let no_session = seed_thread(&workspace.id, None);
        let error = fork_agent_session(&no_session.id, "x", "request")
            .await
            .expect_err("no session");
        assert_eq!(error.to_string(), "No agent session for this thread.");

        let error = fork_agent_session(&thread.id, "", "request")
            .await
            .expect_err("missing persisted source identity");
        assert_eq!(
            error.to_string(),
            "The selected message is not yet persisted."
        );
        let error = fork_agent_session(&thread.id, "entry", "")
            .await
            .expect_err("missing request identity");
        assert_eq!(error.to_string(), "Fork request identity is missing.");

        // Fork transport failure / rejection / missing sessionId.
        mock.push("fork", Reply::Status(tonic::Code::Internal, "boom"));
        let error = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect_err("fork transport");
        assert!(
            error.to_string().contains("Unable to fork session"),
            "{error}"
        );

        mock.push("fork", Reply::Reject("cannot fork".to_string()));
        let error = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect_err("fork reject");
        assert_eq!(error.to_string(), "cannot fork");

        mock.push_data("fork", serde_json::json!({"ok": true}));
        let error = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect_err("no session id");
        assert_eq!(error.to_string(), "Fork did not return a session.");

        // Forked-entries transport failure / rejection.
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-e"}));
        mock.push(
            "get_session_entries",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect_err("fork entries transport");
        assert!(
            error
                .to_string()
                .contains("Unable to list fork session entries"),
            "{error}"
        );

        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-e"}));
        mock.push("get_session_entries", Reply::Reject("bad".to_string()));
        let error = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect_err("fork entries reject");
        assert_eq!(
            error.to_string(),
            "Unable to list fork session entries: bad"
        );
    }

    #[tokio::test]
    async fn fork_uses_agent_session_name_unless_placeholder() {
        let home = TestHome::new("session-fork-name");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-4"}));
        // session_name "(fork)" is a placeholder → default title.
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([
                {"id": "f0", "role": "system", "kind":"session_info", "session": {"sessionName": "(fork)"}}
            ])),
        );
        mock.push(
            "set_cwd",
            Reply::Status(tonic::Code::Internal, "best effort"),
        );
        let new_thread_id = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect("fork");
        let new_thread = crate::store::get_thread(&new_thread_id)
            .expect("thread")
            .expect("exists");
        assert_eq!(new_thread.title, "test thread (fork)");
    }

    #[tokio::test]
    async fn fork_chat_thread_with_empty_title_defaults_to_untitled() {
        let _home = TestHome::new("session-fork-chat");
        let mock = mock_agent();
        // Chat-mode thread (no workspace) with an empty title: the fork title
        // falls back to "Untitled (fork)" and the new thread keeps chat mode
        // with no workspace id.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some(String::new()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-1".to_string()),
        })
        .expect("create chat thread");

        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-chat"}));
        // No session_info entry → title defaults, no model.
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([])),
        );

        let new_thread_id = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect("fork");
        let new_thread = crate::store::get_thread(&new_thread_id)
            .expect("thread")
            .expect("exists");
        assert_eq!(new_thread.title, "Untitled (fork)");
        assert_eq!(new_thread.mode, "chat");
        assert!(!new_thread.workspace_id.is_empty());
    }

    /// A workspace-mode fork whose workspace row has vanished under the
    /// thread (raw delete, FK off) fails at `create_thread` — the `?` error
    /// arm must surface rather than silently creating an orphan thread.
    #[tokio::test]
    async fn fork_create_thread_fails_when_the_workspace_row_is_gone() {
        let home = TestHome::new("session-fork-create-fail");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        // Delete the workspace row directly (FK off) while leaving the thread
        // row behind: `get_thread` still resolves the parent, but the forked
        // `create_thread` can no longer load the workspace.
        let conn =
            rusqlite::Connection::open(home.path().join(".future/app/app.db")).expect("open db");
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("fk off");
        conn.execute("DELETE FROM workspaces WHERE id = ?1", [&workspace.id])
            .expect("delete workspace row");
        drop(conn);

        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-cf"}));
        mock.push_data("get_session_entries", entries_payload(forked_entries()));

        let error = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect_err("create thread");
        assert!(error.to_string().contains("Workspace"), "{error}");
    }

    /// When the forked thread's workspace path can no longer be created on
    /// disk (the directory was replaced by a file), the cwd write-back's
    /// `create_dir_all` error propagates through `?` rather than being
    /// swallowed.
    #[tokio::test]
    async fn fork_propagates_a_create_dir_failure() {
        let home = TestHome::new("session-fork-mkdir-fail");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        // Replace the workspace directory with a plain file: the row still
        // resolves, but `create_dir_all` on that path must fail.
        std::fs::remove_dir_all(&workspace.path).expect("rm workspace dir");
        std::fs::write(&workspace.path, "not a directory").expect("write file");

        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-md"}));
        mock.push_data("get_session_entries", entries_payload(forked_entries()));

        let error = fork_agent_session(&thread.id, "x", "request")
            .await
            .expect_err("create_dir_all");
        assert!(!error.to_string().is_empty());
    }

    // ── synthesize_run_events_from_entries ──────────────────────────────

    #[test]
    fn synthesize_run_events_maps_assistants_to_runs() {
        let home = TestHome::new("session-synthesize");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run_a = seed_run(&thread.id);
        let run_b = seed_run(&thread.id);

        let entries = serde_json::json!([
            {"role":"assistant","blocks":[
                {"kind":"tool_call","toolCallId":"tc-1","name":"shell","arguments":{"command":"ls"}},
                {"kind":"tool_call","toolCallId":"","name":"write","arguments":{}},
                {"kind":"tool_call","toolCallId":"tc-2"},
                {"kind":"tool_call","toolCallId":"tc-err","name":"read","arguments":{}}
            ]},
            {"role":"tool","blocks":[{"kind":"tool_result","toolCallId":"tc-1","text":"ok","isError":false}]},
            {"role":"tool","blocks":[{"kind":"tool_result","toolCallId":"tc-err","text":"Error: missing file","isError":true}]},
            {"role":"tool","blocks":[{"kind":"tool_result","toolCallId":"","text":"no id"}]},
            {"role":"assistant","blocks":[{"kind":"text","text":"no tool calls"}]},
            {"role":"assistant","blocks":[]},
            {"role":"user","blocks":[{"kind":"text","text":"not an assistant"}]}
        ]);
        synthesize_run_events_from_entries(
            &entries.as_array().expect("array").to_vec(),
            &[vec![0, 1, 2, 3], vec![4, 5, 6]],
            &[run_a.id.clone(), run_b.id.clone()],
        );

        // tool_start args are queryable through the projection.
        assert_eq!(
            crate::store::get_tool_call_input(&run_a.id, "tc-1")
                .expect("input")
                .as_deref(),
            Some(r#"{"command":"ls"}"#)
        );
        // The second assistant maps to the second run; further assistants
        // (third) are dropped once run_ids are exhausted.
        assert!(
            crate::store::get_tool_call_input(&run_b.id, "tc-1")
                .expect("input")
                .is_none(),
            "each assistant binds its own run"
        );
    }

    #[test]
    fn groups_multiple_assistant_entries_into_their_canonical_run() {
        let entries = serde_json::json!([
            {"role": "system", "content": {}},
            {"role": "user", "runId": "r1"},
            {"role": "assistant", "runId": "r1"},
            {"role": "assistant", "runId": "r1"},
            {"role": "tool", "runId": "r1"},
            {"role": "user", "runId": "r2"},
            {"role": "assistant", "runId": "r2"}
        ]);
        assert_eq!(
            group_session_entries(entries.as_array().expect("entries")),
            vec![vec![1, 2, 3, 4], vec![5, 6]]
        );
    }

    #[test]
    fn groups_legacy_entries_by_user_exchange() {
        let entries = serde_json::json!([
            {"role": "user"},
            {"role": "assistant"},
            {"role": "assistant"},
            {"role": "tool"},
            {"role": "user"},
            {"role": "assistant"}
        ]);
        assert_eq!(
            group_session_entries(entries.as_array().expect("entries")),
            vec![vec![0, 1, 2, 3], vec![4, 5]]
        );
    }
}
