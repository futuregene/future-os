//! Markdown object-reference handling, split by concern:
//! - [`extract`] — pure parsing of `futureos://` links/fences out of markdown.
//! - [`resolve`] — turn explicit references into live store records (read side).
//! - [`search`] — `@`-mention pick-list search across workspace objects.
//! - [`sync`] — keep the denormalized reference tables in step with messages.

mod extract;
mod metadata;
mod resolve;
mod search;
mod sync;

pub use self::resolve::resolve_markdown_references;
pub use self::search::search_reference_targets;
pub use self::sync::sync_message_markdown_references;

/// First eight characters of an id, used to label runs in search/sync output.
fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::resolve::resolve_markdown_reference;
    use super::search::search_artifact_targets;
    use super::sync_message_markdown_references;
    use crate::store::records::MarkdownReferenceInput;
    use crate::store::schema::SCHEMA;
    use rusqlite::Connection;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        conn
    }

    #[test]
    fn syncs_message_references_into_reference_tables() {
        let conn = test_conn();
        seed_workspace_artifact(&conn);

        sync_message_markdown_references(
            &conn,
            "msg_test",
            "thread_test",
            "[Poem](futureos://artifact/artifact_test)",
        )
        .expect("sync markdown references");

        let target_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM reference_targets", [], |row| {
                row.get(0)
            })
            .expect("count reference targets");
        let object_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM object_references", [], |row| {
                row.get(0)
            })
            .expect("count object references");
        let title: String = conn
            .query_row("SELECT title FROM reference_targets", [], |row| row.get(0))
            .expect("load target title");

        assert_eq!(target_count, 1);
        assert_eq!(object_count, 1);
        assert_eq!(title, "Poem");
    }

    #[test]
    fn searches_workspace_reference_targets_from_objects() {
        let conn = test_conn();
        seed_workspace_artifact(&conn);

        let mut results = vec![];
        search_artifact_targets(&conn, "ws_test", "poem", &mut results)
            .expect("search artifact targets");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_type, "artifact");
        assert_eq!(results[0].target_id, "artifact_test");
        assert_eq!(results[0].title, "Poem");
    }

    #[test]
    fn resolves_references_with_workspace_scope_and_deleted_filter() {
        let conn = test_conn();
        seed_workspace_artifact(&conn);
        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws_other', 'Other', 'temporary', '/tmp/other', 'active', 1, 1)",
            [],
        )
        .expect("insert other workspace");
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 created_at, updated_at
             ) VALUES (
                 'thread_other', 'ws_other', 'chat', 'Other', 'active', 0, 0, 1, 1
             )",
            [],
        )
        .expect("insert other thread");
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, artifact_type, created_at, updated_at
             ) VALUES (
                 'artifact_other', 'ws_other', 'thread_other', 'Other Poem', 'document', 1, 1
             )",
            [],
        )
        .expect("insert other artifact");
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, artifact_type, created_at, updated_at, deleted_at
             ) VALUES (
                 'artifact_deleted', 'ws_test', 'thread_test', 'Deleted Poem',
                 'document', 1, 1, 2
             )",
            [],
        )
        .expect("insert deleted artifact");

        let resolved = [
            ("artifact_test", "resolved"),
            ("artifact_other", "missing"),
            ("artifact_deleted", "missing"),
        ]
        .into_iter()
        .map(|(target_id, expected_status)| {
            let resolved = resolve_markdown_reference(
                &conn,
                "ws_test",
                MarkdownReferenceInput {
                    target_id: target_id.to_string(),
                    target_type: "artifact".to_string(),
                },
            );
            (resolved.status, expected_status)
        })
        .collect::<Vec<_>>();

        assert_eq!(
            resolved,
            vec![
                ("resolved".to_string(), "resolved"),
                ("missing".to_string(), "missing"),
                ("missing".to_string(), "missing"),
            ]
        );
    }

    #[test]
    fn resolves_file_references_by_path_including_slash_restored() {
        let conn = test_conn();
        seed_workspace_artifact(&conn);
        // An absolute path; the frontend URL parser strips its leading slash, so
        // resolution must still match "abs/dir/note.txt" back to "/abs/dir/note.txt".
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, artifact_type, path,
                 created_at, updated_at
             ) VALUES (
                 'artifact_abs', 'ws_test', 'thread_test', 'Note', 'document',
                 '/abs/dir/note.txt', 1, 1
             )",
            [],
        )
        .expect("insert absolute-path artifact");

        let cases = [
            ("poem.md", "resolved"),
            ("abs/dir/note.txt", "resolved"),
            ("/abs/dir/note.txt", "resolved"),
            ("missing.txt", "missing"),
        ]
        .into_iter()
        .map(|(target_id, expected)| {
            let resolved = resolve_markdown_reference(
                &conn,
                "ws_test",
                MarkdownReferenceInput {
                    target_id: target_id.to_string(),
                    target_type: "file".to_string(),
                },
            );
            (resolved.status, expected)
        })
        .collect::<Vec<_>>();

        assert_eq!(
            cases,
            vec![
                ("resolved".to_string(), "resolved"),
                ("resolved".to_string(), "resolved"),
                ("resolved".to_string(), "resolved"),
                ("missing".to_string(), "missing"),
            ]
        );
    }

    fn seed_workspace_artifact(conn: &Connection) {
        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws_test', 'Test', 'temporary', '/tmp/test', 'active', 1, 1)",
            [],
        )
        .expect("insert workspace");
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 created_at, updated_at
             ) VALUES (
                 'thread_test', 'ws_test', 'chat', 'Thread', 'active', 0, 0, 1, 1
             )",
            [],
        )
        .expect("insert thread");
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, artifact_type, path, summary,
                 created_at, updated_at
             ) VALUES (
                 'artifact_test', 'ws_test', 'thread_test', 'Poem', 'document',
                 'poem.md', 'Saved poem', 1, 1
             )",
            [],
        )
        .expect("insert artifact");
    }
}
