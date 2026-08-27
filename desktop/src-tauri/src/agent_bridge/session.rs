//! Agent session lifecycle: ensure/create a session for a thread, set its
//! permission level, and resolve a thread's workspace path and prior-message
//! count. These back the per-prompt setup in the parent module.

use std::collections::HashMap;

use tonic::transport::Channel;

use super::client::{
    fork_command, get_session_entries_command, get_state_command, new_session_command,
    set_cwd_command, set_permission_level_command, set_sandbox_policy_command, RpcResponseExt,
};
use crate::{agent_proto::FutureAgentClient, store};

/// Outcome of `ensure_agent_session`.
#[derive(Debug)]
pub(super) struct EnsuredSession {
    pub session_id: String,
    /// True when the thread ALREADY had a session id but it was unusable
    /// (agent lost the session data, or its cwd no longer matches the
    /// thread's workspace), so a fresh empty session silently replaced it.
    /// The agent-side context is gone even though the GUI still shows the
    /// history — callers must surface this instead of rebinding quietly.
    pub recreated: bool,
}

/// Ensure an agent session exists for the given thread. Returns the session
/// id (the existing one, or the newly-created one if the agent generated it).
/// `model_id` and `thinking_level` are applied to newly-created sessions so
/// the agent starts with the user's selection immediately.
pub(super) async fn ensure_agent_session(
    client: &mut FutureAgentClient<Channel>,
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
            if active_id == session_id && active_cwd == cwd {
                return Ok(EnsuredSession {
                    session_id: session_id.to_string(),
                    recreated: false,
                });
            }
        }
    }

    // Create a new session. Pass empty session_id to let the agent generate it.
    let resp = client
        .execute_command(new_session_command(
            String::new(),
            cwd.to_string(),
            "desktop",
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

    Ok(EnsuredSession {
        session_id: new_id,
        // A non-empty incoming session id means this new session REPLACES one
        // the agent no longer has — the previous context is lost.
        recreated: !session_id.is_empty(),
    })
}

pub(super) async fn set_agent_permission_level(
    client: &mut FutureAgentClient<Channel>,
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
/// directly — only the tier travels over the wire (APPROVAL_PLAN.md):
/// `"manual"` (ask), `"sandbox"` (the available OS sandbox wraps shell commands), or `"off"`
/// (fully open). The tier is a global app preference, defaulting to `"manual"`.
pub(super) async fn set_agent_sandbox_policy(
    client: &mut FutureAgentClient<Channel>,
    session_id: &str,
    _thread_id: &str,
) -> Result<(), crate::AppError> {
    let tier = store::get_app_settings()
        .map(|settings| settings.approval_tier)
        .unwrap_or_else(|_| "off".to_string());
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
/// Creates a dedicated chat workspace named after the forked session id, copies
/// thread metadata from the parent, and creates per-reply completed run records
/// so the right panel is populated immediately.  Messages are served from the
/// agent JSONL (no SQLite `messages` table), so no message import is needed.
pub async fn fork_agent_session(
    thread_id: &str,
    user_message_content: &str,
    // 0-based ordinal of the user message among all user messages. The GUI
    // renders exactly one message per user entry in order, so the Nth user
    // message maps to the Nth user entry — matching by ordinal instead of
    // content means two identical prompts ("continue", "run the tests") fork the
    // intended run, not the first occurrence. `< 0` (unknown) falls back to
    // content matching.
    user_message_index: i64,
) -> Result<String, crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let session_id = thread
        .agent_session_id
        .ok_or_else(|| "No agent session for this thread.".to_string())?;

    let mut client = super::client::connect_agent().await?;

    // ── find the fork point ────────────────────────────────────────────

    let response = client
        .execute_command(get_session_entries_command(session_id.clone()))
        .await
        .map_err(|error| format!("Unable to list session entries: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session-entries request.")?;

    let entries: Vec<serde_json::Value> = future_rpc::decode::response_data(&response)
        .get("entries")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let is_user = |e: &serde_json::Value| e.get("role").and_then(|r| r.as_str()) == Some("user");

    // Prefer the user-message ordinal; fall back to content when it's unknown
    // (< 0) or out of range.
    let match_idx = usize::try_from(user_message_index)
        .ok()
        .and_then(|nth| {
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| is_user(e))
                .nth(nth)
                .map(|(i, _)| i)
        })
        .or_else(|| {
            entries.iter().position(|e| {
                is_user(e)
                    && e.get("content")
                        .and_then(|c| c.as_str())
                        .is_some_and(|c| c.trim() == user_message_content.trim())
            })
        })
        .ok_or_else(|| "No matching user message found in agent session.".to_string())?;

    let mut fork_idx = match_idx;
    for (i, entry) in entries.iter().enumerate().skip(match_idx + 1) {
        let role = entry.get("role").and_then(|r| r.as_str()).unwrap_or("");
        fork_idx = i;
        if role == "user" {
            fork_idx = i - 1;
            break;
        }
    }
    let entry_id = entries[fork_idx]
        .get("id")
        .and_then(|id| id.as_str())
        .ok_or_else(|| "No fork entry found.".to_string())?;

    // ── call agent fork RPC ────────────────────────────────────────────

    let fork_response = client
        .execute_command(fork_command(
            session_id.clone(),
            entry_id.to_string(),
            session_id.clone(),
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

    let entries_response = client
        .execute_command(get_session_entries_command(new_session_id.clone()))
        .await
        .map_err(|error| format!("Unable to list fork session entries: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the fork-session entries request.")?;

    let fork_entries: Vec<serde_json::Value> =
        serde_json::from_str::<serde_json::Value>(&entries_response.data)
            .ok()
            .and_then(|v| v.get("entries").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

    // The agent's fork_session writes metadata into a session_info entry
    // (role = "system"); find it — get_session_entries now includes it.
    let session_info = fork_entries
        .iter()
        .find(|e| e.get("role").and_then(|r| r.as_str()) == Some("system"));
    let agent_session_name = session_info
        .and_then(|e| e.get("content"))
        .and_then(|c| c.get("session_name"))
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
    let session_model = session_info
        .and_then(|e| e.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    let assistant_count = fork_entries
        .iter()
        .filter(|e| e.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .count();

    // ── create workspace + thread ──────────────────────────────────────

    let new_thread = store::create_thread(store::CreateThreadInput {
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
    let run_count = assistant_count.max(1);
    let mut run_ids: Vec<String> = Vec::with_capacity(run_count);
    for _ in 0..run_count {
        let run = store::create_run(store::CreateRunInput {
            id: None,
            thread_id: new_thread.id.clone(),
            trigger_message_id: None,
            model_provider: provider.clone(),
            model_id: model_id.clone(),
        })?;
        let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        });
        run_ids.push(run.id);
    }

    // Write synthetic run events so the right panel (Runs tab) shows tool calls
    // from the forked history immediately — no live stream exists for these runs.
    synthesize_run_events_from_entries(&fork_entries, &run_ids);

    Ok(new_thread.id)
}

/// Write synthetic `tool_start` and `tool_end` run events from agent session
/// entries for runs that have no live event stream (forked and imported
/// sessions). The persistence pass extracts file artifacts and folds the
/// events into the Runs-panel tool projection (the Agent journal has no
/// forked history to serve it). Panel state is in-memory, so it lasts for
/// this process lifetime.
///
/// The Nth assistant entry maps to the Nth run_id. Tool result entries
/// (role = "tool") are matched by `tool_call_id`.
pub(super) fn synthesize_run_events_from_entries(
    entries: &[serde_json::Value],
    run_ids: &[String],
) {
    // Index tool result entries by tool_call_id.
    let tool_results: HashMap<&str, &serde_json::Value> = entries
        .iter()
        .filter(|e| e.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .filter_map(|e| {
            let id = e.get("tool_call_id").and_then(|v| v.as_str())?;
            if id.is_empty() {
                None
            } else {
                Some((id, e))
            }
        })
        .collect();

    let mut run_idx: usize = 0;
    let mut seq: i64 = 0;

    for entry in entries {
        if entry.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        if run_idx >= run_ids.len() {
            break;
        }
        let run_id = &run_ids[run_idx];
        run_idx += 1;

        let Some(tool_calls) = entry.get("tool_calls").and_then(|v| v.as_array()) else {
            continue;
        };

        for tc in tool_calls {
            let tc_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if tc_id.is_empty() {
                continue;
            }
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

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
                let content = result.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let is_error = content.starts_with("Error:");
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
        break_home, mock_agent, restore_home, seed_run, seed_thread, seed_workspace, Reply,
        TestHome,
    };
    use super::*;

    async fn mock_client() -> (
        super::super::test_support::MockAgentGuard,
        FutureAgentClient<Channel>,
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
        assert!(!ensured.recreated);
        assert!(mock.requests_of("new_session").is_empty());
    }

    #[tokio::test]
    async fn ensure_recreates_when_the_agent_lost_or_moved_the_session() {
        let (mock, mut client) = mock_client().await;

        // cwd drift → recreate.
        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-1", "cwd": "/elsewhere"}),
        );
        mock.push_data("new_session", serde_json::json!({"sessionId": "sess-new"}));
        let ensured = ensure_agent_session(
            &mut client,
            "sess-1",
            "/tmp/ws",
            Some("future/k3"),
            Some("high"),
        )
        .await
        .expect("ensured");
        assert_eq!(ensured.session_id, "sess-new");
        assert!(ensured.recreated, "a replaced session reports context loss");
        let created = &mock.requests_of("new_session")[0];
        assert_eq!(created.session_id, "", "the agent generates the id");
        assert_eq!(created.cwd, "/tmp/ws");
        assert_eq!(created.created_by, "desktop");
        assert_eq!(created.model_id, "future/k3");
        assert_eq!(created.level, "high");

        // get_state rejected (session gone) → recreate too.
        mock.push("get_state", Reply::Reject("no such session".to_string()));
        mock.push_data(
            "new_session",
            serde_json::json!({"sessionId": "sess-newer"}),
        );
        let ensured = ensure_agent_session(&mut client, "sess-1", "/tmp/ws", None, None)
            .await
            .expect("ensured");
        assert_eq!(ensured.session_id, "sess-newer");
        assert!(ensured.recreated);
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
        assert!(!ensured.recreated, "nothing was replaced");
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

        // new_session success without a sessionId → empty id.
        mock.push_data("new_session", serde_json::json!({"ok": true}));
        let ensured = ensure_agent_session(&mut client, "", "/tmp/ws", None, None)
            .await
            .expect("ensured");
        assert_eq!(ensured.session_id, "");
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
            show_thinking: None,
            auto_upgrade_skills: None,
            auto_connect_remote: None,
            skill_guide_dismissed: None,
            skill_intro_dismissed: None,
            bell_on_complete: None,
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

        // Store unreadable → falls back to "off".
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
            "off"
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
        serde_json::json!({"entries": entries})
    }

    fn conversation_entries() -> serde_json::Value {
        serde_json::json!([
            {"id": "e1", "role": "user", "content": "first question"},
            {"id": "e2", "role": "assistant", "content": "first answer"},
            {"id": "e3", "role": "user", "content": "second question"},
            {"id": "e4", "role": "assistant", "content": "second answer"},
            {"id": "e5", "role": "user", "content": "third question"}
        ])
    }

    fn forked_entries() -> serde_json::Value {
        serde_json::json!([
            {"id": "f0", "role": "system", "content": {"session_name": "Forked Chat"}, "model": "future/k3"},
            {"id": "f1", "role": "user", "content": "first question"},
            {"id": "f2", "role": "assistant", "content": "first answer", "tool_calls": [
                {"id": "tc-1", "function": {"name": "shell", "arguments": "{\"command\":\"ls\"}"}}
            ]},
            {"id": "f3", "role": "tool", "tool_call_id": "tc-1", "content": "file.txt"}
        ])
    }

    #[tokio::test]
    async fn fork_by_ordinal_creates_thread_runs_and_events() {
        let home = TestHome::new("session-fork");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork"}));
        mock.push_data("get_session_entries", entries_payload(forked_entries()));
        mock.push("set_cwd", Reply::Data("{}".to_string()));

        let new_thread_id = fork_agent_session(&thread.id, "ignored", 1)
            .await
            .expect("fork");

        // Fork point: the second user message (ordinal 1) → entry e3; the
        // following user message e5 bounds the fork at e4.
        let fork_request = &mock.requests_of("fork")[0];
        assert_eq!(fork_request.entry_id, "e4");
        assert_eq!(fork_request.session_id, "sess-1");
        assert_eq!(fork_request.parent_session, "sess-1");

        let new_thread = crate::store::get_thread(&new_thread_id)
            .expect("thread")
            .expect("exists");
        assert_eq!(new_thread.title, "Forked Chat");
        assert_eq!(new_thread.agent_session_id.as_deref(), Some("sess-fork"));

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
    async fn fork_falls_back_to_content_matching_and_title_defaults() {
        let home = TestHome::new("session-fork-content");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        // Unknown ordinal (-1) → content match on "second question".
        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-2"}));
        // No session_info entry → title defaults to "<parent> (fork)", no model.
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([
                {"id": "f1", "role": "user", "content": "first question"}
            ])),
        );
        mock.push("set_cwd", Reply::Data("{}".to_string()));

        let new_thread_id = fork_agent_session(&thread.id, " second question ", -1)
            .await
            .expect("fork");
        assert_eq!(mock.requests_of("fork")[0].entry_id, "e4");
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

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-3"}));
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([])),
        );
        mock.push("set_cwd", Reply::Data("{}".to_string()));

        fork_agent_session(&thread.id, "ignored", 2)
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
        let error = fork_agent_session("no-such-thread", "x", 0)
            .await
            .expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");

        // Thread without an agent session.
        let no_session = seed_thread(&workspace.id, None);
        let error = fork_agent_session(&no_session.id, "x", 0)
            .await
            .expect_err("no session");
        assert_eq!(error.to_string(), "No agent session for this thread.");

        // Entries transport failure / rejection.
        mock.push(
            "get_session_entries",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("entries transport");
        assert!(
            error.to_string().contains("Unable to list session entries"),
            "{error}"
        );
        mock.push("get_session_entries", Reply::Reject("bad".to_string()));
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("entries reject");
        assert_eq!(error.to_string(), "bad");

        // No matching user message.
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([])),
        );
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("no match");
        assert_eq!(
            error.to_string(),
            "No matching user message found in agent session."
        );

        // Fork transport failure / rejection / missing sessionId.
        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push("fork", Reply::Status(tonic::Code::Internal, "boom"));
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("fork transport");
        assert!(
            error.to_string().contains("Unable to fork session"),
            "{error}"
        );

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push("fork", Reply::Reject("cannot fork".to_string()));
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("fork reject");
        assert_eq!(error.to_string(), "cannot fork");

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"ok": true}));
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("no session id");
        assert_eq!(error.to_string(), "Fork did not return a session.");

        // Forked-entries transport failure / rejection.
        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-e"}));
        mock.push(
            "get_session_entries",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("fork entries transport");
        assert!(
            error
                .to_string()
                .contains("Unable to list fork session entries"),
            "{error}"
        );

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-e"}));
        mock.push("get_session_entries", Reply::Reject("bad".to_string()));
        let error = fork_agent_session(&thread.id, "x", 0)
            .await
            .expect_err("fork entries reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn fork_uses_agent_session_name_unless_placeholder() {
        let home = TestHome::new("session-fork-name");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-4"}));
        // session_name "(fork)" is a placeholder → default title.
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([
                {"id": "f0", "role": "system", "content": {"session_name": "(fork)"}}
            ])),
        );
        mock.push(
            "set_cwd",
            Reply::Status(tonic::Code::Internal, "best effort"),
        );
        let new_thread_id = fork_agent_session(&thread.id, "x", 0).await.expect("fork");
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

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-chat"}));
        // No session_info entry → title defaults, no model.
        mock.push_data(
            "get_session_entries",
            entries_payload(serde_json::json!([])),
        );

        let new_thread_id = fork_agent_session(&thread.id, "x", 0).await.expect("fork");
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

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-cf"}));
        mock.push_data("get_session_entries", entries_payload(forked_entries()));

        let error = fork_agent_session(&thread.id, "x", 0)
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

        mock.push_data(
            "get_session_entries",
            entries_payload(conversation_entries()),
        );
        mock.push_data("fork", serde_json::json!({"sessionId": "sess-fork-md"}));
        mock.push_data("get_session_entries", entries_payload(forked_entries()));

        let error = fork_agent_session(&thread.id, "x", 0)
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
            {"role": "assistant", "tool_calls": [
                {"id": "tc-1", "function": {"name": "shell", "arguments": "{\"command\":\"ls\"}"}},
                {"id": "", "function": {"name": "write", "arguments": "{}"}},
                {"id": "tc-2"},
                {"id": "tc-err", "function": {"name": "read", "arguments": "{}"}}
            ]},
            {"role": "tool", "tool_call_id": "tc-1", "content": "ok"},
            {"role": "tool", "tool_call_id": "tc-err", "content": "Error: missing file"},
            {"role": "tool", "tool_call_id": "", "content": "no id"},
            {"role": "assistant", "content": "no tool calls"},
            {"role": "assistant", "tool_calls": []},
            {"role": "user", "content": "not an assistant"}
        ]);
        synthesize_run_events_from_entries(
            &entries.as_array().expect("array").to_vec(),
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
}
