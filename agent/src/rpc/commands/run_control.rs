//! Run-lifecycle command handlers: prompt enqueue, cancellation, abort,
//! persistence recovery, and approval decisions.

use std::sync::Arc;

use crate::rpc::{
    AppState, ApprovalDecision, ApprovalDecisionStatus, RpcCommand, RpcResponse, ServerSession,
};

pub(crate) fn handle_prompt(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    if state
        .shutting_down
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return RpcResponse::build_fail(
            id,
            "prompt",
            "agent is shutting down; no new prompts accepted",
        );
    }
    // NOTE: `session` was already resolved by the session-scoped guard in the
    // dispatcher — re-fetching it here can never fail, so the old
    // "session does not exist" arm was unreachable dead code.
    let mut sess = session.write();
    let busy_policy = match crate::runtime::BusyPolicy::parse(&cmd.busy_policy) {
        Ok(policy) => policy,
        Err(error) => {
            return RpcResponse::build_fail_code(
                id,
                "prompt",
                "invalid_busy_policy",
                &error,
                serde_json::json!({
                    "provided": cmd.busy_policy,
                    "valid": crate::runtime::BusyPolicy::VALID_VALUES,
                }),
            );
        }
    };
    let client_request_id = if cmd.client_request_id.is_empty() {
        format!("request_{}", uuid::Uuid::new_v4().simple())
    } else {
        cmd.client_request_id.clone()
    };
    match sess.enqueue_prompt_with_model_context(
        crate::rpc::session_prompt::PromptText::new(&cmd.message, &cmd.model_context),
        &cmd.images,
        &cmd.attachments,
        (!cmd.requested_run_id.is_empty()).then_some(cmd.requested_run_id.as_str()),
        &client_request_id,
        busy_policy,
    ) {
        Ok(ack) => RpcResponse::ok(id, "prompt", serde_json::to_value(ack).unwrap_or_default()),
        Err(error) => {
            if let Some(queue_error) = error.downcast_ref::<crate::runtime::RunQueueError>() {
                let (code, details) = match queue_error {
                    crate::runtime::RunQueueError::DuplicateRequestConflict(_) => (
                        "duplicate_request_conflict",
                        serde_json::json!({"client_request_id": client_request_id}),
                    ),
                    crate::runtime::RunQueueError::QueueFull { limit }
                    | crate::runtime::RunQueueError::GlobalQueueFull { limit } => {
                        ("queue_full", serde_json::json!({"limit": limit}))
                    }
                    crate::runtime::RunQueueError::RequestTooLarge { actual, limit } => (
                        "request_too_large",
                        serde_json::json!({"actual_bytes": actual, "limit_bytes": limit}),
                    ),
                    crate::runtime::RunQueueError::QueueBytesExceeded { actual, limit }
                    | crate::runtime::RunQueueError::GlobalQueueBytesExceeded { actual, limit } => {
                        (
                            "queue_memory_limit",
                            serde_json::json!({"actual_bytes": actual, "limit_bytes": limit}),
                        )
                    }
                    crate::runtime::RunQueueError::Deleting => (
                        "deleting",
                        serde_json::json!({"session_id": cmd.session_id}),
                    ),
                    crate::runtime::RunQueueError::PersistenceUnavailable(reason) => (
                        "persistence_unavailable",
                        serde_json::json!({"reason": reason}),
                    ),
                    crate::runtime::RunQueueError::InvalidRunId(run_id) => {
                        ("invalid_run_id", serde_json::json!({"run_id": run_id}))
                    }
                    _ => ("scheduler_error", serde_json::json!({})),
                };
                RpcResponse::build_fail_code(id, "prompt", code, &queue_error.to_string(), details)
            } else {
                RpcResponse::build_fail(id, "prompt", &error.to_string())
            }
        }
    }
}

pub(crate) fn handle_cancel_queued_run(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    if cmd.run_id.is_empty() {
        return RpcResponse::build_fail_code(
            id,
            "cancel_queued_run",
            "run_not_queued",
            "run_id is required",
            serde_json::json!({}),
        );
    }
    match session.write().cancel_queued_run(
        &cmd.run_id,
        crate::runtime::QueuedCancellationReason::Cancelled,
    ) {
        Ok(cancelled) => RpcResponse::ok(
            id,
            "cancel_queued_run",
            serde_json::json!({
                "run_id": cancelled.run_id,
                "run_sequence": cancelled.run_sequence,
                "state": "cancelled",
                "reason": "cancelled",
            }),
        ),
        Err(error) => RpcResponse::build_fail_code(
            id,
            "cancel_queued_run",
            "run_not_queued",
            &error.to_string(),
            serde_json::json!({"run_id": cmd.run_id}),
        ),
    }
}

pub(crate) fn handle_prune_run_events(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    if cmd.run_id.is_empty()
        || !cmd
            .run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return RpcResponse::build_fail_code(
            id,
            "prune_run_events",
            "invalid_run_id",
            "run_id is required and must be a safe identifier",
            serde_json::json!({"run_id": cmd.run_id}),
        );
    }
    let session = session.read();
    if session
        .scheduler
        .active()
        .is_some_and(|(active, _)| active.run_id == cmd.run_id)
    {
        return RpcResponse::build_fail_code(
            id,
            "prune_run_events",
            "run_active",
            "cannot prune the journal of an active run",
            serde_json::json!({"run_id": cmd.run_id}),
        );
    }
    let path = session
        .session_manager
        .run_data_path(&cmd.session_id)
        .join(format!("{}.jsonl", cmd.run_id));
    match std::fs::remove_file(&path) {
        Ok(()) => RpcResponse::ok(id, "prune_run_events", serde_json::json!({"pruned": true})),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            RpcResponse::ok(id, "prune_run_events", serde_json::json!({"pruned": true}))
        }
        Err(error) => RpcResponse::build_fail_code(
            id,
            "prune_run_events",
            "prune_failed",
            &error.to_string(),
            serde_json::json!({"run_id": cmd.run_id, "retryable": true}),
        ),
    }
}

pub(crate) fn handle_abort_session(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let (active_run_id, cancelled) = {
        let mut sess = session.write();
        let cancelled =
            sess.cancel_all_queued_runs(crate::runtime::QueuedCancellationReason::Cancelled);
        let active_run_id = sess.runtime.snapshot().map(|active| active.run_id);
        if let Some(run_id) = active_run_id.as_deref() {
            let _ = sess.runtime.request_abort(Some(run_id));
        }
        (active_run_id, cancelled)
    };
    RpcResponse::ok(
        id,
        "abort_session",
        serde_json::json!({
            "active_run_id": active_run_id,
            "queued_cancelled": cancelled.len(),
            "state": "cancelling",
        }),
    )
}

pub(crate) fn handle_retry_persistence(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    match session.write().recover_persistence_degraded() {
        Ok(lease) => RpcResponse::ok(
            id,
            "retry_persistence",
            serde_json::json!({
                "run_id": lease.run_id,
                "state": "interrupted",
                "recovered": true,
            }),
        ),
        Err(error) => RpcResponse::build_fail_code(
            id,
            "retry_persistence",
            "persistence_recovery_failed",
            &error.to_string(),
            serde_json::json!({}),
        ),
    }
}

pub(crate) fn handle_abort(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    // abort() only needs &self — take a read lock so a concurrent
    // reader (get_state polling) can never make the abort a no-op,
    // which a failed try_write() silently did.
    let abort_result = {
        let sess = session.read();
        tracing::info!(
            session_id = %sess.session_id,
            requested_run_id = %cmd.run_id,
            source = "rpc_abort",
            "session abort requested by RPC client"
        );
        sess.abort_run((!cmd.run_id.is_empty()).then_some(cmd.run_id.as_str()))
            .map(|()| sess.session_id.clone())
    };
    let session_id = match abort_result {
        Ok(session_id) => session_id,
        Err(error) => return RpcResponse::build_fail(id, "abort", &error.to_string()),
    };
    state
        .approval_gate
        .cancel_session(&session_id, "Cancelled because the run was terminated.");
    RpcResponse::ok(id, "abort", serde_json::json!({"run_id": cmd.run_id}))
}

pub(crate) fn handle_approval_decision(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    let (approved, status) = match cmd.mode.as_str() {
        "approved" => (true, ApprovalDecisionStatus::Approved),
        "rejected" => (false, ApprovalDecisionStatus::Rejected),
        "cancelled" => (false, ApprovalDecisionStatus::Cancelled),
        _ => {
            return RpcResponse::build_fail(
                id,
                "approval_decision",
                "mode must be approved, rejected, or cancelled",
            );
        }
    };
    match state.approval_gate.decide(
        &cmd.entry_id,
        &cmd.session_id,
        ApprovalDecision {
            approved,
            note: cmd.message.clone(),
            status,
        },
    ) {
        Ok(()) => RpcResponse::ok(
            id,
            "approval_decision",
            serde_json::json!({"approvalRequestId": cmd.entry_id, "status": cmd.mode}),
        ),
        Err(error) => RpcResponse::build_fail(id, "approval_decision", &error),
    }
}

pub(crate) fn handle_abort_retry(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    session.read().abort();
    RpcResponse::ok(id, "abort_retry", serde_json::json!({}))
}
