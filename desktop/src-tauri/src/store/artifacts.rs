use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::db::*;
use super::get_thread;
use super::records::*;
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub id: String,
    pub workspace_id: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub title: String,
    pub artifact_type: String,
    pub path: Option<String>,
    pub content: Option<String>,
    pub content_storage: Option<String>,
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

sql_record!(pub(super) ARTIFACT_COLUMNS, artifact_from_row -> ArtifactRecord {
    id, workspace_id, thread_id, run_id, title, artifact_type, path, content,
    content_storage, summary, created_at, updated_at, deleted_at,
});

pub fn list_artifacts(thread_id: &str) -> Result<Vec<ArtifactRecord>, crate::AppError> {
    let thread = loaded(get_thread(thread_id)?, "Thread")?;
    let conn = connect()?;
    // Newest touch first: a row now folds every write/edit of one file (see
    // `ensure_artifact`), so `created_at` would pin a file the Agent just
    // reworked to wherever it first appeared.
    let mut stmt = conn.prepare(&format!(
        "SELECT {ARTIFACT_COLUMNS}
             FROM artifacts
             WHERE deleted_at IS NULL
               AND workspace_id = ?1
               AND (?2 = 'workspace' OR thread_id = ?3)
             ORDER BY updated_at DESC"
    ))?;
    let rows = stmt.query_map(
        params![thread.workspace_id, thread.mode, thread.id],
        artifact_from_row,
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn create_artifact(input: CreateArtifactInput) -> Result<ArtifactRecord, crate::AppError> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("artifact title cannot be empty.".to_string().into());
    }
    let artifact_type = input.artifact_type.trim();
    if artifact_type.is_empty() {
        return Err("artifact type cannot be empty.".to_string().into());
    }

    let id = create_id("artifact");
    let now = now_millis();
    const SQL: &str = "INSERT INTO artifacts (
             id, workspace_id, thread_id, run_id, title, artifact_type, path, content,
             content_storage, summary, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)";
    let conn = connect()?;
    let args = params![
        id,
        input.workspace_id,
        input.thread_id,
        input.run_id,
        title,
        artifact_type,
        input.path,
        input.content,
        input.content_storage,
        input.summary,
        now
    ];
    conn.execute(SQL, args)?;

    loaded(get_artifact(&id)?, "Created artifact")
}

pub fn import_attachment_artifact(
    input: ImportAttachmentArtifactInput,
) -> Result<ArtifactRecord, crate::AppError> {
    let thread = loaded(get_thread(&input.thread_id)?, "Thread")?;
    if thread.mode != "chat" {
        return Err(
            "Attachments are only auto-saved as artifacts for Chat threads."
                .to_string()
                .into(),
        );
    }

    let source_path = PathBuf::from(&input.path);
    if !source_path.is_file() {
        return Err("Attachment path is not a file.".to_string().into());
    }

    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Attachment file name could not be read.".to_string())?;
    let safe_file_name = sanitize_file_name(file_name);
    let artifact_dir = chat_workspace_path(&thread.id)?.join("attachments");
    fs::create_dir_all(&artifact_dir)?;

    let now = now_millis();
    let target_path = unique_attachment_path(&artifact_dir, now, &safe_file_name);
    fs::copy(&source_path, &target_path)?;

    create_artifact(CreateArtifactInput {
        workspace_id: thread.workspace_id,
        thread_id: Some(thread.id),
        run_id: None,
        title: file_name.to_string(),
        artifact_type: artifact_type_from_path(&source_path),
        path: Some(target_path.display().to_string()),
        content: None,
        content_storage: Some("file".to_string()),
        summary: Some("Attached by user.".to_string()),
    })
}

/// Record a file (or inline) artifact produced by a Run, folding repeat touches
/// of the same file into one row.
///
/// A file artifact's identity is its `path` within the Thread: one file written
/// then edited again across several Runs is a single work product, so the Panel
/// must show one row carrying its latest state — not one row per touch. Row
/// identity is enforced by `idx_artifacts_thread_path`. Path-less (inline)
/// artifacts have no such identity and stay keyed by (run_id, title).
pub fn ensure_artifact(input: EnsureArtifactInput) -> Result<(), crate::AppError> {
    // BEGIN IMMEDIATE so the lookup and the write are one atomic transaction;
    // concurrent agent events for the same artifact would otherwise both miss
    // the existing row and insert duplicates.
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let thread_id = run_thread_id(&tx, &input.run_id)?;
    let workspace_id: String = {
        const SQL: &str = "SELECT workspace_id FROM threads WHERE id = ?1";
        tx.query_row(SQL, params![thread_id], |row| row.get(0))?
    };
    let existing: Option<String> = match input.path.as_deref() {
        Some(path) => {
            const SQL: &str = "SELECT id
                 FROM artifacts
                 WHERE thread_id = ?1
                   AND path = ?2
                   AND deleted_at IS NULL
                 LIMIT 1";
            tx.query_row(SQL, params![thread_id, path], |row| row.get(0))
                .optional()?
        }
        None => {
            const SQL: &str = "SELECT id
                 FROM artifacts
                 WHERE run_id = ?1
                   AND title = ?2
                   AND path IS NULL
                   AND deleted_at IS NULL
                 LIMIT 1";
            tx.query_row(SQL, params![input.run_id, input.title], |row| row.get(0))
                .optional()?
        }
    };

    let now = now_millis();
    match existing {
        // Fold this touch into the row: `created_at` keeps the first sighting,
        // `run_id`/`updated_at` move to the latest one.
        Some(id) => {
            const SQL: &str = "UPDATE artifacts
             SET run_id = ?1, title = ?2, artifact_type = ?3, content = ?4,
                 content_storage = ?5, summary = ?6, updated_at = ?7
             WHERE id = ?8";
            let args = params![
                input.run_id,
                input.title,
                input.artifact_type,
                input.content,
                input.content_storage,
                input.summary,
                now,
                id
            ];
            tx.execute(SQL, args)?
        }
        None => {
            const SQL: &str = "INSERT INTO artifacts (
                 id, workspace_id, thread_id, run_id, title, artifact_type, path, content,
                 content_storage, summary, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)";
            let args = params![
                create_id("artifact"),
                workspace_id,
                thread_id,
                input.run_id,
                input.title,
                input.artifact_type,
                input.path,
                input.content,
                input.content_storage,
                input.summary,
                now
            ];
            tx.execute(SQL, args)?
        }
    };
    tx.commit()?;
    Ok(())
}

pub fn artifact_type_from_path(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" => "image",
        "pdf" => "pdf",
        "doc" | "docx" | "md" | "rtf" | "txt" => "document",
        "csv" | "tsv" | "xls" | "xlsx" => "spreadsheet",
        "json" | "jsonl" | "parquet" | "sqlite" | "db" => "data",
        "py" | "rs" | "ts" | "tsx" | "js" | "jsx" | "go" | "java" | "c" | "cpp" | "h" | "hpp" => {
            "code"
        }
        _ => "file",
    }
    .to_string()
}

fn sanitize_file_name(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ if character.is_control() => '_',
            _ => character,
        })
        .collect();

    if sanitized.trim().is_empty() {
        "attachment".to_string()
    } else {
        sanitized
    }
}

fn unique_attachment_path(dir: &Path, now: i64, file_name: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{now}_{file_name}"));
    let mut index = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{now}_{index}_{file_name}"));
        index += 1;
    }
    candidate
}

pub fn get_artifact(id: &str) -> Result<Option<ArtifactRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!("SELECT {ARTIFACT_COLUMNS} FROM artifacts WHERE id = ?1"),
        params![id],
        artifact_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

pub fn delete_artifact(id: &str) -> Result<ArtifactRecord, crate::AppError> {
    let now = now_millis();
    const SQL: &str = "UPDATE artifacts
         SET deleted_at = ?1, updated_at = ?1
         WHERE id = ?2 AND deleted_at IS NULL";
    let conn = connect()?;
    conn.execute(SQL, params![now, id])?;

    loaded(get_artifact(id)?, "Artifact")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::guarded_conn;
    use rusqlite::Connection;

    /// ws1 + chat thread t1 + workspace thread t2 + run r1 (on t1).
    fn seed_graph(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES
                 ('t1', 'ws1', 'chat', 'Chat', 1, 1),
                 ('t2', 'ws1', 'workspace', 'Work', 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES ('r1', 't1', 'running', 1, 1);",
        )
        .expect("seed graph");
    }

    fn create_input(title: &str) -> CreateArtifactInput {
        CreateArtifactInput {
            workspace_id: "ws1".to_string(),
            thread_id: Some("t1".to_string()),
            run_id: None,
            title: title.to_string(),
            artifact_type: "document".to_string(),
            path: None,
            content: Some("body".to_string()),
            content_storage: None,
            summary: None,
        }
    }

    #[test]
    fn artifact_type_classification() {
        let kinds = [
            ("a.PNG", "image"),
            ("a.pdf", "pdf"),
            ("a.md", "document"),
            ("a.xlsx", "spreadsheet"),
            ("a.jsonl", "data"),
            ("a.rs", "code"),
            ("a.unknownext", "file"),
            ("noextension", "file"),
        ];
        for (name, expected) in kinds {
            assert_eq!(artifact_type_from_path(Path::new(name)), expected, "{name}");
        }
    }

    #[test]
    fn sanitize_file_name_replaces_unsafe_chars() {
        assert_eq!(sanitize_file_name("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_file_name("ctrl\u{0007}bell"), "ctrl_bell");
        assert_eq!(sanitize_file_name("  "), "attachment");
        assert_eq!(sanitize_file_name("ok.txt"), "ok.txt");
    }

    #[test]
    fn unique_attachment_path_disambiguates_collisions() {
        let dir = std::env::temp_dir().join(format!("futureos-uap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("7_a.txt"), b"x").unwrap();
        fs::write(dir.join("7_1_a.txt"), b"x").unwrap();
        assert_eq!(
            unique_attachment_path(&dir, 7, "a.txt"),
            dir.join("7_2_a.txt")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_list_delete_artifacts() {
        let (_home, conn) = guarded_conn("artifacts_crud");
        seed_graph(&conn);
        drop(conn);

        // Validation.
        let bad = create_input("  ");
        assert!(create_artifact(bad).is_err(), "blank title rejected");
        let mut bad = create_input("T");
        bad.artifact_type = " ".to_string();
        assert!(create_artifact(bad).is_err(), "blank type rejected");

        let created = create_artifact(create_input("Report")).expect("create");
        assert_eq!(created.title, "Report");

        // Chat thread t1 sees its own artifact; workspace thread t2 sees every
        // artifact of the workspace.
        let chat_list = list_artifacts("t1").expect("list chat");
        assert_eq!(chat_list.len(), 1);
        let ws_list = list_artifacts("t2").expect("list workspace");
        assert_eq!(ws_list.len(), 1);
        assert!(list_artifacts("ghost").is_err());

        // get / delete.
        assert!(get_artifact(&created.id).expect("get").is_some());
        let deleted = delete_artifact(&created.id).expect("delete");
        assert!(deleted.deleted_at.is_some());
        assert!(list_artifacts("t1").expect("list").is_empty());
        assert!(delete_artifact("ghost").is_err());
    }

    #[test]
    fn ensure_artifact_folds_repeat_touches_of_one_file() {
        let (_home, conn) = guarded_conn("artifacts_ensure");
        seed_graph(&conn);
        drop(conn);

        let input = |title: &str, summary: &str| EnsureArtifactInput {
            run_id: "r1".to_string(),
            title: title.to_string(),
            artifact_type: "document".to_string(),
            path: Some("/ws/report.md".to_string()),
            content: None,
            content_storage: Some("file".to_string()),
            summary: Some(summary.to_string()),
        };
        ensure_artifact(input("Report", "first")).expect("first touch");
        ensure_artifact(input("Report v2", "second")).expect("fold");

        let artifacts = list_artifacts("t1").expect("list");
        assert_eq!(artifacts.len(), 1, "one row per (thread, path)");
        assert_eq!(artifacts[0].title, "Report v2");
        assert_eq!(artifacts[0].summary.as_deref(), Some("second"));

        // Path-less artifacts fold by (run, title).
        let inline = |title: &str| EnsureArtifactInput {
            run_id: "r1".to_string(),
            title: title.to_string(),
            artifact_type: "data".to_string(),
            path: None,
            content: Some("[]".to_string()),
            content_storage: None,
            summary: None,
        };
        ensure_artifact(inline("Table")).expect("inline first");
        ensure_artifact(inline("Table")).expect("inline fold");
        ensure_artifact(inline("Other")).expect("inline other");
        let artifacts = list_artifacts("t1").expect("list");
        assert_eq!(artifacts.len(), 3);

        // A missing run errors.
        let mut orphan = inline("X");
        orphan.run_id = "ghost".to_string();
        assert!(ensure_artifact(orphan).is_err());
    }

    #[test]
    fn import_attachment_copies_into_the_chat_workspace() {
        let (_home, conn) = guarded_conn("artifacts_import");
        seed_graph(&conn);
        drop(conn);

        // Source file to import.
        let source = std::env::temp_dir().join(format!("futureos-attach-{}.png", std::process::id()));
        fs::write(&source, b"png-bytes").expect("write source");

        // Workspace-mode thread: rejected.
        assert!(import_attachment_artifact(ImportAttachmentArtifactInput {
            thread_id: "t2".to_string(),
            path: source.display().to_string(),
        })
        .is_err());

        // Missing file: rejected.
        assert!(import_attachment_artifact(ImportAttachmentArtifactInput {
            thread_id: "t1".to_string(),
            path: "/nonexistent/nope.png".to_string(),
        })
        .is_err());

        // Happy path: copied under the chat workspace, artifact recorded.
        let artifact = import_attachment_artifact(ImportAttachmentArtifactInput {
            thread_id: "t1".to_string(),
            path: source.display().to_string(),
        })
        .expect("import");
        assert_eq!(artifact.artifact_type, "image");
        assert_eq!(artifact.summary.as_deref(), Some("Attached by user."));
        let target = artifact.path.expect("path");
        assert!(Path::new(&target).is_file(), "copy exists");
        assert!(target.contains("attachments"));

        // Missing thread errors.
        assert!(import_attachment_artifact(ImportAttachmentArtifactInput {
            thread_id: "ghost".to_string(),
            path: source.display().to_string(),
        })
        .is_err());

        let _ = fs::remove_file(&source);
    }
}
