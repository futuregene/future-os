//! Tools — 1:1 compatible with Go internal/tools/

mod cmd_exe_rewrite;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::sandbox::{EscalationDecision, EscalationRequest, EscalationRequester, ResolvedSandbox};

/// Callback invoked when a shell command is about to run inside the OS
/// sandbox (the RPC layer wires this to a `tool_sandboxed` SSE event).
pub type SandboxedNotifier = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct ToolExecutionScope {
    workspace: PathBuf,
    approved_outside_paths: Arc<Mutex<Vec<PathBuf>>>,
    approved_windows_capabilities:
        Arc<Mutex<Vec<crate::sandbox::windows_request::ApprovedWriteCapability>>>,
    /// "all" | "workspace" | "none" — controls workspace boundary enforcement
    permission_level: String,
    /// Interrupt flag for cooperative cancellation of long-running tool operations
    /// (e.g., shell commands). When set, in-flight tool work returns an "interrupted"
    /// error promptly and child processes are dropped (kill_on_drop).
    interrupt_flag: Arc<AtomicBool>,
    /// Resolved sandbox boundary: OS sandbox wrapping for shell runs, writable-roots
    /// boundary for write/edit. Shared with the approval layer so both reach
    /// the same verdicts.
    sandbox: Arc<ResolvedSandbox>,
    /// Post-hoc approval hook for escalated (out-of-sandbox) shell runs.
    /// Injected by the RPC layer; None means escalation is unavailable.
    escalation: Option<EscalationRequester>,
    /// Notifier for sandboxed shell executions (progress/event plumbing).
    on_sandboxed: Option<SandboxedNotifier>,
}

tokio::task_local! {
    static TOOL_SCOPE: ToolExecutionScope;
}

/// Full scope configuration for tool execution.
pub struct ScopeOptions {
    pub workspace: String,
    pub permission_level: String,
    pub interrupt_flag: Arc<AtomicBool>,
    pub sandbox: Arc<ResolvedSandbox>,
    pub escalation: Option<EscalationRequester>,
    pub on_sandboxed: Option<SandboxedNotifier>,
}

pub async fn with_tool_scope<F>(options: ScopeOptions, future: F) -> F::Output
where
    F: Future,
{
    let scope = ToolExecutionScope {
        workspace: crate::sandbox::paths::normalize_lexically(&PathBuf::from(options.workspace)),
        approved_outside_paths: Arc::new(Mutex::new(vec![])),
        approved_windows_capabilities: Arc::new(Mutex::new(vec![])),
        permission_level: options.permission_level,
        interrupt_flag: options.interrupt_flag,
        sandbox: options.sandbox,
        escalation: options.escalation,
        on_sandboxed: options.on_sandboxed,
    };
    TOOL_SCOPE.scope(scope, future).await
}

pub async fn with_workspace_scope<F>(
    workspace: String,
    permission_level: String,
    future: F,
) -> F::Output
where
    F: Future,
{
    with_workspace_scope_with_interrupt(
        workspace,
        permission_level,
        Arc::new(AtomicBool::new(false)),
        future,
    )
    .await
}

pub async fn with_workspace_scope_with_interrupt<F>(
    workspace: String,
    permission_level: String,
    interrupt_flag: Arc<AtomicBool>,
    future: F,
) -> F::Output
where
    F: Future,
{
    // Legacy entry point: dormant sandbox (no OS wrapping, workspace-only
    // boundary) — identical to pre-sandbox behavior. The RPC layer uses
    // with_tool_scope directly with the session's resolved policy.
    let sandbox = ResolvedSandbox::disabled(&workspace);
    with_tool_scope(
        ScopeOptions {
            workspace,
            permission_level,
            interrupt_flag,
            sandbox: Arc::new(sandbox),
            escalation: None,
            on_sandboxed: None,
        },
        future,
    )
    .await
}

pub fn approve_outside_path(path: &str) {
    // Canonicalize so the later boundary check (which also canonicalizes)
    // matches regardless of symlinks/case (§3.5).
    let path = crate::sandbox::paths::canonicalize_lenient(&PathBuf::from(path));
    let _ = TOOL_SCOPE.try_with(|scope| {
        scope.approved_outside_paths.lock().push(path);
    });
}

pub(crate) fn approve_windows_capability(
    receipt: crate::sandbox::windows_request::ApprovedWriteCapability,
) {
    let _ = TOOL_SCOPE.try_with(|scope| {
        scope.approved_windows_capabilities.lock().push(receipt);
    });
}

fn consume_windows_capability(
    prepared: &crate::sandbox::windows_request::PreparedWritePermissions,
) -> Option<crate::sandbox::windows_request::ApprovedWriteCapability> {
    let expected_targets = &prepared.approval.as_ref()?.targets;
    TOOL_SCOPE
        .try_with(|scope| {
            let mut receipts = scope.approved_windows_capabilities.lock();
            let index = receipts.iter().position(|receipt| {
                windows_capability_receipt_matches(prepared, expected_targets, receipt)
            })?;
            Some(receipts.remove(index))
        })
        .ok()
        .flatten()
}

fn windows_capability_receipt_matches(
    prepared: &crate::sandbox::windows_request::PreparedWritePermissions,
    expected_targets: &[crate::sandbox::windows_request::ApprovalTarget],
    receipt: &crate::sandbox::windows_request::ApprovedWriteCapability,
) -> bool {
    receipt.command_hash == prepared.command_hash && receipt.targets == expected_targets
}

// ─── Tool definitions ────────────────────────────────────────────────────────

use crate::types::AgentTool;
use crate::types::FunctionDef;
use crate::types::ToolDef;
use crate::types::ToolHandler;

fn make_tool(
    name: &str,
    description: &str,
    parameters: serde_json::Value,
    handler: ToolHandler,
    guidelines: Vec<&str>,
) -> AgentTool {
    AgentTool {
        def: ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: name.to_string(),
                description: description.to_string(),
                parameters,
            },
        },
        handler,
        guidelines: guidelines.into_iter().map(String::from).collect(),
    }
}

// ─── Shell Tool ───────────────────────────────────────────────────────────────

fn shell_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "command": {
                "type": "string",
                "description": "The shell command to execute"
            },
            "timeout": {
                "type": "integer",
                "description": "Optional timeout in seconds"
            },
            "escalated": {
                "type": "boolean",
                "description": "Request to run this command outside the sandbox (requires user approval). Set only after a command failed due to sandbox restrictions (blocked network or a write outside the workspace) and it genuinely needs those permissions."
            },
            "justification": {
                "type": "string",
                "description": "One-sentence reason why escalated permissions are needed. Required when escalated is true."
            },
            "additional_permissions": {
                "type": "object",
                "description": "Windows write-protection only. Declare each additional path the command must write before execution. This never grants access by itself.",
                "properties": {
                    "write": {
                        "type": "array",
                        "maxItems": 8,
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "A literal path. Wildcards are not accepted."
                                },
                                "scope": {
                                    "type": "string",
                                    "enum": ["file", "subtree"],
                                    "description": "file is one existing regular file; subtree is one existing directory and its descendants."
                                },
                                "reason": {
                                    "type": "string",
                                    "description": "A short diagnostic reason. The application generates the user-facing approval text itself."
                                }
                            },
                            "required": ["path", "scope", "reason"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["write"],
                "additionalProperties": false
            }
        },
        "required": ["command"]
    })
}

fn shell_handler(args: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ShellParams {
            command: String,
            timeout: Option<u64>,
            escalated: Option<bool>,
            justification: Option<String>,
            #[serde(
                default,
                rename = "additional_permissions",
                alias = "additionalPermissions"
            )]
            additional_permissions: Option<crate::sandbox::windows_request::AdditionalPermissions>,
        }
        let params: ShellParams = serde_json::from_value(args)?;
        let approved_capability = if let Some(permissions) = params.additional_permissions.as_ref()
        {
            let sandbox = TOOL_SCOPE
                .try_with(|scope| scope.sandbox.clone())
                .unwrap_or_default();
            let prepared =
                crate::sandbox::windows_request::prepare(&sandbox, &params.command, permissions)?;
            if prepared.needs_approval() {
                let receipt = consume_windows_capability(&prepared).ok_or_else(|| {
                    anyhow!("additional write permission is missing an exact approval receipt")
                })?;
                Some(receipt)
            } else {
                None
            }
        } else {
            None
        };
        run_shell_with_capability(
            &params.command,
            params.timeout.unwrap_or(120),
            params.escalated.unwrap_or(false),
            params.justification.as_deref().unwrap_or(""),
            approved_capability.as_ref(),
        )
        .await
    })
}

pub fn shell_tool() -> AgentTool {
    // The description tells the model which shell actually interprets the
    // command on this platform — it is the model's only reliable signal for
    // generating syntax that will parse (see sandbox::shell_invocation).
    #[cfg(not(target_os = "windows"))]
    let description = "Execute a shell command in the current working directory. Commands are interpreted by bash. Use this for exploration and command-line programs. For ordinary file creation or edits, prefer write/edit tools, but shell redirection and heredocs may be used when they are the better fit. Returns stdout and stderr merged. Output is truncated to last 500000 bytes.";
    // Version-neutral on Windows: the precise interpreter (pwsh 7 vs Windows
    // PowerShell 5.1) and its chaining rules live in the host-platform section
    // of the system prompt (prompt::os_hint), resolved at runtime.
    #[cfg(target_os = "windows")]
    let description = "Execute a shell command in the current working directory. Commands are interpreted by PowerShell — use PowerShell syntax: environment variables as $env:VAR (never %VAR%), single quotes for literal strings, and see the host-platform note for command chaining. To run an executable whose path contains spaces, use the call operator: & \"C:\\Program Files\\app\\tool.exe\" args. Use this for exploration and command-line programs. For ordinary file creation or edits, prefer write/edit tools. Returns stdout and stderr merged. Output is truncated to last 500000 bytes.";

    #[cfg(not(target_os = "windows"))]
    let guidelines = vec![
        "Prefer one shell command per turn",
        "Prefer write/edit for ordinary file writes; use shell redirection, heredocs, tee, or cat > file only when they are more appropriate for the task.",
    ];
    #[cfg(target_os = "windows")]
    let guidelines = vec![
        "Prefer one shell command per turn",
        "Prefer write/edit for ordinary file writes; use PowerShell redirection (> or Out-File) only when it is more appropriate for the task. Note: on Windows PowerShell 5.1 these default to UTF-16 with a BOM — pass -Encoding utf8 if another tool must read the file.",
    ];

    make_tool(
        "shell",
        description,
        shell_schema(),
        shell_handler,
        guidelines,
    )
}

// ─── Read Tool ─────────────────────────────────────────────────────────────

fn read_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Path to the file to read"
            },
            "offset": {
                "type": "integer",
                "description": "Line number to start reading from (1-indexed)"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of lines to read"
            }
        }
    })
}

fn read_handler(args: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ReadParams {
            path: String,
            offset: Option<usize>,
            limit: Option<usize>,
        }
        let params: ReadParams = serde_json::from_value(args)?;
        run_read(&params.path, params.offset, params.limit).await
    })
}

pub fn read_tool() -> AgentTool {
    make_tool(
        "read",
        "Read a file from the filesystem.",
        read_schema(),
        read_handler,
        vec![],
    )
}

// ─── Write Tool ────────────────────────────────────────────────────────────

fn write_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "content": { "type": "string" }
        },
        "required": ["path", "content"]
    })
}

fn write_handler(args: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct WriteParams {
            path: String,
            content: String,
        }
        let params: WriteParams = serde_json::from_value(args)?;
        let path = run_write(&params.path, &params.content).await?;
        Ok(format!("Written to {}", path.display()))
    })
}

pub fn write_tool() -> AgentTool {
    make_tool(
        "write",
        "Write content to a file, creating or overwriting. Prefer this for ordinary user-requested file saves.",
        write_schema(),
        write_handler,
        vec![
            "When asked to create, save, or overwrite a normal file, prefer this write tool.",
            "Emit the `path` field first, before `content`, so the target file is known while the content streams.",
        ],
    )
}

// ─── Edit Tool ─────────────────────────────────────────────────────────────

fn edit_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "oldText": { "type": "string" },
            "newText": { "type": "string" },
            "edits": {
                "type": "array",
                "description": "Array of {oldText, newText} for multi-edit mode",
                "items": {
                    "type": "object",
                    "properties": {
                        "oldText": { "type": "string" },
                        "newText": { "type": "string" }
                    },
                    "required": ["oldText", "newText"]
                }
            }
        },
        "required": ["path"]
    })
}

fn edit_handler(args: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct EditParams {
            path: String,
            #[serde(alias = "old_text", alias = "old_string")]
            old_text: Option<String>,
            #[serde(alias = "new_text", alias = "new_string")]
            new_text: Option<String>,
            edits: Option<Vec<EditOp>>,
        }
        let params: EditParams = serde_json::from_value(args)?;
        let old_text = params.old_text;
        let new_text = params.new_text;
        let edits: Option<Vec<EditOp>> = params.edits.map(|es| {
            es.into_iter()
                .map(|e| EditOp {
                    old_text: e.old_text,
                    new_text: e.new_text,
                })
                .collect()
        });
        run_edit(
            &params.path,
            old_text.as_deref(),
            new_text.as_deref(),
            edits.as_deref(),
        )
        .await?;
        Ok(format!("Edited {}", params.path))
    })
}

pub fn edit_tool() -> AgentTool {
    make_tool(
        "edit",
        "Edit a file using exact text replacement. Supports multi-edit via edits array.",
        edit_schema(),
        edit_handler,
        vec![
            "Include enough context for unique matching",
            "Emit the `path` field first, before `oldText`/`newText`/`edits`, so the target file is known while the edit streams.",
        ],
    )
}

// ─── Tool sets ─────────────────────────────────────────────────────────────

/// Core coding tools (default set): read, write, edit, shell
pub fn coding_tools() -> Vec<AgentTool> {
    vec![read_tool(), write_tool(), edit_tool(), shell_tool()]
}

/// All built-in tools
pub fn all_tools() -> Vec<AgentTool> {
    vec![read_tool(), write_tool(), edit_tool(), shell_tool()]
}

// ─── Tool runners (async, using tokio) ─────────────────────────────────────

/// SIGKILL an entire process group by its group-leader PID. Used to tear down a
/// shell command's full process tree on abort/timeout, since `kill_on_drop` only
/// reaps the direct child and leaves grandchildren (e.g. `sleep`) orphaned.
#[cfg(unix)]
fn kill_process_group(pgid: Option<i32>) {
    // SAFETY: killpg is async-signal-safe and we target the group led by our
    // own just-spawned child. A stale/reaped pgid yields a harmless ESRCH.
    // (map, not if-let: a lone if-let closing brace here collected a phantom
    // zero-count coverage region.)
    let _ = pgid.map(|pgid| unsafe { libc::killpg(pgid, libc::SIGKILL) });
}

/// Polls the interrupt flag every 50ms. Returns when the flag is set to true.
/// Used by tokio::select! to cooperatively cancel long-running operations.
async fn wait_for_interrupt(flag: Arc<AtomicBool>) {
    loop {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Reject shell commands that match known-dangerous patterns.  This is a
/// defense-in-depth layer — the primary enforcement lives in the OS sandbox
/// and approval rules.  We catch the most obvious destructive patterns here
/// so they fail fast with a clear error instead of relying solely on the
/// sandbox to block them.
fn reject_dangerous_command(command: &str) -> Result<()> {
    let lower = command.to_lowercase();

    // Recursive removal (rm -r in any flag order, rmdir) targeting home or a
    // protected system root. Checked per command in a chain, so neither
    // "x && rm -rf ~" nor "sudo rm  -rf ~" can dodge the match, while quoted
    // text like `echo "rm -rf ~"` does not false-positive.
    for segment in shell_segments(&lower) {
        let tokens: Vec<&str> = segment.iter().map(String::as_str).collect();
        // Privilege wrappers don't change the target check.
        let tokens = match tokens.first() {
            Some(&"sudo") | Some(&"doas") => &tokens[1..],
            _ => &tokens[..],
        };
        if tokens.is_empty() {
            continue;
        }
        let cmd_name = tokens[0].rsplit('/').next().unwrap_or(tokens[0]);
        let recursive_rm = cmd_name == "rm"
            && tokens[1..]
                .iter()
                .take_while(|t| t.starts_with('-'))
                .any(|f| {
                    if let Some(long) = f.strip_prefix("--") {
                        long == "recursive"
                    } else {
                        // Flag cluster like -r, -rf, -fr, -rfv (already lowercased).
                        f[1..].contains('r')
                    }
                });
        if recursive_rm || cmd_name == "rmdir" {
            for target in tokens[1..].iter().skip_while(|t| t.starts_with('-')) {
                if is_protected_rm_target(target) {
                    return Err(anyhow!(
                        "Shell command rejected: destructive file removal targeting \
                         a system or home directory ('{command}'). Use targeted \
                         rm on specific project files instead."
                    ));
                }
            }
        }
    }

    // Fork-bomb / resource exhaustion patterns.
    let normalized: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.contains(":(){ :|:& };:")
        || normalized.contains("fork bomb")
        || (normalized.contains("while true") && normalized.contains("dd if="))
    {
        return Err(anyhow!(
            "Shell command rejected: pattern matches a known fork-bomb or \
             resource-exhaustion attack."
        ));
    }

    Ok(())
}

/// System roots that must never be recursively removed. Deeper absolute paths
/// (e.g. /tmp/build, /Users/alice/project/target) are allowed — the sandbox is
/// the primary boundary; this layer only fails fast on catastrophic targets.
const PROTECTED_RM_ROOTS: &[&str] = &[
    "/",
    "/bin",
    "/sbin",
    "/usr",
    "/etc",
    "/var",
    "/boot",
    "/proc",
    "/sys",
    "/dev",
    "/system",
    "/library",
    "/applications",
    "/users",
    "/home",
    "/root",
    "/private",
    "/private/etc",
    "/private/var",
];

/// True if a recursive-removal target points at the user's home or a
/// protected system root. `.`/`..` are resolved lexically so "/tmp/.." can't
/// dodge the root check.
fn is_protected_rm_target(target: &str) -> bool {
    let t = target.trim().trim_end_matches('/');
    let t = if t.is_empty() { "/" } else { t };

    // Home references in any shell spelling.
    if t == "~"
        || t.starts_with("~/")
        || t == "$home"
        || t.starts_with("$home/")
        || t.starts_with("${home}")
    {
        return true;
    }

    // Lexically resolve . and .. segments.
    let mut parts: Vec<&str> = Vec::new();
    for seg in t.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        // Resolves to a filesystem root or to all-dots relative path.
        return t.starts_with('/') || t.contains("..");
    }
    if t.starts_with('/') {
        let normalized = format!("/{}", parts.join("/"));
        if PROTECTED_RM_ROOTS.contains(&normalized.as_str()) {
            return true;
        }
        // Glob straight off the root: rm -rf /*
        if parts == ["*"] {
            return true;
        }
    }
    false
}

/// Minimal shell tokenizer: splits a command line into pipeline/chain
/// segments of tokens, honoring single/double quotes (quotes are stripped
/// from tokens; separators inside quotes are ignored). Not a full shell
/// parser — just enough that `echo "rm -rf ~"` doesn't false-positive while
/// `x && rm -rf ~` is still caught.
fn shell_segments(command: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut tok = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !tok.is_empty() {
                    current.push(std::mem::take(&mut tok));
                }
            }
            '&' | ';' | '|' if !in_single && !in_double => {
                if !tok.is_empty() {
                    current.push(std::mem::take(&mut tok));
                }
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                // Swallow the second char of && / ||.
                if chars.peek() == Some(&c) {
                    chars.next();
                }
            }
            _ => tok.push(c),
        }
    }
    if !tok.is_empty() {
        current.push(tok);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// Pre-execution escalation: the model explicitly asked for unsandboxed
/// execution. Returns the decision-driven outcome to propagate, or None when
/// no escalation channel is registered (caller falls through to a normal
/// sandboxed run).
async fn pre_execution_escalation(
    escalation: &Option<crate::sandbox::EscalationRequester>,
    command: &str,
    timeout_secs: u64,
    justification: &str,
    sandbox: &ResolvedSandbox,
) -> Option<Result<String>> {
    let requester = escalation.as_ref()?;
    let request = EscalationRequest {
        trigger: crate::sandbox::EscalationTrigger::ModelRequest,
        command: command.to_string(),
        justification: justification.to_string(),
        failure_summary: String::new(),
    };
    Some(match requester(&request) {
        EscalationDecision::Approved => {
            spawn_shell(command, timeout_secs, sandbox, true, None).await
        }
        EscalationDecision::Denied(note) => Err(anyhow!(
            "Escalated execution was not approved{}. Run the command inside the sandbox instead, or explain to the user why it needs these permissions.",
            if note.is_empty() { String::new() } else { format!(": {note}") }
        )),
    })
}

/// Post-hoc escalation: the sandboxed run failed with a sandbox-denial
/// signature. Returns the outcome to propagate (approved → unsandboxed
/// re-run; denied → annotated original output), or None when the failure
/// doesn't look like a sandbox denial / no escalation channel exists.
async fn post_hoc_escalation(
    escalation: &Option<crate::sandbox::EscalationRequester>,
    sandbox: &ResolvedSandbox,
    command: &str,
    timeout_secs: u64,
    result: &str,
) -> Option<Result<String>> {
    let requester = escalation.as_ref()?;
    let (exit_code, tail) = parse_result_failure(result);
    if exit_code == 0 || !crate::sandbox::looks_like_sandbox_denial(sandbox, exit_code, &tail) {
        return None;
    }
    let request = EscalationRequest {
        trigger: crate::sandbox::EscalationTrigger::SandboxFailure,
        command: command.to_string(),
        justification: String::new(),
        failure_summary: tail,
    };
    Some(match requester(&request) {
        EscalationDecision::Approved => {
            spawn_shell(command, timeout_secs, sandbox, true, None).await
        }
        EscalationDecision::Denied(note) => Ok(format!(
            "{result}\n[sandbox] The command appears to have been blocked by the sandbox; running it without the sandbox was not approved{}.",
            if note.is_empty() { String::new() } else { format!(": {note}") }
        )),
    })
}

#[cfg(test)]
async fn run_shell(
    command: &str,
    timeout_secs: u64,
    escalated: bool,
    justification: &str,
) -> Result<String> {
    run_shell_with_capability(command, timeout_secs, escalated, justification, None).await
}

async fn run_shell_with_capability(
    command: &str,
    timeout_secs: u64,
    escalated: bool,
    justification: &str,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
) -> Result<String> {
    // Defense-in-depth: reject obviously destructive commands before they
    // reach the OS.  The sandbox provides the primary enforcement boundary;
    // this is a loud, fast-fail layer that catches the most egregious patterns.
    reject_dangerous_command(command)?;

    // On Windows, cmd.exe strips double quotes when processing arguments to
    // npm-generated .cmd wrappers (like the `future` CLI). This corrupts
    // --args JSON that contains commas in string values. Rewrite such
    // commands to pipe JSON through --stdin via a temp file.
    let command_owned =
        cmd_exe_rewrite::rewrite_future_tools_args(command).unwrap_or_else(|| command.to_string());
    let command: &str = &command_owned;

    let sandbox = TOOL_SCOPE
        .try_with(|scope| scope.sandbox.clone())
        .unwrap_or_default();
    let escalation = TOOL_SCOPE
        .try_with(|scope| scope.escalation.clone())
        .unwrap_or(None);

    // Model explicitly requested escalated permissions: approve BEFORE running.
    // Only honored when the command would actually run sandboxed — in degraded
    // or full-access modes the pre-execution approval flow already covered it,
    // and escalating would double-prompt the user.
    if escalated && sandbox.wraps_shell() {
        // No escalation channel: fall through to a normal sandboxed run.
        #[allow(clippy::single_match)]
        // match keeps each edge's region on its arm line; an if-let whose body always diverges leaves a phantom zero-count region on its closing brace
        match pre_execution_escalation(&escalation, command, timeout_secs, justification, &sandbox)
            .await
        {
            Some(outcome) => return outcome,
            None => {}
        }
    }

    let sandboxed = sandbox.wraps_shell();
    if sandboxed {
        if let Ok(Some(notify)) = TOOL_SCOPE.try_with(|scope| scope.on_sandboxed.clone()) {
            notify(command);
        }
    }
    let mut allow_post_hoc = true;
    let result = spawn_shell_with_report(
        command,
        timeout_secs,
        &sandbox,
        false,
        approved_capability,
        &mut allow_post_hoc,
    )
    .await?;

    // Post-hoc escalation: only when the failure narrowly looks like a sandbox
    // denial (conservative heuristic — ordinary failures go back to the model).
    if sandboxed && allow_post_hoc {
        #[allow(clippy::single_match)]
        // match keeps each edge's region on its arm line; an if-let whose body always diverges leaves a phantom zero-count region on its closing brace
        match post_hoc_escalation(&escalation, &sandbox, command, timeout_secs, &result).await {
            Some(outcome) => return outcome,
            None => {}
        }
    }

    Ok(result)
}

/// The exit code carried by a shell tool result's `[exit: N]` footer, if any.
/// `None` for non-shell results and for `[exit: signal]` (killed by a signal —
/// no numeric code).
pub fn shell_result_exit_code(result: &str) -> Option<i32> {
    result.lines().rev().find_map(|line| {
        line.strip_prefix("[exit: ")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|code| code.parse::<i32>().ok())
    })
}

/// A bare grep/diff/cmp/test command exiting 1 is a normal "no match / differs
/// / false" signal, not an error. Any shell operator makes the exit code
/// ambiguous (pipeline/list), so those stay failures. `findstr` is the Windows
/// grep (the shell tool runs via PowerShell there); `find` is deliberately
/// absent — it means different things on Windows vs Unix.
pub fn is_soft_fail_command(command: &str) -> bool {
    if command.contains(['|', '&', ';', '\n', '`', '<', '>']) || command.contains("$(") {
        return false;
    }
    let Some(first) = command.split_whitespace().next() else {
        return false;
    };
    // Basename of the program, tolerant of Windows paths (`\`), a `.exe`
    // suffix, and case (Windows resolves names case-insensitively).
    let base = first
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first)
        .to_ascii_lowercase();
    let program = base.strip_suffix(".exe").unwrap_or(base.as_str());
    matches!(
        program,
        "grep" | "egrep" | "fgrep" | "rg" | "findstr" | "diff" | "cmp" | "test" | "["
    )
}

/// Structured `tool_end` semantics for a tool result, so consumers (GUI Runs
/// panel, artifact persistence, other clients) stop re-parsing the output
/// prose. Empty object when the tool has nothing structured to report:
///
/// - `shell`: `exit_code` from the result's `[exit: N]` footer, plus
///   `is_soft_fail` when exit 1 is the command's normal no-match signal (see
///   [`is_soft_fail_command`]); exit 2+ from those programs is a real error.
/// - `write` / `edit`: `target_path` from the call arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolEndSemantics {
    pub exit_code: Option<i32>,
    pub is_soft_fail: Option<bool>,
    pub target_path: Option<String>,
}

pub fn tool_end_semantics(
    tool_name: &str,
    tool_args: &serde_json::Value,
    result: &str,
) -> ToolEndSemantics {
    let mut semantics = ToolEndSemantics::default();
    match tool_name {
        "shell" => {
            if let Some(code) = shell_result_exit_code(result) {
                semantics.exit_code = Some(code);
                let args = tool_args_object(tool_args);
                let command = args
                    .as_ref()
                    .and_then(|args| args.get("command"))
                    .and_then(|command| command.as_str())
                    .unwrap_or_default();
                if code == 1 && is_soft_fail_command(command) {
                    semantics.is_soft_fail = Some(true);
                }
            }
        }
        "write" | "edit" => {
            let args = tool_args_object(tool_args);
            if let Some(path) = args
                .as_ref()
                .and_then(|args| args.get("path"))
                .and_then(|path| path.as_str())
                .map(str::trim)
                .filter(|path| !path.is_empty())
            {
                semantics.target_path = Some(path.to_string());
            }
        }
        _ => {}
    }
    semantics
}

/// Tool-call arguments as an object: the wire shape is either a JSON object or
/// a JSON-encoded string of one (the agent serializes args to a string field).
fn tool_args_object(tool_args: &serde_json::Value) -> Option<serde_json::Value> {
    match tool_args {
        serde_json::Value::String(s) => serde_json::from_str(s).ok(),
        other => Some(other.clone()),
    }
}

/// Extract the exit code and output tail from a formatted run_shell result, for
/// the sandbox-denial heuristic. Exit code is now at the end as "[exit: N]".
fn parse_result_failure(result: &str) -> (i32, String) {
    let exit_code = shell_result_exit_code(result).unwrap_or(0);
    let tail_start = result.len().saturating_sub(2000);
    let tail = result.get(tail_start..).unwrap_or(result).to_string();
    (exit_code, tail)
}

/// Spawn a shell command (sandbox-wrapped unless `escalated`) and wait for it
/// with timeout + interrupt handling. Returns the formatted combined output.
#[cfg(windows)]
async fn spawn_windows_restricted_shell(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    cwd: &Path,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
) -> Result<String> {
    use tokio::io::AsyncReadExt;

    let mut env_overrides = vec![(
        std::ffi::OsString::from("PWD"),
        cwd.as_os_str().to_os_string(),
    )];
    if let Some(path) = path_with_own_dir(std::env::current_exe()) {
        env_overrides.push((std::ffi::OsString::from("PATH"), path.into()));
    }
    let mut child = crate::sandbox::windows::runner::spawn(
        sandbox,
        command,
        cwd,
        &env_overrides,
        approved_capability,
    )
    .map_err(|error| anyhow!("Failed to initialize Windows write protection: {error}"))?;
    let mut stdout = tokio::fs::File::from_std(
        child
            .take_stdout()
            .ok_or_else(|| anyhow!("Failed to capture restricted stdout"))?,
    );
    let mut stderr = tokio::fs::File::from_std(
        child
            .take_stderr()
            .ok_or_else(|| anyhow!("Failed to capture restricted stderr"))?,
    );
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let interrupt_flag = TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.clone())
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
    let timeout = std::time::Duration::from_secs(timeout_secs.max(1));

    enum Completion {
        Exit(std::io::Result<u32>),
        Timeout,
        Interrupted,
    }
    let completion = tokio::select! {
        result = tokio::time::timeout(timeout, child.wait()) => match result {
            Ok(exit) => Completion::Exit(exit),
            Err(_) => Completion::Timeout,
        },
        _ = wait_for_interrupt(interrupt_flag) => Completion::Interrupted,
    };
    if matches!(&completion, Completion::Timeout | Completion::Interrupted) {
        child.terminate();
        let _ = child.wait().await;
    }
    let stdout = stdout_task
        .await
        .map_err(|error| anyhow!("restricted stdout task failed: {error}"))??;
    let stderr = stderr_task
        .await
        .map_err(|error| anyhow!("restricted stderr task failed: {error}"))??;
    let mut combined = stdout;
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with(b"\n") {
            combined.push(b'\n');
        }
        combined.extend_from_slice(&stderr);
    }
    let combined = crate::sandbox::decode_restricted_shell_output(&combined);

    match completion {
        Completion::Exit(exit) => {
            let exit = exit.map_err(|error| anyhow!("Restricted shell wait failed: {error}"))?;
            Ok(format_shell_output(&combined, combined.len(), exit as i32))
        }
        Completion::Timeout if combined.is_empty() => Err(anyhow!(
            "Shell command timed out after {} seconds (no output captured)",
            timeout_secs.max(1)
        )),
        Completion::Timeout => Err(anyhow!(
            "Shell command timed out after {} seconds.\nPartial output ({} total):\n{}",
            timeout_secs.max(1),
            human_size(combined.len()),
            format_shell_output(&combined, combined.len(), -1),
        )),
        Completion::Interrupted => Err(anyhow!("Shell command interrupted by abort")),
    }
}

async fn spawn_shell(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    escalated: bool,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
) -> Result<String> {
    spawn_shell_with_report(
        command,
        timeout_secs,
        sandbox,
        escalated,
        approved_capability,
        &mut true,
    )
    .await
}

async fn spawn_shell_with_report(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    escalated: bool,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
    allow_post_hoc: &mut bool,
) -> Result<String> {
    let cwd = active_workspace()?;
    #[cfg(windows)]
    if !escalated && sandbox.wraps_shell() {
        return spawn_windows_restricted_shell(
            command,
            timeout_secs,
            sandbox,
            &cwd,
            approved_capability,
        )
        .await;
    }
    #[cfg(not(windows))]
    let _ = approved_capability;
    // Unix: wrap in a subshell to merge stderr into stdout, preserving the
    // original interleaving order that separate pipes lose. Internal
    // redirections in the user's command are respected inside the subshell;
    // only the subshell's own stderr (empty after the merge) goes to /dev/null.
    #[cfg(not(windows))]
    let merged_cmd = format!("( {} ) 2>&1", command);
    // Windows: `( … ) 2>&1` is a bash-ism — PowerShell's `( … )` rejects
    // multi-statement commands. The PowerShell wrapper built by
    // `sandbox::shell_invocation` does the stderr merge and exit-code capture
    // itself, so the command passes through unmodified.
    #[cfg(windows)]
    let merged_cmd = command.to_string();
    // Preparation can scan a large workspace before a child exists. Let Abort
    // cancel that scan as well as the process execution below.
    let interrupt_flag = TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.clone())
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
    // Keep directory I/O off the async executor so it can process the Abort
    // request even on a single-worker runtime. The worker only prepares a
    // request: dropping this future can never launch the user command later.
    #[cfg(target_os = "linux")]
    let preparation = {
        let sandbox = sandbox.clone();
        let cwd = cwd.clone();
        let cancelled = interrupt_flag.clone();
        tokio::task::spawn_blocking(move || {
            sandbox.prepare_shell_for_cwd_with_cancel(&merged_cmd, escalated, &cwd, &|| {
                cancelled.load(Ordering::Relaxed)
            })
        })
        .await
        .map_err(|error| anyhow!("Failed to initialize OS sandbox worker: {error}"))?
    };
    #[cfg(not(target_os = "linux"))]
    let preparation = sandbox.prepare_shell_for_cwd(&merged_cmd, escalated, &cwd);
    let prepared =
        preparation.map_err(|error| anyhow!("Failed to initialize OS sandbox: {error}"))?;
    if interrupt_flag.load(Ordering::Relaxed) {
        return Err(anyhow!(
            "Shell command interrupted by abort before execution"
        ));
    }
    let report_digest = prepared.boundary.policy_digest.clone();
    let expects_report =
        prepared.boundary.backend == crate::sandbox::backend::ShellBackend::LinuxBubblewrap;
    if expects_report {
        *allow_post_hoc = false;
    }
    let (mut child, mut report_reader) = prepared
        .into_command_with_report()
        .map_err(|error| anyhow!("Failed to initialize OS sandbox request transport: {error}"))?;
    #[cfg(windows)]
    let _ = (&report_digest, &mut report_reader);
    child.current_dir(&cwd).env("PWD", &cwd);
    // Prepend the agent binary's directory to PATH so bundled tools in the
    // same directory are discoverable by shell commands. (map + discard: a
    // lone if-let closing brace here collected a phantom zero-count region.)
    let _ = path_with_own_dir(std::env::current_exe()).map(|path| child.env("PATH", path));
    child.stdout(std::process::Stdio::piped());
    // Plain Unix shells merge in the subshell. Linux sandbox helpers instead
    // dup stderr to stdout before exec (PreparedShell::into_command), preserving
    // initialization errors emitted before that subshell even exists.
    // Windows: PowerShell's own failures (a parse error in the
    // -Command string never executes the 2>&1 merge) surface only on the
    // process's stderr — capture it so those errors aren't silently dropped.
    #[cfg(not(windows))]
    child.stderr(std::process::Stdio::null());
    #[cfg(windows)]
    child.stderr(std::process::Stdio::piped());
    child.kill_on_drop(true);
    // Run the shell as the leader of its own process group so abort/timeout can kill
    // the whole tree. kill_on_drop alone only SIGKILLs the shell itself, leaving
    // grandchildren (e.g. a `sleep` spawned by the command) running as orphans.
    // sandbox-exec execs its child, so the group covers the wrapped tree too.
    #[cfg(unix)]
    child.process_group(0);

    let mut spawned = child
        .spawn()
        .map_err(|e| anyhow!("Failed to run shell command: {}", e))?;
    #[cfg(unix)]
    let pgid = spawned.id().map(|id| id as i32);
    #[cfg(windows)]
    let job = {
        let job = crate::sandbox::windows::Job::create().ok();
        if let (Some(job), Some(pid)) = (&job, spawned.id()) {
            let _ = job.assign(pid);
        }
        job
    };

    // Windows: drain stderr concurrently so a PowerShell parse error can't
    // deadlock the pipe, and its text can be appended to the output below.
    #[cfg(windows)]
    let stderr_task = spawned.stderr.take().map(|mut err| {
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf).await;
            buf
        })
    });

    // Read stdout incrementally — on timeout we keep whatever was captured.
    let mut stdout = spawned
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stdout"))?;
    let mut output_buf = Vec::new();
    let mut read_buf = [0u8; 8192];
    let timeout_dur = std::time::Duration::from_secs(timeout_secs.max(1));

    // On Windows the CLI terminates itself via TerminateProcess/process.exit.
    // PowerShell can keep waiting for the browser descendant, so waiting for
    // the shell process would hang forever. The wrapper already merges stderr
    // into stdout; EOF therefore means the CLI result is complete.
    #[cfg(windows)]
    {
        let result = tokio::select! {
            result = tokio::time::timeout(timeout_dur, async {
                use tokio::io::AsyncReadExt;
                loop {
                    match stdout.read(&mut read_buf).await {
                        Ok(0) => break,
                        Ok(n) => output_buf.extend_from_slice(&read_buf[..n]),
                        Err(e) => return Err(anyhow!("Failed to read shell output: {}", e)),
                    }
                }
                Ok(())
            }) => result,
            _ = wait_for_interrupt(interrupt_flag.clone()) => {
                if let Some(job) = &job {
                    job.terminate();
                }
                return Err(anyhow!("Shell command interrupted by abort"));
            }
        };

        if let Some(job) = &job {
            job.disarm();
        }
        // PowerShell may keep stderr open while waiting for the browser too.
        // Do not await that drain after stdout has provided the completion
        // signal, or it would recreate the same hang.
        drop(stderr_task);

        match result {
            Ok(Ok(())) => {
                let combined = String::from_utf8_lossy(&output_buf);
                Ok(format_shell_output(&combined, combined.len(), 0))
            }
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => {
                let combined = String::from_utf8_lossy(&output_buf);
                let combined = if expects_report {
                    std::borrow::Cow::Owned(crate::sandbox::linux::report::untrusted_output(
                        &combined,
                    ))
                } else {
                    combined
                };
                let total = combined.len();
                if total == 0 {
                    Err(anyhow!(
                        "Shell command timed out after {} seconds (no output captured)",
                        timeout_secs.max(1)
                    ))
                } else {
                    spawned.kill().await.ok();
                    Ok(format_shell_output(&combined, total, 0))
                }
            }
        }
    }

    #[cfg(not(windows))]
    let read_result = tokio::select! {
        result = tokio::time::timeout(timeout_dur, async {
            read_shell_output(&mut stdout, &mut output_buf, &mut read_buf).await?;
            // Channel a wait() failure through the same error edge as read
            // failures so the match below needs no OS-failure-only arm.
            spawned
                .wait()
                .await
                .map_err(|e| anyhow!("Failed to run shell command: {e}"))
        }) => result,
        _ = wait_for_interrupt(interrupt_flag.clone()) => {
            kill_process_group(pgid);
            return Err(anyhow!("Shell command interrupted by abort"));
        }
    };

    #[cfg(not(windows))]
    {
        // `outcome?` keeps the (injection-proof) read/wait error edge on the
        // same line as the success pattern — no unreachable match arm.
        let status = match read_result {
            Ok(outcome) => outcome?,
            Err(_elapsed) => {
                // Timeout — kill process tree, drain remaining pipe content.
                kill_process_group(pgid);
                // Drain whatever the process wrote before the kill took effect.
                drain_shell_output(&mut stdout, &mut output_buf, &mut read_buf).await;
                let combined = String::from_utf8_lossy(&output_buf);
                let combined = if expects_report {
                    std::borrow::Cow::Owned(crate::sandbox::linux::report::untrusted_output(
                        &combined,
                    ))
                } else {
                    combined
                };
                let total = combined.len();
                if total == 0 {
                    return Err(anyhow!(
                        "Shell command timed out after {} seconds (no output captured)",
                        timeout_secs.max(1)
                    ));
                }
                let formatted = format_shell_output(&combined, total, -1);
                return Err(anyhow!(
                    "Shell command timed out after {} seconds.\nPartial output ({} total):\n{}",
                    timeout_secs.max(1),
                    human_size(total),
                    formatted,
                ));
            }
        };
        // Normal completion. On unix a successful command never kills the
        // process group, so intentionally detached grandchildren survive.
        // Drain leftover bytes (rare: process exited but pipe still has data).
        drain_shell_output(&mut stdout, &mut output_buf, &mut read_buf).await;
        let combined = String::from_utf8_lossy(&output_buf);
        let exit_code = status.code().unwrap_or(-1);
        if expects_report {
            // Never parse command-printed markers as helper evidence. Only the
            // per-spawn anonymous channel may suppress post-hoc escalation.
            let mut output = crate::sandbox::linux::report::untrusted_output(&combined);
            let report = report_reader
                .as_mut()
                .zip(report_digest.as_deref())
                .ok_or_else(|| anyhow!("missing helper report channel"))
                .and_then(|(file, digest)| {
                    crate::sandbox::linux::report::HelperReport::read(file, digest)
                });
            match report {
                Ok(report) => {
                    *allow_post_hoc = report.events.is_empty();
                    // Format/truncate command text before appending verified
                    // reports so they cannot be lost to command-output volume.
                    output = format_shell_output(&output, output.len(), exit_code);
                    for event in &report.events {
                        output.push('\n');
                        output.push_str(&crate::sandbox::linux::violation::marker(event));
                    }
                }
                Err(_) => {
                    output = format_shell_output(&output, output.len(), exit_code);
                    output.push_str("\n[sandbox] Helper report unavailable or invalid. Detection results are unknown; automatic unsandboxed retry is disabled. The command's exit status is unchanged.");
                }
            }
            return Ok(output);
        }
        Ok(format_shell_output(&combined, combined.len(), exit_code))
    }
}

/// Prepend the agent binary's own directory to the inherited PATH, so tools
/// bundled next to the binary are discoverable by shell commands. Returns
/// None when the binary's location can't be determined (no PATH prepend).
fn path_with_own_dir(exe: std::io::Result<std::path::PathBuf>) -> Option<String> {
    let exe = exe.ok()?;
    let dir = exe.parent()?;
    let existing = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    Some(format!("{}{}{}", dir.display(), sep, existing))
}

/// Read the child's stdout into `buf` until EOF; a read error aborts the run.
/// Extracted from spawn_shell so the error arm is directly testable with a
/// failing reader (a real pipe read failure has no reliable injection point).
#[cfg(not(windows))]
async fn read_shell_output<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    chunk: &mut [u8],
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    loop {
        match reader.read(chunk).await {
            Ok(0) => return Ok(()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(anyhow!("Failed to read shell output: {}", e)),
        }
    }
}

/// Drain leftover pipe bytes after process exit/kill; EOF or a read error
/// (the kill racing the pipe) both end the drain silently.
#[cfg(not(windows))]
async fn drain_shell_output<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    chunk: &mut [u8],
) {
    use tokio::io::AsyncReadExt;
    loop {
        match reader.read(chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Drop the CLIXML noise Windows PowerShell serializes onto its stderr when it
/// is a redirected pipe (each block starts with a `#< CLIXML` marker line
/// followed by a `<Objs …>…</Objs>` XML payload). Line-based so it never eats
/// genuine error text. No-op when there is no CLIXML marker.
#[cfg(all(windows, test))]
fn strip_powershell_clixml(text: &str) -> String {
    if !text.contains("#< CLIXML") {
        return text.to_string();
    }
    text.lines()
        .filter(|line| {
            let t = line.trim_start();
            !(t.starts_with("#< CLIXML")
                || (t.starts_with("<Objs") && t.contains("schemas.microsoft.com/powershell")))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format shell output with truncation info and exit code footer.
/// Kept out of the hot path so the timeout branch can reuse it.
fn format_shell_output(raw: &str, total_bytes: usize, exit_code: i32) -> String {
    const MAX_KEEP: usize = 500_000;

    let body = if total_bytes > MAX_KEEP {
        let truncated = total_bytes - MAX_KEEP;
        // Keep the LAST MAX_KEEP bytes (most relevant output is at the end).
        let start = raw.ceil_char_boundary(raw.len() - MAX_KEEP);
        format!(
            "[output: {} total, showing last {}; {} truncated]\n{}",
            human_size(total_bytes),
            human_size(MAX_KEEP),
            human_size(truncated),
            &raw[start..],
        )
    } else {
        raw.to_string()
    };

    let footer = if exit_code >= 0 {
        format!("[exit: {}]", exit_code)
    } else {
        "[exit: signal]".to_string()
    };

    let result = format!("{}\n{}", body, footer);
    // The footer ("[exit: …]") is always non-empty, so trim_end can never
    // yield an empty string — the untrimmed fallback was dead by construction.
    result.trim_end().to_string()
}

fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

async fn run_read(path: &str, offset: Option<usize>, limit: Option<usize>) -> Result<String> {
    let path = workspace_path(path)?;
    let content = tokio::fs::read_to_string(path).await?;

    let offset = offset.unwrap_or(1).saturating_sub(1); // 1-indexed → 0-indexed
    let limit = limit.unwrap_or(usize::MAX);

    let lines: Vec<&str> = content.lines().skip(offset).take(limit).collect();
    let result = lines.join("\n");

    Ok(result)
}

async fn run_write(path: &str, content: &str) -> Result<PathBuf> {
    let path = workspace_path(path)?;
    let cwd = active_workspace()?;
    ensure_workspace_access(&cwd, &path)?;
    // parent() is always Some for a canonical workspace path; the eager
    // fallback keeps the None edge branchless (a lone if-let closing brace
    // collected a phantom zero-count coverage region here).
    tokio::fs::create_dir_all(path.parent().unwrap_or(&path))
        .await
        .ok();
    tokio::fs::write(&path, content).await?;
    Ok(path)
}

async fn run_edit(
    path: &str,
    old_text: Option<&str>,
    new_text: Option<&str>,
    edits: Option<&[EditOp]>,
) -> Result<()> {
    let path = workspace_path(path)?;
    let cwd = active_workspace()?;
    ensure_workspace_access(&cwd, &path)?;
    let current = tokio::fs::read_to_string(&path).await?;

    let final_content = if let Some(edits) = edits {
        // Multi-edit mode — all-or-nothing: if any edit fails to match,
        // the file is not modified and the error lists every failed edit.
        let mut result = current.clone();
        let mut failures: Vec<String> = Vec::new();
        for (i, edit) in edits.iter().enumerate() {
            if let Some(pos) = result.rfind(&edit.old_text) {
                result = format!(
                    "{}{}{}",
                    &result[..pos],
                    edit.new_text,
                    &result[pos + edit.old_text.len()..]
                );
            } else {
                failures.push(format!(
                    "edit {}: could not find \"{}\"",
                    i + 1,
                    truncate_for_error(&edit.old_text),
                ));
            }
        }
        if !failures.is_empty() {
            return Err(anyhow!(
                "Edit failed: {} of {} edit(s) could not be applied.\n{}",
                failures.len(),
                edits.len(),
                failures.join("\n"),
            ));
        }
        result
    } else if let (Some(old), Some(new)) = (old_text, new_text) {
        if let Some(pos) = current.find(old) {
            format!("{}{}{}", &current[..pos], new, &current[pos + old.len()..])
        } else {
            return Err(anyhow!(
                "Edit failed: could not find the text to replace in the file. \
                 The file may have changed since it was last read. Try reading \
                 the file again and re-applying the edit."
            ));
        }
    } else {
        return Err(anyhow!(
            "Edit failed: missing required parameters. Provide either \
             oldText + newText for a simple replacement, or an edits \
             array for structured changes."
        ));
    };

    tokio::fs::write(path, &final_content).await?;
    Ok(())
}

/// Truncate a string for error messages — keeps the first 80 chars so the
/// error is readable without dumping an entire file into the log.
fn truncate_for_error(s: &str) -> String {
    match s.char_indices().nth(80) {
        Some((idx, _)) => format!("{}…", &s[..idx]),
        None => s.to_string(),
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EditOp {
    #[serde(alias = "old_text", alias = "old_string")]
    old_text: String,
    #[serde(alias = "new_text", alias = "new_string")]
    new_text: String,
}

fn workspace_path(path: &str) -> Result<PathBuf> {
    let cwd = active_workspace()?;
    // `~` resolves to the real home directory (NOT the workspace — the legacy
    // behavior disagreed with what the OS sandbox enforces, see §3.5).
    let absolute_path = crate::sandbox::paths::resolve_against(&cwd, path);
    let normalized_path = crate::sandbox::paths::normalize_lexically(&absolute_path);
    Ok(normalized_path)
}

fn active_workspace() -> Result<PathBuf> {
    if let Ok(workspace) = TOOL_SCOPE.try_with(|scope| scope.workspace.clone()) {
        return Ok(workspace);
    }
    Ok(std::env::current_dir()?)
}

fn ensure_workspace_access(_workspace: &Path, path: &Path) -> Result<()> {
    if TOOL_SCOPE.try_with(|_| ()).is_err() {
        return Ok(());
    }

    // "all" permission: no workspace restrictions
    if TOOL_SCOPE
        .try_with(|scope| scope.permission_level.clone())
        .unwrap_or_default()
        == "all"
    {
        return Ok(());
    }

    // §3.5 normalization: symlinks resolve to their final target, `..` cannot
    // escape, comparison is case-insensitive on macOS.
    let candidate = crate::sandbox::paths::canonicalize_lenient(path);
    if is_approved_outside_path(&candidate) {
        return Ok(());
    }

    let sandbox = TOOL_SCOPE
        .try_with(|scope| scope.sandbox.clone())
        .unwrap_or_default();
    // Disabled (non-GUI) sessions run fully open; otherwise the write must
    // resolve to Allow (the before_tool_call hook already prompted for Ask and
    // recorded approved paths above).
    if !sandbox.enabled() {
        return Ok(());
    }
    match sandbox.evaluate(&candidate, crate::sandbox::rules::Op::Write) {
        crate::sandbox::rules::Decision::Allow => Ok(()),
        crate::sandbox::rules::Decision::Deny => Err(anyhow!(
            "Writing {} is denied by an approval rule.",
            candidate.display()
        )),
        crate::sandbox::rules::Decision::Ask => Err(anyhow!(
            "Path is outside the writable area and requires approval: {}",
            candidate.display()
        )),
    }
}

fn is_approved_outside_path(path: &Path) -> bool {
    TOOL_SCOPE
        .try_with(|scope| {
            scope
                .approved_outside_paths
                .lock()
                .iter()
                .any(|approved_path| crate::sandbox::paths::path_within(path, approved_path))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_binding_fixture() -> (
        crate::sandbox::windows_request::PreparedWritePermissions,
        Vec<crate::sandbox::windows_request::ApprovalTarget>,
        crate::sandbox::windows_request::ApprovedWriteCapability,
    ) {
        use crate::sandbox::windows_request::{
            ApprovalTarget, ApprovedWriteCapability, CapabilityApprovalSemantics,
            PreparedWritePermissions, WriteScope,
        };
        let targets = vec![ApprovalTarget {
            path: r"D:\release".to_owned(),
            scope: WriteScope::Subtree,
        }];
        let prepared = PreparedWritePermissions {
            command_hash: "expected-command".to_owned(),
            targets: vec![],
            approval: Some(CapabilityApprovalSemantics {
                behavior: "manage_files",
                targets: targets.clone(),
            }),
        };
        let receipt = ApprovedWriteCapability {
            request_id: "request-1".to_owned(),
            command_hash: prepared.command_hash.clone(),
            targets: targets.clone(),
        };
        (prepared, targets, receipt)
    }

    #[test]
    fn windows_capability_receipt_binds_exact_command_and_scope() {
        let (prepared, targets, receipt) = capability_binding_fixture();
        assert!(windows_capability_receipt_matches(
            &prepared, &targets, &receipt
        ));

        let mut wrong_command = receipt.clone();
        wrong_command.command_hash = "tampered-command".to_owned();
        assert!(!windows_capability_receipt_matches(
            &prepared,
            &targets,
            &wrong_command
        ));

        let mut wrong_scope = receipt;
        wrong_scope.targets[0].scope = crate::sandbox::windows_request::WriteScope::File;
        assert!(!windows_capability_receipt_matches(
            &prepared,
            &targets,
            &wrong_scope
        ));
    }

    #[test]
    fn windows_capability_receipt_binds_complete_ordered_target_set() {
        let (prepared, targets, mut receipt) = capability_binding_fixture();
        receipt
            .targets
            .push(crate::sandbox::windows_request::ApprovalTarget {
                path: r"D:\unexpected".to_owned(),
                scope: crate::sandbox::windows_request::WriteScope::Subtree,
            });
        assert!(!windows_capability_receipt_matches(
            &prepared, &targets, &receipt
        ));
    }

    #[tokio::test]
    async fn approve_and_consume_windows_capability_roundtrip() {
        let (prepared, _targets, receipt) = capability_binding_fixture();
        let sandbox = Arc::new(ResolvedSandbox::disabled("/tmp"));
        with_tool_scope(
            ScopeOptions {
                workspace: "/tmp".to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox,
                escalation: None,
                on_sandboxed: None,
            },
            async {
                // Nothing approved yet → consume returns None.
                assert!(consume_windows_capability(&prepared).is_none());
                // Approve, then consume returns the exact receipt and drains it.
                approve_windows_capability(receipt.clone());
                assert_eq!(consume_windows_capability(&prepared), Some(receipt.clone()));
                assert!(consume_windows_capability(&prepared).is_none());
                // A prepared permission with no approval is also None.
                let no_approval = crate::sandbox::windows_request::PreparedWritePermissions {
                    command_hash: "h".into(),
                    targets: vec![],
                    approval: None,
                };
                assert!(consume_windows_capability(&no_approval).is_none());
            },
        )
        .await;
    }

    #[tokio::test]
    async fn shell_handler_additional_permissions_without_approval() {
        // A target inside the workspace (disabled sandbox → Allow) needs no
        // approval: the handler runs the command with no capability receipt.
        let workspace = test_path("shell-additional-allow");
        std::fs::create_dir_all(&workspace).unwrap();
        let target = workspace.join("target.txt");
        std::fs::write(&target, "x").unwrap();
        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async {
                shell_handler(serde_json::json!({
                    "command": "echo ok",
                    "additional_permissions": {
                        "write": [{
                            "path": target.to_string_lossy(),
                            "scope": "file",
                            "reason": "test"
                        }]
                    }
                }))
                .await
            },
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("ok"));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn shell_handler_additional_permissions_require_approval_receipt() {
        // An enabled (Manual) sandbox marks a write outside the workspace and
        // outside /tmp as Ask → needs_approval. Without a receipt it fails fast.
        let workspace = test_path("shell-additional-ask");
        std::fs::create_dir_all(&workspace).unwrap();
        let outside = std::env::current_dir()
            .unwrap()
            .join(format!("__cov_outside_{}.txt", std::process::id()));
        std::fs::write(&outside, "x").unwrap();
        let sandbox = Arc::new(ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Manual,
            },
            &workspace.to_string_lossy(),
        ));
        let result = with_tool_scope(
            ScopeOptions {
                workspace: workspace.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox,
                escalation: None,
                on_sandboxed: None,
            },
            async {
                shell_handler(serde_json::json!({
                    "command": "echo ok",
                    "additional_permissions": {
                        "write": [{
                            "path": outside.to_string_lossy(),
                            "scope": "file",
                            "reason": "test"
                        }]
                    }
                }))
                .await
            },
        )
        .await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("approval receipt"), "{error}");
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn shell_handler_additional_permissions_consumes_approved_receipt() {
        let workspace = test_path("shell-additional-approved");
        std::fs::create_dir_all(&workspace).unwrap();
        let outside = std::env::current_dir()
            .unwrap()
            .join(format!("__cov_approved_{}.txt", std::process::id()));
        std::fs::write(&outside, "x").unwrap();
        let sandbox = Arc::new(ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Manual,
            },
            &workspace.to_string_lossy(),
        ));
        let permissions = crate::sandbox::windows_request::AdditionalPermissions {
            write: vec![crate::sandbox::windows_request::WritePermissionRequest {
                path: outside.to_string_lossy().to_string(),
                scope: crate::sandbox::windows_request::WriteScope::File,
                reason: "test".to_string(),
            }],
        };
        let result = with_tool_scope(
            ScopeOptions {
                workspace: workspace.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: sandbox.clone(),
                escalation: None,
                on_sandboxed: None,
            },
            async {
                // Pre-approve by deriving the receipt from the same command+target.
                let prepared =
                    crate::sandbox::windows_request::prepare(&sandbox, "echo ok", &permissions)
                        .unwrap();
                let receipt = prepared.approved_receipt("req-1".to_string()).unwrap();
                approve_windows_capability(receipt);

                shell_handler(serde_json::json!({
                    "command": "echo ok",
                    "additional_permissions": {
                        "write": [{
                            "path": outside.to_string_lossy(),
                            "scope": "file",
                            "reason": "test"
                        }]
                    }
                }))
                .await
            },
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("ok"));
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    #[cfg(windows)]
    fn clixml_lines_are_stripped_but_real_text_survives() {
        let input = "Cannot find path 'C:\\nope.txt' because it does not exist.\n\
                     #< CLIXML\n\
                     <Objs Version=\"1.1.0.1\" xmlns=\"http://schemas.microsoft.com/powershell/2004/04\"><Obj S=\"progress\">x</Obj></Objs>\n\
                     real trailing error line";
        let out = strip_powershell_clixml(input);
        assert!(out.contains("Cannot find path"));
        assert!(out.contains("real trailing error line"));
        assert!(!out.contains("CLIXML"));
        assert!(!out.contains("<Objs"));
        // No CLIXML marker → returned unchanged.
        assert_eq!(
            strip_powershell_clixml("plain error text"),
            "plain error text"
        );
    }

    fn test_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("futureos-tools-{name}-{stamp}"))
    }

    #[tokio::test]
    async fn edit_handler_accepts_camel_case_batch_edits() {
        let path = test_path("batch-edit");
        std::fs::write(&path, "alpha beta gamma").unwrap();

        let result = edit_handler(serde_json::json!({
            "path": path.to_string_lossy(),
            "edits": [
                { "oldText": "alpha", "newText": "one" },
                { "oldText": "gamma", "newText": "three" }
            ]
        }))
        .await;

        assert!(result.is_ok(), "camelCase batch edit failed: {result:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one beta three");
        std::fs::remove_file(path).ok();
    }

    // ─── reject_dangerous_command ──────────────────────────────────────────

    #[test]
    fn rejects_recursive_rm_of_home_and_roots() {
        for cmd in [
            "rm -rf ~",
            "rm -rf ~/",
            "rm -r $HOME",
            "rm -rf ${HOME}/src",
            "rm -rf /",
            "rm -rf /etc",
            "rm -rf /Users",
            "rm -rf /*",
            "rmdir ~",
            "rm -rf /tmp/..", // dot-segment traversal to root
            "rm -rf /private/var",
        ] {
            assert!(
                reject_dangerous_command(cmd).is_err(),
                "should reject: {cmd}"
            );
        }
    }

    #[test]
    fn rejects_bypass_spellings_of_recursive_rm() {
        for cmd in [
            "rm -fr ~",            // reordered flag cluster
            "rm -f -r ~",          // split flags
            "rm --recursive ~",    // long flag
            "rm  -rf  ~",          // extra whitespace
            "sudo rm -rf /",       // privilege wrapper
            "echo ok && rm -rf ~", // chained command
            "true; rm -r $HOME/x", // semicolon chain
            "RM -RF ~",            // case
        ] {
            assert!(
                reject_dangerous_command(cmd).is_err(),
                "should reject: {cmd}"
            );
        }
    }

    #[test]
    fn allows_legitimate_rm_targets() {
        for cmd in [
            "rm -rf target",
            "rm -rf ./node_modules",
            "rm -rf /tmp/future-build-cache",
            "rm -rf /Users/alice/project/target",
            "rm -f /tmp/stale.lock",
            "rm file.txt",
            "rmdir /tmp/empty-dir",
            "echo \"rm -rf ~\"", // quoted text is not a command
            "echo 'rm -rf /'",
        ] {
            assert!(reject_dangerous_command(cmd).is_ok(), "should allow: {cmd}");
        }
    }

    #[test]
    fn rejects_fork_bomb_patterns() {
        assert!(reject_dangerous_command(":(){ :|:& };:").is_err());
        assert!(reject_dangerous_command("while true; do dd if=/dev/zero; done").is_err());
        assert!(reject_dangerous_command("echo hello world").is_ok());
    }

    #[tokio::test]
    async fn scoped_workspace_writes_inside_workspace() {
        let workspace = test_path("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_string = workspace.to_string_lossy().to_string();
        let inside = workspace.join("poem.txt");

        let written_path =
            with_workspace_scope(workspace_string.clone(), "all".to_string(), async {
                run_write(&inside.to_string_lossy(), "inside workspace").await
            })
            .await
            .unwrap();

        assert_eq!(written_path, inside);
        assert_eq!(
            std::fs::read_to_string(&inside).unwrap(),
            "inside workspace"
        );
    }

    /// A path outside every writable root (workspace, tmp). Never created.
    fn outside_root_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dirs::home_dir()
            .unwrap()
            .join(format!("futureos-tools-test-{name}-{stamp}"))
            .join("outside.txt")
    }

    /// A tool scope with the sandbox ENABLED (rules active), OS wrapping off so
    /// only the application-layer boundary check is exercised. Mirrors GUI.
    fn active_policy_scope(workspace: &Path) -> ScopeOptions {
        let mut sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            workspace.to_string_lossy().as_ref(),
        );
        sandbox.set_backend_available_for_test(false);
        ScopeOptions {
            workspace: workspace.to_string_lossy().to_string(),
            permission_level: "workspace".to_string(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: None,
            on_sandboxed: None,
        }
    }

    #[tokio::test]
    async fn enabled_scope_rejects_unapproved_absolute_outside_write() {
        let workspace = test_path("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let outside = outside_root_path("reject");
        let outside_string = outside.to_string_lossy().to_string();

        let result = with_tool_scope(active_policy_scope(&workspace), async {
            run_write(&outside_string, "no").await
        })
        .await;

        assert!(result.is_err());
        assert!(!outside.exists());
    }

    #[tokio::test]
    async fn disabled_scope_is_fully_open() {
        // Non-GUI sessions (no policy) run fully open: even an outside write
        // succeeds (v2 decision — the sandbox is dormant unless GUI enables it).
        let workspace = test_path("ws-disabled");
        std::fs::create_dir_all(&workspace).unwrap();
        let tmp_target = test_path("open-write.txt");

        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "workspace".to_string(),
            async { run_write(&tmp_target.to_string_lossy(), "ok").await },
        )
        .await;

        assert!(
            result.is_ok(),
            "disabled scope should allow any write: {result:?}"
        );
        assert_eq!(std::fs::read_to_string(&tmp_target).unwrap(), "ok");
    }

    #[tokio::test]
    async fn active_policy_scope_allows_temp_dir_writes() {
        // With an active sandbox policy (GUI opt-in), temp dirs are writable
        // roots (desktop/DEV_MD/SANDBOX/COMMON.md).
        let workspace = test_path("ws-tmp");
        std::fs::create_dir_all(&workspace).unwrap();
        let tmp_target = test_path("tmp-write.txt");

        let result = with_tool_scope(active_policy_scope(&workspace), async {
            run_write(&tmp_target.to_string_lossy(), "tmp ok").await
        })
        .await;

        assert!(
            result.is_ok(),
            "temp-dir write should be allowed under an active policy: {result:?}"
        );
        assert_eq!(std::fs::read_to_string(&tmp_target).unwrap(), "tmp ok");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scoped_workspace_rejects_symlink_escape() {
        // A symlink inside the workspace pointing outside must not be treated
        // as an inside-workspace write (§3.5 rule 3).
        let workspace = test_path("ws-symlink");
        std::fs::create_dir_all(&workspace).unwrap();
        let outside_dir = dirs::home_dir()
            .unwrap()
            .join(format!("futureos-symlink-escape-{}", std::process::id()));
        // Target dir does not need to exist for the boundary check to resolve.
        let link = workspace.join("escape");
        std::os::unix::fs::symlink(&outside_dir, &link).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        let target = link.join("file.txt");

        let result = with_tool_scope(active_policy_scope(&workspace), async {
            run_write(&target.to_string_lossy(), "no").await
        })
        .await;

        std::fs::remove_dir_all(&outside_dir).ok();
        assert!(result.is_err(), "symlink escape should be rejected");
    }

    /// Scope with a sandbox forced "available" and a mock escalation requester
    /// that records calls and returns a fixed decision.
    fn escalation_scope(
        workspace: &Path,
        available: bool,
        decision: EscalationDecision,
        calls: Arc<Mutex<Vec<EscalationRequest>>>,
    ) -> ScopeOptions {
        let mut sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            workspace.to_string_lossy().as_ref(),
        );
        sandbox.set_backend_available_for_test(available);
        let requester: EscalationRequester = Arc::new(move |request: &EscalationRequest| {
            calls.lock().push(request.clone());
            decision.clone()
        });
        ScopeOptions {
            workspace: workspace.to_string_lossy().to_string(),
            permission_level: "workspace".to_string(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: Some(requester),
            on_sandboxed: None,
        }
    }

    #[tokio::test]
    async fn escalated_shell_denied_returns_error_without_running() {
        let workspace = test_path("escalate-denied");
        std::fs::create_dir_all(&workspace).unwrap();
        let marker = workspace.join("ran.marker");
        let calls = Arc::new(Mutex::new(vec![]));

        let result = with_tool_scope(
            escalation_scope(
                &workspace,
                true,
                EscalationDecision::Denied("not needed".to_string()),
                calls.clone(),
            ),
            async {
                run_shell(
                    &format!("touch {}", marker.to_string_lossy()),
                    30,
                    true,
                    "test needs it",
                )
                .await
            },
        )
        .await;

        assert!(result.is_err(), "denied escalation should error");
        assert!(!marker.exists(), "command must not run when denied");
        let recorded = calls.lock();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].justification, "test needs it");
        assert_eq!(
            recorded[0].trigger,
            crate::sandbox::EscalationTrigger::ModelRequest
        );
    }

    #[tokio::test]
    async fn escalated_shell_approved_runs_unsandboxed() {
        let workspace = test_path("escalate-approved");
        std::fs::create_dir_all(&workspace).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));

        let result = with_tool_scope(
            escalation_scope(
                &workspace,
                true,
                EscalationDecision::Approved,
                calls.clone(),
            ),
            async { run_shell("echo escalated-ok", 30, true, "why").await },
        )
        .await;

        assert!(result.unwrap().contains("escalated-ok"));
        assert_eq!(calls.lock().len(), 1);
    }

    #[tokio::test]
    async fn escalated_flag_is_ignored_when_sandbox_unavailable() {
        // Degraded mode: pre-execution approval already covered this command;
        // honoring `escalated` would double-prompt the user.
        let workspace = test_path("escalate-degraded");
        std::fs::create_dir_all(&workspace).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));

        let result = with_tool_scope(
            escalation_scope(
                &workspace,
                false,
                EscalationDecision::Denied("should never be asked".to_string()),
                calls.clone(),
            ),
            async { run_shell("echo degraded-ok", 30, true, "why").await },
        )
        .await;

        assert!(result.unwrap().contains("degraded-ok"));
        assert!(
            calls.lock().is_empty(),
            "escalation must not be raised in degraded mode"
        );
    }

    #[tokio::test]
    async fn enabled_scope_denies_unapproved_workspace_secret_write() {
        // `.env` inside the workspace is a built-in ask; a direct write that
        // never went through approval is rejected by ensure_workspace_access.
        let workspace = test_path("ws-secret");
        std::fs::create_dir_all(&workspace).unwrap();
        let env_file = workspace.join(".env");

        let result = with_tool_scope(active_policy_scope(&workspace), async {
            run_write(&env_file.to_string_lossy(), "SECRET=1").await
        })
        .await;

        assert!(result.is_err(), "unapproved .env write should be rejected");
        assert!(!env_file.exists());
    }

    #[tokio::test]
    async fn run_shell_abort_interrupt() {
        let workspace = test_path("abort-test");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_string = workspace.to_string_lossy().to_string();
        let interrupt_flag = Arc::new(AtomicBool::new(false));

        let flag_clone = interrupt_flag.clone();
        let shell_task = tokio::spawn(async move {
            with_workspace_scope_with_interrupt(
                workspace_string,
                "all".to_string(),
                flag_clone,
                async { run_shell("sleep 30", 60, false, "").await },
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        interrupt_flag.store(true, Ordering::SeqCst);

        let result = shell_task.await.unwrap();
        assert!(
            result.is_err(),
            "run_shell should return Err when interrupted"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("interrupted") || err.contains("Interrupted") || err.contains("abort"),
            "Error message should mention interruption: got '{err}'"
        );
    }

    // Aborting a shell command must kill its whole process group, not just the shell.
    // The command backgrounds a `sleep` that writes a marker file after it wakes;
    // if the grandchild survived the abort, the marker would appear.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_shell_abort_kills_grandchildren() {
        let workspace = test_path("abort-grandchild");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_string = workspace.to_string_lossy().to_string();
        let marker = workspace.join("survived.marker");
        let marker_string = marker.to_string_lossy().to_string();
        let interrupt_flag = Arc::new(AtomicBool::new(false));

        // `sh -c 'sleep 2; touch MARKER' &` — a grandchild that outlives the shell's
        // own exit unless the process group is killed.
        let command = format!("sh -c 'sleep 2; touch {marker_string}' & wait");
        let flag_clone = interrupt_flag.clone();
        let shell_task = tokio::spawn(async move {
            with_workspace_scope_with_interrupt(
                workspace_string,
                "all".to_string(),
                flag_clone,
                async move { run_shell(&command, 60, false, "").await },
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        interrupt_flag.store(true, Ordering::SeqCst);
        let result = shell_task.await.unwrap();
        assert!(
            result.is_err(),
            "run_shell should return Err when interrupted"
        );

        // Wait past the grandchild's sleep; if the group was killed, no marker.
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        assert!(
            !marker.exists(),
            "grandchild process survived abort and wrote the marker file"
        );
    }

    // ─── parse_result_failure ──────────────────────────────────────────────

    #[test]
    fn parse_result_failure_extracts_exit_code() {
        assert_eq!(
            parse_result_failure("some output\n[exit: 1]"),
            (1, "some output\n[exit: 1]".to_string())
        );
        assert_eq!(
            parse_result_failure("[exit: 0]"),
            (0, "[exit: 0]".to_string())
        );
        assert_eq!(
            parse_result_failure("no exit code here"),
            (0, "no exit code here".to_string())
        );
        // Long output gets tail-truncated
        let long = "a".repeat(5000) + "\n[exit: 5]";
        let (code, tail) = parse_result_failure(&long);
        assert_eq!(code, 5);
        assert!(tail.len() <= 2100);
        assert!(tail.contains("[exit: 5]"));
    }

    // ─── tool_end_semantics ────────────────────────────────────────────────

    #[test]
    fn tool_end_semantics_shell_exit_code_and_soft_fail() {
        // Non-zero exit of a plain grep is a normal signal, not a failure.
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::json!({"command": "grep -r pattern src"}),
            "no matches\n[exit: 1]",
        );
        assert_eq!(semantics.exit_code, Some(1));
        assert_eq!(semantics.is_soft_fail, Some(true));

        // A pipeline makes the exit code ambiguous — no soft-fail conclusion.
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::json!({"command": "grep -r pattern src | head"}),
            "[exit: 1]",
        );
        assert_eq!(semantics.exit_code, Some(1));
        assert_eq!(semantics.is_soft_fail, None);

        // Real failure keeps its code, no soft-fail.
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::json!({"command": "cargo build"}),
            "error[E0308]: mismatched types\n[exit: 101]",
        );
        assert_eq!(semantics.exit_code, Some(101));
        assert_eq!(semantics.is_soft_fail, None);

        // Exit 2 from grep is a real error (only exit 1 is "no match").
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::json!({"command": "grep -r pattern src"}),
            "grep: src: Permission denied\n[exit: 2]",
        );
        assert_eq!(semantics.exit_code, Some(2));
        assert_eq!(semantics.is_soft_fail, None);

        // Exit 0 carries the code but no failure semantics.
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::json!({"command": "ls"}),
            "file.txt\n[exit: 0]",
        );
        assert_eq!(semantics.exit_code, Some(0));
        assert_eq!(semantics.is_soft_fail, None);

        // Killed by a signal: no numeric code, no semantics.
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::json!({"command": "sleep 99"}),
            "[exit: signal]",
        );
        assert_eq!(semantics, ToolEndSemantics::default());

        // Windows: findstr is the grep analogue; `.exe` suffix and case
        // tolerated.
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::json!({"command": "C:\\Bin\\FINDSTR.exe pattern file"}),
            "[exit: 1]",
        );
        assert_eq!(semantics.is_soft_fail, Some(true));

        // Args may arrive as a JSON-encoded string (the agent's wire shape).
        let semantics = tool_end_semantics(
            "shell",
            &serde_json::Value::String(r#"{"command":"diff a b"}"#.to_string()),
            "1c1\n[exit: 1]",
        );
        assert_eq!(semantics.is_soft_fail, Some(true));
    }

    #[test]
    fn tool_end_semantics_target_path_for_write_and_edit() {
        let semantics = tool_end_semantics(
            "write",
            &serde_json::json!({"path": "/ws/report.md", "content": "..."}),
            "Written to /ws/report.md",
        );
        assert_eq!(semantics.target_path.as_deref(), Some("/ws/report.md"));

        // String-encoded args work too.
        let semantics = tool_end_semantics(
            "edit",
            &serde_json::Value::String(r#"{"path":"C:\\ws\\main.rs"}"#.to_string()),
            "Edited C:\\ws\\main.rs",
        );
        assert_eq!(semantics.target_path.as_deref(), Some("C:\\ws\\main.rs"));

        // Other tools report nothing structured.
        let semantics = tool_end_semantics(
            "read",
            &serde_json::json!({"path": "/ws/main.rs"}),
            "fn main() {}",
        );
        assert_eq!(semantics, ToolEndSemantics::default());
    }

    // ─── shell_segments ────────────────────────────────────────────────────

    #[test]
    fn shell_segments_splits_pipes() {
        let segments = shell_segments("echo hello | grep h");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0], vec!["echo", "hello"]);
        assert_eq!(segments[1], vec!["grep", "h"]);
    }

    #[test]
    fn shell_segments_single_command() {
        let segments = shell_segments("echo hello world");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0], vec!["echo", "hello", "world"]);
    }

    // ─── human_size ────────────────────────────────────────────────────────

    #[test]
    fn human_size_formats_correctly() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(500), "500B");
        assert_eq!(human_size(1024), "1KB");
        assert_eq!(human_size(1536), "1KB");
        assert_eq!(human_size(2048), "2KB");
        assert_eq!(human_size(1048576), "1.0MB");
        assert_eq!(human_size(1572864), "1.5MB");
    }

    // ─── format_shell_output ───────────────────────────────────────────────

    #[test]
    fn format_shell_output_includes_exit_code() {
        let output = format_shell_output("hello world", 11, 0);
        assert!(output.contains("hello world"));
        assert!(output.contains("[exit: 0]"));
    }

    #[test]
    fn format_shell_output_signal_exit() {
        let output = format_shell_output("killed", 6, -1);
        assert!(output.contains("[exit: signal]"));
    }

    // ─── truncate_for_error ────────────────────────────────────────────────

    #[test]
    fn truncate_for_error_shortens() {
        assert_eq!(truncate_for_error("hello"), "hello");
        assert!(truncate_for_error("").is_empty());
        let long = "a".repeat(500);
        let truncated = truncate_for_error(&long);
        assert!(truncated.len() < 500);
        assert!(truncated.ends_with('…'));
    }

    // ─── make_tool / tool schemas ──────────────────────────────────────────

    #[test]
    fn shell_tool_has_correct_name() {
        let tool = shell_tool();
        assert_eq!(tool.def.function.name, "shell");
        assert!(tool.def.function.description.contains("command"));
    }

    #[test]
    fn read_tool_has_correct_name() {
        let tool = read_tool();
        assert_eq!(tool.def.function.name, "read");
    }

    #[test]
    fn write_tool_has_correct_name() {
        let tool = write_tool();
        assert_eq!(tool.def.function.name, "write");
    }

    #[test]
    fn edit_tool_has_correct_name() {
        let tool = edit_tool();
        assert_eq!(tool.def.function.name, "edit");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn make_tool_includes_guidelines() {
        let tool = make_tool(
            "test_tool",
            "A test tool",
            serde_json::json!({"type": "object"}),
            |_: serde_json::Value| Box::pin(async { Ok("ok".to_string()) }),
            vec!["guideline 1"],
        );
        assert_eq!(tool.def.function.name, "test_tool");
        assert_eq!(tool.guidelines.len(), 1);
        // Invoke the handler once: an never-called test closure leaves its
        // creation line uncovered.
        let output = (tool.handler)(serde_json::json!({})).await.unwrap();
        assert_eq!(output, "ok");
    }

    #[tokio::test]
    async fn run_read_returns_file_contents() {
        let workspace = test_path("read-ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let file = workspace.join("test.txt");
        std::fs::write(&file, "line1\nline2\nline3").unwrap();

        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async { run_read("test.txt", None, None).await },
        )
        .await;

        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("line1"));
        assert!(content.contains("line3"));
    }

    #[tokio::test]
    async fn run_read_with_offset_and_limit() {
        let workspace = test_path("read-offset");
        std::fs::create_dir_all(&workspace).unwrap();
        let file = workspace.join("test.txt");
        std::fs::write(&file, "line1\nline2\nline3\nline4\nline5").unwrap();

        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async { run_read("test.txt", Some(2), Some(2)).await },
        )
        .await;

        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.contains("line1"));
        assert!(content.contains("line2"));
        assert!(content.contains("line3"));
        assert!(!content.contains("line4"));
    }

    #[tokio::test]
    async fn run_read_missing_file_errors() {
        let workspace = test_path("read-missing");
        std::fs::create_dir_all(&workspace).unwrap();

        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async { run_read("nonexistent.txt", None, None).await },
        )
        .await;

        assert!(result.is_err());
    }

    #[test]
    fn approve_outside_path_adds_to_approved_list() {
        // Without a scope, this is a no-op (should not panic)
        approve_outside_path("/tmp/test");
    }

    #[test]
    fn shell_schema_is_valid() {
        let schema = shell_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["command"].is_object());
    }

    #[test]
    fn read_schema_is_valid() {
        let schema = read_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
    }

    #[test]
    fn write_schema_is_valid() {
        let schema = write_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
    }

    #[test]
    fn edit_schema_is_valid() {
        let schema = edit_schema();
        assert_eq!(schema["type"], "object");
    }

    #[tokio::test]
    async fn shell_handler_executes_command() {
        let workspace = test_path("shell-hdl");
        std::fs::create_dir_all(&workspace).unwrap();
        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async { shell_handler(serde_json::json!({"command": "echo handler-works"})).await },
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("handler-works"));
    }

    #[tokio::test]
    async fn read_handler_reads_file() {
        let workspace = test_path("read-hdl");
        std::fs::create_dir_all(&workspace).unwrap();
        let file = workspace.join("data.txt");
        std::fs::write(&file, "file contents").unwrap();

        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async { read_handler(serde_json::json!({"path": "data.txt"})).await },
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("file contents"));
    }

    #[tokio::test]
    async fn write_handler_writes_file() {
        let workspace = test_path("write-hdl");
        std::fs::create_dir_all(&workspace).unwrap();

        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async {
                write_handler(serde_json::json!({
                    "path": "output.txt",
                    "content": "written content"
                }))
                .await
            },
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            std::fs::read_to_string(workspace.join("output.txt")).unwrap(),
            "written content"
        );
    }

    #[tokio::test]
    async fn edit_handler_edits_file() {
        let workspace = test_path("edit-hdl");
        std::fs::create_dir_all(&workspace).unwrap();
        let file = workspace.join("edit.txt");
        std::fs::write(&file, "before text").unwrap();

        let result = with_workspace_scope(
            workspace.to_string_lossy().to_string(),
            "all".to_string(),
            async {
                edit_handler(serde_json::json!({
                    "path": "edit.txt",
                    "oldText": "before",
                    "newText": "after"
                }))
                .await
            },
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "after text");
    }

    // ─── coding_tools / all_tools ──────────────────────────────────────────

    #[test]
    fn coding_tools_includes_four_tools() {
        let tools = coding_tools();
        assert_eq!(tools.len(), 4);
        let names: Vec<&str> = tools.iter().map(|t| t.def.function.name.as_str()).collect();
        assert!(names.contains(&"shell"));
        assert!(names.contains(&"read"));
        assert!(names.contains(&"write"));
        assert!(names.contains(&"edit"));
    }

    #[test]
    fn all_tools_equals_coding_tools() {
        let coding = coding_tools();
        let all = all_tools();
        assert_eq!(coding.len(), all.len());
    }

    // ── coverage batch: shell timeouts, truncation, edit/write errors ──────

    #[tokio::test(flavor = "current_thread")]
    async fn shell_timeout_kills_process_and_reports_partial_output() {
        // Partial output is drained and reported after the kill.
        let result = run_shell("echo partial-out; sleep 30", 1, false, "").await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("partial-out"), "{error}");

        // No output at all → the shorter error form.
        let result = run_shell("sleep 30", 1, false, "").await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("no output captured"), "{error}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_output_is_truncated_beyond_max_keep() {
        // seq 1..120000 ≈ 670 KB > MAX_KEEP (500 KB).
        let result = run_shell("seq 1 120000", 30, false, "").await.unwrap();
        assert!(result.contains("truncated"), "{result:.200}");
        assert!(result.contains("120000"), "tail kept: {result:.200}");
    }

    #[test]
    fn soft_fail_command_detection() {
        assert!(!is_soft_fail_command(""));
        assert!(!is_soft_fail_command("grep foo | head"));
        assert!(is_soft_fail_command("grep foo file.txt"));
        assert!(is_soft_fail_command("diff a b"));
    }

    #[test]
    fn dangerous_command_rejection_tolerates_empty_segments() {
        // Trailing operator → empty final segment takes the continue arm.
        assert!(reject_dangerous_command("ls && ").is_ok());
        assert!(reject_dangerous_command("echo hi").is_ok());
        assert!(reject_dangerous_command("rm -rf ~").is_err());
        assert!(reject_dangerous_command("sudo rm -rf ~").is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_handler_reports_missing_and_unmatched_edits() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit-me.txt");
        std::fs::write(&file, "hello world").unwrap();

        // Missing parameters entirely.
        let result = edit_handler(serde_json::json!({
            "path": file.to_string_lossy()
        }))
        .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing required parameters"));

        // oldText not present in the file.
        let result = edit_handler(serde_json::json!({
            "path": file.to_string_lossy(),
            "oldText": "not there",
            "newText": "x"
        }))
        .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("could not find the text to replace"));

        // Batch edits: one applies, one doesn't → aggregated error.
        let result = edit_handler(serde_json::json!({
            "path": file.to_string_lossy(),
            "edits": [
                {"oldText": "hello", "newText": "goodbye"},
                {"oldText": "missing", "newText": "x"}
            ]
        }))
        .await;
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("1 of 2 edit(s) could not be applied"),
            "{error}"
        );
        assert!(error.contains("missing"), "{error}");

        // All batch edits apply.
        let result = edit_handler(serde_json::json!({
            "path": file.to_string_lossy(),
            "edits": [
                {"oldText": "hello", "newText": "goodbye"},
                {"oldText": "world", "newText": "there"}
            ]
        }))
        .await;
        assert!(result.is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "goodbye there");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_handler_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b/c/deep.txt");
        let result = write_handler(serde_json::json!({
            "path": file.to_string_lossy(),
            "content": "deep"
        }))
        .await;
        assert!(result.is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "deep");
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn shell_escalated_prequest_approval_paths() {
        let ws = std::env::temp_dir().join(format!("futureos-esc-{}", crate::utils::generate_id()));
        std::fs::create_dir_all(&ws).unwrap();
        let sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        );
        // Force availability so the wrap check holds on platforms without
        // sandbox-exec: the approved/denied legs exercise escalation LOGIC
        // (the approved re-run is unsandboxed), not the OS boundary. An
        // early-return guard would be a dead line where Seatbelt exists.
        let mut sandbox = sandbox;
        sandbox.set_backend_available_for_test(true);
        // Approved pre-execution escalation runs unsandboxed.
        let approve: crate::sandbox::EscalationRequester =
            Arc::new(|_request| crate::sandbox::EscalationDecision::Approved);
        let result = with_tool_scope(
            ScopeOptions {
                workspace: ws.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: Some(approve),
                on_sandboxed: None,
            },
            run_shell("echo escalated-ok", 10, true, "because test"),
        )
        .await;
        assert!(result.unwrap().contains("escalated-ok"));

        // Denied escalation is an error, not a silent fallback.
        let sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        );
        let mut sandbox = sandbox;
        sandbox.set_backend_available_for_test(true);
        let deny: crate::sandbox::EscalationRequester =
            Arc::new(|_request| crate::sandbox::EscalationDecision::Denied("no way".to_string()));
        let result = with_tool_scope(
            ScopeOptions {
                workspace: ws.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: Some(deny),
                on_sandboxed: None,
            },
            run_shell("echo nope", 10, true, "because test"),
        )
        .await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("not approved: no way"), "{error}");
    }

    #[tokio::test]
    async fn linux_denial_uses_whole_command_escalation_and_infra_does_not() {
        let ws = test_path("linux-post-hoc");
        std::fs::create_dir_all(&ws).unwrap();
        let mut sandbox = ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        );
        sandbox.set_linux_backend_available_for_test();
        let deny: crate::sandbox::EscalationRequester = Arc::new(|request| {
            assert_eq!(request.command, "touch /protected");
            assert_eq!(
                request.trigger,
                crate::sandbox::EscalationTrigger::SandboxFailure
            );
            crate::sandbox::EscalationDecision::Denied("linux policy".into())
        });
        let denied = post_hoc_escalation(
            &Some(deny),
            &sandbox,
            "touch /protected",
            10,
            "touch: Permission denied\n\n[exit: 1]",
        )
        .await
        .expect("Linux denial should request whole-command escalation")
        .unwrap();
        assert!(denied.contains("not approved: linux policy"));

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_counter = calls.clone();
        let requester: crate::sandbox::EscalationRequester = Arc::new(move |_| {
            call_counter.fetch_add(1, Ordering::SeqCst);
            crate::sandbox::EscalationDecision::Approved
        });
        assert!(post_hoc_escalation(
            &Some(requester),
            &sandbox,
            "bad-helper",
            10,
            "future-linux-sandbox-helper: identity changed\n\n[exit: 125]",
        )
        .await
        .is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let dynamic = crate::sandbox::linux::violation::marker(
            &crate::sandbox::linux::violation::LinuxSandboxViolation {
                kind: crate::sandbox::linux::violation::LinuxViolationKind::DynamicGlobCreated,
                path_provenance: "glob_snapshot".into(),
                policy_digest: "a".repeat(64),
                detection_only: true,
                affected_count: 1,
            },
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_counter = calls.clone();
        let requester: crate::sandbox::EscalationRequester = Arc::new(move |_| {
            call_counter.fetch_add(1, Ordering::SeqCst);
            crate::sandbox::EscalationDecision::Approved
        });
        assert!(post_hoc_escalation(
            &Some(requester),
            &sandbox,
            "created-secret-but-failed",
            10,
            &format!("{dynamic}\n\n[exit: 1]"),
        )
        .await
        .is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(ws);
    }

    #[cfg(target_os = "macos")]
    #[allow(clippy::await_holding_lock)] // HOME must stay pinned across awaits
    #[tokio::test(flavor = "current_thread")]
    async fn shell_sandbox_denial_triggers_post_hoc_escalation() {
        let _home_guard = crate::test_support::home_env_lock();
        let ws =
            std::env::temp_dir().join(format!("futureos-esc2-{}", crate::utils::generate_id()));
        std::fs::create_dir_all(&ws).unwrap();
        let sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        );
        // This test is cfg(macos): Seatbelt always exists there. Force the
        // flag anyway so an early-return guard (a dead line) isn't needed.
        let mut sandbox = sandbox;
        sandbox.set_backend_available_for_test(true);
        // A write outside every writable root is denied by Seatbelt; the
        // escalation requester approves, so the command re-runs unsandboxed.
        let target = dirs::home_dir()
            .unwrap()
            .join(format!("futureos-escalated-{}.txt", std::process::id()));
        let approve: crate::sandbox::EscalationRequester =
            Arc::new(|_request| crate::sandbox::EscalationDecision::Approved);
        let command = format!("touch {}", target.display());
        let result = with_tool_scope(
            ScopeOptions {
                workspace: ws.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: Some(approve),
                on_sandboxed: None,
            },
            run_shell(&command, 15, false, ""),
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert!(target.exists(), "unsandboxed re-run created the file");
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_dir_all(&ws);
    }

    // ─── coverage batch 14: residual shell/sandbox arms ────────────────────

    #[test]
    fn reject_dangerous_command_skips_bare_privilege_wrapper() {
        // A segment that is ONLY the privilege wrapper strips to no tokens;
        // the segment is skipped rather than flagged.
        assert!(reject_dangerous_command("sudo").is_ok());
        assert!(reject_dangerous_command("doas").is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_handler_rule_file_deny_surfaces_error() {
        let ws =
            std::env::temp_dir().join(format!("futureos-deny-{}", crate::utils::generate_id()));
        std::fs::create_dir_all(&ws).unwrap();
        let sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Manual,
            },
            ws.to_string_lossy().as_ref(),
        );
        // The layer-0 builtin denies writes to the approval rule file.
        let rule_file = ws.join(".future/approval_rule.json");
        let result = with_tool_scope(
            ScopeOptions {
                workspace: ws.to_string_lossy().to_string(),
                permission_level: "default".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: None,
                on_sandboxed: None,
            },
            write_handler(serde_json::json!({
                "path": rule_file.to_string_lossy(),
                "content": "x"
            })),
        )
        .await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("denied by an approval rule"), "{error}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[cfg(target_os = "macos")]
    #[allow(clippy::await_holding_lock)] // HOME must stay pinned across awaits
    #[tokio::test(flavor = "current_thread")]
    async fn shell_sandbox_denial_post_hoc_denied_returns_annotated_output() {
        let _home_guard = crate::test_support::home_env_lock();
        let ws =
            std::env::temp_dir().join(format!("futureos-esc3-{}", crate::utils::generate_id()));
        std::fs::create_dir_all(&ws).unwrap();
        let sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        );
        // cfg(macos): Seatbelt always exists; force the flag (see the
        // approved-path sibling test).
        let mut sandbox = sandbox;
        sandbox.set_backend_available_for_test(true);
        // Seatbelt denies the write; the requester denies the escalation, so
        // the caller gets the original output plus a "not approved" note.
        let target = dirs::home_dir().unwrap().join(format!(
            "futureos-escalated-deny-{}.txt",
            std::process::id()
        ));
        let deny: crate::sandbox::EscalationRequester = Arc::new(|_request| {
            crate::sandbox::EscalationDecision::Denied("stay sandboxed".to_string())
        });
        let command = format!("touch {}", target.display());
        let result = with_tool_scope(
            ScopeOptions {
                workspace: ws.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: Some(deny),
                on_sandboxed: None,
            },
            run_shell(&command, 15, false, ""),
        )
        .await;
        let output = result.unwrap();
        assert!(output.contains("not approved: stay sandboxed"), "{output}");
        assert!(!target.exists(), "denied escalation never re-runs");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[cfg(not(windows))]
    struct FailingReader;

    #[cfg(not(windows))]
    impl tokio::io::AsyncRead for FailingReader {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::other("injected read failure")))
        }
    }

    #[cfg(not(windows))]
    #[tokio::test(flavor = "current_thread")]
    async fn read_shell_output_reads_until_eof_and_propagates_errors() {
        use tokio::io::AsyncWriteExt;
        let (mut tx, mut rx) = tokio::io::duplex(64);
        tx.write_all(b"hello").await.unwrap();
        drop(tx);
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        read_shell_output(&mut rx, &mut buf, &mut chunk)
            .await
            .unwrap();
        assert_eq!(buf, b"hello");

        let mut failing = FailingReader;
        let error = read_shell_output(&mut failing, &mut buf, &mut chunk)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Failed to read shell output"));
    }

    #[cfg(not(windows))]
    #[tokio::test(flavor = "current_thread")]
    async fn drain_shell_output_stops_on_eof() {
        use tokio::io::AsyncWriteExt;
        let (mut tx, mut rx) = tokio::io::duplex(64);
        tx.write_all(b"leftover").await.unwrap();
        drop(tx);
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        drain_shell_output(&mut rx, &mut buf, &mut chunk).await;
        assert_eq!(buf, b"leftover");
    }

    /// Shared denying escalation requester as a fn item: an inline closure
    /// that a test never triggers would itself be an uncovered line.
    #[cfg(test)]
    fn deny_escalation_fn(
        _request: &crate::sandbox::EscalationRequest,
    ) -> crate::sandbox::EscalationDecision {
        crate::sandbox::EscalationDecision::Denied("no".to_string())
    }

    #[test]
    fn deny_escalation_fn_denies() {
        let request = crate::sandbox::EscalationRequest {
            trigger: crate::sandbox::EscalationTrigger::ModelRequest,
            command: "x".to_string(),
            justification: String::new(),
            failure_summary: String::new(),
        };
        assert!(matches!(
            deny_escalation_fn(&request),
            crate::sandbox::EscalationDecision::Denied(_)
        ));
    }

    #[test]
    fn path_with_own_dir_prepends_binary_dir() {
        // Happy path: the binary's parent dir is prepended to PATH.
        let exe = std::env::current_exe().unwrap();
        let with = path_with_own_dir(Ok(exe.clone())).unwrap();
        let parent = exe.parent().unwrap();
        let sep = if cfg!(windows) { ";" } else { ":" };
        assert!(
            with.starts_with(&format!("{}{}", parent.display(), sep)),
            "{with}"
        );
        // An unreadable exe location → None (no prepend).
        assert!(path_with_own_dir(Err(std::io::Error::other("gone"))).is_none());
        // A parentless exe path (filesystem root) → None.
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(path_with_own_dir(Ok(std::path::PathBuf::from(root))).is_none());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "current_thread")]
    async fn shell_escalated_without_channel_falls_through_to_sandboxed_run() {
        let ws =
            std::env::temp_dir().join(format!("futureos-esc4-{}", crate::utils::generate_id()));
        std::fs::create_dir_all(&ws).unwrap();
        let sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        );
        // cfg(macos): Seatbelt always exists; force the flag (see siblings).
        let mut sandbox = sandbox;
        sandbox.set_backend_available_for_test(true);
        // escalated=true but NO escalation channel registered: the request
        // can't be approved, so the command runs normally (sandboxed).
        let result = with_tool_scope(
            ScopeOptions {
                workspace: ws.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: None,
                on_sandboxed: None,
            },
            run_shell("echo plain-fallthrough", 10, true, "no channel"),
        )
        .await;
        let output = result.unwrap();
        assert!(output.contains("plain-fallthrough"), "{output}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "current_thread")]
    async fn shell_non_denial_failure_does_not_escalate() {
        let ws =
            std::env::temp_dir().join(format!("futureos-esc5-{}", crate::utils::generate_id()));
        std::fs::create_dir_all(&ws).unwrap();
        let sandbox = crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
            },
            ws.to_string_lossy().as_ref(),
        );
        let mut sandbox = sandbox;
        sandbox.set_backend_available_for_test(true);
        // A sandboxed command that fails for an ORDINARY reason (not a
        // sandbox denial) must not even consult the escalation channel.
        let deny: crate::sandbox::EscalationRequester = Arc::new(deny_escalation_fn);
        let result = with_tool_scope(
            ScopeOptions {
                workspace: ws.to_string_lossy().to_string(),
                permission_level: "all".to_string(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: Some(deny),
                on_sandboxed: None,
            },
            run_shell("exit 3", 10, false, ""),
        )
        .await;
        let output = result.unwrap();
        assert!(output.contains("[exit: 3]"), "{output}");
        assert!(!output.contains("not approved"), "{output}");
        let _ = std::fs::remove_dir_all(&ws);
    }
}
