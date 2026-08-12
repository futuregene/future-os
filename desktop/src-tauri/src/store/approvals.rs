use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::db::*;
use super::records::*;
use super::review_snapshots::{
    review_file_change_from_row, ReviewFileChangeRecord, REVIEW_FILE_CHANGE_COLUMNS,
};
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestRecord {
    pub id: String,
    pub thread_id: String,
    pub run_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub summary: Option<String>,
    pub risk_level: Option<String>,
    pub requested_action: Option<String>,
    pub decision_note: Option<String>,
    pub decided_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    // P2: structured action and sandbox boundary
    pub action_category: Option<String>,
    pub action_payload: Option<String>,
    pub sandbox_boundary: Option<String>,
    // Phase 2: suggested rule (JSON) for session/always-allow persistence.
    pub save_suggestion: Option<String>,
    pub reviewer: String,
    pub decision_scope: String,
    pub decision_source: String,
}

sql_record!(pub(super) APPROVAL_REQUEST_COLUMNS, approval_request_from_row -> ApprovalRequestRecord {
    id, thread_id, run_id, tool_call_id, kind, status, title, summary,
    risk_level, requested_action, decision_note, decided_at, created_at, updated_at,
    action_category, action_payload, sandbox_boundary, save_suggestion, reviewer,
    decision_scope, decision_source,
});

pub fn ensure_approval_request(input: EnsureApprovalRequestInput) -> Result<(), crate::AppError> {
    // BEGIN IMMEDIATE so the existence check and the insert are one atomic
    // write — the agent can stream concurrent events for the same tool call, and
    // a plain check-then-insert would let two of them both insert a duplicate.
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let thread_id = run_thread_id(&tx, &input.run_id)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT id
             FROM approval_requests
             WHERE (?1 IS NOT NULL AND id = ?1)
                OR (?1 IS NULL AND tool_call_id = ?2 AND kind = ?3)
             LIMIT 1",
            params![input.approval_request_id, input.tool_call_id, input.kind],
            |row| row.get(0),
        )
        .optional()?;

    if existing.is_some() {
        return Ok(());
    }

    let now = now_millis();
    let reviewer = input.reviewer.unwrap_or_else(|| "user".to_string());
    let approval_request_id = input
        .approval_request_id
        .unwrap_or_else(|| create_id("approval"));
    tx.execute(
        "INSERT INTO approval_requests (
             id, thread_id, run_id, tool_call_id, kind, status, title, summary,
             risk_level, requested_action, created_at, updated_at,
             action_category, action_payload, sandbox_boundary, save_suggestion,
             reviewer, decision_scope, decision_source
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8, ?9, ?10, ?10,
                   ?11, ?12, ?13, ?14, ?15, 'once', 'user')",
        params![
            approval_request_id,
            thread_id,
            input.run_id,
            input.tool_call_id,
            input.kind,
            input.title,
            input.summary,
            input.risk_level,
            input.requested_action,
            now,
            input.action_category,
            input.action_payload,
            input.sandbox_boundary,
            input.save_suggestion,
            reviewer,
        ],
    )?;
    tx.commit()?;
    // The pending row is durable — push it to the webview immediately.
    crate::emit_approvals_updated(&thread_id, &approval_request_id);
    Ok(())
}

pub fn list_approval_requests(
    thread_id: &str,
) -> Result<Vec<ApprovalRequestRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {APPROVAL_REQUEST_COLUMNS}
             FROM approval_requests
             WHERE thread_id = ?1
             ORDER BY created_at DESC"
    ))?;
    let rows = stmt.query_map(params![thread_id], approval_request_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// Every still-pending approval across all threads. Used by the sidebar badge
/// (which counts approvals outside the open thread) and by startup/watchdog
/// reconciliation against the Agent's authoritative pending set.
pub fn list_pending_approval_requests() -> Result<Vec<ApprovalRequestRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {APPROVAL_REQUEST_COLUMNS}
             FROM approval_requests
             WHERE status = 'pending'
             ORDER BY created_at DESC"
    ))?;
    let rows = stmt.query_map([], approval_request_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn decide_approval_request(
    input: DecideApprovalRequestInput,
) -> Result<ApprovalRequestRecord, crate::AppError> {
    let status = match input.status.as_str() {
        "approved" | "rejected" | "cancelled" => input.status,
        _ => {
            return Err("approval status must be approved, rejected, or cancelled."
                .to_string()
                .into())
        }
    };
    let now = now_millis();
    let conn = connect()?;
    // Compare-and-set on `pending`: a decision is only recorded once, so a
    // concurrent/late decision (or a duplicate event) can't rewrite an already
    // decided request — the audit record stays immutable.
    let affected = conn.execute(
        "UPDATE approval_requests
         SET status = ?1, decision_note = ?2, decided_at = ?3, updated_at = ?3
         WHERE id = ?4
           AND status = 'pending'",
        params![status, input.decision_note, now, input.approval_request_id],
    )?;

    let record = loaded(
        get_approval_request(&input.approval_request_id)?,
        "Approval request",
    )?;
    // Only a decision that actually flipped a pending row changes the queue —
    // duplicate/late decisions must not re-notify.
    if affected > 0 {
        crate::emit_approvals_updated(&record.thread_id, &record.id);
    }
    Ok(record)
}

pub fn list_review_file_changes(
    changeset_id: &str,
) -> Result<Vec<ReviewFileChangeRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {REVIEW_FILE_CHANGE_COLUMNS}
             FROM review_file_changes
             WHERE changeset_id = ?1
             ORDER BY created_at ASC",
    ))?;
    let rows = stmt.query_map(params![changeset_id], review_file_change_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::guarded_conn;

    /// ws1/t1/r1 plus a pending approval fixture scaffold.
    fn seed_run(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES ('r1', 't1', 'running', 1, 1);",
        )
        .expect("seed run");
    }

    fn ensure_input() -> EnsureApprovalRequestInput {
        EnsureApprovalRequestInput {
            approval_request_id: Some("ap1".to_string()),
            run_id: "r1".to_string(),
            tool_call_id: Some("tc1".to_string()),
            kind: "shell".to_string(),
            title: "Deploy".to_string(),
            summary: Some("Ship it".to_string()),
            risk_level: Some("high".to_string()),
            requested_action: Some("deploy --prod".to_string()),
            action_category: Some("command".to_string()),
            action_payload: Some("{}".to_string()),
            sandbox_boundary: Some("workspace".to_string()),
            save_suggestion: None,
            reviewer: None,
        }
    }

    #[test]
    fn ensure_inserts_once_and_dedupes_by_id_or_tool_call() {
        let (_home, conn) = guarded_conn("approvals_ensure");
        seed_run(&conn);
        drop(conn);

        ensure_approval_request(ensure_input()).expect("first insert");
        // Same explicit id → dedup…
        ensure_approval_request(ensure_input()).expect("dedup by id");
        // …and the tool_call+kind path dedups a request without an id.
        let mut anon = ensure_input();
        anon.approval_request_id = None;
        ensure_approval_request(anon).expect("dedup by tool call");

        let pending = list_pending_approval_requests().expect("pending");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "ap1");
        assert_eq!(pending[0].reviewer, "user", "default reviewer");

        let for_thread = list_approval_requests("t1").expect("list");
        assert_eq!(for_thread.len(), 1);
        assert_eq!(for_thread[0].summary.as_deref(), Some("Ship it"));
        assert!(list_approval_requests("t_other").expect("list").is_empty());
    }

    #[test]
    fn ensure_fails_when_the_run_is_missing() {
        let (_home, conn) = guarded_conn("approvals_ensure_missing");
        drop(conn);
        assert!(ensure_approval_request(ensure_input()).is_err());
    }

    #[test]
    fn decide_flips_pending_once_and_rejects_bad_status() {
        let (_home, conn) = guarded_conn("approvals_decide");
        seed_run(&conn);
        drop(conn);
        ensure_approval_request(ensure_input()).expect("insert");

        let bad = decide_approval_request(DecideApprovalRequestInput {
            approval_request_id: "ap1".to_string(),
            status: "maybe".to_string(),
            decision_note: None,
        });
        assert!(bad.is_err(), "unknown status rejected");

        let decided = decide_approval_request(DecideApprovalRequestInput {
            approval_request_id: "ap1".to_string(),
            status: "approved".to_string(),
            decision_note: Some("lgtm".to_string()),
        })
        .expect("decide");
        assert_eq!(decided.status, "approved");
        assert_eq!(decided.decision_note.as_deref(), Some("lgtm"));
        assert!(decided.decided_at.is_some());

        // A late duplicate decision doesn't rewrite the record.
        let again = decide_approval_request(DecideApprovalRequestInput {
            approval_request_id: "ap1".to_string(),
            status: "rejected".to_string(),
            decision_note: None,
        })
        .expect("late decide returns the record");
        assert_eq!(again.status, "approved", "CAS kept the first decision");

        assert!(list_pending_approval_requests().expect("pending").is_empty());
    }

    #[test]
    fn decide_missing_request_errors() {
        let (_home, conn) = guarded_conn("approvals_decide_missing");
        drop(conn);
        let result = decide_approval_request(DecideApprovalRequestInput {
            approval_request_id: "ghost".to_string(),
            status: "approved".to_string(),
            decision_note: None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn list_review_file_changes_orders_by_creation() {
        let (_home, conn) = guarded_conn("approvals_file_changes");
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 1, 1);
             INSERT INTO review_changesets (
                 id, thread_id, title, status, created_at, updated_at
             ) VALUES ('cs1', 't1', 'Changes', 'ready', 1, 1);
             INSERT INTO review_file_changes (
                 id, changeset_id, target_type, change_type, path, created_at, updated_at
             ) VALUES
                 ('fc2', 'cs1', 'file', 'edit', 'b.md', 2, 2),
                 ('fc1', 'cs1', 'file', 'add', 'a.md', 1, 1);",
        )
        .expect("seed file changes");
        drop(conn);

        let changes = list_review_file_changes("cs1").expect("list");
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].id, "fc1");
        assert_eq!(changes[1].id, "fc2");
        assert!(list_review_file_changes("cs_ghost").expect("list").is_empty());
    }
}
