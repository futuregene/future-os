//! Approval decisions: notify the agent of a pending decision, persist it, and
//! resume the owning run. Stale requests the agent already dropped are
//! reconciled by cancelling locally.

use super::client::{approval_decision_command, connect_agent, RpcResponseExt};
use crate::store;

async fn notify_agent_approval_decision(
    approval: &store::ApprovalRequestRecord,
    input: &store::DecideApprovalRequestInput,
) -> Result<(), crate::AppError> {
    let thread = store::get_thread(&approval.thread_id)?
        .ok_or_else(|| "Approval thread could not be loaded.".to_string())?;
    let mut client = connect_agent().await?;
    client
        .execute_command(approval_decision_command(
            approval.id.clone(),
            input.status.clone(),
            input.decision_note.clone().unwrap_or_default(),
            thread.agent_session_id.unwrap_or(thread.id),
        ))
        .await
        .map_err(|error| format!("Unable to send approval decision to Future Agent: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the approval decision.")?;
    Ok(())
}

/// Record an approval decision: notify the agent while the request is still
/// pending, persist the decision, and resume the owning run. A request the
/// agent already dropped is reconciled by cancelling it locally.
pub async fn decide_approval(
    input: store::DecideApprovalRequestInput,
) -> Result<store::ApprovalRequestRecord, crate::AppError> {
    let current = store::get_approval_request(&input.approval_request_id)?
        .ok_or_else(|| "Approval request could not be loaded.".to_string())?;
    if current.status == "pending" {
        if let Err(error) = notify_agent_approval_decision(&current, &input).await {
            if is_stale_approval_error(&error.to_string()) {
                return store::decide_approval_request(store::DecideApprovalRequestInput {
                    approval_request_id: input.approval_request_id,
                    status: "cancelled".to_string(),
                    decision_note: Some("Cancelled because the approval request is no longer active in Future Agent.".to_string()),
                });
            }
            return Err(error);
        }
    }
    let updated = store::decide_approval_request(input)?;
    if let Some(run_id) = &updated.run_id {
        // Compare-and-set: resume the owning run only while it is non-terminal,
        // so a concurrent `abort_run` that already set `cancelled` is never
        // overwritten back to `running` by this late read-then-write (B-13).
        let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run_id.clone(),
            status: "running".to_string(),
            error_message: None,
            error_type: None,
        });
    }
    Ok(updated)
}

fn is_stale_approval_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("approval request") && normalized.contains("not pending")
}
