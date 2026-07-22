use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use super::db::connect;
use super::records::ThreadCleanupSummary;
use super::review_snapshots::delete_run_review_in;
use super::runs::{cancel_children_of_runs, CancelScope};
use super::status::TERMINAL_RUN_STATUSES_SQL;

/// A run that was cancelled by startup convergence after a GUI crash.
#[allow(dead_code)]
pub struct InterruptedRun {
    pub run_id: String,
    pub thread_id: String,
    pub session_id: String,
}

/// Returns runs that were interrupted by a previous process crash and
/// need re-examination against the agent's actual state.
pub fn list_interrupted_runs() -> Result<Vec<InterruptedRun>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT r.id, r.thread_id,
                COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id) AS session_id
         FROM runs r
         JOIN threads t ON t.id = r.thread_id
         WHERE r.error_type = 'interrupted'
           AND r.status = 'cancelled'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(InterruptedRun {
            run_id: row.get(0)?,
            thread_id: row.get(1)?,
            session_id: row.get(2)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// Reset a run back to "running", clearing interrupted/error markers.
pub fn reanimate_run(run_id: &str) -> Result<(), crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE runs
         SET status = 'running',
             error_message = NULL,
             error_type = NULL,
             ended_at = NULL,
             updated_at = ?1
         WHERE id = ?2",
        params![now, run_id],
    )?;
    Ok(())
}

/// The agent confirmed this run completed normally — mark it as such.
pub fn settle_interrupted_run(run_id: &str, status: &str) -> Result<(), crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE runs
         SET status = ?1,
             error_message = NULL,
             error_type = NULL,
             updated_at = ?2
         WHERE id = ?3",
        params![status, now, run_id],
    )?;
    Ok(())
}
use super::util::{count_workspace_files, loaded, now_millis};
use super::{delete_thread, get_thread, get_workspace};

/// Startup reconciliation: delete active threads whose agent base data
/// (the session JSONL the agent writes under `~/.future/agent/sessions/`) has
/// been removed out from under the GUI — e.g. via the TUI/CLI `delete_session`
/// or a manual delete.
///
/// The agent treats that JSONL as the source of truth for a conversation's
/// context and reloads it on a cold start; the GUI keeps only a rendered mirror
/// (text + events), which cannot losslessly rebuild the agent's native message
/// structure (tool calls, tool results, thinking). So when the base file is gone
/// there is no faithful recovery — we delete-to-match, hard-deleting the GUI
/// thread so the two sides stay consistent instead of the model silently
/// "forgetting" a conversation the UI still shows.
///
/// Only threads with at least one `completed` run are considered: the agent
/// persists the JSONL on the successful-turn path *before* it signals
/// completion, so a completed run proves base data was written at some point. A
/// missing file then means external deletion, not a conversation that simply
/// hasn't produced base data yet (which must never be deleted). Runs once at
/// startup — mid-session the agent still holds the context in memory, so drift
/// only surfaces on the next cold start. Returns the number of threads deleted.
pub fn reconcile_orphan_sessions() -> Result<usize, crate::AppError> {
    let Some(home) = crate::home_dir() else {
        return Ok(0);
    };
    let sessions_dir = PathBuf::from(home)
        .join(".future")
        .join("agent")
        .join("sessions");
    // A missing directory is ambiguous (fresh install, or the agent has never
    // run) — never treat that as "every conversation was deleted".
    if !sessions_dir.exists() {
        return Ok(0);
    }

    let orphans = {
        let conn = connect()?;
        orphan_thread_ids(&conn, &sessions_dir)?
    };
    for thread_id in &orphans {
        // Hard delete: the JSONL (source of truth) is already gone, so purge the
        // GUI mirror and its child rows too. Also marks temp chat workspaces for
        // cleanup.
        delete_thread(thread_id)?;
    }
    Ok(orphans.len())
}

/// Decide which active threads have lost their agent base data. Split out from
/// the deletion so the (subtle) detection rules can be unit-tested against an
/// in-memory DB and a temp sessions dir.
fn orphan_thread_ids(
    conn: &Connection,
    sessions_dir: &Path,
) -> Result<Vec<String>, crate::AppError> {
    // Thread ids that have produced base data (a completed run) at least once.
    let threads_with_base: HashSet<String> = {
        let mut stmt =
            conn.prepare("SELECT DISTINCT thread_id FROM runs WHERE status = 'completed'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let candidates: Vec<(String, Option<String>)> = {
        let mut stmt =
            conn.prepare("SELECT id, agent_session_id FROM threads WHERE status != 'deleted'")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let mut orphans = Vec::new();
    for (id, agent_session_id) in candidates {
        if !threads_with_base.contains(&id) {
            continue;
        }
        // Mirror the GUI's own session-id resolution: agentSessionId when set,
        // else the thread id (see useAgentThreadState).
        let session_id = agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(id.as_str());
        if !sessions_dir.join(format!("{session_id}.jsonl")).exists() {
            orphans.push(id);
        }
    }
    Ok(orphans)
}

/// Reclaim per-thread image directories (`~/.future/app/images/<tid>`) whose
/// thread no longer lives in the DB. This is the primary reclamation path for
/// attachment thumbnails and workspace-mode originals: there is no per-delete
/// physical executor, and threads can be removed out-of-band by the TUI/CLI
/// (`delete_session`) without the GUI observing it. A thread counts as "gone"
/// once it is absent or soft-deleted (`status = 'deleted'`) — there is no
/// soft-delete undo, so a deleted thread's images are safe to drop. Runs once at
/// startup, best-effort. Returns the number of directories removed.
pub fn reconcile_orphan_images() -> Result<usize, crate::AppError> {
    reclaim_orphan_subdirs(crate::store::app_images_root()?, live_thread_ids)
}

/// Reclaim per-thread temporary chat-workspace directories
/// (`~/.future/app/workspaces/chat/<tid>`) whose thread no longer lives in the
/// DB. Deleting a thread only flags its temp workspace `pending_cleanup`; there
/// is no per-delete physical executor, so without this sweep the scratch dirs
/// leak forever. Symmetric to `reconcile_orphan_images`. User workspaces live at
/// their own user-chosen paths (never under this root), so this can never touch
/// them. Runs once at startup, best-effort. Returns the number removed.
pub fn reconcile_orphan_chat_workspaces() -> Result<usize, crate::AppError> {
    reclaim_orphan_subdirs(
        crate::store::chat_workspaces_root()?,
        live_chat_workspace_dir_ids,
    )
}

/// Reclaim per-workspace shadow-review repos (`~/.future/app/review/<wsid>`)
/// whose workspace is gone or soft-deleted. Keyed by workspace (the repo is
/// shared across a workspace's runs), so a live workspace's repo is always
/// kept — only absent/`deleted_at` workspaces are reclaimed. Runs once at
/// startup, best-effort. Returns the number removed.
pub fn reconcile_orphan_review_repos() -> Result<usize, crate::AppError> {
    reclaim_orphan_subdirs(crate::store::review_repos_root()?, live_workspace_ids)
}

/// Live (non-deleted) thread ids — the owners of `images/<tid>` directories.
fn live_thread_ids(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT id FROM threads WHERE status != 'deleted'")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Live chat workspace directory names: both thread ids (legacy) and agent
/// session ids (current).  Directories under `~/.future/workspaces/chat/` are
/// now named after the session id, but older ones may still use the thread id.
fn live_chat_workspace_dir_ids(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM threads WHERE status != 'deleted'
         UNION
         SELECT agent_session_id FROM threads
         WHERE agent_session_id IS NOT NULL AND agent_session_id != ''
           AND status != 'deleted'",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Live (non-deleted) workspace ids — the owners of `review/<wsid>` repos. Uses
/// `deleted_at IS NULL` so a soft-deleted workspace's repo becomes reclaimable
/// while every live user workspace is kept.
fn live_workspace_ids(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT id FROM workspaces WHERE deleted_at IS NULL")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Subdirectories of `root` whose name is not in `live`. Shared by the image /
/// chat-workspace / review reclaimers so they scan identically. Split out so the
/// rule can be unit-tested against an in-memory DB and a temp dir.
fn orphan_subdirs(root: &Path, live: &HashSet<String>) -> Result<Vec<PathBuf>, crate::AppError> {
    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !live.contains(&name) {
            orphans.push(entry.path());
        }
    }
    Ok(orphans)
}

/// Remove every subdir of `root` whose name has no live owner id (resolved by
/// `live_ids`). A missing root is 0; each removal is best-effort. Returns the
/// number of directories removed.
fn reclaim_orphan_subdirs(
    root: PathBuf,
    live_ids: fn(&Connection) -> rusqlite::Result<HashSet<String>>,
) -> Result<usize, crate::AppError> {
    if !root.exists() {
        return Ok(0);
    }
    let orphans = {
        let conn = connect()?;
        orphan_subdirs(&root, &live_ids(&conn)?)?
    };
    for dir in &orphans {
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(orphans.len())
}

/// Test shim preserving the original `orphan_image_dirs` name/signature.
#[cfg(test)]
fn orphan_image_dirs(conn: &Connection, root: &Path) -> Result<Vec<PathBuf>, crate::AppError> {
    orphan_subdirs(root, &live_thread_ids(conn)?)
}

pub fn get_thread_cleanup_summary(
    thread_id: &str,
) -> Result<ThreadCleanupSummary, crate::AppError> {
    let thread = loaded(get_thread(thread_id)?, "Thread")?;
    let workspace = loaded(get_workspace(&thread.workspace_id)?, "Thread workspace")?;
    let conn = connect()?;
    let artifact_count = conn.query_row(
        "SELECT COUNT(*)
             FROM artifacts
             WHERE workspace_id = ?1
               AND (thread_id = ?2 OR ?3 = 'workspace')
               AND deleted_at IS NULL",
        params![workspace.id, thread.id, thread.mode],
        |row| row.get(0),
    )?;
    let workspace_file_count = if workspace.kind == "temporary" {
        count_workspace_files(&workspace.path)?
    } else {
        0
    };

    Ok(ThreadCleanupSummary {
        thread_id: thread.id,
        workspace_id: workspace.id,
        workspace_kind: workspace.kind,
        workspace_path: workspace.path,
        cleanup_status: workspace.cleanup_status,
        artifact_count,
        workspace_file_count,
    })
}

/// Startup convergence for interrupted runs. A freshly started process has no
/// live event collector for any run, so *every* non-terminal run is an orphan —
/// its `collect_agent_response` task died when the previous process exited and
/// no event will ever settle it. Left alone, such a run strands the UI
/// in a permanent "generating" state (composer disabled, polling spinning).
///
/// This cancels all of them in one transaction and cascades the cancellation to
/// their still-open approvals and running tool calls, so a run and its children
/// never end up in mismatched states. The previous, narrower version cancelled
/// only runs that owned a pending approval; that logic is subsumed here. Returns
/// the number of runs cancelled.
///
/// Called only from the backend's setup (`lib.rs`), once per process — it is
/// deliberately NOT a Tauri command: a webview reload re-runs the frontend
/// bootstrap while this process may still own live event collectors, and a
/// reload-triggered call would cancel those live runs.
pub fn cancel_stale_approval_requests() -> Result<usize, crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let cancelled_runs = converge_orphan_runs_tx(&tx, now)?;
    tx.commit()?;
    Ok(cancelled_runs)
}

/// Cancel every non-terminal run and cascade to its still-open approvals and
/// running tool calls. Returns the number of runs cancelled. Factored out of
/// [`cancel_stale_approval_requests`] so it can be exercised against an
/// in-memory connection.
fn converge_orphan_runs_tx(tx: &rusqlite::Transaction<'_>, now: i64) -> rusqlite::Result<usize> {
    let cancelled_runs = tx.execute(
        &format!(
            "UPDATE runs
         SET status = 'cancelled',
             error_message = COALESCE(error_message, 'Interrupted because FutureOS restarted.'),
             error_type = COALESCE(error_type, 'interrupted'),
             ended_at = COALESCE(ended_at, ?1),
             updated_at = ?1
         WHERE status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![now],
    )?;
    // Cascade: any pending approval or running tool call now belongs to a
    // terminal run and must be settled too (shared with the single-run path).
    cancel_children_of_runs(
        tx,
        CancelScope::TerminalRuns,
        "Cancelled because FutureOS restarted.",
        now,
    )?;
    Ok(cancelled_runs)
}

pub fn clear_finished_runs(thread_id: &str) -> Result<usize, crate::AppError> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    // Terminal runs of this Thread, read once up front so the per-run review
    // cascade below can reuse `delete_run_review_in` (the FK-safe delete order is
    // owned by review_snapshots, not restated here). Scoped so the statement is
    // dropped before the transaction's own `execute` calls borrow it.
    let terminal_run_ids: Vec<String> = {
        let mut stmt = tx.prepare(&format!(
            "SELECT id FROM runs
             WHERE thread_id = ?1 AND status IN ({TERMINAL_RUN_STATUSES_SQL})"
        ))?;
        let rows = stmt.query_map(params![thread_id], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    // messages table dropped — no longer needed
    tx.execute(
        &format!(
            "UPDATE artifacts
         SET run_id = NULL
         WHERE thread_id = ?1
           AND run_id IN (
             SELECT id FROM runs
             WHERE thread_id = ?1
               AND status IN ({TERMINAL_RUN_STATUSES_SQL})
           )"
        ),
        params![thread_id],
    )?;
    // tool_outputs table dropped
    // run_events table dropped
    tx.execute(
        &format!(
            "DELETE FROM approval_requests
         WHERE thread_id = ?1
           AND run_id IN (
             SELECT id FROM runs
             WHERE thread_id = ?1
               AND status IN ({TERMINAL_RUN_STATUSES_SQL})
           )"
        ),
        params![thread_id],
    )?;
    // Review data (file changes → changeset → snapshots) for each terminal run,
    // deleted in the FK-safe order encoded once by `delete_run_review_in`.
    for run_id in &terminal_run_ids {
        delete_run_review_in(&tx, run_id)?;
    }
    // tool_calls table dropped
    let deleted_runs = tx.execute(
        &format!(
            "DELETE FROM runs
         WHERE thread_id = ?1
           AND status IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![thread_id],
    )?;
    tx.commit()?;
    // Event logs live outside SQLite, so remove them only after the database
    // transaction commits. Keeping this paired with run deletion prevents the
    // "clear finished" action from leaving orphaned JSONL files indefinitely.
    for run_id in &terminal_run_ids {
        super::delete_run_events_file(run_id);
        super::clear_run_event_buffer(run_id);
    }
    Ok(deleted_runs)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::schema::SCHEMA;
    use crate::store::{
        AppendRunEventInput, CreateRunInput, CreateThreadInput, CreateWorkspaceInput,
        UpdateRunStatusInput,
    };

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        // Insert threads/runs directly without their workspace parents.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("disable foreign keys");
        conn
    }

    fn insert_thread(conn: &Connection, id: &str, agent_session_id: Option<&str>) {
        conn.execute(
            "INSERT INTO threads
                 (id, workspace_id, mode, title, status, pinned, readonly,
                  agent_session_id, created_at, updated_at)
             VALUES (?1, 'ws', 'chat', 'T', 'active', 0, 0, ?2, 1, 1)",
            params![id, agent_session_id],
        )
        .expect("insert thread");
    }

    fn insert_run(conn: &Connection, id: &str, thread_id: &str, status: &str) {
        conn.execute(
            "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 1)",
            params![id, thread_id, status],
        )
        .expect("insert run");
    }

    fn temp_sessions_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "futureos-reconcile-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("create temp sessions dir");
        dir
    }

    fn touch_session(dir: &Path, session_id: &str) {
        std::fs::write(dir.join(format!("{session_id}.jsonl")), b"{}\n")
            .expect("write session file");
    }

    #[test]
    fn orphans_are_only_threads_with_base_data_whose_jsonl_is_gone() {
        let conn = test_conn();
        let dir = temp_sessions_dir();

        // A: completed run + JSONL present -> kept.
        insert_thread(&conn, "A", None);
        insert_run(&conn, "rA", "A", "completed");
        touch_session(&dir, "A");

        // B: completed run + JSONL missing -> orphan.
        insert_thread(&conn, "B", None);
        insert_run(&conn, "rB", "B", "completed");

        // C: never completed a run (only failed) + JSONL missing -> kept
        // (never produced base data, must not be deleted).
        insert_thread(&conn, "C", None);
        insert_run(&conn, "rC", "C", "failed");

        // D: agent_session_id set, JSONL stored under it -> kept.
        insert_thread(&conn, "D", Some("sessD"));
        insert_run(&conn, "rD", "D", "completed");
        touch_session(&dir, "sessD");

        // E: agent_session_id set, its JSONL missing -> orphan (resolves by
        // agent_session_id, not thread id).
        insert_thread(&conn, "E", Some("sessE"));
        insert_run(&conn, "rE", "E", "completed");
        touch_session(&dir, "E"); // decoy under the thread id must not save it

        let mut orphans = orphan_thread_ids(&conn, &dir).expect("reconcile");
        orphans.sort();

        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(orphans, vec!["B".to_string(), "E".to_string()]);
    }

    fn run_status(conn: &Connection, id: &str) -> String {
        conn.query_row("SELECT status FROM runs WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .expect("read run status")
    }

    #[test]
    fn converge_orphan_runs_cancels_non_terminal_and_cascades() {
        let mut conn = test_conn();
        insert_thread(&conn, "T", None);
        // Non-terminal orphans: a plain running run, plus one waiting on approval.
        insert_run(&conn, "run_running", "T", "running");
        insert_run(&conn, "run_waiting", "T", "waiting_approval");
        // Terminal runs must be left untouched.
        insert_run(&conn, "run_done", "T", "completed");
        insert_run(&conn, "run_cancelled", "T", "cancelled");

        conn.execute(
            "INSERT INTO approval_requests (id, thread_id, run_id, kind, status, title, created_at, updated_at)
             VALUES ('ap', 'T', 'run_waiting', 'shell', 'pending', 't', 1, 1)",
            [],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let cancelled = converge_orphan_runs_tx(&tx, 42).unwrap();
        tx.commit().unwrap();

        assert_eq!(cancelled, 2, "only the two non-terminal runs are cancelled");
        assert_eq!(run_status(&conn, "run_running"), "cancelled");
        assert_eq!(run_status(&conn, "run_waiting"), "cancelled");
        // Terminal runs preserved.
        assert_eq!(run_status(&conn, "run_done"), "completed");
        assert_eq!(run_status(&conn, "run_cancelled"), "cancelled");
        // Cascades fired.
        let ap_status: String = conn
            .query_row(
                "SELECT status FROM approval_requests WHERE id = 'ap'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ap_status, "cancelled");
        // (tool_calls cascade dropped — the table no longer exists; converge
        // only cancels the run + its pending approvals now.)
    }

    #[test]
    fn clear_finished_runs_removes_event_log() {
        let _home = HomeGuard::new("clear-finished-run-events");
        crate::store::initialize_app_store().expect("initialize store");
        let workspace = crate::store::create_workspace(CreateWorkspaceInput {
            name: Some("test".to_string()),
            path: PathBuf::from(std::env::var("HOME").expect("test home"))
                .join("workspace")
                .display()
                .to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = crate::store::create_thread(CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some("test".to_string()),
            workspace_id: Some(workspace.id),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        let run = crate::store::create_run(CreateRunInput {
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::store::append_run_event(AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "text_chunk".to_string(),
            payload: Some(r#"{"text":"hello"}"#.to_string()),
            sequence: 1,
        })
        .expect("append event");
        let log_path = PathBuf::from(
            crate::store::app_data_path()
                .expect("app data path")
                .app_dir,
        )
        .join("run_events")
        .join(format!("{}.jsonl", run.id));
        assert!(log_path.exists(), "event log should exist before cleanup");

        crate::store::update_run_status_if_active(UpdateRunStatusInput {
            run_id: run.id,
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("complete run");
        assert_eq!(clear_finished_runs(&thread.id).expect("clear runs"), 1);
        assert!(
            !log_path.exists(),
            "event log should be removed with its run"
        );
    }

    #[test]
    fn orphan_image_dirs_keeps_only_live_threads() {
        let conn = test_conn();
        // Active thread -> its image dir is kept.
        insert_thread(&conn, "live", None);
        // Soft-deleted thread -> swept (there is no soft-delete undo).
        insert_thread(&conn, "dead", None);
        conn.execute(
            "UPDATE threads SET status = 'deleted' WHERE id = 'dead'",
            [],
        )
        .expect("soft-delete thread");
        // "ghost" has no thread row at all -> swept.

        let root = temp_sessions_dir(); // a unique, freshly-created temp dir
        for tid in ["live", "dead", "ghost"] {
            std::fs::create_dir_all(root.join(tid).join("thumb")).expect("create image dir");
        }
        // A stray file at the root must be ignored (only directories are dirs).
        std::fs::write(root.join("stray.txt"), b"x").expect("write stray file");

        let mut names: Vec<String> = orphan_image_dirs(&conn, &root)
            .expect("sweep")
            .into_iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .collect();
        names.sort();

        std::fs::remove_dir_all(&root).ok();
        assert_eq!(names, vec!["dead".to_string(), "ghost".to_string()]);
    }

    fn insert_workspace(conn: &Connection, id: &str, kind: &str, deleted: bool) {
        conn.execute(
            "INSERT INTO workspaces
                 (id, name, kind, path, cleanup_status, created_at, updated_at, deleted_at)
             VALUES (?1, 'W', ?2, '/tmp/ws', 'active', 1, 1, ?3)",
            params![id, kind, if deleted { Some(1_i64) } else { None }],
        )
        .expect("insert workspace");
    }

    /// review/<wsid> reclamation keeps every live workspace's repo — a user
    /// workspace is NEVER swept (item 3) — and reclaims only absent or
    /// soft-deleted workspaces.
    #[test]
    fn orphan_review_repos_keeps_live_workspaces() {
        let conn = test_conn();
        // Live user workspace -> kept no matter what.
        insert_workspace(&conn, "user_ws", "user", false);
        // Live temporary workspace -> kept.
        insert_workspace(&conn, "temp_ws", "temporary", false);
        // Soft-deleted workspace -> reclaimable.
        insert_workspace(&conn, "dead_ws", "user", true);
        // "ghost_ws" has no row at all -> reclaimable.

        let root = temp_sessions_dir();
        for wsid in ["user_ws", "temp_ws", "dead_ws", "ghost_ws"] {
            std::fs::create_dir_all(root.join(wsid).join(".git")).expect("create review repo");
        }

        let mut names: Vec<String> = orphan_subdirs(&root, &live_workspace_ids(&conn).unwrap())
            .expect("sweep")
            .into_iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .collect();
        names.sort();

        std::fs::remove_dir_all(&root).ok();
        assert_eq!(names, vec!["dead_ws".to_string(), "ghost_ws".to_string()]);
    }
}
