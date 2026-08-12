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
use crate::store::runs::{run_from_row, RunRecord, RUN_COLUMNS};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::SCHEMA;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        conn
    }

    /// Workspace `ws1` (`/tmp/ws1`) with thread `t1` carrying run `r1`,
    /// approval `a1`, and review changeset `rc1`.
    fn seed_objects(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 'active', 0, 0, 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES ('r1', 't1', 'completed', 1, 1);
             INSERT INTO approval_requests (
                 id, thread_id, kind, status, title, created_at, updated_at
             ) VALUES ('a1', 't1', 'shell', 'pending', 'Deploy', 1, 1);
             INSERT INTO review_changesets (
                 id, thread_id, title, status, created_at, updated_at
             ) VALUES ('rc1', 't1', 'Changes', 'ready', 1, 1);",
        )
        .expect("seed objects");
    }

    fn reference(target_type: &str, target_id: &str) -> MarkdownReferenceInput {
        MarkdownReferenceInput {
            target_id: target_id.to_string(),
            target_type: target_type.to_string(),
        }
    }

    #[test]
    fn public_resolver_requires_a_workspace_id() {
        let result = resolve_markdown_references(ResolveMarkdownReferencesInput {
            workspace_id: "  ".to_string(),
            references: vec![],
        });
        assert!(result.is_err());
    }

    #[test]
    fn public_resolver_maps_each_reference() {
        let _home = crate::auth_store::test_support::HomeGuard::new("resolve_pub");
        {
            let conn = connect().expect("connect");
            crate::store::db::apply_schema(&conn).expect("apply schema");
        }
        let resolved = resolve_markdown_references(ResolveMarkdownReferencesInput {
            workspace_id: "ws_any".to_string(),
            references: vec![reference("run", "missing_run")],
        })
        .expect("resolve batch");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].status, "missing");
    }

    #[test]
    fn resolves_runs_scoped_to_the_workspace() {
        let conn = test_conn();
        seed_objects(&conn);

        let resolved = resolve_markdown_reference(&conn, "ws1", reference("run", "r1"));
        assert_eq!(resolved.status, "resolved");
        assert_eq!(resolved.data.expect("data")["id"], "r1");

        let missing = resolve_markdown_reference(&conn, "ws1", reference("run", "nope"));
        assert_eq!(missing.status, "missing");
        assert_eq!(missing.error.as_deref(), Some("run was not found"));
    }

    #[test]
    fn resolves_approvals_scoped_to_the_workspace() {
        let conn = test_conn();
        seed_objects(&conn);

        let resolved = resolve_markdown_reference(&conn, "ws1", reference("approval", "a1"));
        assert_eq!(resolved.status, "resolved");
        assert_eq!(resolved.data.expect("data")["title"], "Deploy");

        let missing = resolve_markdown_reference(&conn, "ws1", reference("approval", "nope"));
        assert_eq!(
            missing.error.as_deref(),
            Some("approval request was not found")
        );
    }

    #[test]
    fn resolves_review_changesets_scoped_to_the_workspace() {
        let conn = test_conn();
        seed_objects(&conn);

        let resolved = resolve_markdown_reference(&conn, "ws1", reference("review", "rc1"));
        assert_eq!(resolved.status, "resolved");
        assert_eq!(resolved.data.expect("data")["title"], "Changes");

        let missing = resolve_markdown_reference(&conn, "ws1", reference("review", "nope"));
        assert_eq!(
            missing.error.as_deref(),
            Some("review changeset was not found")
        );
    }

    #[test]
    fn unsupported_types_and_query_failures_resolve_as_missing() {
        let conn = test_conn();
        let unsupported = resolve_markdown_reference(&conn, "ws1", reference("tool", "t1"));
        assert_eq!(unsupported.status, "missing");
        assert_eq!(
            unsupported.error.as_deref(),
            Some("reference type is not supported yet")
        );

        // A connection without the schema turns each lookup's error into a
        // `missing` result carrying the message — never a batch abort.
        let bare = Connection::open_in_memory().expect("open bare database");
        for target_type in ["artifact", "file", "run", "approval", "review"] {
            let failed = resolve_markdown_reference(&bare, "ws1", reference(target_type, "x"));
            assert_eq!(failed.status, "missing", "{target_type}");
            assert!(failed.error.is_some(), "{target_type}");
        }
    }

    #[test]
    fn file_reference_empty_raw_path_resolves_to_none() {
        let conn = test_conn();
        assert!(
            resolve_file_reference(&conn, "ws1", "   ")
                .expect("resolve")
                .is_none()
        );
    }

    #[test]
    fn file_reference_relative_path_without_workspace_root_is_anchored_absolute() {
        let conn = test_conn();
        // No `ws_ghost` row: a relative path is surfaced as `/<raw>`.
        let file = resolve_file_reference(&conn, "ws_ghost", "notes/a.md")
            .expect("resolve")
            .expect("file");
        assert_eq!(file.path, "/notes/a.md");
        assert!(!file.inside_workspace);
        assert_eq!(file.relative_path, None);
    }

    #[test]
    fn file_reference_root_path_has_no_name() {
        let conn = test_conn();
        seed_objects(&conn);
        let file = resolve_file_reference(&conn, "ws1", "/")
            .expect("resolve")
            .expect("file");
        assert_eq!(file.name, "");
        assert!(!file.inside_workspace);
    }
}
