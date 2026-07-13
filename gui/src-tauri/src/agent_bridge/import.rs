//! Import agent sessions into the GUI on startup. Discovers sessions that exist
//! on the agent but not in the local SQLite DB, then creates workspace + thread
//! records + per-turn run records so they appear in the thread list and right
//! panel immediately.

use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use super::client::{connect_agent, get_session_entries_command, list_sessions_command};
use crate::store;

// ─── agent RPC types ────────────────────────────────────────────────────────

/// Lightweight session summary from the agent's `list_sessions` RPC.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionSummary {
    id: String,
    #[serde(default)]
    name: Option<String>,
    // Tolerate a missing/null cwd (e.g. channel sessions) — an empty cwd is
    // routed to a chat workspace by `thread_mode`, not dropped.
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    first_message: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    parent_session_id: String,
}

/// A single entry from the agent's `get_session_entries` RPC.
/// Only `role` is used for counting assistant turns; the rest exist for
/// correct deserialization of the agent response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct Entry {
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    thinking: String,
    #[serde(default)]
    tool_calls: Vec<serde_json::Value>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tool_args: String,
}

#[derive(Debug, Deserialize)]
struct EntriesResponse {
    entries: Vec<Entry>,
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
async fn fetch_session_entries(session_id: &str) -> Vec<Entry> {
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
    serde_json::from_str::<EntriesResponse>(&resp.data)
        .map(|r| r.entries)
        .unwrap_or_default()
}

// ─── helpers ────────────────────────────────────────────────────────────────

/// Derive a display title for a session. Prefer the agent-stored name, then the
/// first user message (truncated), then the cwd basename.
fn session_title(summary: &AgentSessionSummary) -> String {
    if let Some(ref name) = summary.name {
        if !name.trim().is_empty() {
            return name.trim().to_string();
        }
    }
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
    std::path::Path::new(&summary.cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Imported Chat")
        .to_string()
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
fn thread_mode(
    summary: &AgentSessionSummary,
    title: &str,
) -> (String, Option<String>, Option<String>, Option<String>) {
    if is_gui_chat_cwd(&summary.cwd) {
        match store::get_or_create_chat_workspace(&summary.id, Some(title.to_string())) {
            Ok(ws) => return ("chat".to_string(), Some(ws.id), None, None),
            Err(_) => return ("chat".to_string(), None, None, None),
        }
    }

    if summary.cwd.is_empty() {
        match store::get_or_create_chat_workspace(&summary.id, Some(title.to_string())) {
            Ok(ws) => return ("chat".to_string(), Some(ws.id), None, None),
            Err(_) => return ("chat".to_string(), None, None, None),
        }
    }

    // Real project directory → workspace thread.
    let name = std::path::Path::new(&summary.cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(title)
        .to_string();
    (
        "workspace".to_string(),
        None,
        Some(summary.cwd.clone()),
        Some(name),
    )
}

/// Returns `true` when `cwd` is strictly under the GUI chat workspace
/// directory: `$HOME/.future/workspaces/chat/…`
fn is_gui_chat_cwd(cwd: &str) -> bool {
    if cwd.is_empty() {
        return false;
    }
    // Literal tilde: ~/.future/workspaces/chat/…
    if cwd.starts_with("~/.future/workspaces/chat/") {
        return true;
    }
    // Expanded home: /Users/<user>/.future/workspaces/chat/…
    if let Ok(home) = std::env::var("HOME") {
        let prefix = format!("{}/.future/workspaces/chat/", home.trim_end_matches('/'));
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
    if store::find_thread_by_agent_session(&summary.id)?.is_some() {
        return Ok(0);
    }

    let title = session_title(summary);
    let (mode, workspace_id, workspace_path, workspace_name) = thread_mode(summary, &title);

    let thread = store::create_thread(store::CreateThreadInput {
        mode,
        title: Some(title),
        workspace_id,
        workspace_path: workspace_path.clone(),
        workspace_name,
        agent_session_id: Some(summary.id.clone()),
    })?;

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

    // Fetch entries to count assistant turns for per-turn run records.
    let entries = fetch_session_entries(&summary.id).await;
    let assistant_count = entries.iter().filter(|e| e.role == "assistant").count();

    if assistant_count > 0 {
        for _ in 0..assistant_count {
            create_historical_run(&thread.id, &summary.model)?;
        }
    } else {
        // At least one run so the session appears in the right panel.
        create_historical_run(&thread.id, &summary.model)?;
    }

    Ok(assistant_count.max(1))
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
