use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

use super::{SseBroadcaster, SseEvent};

#[derive(Clone, Default)]
pub struct ApprovalGate {
    pending: Arc<Mutex<HashMap<String, mpsc::Sender<ApprovalDecision>>>>,
}

#[derive(Debug, Clone)]
pub struct ApprovalDecision {
    pub approved: bool,
    pub note: String,
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

        let request_id = format!("approval_{}", crate::utils::generate_entry_id());
        let (tx, rx) = mpsc::channel::<ApprovalDecision>();
        self.pending.lock().unwrap().insert(request_id.clone(), tx);

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
            }),
        ));

        let decision =
            tokio::task::block_in_place(|| rx.recv_timeout(Duration::from_secs(60 * 30)));
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
                        "note": "Approval request timed out.",
                    }),
                ));
                Some(crate::types::ToolCallResult {
                    result: format!(
                        "Tool call `{tool_name}` was cancelled because approval timed out."
                    ),
                    is_error: true,
                })
            }
        }
    }

    pub fn decide(&self, request_id: &str, decision: ApprovalDecision) -> Result<(), String> {
        let tx = self
            .pending
            .lock()
            .unwrap()
            .remove(request_id)
            .ok_or_else(|| format!("approval request `{request_id}` is not pending"))?;
        tx.send(decision).map_err(|error| error.to_string())
    }
}

struct ApprovalShape {
    kind: &'static str,
    risk_level: &'static str,
    title: String,
    summary: String,
}

fn approval_shape(
    cwd: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Option<ApprovalShape> {
    match tool_name {
        "bash" if bash_requires_approval(cwd, arguments) => Some(ApprovalShape {
            kind: "shell_command",
            risk_level: "high",
            title: "Approve shell command".to_string(),
            summary: "Agent wants to run a shell command.".to_string(),
        }),
        "bash" => None,
        "write" | "edit" if path_is_outside_workspace(cwd, arguments) => Some(ApprovalShape {
            kind: "outside_workspace_write",
            risk_level: "medium",
            title: "Approve outside-workspace write".to_string(),
            summary: "Agent wants to modify a file outside the current workspace.".to_string(),
        }),
        _ => None,
    }
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
            "cat" | "find" | "grep" | "head" | "ls" | "pwd" | "rg" | "sed" | "tail" | "wc"
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
