use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use super::db::connect;
use super::records::ThreadCleanupSummary;
use super::status::TERMINAL_RUN_STATUSES_SQL;

/// A run that was cancelled by startup convergence after a GUI crash.
#[allow(dead_code)]
pub struct InterruptedRun {
    pub run_id: String,
    pub thread_id: String,
    pub session_id: String,
}

/// A run that is still non-terminal, with the session the agent knows it by.
/// Consumed by the runtime watchdog (`agent_bridge::spawn_active_run_watchdog`),
/// which reconciles rows whose owning pipeline/collector never finalized them.
pub struct ActiveRun {
    pub run_id: String,
    pub thread_id: String,
    pub session_id: String,
    pub created_at: i64,
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

/// Every run that has not reached a terminal state (`running` /
/// `waiting_approval`), ordered oldest first. The runtime watchdog uses this to
/// find rows whose owning pipeline or collector never settled them and
/// reconcile each against the agent's authoritative state. `session_id`
/// resolution mirrors [`list_interrupted_runs`].
pub fn list_active_runs() -> Result<Vec<ActiveRun>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT r.id, r.thread_id, r.created_at,
                COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id) AS session_id
         FROM runs r
         JOIN threads t ON t.id = r.thread_id
         WHERE r.status NOT IN ({TERMINAL_RUN_STATUSES_SQL})
         ORDER BY r.created_at"
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok(ActiveRun {
            run_id: row.get(0)?,
            thread_id: row.get(1)?,
            created_at: row.get(2)?,
            session_id: row.get(3)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// Reset a run back to "running", clearing interrupted/error markers — but only
/// when it is still in the interrupted state startup convergence created
/// (`status='cancelled' AND error_type='interrupted'`). Returns whether a row
/// changed. The guard is a compare-and-set so a run the user already aborted or
/// that another path settled (a terminal state that is NOT the interrupted
/// marker) is left untouched and the caller skips spawning a collector for it.
/// This honors the "no unguarded status writer" rule (see desktop/CLAUDE.md #11).
pub fn reanimate_run(run_id: &str) -> Result<bool, crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    let affected = conn.execute(
        "UPDATE runs
         SET status = 'running',
             error_message = NULL,
             error_type = NULL,
             ended_at = NULL,
             updated_at = ?1
         WHERE id = ?2
           AND status = 'cancelled'
           AND error_type = 'interrupted'",
        params![now, run_id],
    )?;
    Ok(affected > 0)
}

/// Apply the Agent journal's authoritative terminal state to a run recovered
/// after GUI restart. Unknown states remain interrupted rather than being
/// guessed as successful.
pub fn settle_interrupted_run_from_agent(
    run_id: &str,
    agent_state: &str,
    error: Option<&str>,
) -> Result<(), crate::AppError> {
    let Some((status, error_type, default_message)) = agent_terminal_settlement(agent_state) else {
        return Ok(());
    };
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE runs
         SET status = ?1,
             error_message = ?2,
             error_type = ?3,
             ended_at = COALESCE(ended_at, ?4),
             updated_at = ?4
         WHERE id = ?5
           AND error_type = 'interrupted'",
        params![status, error.or(default_message), error_type, now, run_id],
    )?;
    Ok(())
}

fn agent_terminal_settlement(
    agent_state: &str,
) -> Option<(&'static str, Option<&'static str>, Option<&'static str>)> {
    Some(match agent_state {
        "completed" => ("completed", None, None),
        "cancelled" => ("cancelled", Some("cancelled"), Some("Run was cancelled.")),
        "error" => (
            "failed",
            Some("agent_error"),
            Some("Future Agent run failed."),
        ),
        "incomplete" => (
            "failed",
            Some("stream_interrupted"),
            Some("Future Agent response ended before a clean terminal."),
        ),
        _ => return None,
    })
}
use super::util::{count_workspace_files, loaded, now_millis};
use super::{delete_thread, get_thread, get_workspace};

/// Startup reconciliation: delete active threads whose agent session has been
/// removed out from under the GUI — e.g. via the TUI/CLI `delete_session` or a
/// manual delete. The authoritative source is the agent's FILENAME-ONLY session
/// enumeration (`list_session_ids` RPC): ids are read from the session file
/// names without touching file contents, so a session whose journal is
/// momentarily unreadable or corrupt is still reported as live and is never
/// mistaken for a deleted session. The GUI no longer probes `{id}.jsonl`
/// filenames itself, so it stays correct regardless of how or where the agent
/// persists sessions.
///
/// The agent treats the session journal as the source of truth for a
/// conversation's context and reloads it on a cold start; the GUI keeps only a
/// rendered mirror (text + events), which cannot losslessly rebuild the agent's
/// native message structure (tool calls, tool results, thinking). So when the
/// session is gone there is no faithful recovery — we delete-to-match,
/// hard-deleting the GUI thread so the two sides stay consistent instead of the
/// model silently "forgetting" a conversation the UI still shows.
///
/// Only threads with at least one `completed` run are considered: the agent
/// persists the journal on the successful-run path *before* it signals
/// completion, so a completed run proves base data was written at some point. A
/// missing session then means external deletion, not a conversation that simply
/// hasn't produced base data yet (which must never be deleted).
///
/// An unreachable agent is ambiguous — the pass is skipped rather than
/// treating unknown state as "everything was deleted". Runs once at startup,
/// after the agent has had time to come up. Returns the number of threads
/// deleted.
pub async fn reconcile_orphan_sessions() -> Result<usize, crate::AppError> {
    // The bundled agent may still be booting when this pass runs; retry
    // briefly before concluding it is down.
    let mut live_sessions: Option<HashSet<String>> = None;
    for _ in 0..3 {
        match crate::agent_bridge::list_agent_session_ids().await {
            Ok(ids) => {
                live_sessions = Some(ids);
                break;
            }
            Err(crate::AppError::AgentUnavailable(_)) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(error) => {
                eprintln!("FutureOS: orphan-session reconcile skipped: {error}");
                return Ok(0);
            }
        }
    }
    let Some(live_sessions) = live_sessions else {
        eprintln!("FutureOS: orphan-session reconcile skipped: agent unreachable");
        return Ok(0);
    };

    let orphans = {
        let conn = connect()?;
        orphan_thread_ids(&conn, &live_sessions)?
    };
    for thread_id in &orphans {
        // Hard delete: the agent's session (source of truth) is already gone,
        // so purge the GUI mirror and its child rows too. Also marks temp chat
        // workspaces for cleanup.
        delete_thread(thread_id)?;
    }
    Ok(orphans.len())
}

/// Decide which active threads have lost their agent base data. Split out from
/// the deletion so the (subtle) detection rules can be unit-tested against an
/// in-memory DB and a fixture session set.
fn orphan_thread_ids(
    conn: &Connection,
    live_sessions: &HashSet<String>,
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
        if !live_sessions.contains(session_id) {
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
/// named after the thread id, but older ones may still use the session id.
fn live_chat_workspace_dir_ids(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    const SQL: &str = "SELECT id FROM threads WHERE status != 'deleted'
         UNION
         SELECT agent_session_id FROM threads
         WHERE agent_session_id IS NOT NULL AND agent_session_id != ''
           AND status != 'deleted'";
    let mut stmt = conn.prepare(SQL)?;
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
        // Lossy: a non-UTF8 name can never match a live id (SQLite TEXT is
        // UTF-8), so the worst case is reclaiming a dir no row could own.
        let name = entry.file_name().to_string_lossy().into_owned();
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
    const ARTIFACT_COUNT_SQL: &str = "SELECT COUNT(*)
             FROM artifacts
             WHERE workspace_id = ?1
               AND (thread_id = ?2 OR ?3 = 'workspace')
               AND deleted_at IS NULL";
    let artifact_count = {
        let (ws, tid, mode) = (&workspace.id, &thread.id, &thread.mode);
        conn.query_row(ARTIFACT_COUNT_SQL, params![ws, tid, mode], |row| row.get(0))?
    };
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

/// Hide terminal runs from the Runs panel while retaining their local metadata
/// and Agent-owned event history for transcript inspection and deep links.
pub fn archive_finished_runs(thread_id: &str) -> Result<usize, crate::AppError> {
    let mut conn = connect()?;
    if !super::runs::run_archive_supported(&conn)? {
        return Err(
            "Run archiving is unavailable because the local database upgrade did not complete."
                .into(),
        );
    }
    let tx = conn.transaction()?;
    let now = now_millis();
    let archived_runs = tx.execute(
        &format!(
            "UPDATE runs
             SET archived_at = ?2, updated_at = ?2
             WHERE thread_id = ?1
               AND archived_at IS NULL
               AND status IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![thread_id, now],
    )?;
    tx.commit()?;
    Ok(archived_runs)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
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

    #[test]
    fn agent_terminal_states_preserve_non_success_outcomes() {
        assert_eq!(
            agent_terminal_settlement("completed").map(|value| value.0),
            Some("completed")
        );
        assert_eq!(
            agent_terminal_settlement("error").map(|value| value.0),
            Some("failed")
        );
        assert_eq!(
            agent_terminal_settlement("cancelled").map(|value| value.0),
            Some("cancelled")
        );
        assert_eq!(
            agent_terminal_settlement("incomplete").map(|value| value.1),
            Some(Some("stream_interrupted"))
        );
        assert!(agent_terminal_settlement("interrupted_by_restart").is_none());
        assert!(agent_terminal_settlement("future-state").is_none());
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

    #[test]
    fn orphans_are_only_threads_with_base_data_whose_session_is_gone() {
        let conn = test_conn();

        // The live session set as reported by the agent's list_sessions RPC.
        let live_sessions: HashSet<String> =
            ["A", "sessD"].into_iter().map(str::to_string).collect();

        // A: completed run + session live -> kept.
        insert_thread(&conn, "A", None);
        insert_run(&conn, "rA", "A", "completed");

        // B: completed run + session gone -> orphan.
        insert_thread(&conn, "B", None);
        insert_run(&conn, "rB", "B", "completed");

        // C: never completed a run (only failed) + session gone -> kept
        // (never produced base data, must not be deleted).
        insert_thread(&conn, "C", None);
        insert_run(&conn, "rC", "C", "failed");

        // D: agent_session_id set, session live under it -> kept.
        insert_thread(&conn, "D", Some("sessD"));
        insert_run(&conn, "rD", "D", "completed");

        // E: agent_session_id set, its session gone -> orphan (resolves by
        // agent_session_id, not thread id).
        insert_thread(&conn, "E", Some("sessE"));
        insert_run(&conn, "rE", "E", "completed");

        let mut orphans = orphan_thread_ids(&conn, &live_sessions).expect("reconcile");
        orphans.sort();
        assert_eq!(orphans, vec!["B".to_string(), "E".to_string()]);
    }

    #[test]
    fn archive_finished_runs_keeps_event_log() {
        let _home = HomeGuard::new("archive-finished-run-events");
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
            id: None,
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
        // Disk writes are async (single writer thread) — flush before
        // asserting on the file.
        crate::store::flush_run_event_log_for_test(&run.id);
        let log_path = PathBuf::from(
            crate::store::app_data_path()
                .expect("app data path")
                .app_dir,
        )
        .join("run_events")
        .join(format!("{}.jsonl", run.id));
        assert!(log_path.exists(), "event log should exist before cleanup");

        crate::store::update_run_status_if_active(UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("complete run");
        assert_eq!(archive_finished_runs(&thread.id).expect("archive runs"), 1);
        assert!(
            log_path.exists(),
            "event log must remain available after archiving"
        );
        assert!(crate::store::get_run(&run.id)
            .expect("get run")
            .expect("run retained")
            .archived_at
            .is_some());
    }

    #[test]
    fn archive_finished_runs_errors_when_archive_migration_did_not_complete() {
        let _home = HomeGuard::new("archive-runs-unsupported");
        // Build the app DB manually with a legacy `runs` table (no
        // `archived_at` column), as an older build that never completed the
        // optional archive migration would leave it. The command must report
        // the unavailable error instead of touching a table it cannot archive.
        let app_dir = crate::store::app_data_path()
            .expect("app data path")
            .app_dir;
        std::fs::create_dir_all(&app_dir).expect("create app dir");
        let conn = Connection::open(PathBuf::from(&app_dir).join("app.db")).expect("open app db");
        conn.execute_batch(
            "CREATE TABLE runs (
                 id TEXT PRIMARY KEY,
                 thread_id TEXT NOT NULL,
                 trigger_message_id TEXT,
                 status TEXT NOT NULL,
                 model_provider TEXT,
                 model_id TEXT,
                 started_at INTEGER,
                 ended_at INTEGER,
                 error_message TEXT,
                 error_type TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );",
        )
        .expect("create legacy runs table");
        drop(conn);

        let error = archive_finished_runs("thread_x").expect_err("archive unavailable");
        assert!(error.to_string().contains("unavailable"));
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

    #[test]
    fn reanimate_is_guarded_and_late_settle_cannot_overwrite_an_abort() {
        let _home = HomeGuard::new("reanimate-cas-guard");
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

        // Startup-convergence shape: cancelled + error_type='interrupted'.
        let interrupted = crate::store::create_run(CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::store::update_run_status_if_active(UpdateRunStatusInput {
            run_id: interrupted.id.clone(),
            status: "cancelled".to_string(),
            error_message: Some("Interrupted because FutureOS restarted.".to_string()),
            error_type: Some("interrupted".to_string()),
        })
        .expect("mark interrupted");

        // User-abort shape: cancelled + error_type='abort_requested'.
        let aborted = crate::store::create_run(CreateRunInput {
            id: None,
            thread_id: thread.id,
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");
        crate::store::update_run_status_if_active(UpdateRunStatusInput {
            run_id: aborted.id.clone(),
            status: "cancelled".to_string(),
            error_message: Some("Cancelled because the run was terminated.".to_string()),
            error_type: Some("abort_requested".to_string()),
        })
        .expect("mark aborted");

        // reanimate flips the interrupted run back to running...
        assert!(
            reanimate_run(&interrupted.id).expect("reanimate interrupted"),
            "interrupted run must be reanimated"
        );
        assert_eq!(
            crate::store::get_run(&interrupted.id)
                .expect("get")
                .expect("row")
                .status,
            "running"
        );
        // ...but the CAS guard leaves an already-aborted run untouched.
        assert!(
            !reanimate_run(&aborted.id).expect("reanimate aborted"),
            "an aborted run must NOT be reanimated"
        );
        let aborted_row = crate::store::get_run(&aborted.id)
            .expect("get")
            .expect("row");
        assert_eq!(aborted_row.status, "cancelled");
        assert_eq!(aborted_row.error_type.as_deref(), Some("abort_requested"));

        // The H2 regression: a reanimated run that the user then aborts must keep
        // its cancelled state when a late completion tries to settle it. The
        // compare-and-set writer refuses to touch the terminal row.
        crate::store::update_run_status_if_active(UpdateRunStatusInput {
            run_id: interrupted.id.clone(),
            status: "cancelled".to_string(),
            error_message: Some("Cancelled because the run was terminated.".to_string()),
            error_type: Some("abort_requested".to_string()),
        })
        .expect("user abort");
        let changed = crate::store::update_run_status_if_active(UpdateRunStatusInput {
            run_id: interrupted.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("late settle");
        assert!(!changed, "late completion must not overwrite the abort");
        let final_row = crate::store::get_run(&interrupted.id)
            .expect("get")
            .expect("row");
        assert_eq!(final_row.status, "cancelled");
        assert_eq!(final_row.error_type.as_deref(), Some("abort_requested"));
    }

    // ── remaining API surface ───────────────────────────────────────────────

    use crate::store::db::test_support::guarded_conn;

    /// Full-graph fixture: ws1 (temporary, files on disk) with threads and
    /// runs in assorted states. `sessions`: (thread, agent_session_id).
    fn seed_full(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, agent_session_id, created_at, updated_at
             ) VALUES
                 ('t_sess', 'ws1', 'chat', 'T', 'sess_live', 1, 1),
                 ('t_blank', 'ws1', 'chat', 'T', '  ', 1, 1),
                 ('t_none', 'ws1', 'chat', 'T', NULL, 1, 1);
             INSERT INTO runs (
                 id, thread_id, status, error_type, created_at, updated_at
             ) VALUES
                 ('r_int', 't_sess', 'cancelled', 'interrupted', 1, 1),
                 ('r_run', 't_blank', 'running', NULL, 2, 2),
                 ('r_wait', 't_none', 'waiting_approval', NULL, 3, 3),
                 ('r_done', 't_none', 'completed', NULL, 4, 4);",
        )
        .expect("seed full graph");
    }

    #[test]
    fn interrupted_and_active_run_listings_resolve_session_ids() {
        let (_home, conn) = guarded_conn("cleanup_lists");
        seed_full(&conn);
        drop(conn);

        let interrupted = list_interrupted_runs().expect("interrupted");
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].run_id, "r_int");
        assert_eq!(interrupted[0].session_id, "sess_live");

        let active = list_active_runs().expect("active");
        let ids: Vec<&str> = active.iter().map(|run| run.run_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["r_run", "r_wait"],
            "non-terminal only (cancelled r_int is terminal), oldest first"
        );
        // Blank session id falls back to the thread id.
        assert_eq!(active[0].session_id, "t_blank");
        assert_eq!(active[1].session_id, "t_none");
    }

    #[test]
    fn settle_interrupted_run_applies_agent_terminal_state() {
        let (_home, conn) = guarded_conn("cleanup_settle");
        seed_full(&conn);
        drop(conn);

        // Unknown agent states are a no-op.
        settle_interrupted_run_from_agent("r_int", "still_running", None).expect("no-op");
        let row = crate::store::get_run("r_int").expect("get").expect("some");
        assert_eq!(row.status, "cancelled");

        // A known terminal state settles the interrupted row, preserving a
        // caller-supplied error message over the default.
        settle_interrupted_run_from_agent("r_int", "error", Some("agent said no")).expect("settle");
        let row = crate::store::get_run("r_int").expect("get").expect("some");
        assert_eq!(row.status, "failed");
        assert_eq!(row.error_type.as_deref(), Some("agent_error"));
        assert_eq!(row.error_message.as_deref(), Some("agent said no"));
        assert!(row.ended_at.is_some());

        // A row without the interrupted marker is left alone.
        settle_interrupted_run_from_agent("r_done", "cancelled", None).expect("settle");
        let row = crate::store::get_run("r_done").expect("get").expect("some");
        assert_eq!(row.status, "completed");
    }

    #[test]
    fn reconcile_orphan_images_removes_dead_thread_dirs() {
        let (_home, conn) = guarded_conn("cleanup_images");
        seed_full(&conn);
        drop(conn);

        let root = crate::store::app_images_root().expect("images root");
        // Missing root → 0.
        assert_eq!(reconcile_orphan_images().expect("reconcile"), 0);

        std::fs::create_dir_all(root.join("t_sess")).expect("live dir");
        std::fs::create_dir_all(root.join("ghost")).expect("orphan dir");
        let removed = reconcile_orphan_images().expect("reconcile");
        assert_eq!(removed, 1);
        assert!(root.join("t_sess").is_dir(), "live thread's dir kept");
        assert!(!root.join("ghost").exists(), "orphan dir reclaimed");
    }

    #[test]
    fn reconcile_orphan_chat_workspaces_and_review_repos() {
        let (_home, conn) = guarded_conn("cleanup_dirs");
        seed_full(&conn);
        drop(conn);

        // Chat workspaces: live names are thread ids AND session ids.
        let chat_root = std::path::PathBuf::from(std::env::var("HOME").expect("home"))
            .join(".future/workspaces/chat");
        std::fs::create_dir_all(chat_root.join("t_sess")).expect("thread dir");
        std::fs::create_dir_all(chat_root.join("sess_live")).expect("session dir");
        std::fs::create_dir_all(chat_root.join("gone")).expect("orphan dir");
        assert_eq!(reconcile_orphan_chat_workspaces().expect("reconcile"), 1);
        assert!(chat_root.join("t_sess").is_dir());
        assert!(chat_root.join("sess_live").is_dir());
        assert!(!chat_root.join("gone").exists());

        // Review repos: keyed by workspace id.
        let review_root = std::path::PathBuf::from(std::env::var("HOME").expect("home"))
            .join(".future/app/review");
        std::fs::create_dir_all(review_root.join("ws1")).expect("live repo");
        std::fs::create_dir_all(review_root.join("ws_ghost")).expect("orphan repo");
        assert_eq!(reconcile_orphan_review_repos().expect("reconcile"), 1);
        assert!(review_root.join("ws1").is_dir());
        assert!(!review_root.join("ws_ghost").exists());
    }

    #[cfg(unix)]
    #[test]
    fn orphan_subdirs_sweep_non_utf8_names_as_orphans() {
        use std::os::unix::ffi::OsStrExt;
        let root = temp_sessions_dir();
        let non_utf8 = std::ffi::OsStr::from_bytes(b"bad-\xff-name");
        // Branch-free: APFS rejects non-UTF8 names (EILSEQ), Linux allows them.
        let created = std::fs::create_dir_all(root.join(non_utf8)).is_ok();
        let orphans = orphan_subdirs(&root, &HashSet::new()).expect("scan");
        // A non-UTF8 name never matches a live (UTF-8) id, so when the
        // platform allowed creating it, it is reclaimed.
        assert_eq!(orphans.len(), usize::from(created));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn thread_cleanup_summary_counts_artifacts_and_files() {
        // A temporary workspace whose path holds two real files.
        let files_dir =
            std::env::temp_dir().join(format!("futureos-cleanup-summary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files_dir);
        std::fs::create_dir_all(&files_dir).expect("create files dir");
        std::fs::write(files_dir.join("a.txt"), b"a").expect("write");
        std::fs::write(files_dir.join("b.txt"), b"b").expect("write");

        let (_home, conn) = guarded_conn("cleanup_summary");
        conn.execute_batch(&format!(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '{}', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 1, 1),
                      ('t_other', 'ws1', 'chat', 'T2', 1, 1);
             INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, artifact_type, created_at, updated_at
             ) VALUES
                 ('a1', 'ws1', 't1', 'A', 'document', 1, 1),
                 ('a2', 'ws1', 't_other', 'B', 'document', 1, 1);",
            files_dir.display()
        ))
        .expect("seed");
        drop(conn);

        let summary = get_thread_cleanup_summary("t1").expect("summary");
        assert_eq!(summary.workspace_kind, "temporary");
        // Chat mode: only the thread's own artifact counts.
        assert_eq!(summary.artifact_count, 1);
        assert_eq!(summary.workspace_file_count, 2);
        let _ = std::fs::remove_dir_all(&files_dir);

        assert!(get_thread_cleanup_summary("ghost").is_err());
    }

    #[test]
    fn thread_cleanup_summary_user_workspace_counts_all_artifacts_no_files() {
        let (_home, conn) = guarded_conn("cleanup_summary_user");
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'user', '/tmp/userws', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'workspace', 'T', 1, 1),
                      ('t_other', 'ws1', 'workspace', 'T2', 1, 1);
             INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, artifact_type, created_at, updated_at
             ) VALUES
                 ('a1', 'ws1', 't1', 'A', 'document', 1, 1),
                 ('a2', 'ws1', 't_other', 'B', 'document', 1, 1);",
        )
        .expect("seed");
        drop(conn);

        let summary = get_thread_cleanup_summary("t1").expect("summary");
        // Workspace mode: every artifact of the workspace counts…
        assert_eq!(summary.artifact_count, 2);
        // …but a user workspace's files are never walked.
        assert_eq!(summary.workspace_file_count, 0);
    }

    // ── reconcile_orphan_sessions against the shared mock agent ─────────────

    use crate::commands::agent_mock::{
        ensure_mock_agent, mock_agent_lock, script_mock_agent, MockScript,
    };

    #[tokio::test]
    async fn reconcile_deletes_orphans_reported_gone_by_the_agent() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        script_mock_agent(MockScript {
            down: false,
            fail_list_session_ids: false,
            session_ids: vec!["sess_live".to_string()],
            ..Default::default()
        });

        let (_home, conn) = guarded_conn("reconcile_ok");
        seed_full(&conn);
        drop(conn);

        // t_none completed a run and its session (its own thread id) is not
        // live → hard-deleted. t_sess/t_blank have no completed run → kept.
        let deleted = reconcile_orphan_sessions().await.expect("reconcile");
        assert_eq!(deleted, 1);
        assert!(crate::store::get_thread("t_none").expect("get").is_none());
        assert!(crate::store::get_thread("t_sess").expect("get").is_some());

        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn reconcile_skips_when_enumeration_fails() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        script_mock_agent(MockScript {
            down: false,
            fail_list_session_ids: true,
            session_ids: vec![],
            ..Default::default()
        });

        let (_home, conn) = guarded_conn("reconcile_fail");
        seed_full(&conn);
        drop(conn);

        let deleted = reconcile_orphan_sessions().await.expect("reconcile");
        assert_eq!(deleted, 0, "a failed enumeration deletes nothing");
        assert!(crate::store::get_thread("t_none").expect("get").is_some());

        script_mock_agent(MockScript::default());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconcile_skips_when_the_agent_is_unreachable() {
        let _lock = mock_agent_lock();
        ensure_mock_agent();
        // Down mode: every RPC answers Unavailable, so connect (or the first
        // command on the latched channel) surfaces AgentUnavailable and the
        // pass retries, then skips.
        script_mock_agent(MockScript {
            down: true,
            ..Default::default()
        });

        let (_home, conn) = guarded_conn("reconcile_down");
        seed_full(&conn);
        drop(conn);

        let deleted = reconcile_orphan_sessions().await.expect("reconcile");
        assert_eq!(deleted, 0, "an unreachable agent deletes nothing");
        assert!(crate::store::get_thread("t_none").expect("get").is_some());

        script_mock_agent(MockScript::default());
    }
}
