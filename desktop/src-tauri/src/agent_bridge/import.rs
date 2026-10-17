//! Import agent sessions into the GUI on startup. Discovers sessions that exist
//! on the agent but not in the local SQLite DB, then creates workspace + thread
//! records + per-reply run records so they appear in the thread list and right
//! panel immediately.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

use super::client::{
    connect_agent, get_session_entries_page_command, get_state_command, list_session_ids_command,
    list_sessions_command, map_rpc_error, set_session_name_command, RpcResponseExt,
};
use crate::store;

/// Serialize the complete import lifecycle per Agent session. The database
/// unique index prevents duplicate thread rows, while this lock also prevents
/// a push observer from starting against a row whose full-history import is
/// still synthesizing run projections.
static SESSION_IMPORT_LOCKS: LazyLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn session_import_lock(session_id: &str) -> Arc<AsyncMutex<()>> {
    let mut locks = SESSION_IMPORT_LOCKS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(session_id).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(session_id.to_string(), Arc::downgrade(&lock));
    lock
}

// ─── agent RPC types ────────────────────────────────────────────────────────

/// Desktop import shape derived from the typed `list_sessions` response.
#[derive(Debug)]
pub struct AgentSessionSummary {
    pub id: String,
    pub name: Option<String>,
    pub cwd: String,
    pub model: String,
    pub first_message: Option<String>,
    pub parent_session_id: String,
    /// Whether the agent is currently streaming a response for this session.
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

    let Some(sessions) = future_rpc::decode::decode_list_sessions(&inner) else {
        eprintln!("FutureOS: session import parse failed: typed payload is missing or invalid");
        return vec![];
    };
    sessions
        .into_iter()
        .map(|summary| AgentSessionSummary {
            id: summary.id,
            name: summary.session_name,
            cwd: summary.cwd,
            model: summary.model,
            first_message: summary.first_message,
            parent_session_id: summary.parent_session_id.unwrap_or_default(),
            is_streaming: summary.is_streaming,
        })
        .collect()
}

/// The set of session ids the agent currently knows about, with transport
/// failures surfaced as `Err` instead of an empty list. Consumed by the
/// orphan-thread cleanup, which must distinguish "agent unreachable" (skip,
/// delete nothing) from "agent has no sessions" — an ambiguous empty list
/// could nuke every thread on a transient failure.
///
/// Uses the filename-only `list_session_ids` RPC (not `list_sessions`), so a
/// session whose journal is momentarily unreadable, truncated, or corrupt is
/// still reported as live here — reconciliation must never mistake a transient
/// read failure for a deleted session and hard-delete local threads.
pub(crate) async fn list_agent_session_ids(
) -> Result<std::collections::HashSet<String>, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(list_session_ids_command())
        .await
        .map_err(|status| map_rpc_error("list_session_ids", status))?
        .into_inner()
        .ok_or_rpc_error("list_session_ids rejected")?;
    let value: serde_json::Value = future_rpc::decode::response_data(&response);
    let mut ids = std::collections::HashSet::new();
    for id in value
        .get("ids")
        .and_then(|ids| ids.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(id) = id.as_str().filter(|id| !id.is_empty()) {
            ids.insert(id.to_string());
        }
    }
    Ok(ids)
}

/// Fetch the full entry list for a session. An unreadable journal is a real
/// import failure, not an empty conversation.
async fn fetch_session_entries(
    session_id: &str,
) -> Result<Vec<serde_json::Value>, crate::AppError> {
    let data = super::get_session_entries(session_id.to_string()).await?;
    Ok(data
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect())
}

// ─── helpers ────────────────────────────────────────────────────────────────

/// Strip trailing whitespace AND path separators (`/` and `\`) so the
/// final directory name is always meaningful regardless of platform.
fn clean_cwd(raw: &str) -> &str {
    raw.trim().trim_end_matches(['/', '\\'])
}

/// Derive a display title for a session. Prefer a name someone chose, then the
/// first user message, then the cwd basename.
fn session_title(summary: &AgentSessionSummary) -> String {
    // Trim trailing whitespace / separators so the basename is meaningful
    // (a lone space from "project/ " would otherwise leak into the title).
    let cwd = clean_cwd(&summary.cwd);
    let cwd_basename = cwd_basename(cwd);

    // A name someone chose outranks the first message. It can come from
    // `future session new --name`, `/name`, the CLI or a channel, and the first
    // message is often machine-generated: this title is synced back to the
    // agent (see `import_one`), so preferring the message renamed sessions to
    // the first 40 characters of their own first prompt.
    if let Some(name) = explicit_name(summary, cwd_basename) {
        return name.to_string();
    }

    // Nothing named it: the first user message is the most descriptive title.
    // `plain_title_text` is the same rule the Agent applies when it falls back
    // to the message for `session_name`, so a reference to another conversation
    // does not become this thread's title on one surface and not the other.
    if let Some(ref first) = summary.first_message {
        let plain = future_rpc::title::plain_title_text(first);
        if !plain.is_empty() {
            let title: String = plain.chars().take(40).collect();
            return if title.len() < plain.len() {
                format!("{}…", title)
            } else {
                title
            };
        }
    }

    if !cwd_basename.is_empty() {
        return cwd_basename.to_string();
    }
    "Imported Chat".to_string()
}

/// The final directory name of a cleaned cwd, or `""` when there is none.
fn cwd_basename(cwd: &str) -> &str {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("")
}

/// The name this session was given, if it says more than the workspace
/// directory the agent falls back to. Shared by the title derivation and the
/// name sync-back so the two cannot disagree about what counts as a real name.
fn explicit_name<'a>(summary: &'a AgentSessionSummary, cwd_basename: &str) -> Option<&'a str> {
    let name = summary.name.as_deref()?.trim();
    (!name.is_empty() && name != cwd_basename).then_some(name)
}

/// The entry roles a transcript renders as a message. Everything else a journal
/// holds — `session_info` metadata, `run_started` / `run_terminal` markers,
/// compaction checkpoints — is bookkeeping that no list should turn into a
/// conversation row.
fn is_message_role(role: &str) -> bool {
    matches!(role, "user" | "assistant" | "tool")
}

/// Whether a fetched history holds at least one message entry.
///
/// A session exists (and is announced) the moment a client creates it, while
/// its first prompt arrives afterwards: mirroring that interval produces an
/// empty conversation in every list, plus the throw-away temp workspace the
/// mirror creates for it. Import waits for the first message instead — the
/// periodic pass re-reads the session and imports it then, so nothing is lost
/// by waiting, and a session that is still empty after a restart is simply not
/// a conversation yet.
fn has_message(entries: &[serde_json::Value]) -> bool {
    entries.iter().any(|entry| {
        entry
            .get("role")
            .and_then(|role| role.as_str())
            .is_some_and(is_message_role)
    })
}

/// Entries read to decide whether a session holds any message. The probe is
/// only a prefix — a conversation's first exchange is at its very start — and a
/// page that ends mid-journal is reported as "has messages" (see below), so the
/// bound can never turn a partial read into "empty".
const MESSAGE_PROBE_PAGE: i64 = 50;

/// Whether the Agent can already show a message for `session_id`.
///
/// Errors (an unreadable journal, an offline Agent) propagate: no caller may
/// treat "could not read" as "empty" — one of them deletes local rows.
pub(crate) async fn session_has_messages(session_id: &str) -> Result<bool, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_session_entries_page_command(
            session_id.to_string(),
            0,
            MESSAGE_PROBE_PAGE,
        ))
        .await
        .map_err(|status| map_rpc_error("get_session_entries", status))?
        .into_inner()
        .ok_or_rpc_error("get_session_entries rejected")?;
    let page = future_rpc::decode::decode_session_entries_page(&response).ok_or_else(|| {
        crate::AppError::Message("get_session_entries typed page is missing or invalid".to_string())
    })?;
    // A page that stops mid-journal proves nothing about the entries it did not
    // include, so it counts as "has messages".
    Ok(page.has_more
        || page
            .entries
            .iter()
            .any(|entry| is_message_role(&entry.role)))
}

/// Whether the Agent's journal already holds the terminal for this group's run.
///
/// A run that reached a terminal stamps its outcome (`run.status`, projected
/// from the journal's `run_terminal` marker) onto every display entry of that
/// run; the entries of a run the Agent has not terminated carry none. Import
/// therefore reads the group's own outcome rather than assuming a conversation
/// on disk is over — see [`create_imported_run`] for what an unterminated run
/// must not become.
fn group_has_run_terminal(entries: &[serde_json::Value], group: &[usize]) -> bool {
    group
        .iter()
        .filter_map(|index| entries.get(*index))
        .any(|entry| entry.get("run").and_then(|run| run.get("status")).is_some())
}

/// Create the run record for one reply in an imported session.
///
/// `terminated` says whether the Agent's journal already holds this run's
/// terminal (see [`group_has_run_terminal`]). A run it does not hold is one the
/// import met while it was still executing (a prompt from the TUI, the CLI or
/// another machine) or one the Agent was killed before it could terminate:
/// that run is created `running` and left alone. Recorded completed it left
/// `active_run_sessions` the moment it was imported, so no client reading the
/// catalogue (the phone's merged list, the quit guard) could see the live run —
/// only the desktop's own `list_streaming_sessions` poll kept its sidebar
/// spinner. The session observer binds this row (it carries the Agent's
/// canonical run id) and settles it at the run's real `agent_end`; the
/// active-run watchdog settles one whose Agent never terminated it. A
/// terminated run keeps the behaviour this import has always had.
fn create_imported_run(
    thread_id: &str,
    model: &str,
    run_id: String,
    terminated: bool,
) -> Result<store::RunRecord, crate::AppError> {
    let (provider, model_id) = super::session::split_model(model);
    let run = store::create_run(store::CreateRunInput {
        id: Some(run_id),
        thread_id: thread_id.to_string(),
        trigger_message_id: None,
        model_provider: provider,
        model_id,
    })?;
    if terminated {
        store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })?;
    }
    Ok(run)
}

/// Decide whether a session should be imported as a chat or workspace thread.
///
/// - a session that belongs to another one → it inherits the parent's mode and
///   workspace (see below)
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
    // A conversation that belongs to another one hangs under it, in the
    // parent's workspace: forking copies the parent's cwd, so deriving a
    // workspace from the child's own cwd would open a second group for the
    // same directory — or an empty one, when the child lives in the parent's
    // chat directory. Inheriting also keeps the child's workspace path equal
    // to the cwd its Agent session actually runs in, so the first prompt does
    // not look like cwd drift and silently replace the session.
    if let Some(parent) = parent_thread(summary) {
        return (parent.mode, Some(parent.workspace_id), None, None);
    }

    // Normalise: trim trailing whitespace + separators so the path
    // behaves as the user intended and `file_name()` is meaningful.
    let cwd = clean_cwd(&summary.cwd);

    if is_desktop_chat_cwd(cwd) {
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

/// The local thread a session's parent session maps to, if Desktop already
/// knows it. Absent parent — a lineage minted elsewhere, or one whose
/// conversation has not been imported yet — falls back to cwd-derived mode;
/// parenthood itself is recorded separately by `sync_thread_parent_session`.
fn parent_thread(summary: &AgentSessionSummary) -> Option<store::ThreadRecord> {
    let parent = summary.parent_session_id.trim();
    if parent.is_empty() {
        return None;
    }
    store::find_thread_by_agent_session(parent).ok().flatten()
}

/// Returns `true` when `cwd` is strictly under the desktop chat workspace
/// directory: `$HOME/.future/workspaces/chat/…`. Tolerates Windows `\`
/// separators and resolves the home dir via `crate::home_dir()` (which falls
/// back to `USERPROFILE`, since `HOME` is normally unset on Windows).
fn is_desktop_chat_cwd(cwd: &str) -> bool {
    future_app_workspaces::is_chat_directory(cwd)
}

/// Best-effort write-back: tell the agent to use `cwd` for the session so the
/// session file matches the assigned desktop chat workspace.
async fn write_back_cwd(session_id: &str, cwd: &str) -> Result<(), String> {
    let mut client = connect_agent().await.map_err(|e| format!("connect: {e}"))?;
    // Use new_session with the existing id and the assigned cwd.
    // The agent will load the session from disk (idempotent) and save
    // it back with the updated cwd in the session_info entry.
    let cmd = super::client::new_session_command(
        session_id.to_string(),
        cwd.to_string(),
        "desktop",
        crate::device_identity::device_id_or_empty(),
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

/// Record that a machine-named conversation's title is a placeholder the
/// background title task may replace once it settles (see
/// `run_control::title_settled_thread_if_enabled` and
/// `store::seed_thread_auto_title`).
///
/// Best-effort on purpose. The seed is a hint for an *optional* later title
/// generation, while the thread and its run rows are the conversation itself:
/// `import_one` synthesizes the runs *after* this point, and the next discovery
/// pass sees the thread already exists and returns early, so failing the import
/// here would strand the conversation without its runs for good. The other
/// title write in this file is best-effort for the same reason.
fn seed_auto_title_best_effort(thread_id: &str, title: &str) {
    if let Err(error) = crate::store::seed_thread_auto_title(thread_id, title) {
        eprintln!("FutureOS: auto-title seed write failed for {thread_id}: {error}");
    }
}

/// Import a single agent session. Creates workspace, thread, and per-reply run
/// records. Idempotent via `find_thread_by_agent_session`.
async fn import_one(summary: &AgentSessionSummary) -> Result<usize, crate::AppError> {
    #[cfg(test)]
    if summary.id == "cov-test-import-panic" {
        panic!("cov test seam: simulated import panic");
    }
    let import_lock = session_import_lock(&summary.id);
    let _import_guard = import_lock.lock().await;
    if store::is_agent_session_tombstoned(&summary.id)? {
        return Ok(0);
    }
    let best_title = session_title(summary);
    // `explicit_name` compares against this, so it comes from the same helper
    // `session_title` uses: a name equal to the workspace directory name is the
    // agent's fallback, not a name someone chose.
    let cwd_basename = cwd_basename(clean_cwd(&summary.cwd));

    // If a thread already exists, check whether its title needs healing:
    // old imports that couldn't parse first_message/name fell back to the
    // workspace directory name. Re-sync when the stored title is clearly
    // stale and a better one is available.
    if let Some(existing) = store::find_thread_by_agent_session(&summary.id)? {
        if store::sync_thread_parent_session(&summary.id, &summary.parent_session_id)? {
            // Upgrade/backfill can change an already-rendered tree without
            // importing any new rows. Do not wait for a restart to reveal it.
            crate::emit_threads_updated();
        }
        // Converge the DB title to the agent's session_name — the name shared
        // with every client (TUI `/name`, CLI, channels). Renames made outside
        // the GUI never reach the GUI DB, and the sidebar falls back to that
        // stale DB title whenever agent state is unavailable.
        let mut current_title = existing.title.clone();
        if let Some(name) = summary
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            if name != current_title
                && crate::store::sync_thread_title(&existing.id, name).unwrap_or(false)
            {
                current_title = name.to_string();
            }
        }
        let is_default = current_title.is_empty()
            || current_title == cwd_basename
            || current_title == "New Chat"
            || current_title == "新对话";
        if is_default
            && !best_title.is_empty()
            && best_title != current_title
            && best_title != cwd_basename
        {
            let input = crate::store::RenameThreadInput {
                thread_id: existing.id.clone(),
                title: best_title.clone(),
            };
            let _ = crate::store::rename_thread(input);
            // `rename_thread` only moves the title, which would leave a seeded
            // thread wearing a title that is no longer its seed. The importer's
            // replacement is machine-derived like the one it replaced, so a
            // conversation that could still be titled keeps that ability; one
            // that never could stays unseeded.
            let _ = crate::store::reseed_thread_auto_title(&existing.id, &best_title);
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

    // Validate/read the history before creating any local rows. A corrupt
    // journal must not leave behind a thread that looks like an empty session.
    let entries = fetch_session_entries(&summary.id).await?;
    // Nor may a session that has produced no message yet: it is announced at
    // creation, before its first prompt, and mirroring it would add an empty
    // conversation (and its temp workspace) to every client's list. Waiting is
    // lossless — the next discovery pass imports it with its history.
    if !has_message(&entries) {
        return Ok(0);
    }
    let title = best_title;
    let (mode, workspace_id, workspace_path, workspace_name) = thread_mode(summary, &title);

    let (thread, created) =
        store::get_or_create_thread_for_agent_session(store::CreateThreadInput {
            mode,
            title: Some(title.clone()),
            workspace_id,
            workspace_path: workspace_path.clone(),
            workspace_name,
            agent_session_id: Some(summary.id.clone()),
        })?;
    store::sync_thread_parent_session(&summary.id, &summary.parent_session_id)?;
    if !created {
        return Ok(0);
    }
    // A derived conversation was named by a machine: the goal plane mints its
    // workers with the goal's objective as the session name, which is a
    // placeholder exactly like a fresh conversation's "New Chat" — it may be
    // replaced once the worker settles. A standalone imported conversation is
    // left alone: its name came from wherever the conversation did, and a rename
    // made in another client never reaches this store.
    if !summary.parent_session_id.trim().is_empty() {
        seed_auto_title_best_effort(&thread.id, &title);
    }

    // Sync the agent's session_name to the newly-derived title so the sidebar
    // and agent state stay consistent — the agent may have a stale session_name
    // (e.g. workspace directory name) that no longer matches the thread title.
    //
    // Never for a session that was given a name: the agent shares its
    // session_name with every client, and the title is derived from the first
    // message when nothing named the session, so this write renamed a session
    // to the first 40 characters of its own first prompt (a machine-generated
    // payload, in the case that was reported). `session_title` already returns
    // the chosen name for such a session, so there is nothing to converge.
    if explicit_name(summary, cwd_basename).is_none() {
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
    // based), not the summary-id path from `thread_mode`. The prompt path can
    // repair a later drift in place, but eager write-back keeps the imported
    // session metadata and workspace projection converged immediately.
    if summary.cwd.is_empty() {
        let cwd = super::session::workspace_path_for_thread(&thread.id)
            .expect("invariant: thread workspace exists immediately after create_thread");
        let sid = summary.id.clone();
        tokio::spawn(async move {
            if let Err(e) = write_back_cwd(&sid, &cwd).await {
                eprintln!("FutureOS: cwd write-back failed for {sid}: {e}");
            }
        });
    }

    // Count real exchanges and synthesize run events from the fetched history.
    let entry_groups = super::session::group_session_entries(&entries);
    let run_count = entry_groups.len();

    let mut run_ids: Vec<String> = Vec::with_capacity(run_count);
    for group in &entry_groups {
        let canonical_id = group
            .iter()
            .filter_map(|index| entries.get(*index))
            .find_map(|entry| entry["runId"].as_str())
            .ok_or_else(|| "Imported history has no canonical run identity".to_string())?;
        let run = create_imported_run(
            &thread.id,
            &summary.model,
            canonical_id.to_owned(),
            group_has_run_terminal(&entries, group),
        )?;
        run_ids.push(run.id);
    }

    // Write synthetic run events from the imported session's tool calls
    // so the right panel (Runs tab) is populated immediately.
    super::session::synthesize_run_events_from_entries(&entries, &entry_groups, &run_ids);

    Ok(run_count)
}

/// Runtime discovery import for a session reported by another client
/// (TUI/CLI/channels/another machine) via the `session_created` push. Creates
/// only the thread stub: for a streaming
/// session the observer mints run rows live as events arrive, so minting
/// synthetic historical runs here (as `import_one` does) would duplicate the
/// live run. Title/model heal on the next full `import_missing_sessions`
/// pass, which has richer summaries. Returns `true` when a stub was created,
/// `false` when the session was already known, tombstoned, or still without a
/// message of its own.
pub(crate) async fn import_discovered_session(session_id: &str) -> Result<bool, crate::AppError> {
    let import_lock = session_import_lock(session_id);
    let _import_guard = import_lock.lock().await;
    if store::is_agent_session_tombstoned(session_id)? {
        return Ok(false);
    }
    if store::find_thread_by_agent_session(session_id)?.is_some() {
        return Ok(false);
    }
    // The announcement arrives when a client *creates* the session, before its
    // first prompt exists, so it is not evidence of a conversation. Mirror only
    // what the Agent can already show a message for; the periodic discovery
    // pass imports the rest as soon as they have one.
    if !session_has_messages(session_id).await? {
        return Ok(false);
    }
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|e| format!("get_state: {e}"))?
        .into_inner();
    if !response.success {
        return Err(format!("agent rejected get_state: {}", response.error).into());
    }
    let state: serde_json::Value = future_rpc::decode::response_data(&response);
    let summary = AgentSessionSummary {
        id: session_id.to_string(),
        // get_state emits canonical `sessionName` plus the legacy `session_name`
        // alias (audit item 1); prefer canonical, tolerate pre-item-1 agents.
        name: state
            .get("sessionName")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        cwd: state
            .get("cwd")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        model: state
            .get("model")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        first_message: None,
        parent_session_id: state
            .get("parentSessionId")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        is_streaming: true,
    };
    let title = session_title(&summary);
    let (mode, workspace_id, workspace_path, workspace_name) = thread_mode(&summary, &title);
    let (thread, created) =
        store::get_or_create_thread_for_agent_session(store::CreateThreadInput {
            mode,
            title: Some(title.clone()),
            workspace_id,
            workspace_path,
            workspace_name,
            agent_session_id: Some(session_id.to_string()),
        })?;
    store::sync_thread_parent_session(session_id, &summary.parent_session_id)?;
    // See `import_one`: a worker's name is a machine placeholder, an imported
    // standalone conversation's is not — and a failed seed write must not fail
    // the discovery of an otherwise valid conversation.
    if created && !summary.parent_session_id.trim().is_empty() {
        seed_auto_title_best_effort(&thread.id, &title);
    }
    Ok(created)
}

/// Imports running at once — bounds the RPC load of a cold start with many
/// sessions (each import may fetch the session's entries).
const IMPORT_CONCURRENCY: usize = 4;

/// Discover agent sessions not yet in the GUI DB and import them. Runs in the
/// background on startup — failures are logged but never block the UI.
///
/// Each import may fetch session entries (one extra RPC) to create per-reply
/// run records. Imports run in generations — parents before their own
/// children, up to [`IMPORT_CONCURRENCY`] in parallel within a generation — so
/// a child always sees the parent conversation it attaches to. Without that
/// order a fork whose parent is imported in the same pass could land in the
/// Agent list first and open a workspace group of its own.
pub async fn import_missing_sessions() {
    let sessions = list_agent_sessions().await;
    if sessions.is_empty() {
        return;
    }

    let total = sessions.len();
    let semaphore = Arc::new(Semaphore::new(IMPORT_CONCURRENCY));
    let mut imported = 0usize;
    let mut total_runs = 0usize;

    for generation in generations(sessions) {
        let mut handles = Vec::new();
        for summary in generation {
            let permit = semaphore.clone().acquire_owned().await;
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                import_one(&summary).await
            }));
        }
        for handle in handles {
            match handle.await {
                // `import_one` returns the created run count: 0 for an already-known
                // session (title-heal only), >= 1 for a genuinely new import. Only
                // count the latter, so the summary log fires when something actually
                // landed — steady-state runs (all sessions known) stay silent.
                Ok(Ok(runs)) => {
                    total_runs += runs;
                    if runs > 0 {
                        imported += 1;
                    }
                }
                Ok(Err(error)) => {
                    eprintln!("FutureOS: session import error: {error}");
                }
                Err(join_error) => {
                    eprintln!("FutureOS: session import panic: {join_error}");
                }
            }
        }
    }

    if imported > 0 {
        eprintln!(
            "FutureOS: imported {imported} session(s) ({total_runs} runs) out of {total} agent session(s)"
        );
        // New threads landed in the store — let the sidebar know.
        crate::emit_threads_updated();
    }
}

/// Split sessions into generations: index 0 holds every session whose parent
/// is outside the batch, later ones hold their descendants, and a whole
/// generation is imported before the next starts.
///
/// A lineage that leaves the batch (the parent lives on another machine),
/// loops, or nests absurdly deep terminates: the walk stops at the first
/// ancestor it cannot follow, and each step is bounded.
fn generations(sessions: Vec<AgentSessionSummary>) -> Vec<Vec<AgentSessionSummary>> {
    const MAX_DEPTH: usize = 64;
    let present: std::collections::HashSet<String> =
        sessions.iter().map(|session| session.id.clone()).collect();
    let parents: HashMap<String, String> = sessions
        .iter()
        .filter(|session| !session.parent_session_id.is_empty())
        .map(|session| (session.id.clone(), session.parent_session_id.clone()))
        .collect();

    let mut batches: Vec<Vec<AgentSessionSummary>> = Vec::new();
    for session in sessions {
        let mut generation = 0usize;
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut current = session.id.clone();
        while let Some(parent) = parents.get(&current).cloned() {
            // Only ancestors inside this batch order the import; a lineage
            // leaving the batch (parent on another machine) is not waited on.
            if generation >= MAX_DEPTH || !visited.insert(current) || !present.contains(&parent) {
                break;
            }
            current = parent;
            generation += 1;
        }
        if batches.len() <= generation {
            batches.resize_with(generation + 1, Vec::new);
        }
        batches[generation].push(session);
    }
    batches
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── fetch helpers ──────────────────────────────────────────────────

    fn summary(id: &str, cwd: &str) -> AgentSessionSummary {
        AgentSessionSummary {
            id: id.to_string(),
            name: None,
            cwd: cwd.to_string(),
            model: "future/k3".to_string(),
            first_message: Some(format!("first message for {id}")),
            parent_session_id: String::new(),
            is_streaming: false,
        }
    }

    #[tokio::test]
    async fn list_agent_sessions_survives_rpc_and_payload_errors() {
        let mock = super::super::test_support::mock_agent();

        // Reject → empty.
        mock.push(
            "list_sessions",
            super::super::test_support::Reply::Reject("nope".into()),
        );
        assert!(list_agent_sessions().await.is_empty());
        // Transport error → empty.
        mock.push(
            "list_sessions",
            super::super::test_support::Reply::Status(tonic::Code::Internal, "down"),
        );
        assert!(list_agent_sessions().await.is_empty());

        // Unparseable data → empty.
        mock.push(
            "list_sessions",
            super::super::test_support::Reply::Data("not json".into()),
        );
        assert!(list_agent_sessions().await.is_empty());
    }

    #[tokio::test]
    async fn list_agent_sessions_decodes_typed_only_response() {
        let mock = super::super::test_support::mock_agent();
        mock.push_typed_data(
            "list_sessions",
            serde_json::json!({"sessions": [{
                "id": "typed-session",
                "sessionName": "Typed history",
                "model": "future/k3",
                "cwd": "/ws/typed",
                "updatedAtMs": 20692000,
                "parentSessionId": null,
                "firstMessage": "hello",
                "queryCount": 1,
                "isStreaming": false
            }]}),
        );

        let sessions = list_agent_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "typed-session");
        assert_eq!(sessions[0].first_message.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn list_agent_session_ids_filters_empty_and_surfaces_failures() {
        let mock = super::super::test_support::mock_agent();

        mock.push_data(
            "list_session_ids",
            serde_json::json!({"ids": ["a", "", "b", "a", 7]}),
        );
        let ids = list_agent_session_ids().await.expect("ids");
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("a") && ids.contains("b"));

        // Reject → Err (never coerced to empty).
        mock.push(
            "list_session_ids",
            super::super::test_support::Reply::Reject("bad".into()),
        );
        let err = list_agent_session_ids().await.expect_err("reject");
        assert_eq!(err.to_string(), "bad");

        // Empty error string → the reject fallback message.
        mock.push(
            "list_session_ids",
            super::super::test_support::Reply::Reject(String::new()),
        );
        let err = list_agent_session_ids().await.expect_err("reject empty");
        assert_eq!(err.to_string(), "list_session_ids rejected");
    }

    #[tokio::test]
    async fn fetch_session_entries_reports_failures() {
        let mock = super::super::test_support::mock_agent();

        mock.push(
            "get_session_entries",
            super::super::test_support::Reply::Reject("no".into()),
        );
        assert!(fetch_session_entries("s").await.is_err());
        mock.push(
            "get_session_entries",
            super::super::test_support::Reply::Data("not json".into()),
        );
        assert!(fetch_session_entries("s").await.is_err());
    }

    #[tokio::test]
    async fn fetch_session_entries_decodes_typed_only_response() {
        let mock = super::super::test_support::mock_agent();
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [{
                "id": "entry-1",
                "role": "user",
                "kind":"user","blocks":[{"kind":"text","text":"hello"}],"createdAtMs":1000,"runId":"history-typed-session"
            }]}),
        );

        let entries = fetch_session_entries("typed-session")
            .await
            .expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["blocks"][0]["text"], "hello");
    }

    // ── thread_mode / is_desktop_chat_cwd ──────────────────────────────

    #[test]
    fn thread_mode_routes_chat_cwd_to_a_chat_workspace() {
        let home = super::super::test_support::TestHome::new("import-mode-chat");
        let chat_cwd = format!("{}/.future/workspaces/chat/sess", home.path().display());
        let s = summary("sess", &chat_cwd);
        let title = session_title(&s);
        let (mode, workspace_id, workspace_path, _name) = thread_mode(&s, &title);
        assert_eq!(mode, "chat");
        assert!(workspace_id.is_some());
        assert!(workspace_path.is_none());
    }

    #[test]
    fn thread_mode_routes_empty_cwd_to_a_chat_workspace() {
        let _home = super::super::test_support::TestHome::new("import-mode-empty");
        let s = summary("sess", "");
        let title = session_title(&s);
        let (mode, workspace_id, workspace_path, _name) = thread_mode(&s, &title);
        assert_eq!(mode, "chat");
        assert!(workspace_id.is_some());
        assert!(workspace_path.is_none());
    }

    #[test]
    fn thread_mode_routes_real_cwd_to_a_workspace() {
        let home = super::super::test_support::TestHome::new("import-mode-ws");
        let cwd = home.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let s = summary("sess", &cwd.display().to_string());
        let title = session_title(&s);
        let (mode, workspace_id, workspace_path, name) = thread_mode(&s, &title);
        assert_eq!(mode, "workspace");
        assert!(workspace_id.is_none());
        assert_eq!(
            workspace_path.as_deref(),
            Some(cwd.display().to_string().as_str())
        );
        assert_eq!(name.as_deref(), Some("proj"));
    }

    #[test]
    fn is_desktop_chat_cwd_recognizes_tilde_home_and_windows_separators() {
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let saved = std::env::var("HOME").ok();
        // The fake home must be platform-absolute: `home_dir_from` rejects a
        // drive-less `/Users/...` on Windows as non-absolute and falls back to
        // the real USERPROFILE, which would silently defeat the override.
        let fake_home = std::env::temp_dir()
            .join("futureos-fake-home")
            .to_string_lossy()
            .replace('\\', "/");
        std::env::set_var("HOME", &fake_home);

        assert!(!is_desktop_chat_cwd(""));
        assert!(is_desktop_chat_cwd("~/.future/workspaces/chat/abc"));
        assert!(!is_desktop_chat_cwd("/tmp/.future/workspaces/chat/abc"));
        // Expanded home (forward slashes) matches.
        assert!(is_desktop_chat_cwd(&format!(
            "{fake_home}/.future/workspaces/chat/abc"
        )));
        // Windows separators normalize to the same suffix.
        assert!(is_desktop_chat_cwd(&format!(
            "{}/.future/workspaces/chat/abc",
            fake_home.replace('/', "\\")
        )));
        // Expanded home under a NON-chat directory → falls through to false
        // (covers the `if let Some(home)` close brace).
        assert!(!is_desktop_chat_cwd(&format!(
            "{fake_home}/.future/workspaces/other/abc"
        )));

        super::super::test_support::restore_home(saved);
    }

    // ── connect failures ─────────────────────────────────────────────

    #[tokio::test]
    async fn import_fetch_helpers_tolerate_an_unreachable_agent() {
        let _mock = super::super::test_support::mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        assert!(list_agent_sessions().await.is_empty());
        assert!(fetch_session_entries("s").await.is_err());
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[tokio::test]
    async fn fetch_session_entries_survives_transport_error() {
        let mock = super::super::test_support::mock_agent();
        mock.push(
            "get_session_entries",
            super::super::test_support::Reply::Status(tonic::Code::Internal, "down"),
        );
        assert!(fetch_session_entries("s").await.is_err());
    }

    #[test]
    fn session_title_truncates_a_long_first_message() {
        let long = "x".repeat(60);
        let summary = AgentSessionSummary {
            id: "abc".into(),
            name: None,
            cwd: String::new(),
            model: "deepseek".into(),
            first_message: Some(long.clone()),
            parent_session_id: String::new(),
            is_streaming: false,
        };
        let title = session_title(&summary);
        assert!(title.ends_with('…'), "{title}");
        assert_eq!(title.trim_end_matches('…').len(), 40, "40 chars + ellipsis");
    }

    #[test]
    fn thread_mode_chat_workspace_failure_falls_back_to_no_workspace() {
        let _home = super::super::test_support::TestHome::new("import-mode-chat-err");
        // The tilde form makes `is_desktop_chat_cwd` true without reading HOME,
        // so breaking HOME only fails the store's `get_or_create_chat_workspace`.
        let s = summary("sess", "~/.future/workspaces/chat/sess");
        let title = session_title(&s);
        let prev = super::super::test_support::break_home();
        let (mode, workspace_id, workspace_path, _name) = thread_mode(&s, &title);
        super::super::test_support::restore_home(prev);
        assert_eq!(mode, "chat");
        assert!(workspace_id.is_none());
        assert!(workspace_path.is_none());
    }

    #[test]
    fn thread_mode_empty_cwd_failure_falls_back_to_no_workspace() {
        let _home = super::super::test_support::TestHome::new("import-mode-empty-err");
        let s = summary("sess", "");
        let title = session_title(&s);
        let prev = super::super::test_support::break_home();
        let (mode, workspace_id, workspace_path, _name) = thread_mode(&s, &title);
        super::super::test_support::restore_home(prev);
        assert_eq!(mode, "chat");
        assert!(workspace_id.is_none());
        assert!(workspace_path.is_none());
    }

    #[tokio::test]
    async fn write_back_cwd_surfaces_rejections() {
        let mock = super::super::test_support::mock_agent();

        mock.push(
            "new_session",
            super::super::test_support::Reply::Reject("agent said no".into()),
        );
        let err = write_back_cwd("sess", "/cwd").await.expect_err("reject");
        assert_eq!(err, "agent said no");

        mock.push(
            "new_session",
            super::super::test_support::Reply::Reject(String::new()),
        );
        let err = write_back_cwd("sess", "/cwd")
            .await
            .expect_err("reject empty");
        assert_eq!(err, "agent rejected");

        // Success → Ok.
        mock.push_data("new_session", serde_json::json!({ "sessionId": "sess" }));
        write_back_cwd("sess", "/cwd").await.expect("ok");
    }

    #[tokio::test]
    async fn import_one_syncs_an_agent_name_into_a_stale_title() {
        let home = super::super::test_support::TestHome::new("import-name");
        let workspace = super::super::test_support::seed_workspace(home.path(), "ws");
        let thread = super::super::test_support::seed_thread(&workspace.id, Some("sess-name"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "stale".to_string(),
        })
        .expect("rename");

        let mut s = summary("sess-name", &workspace.path);
        s.name = Some("Renamed elsewhere".to_string());
        s.parent_session_id = "late-parent".to_string();
        assert_eq!(import_one(&s).await.expect("sync"), 0);
        let updated = crate::store::get_thread(&thread.id)
            .expect("get")
            .expect("some");
        assert_eq!(updated.title, "Renamed elsewhere");
        assert_eq!(updated.parent_session_id.as_deref(), Some("late-parent"));
    }

    #[tokio::test]
    async fn import_one_heals_a_chinese_default_title() {
        let home = super::super::test_support::TestHome::new("import-cn");
        let workspace = super::super::test_support::seed_workspace(home.path(), "ws");
        let thread = super::super::test_support::seed_thread(&workspace.id, Some("sess-cn"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "新对话".to_string(),
        })
        .expect("rename");

        let s = summary("sess-cn", &workspace.path);
        assert_eq!(import_one(&s).await.expect("heal"), 0);
        settle_spawns().await;
        let updated = crate::store::get_thread(&thread.id)
            .expect("get")
            .expect("some");
        assert_eq!(updated.title, "first message for sess-cn");
    }

    /// One `list_sessions` row in the Agent's full shape. `SessionSummaryPayload`
    /// requires `updatedAtMs`, `queryCount` and `isStreaming`, so a minimal body
    /// does not decode: the typed payload fails, the `data` fallback is empty,
    /// and `list_agent_sessions` returns nothing — which makes an import test
    /// vacuous while still passing.
    fn session_row(id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "sessionName": null,
            "cwd": "",
            "model": "future/k3",
            "updatedAtMs": 20692000,
            "parentSessionId": null,
            "firstMessage": "hello",
            "queryCount": 1,
            "isStreaming": false
        })
    }

    #[tokio::test]
    async fn import_missing_sessions_logs_an_import_failure() {
        let _home = super::super::test_support::TestHome::new("import-missing-err");
        let mock = super::super::test_support::mock_agent();
        let page = serde_json::json!({"sessions": [session_row("sess-e1")]});
        mock.push_typed_data("list_sessions", page.clone());
        assert_eq!(
            list_agent_sessions().await.len(),
            1,
            "the fixture must reach the importer, not an empty list"
        );
        mock.push_typed_data("list_sessions", page);
        // Break the store AFTER list_agent_sessions (which only touches the
        // agent) so import_one fails on its first store call → the Ok(Err)
        // arm logs and the pass still returns Ok.
        let prev = super::super::test_support::break_home();
        import_missing_sessions().await;
        super::super::test_support::restore_home(prev);
    }

    /// One session whose history cannot be read is reported and skipped; the
    /// pass still returns, and no thread row is left behind for it.
    #[tokio::test]
    async fn import_missing_sessions_reports_a_rejected_history() {
        let _home = super::super::test_support::TestHome::new("import-missing-rejected");
        let mock = super::super::test_support::mock_agent();
        mock.push_typed_data(
            "list_sessions",
            serde_json::json!({"sessions": [session_row("sess-rejected")]}),
        );
        mock.push(
            "get_session_entries",
            super::super::test_support::Reply::Reject("journal unavailable".into()),
        );
        import_missing_sessions().await;
        settle_spawns().await;
        assert!(
            crate::store::find_thread_by_agent_session("sess-rejected")
                .expect("find")
                .is_none(),
            "a session whose history could not be read must not be mirrored"
        );
    }

    #[tokio::test]
    async fn list_agent_sessions_rejects_with_an_empty_error() {
        let mock = super::super::test_support::mock_agent();
        // Reject with an empty error string → the "list_sessions rejected"
        // fallback is used and the list is empty.
        mock.push(
            "list_sessions",
            super::super::test_support::Reply::Reject(String::new()),
        );
        assert!(list_agent_sessions().await.is_empty());
    }

    /// A first message may open with a reference to another conversation. The
    /// title must be the question, not the link: this thread is not that one,
    /// and its own name then appears twice in a list.
    #[test]
    fn session_title_drops_a_conversation_reference_from_the_first_message() {
        let summary = AgentSessionSummary {
            id: "abc".into(),
            name: None,
            cwd: "/Users/test/proj".into(),
            model: "m".into(),
            first_message: Some(
                "[手机截图镜像修复](futureos://session/20261016-142101-2a944dead697482d8a6adad9942fe403) 这个会话在流式回复中"
                    .into(),
            ),
            parent_session_id: String::new(),
            is_streaming: false,
        };
        assert_eq!(session_title(&summary), "这个会话在流式回复中");
    }

    #[test]
    fn session_title_skips_a_whitespace_only_first_message() {
        let summary = AgentSessionSummary {
            id: "abc".into(),
            name: Some("named".into()),
            cwd: "/Users/test/proj".into(),
            model: "m".into(),
            first_message: Some("   \n".into()),
            parent_session_id: String::new(),
            is_streaming: false,
        };
        // The whitespace-only first message is ignored (inner `if` is false),
        // falling through to the agent-stored name.
        assert_eq!(session_title(&summary), "named");
    }

    #[test]
    fn is_desktop_chat_cwd_falls_through_when_home_is_unset() {
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let saved_home = std::env::var("HOME").ok();
        let saved_profile = std::env::var("USERPROFILE").ok();
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        // No home → the `if let Some(home)` else path runs, returning false.
        assert!(!is_desktop_chat_cwd(
            "/Users/who/.future/workspaces/chat/abc"
        ));
        super::super::test_support::restore_home(saved_home);
        std::env::set_var("USERPROFILE", saved_profile.unwrap_or_default());
    }

    /// Fire-and-forget spawned tasks (title sync, cwd write-back) run against
    /// the in-process mock; give them a bounded window to complete so their
    /// bodies are exercised before the test runtime is torn down.
    async fn settle_spawns() {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }

    #[tokio::test]
    async fn import_one_spawns_cwd_write_back_and_logs_its_failure() {
        let _home = super::super::test_support::TestHome::new("import-cwd-fail");
        let mock = super::super::test_support::mock_agent();
        // The page must carry a message: an empty one returns from
        // `import_one`'s `has_message` guard two statements *before* the
        // write-back is spawned, which is how this test passed for a whole
        // segment while the arm it is named for read `DA:0`.
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","runId":"history-sess-cwd-fail","blocks":[{"kind":"text","text":"hi"}],"createdAtMs":1000}
            ]}),
        );
        // The spawned cwd write-back calls `new_session`; reject it so the
        // `if let Err` branch logs the failure. The rejection is *sticky*: a
        // one-shot reply can be taken by a lingering spawn from an earlier
        // test, which is the second way this arm stayed uncovered.
        mock.reject_default("new_session", "no cwd");
        let s = summary("sess-cwd-fail", "");
        let runs = import_one(&s).await.expect("import");
        assert_eq!(runs, 1, "the history is imported before the write-back");
        settle_spawns().await;
        // The spawned write-back is what must fail here — assert on the request
        // it made, since the log line inside the arm is not observable.
        let attempts = mock.requests_of("new_session");
        assert!(
            attempts.iter().any(|cmd| cmd.session_id == "sess-cwd-fail"),
            "the write-back must have been attempted: {attempts:?}"
        );
        mock.clear_reject_defaults();
    }

    #[tokio::test]
    async fn import_missing_sessions_logs_a_panicking_import() {
        let _home = super::super::test_support::TestHome::new("import-missing-panic");
        let mock = super::super::test_support::mock_agent();
        let page = serde_json::json!({"sessions": [session_row("cov-test-import-panic")]});
        mock.push_typed_data("list_sessions", page.clone());
        assert_eq!(
            list_agent_sessions().await.len(),
            1,
            "the fixture must reach the importer, not an empty list"
        );
        mock.push_typed_data("list_sessions", page);
        // The panicking import is caught and logged; the pass still returns Ok.
        import_missing_sessions().await;
        settle_spawns().await;
        assert_eq!(
            mock.requests_of("list_sessions").len(),
            2,
            "the pass listed sessions, so the import loop really ran"
        );
    }

    // ── import_missing_sessions / import_missing ─────────

    #[tokio::test]
    async fn import_one_title_sync_spawns_tolerate_an_unreachable_agent() {
        let home = super::super::test_support::TestHome::new("import-sync-down");
        let _mock = super::super::test_support::mock_agent();
        let workspace = super::super::test_support::seed_workspace(home.path(), "ws");

        // Existing thread with a default title → the heal path spawns a
        // title-sync task whose `if let Ok(connect_agent)` else arm runs when
        // the agent endpoint is unreachable.
        let heal = super::super::test_support::seed_thread(&workspace.id, Some("sess-heal-down"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: heal.id.clone(),
            title: "New Chat".to_string(),
        })
        .expect("rename");

        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");

        let s = summary("sess-heal-down", &workspace.path);
        assert_eq!(import_one(&s).await.expect("heal"), 0);

        // A brand-new import validates history first; an unreachable agent is
        // reported without leaving a partial empty thread behind.
        _mock.push_data("get_session_entries", serde_json::json!({"entries": []}));
        let s2 = summary("sess-new-down", "");
        assert!(import_one(&s2).await.is_err());
        assert!(crate::store::find_thread_by_agent_session("sess-new-down")
            .expect("find")
            .is_none());

        settle_spawns().await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[tokio::test]
    async fn import_one_skips_tombstoned_sessions() {
        let home = super::super::test_support::TestHome::new("import-tomb");
        let workspace = super::super::test_support::seed_workspace(home.path(), "ws");
        let thread = super::super::test_support::seed_thread(&workspace.id, Some("sess-tomb"));
        crate::store::delete_thread(&thread.id).expect("delete");

        let s = summary("sess-tomb", "");
        assert_eq!(import_one(&s).await.expect("skip"), 0);
    }

    #[tokio::test]
    async fn import_one_heals_an_existing_default_title() {
        let home = super::super::test_support::TestHome::new("import-heal");
        let workspace = super::super::test_support::seed_workspace(home.path(), "ws");
        let thread = super::super::test_support::seed_thread(&workspace.id, Some("sess-heal"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "New Chat".to_string(),
        })
        .expect("rename");

        let s = summary("sess-heal", &workspace.path);
        assert_eq!(import_one(&s).await.expect("heal"), 0);
        settle_spawns().await;
        let updated = crate::store::get_thread(&thread.id)
            .expect("get")
            .expect("some");
        assert_eq!(updated.title, "first message for sess-heal");
    }

    #[tokio::test]
    async fn import_one_creates_a_new_chat_thread_and_runs() {
        let _home = super::super::test_support::TestHome::new("import-new");
        let mock = super::super::test_support::mock_agent();
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"a1","role":"assistant","kind":"assistant","runId":"history-sess-new","blocks":[{"kind":"text","text":"a1"}],"createdAtMs":1000},
                {"id":"a2","role":"assistant","kind":"assistant","runId":"history-sess-new","blocks":[{"kind":"text","text":"a2"}],"createdAtMs":1000}
            ]}),
        );
        let s = summary("sess-new", "");
        let runs = import_one(&s).await.expect("import");
        assert_eq!(runs, 1, "consecutive assistants belong to one exchange");
        settle_spawns().await;
        let thread = crate::store::find_thread_by_agent_session("sess-new")
            .expect("find")
            .expect("some");
        assert_eq!(thread.mode, "chat");
        assert_eq!(crate::store::list_runs(&thread.id).expect("runs").len(), 1);
    }

    /// The import's own create can still lose: the thread may appear between the
    /// lookup at the top of `import_one` and the create itself — another client,
    /// the TUI, or a second import. The row that comes back then belongs to the
    /// winner, and the import must report "nothing imported" rather than counting
    /// a thread it did not create.
    ///
    /// The ordering is *forced*, not raced: a second connection stages the winner
    /// and holds the write lock, so `find_thread_by_agent_session` cannot see it
    /// (a WAL reader ignores uncommitted rows), while the import's own insert waits
    /// on that lock (`busy_timeout` 5000 ms) and then hits the unique index on the
    /// session id.
    ///
    /// The winner is staged with a stale title while the summary carries a `name`,
    /// so a run that took the *early* "a thread already exists" path would rename
    /// it. Asserting the title is untouched is what proves the late arm was taken —
    /// without it this test could pass by never reaching the `!created` return.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn import_one_returns_zero_when_the_thread_appears_after_the_lookup() {
        let _home = super::super::test_support::TestHome::new("import-create-race");
        let mock = super::super::test_support::mock_agent();
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"a1","role":"assistant","kind":"assistant","runId":"sess-race","blocks":[{"kind":"text","text":"a1"}],"createdAtMs":1000},
                {"id":"a2","role":"assistant","kind":"assistant","runId":"sess-race","blocks":[{"kind":"text","text":"a2"}],"createdAtMs":1000}
            ]}),
        );

        let dir = std::env::temp_dir().join(format!("futureos-import-race-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("import-race".into()),
            path: dir.display().to_string(),
            description: None,
            create_directory: Some(false),
        })
        .expect("workspace");

        // The competing writer: holds the write lock with the winning row staged.
        let path = future_app_settings::app_db_path().expect("db path");
        let mut other = rusqlite::Connection::open(&path).expect("second connection");
        let tx = other
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("write lock");
        tx.execute(
            "INSERT INTO threads (id, workspace_id, mode, title, status, pinned, readonly,
                                 agent_session_id, asset_root_id, last_opened_at, created_at, updated_at)
             VALUES ('thread-winner', ?1, 'chat', 'Stale Title', 'active', 0, 0, 'sess-race',
                     'thread-winner', 1, 1, 1)",
            rusqlite::params![workspace.id],
        )
        .expect("stage the winner");

        let mut summary = summary("sess-race", "");
        summary.name = Some("Renamed By The Early Path".into());
        let import = tokio::spawn(async move { import_one(&summary).await });
        // Let the import get past its lookup (and wait on the write lock).
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        tx.commit()
            .expect("release the lock with the winner committed");

        let runs = import
            .await
            .expect("join")
            .expect("a lost race is not an error");
        settle_spawns().await;
        assert_eq!(
            runs, 0,
            "the thread already existed, so nothing was imported"
        );
        let thread = crate::store::find_thread_by_agent_session("sess-race")
            .expect("find")
            .expect("some");
        assert_eq!(thread.id, "thread-winner");
        assert_eq!(
            thread.title, "Stale Title",
            "the early `already exists` path would have renamed this, so reaching the \
             `!created` return required the winner to be invisible to the lookup"
        );
    }

    /// A run the Agent has not terminated must not be imported as a finished
    /// reply. Every display entry of a *settled* run carries that run's outcome
    /// (`run`, projected from the journal's `run_terminal` marker); the entries
    /// of a run still executing carry none. Recording the live one `completed`
    /// drops it from `active_run_sessions` for the rest of its life — and that
    /// list is the `streaming` field of the catalogue the phone's merged list is
    /// built from, so the run showed no spinner there while the desktop's own
    /// `list_streaming_sessions` poll still marked its sidebar row.
    #[tokio::test]
    async fn import_one_leaves_a_run_the_agent_never_terminated_running() {
        let _home = super::super::test_support::TestHome::new("import-live-run");
        let mock = super::super::test_support::mock_agent();
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"first"}],"runId":"history-settled","createdAtMs":1000},
                {"id":"a1","role":"assistant","kind":"assistant","blocks":[{"kind":"text","text":"done"}],"runId":"history-settled","createdAtMs":1001,
                 "run":{"status":"completed","durationMs":900}},
                {"id":"u2","role":"user","kind":"user","blocks":[{"kind":"text","text":"second"}],"runId":"history-live","createdAtMs":2000},
                {"id":"a2","role":"assistant","kind":"assistant","blocks":[{"kind":"text","text":"still writing"}],"runId":"history-live","createdAtMs":2001}
            ]}),
        );
        let runs = import_one(&summary("sess-live-run", ""))
            .await
            .expect("import");
        assert_eq!(runs, 2, "one record per exchange, live or not");
        settle_spawns().await;

        let thread = crate::store::find_thread_by_agent_session("sess-live-run")
            .expect("find")
            .expect("some");
        let statuses: Vec<(String, String)> = crate::store::list_runs(&thread.id)
            .expect("runs")
            .into_iter()
            .map(|run| (run.id, run.status))
            .collect();
        for expected in [
            ("history-settled", "completed"),
            ("history-live", "running"),
        ] {
            assert!(
                statuses.contains(&(expected.0.to_string(), expected.1.to_string())),
                "{expected:?} missing from {statuses:?}"
            );
        }
        assert_eq!(
            crate::store::active_run_sessions().expect("active runs"),
            vec!["sess-live-run".to_string()],
            "the live run must stay visible to every catalogue reader"
        );
    }

    #[tokio::test]
    async fn import_one_creates_a_workspace_thread() {
        let home = super::super::test_support::TestHome::new("import-ws");
        let mock = super::super::test_support::mock_agent();
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"hi"}],"runId":"history-ws","createdAtMs":1000}
            ]}),
        );
        let cwd = home.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let s = summary("sess-ws", &cwd.display().to_string());
        let runs = import_one(&s).await.expect("import");
        assert_eq!(runs, 1);
        settle_spawns().await;
        let thread = crate::store::find_thread_by_agent_session("sess-ws")
            .expect("find")
            .expect("some");
        assert_eq!(thread.mode, "workspace");
    }

    /// A session named with `future session new --name` (or `/name`, the CLI, a
    /// channel) keeps that name. Its first message is often machine-generated —
    /// the reported case opened with a `<file name="…/turn_payload.json">`
    /// payload — and preferring that message labelled the conversation with its
    /// first 40 characters, on the agent too, because the derived title is
    /// synced back to `session_name`.
    #[tokio::test]
    async fn import_one_keeps_a_chosen_name_over_the_first_message() {
        let home = super::super::test_support::TestHome::new("import-explicit-name");
        let mock = super::super::test_support::mock_agent();
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"<file name=\"/p/turn_payload.json\">"}],"runId":"history-1","createdAtMs":1000},
                {"id":"a1","role":"assistant","kind":"assistant","blocks":[{"kind":"text","text":"ok"}],"runId":"history-1","createdAtMs":1001}
            ]}),
        );
        let cwd = home.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let mut s = summary("sess-named", &cwd.display().to_string());
        s.name = Some("会话总管".to_string());
        s.first_message =
            Some("<file name=\"/Users/geilige/state/turn_payload.json\">".to_string());
        let runs = import_one(&s).await.expect("import");
        assert_eq!(runs, 1);
        settle_spawns().await;
        let thread = crate::store::find_thread_by_agent_session("sess-named")
            .expect("find")
            .expect("some");
        assert_eq!(thread.title, "会话总管");
        assert!(
            mock.requests_of("set_session_name").is_empty(),
            "a chosen name must not be rewritten from the first message"
        );
    }

    /// The positive control for the test above: a session nobody named is still
    /// labelled from its first message, and that label is still written back to
    /// the agent. Without this, "no `set_session_name`" would also pass if the
    /// sync never ran at all.
    #[tokio::test]
    async fn import_one_still_names_an_unnamed_session_and_syncs_it() {
        let home = super::super::test_support::TestHome::new("import-unnamed");
        let mock = super::super::test_support::mock_agent();
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"hi"}],"runId":"history-1","createdAtMs":1000}
            ]}),
        );
        let cwd = home.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let s = summary("sess-unnamed", &cwd.display().to_string());
        assert_eq!(import_one(&s).await.expect("import"), 1);
        settle_spawns().await;
        let thread = crate::store::find_thread_by_agent_session("sess-unnamed")
            .expect("find")
            .expect("some");
        assert_eq!(thread.title, "first message for sess-unnamed");
        assert_eq!(
            mock.requests_of("set_session_name").len(),
            1,
            "an unnamed session still takes its label from the first message"
        );
    }

    /// A session announced at creation has no message until its first prompt
    /// lands. Mirroring that interval is what put empty conversations (and the
    /// temp workspace a mirror allocates) in every client's list, so the import
    /// must wait for a message — and must not leave any row behind meanwhile.
    #[tokio::test]
    async fn import_one_skips_a_session_without_messages() {
        let home = super::super::test_support::TestHome::new("import-empty");
        let mock = super::super::test_support::mock_agent();
        // Metadata only: a session_info entry is bookkeeping, not a message.
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"m1","role":"system","kind":"session_info","blocks":[],"createdAtMs":1000}
            ]}),
        );
        let cwd = home.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let s = summary("sess-empty", &cwd.display().to_string());

        assert_eq!(import_one(&s).await.expect("skip"), 0);
        settle_spawns().await;
        assert!(crate::store::find_thread_by_agent_session("sess-empty")
            .expect("find")
            .is_none());
        assert!(
            crate::store::list_workspaces()
                .expect("workspaces")
                .is_empty(),
            "a skipped session must not allocate its workspace either"
        );
    }

    #[tokio::test]
    async fn import_discovered_session_variants() {
        let _home = super::super::test_support::TestHome::new("import-streaming");
        let mock = super::super::test_support::mock_agent();
        let page_with_message = serde_json::json!({"entries": [
            {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"hi"}],"createdAtMs":1000}
        ]});

        // get_state reject → Err (probe first: the session has a message).
        mock.push_data("get_session_entries", page_with_message.clone());
        mock.push(
            "get_state",
            super::super::test_support::Reply::Reject("gone".into()),
        );
        let err = import_discovered_session("sess-live")
            .await
            .expect_err("reject");
        assert!(err.to_string().contains("get_state"), "{err}");

        // A page the Agent answers with that is not a decodable page is a failed
        // read, not "no messages": mirroring nothing is safe, but the caller
        // must hear about it so a broken Agent cannot look like an empty
        // session (the same session is otherwise removed as empty).
        mock.push(
            "get_session_entries",
            super::super::test_support::Reply::Data(String::new()),
        );
        let error = import_discovered_session("sess-undecodable")
            .await
            .expect_err("an undecodable page is an error");
        assert!(
            error
                .to_string()
                .contains("typed page is missing or invalid"),
            "{error}"
        );

        // Announcement of a session that still has no message → nothing mirrored,
        // and no get_state is even asked for.
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [], "hasMore": false}),
        );
        assert!(!import_discovered_session("sess-empty").await.expect("skip"));
        assert!(crate::store::find_thread_by_agent_session("sess-empty")
            .expect("find")
            .is_none());

        // Success → thread stub created.
        mock.push_data("get_session_entries", page_with_message);
        mock.push_data(
            "get_state",
            serde_json::json!({
                "sessionId": "sess-live",
                "sessionName": "Live Session",
                "parentSessionId": "supervisor",
                "cwd": "",
                "model": "future/k3"
            }),
        );
        assert!(import_discovered_session("sess-live")
            .await
            .expect("import"));
        let thread = crate::store::find_thread_by_agent_session("sess-live")
            .expect("find")
            .expect("some");
        assert_eq!(thread.title, "Live Session");
        assert_eq!(thread.parent_session_id.as_deref(), Some("supervisor"));

        // Already-known session → no-op Ok(false).
        assert!(!import_discovered_session("sess-live")
            .await
            .expect("idempotent"));
    }

    #[tokio::test]
    async fn import_discovered_session_skips_tombstoned_sessions() {
        let home = super::super::test_support::TestHome::new("import-disc-tomb");
        let workspace = super::super::test_support::seed_workspace(home.path(), "ws");
        let thread = super::super::test_support::seed_thread(&workspace.id, Some("sess-tomb2"));
        crate::store::delete_thread(&thread.id).expect("delete");

        let _mock = super::super::test_support::mock_agent();
        // No get_state reply is scripted: a tombstoned session must bail
        // before any RPC, so an unserved command would fail the test.
        assert!(!import_discovered_session("sess-tomb2").await.expect("skip"));
        assert!(crate::store::find_thread_by_agent_session("sess-tomb2")
            .expect("find")
            .is_none());
    }

    #[tokio::test]
    async fn concurrent_discovery_creates_one_thread_for_the_agent_session() {
        let _home = super::super::test_support::TestHome::new("import-disc-concurrent");
        let mock = super::super::test_support::mock_agent();
        // The per-session lock makes the loser observe the stored winner before
        // issuing any RPC, so one scripted message probe and one get_state are
        // sufficient.
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"hi"}],"createdAtMs":1000}
            ]}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({
                "sessionId": "sess-race",
                "sessionName": "Race",
                "cwd": "",
                "model": "future/k3"
            }),
        );

        let (left, right) = tokio::join!(
            import_discovered_session("sess-race"),
            import_discovered_session("sess-race")
        );
        let mut outcomes = vec![left.expect("left"), right.expect("right")];
        outcomes.sort_unstable();
        assert_eq!(outcomes, vec![false, true]);
        assert_eq!(
            crate::store::list_threads()
                .expect("list")
                .into_iter()
                .filter(|thread| thread.agent_session_id.as_deref() == Some("sess-race"))
                .count(),
            1
        );
    }

    // ── parent/child workspace inheritance ───────────────────────────

    fn summary_with_parent(id: &str, cwd: &str, parent: &str) -> AgentSessionSummary {
        AgentSessionSummary {
            id: id.to_string(),
            name: None,
            cwd: cwd.to_string(),
            model: "future/k3".to_string(),
            first_message: Some(format!("first message for {id}")),
            parent_session_id: parent.to_string(),
            is_streaming: false,
        }
    }

    /// A derived conversation is the one case where the importer records the
    /// title as a placeholder the Desktop may replace: the goal plane names its
    /// workers after the goal objective, and such a worker is steered into
    /// further turns, so it only ever settles at the end — by then the
    /// background title task must still be allowed to name it. A standalone
    /// conversation arrived with a name this store did not invent (including a
    /// rename made in another client, which never reaches this database), so it
    /// is left alone.
    #[tokio::test]
    async fn import_seeds_auto_title_only_for_a_derived_conversation() {
        let _home = super::super::test_support::TestHome::new("import-title-seed");
        let mock = super::super::test_support::mock_agent();
        let page = serde_json::json!({"entries": [
            {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"hi"}],"runId":"history-worker","createdAtMs":1000}
        ]});

        // The periodic pass: a worker (parent declared) is seeded, a standalone
        // conversation is not. Each session needs its own run identity.
        mock.push_typed_data("get_session_entries", page.clone());
        let worker = summary_with_parent("sess-seed-worker", "", "sess-supervisor");
        import_one(&worker).await.expect("import worker");
        let worker_thread = crate::store::find_thread_by_agent_session("sess-seed-worker")
            .expect("find")
            .expect("worker thread");
        assert_eq!(
            worker_thread.title_seed.as_deref(),
            Some(worker_thread.title.as_str()),
            "a worker's goal-objective title is a seed"
        );
        let standalone_page = serde_json::json!({"entries": [
            {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"hi"}],"runId":"history-alone","createdAtMs":1000}
        ]});
        mock.push_typed_data("get_session_entries", standalone_page);
        let standalone = summary("sess-seed-alone", "");
        import_one(&standalone).await.expect("import standalone");
        let standalone_thread = crate::store::find_thread_by_agent_session("sess-seed-alone")
            .expect("find")
            .expect("standalone thread");
        assert_eq!(standalone_thread.title_seed, None);
        settle_spawns().await;
    }

    /// The discovered-session path is the other way a worker enters the
    /// database, and it must record the seed too — it is the path a goal plane
    /// session takes when it appears while the Desktop is running.
    #[tokio::test]
    async fn import_discovered_session_seeds_a_derived_conversation() {
        let _home = super::super::test_support::TestHome::new("import-disc-title-seed");
        let mock = super::super::test_support::mock_agent();
        let page = serde_json::json!({"entries": [
            {"id":"u1","role":"user","kind":"user","blocks":[{"kind":"text","text":"hi"}],"createdAtMs":1000}
        ]});

        mock.push_data("get_session_entries", page.clone());
        mock.push_data(
            "get_state",
            serde_json::json!({
                "sessionId": "sess-disc-worker",
                "sessionName": "goal objective",
                "parentSessionId": "sess-supervisor",
                "cwd": "",
                "model": "future/k3"
            }),
        );
        assert!(import_discovered_session("sess-disc-worker")
            .await
            .expect("import"));
        let worker = crate::store::find_thread_by_agent_session("sess-disc-worker")
            .expect("find")
            .expect("worker thread");
        assert_eq!(worker.title_seed.as_deref(), Some("goal objective"));

        mock.push_data("get_session_entries", page);
        mock.push_data(
            "get_state",
            serde_json::json!({
                "sessionId": "sess-disc-alone",
                "sessionName": "my own name",
                "parentSessionId": null,
                "cwd": "",
                "model": "future/k3"
            }),
        );
        assert!(import_discovered_session("sess-disc-alone")
            .await
            .expect("import"));
        let standalone = crate::store::find_thread_by_agent_session("sess-disc-alone")
            .expect("find")
            .expect("standalone thread");
        assert_eq!(standalone.title_seed, None);
        assert_eq!(standalone.title, "my own name");
    }

    /// A child conversation hangs in its parent's workspace instead of opening
    /// a group of its own (a fork copies the parent's cwd, so the derived
    /// group would be a duplicate of the parent's — or empty).
    #[tokio::test]
    async fn thread_mode_inherits_the_parent_workspace() {
        let home = super::super::test_support::TestHome::new("import-child-workspace");
        let _mock = super::super::test_support::mock_agent();
        let parent_workspace = super::super::test_support::seed_workspace(home.path(), "project");
        super::super::test_support::seed_thread(&parent_workspace.id, Some("sess-parent"));

        // The child's cwd is a different directory — still no new workspace.
        let child = summary_with_parent("sess-child", "/elsewhere/other", "sess-parent");
        let (mode, workspace_id, path, name) = thread_mode(&child, "Child");
        assert_eq!(mode, "workspace");
        assert_eq!(workspace_id.as_deref(), Some(parent_workspace.id.as_str()));
        assert!(path.is_none() && name.is_none());

        // A chat parent: the child inherits the chat workspace too, so the
        // workspace path stays equal to the cwd its Agent session runs in.
        let chat_thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Chat parent".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-chat-parent".to_string()),
        })
        .expect("chat thread");
        let chat_child = summary_with_parent("sess-chat-child", "", "sess-chat-parent");
        let (mode, workspace_id, path, _) = thread_mode(&chat_child, "Child");
        assert_eq!(mode, "chat");
        assert_eq!(
            workspace_id.as_deref(),
            Some(chat_thread.workspace_id.as_str())
        );
        assert!(path.is_none());

        // Unknown parent (or none) → the cwd still decides.
        let orphan = summary_with_parent("sess-orphan", "/elsewhere/other", "sess-unknown");
        let (mode, workspace_id, path, _) = thread_mode(&orphan, "Orphan");
        assert_eq!(mode, "workspace");
        assert!(workspace_id.is_none());
        assert_eq!(path.as_deref(), Some("/elsewhere/other"));
        assert_eq!(
            thread_mode(&summary("sess-solo", "/elsewhere/other"), "Solo").0,
            "workspace"
        );
    }

    /// Importing a child in the same pass as its parent must still attach it
    /// to the parent: generations put ancestors first.
    #[tokio::test]
    async fn import_missing_sessions_imports_parents_before_children() {
        let home = super::super::test_support::TestHome::new("import-generations");
        let mock = super::super::test_support::mock_agent();
        // The child is listed FIRST — a cold Agent list has no ordering
        // guarantee, so only the generation split can order the imports.
        mock.push_typed_data(
            "list_sessions",
            serde_json::json!({"sessions": [
                {"id": "sess-child", "sessionName": null, "cwd": "/tmp/proj", "model": "future/k3",
                 "updatedAtMs": 200, "parentSessionId": "sess-parent", "firstMessage": "child", "queryCount": 1, "isStreaming": false},
                {"id": "sess-parent", "sessionName": null, "cwd": "/tmp/proj", "model": "future/k3",
                 "updatedAtMs": 100, "parentSessionId": null, "firstMessage": "parent", "queryCount": 1, "isStreaming": false}
            ]}),
        );
        for index in 0..2 {
            mock.push_typed_data(
                "get_session_entries",
                serde_json::json!({"entries": [{
                    "id": format!("reply-{index}"), "role": "assistant", "kind": "assistant",
                    "runId": format!("history-{index}"), "blocks": [{"kind": "text", "text": "ok"}],
                    "createdAtMs": 1000
                }]}),
            );
        }
        import_missing_sessions().await;
        settle_spawns().await;

        let parent = crate::store::find_thread_by_agent_session("sess-parent")
            .expect("find parent")
            .expect("parent thread");
        let child = crate::store::find_thread_by_agent_session("sess-child")
            .expect("find child")
            .expect("child thread");
        assert_eq!(child.parent_session_id.as_deref(), Some("sess-parent"));
        assert_eq!(
            child.workspace_id, parent.workspace_id,
            "the child conversation joins its parent's workspace"
        );
        // One workspace group for the directory, not one per conversation.
        let groups = crate::store::list_workspaces()
            .expect("workspaces")
            .into_iter()
            .filter(|workspace| workspace.kind == "user")
            .count();
        assert_eq!(groups, 1);
        let _ = home;
    }

    /// Generations: ancestors first, a lineage leaving the batch does not
    /// stall, and a cycle terminates.
    #[test]
    fn generations_order_ancestors_first() {
        let batch = generations(vec![
            summary_with_parent("c", "/x", "b"),
            summary_with_parent("b", "/x", "a"),
            summary_with_parent("a", "/x", ""),
            summary_with_parent("outside", "/x", "not-in-batch"),
            summary_with_parent("loop-1", "/x", "loop-2"),
            summary_with_parent("loop-2", "/x", "loop-1"),
        ]);
        let ids = |index: usize| -> Vec<String> {
            let mut ids = batch[index]
                .iter()
                .map(|session| session.id.clone())
                .collect::<Vec<_>>();
            ids.sort();
            ids
        };
        assert_eq!(ids(0), vec!["a", "outside"]);
        assert_eq!(ids(1), vec!["b"]);
        assert_eq!(ids(2), vec!["c", "loop-1", "loop-2"]);
    }

    #[tokio::test]
    async fn import_missing_sessions_imports_and_logs_errors() {
        let _home = super::super::test_support::TestHome::new("import-missing");
        let mock = super::super::test_support::mock_agent();

        // Empty list → silent Ok.
        mock.push_typed_data("list_sessions", serde_json::json!({"sessions": []}));
        import_missing_sessions().await;

        // Two sessions, each with one assistant reply.
        mock.push_typed_data(
            "list_sessions",
            serde_json::json!({"sessions": [
                {"id": "sess-m1", "sessionName": null, "cwd": "", "model": "future/k3", "updatedAtMs": 20692000, "parentSessionId": null, "firstMessage": "one", "queryCount": 1, "isStreaming": false},
                {"id": "sess-m2", "sessionName": null, "cwd": "", "model": "future/k3", "updatedAtMs": 20692000, "parentSessionId": null, "firstMessage": "two", "queryCount": 1, "isStreaming": false}
            ]}),
        );
        for index in 0..2 {
            mock.push_typed_data(
                "get_session_entries",
                serde_json::json!({"entries": [{
                    "id": format!("reply-{index}"), "role": "assistant", "kind":"assistant","runId":format!("history-sess-{index}"),"blocks":[{"kind":"text","text":"ok"}],"createdAtMs":1000
                }]}),
            );
        }
        import_missing_sessions().await;
        settle_spawns().await;
        assert!(crate::store::find_thread_by_agent_session("sess-m1")
            .expect("find")
            .is_some());
        assert!(crate::store::find_thread_by_agent_session("sess-m2")
            .expect("find")
            .is_some());
        let observers = super::super::observer::OBSERVERS.lock().unwrap();
        assert!(
            !observers.contains_key("sess-m1") && !observers.contains_key("sess-m2"),
            "discovery imports cold sessions without hydrating observers"
        );
    }
}
