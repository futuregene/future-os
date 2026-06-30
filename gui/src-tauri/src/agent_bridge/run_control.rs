//! Run control: abort an in-flight agent run, mark a run failed, and wait for
//! the agent to confirm idle before snapshotting. These back the abort command
//! and the parent module's prompt finalization.

use super::client::{base_command, connect_agent, get_state_command, RpcResponseExt};
use crate::store;

pub(super) async fn abort_agent_thread(thread_id: &str) -> Result<(), crate::AppError> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let mut client = connect_agent().await?;
    client
        .execute_command(base_command(
            "abort",
            thread.agent_session_id.unwrap_or(thread.id),
        ))
        .await
        .map_err(|error| format!("Unable to abort Future Agent run: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the abort request.")?;
    Ok(())
}

/// Abort an in-flight agent run, then mark its store run cancelled. A missing
/// agent (e.g. the backend is down) is tolerated — the run is still cancelled
/// locally so the UI doesn't strand on a "running" row.
pub async fn abort_run(
    thread_id: String,
    run_id: String,
) -> Result<store::RunRecord, crate::AppError> {
    if let Err(error) = abort_agent_thread(&thread_id).await {
        if !is_agent_unavailable_error(&error) {
            return Err(error);
        }
        eprintln!("FutureOS agent abort skipped because agent is unavailable: {error}");
    }
    store::update_run_status(store::UpdateRunStatusInput {
        run_id,
        status: "cancelled".to_string(),
        error_message: Some("Terminated by user.".to_string()),
        error_type: Some("abort_requested".to_string()),
    })
}

pub(super) fn mark_run_failed_if_active(run_id: Option<&str>, error: &str) {
    let Some(run_id) = run_id else {
        return;
    };
    let error_type = crate::run_error::classify_run_error(error);
    // Compare-and-set: only fails a run that isn't already terminal, atomically,
    // so a concurrent `abort_run` (which sets `cancelled`) is never overwritten.
    if let Err(update_error) = store::fail_run_if_active(run_id, error, error_type) {
        eprintln!("FutureOS run failure status update failed: {update_error}");
    }
}

/// Poll the Agent's `get_state.isStreaming` until it reports idle (or a short
/// timeout / the agent disappears). Best-effort confirmation that the Agent has
/// stopped writing files before the after snapshot (§6.2).
pub(super) async fn wait_for_agent_idle(session_id: &str) {
    let Ok(mut client) = connect_agent().await else {
        return;
    };
    // ~5s budget at 200ms intervals.
    for _ in 0..25 {
        match client
            .execute_command(get_state_command(session_id.to_string()))
            .await
        {
            Ok(response) => {
                let data = response.into_inner().data;
                let streaming = serde_json::from_str::<serde_json::Value>(&data)
                    .ok()
                    .and_then(|value| value.get("isStreaming").and_then(|s| s.as_bool()))
                    .unwrap_or(false);
                if !streaming {
                    return;
                }
            }
            // Agent unreachable → treat as idle; nothing more we can confirm.
            Err(_) => return,
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

fn is_agent_unavailable_error(error: &crate::AppError) -> bool {
    matches!(error, crate::AppError::AgentUnavailable(_))
}
