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
    let conn = connect()?;
    conn.execute(
        "INSERT INTO artifacts (
             id, workspace_id, thread_id, run_id, title, artifact_type, path, content,
             content_storage, summary, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
        params![
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
        ],
    )?;

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
    let workspace_id: String = tx.query_row(
        "SELECT workspace_id FROM threads WHERE id = ?1",
        params![thread_id],
        |row| row.get(0),
    )?;
    let existing: Option<String> = match input.path.as_deref() {
        Some(path) => tx
            .query_row(
                "SELECT id
                 FROM artifacts
                 WHERE thread_id = ?1
                   AND path = ?2
                   AND deleted_at IS NULL
                 LIMIT 1",
                params![thread_id, path],
                |row| row.get(0),
            )
            .optional()?,
        None => tx
            .query_row(
                "SELECT id
                 FROM artifacts
                 WHERE run_id = ?1
                   AND title = ?2
                   AND path IS NULL
                   AND deleted_at IS NULL
                 LIMIT 1",
                params![input.run_id, input.title],
                |row| row.get(0),
            )
            .optional()?,
    };

    let now = now_millis();
    match existing {
        // Fold this touch into the row: `created_at` keeps the first sighting,
        // `run_id`/`updated_at` move to the latest one.
        Some(id) => tx.execute(
            "UPDATE artifacts
             SET run_id = ?1, title = ?2, artifact_type = ?3, content = ?4,
                 content_storage = ?5, summary = ?6, updated_at = ?7
             WHERE id = ?8",
            params![
                input.run_id,
                input.title,
                input.artifact_type,
                input.content,
                input.content_storage,
                input.summary,
                now,
                id
            ],
        )?,
        None => tx.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, run_id, title, artifact_type, path, content,
                 content_storage, summary, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
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
            ],
        )?,
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
    let conn = connect()?;
    conn.execute(
        "UPDATE artifacts
         SET deleted_at = ?1, updated_at = ?1
         WHERE id = ?2 AND deleted_at IS NULL",
        params![now, id],
    )?;

    loaded(get_artifact(id)?, "Artifact")
}
