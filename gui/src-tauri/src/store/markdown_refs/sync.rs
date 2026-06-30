//! Keeps the denormalized `reference_targets` / `object_references` tables in
//! sync with the `futureos://` links found in a message body. On every message
//! write the prior links for that message are cleared and re-derived, upserting
//! a cached metadata snapshot per referenced object so search stays fast.

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::util::{create_id, now_millis};

use super::extract::{extract_markdown_references, MarkdownObjectReference};
use super::metadata::{
    approval_metadata, artifact_metadata, research_metadata, review_metadata, run_metadata,
    tool_metadata, ReferenceMetadata,
};

pub fn sync_message_markdown_references(
    conn: &Connection,
    message_id: &str,
    thread_id: &str,
    content: &str,
) -> Result<(), crate::AppError> {
    let references = extract_markdown_references(content);
    conn.execute(
        "DELETE FROM object_references
         WHERE source_type = 'message' AND source_id = ?1",
        params![message_id],
    )?;

    if references.is_empty() {
        return Ok(());
    }

    let workspace_id: String = conn.query_row(
        "SELECT workspace_id FROM threads WHERE id = ?1",
        params![thread_id],
        |row| row.get(0),
    )?;

    let now = now_millis();
    for reference in references {
        if let Some(target) = resolve_reference_target_metadata(conn, &reference, &workspace_id)? {
            let reference_target_id =
                upsert_reference_target(conn, &reference, target, &workspace_id, now)?;
            conn.execute(
                "INSERT INTO object_references (
                     id, source_type, source_id, reference_target_id, created_at
                 ) VALUES (?1, 'message', ?2, ?3, ?4)",
                params![
                    create_id("object_ref"),
                    message_id,
                    reference_target_id,
                    now
                ],
            )?;
        }
    }

    Ok(())
}

fn resolve_reference_target_metadata(
    conn: &Connection,
    reference: &MarkdownObjectReference,
    workspace_id: &str,
) -> Result<Option<ReferenceMetadata>, crate::AppError> {
    match reference.target_type.as_str() {
        "artifact" => conn
            .query_row(
                "SELECT title, type, path, summary
                 FROM artifacts
                 WHERE id = ?1
                   AND workspace_id = ?2
                   AND deleted_at IS NULL",
                params![reference.target_id, workspace_id],
                |row| {
                    Ok(artifact_metadata(
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()
            .map_err(crate::AppError::from),
        "run" => conn
            .query_row(
                "SELECT id, status, model_id, error_message
                 FROM runs
                 WHERE id = ?1
                   AND thread_id IN (
                     SELECT id FROM threads WHERE workspace_id = ?2
                   )",
                params![reference.target_id, workspace_id],
                |row| {
                    let id: String = row.get(0)?;
                    Ok(run_metadata(&id, row.get(1)?, row.get(2)?, row.get(3)?))
                },
            )
            .optional()
            .map_err(crate::AppError::from),
        "tool" => conn
            .query_row(
                "SELECT tc.name, tc.kind, tc.status, tc.input
                 FROM tool_calls tc
                 JOIN runs r ON r.id = tc.run_id
                 JOIN threads t ON t.id = r.thread_id
                 WHERE tc.id = ?1 AND t.workspace_id = ?2",
                params![reference.target_id, workspace_id],
                |row| {
                    Ok(tool_metadata(
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()
            .map_err(crate::AppError::from),
        "approval" => conn
            .query_row(
                "SELECT title, kind, status, summary, requested_action
                 FROM approval_requests
                 WHERE id = ?1
                   AND thread_id IN (
                     SELECT id FROM threads WHERE workspace_id = ?2
                   )",
                params![reference.target_id, workspace_id],
                |row| {
                    Ok(approval_metadata(
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(crate::AppError::from),
        "review" => conn
            .query_row(
                "SELECT title, status, summary, files_changed, additions, deletions
                 FROM review_changesets
                 WHERE id = ?1
                   AND thread_id IN (
                     SELECT id FROM threads WHERE workspace_id = ?2
                   )",
                params![reference.target_id, workspace_id],
                |row| {
                    Ok(review_metadata(
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(crate::AppError::from),
        "research" => conn
            .query_row(
                "SELECT r.title, r.type, r.source_uri, r.summary
                 FROM research_resources r
                 JOIN research_collections c ON c.id = r.collection_id
                 WHERE r.id = ?1 AND c.workspace_id = ?2",
                params![reference.target_id, workspace_id],
                |row| {
                    Ok(research_metadata(
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()
            .map_err(crate::AppError::from),
        _ => Ok(None),
    }
}

fn upsert_reference_target(
    conn: &Connection,
    reference: &MarkdownObjectReference,
    metadata: ReferenceMetadata,
    workspace_id: &str,
    now: i64,
) -> Result<String, crate::AppError> {
    let existing_id: Option<String> = conn
        .query_row(
            "SELECT id
             FROM reference_targets
             WHERE target_type = ?1
               AND target_id = ?2
               AND scope = 'workspace'
               AND workspace_id = ?3
             LIMIT 1",
            params![reference.target_type, reference.target_id, workspace_id],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(existing_id) = existing_id {
        conn.execute(
            "UPDATE reference_targets
             SET title = ?1, subtitle = ?2, search_text = ?3, updated_at = ?4
             WHERE id = ?5",
            params![
                metadata.title,
                metadata.subtitle,
                metadata.search_text,
                now,
                existing_id
            ],
        )?;
        return Ok(existing_id);
    }

    let id = create_id("ref_target");
    conn.execute(
        "INSERT INTO reference_targets (
             id, target_type, target_id, scope, workspace_id, title, subtitle,
             search_text, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'workspace', ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id,
            reference.target_type,
            reference.target_id,
            workspace_id,
            metadata.title,
            metadata.subtitle,
            metadata.search_text,
            now
        ],
    )?;
    Ok(id)
}
