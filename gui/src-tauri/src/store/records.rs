use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDataPath {
    pub app_dir: String,
    pub db_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecord {
    pub id: String,
    pub workspace_id: String,
    pub mode: String,
    pub title: String,
    pub status: String,
    pub pinned: bool,
    pub readonly: bool,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
    pub agent_session_id: Option<String>,
    pub last_message_at: Option<i64>,
    pub last_opened_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub description: Option<String>,
    pub cleanup_status: String,
    pub cleanup_requested_at: Option<i64>,
    pub cleaned_at: Option<i64>,
    pub last_opened_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub id: String,
    pub thread_id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content_type: String,
    pub content: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub thread_id: String,
    pub trigger_message_id: Option<String>,
    pub status: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub error_message: Option<String>,
    /// Structured error classification. One of:
    /// 'stream_disconnected', 'command_failed', 'model_failed',
    /// 'abort_requested', 'timeout', 'unknown'. NULL when the run did not
    /// fail or the error type is unknown.
    pub error_type: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEventRecord {
    pub id: String,
    pub run_id: String,
    pub event_type: String,
    pub payload: Option<String>,
    pub sequence: i64,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub id: String,
    pub run_id: String,
    pub name: String,
    pub kind: String,
    pub input: Option<String>,
    pub status: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputRecord {
    pub id: String,
    pub tool_call_id: String,
    pub kind: String,
    pub content: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestRecord {
    pub id: String,
    pub thread_id: String,
    pub run_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub summary: Option<String>,
    pub risk_level: Option<String>,
    pub requested_action: Option<String>,
    pub decision_note: Option<String>,
    pub decided_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    // P2: structured action and sandbox boundary
    pub action_category: Option<String>,
    pub action_payload: Option<String>,
    pub sandbox_boundary: Option<String>,
    pub reviewer: String,
    pub decision_scope: String,
    pub decision_source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewChangesetRecord {
    pub id: String,
    pub thread_id: String,
    pub run_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub status: String,
    pub files_changed: i64,
    pub additions: i64,
    pub deletions: i64,
    pub source_kind: String,
    pub workspace_id: Option<String>,
    pub before_snapshot_id: Option<String>,
    pub after_snapshot_id: Option<String>,
    pub binary_files: i64,
    pub omitted_files: i64,
    pub completeness: String,
    pub confidence: String,
    pub overlapped: bool,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileChangeRecord {
    pub id: String,
    pub changeset_id: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub path: Option<String>,
    pub change_type: String,
    pub before_ref: Option<String>,
    pub after_ref: Option<String>,
    pub diff: Option<String>,
    pub summary: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    pub previous_path: Option<String>,
    pub binary: bool,
    pub before_size: Option<i64>,
    pub after_size: Option<i64>,
    pub mime: Option<String>,
    pub diff_truncated: bool,
    pub omission_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotRecord {
    pub id: String,
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
    pub created_at: i64,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchCollectionRecord {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchResourceRecord {
    pub id: String,
    pub collection_id: String,
    pub workspace_id: String,
    pub source_artifact_id: Option<String>,
    pub title: String,
    pub resource_type: String,
    pub source_uri: Option<String>,
    pub content: Option<String>,
    pub content_storage: Option<String>,
    pub summary: Option<String>,
    pub metadata: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReferenceTargetsInput {
    pub workspace_id: String,
    pub query: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceTargetSearchResult {
    pub target_type: String,
    pub target_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub search_text: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug)]
pub struct UpsertToolCallInput {
    pub run_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub kind: String,
    pub input: Option<String>,
    pub status: String,
}

#[derive(Debug)]
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
    pub tool_call_id: String,
    pub kind: String,
    pub title: String,
    pub summary: Option<String>,
    pub risk_level: Option<String>,
    pub requested_action: Option<String>,
    // P2: structured fields. Default to None / "user" / "once" when absent.
    pub action_category: Option<String>,
    pub action_payload: Option<String>,
    pub sandbox_boundary: Option<String>,
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
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
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
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadThinkingLevelInput {
    pub thread_id: String,
    pub thinking_level: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinThreadInput {
    pub thread_id: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SandboxConfigRecord {
    pub id: String,
    pub workspace_id: Option<String>,
    pub mode: String,
    pub writable_roots: Option<String>,
    pub network_access: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ApprovalPolicyConfigRecord {
    pub id: String,
    pub workspace_id: Option<String>,
    pub policy: String,
    pub reviewer: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ApprovalRuleRecord {
    pub id: String,
    pub workspace_id: Option<String>,
    pub scope: String,
    pub match_kind: String,
    pub match_value: String,
    pub decision: String,
    pub enabled: bool,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

// ── Row mappers ─────────────────────────────────────────────────────────────
// `*_from_row` converters live next to the records they build. Column order
// must match the `SELECT` lists in the query modules.

pub(super) fn thread_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadRecord> {
    Ok(ThreadRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        mode: row.get(2)?,
        title: row.get(3)?,
        status: row.get(4)?,
        pinned: row.get::<_, i64>(5)? != 0,
        readonly: row.get::<_, i64>(6)? != 0,
        model_provider: row.get(7)?,
        model_id: row.get(8)?,
        thinking_level: row.get(9)?,
        agent_session_id: row.get(10)?,
        last_message_at: row.get(11)?,
        last_opened_at: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        archived_at: row.get(15)?,
        deleted_at: row.get(16)?,
    })
}

pub(super) fn message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        role: row.get(3)?,
        content_type: row.get(4)?,
        content: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub(super) fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        trigger_message_id: row.get(2)?,
        status: row.get(3)?,
        model_provider: row.get(4)?,
        model_id: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        error_message: row.get(8)?,
        error_type: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub(super) fn workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    Ok(WorkspaceRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        path: row.get(3)?,
        description: row.get(4)?,
        cleanup_status: row.get(5)?,
        cleanup_requested_at: row.get(6)?,
        cleaned_at: row.get(7)?,
        last_opened_at: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        deleted_at: row.get(11)?,
    })
}

pub(super) fn run_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunEventRecord> {
    Ok(RunEventRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        event_type: row.get(2)?,
        payload: row.get(3)?,
        sequence: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub(super) fn tool_call_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCallRecord> {
    Ok(ToolCallRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        name: row.get(2)?,
        kind: row.get(3)?,
        input: row.get(4)?,
        status: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        created_at: row.get(8)?,
    })
}

pub(super) fn tool_output_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolOutputRecord> {
    Ok(ToolOutputRecord {
        id: row.get(0)?,
        tool_call_id: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(super) fn approval_request_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ApprovalRequestRecord> {
    Ok(ApprovalRequestRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        tool_call_id: row.get(3)?,
        kind: row.get(4)?,
        status: row.get(5)?,
        title: row.get(6)?,
        summary: row.get(7)?,
        risk_level: row.get(8)?,
        requested_action: row.get(9)?,
        decision_note: row.get(10)?,
        decided_at: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        action_category: row.get(14)?,
        action_payload: row.get(15)?,
        sandbox_boundary: row.get(16)?,
        reviewer: row.get(17)?,
        decision_scope: row.get(18)?,
        decision_source: row.get(19)?,
    })
}

/// Column list for `review_changeset_from_row`, in struct order. Reuse this in
/// every `SELECT` that maps into `ReviewChangesetRecord`.
pub(super) const REVIEW_CHANGESET_COLUMNS: &str =
    "id, thread_id, run_id, tool_call_id, title, summary, status, \
     files_changed, additions, deletions, source_kind, workspace_id, \
     before_snapshot_id, after_snapshot_id, binary_files, omitted_files, \
     completeness, confidence, overlapped, error_message, created_at, updated_at";

pub(super) fn review_changeset_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewChangesetRecord> {
    Ok(ReviewChangesetRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        tool_call_id: row.get(3)?,
        title: row.get(4)?,
        summary: row.get(5)?,
        status: row.get(6)?,
        files_changed: row.get(7)?,
        additions: row.get(8)?,
        deletions: row.get(9)?,
        source_kind: row.get(10)?,
        workspace_id: row.get(11)?,
        before_snapshot_id: row.get(12)?,
        after_snapshot_id: row.get(13)?,
        binary_files: row.get(14)?,
        omitted_files: row.get(15)?,
        completeness: row.get(16)?,
        confidence: row.get(17)?,
        overlapped: row.get(18)?,
        error_message: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

/// Column list for `review_file_change_from_row`, in struct order.
pub(super) const REVIEW_FILE_CHANGE_COLUMNS: &str =
    "id, changeset_id, target_type, target_id, path, change_type, \
     before_ref, after_ref, diff, summary, additions, deletions, \
     previous_path, binary, before_size, after_size, mime, diff_truncated, \
     omission_reason, created_at, updated_at";

pub(super) fn review_file_change_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewFileChangeRecord> {
    Ok(ReviewFileChangeRecord {
        id: row.get(0)?,
        changeset_id: row.get(1)?,
        target_type: row.get(2)?,
        target_id: row.get(3)?,
        path: row.get(4)?,
        change_type: row.get(5)?,
        before_ref: row.get(6)?,
        after_ref: row.get(7)?,
        diff: row.get(8)?,
        summary: row.get(9)?,
        additions: row.get(10)?,
        deletions: row.get(11)?,
        previous_path: row.get(12)?,
        binary: row.get(13)?,
        before_size: row.get(14)?,
        after_size: row.get(15)?,
        mime: row.get(16)?,
        diff_truncated: row.get(17)?,
        omission_reason: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

/// Column list for `review_snapshot_from_row`, in struct order.
pub(super) const REVIEW_SNAPSHOT_COLUMNS: &str =
    "id, workspace_id, thread_id, run_id, phase, commit_id, tree_id, status, \
     file_count, total_bytes, ignored_count, omitted_count, error_message, created_at";

pub(super) fn review_snapshot_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewSnapshotRecord> {
    Ok(ReviewSnapshotRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        thread_id: row.get(2)?,
        run_id: row.get(3)?,
        phase: row.get(4)?,
        commit_id: row.get(5)?,
        tree_id: row.get(6)?,
        status: row.get(7)?,
        file_count: row.get(8)?,
        total_bytes: row.get(9)?,
        ignored_count: row.get(10)?,
        omitted_count: row.get(11)?,
        error_message: row.get(12)?,
        created_at: row.get(13)?,
    })
}

pub(super) fn artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    Ok(ArtifactRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        thread_id: row.get(2)?,
        run_id: row.get(3)?,
        title: row.get(4)?,
        artifact_type: row.get(5)?,
        path: row.get(6)?,
        content: row.get(7)?,
        content_storage: row.get(8)?,
        summary: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        deleted_at: row.get(12)?,
    })
}

pub(super) fn research_collection_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ResearchCollectionRecord> {
    Ok(ResearchCollectionRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub(super) fn research_resource_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ResearchResourceRecord> {
    Ok(ResearchResourceRecord {
        id: row.get(0)?,
        collection_id: row.get(1)?,
        workspace_id: row.get(2)?,
        source_artifact_id: row.get(3)?,
        title: row.get(4)?,
        resource_type: row.get(5)?,
        source_uri: row.get(6)?,
        content: row.get(7)?,
        content_storage: row.get(8)?,
        summary: row.get(9)?,
        metadata: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}
