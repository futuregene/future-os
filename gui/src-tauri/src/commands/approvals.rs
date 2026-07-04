//! Approval-request Tauri commands. `decide_approval_request` delegates its
//! agent + store orchestration to [`crate::agent_bridge`].

use serde::Deserialize;

use crate::{agent_bridge, store};

#[tauri::command]
pub fn list_approval_requests(
    thread_id: String,
) -> Result<Vec<store::ApprovalRequestRecord>, crate::AppError> {
    store::list_approval_requests(&thread_id)
}

#[tauri::command]
pub async fn decide_approval_request(
    input: store::DecideApprovalRequestInput,
) -> Result<store::ApprovalRequestRecord, crate::AppError> {
    agent_bridge::decide_approval(input).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveApprovalRuleInput {
    /// Thread the rule was created from — resolves the target workspace dir.
    pub thread_id: String,
    /// Path glob (workspace-relative, or `~`/absolute), possibly user-edited.
    pub path: String,
    /// "read" | "write".
    pub access: String,
}

/// "Allow in this workspace": append an allow rule to the workspace's
/// `.future/approval_rule.json` (APPROVAL_PLAN.md §6). The agent reads that
/// file directly on its next prompt.
#[tauri::command]
pub fn save_approval_rule(input: SaveApprovalRuleInput) -> Result<(), crate::AppError> {
    let workspace_id = store::get_thread(&input.thread_id)?
        .map(|thread| thread.workspace_id)
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    crate::approval_rules::append_workspace_allow_rule(
        &workspace.path,
        &input.path,
        &input.access,
    )
}
