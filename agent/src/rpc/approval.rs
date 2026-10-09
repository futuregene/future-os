mod escalation;
mod shape;
use escalation::{
    escalation_save_suggestion, extract_blocked_paths_raw, extract_denial_paths, shorten_home,
};
use shape::{
    approval_shape, command_summary, normalize_requested_action, shell_auto_allow,
    shell_command_shape, windows_capability_shape,
};

use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::Path,
    sync::{mpsc, Arc},
};

use super::{SseBroadcaster, SseEvent};
use crate::sandbox::rules::{Decision, Op};
use crate::sandbox::windows_request::AdditionalPermissions;
use crate::sandbox::{paths, EscalationDecision, EscalationRequest, ResolvedSandbox};

#[derive(Clone, Default)]
pub struct ApprovalGate {
    pending: Arc<Mutex<HashMap<String, PendingApproval>>>,
    pub(crate) generation: Arc<std::sync::atomic::AtomicU64>,
    review: Option<crate::approval_review::RunApprovalReviewer>,
}

#[derive(Debug, Clone)]
pub struct ApprovalDecision {
    pub approved: bool,
    pub note: String,
    pub status: ApprovalDecisionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecisionStatus {
    Approved,
    Rejected,
    Cancelled,
}

struct PendingApproval {
    session_id: String,
    /// The exact `approval_request` event payload, kept so get_state can
    /// re-serve the card to a client that missed the broadcast (e.g. a GUI
    /// restart while the run is parked on this decision).
    payload: serde_json::Value,
    tx: mpsc::Sender<ApprovalDecision>,
}

/// Outcome of an approval decision (shared by tool approvals and
/// sandbox escalations).
enum ApprovalOutcome {
    Approved(String),
    Rejected(String),
    Cancelled(String),
}

impl ApprovalGate {
    pub(crate) fn observe_review_message(&self, message: &crate::types::AgentMessage) {
        if let Some(context) = &self.review {
            context.observe_message(message);
        }
    }
    pub(crate) fn for_run(
        &self,
        user_request: String,
        history: &[crate::types::AgentMessage],
        current_source_id: &str,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        annotations: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    ) -> Self {
        self.with_model_reviewer(
            crate::approval_review::RunApprovalReviewer::new(
                user_request,
                cancelled,
                self.generation.clone(),
                annotations,
            )
            .with_evidence_snapshot(history)
            .with_current_source(current_source_id),
        )
    }

    pub(crate) fn with_model_reviewer(
        &self,
        context: crate::approval_review::RunApprovalReviewer,
    ) -> Self {
        Self {
            review: Some(context),
            ..self.clone()
        }
    }
    fn rejection_actor(&self) -> &str {
        if self.review.is_some() {
            "the automatic reviewer"
        } else {
            "the user"
        }
    }
    pub(crate) fn invalidate(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &self,
        broadcaster: &SseBroadcaster,
        session_id: &str,
        cwd: &str,
        tool_name: &str,
        tool_id: &str,
        arguments: &serde_json::Value,
        sandbox: &ResolvedSandbox,
    ) -> Option<crate::types::ToolCallResult> {
        // Off tier runs fully open — no approval at all.
        if !sandbox.enabled() {
            return None;
        }
        // shell. Sandbox tier wraps it in Seatbelt (no pre-approval; boundary
        // hits surface via escalation). Manual tier gates it with a read-only
        // whitelist (Option B): known-safe commands auto-run, everything else
        // asks. shell approvals are allow-once only (no persisted command rule).
        if tool_name == "shell" {
            if let Some(raw_permissions) = arguments
                .get("additional_permissions")
                .or_else(|| arguments.get("additionalPermissions"))
                .filter(|value| !value.is_null())
            {
                let permissions = match serde_json::from_value::<AdditionalPermissions>(
                    raw_permissions.clone(),
                ) {
                    Ok(permissions) => permissions,
                    Err(error) => {
                        return Some(crate::types::ToolCallResult {
                            result: format!(
                                "Tool call `shell` has invalid additional_permissions: {error}"
                            ),
                            is_error: true,
                        });
                    }
                };
                if !permissions.write.is_empty() {
                    return self.request_windows_capability(
                        broadcaster,
                        session_id,
                        tool_id,
                        arguments,
                        sandbox,
                        &permissions,
                    );
                }
            }
            if sandbox.wraps_shell() {
                return None;
            }
            let command = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if shell_auto_allow(command) {
                return None;
            }
            let shape = shell_command_shape(command, sandbox);
            let outcome = self.resolve_approval(
                broadcaster,
                session_id,
                tool_id,
                tool_name,
                &shape,
                normalize_requested_action(arguments),
            );
            crate::tools::shell::record_gate_outcome(match &outcome {
                ApprovalOutcome::Approved(_) => "approved",
                ApprovalOutcome::Rejected(_) => "denied",
                ApprovalOutcome::Cancelled(_)
                    if self.review.as_ref().is_some_and(|context| {
                        context.invalidated().is_some_and(|verdict| {
                            verdict.status == crate::approval_review::ReviewStatus::StaleRequest
                        })
                    }) =>
                {
                    "context_invalidated"
                }
                ApprovalOutcome::Cancelled(_) => "cancelled",
            });
            return match outcome {
                ApprovalOutcome::Approved(_) => None,
                ApprovalOutcome::Cancelled(_) => Some(crate::types::ToolCallResult {
                    result: "Tool call `shell` was cancelled because the approval request ended."
                        .to_string(),
                    is_error: true,
                }),
                ApprovalOutcome::Rejected(note) => Some(crate::types::ToolCallResult {
                    result: format!(
                        "Tool call `shell` was rejected by {}{}.",
                        self.rejection_actor(),
                        if note.is_empty() {
                            String::new()
                        } else {
                            format!(": {note}")
                        }
                    ),
                    is_error: true,
                }),
            };
        }
        // Remaining file-touching tools are pure file-path decisions.
        let op = match tool_name {
            "read" => Op::Read,
            "write" | "edit" => Op::Write,
            _ => return None,
        };
        let raw_path = super::argument_path(arguments)?;
        // Canonicalize once so path_within / strip_prefix agree with the
        // canonicalized sandbox.workspace (symlink + case correct, §3.5).
        let path = paths::canonicalize_lenient(&paths::resolve_against(Path::new(cwd), &raw_path));

        match sandbox.evaluate(&path, op) {
            Decision::Allow => {
                if op == Op::Write {
                    crate::tools::approve_outside_path(&path.to_string_lossy());
                }
                None
            }
            Decision::Deny => Some(crate::types::ToolCallResult {
                result: format!(
                    "Tool call `{tool_name}` was denied by an approval rule for {}.",
                    path.display()
                ),
                is_error: true,
            }),
            Decision::Ask => {
                let shape = approval_shape(tool_name, &path, op, arguments, sandbox);
                let outcome = self.resolve_approval(
                    broadcaster,
                    session_id,
                    tool_id,
                    tool_name,
                    &shape,
                    normalize_requested_action(arguments),
                );
                match outcome {
                    ApprovalOutcome::Approved(_) => {
                        if op == Op::Write {
                            crate::tools::approve_outside_path(&path.to_string_lossy());
                        }
                        None
                    }
                    ApprovalOutcome::Cancelled(_) => Some(crate::types::ToolCallResult {
                        result: format!(
                            "Tool call `{tool_name}` was cancelled because the approval request ended."
                        ),
                        is_error: true,
                    }),
                    ApprovalOutcome::Rejected(note) => Some(crate::types::ToolCallResult {
                        result: format!(
                            "Tool call `{}` was rejected by {}{}.",
                            tool_name,
                            self.rejection_actor(),
                            if note.is_empty() {
                                String::new()
                            } else {
                                format!(": {note}")
                            }
                        ),
                        is_error: true,
                    }),
                }
            }
        }
    }

    fn request_windows_capability(
        &self,
        broadcaster: &SseBroadcaster,
        session_id: &str,
        tool_id: &str,
        arguments: &serde_json::Value,
        sandbox: &ResolvedSandbox,
        permissions: &AdditionalPermissions,
    ) -> Option<crate::types::ToolCallResult> {
        // The field is intentionally rejected everywhere except an active
        // Windows write-protection backend. In particular it must not turn
        // into an ordinary Manual-tier command approval and then run open.
        if !cfg!(windows) || !sandbox.wraps_shell() {
            return Some(crate::types::ToolCallResult {
                result: "Tool call `shell` requested Windows write capabilities, but the Windows write-protection backend is not active.".to_string(),
                is_error: true,
            });
        }

        let command = arguments
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let prepared = match crate::sandbox::windows_request::prepare(sandbox, command, permissions)
        {
            Ok(prepared) => prepared,
            Err(error) => {
                return Some(crate::types::ToolCallResult {
                    result: format!(
                        "Tool call `shell` has an invalid write capability request: {error}"
                    ),
                    is_error: true,
                });
            }
        };
        if !prepared.needs_approval() {
            return None;
        }

        let shape = windows_capability_shape(command, &prepared, sandbox);
        let outcome = self.resolve_approval(
            broadcaster,
            session_id,
            tool_id,
            "shell",
            &shape,
            normalize_requested_action(arguments),
        );
        crate::tools::shell::record_gate_outcome(match &outcome {
            ApprovalOutcome::Approved(_) => "approved",
            ApprovalOutcome::Rejected(_) => "denied",
            ApprovalOutcome::Cancelled(_)
                if self.review.as_ref().is_some_and(|context| {
                    context.invalidated().is_some_and(|verdict| {
                        verdict.status == crate::approval_review::ReviewStatus::StaleRequest
                    })
                }) =>
            {
                "context_invalidated"
            }
            ApprovalOutcome::Cancelled(_) => "cancelled",
        });
        match outcome {
            ApprovalOutcome::Approved(request_id) => {
                let Some(receipt) = prepared.approved_receipt(request_id) else {
                    return Some(crate::types::ToolCallResult {
                        result: "Tool call `shell` approval did not match a capability request."
                            .to_string(),
                        is_error: true,
                    });
                };
                crate::tools::approve_windows_capability(receipt);
                None
            }
            ApprovalOutcome::Cancelled(_) => Some(crate::types::ToolCallResult {
                result: "Tool call `shell` was cancelled because the approval request ended."
                    .to_string(),
                is_error: true,
            }),
            ApprovalOutcome::Rejected(note) => Some(crate::types::ToolCallResult {
                result: format!(
                    "Tool call `shell` was rejected by {}{}.",
                    self.rejection_actor(),
                    if note.is_empty() {
                        String::new()
                    } else {
                        format!(": {note}")
                    }
                ),
                is_error: true,
            }),
        }
    }

    /// Post-hoc approval for running a shell command outside the sandbox
    /// (docs/internals/desktop/SANDBOX/COMMON.md). Approval means the single re-run happens
    /// unsandboxed — "this exact command, once".
    pub fn request_escalation(
        &self,
        broadcaster: &SseBroadcaster,
        session_id: &str,
        request: &EscalationRequest,
        sandbox: &ResolvedSandbox,
    ) -> EscalationDecision {
        let raw_blocked = extract_blocked_paths_raw(&request.failure_summary);
        let blocked_paths: Vec<String> = raw_blocked.iter().map(|p| shorten_home(p)).collect();
        // Offer "allow in this workspace" (a path rule) when the blocked paths
        // are non-secret — a persisted rule then makes that dir writable in the
        // sandbox, so future shell runs there don't re-escalate. Secrets stay
        // one-time-only (None → GUI shows only "allow once").
        // Linux EACCES/EROFS diagnostics improve display only: they do not prove
        // a write was blocked by policy and must not create new persistent-rule
        // suggestions. Keep the historical suggestion input unchanged.
        let suggestion_paths = extract_denial_paths(&request.failure_summary, false);
        let save_suggestion = escalation_save_suggestion(&suggestion_paths, sandbox);
        let action = serde_json::json!({
            "tool": "shell",
            "category": "sandbox_escalation",
            "escalation_trigger": request.trigger,
            "summary": command_summary(&request.command),
            "command": request.command,
            "justification": request.justification,
            "blocked_paths": blocked_paths,
            "diagnostic_paths": raw_blocked,
            "scope": {
                "cwd": sandbox.workspace.to_string_lossy(),
                "inside_workspace": true,
                "estimated_blast_radius": "high"
            }
        });
        let shape = ApprovalShape {
            kind: "sandbox_escalation",
            risk_level: "high",
            title: "Run command without the sandbox".to_string(),
            summary: if request.failure_summary.is_empty() {
                "Agent asks to run this command outside the sandbox.".to_string()
            } else {
                "Command appears blocked by the sandbox; agent asks to re-run it without the sandbox.".to_string()
            },
            action,
            // The approved re-run happens OUTSIDE the sandbox.
            sandbox_boundary: sandbox.boundary_json(Some("sandbox_escalation"), false),
            save_suggestion,
        };
        let requested_action = serde_json::json!({
            "command": request.command,
            "justification": request.justification,
            "failure_summary": request.failure_summary,
        });

        match self.resolve_approval(
            broadcaster,
            session_id,
            &crate::tools::current_tool_call_id(),
            "shell",
            &shape,
            requested_action,
        ) {
            ApprovalOutcome::Approved(_) => EscalationDecision::Approved,
            ApprovalOutcome::Rejected(note) => EscalationDecision::Denied(note),
            ApprovalOutcome::Cancelled(note)
                if self.review.as_ref().is_some_and(|context| {
                    context.invalidated().is_some_and(|verdict| {
                        verdict.status == crate::approval_review::ReviewStatus::StaleRequest
                    })
                }) =>
            {
                EscalationDecision::ContextInvalidated(note)
            }
            ApprovalOutcome::Cancelled(note) => EscalationDecision::Cancelled(if note.is_empty() {
                "approval request ended".to_string()
            } else {
                note
            }),
        }
    }

    /// Review automatically, or broadcast a human request and wait for a decision.
    fn resolve_approval(
        &self,
        broadcaster: &SseBroadcaster,
        session_id: &str,
        tool_id: &str,
        tool_name: &str,
        shape: &ApprovalShape,
        requested_action: serde_json::Value,
    ) -> ApprovalOutcome {
        if let Some(context) = &self.review {
            let cwd = shape
                .sandbox_boundary
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let action = crate::approval_review::prepare_review_action(
                tool_name,
                tool_id,
                &requested_action,
                &shape.action,
                &shape.sandbox_boundary,
                cwd,
            );
            let outcome = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(context.review_action(action, &requested_action))
            });
            let approved = outcome.verdict.approved();
            let status = outcome.verdict.status;
            let request_id = outcome.approval_request_id;
            let event = outcome.event;
            broadcaster.broadcast(SseEvent::new("approval_assessment", event));
            // Journal persistence is synchronous. No action may use an approval
            // whose audit failed, or whose context changed while recording it.
            if let Some(invalid) = context.invalidated() {
                context.block_execution(
                    tool_id,
                    invalid.status,
                    invalid.error_code.as_deref().unwrap_or("context_changed"),
                );
                return ApprovalOutcome::Cancelled("automatic review context ended".into());
            }
            if broadcaster.persistence_error().is_some() {
                context.block_execution(
                    tool_id,
                    crate::approval_review::ReviewStatus::ReviewError,
                    "audit_unavailable",
                );
                return ApprovalOutcome::Rejected("automatic review audit unavailable".into());
            }
            return if approved {
                ApprovalOutcome::Approved(request_id)
            } else if status == crate::approval_review::ReviewStatus::Cancelled {
                ApprovalOutcome::Cancelled("automatic review cancelled".into())
            } else {
                ApprovalOutcome::Rejected(format!(
                    "automatic review: {}; choose a safer action or ask the user for help",
                    status.as_str()
                ))
            };
        }
        let request_id = format!("approval_{}", crate::utils::generate_entry_id());
        let (tx, rx) = mpsc::channel::<ApprovalDecision>();
        let payload = serde_json::json!({
            "type": "approval_request",
            "approval_request_id": request_id,
            "session_id": session_id,
            "tool_id": tool_id,
            "tool_name": tool_name,
            "kind": shape.kind,
            "risk_level": shape.risk_level,
            "title": shape.title,
            "summary": shape.summary,
            "requested_action": requested_action,
            "action": shape.action,
            "sandbox_boundary": shape.sandbox_boundary,
            "save_suggestion": shape.save_suggestion,
            "reviewer": "user",
        });
        self.pending.lock().insert(
            request_id.clone(),
            PendingApproval {
                session_id: session_id.to_string(),
                payload: payload.clone(),
                tx,
            },
        );

        broadcaster.broadcast(SseEvent::new("approval_request", payload));

        let decision = tokio::task::block_in_place(|| rx.recv());
        let (status, note, outcome) = match decision {
            Ok(decision) if decision.approved => (
                "approved",
                decision.note.clone(),
                ApprovalOutcome::Approved(request_id.clone()),
            ),
            Ok(decision) if decision.status == ApprovalDecisionStatus::Cancelled => {
                let note = decision.note.clone();
                ("cancelled", note.clone(), ApprovalOutcome::Cancelled(note))
            }
            Ok(decision) => {
                let note = decision.note.clone();
                ("rejected", note.clone(), ApprovalOutcome::Rejected(note))
            }
            Err(_) => {
                self.pending.lock().remove(&request_id);
                let note = "Approval request was cancelled because the session ended.".to_string();
                ("cancelled", note.clone(), ApprovalOutcome::Cancelled(note))
            }
        };
        broadcaster.broadcast(SseEvent::new(
            "approval_decision",
            serde_json::json!({
                "type": "approval_decision",
                "approval_request_id": request_id,
                "tool_id": tool_id,
                "status": status,
                "note": note,
            }),
        ));
        outcome
    }

    pub fn decide(
        &self,
        request_id: &str,
        session_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        // Session ownership check (auth I1 exception): an approval belongs to the
        // session that raised it. A remote client must not be able to approve a
        // pending request of another session by guessing/leaking its entry_id.
        // In 1:1 this is naturally pair-scoped; the check also future-proofs 1:N.
        let belongs = {
            let guard = self.pending.lock();
            match guard.get(request_id) {
                None => {
                    return Err(format!("approval request `{request_id}` is not pending"));
                }
                Some(pending) => pending.session_id == session_id,
            }
        };
        if !belongs {
            return Err(format!(
                "approval request `{request_id}` does not belong to session `{session_id}`"
            ));
        }
        let pending = self
            .pending
            .lock()
            .remove(request_id)
            .ok_or_else(|| format!("approval request `{request_id}` is not pending"))?;
        pending.tx.send(decision).map_err(|error| error.to_string())
    }

    /// Payloads of the approval requests still awaiting a decision for one
    /// session. Served via get_state so a (re)connecting client can rebuild
    /// approval cards it missed — the pending map is process-wide, so filter
    /// by session here (same ownership rule as `decide`).
    pub fn pending_for_session(&self, session_id: &str) -> Vec<serde_json::Value> {
        self.pending
            .lock()
            .values()
            .filter(|pending| pending.session_id == session_id)
            .map(|pending| pending.payload.clone())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn insert_pending_for_test(
        &self,
        request_id: &str,
        session_id: &str,
    ) -> mpsc::Receiver<ApprovalDecision> {
        let (tx, rx) = mpsc::channel();
        self.pending.lock().insert(
            request_id.to_string(),
            PendingApproval {
                session_id: session_id.to_string(),
                payload: serde_json::json!({
                    "type": "approval_request",
                    "approval_request_id": request_id,
                    "session_id": session_id,
                    "tool_name": "shell",
                }),
                tx,
            },
        );
        rx
    }

    pub fn cancel_session(&self, session_id: &str, note: &str) -> usize {
        let pending = {
            let mut guard = self.pending.lock();
            let request_ids = guard
                .iter()
                .filter(|&(_request_id, pending)| pending.session_id == session_id)
                .map(|(request_id, _pending)| request_id.clone())
                .collect::<Vec<_>>();
            request_ids
                .into_iter()
                .filter_map(|request_id| {
                    guard
                        .remove(&request_id)
                        .map(|pending| (request_id, pending))
                })
                .collect::<Vec<_>>()
        };
        let count = pending.len();

        for (_, pending) in pending {
            let _ = pending.tx.send(ApprovalDecision {
                approved: false,
                note: note.to_string(),
                status: ApprovalDecisionStatus::Cancelled,
            });
        }

        count
    }
}

pub(super) struct ApprovalShape {
    pub kind: &'static str,
    pub risk_level: &'static str,
    pub title: String,
    pub summary: String,
    pub action: serde_json::Value,
    pub sandbox_boundary: serde_json::Value,
    /// Suggested rule to persist if the user picks "session"/"always" allow:
    /// `{ "match_kind": ..., "match_value": ..., "decision": "approve" }`.
    /// `None` for kinds that shouldn't be persisted as a rule (e.g. escalation).
    pub save_suggestion: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests;
