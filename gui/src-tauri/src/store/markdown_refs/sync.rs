//! Keeps the denormalized `reference_targets` / `object_references` tables in
//! sync with the `futureos://` links found in a message body. On every message
//! write the prior links for that message are cleared and re-derived, upserting
//! a cached metadata snapshot per referenced object so search stays fast.

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::util::{create_id, now_millis};

use super::extract::{extract_markdown_references, MarkdownObjectReference};
use super::short_id;

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

#[derive(Debug)]
struct ReferenceTargetMetadata {
    search_text: Option<String>,
    subtitle: Option<String>,
    title: String,
}

fn resolve_reference_target_metadata(
    conn: &Connection,
    reference: &MarkdownObjectReference,
    workspace_id: &str,
) -> Result<Option<ReferenceTargetMetadata>, crate::AppError> {
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
                    let title: String = row.get(0)?;
                    let artifact_type: String = row.get(1)?;
                    let path: Option<String> = row.get(2)?;
                    let summary: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [Some(title.clone()), path.clone(), summary.clone()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        subtitle: path.or(Some(artifact_type)),
                        title,
                    })
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
                    let status: String = row.get(1)?;
                    let model_id: Option<String> = row.get(2)?;
                    let error_message: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [
                                Some(id.clone()),
                                Some(status.clone()),
                                model_id.clone(),
                                error_message.clone(),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ),
                        subtitle: model_id.or(Some(status)),
                        title: format!("Run {}", short_id(&id)),
                    })
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
                    let name: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let status: String = row.get(2)?;
                    let input: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [
                                Some(name.clone()),
                                Some(kind.clone()),
                                Some(status.clone()),
                                input,
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ),
                        subtitle: Some(format!("{kind} · {status}")),
                        title: name,
                    })
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
                    let title: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let status: String = row.get(2)?;
                    let summary: Option<String> = row.get(3)?;
                    let requested_action: Option<String> = row.get(4)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [
                                Some(title.clone()),
                                Some(kind.clone()),
                                Some(status.clone()),
                                summary,
                                requested_action,
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ),
                        subtitle: Some(format!("{kind} · {status}")),
                        title,
                    })
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
                    let title: String = row.get(0)?;
                    let status: String = row.get(1)?;
                    let summary: Option<String> = row.get(2)?;
                    let files_changed: i64 = row.get(3)?;
                    let additions: i64 = row.get(4)?;
                    let deletions: i64 = row.get(5)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [Some(title.clone()), Some(status.clone()), summary]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        subtitle: Some(format!(
                            "{status} · {files_changed} files · +{additions} -{deletions}"
                        )),
                        title,
                    })
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
                    let title: String = row.get(0)?;
                    let resource_type: String = row.get(1)?;
                    let source_uri: Option<String> = row.get(2)?;
                    let summary: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [Some(title.clone()), source_uri.clone(), summary]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        subtitle: source_uri.or(Some(resource_type)),
                        title,
                    })
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
    metadata: ReferenceTargetMetadata,
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
