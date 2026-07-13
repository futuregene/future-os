//! Resolves explicit `futureos://` references into the live store records they
//! point at, scoped to a single workspace. Each reference resolves to one of
//! `resolved` / `missing` (with an error note); failures never abort the batch.

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::approvals::{
    approval_request_from_row, ApprovalRequestRecord, APPROVAL_REQUEST_COLUMNS,
};
use crate::store::artifacts::{artifact_from_row, ArtifactRecord, ARTIFACT_COLUMNS};
use crate::store::db::connect;
use crate::store::records::*;
use crate::store::review_snapshots::{
    review_changeset_from_row, ReviewChangesetRecord, REVIEW_CHANGESET_COLUMNS,
};
use crate::store::runs::{
    run_from_row, tool_call_from_row, RunRecord, ToolCallRecord, RUN_COLUMNS, TOOL_CALL_COLUMNS,
};
use crate::store::util::qualify_columns;

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
        "file" => match resolve_file_reference(conn, workspace_id, &target_id) {
            Ok(Some(file)) => resolved_reference(target_type, target_id, file),
            Ok(None) => missing_reference(
                target_type,
                target_id,
                "file was not found in the workspace",
            ),
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
        &format!(
            "SELECT {ARTIFACT_COLUMNS}
         FROM artifacts
         WHERE id = ?1 AND workspace_id = ?2 AND deleted_at IS NULL"
        ),
        params![id, workspace_id],
        artifact_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

/// A local-file link (a plain markdown path link), rendered by the frontend as
/// a file link. Resolution is pure path arithmetic — no filesystem access — so it never
/// probes whether the path exists (no existence oracle) and never fails: any
/// non-empty path a message names becomes a link. `insideWorkspace` +
/// `relativePath` let the UI show a workspace-relative path for in-workspace
/// files and the full path for ones written elsewhere (e.g. `~/Desktop`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedFile {
    /// Absolute path, used for open / copy-path actions.
    path: String,
    /// File name (last path component), for the copy-filename action.
    name: String,
    /// Path relative to the workspace root, present only when inside it.
    relative_path: Option<String>,
    inside_workspace: bool,
}

/// Turn a file reference into its display model. The path may be absolute (the
/// model writes it verbatim, so the leading slash is intact) or workspace-
/// relative; anything not absolute is resolved against the workspace root.
fn resolve_file_reference(
    conn: &Connection,
    workspace_id: &str,
    raw_path: &str,
) -> Result<Option<ResolvedFile>, crate::AppError> {
    let raw = raw_path.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    let workspace_path: Option<String> = conn
        .query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            params![workspace_id],
            |row| row.get(0),
        )
        .optional()?;

    let raw_ref = std::path::Path::new(raw);
    let absolute = if raw_ref.is_absolute() {
        std::path::PathBuf::from(raw)
    } else if let Some(root) = workspace_path.as_deref() {
        std::path::Path::new(root).join(raw)
    } else {
        // No workspace root to anchor a relative path; treat it as absolute.
        std::path::PathBuf::from(format!("/{raw}"))
    };

    let name = absolute
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string();

    // Lexical containment only (no canonicalize / stat): a deleted file must
    // still resolve, and resolution must not touch the filesystem.
    let (inside_workspace, relative_path) = match workspace_path.as_deref() {
        Some(root) => match absolute.strip_prefix(root) {
            Ok(relative) => (true, Some(relative.to_string_lossy().into_owned())),
            Err(_) => (false, None),
        },
        None => (false, None),
    };

    Ok(Some(ResolvedFile {
        path: absolute.to_string_lossy().into_owned(),
        name,
        relative_path,
        inside_workspace,
    }))
}

fn get_run_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<RunRecord>, crate::AppError> {
    let cols = qualify_columns("r", RUN_COLUMNS);
    conn.query_row(
        &format!(
            "SELECT {cols} FROM runs r
         JOIN threads t ON t.id = r.thread_id
         WHERE r.id = ?1 AND t.workspace_id = ?2"
        ),
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
    let cols = qualify_columns("tc", TOOL_CALL_COLUMNS);
    conn.query_row(
        &format!(
            "SELECT {cols} FROM tool_calls tc
         JOIN runs r ON r.id = tc.run_id
         JOIN threads t ON t.id = r.thread_id
         WHERE tc.id = ?1 AND t.workspace_id = ?2"
        ),
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
    let cols = qualify_columns("a", APPROVAL_REQUEST_COLUMNS);
    conn.query_row(
        &format!(
            "SELECT {cols} FROM approval_requests a
         JOIN threads t ON t.id = a.thread_id
         WHERE a.id = ?1 AND t.workspace_id = ?2"
        ),
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
    // Columns qualified with `c.` because the JOIN onto `threads` makes several
    // names (id, thread_id, created_at, updated_at) ambiguous. Use the shared
    // column list so this stays in sync with `review_changeset_from_row`.
    let cols = qualify_columns("c", REVIEW_CHANGESET_COLUMNS);
    conn.query_row(
        &format!(
            "SELECT {cols} FROM review_changesets c
             JOIN threads t ON t.id = c.thread_id
             WHERE c.id = ?1 AND t.workspace_id = ?2"
        ),
        params![id, workspace_id],
        review_changeset_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}
