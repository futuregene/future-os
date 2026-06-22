//! Approval-request Tauri commands. `decide_approval_request` delegates its
//! agent + store orchestration to [`crate::agent_bridge`].

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
