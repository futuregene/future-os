//! Import agent sessions into the GUI on startup. Discovers sessions that exist
//! on the agent but not in the local SQLite DB, then creates workspace + thread
//! records + per-turn run records so they appear in the thread list and right
//! panel immediately.

use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use super::client::{
    connect_agent, get_session_entries_command, list_sessions_command, set_session_name_command,
};
use crate::store;

// ─── agent RPC types ────────────────────────────────────────────────────────

/// Lightweight session summary from the agent's `list_sessions` RPC.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionSummary {
    pub id: String,
    #[serde(default, rename = "session_name")]
    pub name: Option<String>,
    // Tolerate a missing/null cwd (e.g. channel sessions) — an empty cwd is
    // routed to a chat workspace by `thread_mode`, not dropped.
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub model: String,
    #[serde(default, rename = "first_message")]
    pub first_message: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub parent_session_id: String,
    /// Whether the agent is currently streaming a response for this session.
    #[serde(default, rename = "is_streaming")]
    #[allow(dead_code)]
    pub is_streaming: bool,
}

// ─── fetch helpers ──────────────────────────────────────────────────────────

/// Fetch all sessions from the agent. Returns an empty list when the agent is
/// unreachable or the RPC fails — failures must not block startup.
async fn list_agent_sessions() -> Vec<AgentSessionSummary> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("FutureOS: unable to connect agent for session import: {error}");
            return vec![];
        }
    };

    let inner = match client.execute_command(list_sessions_command()).await {
        Ok(response) => response.into_inner(),
        Err(error) => {
            eprintln!("FutureOS: session import transport error: {error}");
            return vec![];
        }
    };

    if !inner.success {
        let err = if inner.error.is_empty() {
            "list_sessions rejected".to_string()
        } else {
            inner.error
        };
        eprintln!("FutureOS: session import list failed: {err}");
        return vec![];
    }

    // Parse per-session rather than all-or-nothing: a single malformed entry
    // must not drop every other importable session.
    let value: serde_json::Value = match serde_json::from_str(&inner.data) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("FutureOS: session import parse failed: {error}");
            return vec![];
        }
    };
    let raw_sessions = value
        .get("sessions")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let mut sessions = Vec::with_capacity(raw_sessions.len());
    for raw in raw_sessions {
        match serde_json::from_value::<AgentSessionSummary>(raw) {
            Ok(summary) => sessions.push(summary),
            Err(error) => {
                eprintln!("FutureOS: skipping malformed session in import list: {error}");
            }
        }
    }
    sessions
}

/// Fetch the full entry list for a session. Returns empty on any failure.
async fn fetch_session_entries(session_id: &str) -> Vec<serde_json::Value> {
    let mut client = match connect_agent().await {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let resp = match client
        .execute_command(get_session_entries_command(session_id.to_string()))
        .await
    {
        Ok(r) => r.into_inner(),
        Err(_) => return vec![],
    };
    if !resp.success {
        return vec![];
    }
    serde_json::from_str::<serde_json::Value>(&resp.data)
        .ok()
        .and_then(|v| v.get("entries").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

// ─── helpers ────────────────────────────────────────────────────────────────

/// Strip trailing whitespace AND path separators (`/` and `\`) so the
/// final directory name is always meaningful regardless of platform.
fn clean_cwd(raw: &str) -> &str {
    raw.trim().trim_end_matches(['/', '\\'])
}

/// Derive a display title for a session. Prefer the first user message, then
/// the agent-stored name (unless it's just the workspace directory name),
/// then the cwd basename.
fn session_title(summary: &AgentSessionSummary) -> String {
    // Trim trailing whitespace / separators so the basename is meaningful
    // (a lone space from "project/ " would otherwise leak into the title).
    let cwd = clean_cwd(&summary.cwd);
    let cwd_basename = std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("");

    // First user message is the most descriptive title.
    if let Some(ref first) = summary.first_message {
        let trimmed = first.trim();
        if !trimmed.is_empty() {
            let title: String = trimmed.chars().take(40).collect();
            return if title.len() < trimmed.len() {
                format!("{}…", title)
            } else {
                title
            };
        }
    }

    // Use the agent-stored name only when it's a meaningful user-assigned
    // name, not just the workspace directory name leaked into session_name.
    if let Some(ref name) = summary.name {
        let name = name.trim();
        if !name.is_empty() && name != cwd_basename {
            return name.to_string();
        }
    }

    if !cwd_basename.is_empty() {
        return cwd_basename.to_string();
    }
    "Imported Chat".to_string()
}

/// Create a completed run record for one assistant turn in an imported session.
fn create_historical_run(
    thread_id: &str,
    model: &str,
) -> Result<store::RunRecord, crate::AppError> {
    let (provider, model_id) = super::session::split_model(model);
    let run = store::create_run(store::CreateRunInput {
        thread_id: thread_id.to_string(),
        trigger_message_id: None,
        model_provider: provider,
        model_id,
    })?;
    store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run.id.clone(),
        status: "completed".to_string(),
        error_message: None,
        error_type: None,
    })?;
    Ok(run)
}

/// Decide whether a session should be imported as a chat or workspace thread.
///
/// - `$HOME/.future/workspaces/chat/…` → chat (GUI-managed)
/// - empty cwd                            → chat (assign a chat cwd, best-effort write-back)
/// - anything else                        → workspace (real project directory)
///
/// The cwd from the agent may carry trailing whitespace or separators
/// (e.g. `"~/project/ "`), which would make `Path::file_name()` return a
/// lone space instead of the directory name — producing a workspace name
/// that looks empty in the UI.  Trim and canonicalise early.
fn thread_mode(
    summary: &AgentSessionSummary,
    title: &str,
) -> (String, Option<String>, Option<String>, Option<String>) {
    // Normalise: trim trailing whitespace + separators so the path
    // behaves as the user intended and `file_name()` is meaningful.
    let cwd = clean_cwd(&summary.cwd);

    if is_gui_chat_cwd(cwd) {
        match store::get_or_create_chat_workspace(&summary.id, Some(title.to_string())) {
            Ok(ws) => return ("chat".to_string(), Some(ws.id), None, None),
            Err(_) => return ("chat".to_string(), None, None, None),
        }
    }

    if cwd.is_empty() {
        match store::get_or_create_chat_workspace(&summary.id, Some(title.to_string())) {
            Ok(ws) => return ("chat".to_string(), Some(ws.id), None, None),
            Err(_) => return ("chat".to_string(), None, None, None),
        }
    }

    // Real project directory → workspace thread.
    let name = std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(title)
        .to_string();
    (
        "workspace".to_string(),
        None,
        Some(cwd.to_string()),
        Some(name),
    )
}

/// Returns `true` when `cwd` is strictly under the GUI chat workspace
/// directory: `$HOME/.future/workspaces/chat/…`. Tolerates Windows `\`
/// separators and resolves the home dir via `crate::home_dir()` (which falls
/// back to `USERPROFILE`, since `HOME` is normally unset on Windows).
fn is_gui_chat_cwd(cwd: &str) -> bool {
    if cwd.is_empty() {
        return false;
    }
    // Normalize separators so a Windows `C:\Users\<user>\.future\workspaces\chat\`
    // matches the forward-slash suffix below.
    let cwd = cwd.replace('\\', "/");
    const SUFFIX: &str = "/.future/workspaces/chat/";
    // Literal tilde: ~/.future/workspaces/chat/…
    if cwd.starts_with(&format!("~{SUFFIX}")) {
        return true;
    }
    // Expanded home: <home>/.future/workspaces/chat/…
    if let Some(home) = crate::home_dir() {
        let home = home.replace('\\', "/");
        let prefix = format!("{}{SUFFIX}", home.trim_end_matches('/'));
        if cwd.starts_with(&prefix) {
            return true;
        }
    }
    false
}

/// Best-effort write-back: tell the agent to use `cwd` for the session so the
/// session file matches the assigned GUI chat workspace.
async fn write_back_cwd(session_id: &str, cwd: &str) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    // Use new_session with the existing id and the assigned cwd.
    // The agent will load the session from disk (idempotent) and save
    // it back with the updated cwd in the session_info entry.
    let cmd = super::client::new_session_command(
        session_id.to_string(),
        cwd.to_string(),
        "gui",
        serde_json::Value::Null,
        None, // keep existing model
        None, // keep existing thinking level
    );
    let resp = client
        .execute_command(cmd)
        .await
        .map_err(|e| format!("rpc: {e}"))?
        .into_inner();
    if !resp.success {
        return Err(if resp.error.is_empty() {
            "agent rejected".to_string()
        } else {
            resp.error
        });
    }
    Ok(())
}

// ─── import ─────────────────────────────────────────────────────────────────

/// Import a single agent session. Creates workspace, thread, and per-turn run
/// records. Idempotent via `find_thread_by_agent_session`.
async fn import_one(summary: &AgentSessionSummary) -> Result<usize, crate::AppError> {
    let best_title = session_title(summary);
    let cwd_basename = std::path::Path::new(&summary.cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // If a thread already exists, check whether its title needs healing:
    // old imports that couldn't parse first_message/name fell back to the
    // workspace directory name. Re-sync when the stored title is clearly
    // stale and a better one is available.
    if let Some(existing) = store::find_thread_by_agent_session(&summary.id)? {
        let is_default = existing.title.is_empty()
            || existing.title == cwd_basename
            || existing.title == "New Chat"
            || existing.title == "新对话";
        if is_default
            && !best_title.is_empty()
            && best_title != existing.title
            && best_title != cwd_basename
        {
            let input = crate::store::RenameThreadInput {
                thread_id: existing.id.clone(),
                title: best_title.clone(),
            };
            let _ = crate::store::rename_thread(input);
            // Sync the corrected title back to the agent.
            let session_id = summary.id.clone();
            let sync_title = best_title.clone();
            tokio::spawn(async move {
                if let Ok(mut client) = connect_agent().await {
                    let cmd = set_session_name_command(sync_title, session_id);
                    let _ = client.execute_command(cmd).await;
                }
            });
        }
        return Ok(0);
    }

    let title = best_title;
    let (mode, workspace_id, workspace_path, workspace_name) = thread_mode(summary, &title);

    let thread = store::create_thread(store::CreateThreadInput {
        mode,
        title: Some(title.clone()),
        workspace_id,
        workspace_path: workspace_path.clone(),
        workspace_name,
        agent_session_id: Some(summary.id.clone()),
    })?;

    // Sync the agent's session_name to the newly-derived title so the sidebar
    // and agent state stay consistent — the agent may have a stale session_name
    // (e.g. workspace directory name) that no longer matches the thread title.
    {
        let session_id = summary.id.clone();
        let sync_title = title.clone();
        tokio::spawn(async move {
            if let Ok(mut client) = connect_agent().await {
                let cmd = set_session_name_command(sync_title, session_id);
                let _ = client.execute_command(cmd).await;
            }
        });
    }

    // If the session had no cwd, write the assigned chat workspace path back to
    // the agent so its session_info cwd matches what a later resume compares
    // against. Use the *created thread's* actual workspace path (thread-id
    // based), not the summary-id path from `thread_mode` — otherwise
    // `ensure_agent_session` sees a cwd mismatch on resume and forks a fresh,
    // empty session, orphaning the imported history.
    if summary.cwd.is_empty() {
        if let Ok(cwd) = super::session::workspace_path_for_thread(&thread.id) {
            let sid = summary.id.clone();
            tokio::spawn(async move {
                if let Err(e) = write_back_cwd(&sid, &cwd).await {
                    eprintln!("FutureOS: cwd write-back failed for {sid}: {e}");
                }
            });
        }
    }

    // Fetch entries to count assistant turns and synthesize run events.
    let entries = fetch_session_entries(&summary.id).await;
    let assistant_count = entries
        .iter()
        .filter(|e| e.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .count();
    let run_count = assistant_count.max(1);

    let mut run_ids: Vec<String> = Vec::with_capacity(run_count);
    for _ in 0..run_count {
        let run = create_historical_run(&thread.id, &summary.model)?;
        run_ids.push(run.id);
    }

    // Write synthetic run events from the imported session's tool calls
    // so the right panel (Runs tab) is populated immediately.
    if let Err(e) = super::session::synthesize_run_events_from_entries(&entries, &run_ids) {
        eprintln!("FutureOS: import run-event synthesis failed: {e}");
    }

    Ok(run_count)
}

/// Discover agent sessions not yet in the GUI DB and import them. Runs in the
/// background on startup — failures are logged but never block the UI.
///
/// Concurrency is bounded by a semaphore (4 parallel imports). Each import may
/// fetch session entries (one extra RPC) to create per-turn run records.
pub async fn import_missing_sessions() -> Result<(), crate::AppError> {
    let sessions = list_agent_sessions().await;
    if sessions.is_empty() {
        return Ok(());
    }

    let total = sessions.len();
    let semaphore = Arc::new(Semaphore::new(4));
    let mut handles = Vec::new();

    for summary in sessions {
        let permit = semaphore.clone().acquire_owned().await;
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            import_one(&summary).await
        }));
    }

    let mut imported = 0usize;
    let mut total_runs = 0usize;
    for handle in handles {
        match handle.await {
            Ok(Ok(runs)) => {
                imported += 1;
                total_runs += runs;
            }
            Ok(Err(error)) => {
                eprintln!("FutureOS: session import error: {error}");
            }
            Err(join_error) => {
                eprintln!("FutureOS: session import panic: {join_error}");
            }
        }
    }

    if imported > 0 {
        eprintln!(
            "FutureOS: imported {imported} session(s) ({total_runs} runs) out of {total} agent session(s)"
        );
    }

    Ok(())
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that rename_all = "camelCase" + individual rename attributes
    /// correctly parse the agent's snake_case JSON keys.
    #[test]
    fn agent_session_summary_parses_list_sessions_json() {
        let raw = serde_json::json!({
            "id": "abc123",
            "session_name": "fix the login bug",
            "model": "deepseek-v4-pro",
            "cwd": "/Users/test/my-project",
            "updated_at": "2026-07-21 10:00:00",
            "parent_session_id": "parent-1",
            "first_message": "please fix the login bug on the homepage",
            "query_count": 5,
            "is_streaming": true,
        });

        let summary: AgentSessionSummary =
            serde_json::from_value(raw).expect("should parse list_sessions JSON");

        assert_eq!(summary.id, "abc123");
        assert_eq!(summary.name.as_deref(), Some("fix the login bug"));
        assert_eq!(
            summary.first_message.as_deref(),
            Some("please fix the login bug on the homepage")
        );
        assert_eq!(summary.cwd, "/Users/test/my-project");
        assert_eq!(summary.model, "deepseek-v4-pro");
        assert!(summary.is_streaming);
    }

    /// session_title prefers first_message over name and cwd_basename.
    #[test]
    fn session_title_prefers_first_message() {
        let summary = AgentSessionSummary {
            id: "abc".into(),
            name: Some("my-project".into()),
            cwd: "/Users/test/my-project".into(),
            model: "deepseek".into(),
            first_message: Some("help me debug this".into()),
            parent_session_id: String::new(),
            is_streaming: false,
        };
        assert_eq!(session_title(&summary), "help me debug this");
    }

    /// session_title uses name when it differs from cwd_basename.
    #[test]
    fn session_title_uses_name_when_not_cwd() {
        let summary = AgentSessionSummary {
            id: "abc".into(),
            name: Some("custom name".into()),
            cwd: "/Users/test/my-project".into(),
            model: "deepseek".into(),
            first_message: None,
            parent_session_id: String::new(),
            is_streaming: false,
        };
        assert_eq!(session_title(&summary), "custom name");
    }

    /// session_title skips name when it equals cwd_basename and falls back to cwd.
    #[test]
    fn session_title_skips_name_equal_to_cwd() {
        let summary = AgentSessionSummary {
            id: "abc".into(),
            name: Some("my-project".into()),
            cwd: "/Users/test/my-project".into(),
            model: "deepseek".into(),
            first_message: None,
            parent_session_id: String::new(),
            is_streaming: false,
        };
        assert_eq!(session_title(&summary), "my-project"); // falls back to cwd_basename
    }

    /// session_title falls back to "Imported Chat" when nothing else is available.
    #[test]
    fn session_title_fallback() {
        let summary = AgentSessionSummary {
            id: "abc".into(),
            name: None,
            cwd: String::new(),
            model: "deepseek".into(),
            first_message: None,
            parent_session_id: String::new(),
            is_streaming: false,
        };
        assert_eq!(session_title(&summary), "Imported Chat");
    }
}
