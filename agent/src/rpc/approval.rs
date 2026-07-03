use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};

use super::approval_policy::{evaluate_policy, PolicyDecision};
use super::{SseBroadcaster, SseEvent};
use crate::sandbox::{
    ApprovalPolicy, EscalationDecision, EscalationRequest, ResolvedSandbox, SandboxMode,
};

#[derive(Clone, Default)]
pub struct ApprovalGate {
    pending: Arc<Mutex<HashMap<String, PendingApproval>>>,
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
    tx: mpsc::Sender<ApprovalDecision>,
}

/// Outcome of a blocking user decision (shared by tool approvals and
/// sandbox escalations).
enum AskOutcome {
    Approved,
    Rejected(String),
    Cancelled(String),
}

impl ApprovalGate {
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
        // Step 1 (§2.1): explicit rules run FIRST — before the sandbox
        // auto-allow — so a deny rule can stop even a command the sandbox would
        // otherwise contain, and an approve rule can suppress the prompt for a
        // command that would otherwise ask (e.g. under `untrusted`). An approve
        // only skips the prompt; the command still runs sandboxed downstream.
        match evaluate_policy(&sandbox.rules, tool_name, arguments) {
            PolicyDecision::AskUser => {}
            PolicyDecision::AutoApprove => {
                if let Some(path) = approved_argument_path(cwd, arguments) {
                    crate::tools::approve_outside_path(&path);
                }
                return None;
            }
            PolicyDecision::AutoReject(reason) => {
                return Some(crate::types::ToolCallResult {
                    result: format!("Tool call `{tool_name}` was rejected by a rule ({reason})."),
                    is_error: true,
                });
            }
        }

        // Step 2: sandbox auto-allow. None means the call stays inside the
        // boundary (or is an allowlisted read) and needs no approval.
        let shape = approval_shape(cwd, tool_name, arguments, sandbox)?;

        // "never" approval policy: no prompts — boundary-crossing operations
        // fail immediately with a clear error for the model.
        if sandbox.approval_policy == ApprovalPolicy::Never {
            return Some(crate::types::ToolCallResult {
                result: format!(
                    "Tool call `{tool_name}` requires approval, but the approval policy is 'never'. Work inside the sandbox boundary instead."
                ),
                is_error: true,
            });
        }

        let outcome = self.ask_user(
            broadcaster,
            session_id,
            tool_id,
            tool_name,
            &shape,
            normalize_requested_action(arguments),
        );
        match outcome {
            AskOutcome::Approved => {
                if let Some(path) = approved_argument_path(cwd, arguments) {
                    crate::tools::approve_outside_path(&path);
                }
                None
            }
            AskOutcome::Cancelled(_) => Some(crate::types::ToolCallResult {
                result: format!(
                    "Tool call `{tool_name}` was cancelled because the approval request ended."
                ),
                is_error: true,
            }),
            AskOutcome::Rejected(note) => Some(crate::types::ToolCallResult {
                result: format!(
                    "Tool call `{}` was rejected by the user{}.",
                    tool_name,
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

    /// Post-hoc approval for running a bash command outside the sandbox
    /// (SANDBOX_PLAN.md §2.6). Approval means the single re-run happens
    /// unsandboxed — "this exact command, once".
    pub fn request_escalation(
        &self,
        broadcaster: &SseBroadcaster,
        session_id: &str,
        request: &EscalationRequest,
        sandbox: &ResolvedSandbox,
    ) -> EscalationDecision {
        if sandbox.approval_policy == ApprovalPolicy::Never {
            return EscalationDecision::Denied("approval policy is 'never'".to_string());
        }

        let action = serde_json::json!({
            "tool": "bash",
            "category": "sandbox_escalation",
            "summary": command_summary(&request.command),
            "command": request.command,
            "justification": request.justification,
            "failure_summary": request.failure_summary,
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
            // Escalation is a one-time out-of-sandbox run — never persist it.
            save_suggestion: None,
        };
        let requested_action = serde_json::json!({
            "command": request.command,
            "justification": request.justification,
            "failure_summary": request.failure_summary,
        });

        match self.ask_user(
            broadcaster,
            session_id,
            "",
            "bash",
            &shape,
            requested_action,
        ) {
            AskOutcome::Approved => EscalationDecision::Approved,
            AskOutcome::Rejected(note) => EscalationDecision::Denied(note),
            AskOutcome::Cancelled(note) => EscalationDecision::Denied(if note.is_empty() {
                "approval request ended".to_string()
            } else {
                note
            }),
        }
    }

    /// Broadcast an approval request and block until a decision arrives.
    fn ask_user(
        &self,
        broadcaster: &SseBroadcaster,
        session_id: &str,
        tool_id: &str,
        tool_name: &str,
        shape: &ApprovalShape,
        requested_action: serde_json::Value,
    ) -> AskOutcome {
        let request_id = format!("approval_{}", crate::utils::generate_entry_id());
        let (tx, rx) = mpsc::channel::<ApprovalDecision>();
        self.pending.lock().unwrap().insert(
            request_id.clone(),
            PendingApproval {
                session_id: session_id.to_string(),
                tx,
            },
        );

        broadcaster.broadcast(SseEvent::new(
            "approval_request",
            serde_json::json!({
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
            }),
        ));

        let decision = tokio::task::block_in_place(|| rx.recv());
        let (status, note, outcome) = match decision {
            Ok(decision) if decision.approved => {
                ("approved", decision.note.clone(), AskOutcome::Approved)
            }
            Ok(decision) if decision.status == ApprovalDecisionStatus::Cancelled => {
                let note = decision.note.clone();
                ("cancelled", note.clone(), AskOutcome::Cancelled(note))
            }
            Ok(decision) => {
                let note = decision.note.clone();
                ("rejected", note.clone(), AskOutcome::Rejected(note))
            }
            Err(_) => {
                self.pending.lock().unwrap().remove(&request_id);
                let note = "Approval request was cancelled because the session ended.".to_string();
                ("cancelled", note.clone(), AskOutcome::Cancelled(note))
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

    pub fn decide(&self, request_id: &str, decision: ApprovalDecision) -> Result<(), String> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(request_id)
            .ok_or_else(|| format!("approval request `{request_id}` is not pending"))?;
        pending.tx.send(decision).map_err(|error| error.to_string())
    }

    pub fn cancel_session(&self, session_id: &str, note: &str) -> usize {
        let pending = {
            let mut guard = self.pending.lock().unwrap();
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

/// Decide whether a tool call needs pre-execution approval, and shape the
/// approval request (SANDBOX_PLAN.md §2.1 decision flow).
///
/// - `danger-full-access`: nothing needs approval (user opted out).
/// - bash, sandbox available: runs inside the OS sandbox → auto-allowed
///   (except under the `untrusted` policy, where non-allowlisted commands
///   still ask). Boundary crossings surface later via escalation.
/// - bash, degraded (no OS sandbox): legacy behavior — allowlisted read-only
///   commands auto-run, everything else asks.
/// - write/edit: ask when the target is outside the writable roots, or on
///   any write in `read-only` mode.
fn approval_shape(
    cwd: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    sandbox: &ResolvedSandbox,
) -> Option<ApprovalShape> {
    if sandbox.mode == SandboxMode::DangerFullAccess {
        return None;
    }
    match tool_name {
        "bash" => {
            let sandboxed = sandbox.wraps_bash();
            let needs_approval = if sandboxed {
                sandbox.approval_policy == ApprovalPolicy::Untrusted
                    && bash_requires_approval(cwd, arguments)
            } else {
                bash_requires_approval(cwd, arguments)
            };
            if !needs_approval {
                return None;
            }
            let command = argument_command(arguments).unwrap_or_default();
            let action = serde_json::json!({
                "tool": "bash",
                "category": "shell_command",
                "summary": command_summary(&command),
                "command": command,
                "scope": {
                    "cwd": cwd,
                    "inside_workspace": true,
                    "estimated_blast_radius": "high"
                }
            });
            let violation = if sandboxed {
                "untrusted_policy"
            } else {
                "shell_command_not_in_allowlist"
            };
            Some(ApprovalShape {
                kind: "shell_command",
                risk_level: "high",
                title: "Approve shell command".to_string(),
                summary: "Agent wants to run a shell command.".to_string(),
                action,
                sandbox_boundary: sandbox.boundary_json(Some(violation), sandboxed),
                save_suggestion: command_save_suggestion(&command),
            })
        }
        "write" | "edit" => {
            let path = argument_path(arguments)?;
            let read_only = sandbox.mode == SandboxMode::ReadOnly;
            let outside_roots = !sandbox.path_is_writable(&path);
            if !read_only && !outside_roots {
                return None;
            }
            let preview = argument_write_preview(arguments);
            let category = if tool_name == "write" {
                "file_write"
            } else {
                "file_edit"
            };
            let action = serde_json::json!({
                "tool": tool_name,
                "category": category,
                "summary": format!("Modify {}", path),
                "paths": [path.clone()],
                "writes": [{
                    "path": path,
                    "preview": preview,
                }],
                "scope": {
                    "cwd": cwd,
                    "inside_workspace": !outside_roots,
                    "estimated_blast_radius": "medium"
                }
            });
            let (kind, violation, title, summary) = if outside_roots {
                (
                    "outside_workspace_write",
                    "outside_workspace_write",
                    "Approve outside-workspace write",
                    "Agent wants to modify a file outside the writable roots.",
                )
            } else {
                (
                    "file_write",
                    "read_only_write",
                    "Approve file write (read-only mode)",
                    "The sandbox is read-only; agent wants to modify a file.",
                )
            };
            Some(ApprovalShape {
                kind,
                risk_level: "medium",
                title: title.to_string(),
                summary: summary.to_string(),
                action,
                sandbox_boundary: sandbox.boundary_json(Some(violation), false),
                save_suggestion: path_save_suggestion(&path),
            })
        }
        _ => None,
    }
}

/// Suggested `command_prefix` rule for a bash command: program name plus its
/// first non-flag subcommand, then `*` (e.g. `git push origin` → `git push *`,
/// `cargo build --release` → `cargo build *`, `rm -rf x` → `rm *`).
fn command_save_suggestion(command: &str) -> Option<serde_json::Value> {
    let mut tokens = command.split_whitespace();
    let program = tokens.next()?;
    // Only keep a second token if it looks like a subcommand, not a flag/path.
    let pattern = match tokens.next() {
        Some(sub) if !sub.starts_with('-') && !sub.contains('/') => {
            format!("{program} {sub} *")
        }
        _ => format!("{program} *"),
    };
    Some(serde_json::json!({
        "match_kind": "command_prefix",
        "match_value": pattern,
        "decision": "approve",
    }))
}

/// Suggested `path_glob` rule for a write/edit target: everything in its parent
/// directory (`/a/b/c.txt` → `/a/b/*`).
fn path_save_suggestion(path: &str) -> Option<serde_json::Value> {
    let parent = std::path::Path::new(path).parent()?;
    let glob = format!("{}/*", parent.to_string_lossy());
    Some(serde_json::json!({
        "match_kind": "path_glob",
        "match_value": glob,
        "decision": "approve",
    }))
}

fn command_summary(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.len() <= 200 {
        trimmed.to_string()
    } else {
        let mut head: String = trimmed.chars().take(200).collect();
        head.push('\u{2026}');
        head
    }
}

fn argument_write_preview(arguments: &serde_json::Value) -> Option<String> {
    let normalized = match arguments {
        serde_json::Value::String(raw) => serde_json::from_str(raw)
            .ok()
            .or_else(|| repair_partial_json_object(raw)),
        _ => Some(arguments.clone()),
    }?;
    for key in ["content", "newText", "text"] {
        if let Some(value) = normalized.get(key).and_then(|v| v.as_str()) {
            let mut head: String = value.chars().take(200).collect();
            if value.chars().count() > 200 {
                head.push('\u{2026}');
            }
            return Some(head);
        }
    }
    None
}

fn bash_requires_approval(cwd: &str, arguments: &serde_json::Value) -> bool {
    let Some(command) = argument_command(arguments) else {
        return true;
    };
    !is_workspace_read_command(cwd, &command)
}

fn normalize_requested_action(arguments: &serde_json::Value) -> serde_json::Value {
    match arguments {
        serde_json::Value::String(raw) => serde_json::from_str(raw)
            .ok()
            .or_else(|| repair_partial_json_object(raw))
            .unwrap_or_else(|| arguments.clone()),
        _ => arguments.clone(),
    }
}

pub(super) fn argument_command(arguments: &serde_json::Value) -> Option<String> {
    let normalized = match arguments {
        serde_json::Value::String(raw) => serde_json::from_str(raw)
            .ok()
            .or_else(|| repair_partial_json_object(raw)),
        _ => Some(arguments.clone()),
    }?;
    normalized
        .get("command")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn is_workspace_read_command(cwd: &str, command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }

    let sanitized = command
        .replace("2>/dev/null", "")
        .replace("2> /dev/null", "");
    if sanitized.contains(';')
        || sanitized.contains("&&")
        || sanitized.contains("||")
        || sanitized.contains('`')
        || sanitized.contains("$(")
        || sanitized.contains('>')
        || sanitized.contains('<')
    {
        return false;
    }

    let workspace = normalize_path(Path::new(cwd));
    for token in sanitized.split_whitespace() {
        if token.starts_with('/') && token != "/dev/null" {
            let path = normalize_path(Path::new(token.trim_matches('"').trim_matches('\'')));
            if !path.starts_with(&workspace) {
                return false;
            }
        }
    }

    sanitized.split('|').all(|segment| {
        let program = segment.split_whitespace().next().unwrap_or_default().trim();
        matches!(
            program,
            "cat"
                | "echo"
                | "find"
                | "future"
                | "grep"
                | "head"
                | "ls"
                | "pwd"
                | "rg"
                | "sed"
                | "tail"
                | "wc"
        )
    })
}

fn repair_partial_json_object(raw: &str) -> Option<serde_json::Value> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') {
        return None;
    }

    let mut repaired = trimmed.to_string();
    if has_unclosed_string(&repaired) {
        repaired.push('"');
    }

    let open_braces = repaired.chars().filter(|c| *c == '{').count();
    let close_braces = repaired.chars().filter(|c| *c == '}').count();
    if open_braces > close_braces {
        for _ in 0..(open_braces - close_braces) {
            repaired.push('}');
        }
    }

    serde_json::from_str(&repaired).ok()
}

fn has_unclosed_string(value: &str) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            _ => {}
        }
    }
    in_string
}

pub(super) fn argument_path(arguments: &serde_json::Value) -> Option<String> {
    let normalized = match arguments {
        serde_json::Value::String(raw) => serde_json::from_str(raw).unwrap_or(arguments.clone()),
        _ => arguments.clone(),
    };
    ["path", "file_path", "filePath"]
        .iter()
        .find_map(|key| normalized.get(*key).and_then(|value| value.as_str()))
        .map(str::to_string)
}

fn approved_argument_path(cwd: &str, arguments: &serde_json::Value) -> Option<String> {
    let path = argument_path(arguments)?;
    // §3.5: `~` resolves to the real home directory, not the workspace.
    let candidate = crate::sandbox::paths::resolve_against(Path::new(cwd), &path);
    Some(candidate.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("futureos-approval-{name}-{stamp}"))
    }

    /// A path outside every writable root (workspace, tmp). Never created.
    fn outside_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dirs::home_dir()
            .unwrap()
            .join(format!("futureos-approval-outside-{name}-{stamp}.txt"))
    }

    /// Degraded sandbox (no OS wrapping): legacy allowlist + approval behavior.
    pub(super) fn degraded_sandbox(workspace: &Path) -> ResolvedSandbox {
        let mut sandbox = ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy::default(),
            workspace.to_string_lossy().as_ref(),
        );
        sandbox.available = false;
        sandbox
    }

    #[test]
    fn write_inside_workspace_does_not_require_approval() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let args = serde_json::json!({
            "path": workspace.join("poem.txt").to_string_lossy(),
            "content": "hello"
        });

        assert!(approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox
        )
        .is_none());
    }

    #[test]
    fn write_outside_roots_requires_approval() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let outside = outside_path("write");
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": "hello"
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox,
        )
        .expect("outside-roots write should require approval");
        assert_eq!(shape.kind, "outside_workspace_write");
        assert_eq!(shape.risk_level, "medium");
    }

    #[test]
    fn write_to_temp_dir_is_auto_allowed() {
        // Temp dirs are writable roots (§2.2) — no approval.
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let args = serde_json::json!({
            "path": temp_path("free-write.txt").to_string_lossy(),
            "content": "hello"
        });

        assert!(approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox
        )
        .is_none());
    }

    #[test]
    fn edit_outside_roots_requires_approval_from_json_string_args() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let outside = outside_path("edit");
        let args = serde_json::json!(serde_json::json!({
            "path": outside.to_string_lossy(),
            "oldText": "a",
            "newText": "b"
        })
        .to_string());

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "edit",
            &args,
            &sandbox,
        )
        .expect("outside-roots edit should require approval");
        assert_eq!(shape.kind, "outside_workspace_write");
    }

    #[test]
    fn tilde_write_resolves_to_home_and_requires_approval() {
        // Legacy bug: `~/x` was joined onto the workspace, dodging approval.
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let args = serde_json::json!({
            "path": "~/futureos-tilde-test.txt",
            "content": "hello"
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox,
        )
        .expect("~/ write should be treated as a home write, outside the roots");
        assert_eq!(shape.kind, "outside_workspace_write");
    }

    #[test]
    fn read_only_mode_requires_approval_for_inside_writes() {
        let workspace = temp_path("workspace");
        let mut sandbox = degraded_sandbox(&workspace);
        sandbox.mode = SandboxMode::ReadOnly;
        let args = serde_json::json!({
            "path": workspace.join("poem.txt").to_string_lossy(),
            "content": "hello"
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox,
        )
        .expect("read-only mode should require approval for writes");
        assert_eq!(shape.kind, "file_write");
        assert_eq!(shape.sandbox_boundary["violation"], "read_only_write");
    }

    #[test]
    fn sandboxed_bash_is_auto_allowed_under_on_request() {
        let workspace = temp_path("workspace");
        let mut sandbox = degraded_sandbox(&workspace);
        sandbox.available = true; // OS sandbox present → bash runs wrapped
        let args = serde_json::json!({ "command": "rm -rf node_modules" });

        assert!(
            approval_shape(
                workspace.to_string_lossy().as_ref(),
                "bash",
                &args,
                &sandbox
            )
            .is_none(),
            "bash inside the sandbox should not require pre-approval"
        );
    }

    #[test]
    fn sandboxed_bash_still_asks_under_untrusted_policy() {
        let workspace = temp_path("workspace");
        let mut sandbox = degraded_sandbox(&workspace);
        sandbox.available = true;
        sandbox.approval_policy = ApprovalPolicy::Untrusted;
        let args = serde_json::json!({ "command": "rm -rf node_modules" });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "bash",
            &args,
            &sandbox,
        )
        .expect("untrusted policy should still ask");
        assert_eq!(shape.sandbox_boundary["violation"], "untrusted_policy");
        assert_eq!(shape.sandbox_boundary["inside_sandbox"], true);

        // Allowlisted read-only commands stay auto-approved even under untrusted.
        let ls_args = serde_json::json!({ "command": "ls" });
        assert!(approval_shape(
            workspace.to_string_lossy().as_ref(),
            "bash",
            &ls_args,
            &sandbox
        )
        .is_none());
    }

    #[test]
    fn full_access_mode_never_requires_approval() {
        let workspace = temp_path("workspace");
        let mut sandbox = degraded_sandbox(&workspace);
        sandbox.mode = SandboxMode::DangerFullAccess;
        let bash_args = serde_json::json!({ "command": "rm -rf /" });
        let write_args = serde_json::json!({
            "path": outside_path("full").to_string_lossy(),
            "content": "x"
        });

        assert!(approval_shape(
            workspace.to_string_lossy().as_ref(),
            "bash",
            &bash_args,
            &sandbox
        )
        .is_none());
        assert!(approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &write_args,
            &sandbox
        )
        .is_none());
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod structured_tests {
    use super::tests::degraded_sandbox;
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("futureos-approval-{name}-{stamp}"))
    }

    fn outside_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dirs::home_dir()
            .unwrap()
            .join(format!("futureos-approval-outside-{name}-{stamp}.txt"))
    }

    #[test]
    fn shell_command_action_is_structured() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let args = serde_json::json!({
            "command": "rm -rf node_modules"
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "bash",
            &args,
            &sandbox,
        )
        .expect("rm should require approval");

        assert_eq!(shape.action["tool"], "bash");
        assert_eq!(shape.action["category"], "shell_command");
        assert_eq!(shape.action["command"], "rm -rf node_modules");
        assert_eq!(shape.action["scope"]["inside_workspace"], true);
        assert_eq!(shape.action["scope"]["estimated_blast_radius"], "high");
    }

    #[test]
    fn shell_command_sandbox_boundary_reflects_real_state() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let args = serde_json::json!({
            "command": "rm -rf node_modules"
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "bash",
            &args,
            &sandbox,
        )
        .expect("rm should require approval");

        assert_eq!(shape.sandbox_boundary["mode"], "workspace-write");
        assert_eq!(shape.sandbox_boundary["inside_sandbox"], false);
        assert_eq!(shape.sandbox_boundary["sandbox_available"], false);
        assert_eq!(
            shape.sandbox_boundary["violation"],
            "shell_command_not_in_allowlist"
        );
        // Boundary reports canonicalized workspace + tmp roots.
        let roots = shape.sandbox_boundary["writable_roots"].as_array().unwrap();
        assert!(roots.len() >= 2);
        assert_eq!(
            shape.sandbox_boundary["cwd"],
            sandbox.workspace.to_string_lossy().to_string()
        );
    }

    #[test]
    fn file_write_action_is_structured() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let outside = outside_path("structured");
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": "hello world"
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox,
        )
        .expect("outside-roots write should require approval");

        assert_eq!(shape.action["tool"], "write");
        assert_eq!(shape.action["category"], "file_write");
        assert_eq!(
            shape.action["paths"][0],
            outside.to_string_lossy().to_string()
        );
        assert_eq!(
            shape.action["writes"][0]["path"],
            outside.to_string_lossy().to_string()
        );
        assert_eq!(shape.action["writes"][0]["preview"], "hello world");
        assert_eq!(shape.action["scope"]["inside_workspace"], false);
        assert_eq!(shape.action["scope"]["estimated_blast_radius"], "medium");
    }

    #[test]
    fn file_write_sandbox_boundary_is_structured() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let outside = outside_path("boundary");
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": "hello"
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox,
        )
        .expect("outside-roots write should require approval");

        assert_eq!(shape.sandbox_boundary["mode"], "workspace-write");
        assert_eq!(shape.sandbox_boundary["inside_sandbox"], false);
        assert_eq!(
            shape.sandbox_boundary["violation"],
            "outside_workspace_write"
        );
    }

    #[test]
    fn write_preview_truncates_long_content() {
        let workspace = temp_path("workspace");
        let sandbox = degraded_sandbox(&workspace);
        let outside = outside_path("preview");
        let long_content = "x".repeat(300);
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": long_content
        });

        let shape = approval_shape(
            workspace.to_string_lossy().as_ref(),
            "write",
            &args,
            &sandbox,
        )
        .expect("outside-roots write should require approval");

        let preview = shape.action["writes"][0]["preview"].as_str().unwrap();
        assert_eq!(preview.chars().count(), 201); // 200 chars + ellipsis
        assert!(preview.ends_with("…"));
    }

    #[test]
    fn policy_evaluator_asks_when_no_rules() {
        let args = serde_json::json!({ "command": "rm -rf node_modules" });
        let decision = evaluate_policy(&[], "bash", &args);
        assert!(matches!(decision, PolicyDecision::AskUser));
    }

    #[test]
    fn deny_rule_blocks_sandboxed_bash_before_auto_allow() {
        // A deny rule must stop a command the sandbox would otherwise auto-run.
        let workspace = temp_path("workspace");
        let mut sandbox = degraded_sandbox(&workspace);
        sandbox.available = true; // bash would be auto-allowed (on-request)
        sandbox.rules = vec![crate::sandbox::SandboxRule {
            match_kind: "command_prefix".to_string(),
            match_value: "rm *".to_string(),
            decision: "reject".to_string(),
        }];

        let gate = ApprovalGate::default();
        let broadcaster = SseBroadcaster::new();
        let result = gate.request(
            &broadcaster,
            "session_test",
            workspace.to_string_lossy().as_ref(),
            "bash",
            "tool_1",
            &serde_json::json!({ "command": "rm -rf /" }),
            &sandbox,
        );
        let result = result.expect("deny rule should reject");
        assert!(result.is_error);
        assert!(result.result.contains("deny rule"));
    }

    #[test]
    fn approve_rule_skips_prompt_under_untrusted() {
        // Under `untrusted`, a non-allowlisted command normally asks; an
        // approve rule suppresses the prompt (returns None → proceeds).
        let workspace = temp_path("workspace");
        let mut sandbox = degraded_sandbox(&workspace);
        sandbox.available = true;
        sandbox.approval_policy = ApprovalPolicy::Untrusted;
        sandbox.rules = vec![crate::sandbox::SandboxRule {
            match_kind: "command_prefix".to_string(),
            match_value: "cargo *".to_string(),
            decision: "approve".to_string(),
        }];

        let gate = ApprovalGate::default();
        let broadcaster = SseBroadcaster::new();
        let result = gate.request(
            &broadcaster,
            "session_test",
            workspace.to_string_lossy().as_ref(),
            "bash",
            "tool_1",
            &serde_json::json!({ "command": "cargo build" }),
            &sandbox,
        );
        assert!(result.is_none(), "approve rule should skip the prompt");
    }

    #[test]
    fn command_save_suggestion_uses_program_and_subcommand() {
        let s = |cmd: &str| {
            command_save_suggestion(cmd).map(|v| v["match_value"].as_str().unwrap().to_string())
        };
        assert_eq!(s("git push origin main").as_deref(), Some("git push *"));
        assert_eq!(s("cargo build --release").as_deref(), Some("cargo build *"));
        assert_eq!(s("rm -rf x").as_deref(), Some("rm *")); // flag → program only
        assert_eq!(s("ls").as_deref(), Some("ls *"));
        // A path-like second token collapses to program only.
        assert_eq!(s("bash ./run.sh").as_deref(), Some("bash *"));
        let suggestion = command_save_suggestion("git push").unwrap();
        assert_eq!(suggestion["match_kind"], "command_prefix");
        assert_eq!(suggestion["decision"], "approve");
    }

    #[test]
    fn path_save_suggestion_globs_parent_dir() {
        let suggestion = path_save_suggestion("/Users/x/proj/src/main.rs").unwrap();
        assert_eq!(suggestion["match_kind"], "path_glob");
        assert_eq!(suggestion["match_value"], "/Users/x/proj/src/*");
        assert_eq!(suggestion["decision"], "approve");
    }
}
