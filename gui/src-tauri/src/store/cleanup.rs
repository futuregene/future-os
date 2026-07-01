use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use super::db::connect;
use super::records::ThreadCleanupSummary;
use super::status::TERMINAL_RUN_STATUSES_SQL;
use super::util::{count_workspace_files, loaded, now_millis};
use super::{delete_thread, get_thread, get_workspace};

/// Startup reconciliation: soft-delete active threads whose agent base data
/// (the session JSONL the agent writes under `~/.future/agent/sessions/`) has
/// been removed out from under the GUI — e.g. via the TUI/CLI `delete_session`
/// or a manual delete.
///
/// The agent treats that JSONL as the source of truth for a conversation's
/// context and reloads it on a cold start; the GUI keeps only a rendered mirror
/// (text + events), which cannot losslessly rebuild the agent's native message
/// structure (tool calls, tool results, thinking). So when the base file is gone
/// there is no faithful recovery — we delete-to-match, soft-deleting the GUI
/// thread so the two sides stay consistent instead of the model silently
/// "forgetting" a conversation the UI still shows.
///
/// Only threads with at least one `completed` run are considered: the agent
/// persists the JSONL on the successful-turn path *before* it signals
/// completion, so a completed run proves base data was written at some point. A
/// missing file then means external deletion, not a conversation that simply
/// hasn't produced base data yet (which must never be deleted). Runs once at
/// startup — mid-session the agent still holds the context in memory, so drift
/// only surfaces on the next cold start. Returns the number of threads
/// soft-deleted.
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
        // Soft delete (recoverable): also marks temp chat workspaces for cleanup.
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

pub fn cancel_stale_approval_requests() -> Result<usize, crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    // Cancel every non-terminal run that owns a pending approval — the same set
    // the second UPDATE cancels — so a run and its approval never end up in
    // mismatched states (e.g. a still-`running` run whose approval was cancelled).
    tx.execute(
        &format!(
            "UPDATE runs
         SET status = 'cancelled',
             error_message = 'Pending approval was cancelled because FutureOS restarted.',
             ended_at = COALESCE(ended_at, ?1),
             updated_at = ?1
         WHERE status NOT IN ({TERMINAL_RUN_STATUSES_SQL})
           AND id IN (
             SELECT run_id
             FROM approval_requests
             WHERE status = 'pending'
               AND run_id IS NOT NULL
           )"
        ),
        params![now],
    )?;
    let changed = tx.execute(
        "UPDATE approval_requests
             SET status = 'cancelled',
                 decision_note = 'Cancelled because FutureOS restarted.',
                 decided_at = ?1,
                 updated_at = ?1
             WHERE status = 'pending'",
        params![now],
    )?;
    tx.commit()?;
    Ok(changed)
}

pub fn clear_finished_runs(thread_id: &str) -> Result<usize, crate::AppError> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    tx.execute(
        &format!(
            "UPDATE messages
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
    tx.execute(
        &format!(
            "DELETE FROM tool_outputs
         WHERE tool_call_id IN (
           SELECT tc.id
           FROM tool_calls tc
           JOIN runs r ON r.id = tc.run_id
           WHERE r.thread_id = ?1
             AND r.status IN ({TERMINAL_RUN_STATUSES_SQL})
         )"
        ),
        params![thread_id],
    )?;
    tx.execute(
        &format!(
            "DELETE FROM run_events
         WHERE run_id IN (
           SELECT id FROM runs
           WHERE thread_id = ?1
             AND status IN ({TERMINAL_RUN_STATUSES_SQL})
         )"
        ),
        params![thread_id],
    )?;
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
    tx.execute(
        &format!(
            "DELETE FROM review_file_changes
         WHERE changeset_id IN (
           SELECT c.id
           FROM review_changesets c
           JOIN runs r ON r.id = c.run_id
           WHERE r.thread_id = ?1
             AND r.status IN ({TERMINAL_RUN_STATUSES_SQL})
         )"
        ),
        params![thread_id],
    )?;
    tx.execute(
        &format!(
            "DELETE FROM review_changesets
         WHERE thread_id = ?1
           AND run_id IN (
             SELECT id FROM runs
             WHERE thread_id = ?1
               AND status IN ({TERMINAL_RUN_STATUSES_SQL})
           )"
        ),
        params![thread_id],
    )?;
    // review_snapshots is referenced by review_changesets, so it is deleted
    // after the changesets above to avoid orphan snapshot rows.
    tx.execute(
        &format!(
            "DELETE FROM review_snapshots
         WHERE run_id IN (
           SELECT id FROM runs
           WHERE thread_id = ?1
             AND status IN ({TERMINAL_RUN_STATUSES_SQL})
         )"
        ),
        params![thread_id],
    )?;
    tx.execute(
        &format!(
            "DELETE FROM tool_calls
         WHERE run_id IN (
           SELECT id FROM runs
           WHERE thread_id = ?1
             AND status IN ({TERMINAL_RUN_STATUSES_SQL})
         )"
        ),
        params![thread_id],
    )?;
    let deleted_runs = tx.execute(
        &format!(
            "DELETE FROM runs
         WHERE thread_id = ?1
           AND status IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![thread_id],
    )?;
    tx.commit()?;
    Ok(deleted_runs)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::*;
    use crate::store::schema::SCHEMA;

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
}
