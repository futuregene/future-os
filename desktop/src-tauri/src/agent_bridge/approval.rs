//! Approval decisions: notify the agent of a pending decision, persist it, and
//! resume the owning run. Stale requests the agent already dropped are
//! reconciled by cancelling locally.

use super::client::{
    add_session_rule_command, approval_decision_command, connect_agent, get_state_command,
    RpcResponseExt,
};
use crate::store;

/// Inject a same-run "allow in this workspace/chat" rule into the thread's live
/// agent session (in tandem with writing the rule file). Best-effort: a missing
/// session or offline agent just means the rule only applies from the next
/// prompt (when the file is re-read).
pub async fn inject_session_rule(
    thread_id: &str,
    path: &str,
    access: &str,
) -> Result<(), crate::AppError> {
    let Some(thread) = store::get_thread(thread_id)? else {
        return Ok(());
    };
    let session_id = thread.agent_session_id.unwrap_or(thread.id);
    let mut client = connect_agent().await?;
    client
        .execute_command(add_session_rule_command(
            path.to_string(),
            access.to_string(),
            session_id,
        ))
        .await
        .map_err(|error| format!("Unable to inject session rule: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the session rule.")?;
    Ok(())
}

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

/// Reconcile locally-pending approvals against the Agent's authoritative
/// pending set (`get_state.pendingApprovals`).
///
/// Called once after the Agent becomes reachable at startup — after
/// `reconcile_interrupted_runs`, so parked runs are reanimated first — and on
/// every watchdog tick. Startup convergence deliberately leaves pending
/// approvals alone; only this pass, holding the Agent's answer, may settle
/// them:
///
/// - A request still pending Agent-side keeps its card. Its owning run was
///   reanimated back to `running` (or never left it), so CAS it to
///   `waiting_approval` to reflect the wait.
/// - A request the Agent no longer holds was settled while the GUI was down
///   (decided elsewhere, aborted, or lost to an Agent restart) — cancel the
///   stale local row.
/// - A request pending Agent-side with no local row (the GUI crashed between
///   broadcast and persistence) is rebuilt from the payload the Agent serves.
///
/// Transport failures are skipped silently — the next tick retries. Acting
/// only on a definitive Agent answer is what makes this safe: a flaky
/// connection can never wrongly cancel a request the Agent is parked on.
pub async fn reconcile_pending_approvals() {
    let Ok(pending) = store::list_pending_approval_requests() else {
        return;
    };

    // Group by thread so each session is queried exactly once.
    let mut by_thread: std::collections::HashMap<String, Vec<store::ApprovalRequestRecord>> =
        std::collections::HashMap::new();
    for approval in pending {
        by_thread
            .entry(approval.thread_id.clone())
            .or_default()
            .push(approval);
    }

    for (thread_id, approvals) in by_thread {
        let Ok(Some(thread)) = store::get_thread(&thread_id) else {
            continue;
        };
        let Some(session_id) = thread
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(mut client) = connect_agent().await else {
            continue;
        };
        let response = match client
            .execute_command(get_state_command(session_id.clone()))
            .await
        {
            Ok(response) => response.into_inner(),
            Err(_) => continue, // Agent unreachable — retry on the next tick.
        };
        if !response.success {
            // Session unresolvable — leave the rows pending; the run watchdog
            // settles the owning runs, and a later tick retries the approval.
            continue;
        }
        let state: serde_json::Value = future_rpc::decode::response_data(&response);
        let agent_payloads: Vec<&serde_json::Value> = state
            .get("pendingApprovals")
            .and_then(|value| value.as_array())
            .map(|array| array.iter().collect())
            .unwrap_or_default();
        let agent_ids: std::collections::HashSet<&str> = agent_payloads
            .iter()
            .filter_map(|payload| payload["approval_request_id"].as_str())
            .collect();

        for approval in &approvals {
            if agent_ids.contains(approval.id.as_str()) {
                if let Some(run_id) = &approval.run_id {
                    let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                        run_id: run_id.clone(),
                        status: "waiting_approval".to_string(),
                        error_message: None,
                        error_type: None,
                    });
                }
            } else {
                let _ = store::decide_approval_request(store::DecideApprovalRequestInput {
                    approval_request_id: approval.id.clone(),
                    status: "cancelled".to_string(),
                    decision_note: Some(
                        "Settled or cancelled while FutureOS was not running.".to_string(),
                    ),
                });
            }
        }

        // Self-heal: requests the Agent still holds but the GUI never
        // persisted. The approval's thread is derived from its run, so only
        // heal when the parked run has a local row (sessions without local
        // runs are covered by session import, not this path).
        let local_ids: std::collections::HashSet<&str> = approvals
            .iter()
            .map(|approval| approval.id.as_str())
            .collect();
        let active_run_id = state
            .get("activeRun")
            .and_then(|run| run.get("runId"))
            .and_then(|id| id.as_str())
            .map(str::to_string);
        for payload in agent_payloads {
            let Some(id) = payload["approval_request_id"].as_str() else {
                continue;
            };
            if local_ids.contains(id) {
                continue;
            }
            heal_pending_approval_from_agent(active_run_id.as_deref(), payload);
        }
    }
}

/// Rebuild a locally-missing approval row from the payload the Agent serves
/// for a still-parked request. Mirrors the field mapping in
/// `persist::persist_approval_request` — the payload is the exact
/// `approval_request` event data the Agent broadcast.
fn heal_pending_approval_from_agent(active_run_id: Option<&str>, payload: &serde_json::Value) {
    let Some(run_id) = active_run_id else {
        return;
    };
    let Ok(Some(_run)) = store::get_run(run_id) else {
        return;
    };
    let Some(approval_request_id) = payload["approval_request_id"].as_str().map(str::to_string)
    else {
        return;
    };
    let tool_name = payload
        .get("tool_name")
        .and_then(|value| value.as_str())
        .unwrap_or("tool");
    let action_value = payload.get("action");

    if let Err(error) = store::ensure_approval_request(store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id.clone()),
        run_id: run_id.to_string(),
        tool_call_id: payload
            .get("tool_id")
            .and_then(|value| value.as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        kind: payload
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| "tool".to_string()),
        title: payload
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Approve `{tool_name}`")),
        summary: payload
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        risk_level: payload
            .get("risk_level")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        requested_action: payload.get("requested_action").map(compact_json),
        action_category: action_value
            .and_then(|action| action.get("category"))
            .and_then(|category| category.as_str())
            .map(str::to_string),
        action_payload: action_value.map(compact_json),
        sandbox_boundary: payload.get("sandbox_boundary").map(compact_json),
        save_suggestion: payload
            .get("save_suggestion")
            .filter(|value| value.is_object())
            .map(compact_json),
        reviewer: payload
            .get("reviewer")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    }) {
        eprintln!("FutureOS approval heal failed for {approval_request_id}: {error}");
        return;
    }
    // Same CAS as the live persist path: the parked run awaits this decision.
    let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "waiting_approval".to_string(),
        error_message: None,
        error_type: None,
    });
    eprintln!("FutureOS rebuilt missing approval card {approval_request_id} from agent state");
}

fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}
