//! Cross-domain DTOs shared across store modules: command-input types, the
//! markdown-reference resolution types, and the P2 approval-config scaffolding.
//!
//! Output record structs and their `*_COLUMNS` / `*_from_row` mappers live
//! beside the queries that build them, in each domain module (`threads.rs`,
//! `runs.rs`, `workspaces.rs`, …).

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDataPath {
    pub app_dir: String,
    pub db_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCleanupSummary {
    pub thread_id: String,
    pub workspace_id: String,
    pub workspace_kind: String,
    pub workspace_path: String,
    pub cleanup_status: String,
    pub artifact_count: i64,
    pub workspace_file_count: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveMarkdownReferencesInput {
    pub workspace_id: String,
    pub references: Vec<MarkdownReferenceInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownReferenceInput {
    pub target_type: String,
    pub target_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMarkdownReference {
    pub target_type: String,
    pub target_id: String,
    pub status: String,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct UpsertToolCallInput {
    pub run_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub kind: String,
    pub input: Option<String>,
    pub status: String,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct CompleteToolCallInput {
    pub run_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub status: String,
    pub output_kind: String,
    pub output_content: Option<String>,
}

#[derive(Debug)]
pub struct EnsureApprovalRequestInput {
    pub approval_request_id: Option<String>,
    pub run_id: String,
    /// `None` for escalation approvals (no owning tool_call) → stored NULL.
    pub tool_call_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub summary: Option<String>,
    pub risk_level: Option<String>,
    pub requested_action: Option<String>,
    // P2: structured fields. Default to None / "user" / "once" when absent.
    pub action_category: Option<String>,
    pub action_payload: Option<String>,
    pub sandbox_boundary: Option<String>,
    // Phase 2: suggested rule (JSON) for session/always-allow persistence.
    pub save_suggestion: Option<String>,
    pub reviewer: Option<String>,
}

/// Insert a before/after shadow snapshot row (see gui/ER.md §4.10).
#[derive(Debug, Default)]
pub struct CreateReviewSnapshotInput {
    pub workspace_id: String,
    pub thread_id: String,
    pub run_id: String,
    pub phase: String,
    pub commit_id: Option<String>,
    pub tree_id: Option<String>,
    pub status: String,
    pub file_count: i64,
    pub total_bytes: i64,
    pub ignored_count: i64,
    pub omitted_count: i64,
    pub error_message: Option<String>,
}

/// Create-or-replace the single `run_snapshot` changeset for a Run (§8.2).
#[derive(Debug, Default)]
pub struct UpsertRunChangesetInput {
    pub run_id: String,
    pub thread_id: String,
    pub workspace_id: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub before_snapshot_id: Option<String>,
    pub after_snapshot_id: Option<String>,
    pub files_changed: i64,
    pub additions: i64,
    pub deletions: i64,
    pub binary_files: i64,
    pub omitted_files: i64,
    pub completeness: String,
    pub confidence: String,
    pub error_message: Option<String>,
    pub files: Vec<InsertReviewFileChangeInput>,
}

/// One file row inside a `run_snapshot` changeset (§8.3).
#[derive(Debug, Default)]
pub struct InsertReviewFileChangeInput {
    pub path: Option<String>,
    pub previous_path: Option<String>,
    pub change_type: String,
    pub diff: Option<String>,
    pub summary: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    pub binary: bool,
    pub before_size: Option<i64>,
    pub after_size: Option<i64>,
    pub mime: Option<String>,
    pub diff_truncated: bool,
    pub omission_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThreadInput {
    pub mode: String,
    pub title: Option<String>,
    pub workspace_id: Option<String>,
    pub workspace_path: Option<String>,
    pub workspace_name: Option<String>,
    // model_provider, model_id, thinking_level removed — now read from agent
    #[serde(default)]
    pub agent_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceInput {
    pub name: Option<String>,
    pub path: String,
    pub description: Option<String>,
    pub create_directory: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateArtifactInput {
    pub workspace_id: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub title: String,
    pub artifact_type: String,
    pub path: Option<String>,
    pub content: Option<String>,
    pub content_storage: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAttachmentArtifactInput {
    pub thread_id: String,
    pub path: String,
}

#[derive(Debug)]
pub struct EnsureArtifactInput {
    pub run_id: String,
    pub title: String,
    pub artifact_type: String,
    pub path: Option<String>,
    pub content: Option<String>,
    pub content_storage: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendMessageInput {
    pub thread_id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content_type: Option<String>,
    pub content: String,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRunInput {
    pub thread_id: String,
    pub trigger_message_id: Option<String>,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRunStatusInput {
    pub run_id: String,
    pub status: String,
    pub error_message: Option<String>,
    /// Optional structured error classification. See RunRecord::error_type.
    /// When None, the existing error_type column is preserved.
    #[serde(default)]
    pub error_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendRunEventInput {
    pub run_id: String,
    pub event_type: String,
    pub payload: Option<String>,
    pub sequence: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideApprovalRequestInput {
    pub approval_request_id: String,
    pub status: String,
    pub decision_note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameThreadInput {
    pub thread_id: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadModelInput {
    pub thread_id: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub model_provider: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadThinkingLevelInput {
    pub thread_id: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub thinking_level: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinThreadInput {
    pub thread_id: String,
    pub pinned: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameWorkspaceInput {
    pub workspace_id: String,
    pub name: String,
}
