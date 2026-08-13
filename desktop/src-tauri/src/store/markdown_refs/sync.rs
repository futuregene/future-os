//! Keeps the denormalized `reference_targets` / `object_references` tables in
//! sync with the `futureos://` links found in a message body. On every message
//! write the prior links for that message are cleared and re-derived, upserting
//! a cached metadata snapshot per referenced object so search stays fast.

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::util::{create_id, now_millis};

use super::extract::{extract_markdown_references, MarkdownObjectReference};
use super::metadata::{
    approval_metadata, artifact_metadata, review_metadata, run_metadata, ReferenceMetadata,
};

pub fn sync_message_markdown_references(
    conn: &Connection,
    message_id: &str,
    thread_id: &str,
    content: &str,
) -> Result<(), crate::AppError> {
    let references = extract_markdown_references(content);
    const DELETE_SQL: &str = "DELETE FROM object_references
         WHERE source_type = 'message' AND source_id = ?1";
    conn.execute(DELETE_SQL, params![message_id])?;

    if references.is_empty() {
        return Ok(());
    }

    let workspace_id: String = conn.query_row(
        "SELECT workspace_id FROM threads WHERE id = ?1",
        params![thread_id],
        |row| row.get(0),
    )?;

    let now = now_millis();
    const INSERT_LINK_SQL: &str = "INSERT INTO object_references (
                     id, source_type, source_id, reference_target_id, created_at
                 ) VALUES (?1, 'message', ?2, ?3, ?4)";
    for reference in references {
        if let Some(target) = resolve_reference_target_metadata(conn, &reference, &workspace_id)? {
            let reference_target_id =
                upsert_reference_target(conn, &reference, target, &workspace_id, now)?;
            let link_id = create_id("object_ref");
            let args = params![link_id, message_id, reference_target_id, now];
            conn.execute(INSERT_LINK_SQL, args)?;
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
                "SELECT title, artifact_type, path, summary
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
        // `file` references a workspace file by path (resolved against disk at
        // render time, not the `artifacts` table). Denormalize purely from the
        // path string — no DB row is required for the reference to be valid.
        "file" => {
            let path = reference.target_id.as_str();
            let name = path
                .rsplit(['/', '\\'])
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or(path)
                .to_string();
            let artifact_type = crate::store::artifact_type_from_path(std::path::Path::new(path));
            Ok(Some(artifact_metadata(
                name,
                artifact_type,
                Some(path.to_string()),
                None,
            )))
        }
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
        // Tool calls no longer live in a GUI table (the `tool_calls` table was
        // dropped with the journal-era pipeline; the Agent run-events journal
        // is their source of truth), so there is nothing to denormalize from.
        // Skip the target row — consistent with the resolve path, which treats
        // tool references as unsupported. The message write itself must not
        // fail because a link names a tool.
        "tool" => Ok(None),
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
        const UPDATE_SQL: &str = "UPDATE reference_targets
             SET title = ?1, subtitle = ?2, search_text = ?3, updated_at = ?4
             WHERE id = ?5";
        let ReferenceMetadata {
            title,
            subtitle,
            search_text,
        } = metadata;
        let args = params![title, subtitle, search_text, now, existing_id];
        conn.execute(UPDATE_SQL, args)?;
        return Ok(existing_id);
    }

    let id = create_id("ref_target");
    const INSERT_SQL: &str = "INSERT INTO reference_targets (
             id, target_type, target_id, scope, workspace_id, title, subtitle,
             search_text, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'workspace', ?4, ?5, ?6, ?7, ?8, ?8)";
    let ReferenceMetadata {
        title,
        subtitle,
        search_text,
    } = metadata;
    let (ty, tid, ws) = (&reference.target_type, &reference.target_id, workspace_id);
    let args = params![id, ty, tid, ws, title, subtitle, search_text, now];
    conn.execute(INSERT_SQL, args)?;
    Ok(id)
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

    fn seed_thread(conn: &Connection) {
        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1)",
            [],
        )
        .expect("insert workspace");
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 'active', 0, 0, 1, 1)",
            [],
        )
        .expect("insert thread");
    }

    fn target_title(conn: &Connection, target_type: &str, target_id: &str) -> Option<String> {
        conn.query_row(
            "SELECT title FROM reference_targets
             WHERE target_type = ?1 AND target_id = ?2 AND workspace_id = 'ws1'",
            params![target_type, target_id],
            |row| row.get(0),
        )
        .optional()
        .expect("query reference target")
    }

    #[test]
    fn sync_without_references_clears_stale_links_and_returns() {
        let conn = test_conn();
        // No thread rows needed: the early return precedes the workspace lookup.
        sync_message_markdown_references(&conn, "msg_empty", "t_missing", "plain text")
            .expect("sync empty content");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM reference_targets", [], |row| {
                row.get(0)
            })
            .expect("count targets");
        assert_eq!(count, 0);
    }

    #[test]
    fn sync_fails_when_the_thread_is_missing() {
        let conn = test_conn();
        let result =
            sync_message_markdown_references(&conn, "msg_x", "t_missing", "[r](futureos://run/r1)");
        assert!(
            result.is_err(),
            "workspace lookup must fail for a missing thread"
        );
    }

    #[test]
    fn syncs_run_approval_and_review_targets() {
        let conn = test_conn();
        seed_thread(&conn);
        conn.execute(
            "INSERT INTO runs (
                 id, thread_id, status, model_id, error_message, created_at, updated_at
             ) VALUES ('r1', 't1', 'failed', 'gpt-x', 'boom', 1, 1)",
            [],
        )
        .expect("insert run");
        conn.execute(
            "INSERT INTO approval_requests (
                 id, thread_id, kind, status, title, summary, requested_action,
                 created_at, updated_at
             ) VALUES ('a1', 't1', 'shell', 'pending', 'Deploy', 'Ship', 'deploy', 1, 1)",
            [],
        )
        .expect("insert approval");
        conn.execute(
            "INSERT INTO review_changesets (
                 id, thread_id, title, summary, status, files_changed, additions,
                 deletions, created_at, updated_at
             ) VALUES ('rc1', 't1', 'Changes', 'Sum', 'ready', 2, 5, 1, 1, 1)",
            [],
        )
        .expect("insert review changeset");

        sync_message_markdown_references(
            &conn,
            "msg_mix",
            "t1",
            "[r](futureos://run/r1) [a](futureos://approval/a1) [c](futureos://review/rc1)",
        )
        .expect("sync mixed references");

        assert_eq!(target_title(&conn, "run", "r1").as_deref(), Some("Run r1"));
        assert_eq!(
            target_title(&conn, "approval", "a1").as_deref(),
            Some("Deploy")
        );
        assert_eq!(
            target_title(&conn, "review", "rc1").as_deref(),
            Some("Changes")
        );
        let link_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM object_references", [], |row| {
                row.get(0)
            })
            .expect("count links");
        assert_eq!(link_count, 3);
    }

    #[test]
    fn objects_outside_the_workspace_produce_no_target() {
        let conn = test_conn();
        seed_thread(&conn);
        // A run attached to a *different* workspace's thread is invisible.
        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws2', 'Other', 'temporary', '/tmp/ws2', 'active', 1, 1)",
            [],
        )
        .expect("insert other workspace");
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 created_at, updated_at
             ) VALUES ('t2', 'ws2', 'chat', 'T2', 'active', 0, 0, 1, 1)",
            [],
        )
        .expect("insert other thread");
        conn.execute(
            "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES ('r_other', 't2', 'completed', 1, 1)",
            [],
        )
        .expect("insert other run");

        sync_message_markdown_references(&conn, "msg_o", "t1", "[r](futureos://run/r_other)")
            .expect("sync cross-workspace reference");
        assert_eq!(target_title(&conn, "run", "r_other"), None);
    }

    #[test]
    fn syncs_file_targets_from_the_path_alone() {
        let conn = test_conn();
        seed_thread(&conn);

        sync_message_markdown_references(&conn, "msg_f", "t1", "[notes](/abs/dir/notes.md)")
            .expect("sync file reference");
        assert_eq!(
            target_title(&conn, "file", "/abs/dir/notes.md").as_deref(),
            Some("notes.md")
        );

        // A trailing-separator path has no final component — the whole path is
        // the display name.
        sync_message_markdown_references(&conn, "msg_d", "t1", "[dir](/abs/dir/)")
            .expect("sync directory reference");
        assert_eq!(
            target_title(&conn, "file", "/abs/dir/").as_deref(),
            Some("/abs/dir/")
        );
    }

    #[test]
    fn resyncing_updates_the_cached_target_row() {
        let conn = test_conn();
        seed_thread(&conn);
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, artifact_type, created_at, updated_at
             ) VALUES ('art1', 'ws1', 't1', 'Old title', 'document', 1, 1)",
            [],
        )
        .expect("insert artifact");

        let link = "[a](futureos://artifact/art1)";
        sync_message_markdown_references(&conn, "msg_1", "t1", link).expect("first sync");
        assert_eq!(
            target_title(&conn, "artifact", "art1").as_deref(),
            Some("Old title")
        );

        conn.execute(
            "UPDATE artifacts SET title = 'New title' WHERE id = 'art1'",
            [],
        )
        .expect("rename artifact");
        sync_message_markdown_references(&conn, "msg_2", "t1", link).expect("second sync");

        assert_eq!(
            target_title(&conn, "artifact", "art1").as_deref(),
            Some("New title"),
            "the existing target row is updated, not duplicated"
        );
        let target_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM reference_targets", [], |row| {
                row.get(0)
            })
            .expect("count targets");
        assert_eq!(target_count, 1);
    }

    #[test]
    fn unknown_target_types_resolve_to_none() {
        let conn = test_conn();
        let reference = MarkdownObjectReference {
            target_id: "x".to_string(),
            target_type: "widget".to_string(),
        };
        assert!(resolve_reference_target_metadata(&conn, &reference, "ws1")
            .expect("resolve")
            .is_none());
    }
}
