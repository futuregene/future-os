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
    /// Thread the rule was created from — resolves the target workspace.
    pub thread_id: String,
    pub match_kind: String,
    pub match_value: String,
    /// "session" (this app run) or "always" (permanent).
    pub persistence: String,
    /// "workspace" (default) or "global".
    #[serde(default)]
    pub scope: Option<String>,
}

/// Persist a session/always-allow rule from an approval decision. The agent
/// picks it up on the next prompt via `set_sandbox_policy` (§2.5).
#[tauri::command]
pub fn save_approval_rule(input: SaveApprovalRuleInput) -> Result<(), crate::AppError> {
    let workspace_id = store::get_thread(&input.thread_id)?
        .map(|thread| thread.workspace_id)
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let scope = input.scope.as_deref().unwrap_or("workspace");
    store::save_approval_rule(
        Some(&workspace_id),
        scope,
        &input.match_kind,
        &input.match_value,
        "approve",
        &input.persistence,
    )
}
