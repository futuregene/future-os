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
        // Off tier runs fully open — no approval at all.
        if !sandbox.enabled() {
            return None;
        }
        // shell. Sandbox tier wraps it in Seatbelt (no pre-approval; boundary
        // hits surface via escalation). Manual tier gates it with a read-only
        // whitelist (Option B): known-safe commands auto-run, everything else
        // asks. shell approvals are allow-once only (no persisted command rule).
        if tool_name == "shell" {
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
            let outcome = self.ask_user(
                broadcaster,
                session_id,
                tool_id,
                tool_name,
                &shape,
                normalize_requested_action(arguments),
            );
            return match outcome {
                AskOutcome::Approved => None,
                AskOutcome::Cancelled(_) => Some(crate::types::ToolCallResult {
                    result: "Tool call `shell` was cancelled because the approval request ended."
                        .to_string(),
                    is_error: true,
                }),
                AskOutcome::Rejected(note) => Some(crate::types::ToolCallResult {
                    result: format!(
                        "Tool call `shell` was rejected by the user{}.",
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

    /// Post-hoc approval for running a shell command outside the sandbox
    /// (SANDBOX_PLAN.md §2.6). Approval means the single re-run happens
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
        let save_suggestion = escalation_save_suggestion(&raw_blocked, sandbox);
        let action = serde_json::json!({
            "tool": "shell",
            "category": "sandbox_escalation",
            "summary": command_summary(&request.command),
            "command": request.command,
            "justification": request.justification,
            "blocked_paths": blocked_paths,
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

        match self.ask_user(
            broadcaster,
            session_id,
            "",
            "shell",
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

/// Programs that only read (no filesystem writes, no arbitrary exec).
const READONLY_PROGRAMS: &[&str] = &[
    "cat", "ls", "pwd", "echo", "printf", "find", "grep", "egrep", "fgrep", "rg", "ag", "head",
    "tail", "wc", "sort", "uniq", "cut", "tr", "nl", "fold", "column", "file", "stat", "du", "df",
    "date", "whoami", "hostname", "id", "uname", "env", "printenv", "which", "type", "dirname",
    "basename", "realpath", "readlink", "tree", "diff", "cmp", "comm", "less", "more", "true",
    "false", "test", "seq", "yes",
];

/// Read-only PowerShell cmdlets and their built-in aliases (Windows). Compared
/// lower-cased since PowerShell resolves command names case-insensitively. The
/// aliases `ls`/`cat`/`pwd`/`echo`/`type`/`sort` already resolve via
/// READONLY_PROGRAMS. Deliberately excludes anything that writes (Set-/Add-/
/// New-/Remove-/Out-File) or runs arbitrary code (Invoke-/Start-/ForEach-Object/
/// Where-Object — the latter also need a `{ … }` block, which is rejected).
const WINDOWS_READONLY_PROGRAMS: &[&str] = &[
    "get-childitem",
    "gci",
    "dir",
    "get-content",
    "gc",
    "get-location",
    "gl",
    "get-item",
    "gi",
    "get-itemproperty",
    "get-command",
    "gcm",
    "get-date",
    "get-help",
    "select-string",
    "sls",
    "select-object",
    "select",
    "sort-object",
    "measure-object",
    "measure",
    "write-output",
    "write-host",
    "format-table",
    "ft",
    "format-list",
    "fl",
    "out-string",
    "test-path",
    "resolve-path",
    "split-path",
    "compare-object",
    "findstr",
    "where",
];

/// git subcommands that don't mutate the repo or working tree.
const GIT_READONLY: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "tag",
    "describe",
    "blame",
    "shortlog",
    "ls-files",
    "ls-tree",
    "rev-parse",
    "rev-list",
    "remote",
    "reflog",
    "cat-file",
    "grep",
];

/// Read-only shell whitelist for the Manual tier: known-safe commands auto-run,
/// everything else asks. Conservative — anything with shell operators that could
/// write, chain, background, or substitute falls through to "ask". Covers both
/// POSIX shells and PowerShell (see WINDOWS_READONLY_PROGRAMS); the operator
/// guard below (`{`, `` ` ``, `;`, …) also blocks PowerShell script blocks and
/// subexpressions.
fn shell_auto_allow(command: &str) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() {
        return false;
    }
    // Reject write / exec / chain / substitution operators outright. `&` also
    // catches `&&` and backgrounding; `` ` `` and `$(` catch substitution;
    // `{`/`(` catch PowerShell script blocks and subexpressions.
    const DANGEROUS: &[char] = &['>', '<', '`', ';', '&', '\n', '(', '{'];
    if cmd.contains("$(") || cmd.chars().any(|c| DANGEROUS.contains(&c)) {
        return false;
    }
    cmd.split('|').all(|seg| segment_is_read_only(seg.trim()))
}

/// Whether a single pipe segment invokes only a read-only program.
fn segment_is_read_only(seg: &str) -> bool {
    let mut words = seg.split_whitespace();
    let Some(prog) = words.next() else {
        return false;
    };
    // Reduce a leading path to its basename — tolerant of both separators
    // (`/bin/ls`, `C:\Windows\System32\findstr.exe`), a `.exe` suffix, and
    // case (PowerShell and Windows resolve command names case-insensitively).
    let base = prog.rsplit(['/', '\\']).next().unwrap_or(prog);
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    let prog = base.to_ascii_lowercase();
    match prog.as_str() {
        "git" => {
            // First non-flag token is the subcommand; it must be read-only.
            let sub = words.find(|w| !w.starts_with('-'));
            matches!(sub, Some(s) if GIT_READONLY.contains(&s))
        }
        "find" => {
            // find only reads unless it mutates or executes.
            !seg.contains("-exec")
                && !seg.contains("-delete")
                && !seg.contains("-fprint")
                && !seg.contains("-ok")
        }
        other => READONLY_PROGRAMS.contains(&other) || WINDOWS_READONLY_PROGRAMS.contains(&other),
    }
}

/// Approval card for a shell command that isn't auto-allowed (Manual tier).
fn shell_command_shape(command: &str, sandbox: &ResolvedSandbox) -> ApprovalShape {
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

/// Raw (absolute) file paths a sandbox denial mentions. Reads lines with
/// "Operation not permitted" and pulls the path from quotes (`'…'` / `"…"`) or
/// the first absolute-path token. Deduped, capped, order-preserving.
fn extract_blocked_paths_raw(stderr: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in stderr.lines() {
        if !line.contains("Operation not permitted") {
            continue;
        }
        let Some(raw) = quoted_path(line).or_else(|| absolute_path_token(line)) else {
            continue;
        };
        if !out.contains(&raw) {
            out.push(raw);
        }
        if out.len() >= 5 {
            break;
        }
    }
    out
}

/// Shorten `$HOME` to `~` for display.
fn shorten_home(path: &str) -> String {
    match dirs::home_dir() {
        Some(home) if path.starts_with(home.to_string_lossy().as_ref()) => {
            path.replacen(home.to_string_lossy().as_ref(), "~", 1)
        }
        _ => path.to_string(),
    }
}

/// Card-friendly blocked paths (`$HOME` → `~`), for display only.
#[cfg(test)]
fn extract_blocked_paths(stderr: &str) -> Vec<String> {
    extract_blocked_paths_raw(stderr)
        .iter()
        .map(|p| shorten_home(p))
        .collect()
}

/// "Allow in this workspace/chat" suggestion for an escalation: the parent
/// directory of the (first) blocked path. Returns `None` when any blocked path
/// is a secret (secrets stay one-time-only) or none is known — so the card
/// shows only "allow once" for those. Access is `write`: reads of non-secrets
/// are open, so a non-secret denial is a write.
fn escalation_save_suggestion(
    raw_paths: &[String],
    sandbox: &ResolvedSandbox,
) -> Option<serde_json::Value> {
    if raw_paths.is_empty() {
        return None;
    }
    if raw_paths
        .iter()
        .any(|p| sandbox.is_secret_path(Path::new(p)))
    {
        return None;
    }
    let parent = Path::new(&raw_paths[0]).parent()?;
    let glob = match parent.strip_prefix(&sandbox.workspace) {
        Ok(rel) if rel.as_os_str().is_empty() => "*".to_string(),
        Ok(rel) => format!("{}/*", rel.to_string_lossy()),
        // Outside the workspace: keep it portable with `~` when under home.
        Err(_) => format!("{}/*", shorten_home(&parent.to_string_lossy())),
    };
    Some(serde_json::json!({
        "path": glob,
        "access": "write",
        "action": "allow",
    }))
}

/// First `'…'` or `"…"` span that looks like a path (contains `/`).
fn quoted_path(line: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        let mut parts = line.split(quote);
        // parts alternate outside/inside the quote; index 1, 3, … are inside.
        parts.next();
        while let Some(inside) = parts.next() {
            if inside.contains('/') {
                return Some(inside.to_string());
            }
            parts.next(); // skip the following outside span
        }
    }
    None
}

/// First whitespace token that is an absolute path, trimming trailing `:`/`,`.
fn absolute_path_token(line: &str) -> Option<String> {
    line.split_whitespace()
        .map(|tok| tok.trim_end_matches([':', ',']))
        .find(|tok| tok.starts_with('/') && tok.len() > 1)
        .map(str::to_string)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxPolicy;

    #[test]
    fn shell_whitelist_allows_read_only_commands() {
        assert!(shell_auto_allow("ls -la"));
        assert!(shell_auto_allow("cat README.md"));
        assert!(shell_auto_allow("git status"));
        assert!(shell_auto_allow("git log --oneline"));
        assert!(shell_auto_allow("grep -rn foo src | head -20"));
        assert!(shell_auto_allow("/bin/ls"));
        assert!(shell_auto_allow("find . -name '*.rs'"));
    }

    #[test]
    fn shell_whitelist_asks_for_writes_and_chains() {
        assert!(!shell_auto_allow("rm -rf build"));
        assert!(!shell_auto_allow("echo hi > file.txt")); // redirect
        assert!(!shell_auto_allow("git commit -m x")); // mutating subcommand
        assert!(!shell_auto_allow("ls && rm x")); // chain
        assert!(!shell_auto_allow("cat $(whoami)")); // substitution
        assert!(!shell_auto_allow("find . -delete")); // find mutation
        assert!(!shell_auto_allow("grep foo x | rm y")); // pipe to non-read-only
        assert!(!shell_auto_allow("npm install")); // unknown program
        assert!(!shell_auto_allow(""));
    }

    #[test]
    fn shell_whitelist_allows_read_only_powershell() {
        // Cmdlets and aliases, case-insensitive, with Windows paths / .exe.
        assert!(shell_auto_allow("Get-ChildItem"));
        assert!(shell_auto_allow("get-content foo.txt"));
        assert!(shell_auto_allow("Select-String -Pattern foo bar.txt"));
        assert!(shell_auto_allow(
            "Get-ChildItem -Recurse | Select-String foo"
        ));
        assert!(shell_auto_allow("gci | measure"));
        assert!(shell_auto_allow(
            r"C:\Windows\System32\findstr.exe foo bar.txt"
        ));
    }

    #[test]
    fn shell_whitelist_asks_for_powershell_writes_and_blocks() {
        assert!(!shell_auto_allow("Remove-Item x")); // mutating cmdlet
        assert!(!shell_auto_allow("Set-Content foo.txt 'x'")); // writes
        assert!(!shell_auto_allow("Get-Content x > out.txt")); // redirect
        assert!(!shell_auto_allow("Get-ChildItem; Remove-Item x")); // chain
        assert!(!shell_auto_allow("Where-Object { $_.Length -gt 0 }")); // script block
        assert!(!shell_auto_allow("Invoke-Expression 'rm x'")); // arbitrary exec
    }

    #[test]
    fn extract_blocked_paths_from_gpg_denial() {
        let stderr = "\
[exit code: 128]
error: gpg failed to sign the data:
gpg: failed to create temporary file '/Users/x/.gnupg/.#lk0x001.host.9334': Operation not permitted
gpg: 密钥区块资源 '/Users/x/.gnupg/pubring.kbx': Operation not permitted
[GNUPG:] ERROR add_keyblock_resource 33587307";
        let paths = extract_blocked_paths(stderr);
        assert_eq!(
            paths,
            vec![
                "/Users/x/.gnupg/.#lk0x001.host.9334".to_string(),
                "/Users/x/.gnupg/pubring.kbx".to_string(),
            ]
        );
    }

    #[test]
    fn escalation_suggests_parent_for_nonsecret_but_not_secret() {
        let ws = temp_ws("escalation-sug");
        let sandbox = ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Manual,
            },
            &ws,
        );
        let home = dirs::home_dir().unwrap();

        // Non-secret outside-workspace write → suggest the parent dir (~ form).
        let desktop = home.join("Desktop/note.txt");
        let sug = escalation_save_suggestion(&[desktop.to_string_lossy().into_owned()], &sandbox)
            .expect("non-secret blocked path should be persistable");
        assert_eq!(sug["path"], "~/Desktop/*");
        assert_eq!(sug["access"], "write");

        // A secret blocked path (~/.gnupg) → no persistence (one-time only).
        let gnupg = home.join(".gnupg/pubring.kbx");
        assert!(
            escalation_save_suggestion(&[gnupg.to_string_lossy().into_owned()], &sandbox).is_none()
        );
        // Mixed (one secret) → still none.
        assert!(escalation_save_suggestion(
            &[
                desktop.to_string_lossy().into_owned(),
                gnupg.to_string_lossy().into_owned()
            ],
            &sandbox
        )
        .is_none());
        // No known paths → none.
        assert!(escalation_save_suggestion(&[], &sandbox).is_none());
    }

    #[test]
    fn extract_blocked_paths_unquoted_and_deduped() {
        let stderr = "touch: /etc/hosts: Operation not permitted\n\
                      touch: /etc/hosts: Operation not permitted";
        assert_eq!(
            extract_blocked_paths(stderr),
            vec!["/etc/hosts".to_string()]
        );
        // Non-denial lines are ignored.
        assert!(extract_blocked_paths("error[E0308]: mismatched types").is_empty());
    }

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
        ResolvedSandbox::resolve(
            &SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Manual,
            },
            ws,
        )
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
    fn shell_read_only_auto_allowed_in_manual() {
        // Manual tier: read-only whitelist commands run without a prompt.
        let ws = temp_ws("shell-ro");
        let sandbox = enabled(&ws);
        let gate = ApprovalGate::default();
        let b = SseBroadcaster::new();
        let args = serde_json::json!({ "command": "ls -la" });
        assert!(gate
            .request(&b, "s", &ws, "shell", "t", &args, &sandbox)
            .is_none());
    }

    #[test]
    fn shell_never_gated_when_disabled() {
        // Off tier: no approval at all, even for a dangerous command.
        let ws = temp_ws("shell-off");
        let sandbox = ResolvedSandbox::disabled(&ws);
        let gate = ApprovalGate::default();
        let b = SseBroadcaster::new();
        let args = serde_json::json!({ "command": "rm -rf /" });
        assert!(gate
            .request(&b, "s", &ws, "shell", "t", &args, &sandbox)
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
