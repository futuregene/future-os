use rusqlite::{params, OptionalExtension};

use super::db::*;
use super::records::*;
use super::util::*;

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
    tx.execute(
        "INSERT INTO approval_requests (
             id, thread_id, run_id, tool_call_id, kind, status, title, summary,
             risk_level, requested_action, created_at, updated_at,
             action_category, action_payload, sandbox_boundary,
             reviewer, decision_scope, decision_source
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8, ?9, ?10, ?10,
                   ?11, ?12, ?13, ?14, 'once', 'user')",
        params![
            input
                .approval_request_id
                .unwrap_or_else(|| create_id("approval")),
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
            reviewer,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn list_approval_requests(
    thread_id: &str,
) -> Result<Vec<ApprovalRequestRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, thread_id, run_id, tool_call_id, kind, status, title, summary,
                risk_level, requested_action, decision_note, decided_at, created_at, updated_at,
                action_category, action_payload, sandbox_boundary, reviewer, decision_scope, decision_source
         FROM approval_requests
             WHERE thread_id = ?1
             ORDER BY created_at DESC",
        )
        ?;
    let rows = stmt.query_map(params![thread_id], approval_request_from_row)?;
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
    conn.execute(
        "UPDATE approval_requests
         SET status = ?1, decision_note = ?2, decided_at = ?3, updated_at = ?3
         WHERE id = ?4",
        params![status, input.decision_note, now, input.approval_request_id],
    )?;

    get_approval_request(&input.approval_request_id)?
        .ok_or_else(|| "Approval request could not be loaded.".to_string().into())
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
