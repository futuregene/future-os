use rusqlite::{params, OptionalExtension};

use super::db::*;
use super::records::*;
use super::util::*;

pub fn ensure_approval_request(input: EnsureApprovalRequestInput) -> Result<(), crate::AppError> {
    let conn = connect()?;
    let thread_id = run_thread_id(&conn, &input.run_id)?;
    let existing: Option<String> = conn
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
    conn.execute(
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
    Ok(())
}

pub fn ensure_review_change(input: EnsureReviewChangeInput) -> Result<(), crate::AppError> {
    let conn = connect()?;
    let thread_id = run_thread_id(&conn, &input.run_id)?;
    let now = now_millis();
    let changeset_id: Option<String> = conn
        .query_row(
            "SELECT id
             FROM review_changesets
             WHERE tool_call_id = ?1
             LIMIT 1",
            params![input.tool_call_id],
            |row| row.get(0),
        )
        .optional()?;

    let changeset_id = if let Some(id) = changeset_id {
        id
    } else {
        let id = create_id("review");
        conn.execute(
            "INSERT INTO review_changesets (
                 id, thread_id, run_id, tool_call_id, title, summary, status,
                 files_changed, additions, deletions, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, 0, 0, ?7, ?7)",
            params![
                id,
                thread_id,
                input.run_id,
                input.tool_call_id,
                input.title,
                input.summary,
                now
            ],
        )?;
        id
    };

    if let Some(path) = input.path {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id
                 FROM review_file_changes
                 WHERE changeset_id = ?1 AND path = ?2
                 LIMIT 1",
                params![changeset_id, path],
                |row| row.get(0),
            )
            .optional()?;

        if existing.is_none() {
            conn.execute(
                "INSERT INTO review_file_changes (
                     id, changeset_id, target_type, path, change_type, summary,
                     additions, deletions, created_at, updated_at
                 ) VALUES (?1, ?2, 'file', ?3, ?4, ?5, 0, 0, ?6, ?6)",
                params![
                    create_id("review_file"),
                    changeset_id,
                    path,
                    input.change_type,
                    input.summary,
                    now
                ],
            )?;

            conn.execute(
                "UPDATE review_changesets
                 SET files_changed = files_changed + 1, updated_at = ?1
                 WHERE id = ?2",
                params![now, changeset_id],
            )?;
        }
    }

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

pub fn list_review_changesets(
    thread_id: &str,
) -> Result<Vec<ReviewChangesetRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, run_id, tool_call_id, title, summary, status,
                    files_changed, additions, deletions, created_at, updated_at
             FROM review_changesets
             WHERE thread_id = ?1
             ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(params![thread_id], review_changeset_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn update_review_changeset_status(
    input: UpdateReviewChangesetStatusInput,
) -> Result<ReviewChangesetRecord, crate::AppError> {
    let status = match input.status.as_str() {
        "applied" | "discarded" | "pending" => input.status,
        _ => {
            return Err(
                "review changeset status must be pending, applied, or discarded."
                    .to_string()
                    .into(),
            )
        }
    };
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "UPDATE review_changesets
         SET status = ?1, updated_at = ?2
         WHERE id = ?3",
        params![status, now, input.changeset_id],
    )?;

    conn.query_row(
        "SELECT id, thread_id, run_id, tool_call_id, title, summary, status,
                files_changed, additions, deletions, created_at, updated_at
         FROM review_changesets
         WHERE id = ?1",
        params![input.changeset_id],
        review_changeset_from_row,
    )
    .optional()?
    .ok_or_else(|| "Review changeset could not be loaded.".to_string().into())
}

pub fn list_review_file_changes(
    changeset_id: &str,
) -> Result<Vec<ReviewFileChangeRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, changeset_id, target_type, target_id, path, change_type,
                    before_ref, after_ref, diff, summary, additions, deletions,
                    created_at, updated_at
             FROM review_file_changes
             WHERE changeset_id = ?1
             ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![changeset_id], review_file_change_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}
