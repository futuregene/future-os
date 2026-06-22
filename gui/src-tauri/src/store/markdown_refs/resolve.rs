//! Resolves explicit `futureos://` references into the live store records they
//! point at, scoped to a single workspace. Each reference resolves to one of
//! `resolved` / `missing` (with an error note); failures never abort the batch.

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::db::connect;
use crate::store::records::*;

pub fn resolve_markdown_references(
    input: ResolveMarkdownReferencesInput,
) -> Result<Vec<ResolvedMarkdownReference>, crate::AppError> {
    let workspace_id = input.workspace_id.trim().to_string();
    if workspace_id.is_empty() {
        return Err("workspace id is required to resolve markdown references."
            .to_string()
            .into());
    }
    let conn = connect()?;
    Ok(input
        .references
        .into_iter()
        .map(|reference| resolve_markdown_reference(&conn, &workspace_id, reference))
        .collect())
}

pub(super) fn resolve_markdown_reference(
    conn: &Connection,
    workspace_id: &str,
    reference: MarkdownReferenceInput,
) -> ResolvedMarkdownReference {
    let target_type = reference.target_type.trim().to_ascii_lowercase();
    let target_id = reference.target_id.trim().to_string();

    if target_id.is_empty() {
        return missing_reference(target_type, target_id, "reference id is empty");
    }

    match target_type.as_str() {
        "artifact" => match get_artifact_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(artifact)) => resolved_reference(target_type, target_id, artifact),
            Ok(None) => missing_reference(target_type, target_id, "artifact was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "run" => match get_run_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(run)) => resolved_reference(target_type, target_id, run),
            Ok(None) => missing_reference(target_type, target_id, "run was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "tool" => match get_tool_call_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(tool)) => resolved_reference(target_type, target_id, tool),
            Ok(None) => missing_reference(target_type, target_id, "tool call was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "approval" => match get_approval_request_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(approval)) => resolved_reference(target_type, target_id, approval),
            Ok(None) => missing_reference(target_type, target_id, "approval request was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "review" => match get_review_changeset_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(review)) => resolved_reference(target_type, target_id, review),
            Ok(None) => missing_reference(target_type, target_id, "review changeset was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "research" => match get_research_resource_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(resource)) => resolved_reference(target_type, target_id, resource),
            Ok(None) => {
                missing_reference(target_type, target_id, "research resource was not found")
            }
            Err(error) => failed_reference(target_type, target_id, error),
        },
        _ => missing_reference(
            target_type,
            target_id,
            "reference type is not supported yet",
        ),
    }
}

fn resolved_reference<T: serde::Serialize>(
    target_type: String,
    target_id: String,
    value: T,
) -> ResolvedMarkdownReference {
    ResolvedMarkdownReference {
        target_type,
        target_id,
        status: "resolved".to_string(),
        data: serde_json::to_value(value).ok(),
        error: None,
    }
}

fn missing_reference(
    target_type: String,
    target_id: String,
    error: &str,
) -> ResolvedMarkdownReference {
    ResolvedMarkdownReference {
        target_type,
        target_id,
        status: "missing".to_string(),
        data: None,
        error: Some(error.to_string()),
    }
}

fn failed_reference(
    target_type: String,
    target_id: String,
    error: crate::AppError,
) -> ResolvedMarkdownReference {
    ResolvedMarkdownReference {
        target_type,
        target_id,
        status: "missing".to_string(),
        data: None,
        error: Some(error.to_string()),
    }
}

fn get_artifact_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ArtifactRecord>, crate::AppError> {
    conn.query_row(
        "SELECT id, workspace_id, thread_id, run_id, title, type, path, content,
                content_storage, summary, created_at, updated_at, deleted_at
         FROM artifacts
         WHERE id = ?1 AND workspace_id = ?2 AND deleted_at IS NULL",
        params![id, workspace_id],
        artifact_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

fn get_run_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<RunRecord>, crate::AppError> {
    conn.query_row(
        "SELECT r.id, r.thread_id, r.trigger_message_id, r.status, r.model_provider, r.model_id,
                r.started_at, r.ended_at, r.error_message, r.created_at, r.updated_at
         FROM runs r
         JOIN threads t ON t.id = r.thread_id
         WHERE r.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        run_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

fn get_tool_call_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ToolCallRecord>, crate::AppError> {
    conn.query_row(
        "SELECT tc.id, tc.run_id, tc.name, tc.kind, tc.input, tc.status,
                tc.started_at, tc.ended_at, tc.created_at
         FROM tool_calls tc
         JOIN runs r ON r.id = tc.run_id
         JOIN threads t ON t.id = r.thread_id
         WHERE tc.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        tool_call_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

fn get_approval_request_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ApprovalRequestRecord>, crate::AppError> {
    conn.query_row(
        "SELECT a.id, a.thread_id, a.run_id, a.tool_call_id, a.kind, a.status,
                a.title, a.summary, a.risk_level, a.requested_action, a.decision_note,
                a.decided_at, a.created_at, a.updated_at,
                a.action_category, a.action_payload, a.sandbox_boundary,
                a.reviewer, a.decision_scope, a.decision_source
         FROM approval_requests a
         JOIN threads t ON t.id = a.thread_id
         WHERE a.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        approval_request_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

fn get_review_changeset_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ReviewChangesetRecord>, crate::AppError> {
    conn.query_row(
        "SELECT c.id, c.thread_id, c.run_id, c.tool_call_id, c.title, c.summary, c.status,
                c.files_changed, c.additions, c.deletions, c.created_at, c.updated_at
         FROM review_changesets c
         JOIN threads t ON t.id = c.thread_id
         WHERE c.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        review_changeset_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

fn get_research_resource_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ResearchResourceRecord>, crate::AppError> {
    conn.query_row(
        "SELECT r.id, r.collection_id, c.workspace_id, r.source_artifact_id, r.title,
                r.type, r.source_uri, r.content, r.content_storage, r.summary,
                r.metadata, r.created_at, r.updated_at
         FROM research_resources r
         JOIN research_collections c ON c.id = r.collection_id
         WHERE r.id = ?1 AND c.workspace_id = ?2",
        params![id, workspace_id],
        research_resource_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}
