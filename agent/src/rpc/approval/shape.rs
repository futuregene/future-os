use super::ApprovalShape;
use crate::sandbox::windows_request::PreparedWritePermissions;
use crate::sandbox::{paths, rules::Op, ResolvedSandbox};
use std::path::Path;
pub(super) fn windows_capability_shape(
    command: &str,
    prepared: &PreparedWritePermissions,
    sandbox: &ResolvedSandbox,
) -> ApprovalShape {
    let approval = prepared
        .approval
        .as_ref()
        .expect("shape is only built for targets that require approval");
    let paths = approval
        .targets
        .iter()
        .map(|target| target.path.clone())
        .collect::<Vec<_>>();
    let title = if let [target] = approval.targets.as_slice() {
        match target.scope {
            crate::sandbox::windows_request::WriteScope::File => {
                format!("Allow FutureOS to modify {}?", target.path)
            }
            crate::sandbox::windows_request::WriteScope::Subtree => {
                format!("Allow FutureOS to manage files in {}?", target.path)
            }
        }
    } else {
        format!(
            "Allow FutureOS to manage files in these {} locations?",
            approval.targets.len()
        )
    };
    let save_suggestion = if approval
        .targets
        .iter()
        .any(|target| sandbox.is_secret_path(Path::new(&target.path)))
    {
        None
    } else {
        Some(serde_json::json!({
            "rules": approval.targets.iter().map(|target| serde_json::json!({
                "path": target.path,
                "access": "write"
            })).collect::<Vec<_>>()
        }))
    };

    ApprovalShape {
        kind: "windows_write_capability",
        risk_level: "medium",
        title,
        summary: if approval.targets.len() == 1 {
            "FutureOS needs write access to this location for the current command.".to_string()
        } else {
            "FutureOS needs write access to all of these locations for the current command."
                .to_string()
        },
        action: serde_json::json!({
            "tool": "shell",
            "category": "windows_write_capability",
            "behavior": approval.behavior,
            "targets": approval.targets,
            "paths": paths,
            // Existing clients already render this in a collapsed command
            // disclosure. It is never used to construct the trusted title.
            "command": command,
            "scope": {
                "cwd": sandbox.workspace.to_string_lossy(),
                "inside_workspace": false,
                "estimated_blast_radius": "medium"
            }
        }),
        sandbox_boundary: sandbox.boundary_json(Some("additional_write_capability"), false),
        save_suggestion,
    }
}

/// Shape the approval card for a file access that resolved to `Ask`. `path` is
/// the resolved absolute target; `op` its read/write nature.
pub(super) fn approval_shape(
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
pub(super) fn path_save_suggestion(
    path: &Path,
    op: Op,
    workspace: &Path,
) -> Option<serde_json::Value> {
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

/// Manual-tier exemptions are deliberately limited to literal introspection.
/// A program basename does not prove safety: env executes programs, git and
/// text utilities have write/exec options, and even read-only commands can read
/// secrets. Do not try to infer paths from shell syntax; file reads should use
/// the path-aware read tool or ask for approval. OS-wrapped shells and explicit
/// full-permission sessions retain their existing policy.
pub(super) fn shell_auto_allow(command: &str) -> bool {
    // pwd is a shell builtin (PowerShell's built-in Get-Location alias), not
    // a PATH lookup. Even plain `ls` can resolve to a workspace-planted binary.
    command.trim() == "pwd"
}

/// Approval card for a shell command that isn't auto-allowed (Manual tier).
pub(super) fn shell_command_shape(command: &str, sandbox: &ResolvedSandbox) -> ApprovalShape {
    ApprovalShape {
        kind: "shell_command",
        risk_level: "medium",
        title: "Approve shell command".to_string(),
        summary: "Agent wants to run a shell command.".to_string(),
        action: serde_json::json!({
            "tool": "shell",
            "category": "shell_command",
            "summary": command_summary(command),
            "command": command,
            "scope": {
                "cwd": sandbox.workspace.to_string_lossy(),
                "inside_workspace": true,
                "estimated_blast_radius": "medium"
            }
        }),
        sandbox_boundary: sandbox.boundary_json(Some("shell_command"), false),
        // shell approvals are one-time only — v2 rules are path-based, not command-based.
        save_suggestion: None,
    }
}

pub(super) fn command_summary(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.len() <= 200 {
        trimmed.to_string()
    } else {
        let mut head: String = trimmed.chars().take(200).collect();
        head.push('\u{2026}');
        head
    }
}

/// Best-effort paths mentioned by denial diagnostics, not an inventory of all
/// files the command accesses. Linux EACCES/EROFS are included for approval
/// display only; this parser must never decide whether to retry unsandboxed.
pub(super) fn argument_write_preview(arguments: &serde_json::Value) -> Option<String> {
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

pub(super) fn normalize_requested_action(arguments: &serde_json::Value) -> serde_json::Value {
    match arguments {
        serde_json::Value::String(raw) => serde_json::from_str(raw)
            .ok()
            .or_else(|| repair_partial_json_object(raw))
            .unwrap_or_else(|| arguments.clone()),
        _ => arguments.clone(),
    }
}

pub(super) fn repair_partial_json_object(raw: &str) -> Option<serde_json::Value> {
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

pub(super) fn has_unclosed_string(value: &str) -> bool {
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
