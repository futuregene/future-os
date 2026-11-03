//! Resolve explicit object references against live store records.
//!
//! The composer's `@`-mention picker searches workspace files, not objects —
//! see [`super::workspace_files`].

mod resolve;

pub use self::resolve::resolve_markdown_references;

#[cfg(test)]
mod tests {
    use super::resolve::resolve_markdown_reference;
    use crate::store::records::MarkdownReferenceInput;
    use crate::store::schema::SCHEMA;
    use rusqlite::Connection;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        conn
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
    fn resolves_file_references_to_relative_or_absolute_display() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws_fs', 'FS', 'temporary', '/work/space', 'active', 1, 1)",
            [],
        )
        .expect("insert fs workspace");

        let resolve = |target_id: &str| {
            resolve_markdown_reference(
                &conn,
                "ws_fs",
                MarkdownReferenceInput {
                    target_id: target_id.to_string(),
                    target_type: "file".to_string(),
                },
            )
        };

        // Workspace-relative path → inside, shown relative. The absolute path
        // is joined with the platform separator (`space\sub` on Windows) —
        // equivalent for the local open/copy actions it feeds.
        let relative = resolve("sub/note.md");
        assert_eq!(relative.status, "resolved");
        let relative = relative.data.expect("data");
        let expected_path = std::path::Path::new("/work/space").join("sub/note.md");
        assert_eq!(
            relative["path"],
            expected_path.to_string_lossy().into_owned()
        );
        assert_eq!(relative["relativePath"], "sub/note.md");
        assert_eq!(relative["insideWorkspace"], true);
        assert_eq!(relative["name"], "note.md");

        // Absolute path inside the workspace → still shown relative.
        let inside_abs = resolve("/work/space/deep/a.txt");
        let inside_abs = inside_abs.data.expect("data");
        assert_eq!(inside_abs["relativePath"], "deep/a.txt");
        assert_eq!(inside_abs["insideWorkspace"], true);

        // Absolute path outside the workspace (e.g. ~/Desktop) → full path, no relative.
        let outside = resolve("/Users/tao/Desktop/note.txt");
        assert_eq!(outside.status, "resolved");
        let outside = outside.data.expect("data");
        assert_eq!(outside["path"], "/Users/tao/Desktop/note.txt");
        assert_eq!(outside["insideWorkspace"], false);
        assert!(outside["relativePath"].is_null());

        // An empty id is the only non-resolving case.
        assert_eq!(resolve("   ").status, "missing");
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
