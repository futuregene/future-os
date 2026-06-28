use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};

use super::approval_policy::{evaluate_policy, PolicyDecision};
use super::{SseBroadcaster, SseEvent};

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

impl ApprovalGate {
    pub fn request(
        &self,
        broadcaster: &SseBroadcaster,
        session_id: &str,
        cwd: &str,
        tool_name: &str,
        tool_id: &str,
        arguments: &serde_json::Value,
    ) -> Option<crate::types::ToolCallResult> {
        let Some(shape) = approval_shape(cwd, tool_name, arguments) else {
            return None;
        };

        // Policy evaluation hook (stub). Today this always returns AskUser.
        // Future: rule-based auto-approve / auto-reject lives here without
        // touching the rest of this function.
        match evaluate_policy(cwd, tool_name, arguments, &shape) {
            PolicyDecision::AskUser => {}
            PolicyDecision::AutoApprove => {
                if let Some(path) = approved_argument_path(cwd, arguments) {
                    crate::tools::approve_outside_path(&path);
                }
                return None;
            }
            PolicyDecision::AutoReject(reason) => {
                return Some(crate::types::ToolCallResult {
                    result: format!("Tool call `{tool_name}` was rejected by policy: {reason}"),
                    is_error: true,
                });
            }
        }

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
                "requested_action": normalize_requested_action(arguments),
                "action": shape.action,
                "sandbox_boundary": shape.sandbox_boundary,
                "reviewer": "user",
            }),
        ));

        let decision = tokio::task::block_in_place(|| rx.recv());
        match decision {
            Ok(decision) if decision.approved => {
                if let Some(path) = approved_argument_path(cwd, arguments) {
                    crate::tools::approve_outside_path(&path);
                }
                broadcaster.broadcast(SseEvent::new(
                    "approval_decision",
                    serde_json::json!({
                        "type": "approval_decision",
                        "approval_request_id": request_id,
                        "tool_id": tool_id,
                        "status": "approved",
                        "note": decision.note,
                    }),
                ));
                None
            }
            Ok(decision) if decision.status == ApprovalDecisionStatus::Cancelled => {
                broadcaster.broadcast(SseEvent::new(
                    "approval_decision",
                    serde_json::json!({
                        "type": "approval_decision",
                        "approval_request_id": request_id,
                        "tool_id": tool_id,
                        "status": "cancelled",
                        "note": decision.note,
                    }),
                ));
                Some(crate::types::ToolCallResult {
                    result: format!(
                        "Tool call `{tool_name}` was cancelled because the approval request ended."
                    ),
                    is_error: true,
                })
            }
            Ok(decision) => {
                broadcaster.broadcast(SseEvent::new(
                    "approval_decision",
                    serde_json::json!({
                        "type": "approval_decision",
                        "approval_request_id": request_id,
                        "tool_id": tool_id,
                        "status": "rejected",
                        "note": decision.note,
                    }),
                ));
                Some(crate::types::ToolCallResult {
                    result: format!(
                        "Tool call `{}` was rejected by the user{}.",
                        tool_name,
                        if decision.note.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", decision.note)
                        }
                    ),
                    is_error: true,
                })
            }
            Err(_) => {
                self.pending.lock().unwrap().remove(&request_id);
                broadcaster.broadcast(SseEvent::new(
                    "approval_decision",
                    serde_json::json!({
                        "type": "approval_decision",
                        "approval_request_id": request_id,
                        "tool_id": tool_id,
                        "status": "cancelled",
                        "note": "Approval request was cancelled because the session ended.",
                    }),
                ));
                Some(crate::types::ToolCallResult {
                    result: format!(
                        "Tool call `{tool_name}` was cancelled because the approval request ended."
                    ),
                    is_error: true,
                })
            }
        }
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
                .filter_map(|(request_id, pending)| {
                    (pending.session_id == session_id).then(|| request_id.clone())
                })
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
}

fn approval_shape(
    cwd: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Option<ApprovalShape> {
    match tool_name {
        "bash" if bash_requires_approval(cwd, arguments) => {
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
            let sandbox_boundary = serde_json::json!({
                "mode": "workspace-write",
                "inside_sandbox": false,
                "violation": "shell_command_not_in_allowlist",
                "cwd": cwd,
                "writable_roots": [cwd]
            });
            Some(ApprovalShape {
                kind: "shell_command",
                risk_level: "high",
                title: "Approve shell command".to_string(),
                summary: "Agent wants to run a shell command.".to_string(),
                action,
                sandbox_boundary,
            })
        }
        "bash" => None,
        "write" | "edit" if path_is_outside_workspace(cwd, arguments) => {
            let path = argument_path(arguments).unwrap_or_default();
            let preview = argument_write_preview(arguments);
            let category = if tool_name == "write" {
                "file_write"
            } else {
                "file_write"
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
                    "inside_workspace": false,
                    "estimated_blast_radius": "medium"
                }
            });
            let sandbox_boundary = serde_json::json!({
                "mode": "workspace-write",
                "inside_sandbox": false,
                "violation": "outside_workspace_write",
                "cwd": cwd,
                "writable_roots": [cwd]
            });
            Some(ApprovalShape {
                kind: "outside_workspace_write",
                risk_level: "medium",
                title: "Approve outside-workspace write".to_string(),
                summary: "Agent wants to modify a file outside the current workspace.".to_string(),
                action,
                sandbox_boundary,
            })
        }
        _ => None,
    }
}

fn command_summary(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.len() <= 200 {
        trimmed.to_string()
    } else {
        let mut head: String = trimmed.chars().take(200).collect();
        head.push_str("\u{2026}");
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
                head.push_str("\u{2026}");
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

fn argument_command(arguments: &serde_json::Value) -> Option<String> {
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
                | "future-cli"
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

fn path_is_outside_workspace(cwd: &str, arguments: &serde_json::Value) -> bool {
    let Some(path) = argument_path(arguments) else {
        return false;
    };
    let workspace = PathBuf::from(cwd);
    let candidate = if let Some(relative) = path.strip_prefix("~/") {
        workspace.join(relative)
    } else if Path::new(&path).is_absolute() {
        PathBuf::from(path)
    } else {
        workspace.join(path)
    };
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let candidate = candidate.canonicalize().unwrap_or(candidate);
    !candidate.starts_with(workspace)
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

fn approved_argument_path(cwd: &str, arguments: &serde_json::Value) -> Option<String> {
    let path = argument_path(arguments)?;
    let workspace = PathBuf::from(cwd);
    let candidate = if let Some(relative) = path.strip_prefix("~/") {
        workspace.join(relative)
    } else if Path::new(&path).is_absolute() {
        PathBuf::from(path)
    } else {
        workspace.join(path)
    };
    Some(candidate.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("futureos-approval-{name}-{stamp}"))
    }

    #[test]
    fn write_inside_workspace_does_not_require_approval() {
        let workspace = temp_path("workspace");
        let args = serde_json::json!({
            "path": workspace.join("poem.txt").to_string_lossy(),
            "content": "hello"
        });

        assert!(approval_shape(workspace.to_string_lossy().as_ref(), "write", &args).is_none());
    }

    #[test]
    fn write_outside_workspace_requires_approval() {
        let workspace = temp_path("workspace");
        let outside = temp_path("outside.txt");
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": "hello"
        });

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "write", &args)
            .expect("outside workspace write should require approval");
        assert_eq!(shape.kind, "outside_workspace_write");
        assert_eq!(shape.risk_level, "medium");
    }

    #[test]
    fn edit_outside_workspace_requires_approval_from_json_string_args() {
        let workspace = temp_path("workspace");
        let outside = temp_path("outside.txt");
        let args = serde_json::json!(serde_json::json!({
            "path": outside.to_string_lossy(),
            "oldText": "a",
            "newText": "b"
        })
        .to_string());

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "edit", &args)
            .expect("outside workspace edit should require approval");
        assert_eq!(shape.kind, "outside_workspace_write");
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
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("futureos-approval-{name}-{stamp}"))
    }

    #[test]
    fn shell_command_action_is_structured() {
        let workspace = temp_path("workspace");
        let args = serde_json::json!({
            "command": "rm -rf node_modules"
        });

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "bash", &args)
            .expect("rm should require approval");

        assert_eq!(shape.action["tool"], "bash");
        assert_eq!(shape.action["category"], "shell_command");
        assert_eq!(shape.action["command"], "rm -rf node_modules");
        assert_eq!(shape.action["scope"]["inside_workspace"], true);
        assert_eq!(shape.action["scope"]["estimated_blast_radius"], "high");
    }

    #[test]
    fn shell_command_sandbox_boundary_is_structured() {
        let workspace = temp_path("workspace");
        let args = serde_json::json!({
            "command": "rm -rf node_modules"
        });

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "bash", &args)
            .expect("rm should require approval");

        assert_eq!(shape.sandbox_boundary["mode"], "workspace-write");
        assert_eq!(shape.sandbox_boundary["inside_sandbox"], false);
        assert_eq!(
            shape.sandbox_boundary["violation"],
            "shell_command_not_in_allowlist"
        );
        assert_eq!(
            shape.sandbox_boundary["cwd"],
            workspace.to_string_lossy().to_string()
        );
    }

    #[test]
    fn file_write_action_is_structured() {
        let workspace = temp_path("workspace");
        let outside = temp_path("outside.txt");
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": "hello world"
        });

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "write", &args)
            .expect("outside workspace write should require approval");

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
        let outside = temp_path("outside.txt");
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": "hello"
        });

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "write", &args)
            .expect("outside workspace write should require approval");

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
        let outside = temp_path("outside.txt");
        let long_content = "x".repeat(300);
        let args = serde_json::json!({
            "path": outside.to_string_lossy(),
            "content": long_content
        });

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "write", &args)
            .expect("outside workspace write should require approval");

        let preview = shape.action["writes"][0]["preview"].as_str().unwrap();
        assert_eq!(preview.chars().count(), 201); // 200 chars + ellipsis
        assert!(preview.ends_with("…"));
    }

    #[test]
    fn policy_evaluator_returns_ask_user_by_default() {
        let workspace = temp_path("workspace");
        let args = serde_json::json!({
            "command": "rm -rf node_modules"
        });

        let shape = approval_shape(workspace.to_string_lossy().as_ref(), "bash", &args)
            .expect("rm should require approval");

        let decision = evaluate_policy(workspace.to_string_lossy().as_ref(), "bash", &args, &shape);

        assert!(matches!(decision, PolicyDecision::AskUser));
    }
}
