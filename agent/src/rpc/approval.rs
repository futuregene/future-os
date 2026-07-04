use std::{
    collections::HashMap,
    path::Path,
    sync::{mpsc, Arc, Mutex},
};

use super::{SseBroadcaster, SseEvent};
use crate::sandbox::rules::{Decision, Op};
use crate::sandbox::{paths, EscalationDecision, EscalationRequest, ResolvedSandbox};

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
        // v2: everything is a file-path decision. Disabled sessions run fully
        // open. bash is not gated here (it runs Seatbelt-wrapped; boundary
        // hits surface via escalation) — only file-touching tools are.
        if !sandbox.enabled {
            return None;
        }
        let op = match tool_name {
            "read" => Op::Read,
            "write" | "edit" => Op::Write,
            _ => return None,
        };
        let raw_path = argument_path(arguments)?;
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
                        if op == Op::Write {
                            crate::tools::approve_outside_path(&path.to_string_lossy());
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

/// Shape the approval card for a file access that resolved to `Ask`. `path` is
/// the resolved absolute target; `op` its read/write nature.
fn approval_shape(
    tool_name: &str,
    path: &Path,
    op: Op,
    arguments: &serde_json::Value,
    sandbox: &ResolvedSandbox,
) -> ApprovalShape {
    let path_str = path.to_string_lossy().to_string();
    let inside = paths::path_within(path, &sandbox.workspace);
    let (kind, category, title, summary, verb) = match op {
        Op::Read => (
            "file_read",
            "file_read",
            "Approve file read",
            "Agent wants to read a protected file.",
            "Read",
        ),
        Op::Write if inside => (
            "file_write",
            if tool_name == "edit" {
                "file_edit"
            } else {
                "file_write"
            },
            "Approve file write",
            "Agent wants to modify a protected file.",
            "Modify",
        ),
        Op::Write => (
            "outside_workspace_write",
            if tool_name == "edit" {
                "file_edit"
            } else {
                "file_write"
            },
            "Approve outside-workspace write",
            "Agent wants to modify a file outside the workspace.",
            "Modify",
        ),
    };
    let writes = if op == Op::Write {
        serde_json::json!([{ "path": path_str, "preview": argument_write_preview(arguments) }])
    } else {
        serde_json::json!([])
    };
    let action = serde_json::json!({
        "tool": tool_name,
        "category": category,
        "summary": format!("{verb} {path_str}"),
        "paths": [path_str.clone()],
        "writes": writes,
        "scope": {
            "cwd": sandbox.workspace.to_string_lossy(),
            "inside_workspace": inside,
            "estimated_blast_radius": "medium"
        }
    });
    let violation = if op == Op::Read {
        "protected_read"
    } else if inside {
        "protected_write"
    } else {
        "outside_workspace_write"
    };
    // Secret files are "allow once" only — never persistently allowed
    // (Plan A). Suppress the save suggestion so the GUI hides the "allow in
    // this workspace" button; only deny / allow-once remain.
    let save_suggestion = if sandbox.is_secret_path(path) {
        None
    } else {
        path_save_suggestion(path, op, &sandbox.workspace)
    };
    ApprovalShape {
        kind,
        risk_level: "medium",
        title: title.to_string(),
        summary: summary.to_string(),
        action,
        sandbox_boundary: sandbox.boundary_json(Some(violation), false),
        save_suggestion,
    }
}

/// Suggested rule (v2 file format) for "allow in this workspace": everything in
/// the target's parent directory, scoped to the same read/write op. Paths
/// inside the workspace are made **relative** (portable, git-friendly); outside
/// paths stay absolute.
fn path_save_suggestion(path: &Path, op: Op, workspace: &Path) -> Option<serde_json::Value> {
    let parent = path.parent()?;
    let glob = match parent.strip_prefix(workspace) {
        Ok(rel) if rel.as_os_str().is_empty() => "*".to_string(),
        Ok(rel) => format!("{}/*", rel.to_string_lossy()),
        Err(_) => format!("{}/*", parent.to_string_lossy()),
    };
    Some(serde_json::json!({
        "path": glob,
        "access": match op { Op::Read => "read", Op::Write => "write" },
        "action": "allow",
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

fn normalize_requested_action(arguments: &serde_json::Value) -> serde_json::Value {
    match arguments {
        serde_json::Value::String(raw) => serde_json::from_str(raw)
            .ok()
            .or_else(|| repair_partial_json_object(raw))
            .unwrap_or_else(|| arguments.clone()),
        _ => arguments.clone(),
    }
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

fn argument_path(arguments: &serde_json::Value) -> Option<String> {
    let normalized = match arguments {
        serde_json::Value::String(raw) => serde_json::from_str(raw).unwrap_or(arguments.clone()),
        _ => arguments.clone(),
    };
    ["path", "file_path", "filePath"]
        .iter()
        .find_map(|key| normalized.get(*key).and_then(|value| value.as_str()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxPolicy;

    fn temp_ws(name: &str) -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("futureos-approval-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn enabled(ws: &str) -> ResolvedSandbox {
        ResolvedSandbox::resolve(&SandboxPolicy { enabled: true }, ws)
    }

    /// A path outside the workspace and temp (never created).
    fn outside(name: &str) -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dirs::home_dir()
            .unwrap()
            .join(format!("futureos-approval-outside-{name}-{stamp}.txt"))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn disabled_session_never_prompts() {
        let ws = temp_ws("disabled");
        let sandbox = ResolvedSandbox::disabled(&ws);
        let gate = ApprovalGate::default();
        let b = SseBroadcaster::new();
        let args = serde_json::json!({ "path": outside("d"), "content": "x" });
        assert!(gate
            .request(&b, "s", &ws, "write", "t", &args, &sandbox)
            .is_none());
    }

    #[test]
    fn bash_is_never_pre_approved() {
        // bash runs sandboxed; no pre-execution approval in v2.
        let ws = temp_ws("bash");
        let sandbox = enabled(&ws);
        let gate = ApprovalGate::default();
        let b = SseBroadcaster::new();
        let args = serde_json::json!({ "command": "rm -rf /" });
        assert!(gate
            .request(&b, "s", &ws, "bash", "t", &args, &sandbox)
            .is_none());
    }

    #[test]
    fn write_inside_workspace_auto_allowed() {
        let ws = temp_ws("inside");
        let sandbox = enabled(&ws);
        let gate = ApprovalGate::default();
        let b = SseBroadcaster::new();
        let args = serde_json::json!({ "path": format!("{ws}/src/main.rs"), "content": "x" });
        assert!(gate
            .request(&b, "s", &ws, "write", "t", &args, &sandbox)
            .is_none());
    }

    #[test]
    fn rule_file_write_is_denied_without_prompt() {
        let ws = temp_ws("rulefile");
        let sandbox = enabled(&ws);
        let gate = ApprovalGate::default();
        let b = SseBroadcaster::new();
        let args = serde_json::json!({
            "path": format!("{ws}/.future/approval_rule.json"),
            "content": "{}"
        });
        let result = gate
            .request(&b, "s", &ws, "write", "t", &args, &sandbox)
            .expect("rule-file write must be denied");
        assert!(result.is_error);
        assert!(result.result.contains("denied by an approval rule"));
    }

    #[test]
    fn read_of_ordinary_file_auto_allowed() {
        let ws = temp_ws("read-ok");
        let sandbox = enabled(&ws);
        let gate = ApprovalGate::default();
        let b = SseBroadcaster::new();
        let args = serde_json::json!({ "path": format!("{ws}/src/lib.rs") });
        assert!(gate
            .request(&b, "s", &ws, "read", "t", &args, &sandbox)
            .is_none());
    }

    #[test]
    fn shape_for_write_is_structured() {
        let ws = temp_ws("shape");
        let sandbox = enabled(&ws);
        // Use the canonicalized workspace so the suggestion is workspace-relative.
        let path = sandbox.workspace.join("sub/out.txt");
        let args = serde_json::json!({ "path": path.to_string_lossy(), "content": "hello" });
        let shape = approval_shape("write", &path, Op::Write, &args, &sandbox);
        assert_eq!(shape.action["tool"], "write");
        assert_eq!(shape.action["category"], "file_write");
        assert_eq!(shape.action["writes"][0]["preview"], "hello");
        let sug = shape.save_suggestion.unwrap();
        assert_eq!(sug["access"], "write");
        assert_eq!(sug["action"], "allow");
        // Inside the workspace → relative glob.
        assert_eq!(sug["path"], "sub/*");
    }

    #[test]
    fn shape_for_secret_read_suppresses_suggestion() {
        // A secret file (~/.ssh) has no "allow in this workspace" — allow-once only.
        let ws = temp_ws("shape-secret");
        let sandbox = enabled(&ws);
        let path = dirs::home_dir().unwrap().join(".ssh/id_rsa");
        let args = serde_json::json!({ "path": path.to_string_lossy() });
        let shape = approval_shape("read", &path, Op::Read, &args, &sandbox);
        assert_eq!(shape.kind, "file_read");
        assert!(shape.save_suggestion.is_none());
    }

    #[test]
    fn shape_for_nonsecret_read_has_suggestion() {
        let ws = temp_ws("shape-read");
        let sandbox = enabled(&ws);
        let path = sandbox.workspace.join("docs/readme.md");
        let args = serde_json::json!({ "path": path.to_string_lossy() });
        let shape = approval_shape("read", &path, Op::Read, &args, &sandbox);
        assert_eq!(shape.save_suggestion.unwrap()["access"], "read");
    }
}
