//! Real database fixtures shared by host core and entry integration tests.
pub use super::db::test_support::memory_conn;
pub use super::db::PooledConnection;
use rusqlite::Connection;
use std::path::Path;

pub fn open_schema_connection() -> PooledConnection {
    let conn = super::db::connect().expect("connect");
    super::db::apply_schema(&conn).expect("apply schema");
    conn
}
/// Create a real on-disk workspace row rooted under `dir` (created on disk so
/// `is_dir()` checks pass).
pub fn seed_workspace(dir: &Path, name: &str) -> crate::store::WorkspaceRecord {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).expect("create workspace dir");
    crate::store::create_workspace(crate::store::CreateWorkspaceInput {
        name: Some(name.to_string()),
        path: path.display().to_string(),
        description: None,
        create_directory: Some(false),
    })
    .expect("create workspace")
}

/// Create a workspace-mode thread bound to `agent_session_id`.
pub fn seed_thread(
    workspace_id: &str,
    agent_session_id: Option<&str>,
) -> crate::store::ThreadRecord {
    crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "workspace".to_string(),
        title: Some("test thread".to_string()),
        workspace_id: Some(workspace_id.to_string()),
        workspace_path: None,
        workspace_name: None,
        agent_session_id: agent_session_id.map(str::to_string),
    })
    .expect("create thread")
}

/// Create a thread the background title task may name: bound to an Agent
/// session (like an imported worker conversation) and carrying the title seed
/// the importer records for a derived session. Without the seed the thread is
/// a conversation that arrived with a name of its own, which is never
/// auto-titled — see `store::create_thread` and `store::seed_thread_auto_title`.
pub fn seed_titleable_thread(
    workspace_id: &str,
    agent_session_id: &str,
) -> crate::store::ThreadRecord {
    let thread = seed_thread(workspace_id, Some(agent_session_id));
    assert!(
        crate::store::seed_thread_auto_title(&thread.id, &thread.title).expect("seed auto title"),
        "a freshly seeded thread must accept its own title as the seed"
    );
    crate::store::get_thread(&thread.id)
        .expect("reload seeded thread")
        .expect("seeded thread exists")
}

/// Create a (running) run row for the thread.
pub fn seed_run(thread_id: &str) -> crate::store::RunRecord {
    crate::store::create_run(crate::store::CreateRunInput {
        id: None,
        thread_id: thread_id.to_string(),
        trigger_message_id: None,
        model_provider: None,
        model_id: None,
    })
    .expect("create run")
}

/// Create a pending approval row for the run.
pub fn seed_approval(
    approval_request_id: &str,
    run_id: &str,
) -> crate::store::ApprovalRequestRecord {
    crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id.to_string()),
        run_id: run_id.to_string(),
        tool_call_id: None,
        kind: "tool".to_string(),
        title: "Approve `tool`".to_string(),
        summary: None,
        risk_level: None,
        requested_action: None,
        action_category: None,
        action_payload: None,
        sandbox_boundary: None,
        save_suggestion: None,
        reviewer: None,
    })
    .expect("ensure approval");
    crate::store::get_approval_request(approval_request_id)
        .expect("approval query")
        .expect("approval exists")
}

/// Full-graph fixture: ws1 (temporary, files on disk) with threads and
/// runs in assorted states. `sessions`: (thread, agent_session_id).
pub fn seed_full(conn: &Connection) {
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
/// A thread mirrored for an agent session that never produced a run — the
/// shape older builds left behind for sessions announced at creation and
/// never prompted.
pub fn seed_message_less_threads(conn: &Connection, threads: &[(&str, &str, i64)]) {
    conn.execute_batch(
        "INSERT INTO workspaces (
             id, name, kind, path, cleanup_status, created_at, updated_at
         ) VALUES ('ws_empty', 'WS', 'temporary', '/tmp/ws-empty', 'active', 1, 1);",
    )
    .expect("seed workspace");
    for (id, session_id, created_at) in threads {
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, agent_session_id, created_at, updated_at
             ) VALUES (?1, 'ws_empty', 'chat', 'T', ?2, ?3, ?3)",
            rusqlite::params![id, session_id, created_at],
        )
        .expect("seed thread");
    }
}
