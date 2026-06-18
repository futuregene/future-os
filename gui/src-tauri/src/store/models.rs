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
    pub created_at: i64,
    pub updated_at: i64,
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

#[derive(Debug)]
pub struct EnsureReviewChangeInput {
    pub run_id: String,
    pub tool_call_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub path: Option<String>,
    pub change_type: String,
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
pub struct UpdateReviewChangesetStatusInput {
    pub changeset_id: String,
    pub status: String,
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
