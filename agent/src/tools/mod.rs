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
        }
        let params: ShellParams = serde_json::from_value(args)?;
        run_shell(
            &params.command,
            params.timeout.unwrap_or(120),
            params.escalated.unwrap_or(false),
            params.justification.as_deref().unwrap_or(""),
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
        vec!["When asked to create, save, or overwrite a normal file, prefer this write tool."],
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
        vec!["Include enough context for unique matching"],
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
    if let Some(pgid) = pgid {
        // SAFETY: killpg is async-signal-safe and we target the group led by our
        // own just-spawned child. A stale/reaped pgid yields a harmless ESRCH.
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
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

async fn run_shell(
    command: &str,
    timeout_secs: u64,
    escalated: bool,
    justification: &str,
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
        if let Some(requester) = &escalation {
            let request = EscalationRequest {
                command: command.to_string(),
                justification: justification.to_string(),
                failure_summary: String::new(),
            };
            match requester(&request) {
                EscalationDecision::Approved => {
                    return spawn_shell(command, timeout_secs, &sandbox, true).await;
                }
                EscalationDecision::Denied(note) => {
                    return Err(anyhow!(
                        "Escalated execution was not approved{}. Run the command inside the sandbox instead, or explain to the user why it needs these permissions.",
                        if note.is_empty() { String::new() } else { format!(": {note}") }
                    ));
                }
            }
        }
        // No escalation channel: fall through to a normal sandboxed run.
    }

    let sandboxed = sandbox.wraps_shell();
    if sandboxed {
        if let Ok(Some(notify)) = TOOL_SCOPE.try_with(|scope| scope.on_sandboxed.clone()) {
            notify(command);
        }
    }
    let result = spawn_shell(command, timeout_secs, &sandbox, false).await?;

    // Post-hoc escalation: only when the failure narrowly looks like a sandbox
    // denial (conservative heuristic — ordinary failures go back to the model).
    if sandboxed {
        if let Some(requester) = &escalation {
            let (exit_code, tail) = parse_result_failure(&result);
            if exit_code != 0
                && crate::sandbox::looks_like_sandbox_denial(&sandbox, exit_code, &tail)
            {
                let request = EscalationRequest {
                    command: command.to_string(),
                    justification: String::new(),
                    failure_summary: tail,
                };
                match requester(&request) {
                    EscalationDecision::Approved => {
                        return spawn_shell(command, timeout_secs, &sandbox, true).await;
                    }
                    EscalationDecision::Denied(note) => {
                        return Ok(format!(
                            "{result}\n[sandbox] The command appears to have been blocked by the sandbox; running it without the sandbox was not approved{}.",
                            if note.is_empty() { String::new() } else { format!(": {note}") }
                        ));
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Extract the exit code and output tail from a formatted run_shell result, for
/// the sandbox-denial heuristic. Exit code is now at the end as "[exit: N]".
fn parse_result_failure(result: &str) -> (i32, String) {
    let exit_code = result
        .lines()
        .rev()
        .find_map(|line| {
            line.strip_prefix("[exit: ")
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|code| code.parse::<i32>().ok())
        })
        .unwrap_or(0);
    let tail_start = result.len().saturating_sub(2000);
    let tail = result.get(tail_start..).unwrap_or(result).to_string();
    (exit_code, tail)
}

/// Spawn a shell command (sandbox-wrapped unless `escalated`) and wait for it
/// with timeout + interrupt handling. Returns the formatted combined output.
async fn spawn_shell(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    escalated: bool,
) -> Result<String> {
    let cwd = active_workspace()?;
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
    let mut child = sandbox.build_shell_command(&merged_cmd, escalated);
    child.current_dir(&cwd).env("PWD", &cwd);
    // Prepend the agent binary's directory to PATH so bundled tools in the same
    // directory are discoverable by shell commands.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let existing = std::env::var("PATH").unwrap_or_default();
            let sep = if cfg!(windows) { ";" } else { ":" };
            child.env("PATH", format!("{}{}{}", dir.display(), sep, existing));
        }
    }
    child.stdout(std::process::Stdio::piped());
    // Unix: the subshell already merged stderr into stdout, so the outer pipe
    // carries nothing. Windows: PowerShell's own failures (a parse error in the
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

    // Get interrupt flag from task-local scope
    let interrupt_flag = TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.clone())
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));

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

        return match result {
            Ok(Ok(())) => {
                let combined = String::from_utf8_lossy(&output_buf);
                Ok(format_shell_output(&combined, combined.len(), 0))
            }
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => {
                let combined = String::from_utf8_lossy(&output_buf);
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
        };
    }

    #[cfg(not(windows))]
    let read_result = tokio::select! {
        result = tokio::time::timeout(timeout_dur, async {
            use tokio::io::AsyncReadExt;
            loop {
                match stdout.read(&mut read_buf).await {
                    Ok(0) => break, // EOF
                    Ok(n) => output_buf.extend_from_slice(&read_buf[..n]),
                    Err(e) => return Err(anyhow!("Failed to read shell output: {}", e)),
                }
            }
            Ok(spawned.wait().await)
        }) => result,
        _ = wait_for_interrupt(interrupt_flag.clone()) => {
            kill_process_group(pgid);
            return Err(anyhow!("Shell command interrupted by abort"));
        }
    };

    #[cfg(not(windows))]
    match read_result {
        Ok(Ok(Ok(status))) => {
            // Normal completion. On unix a successful command never kills the
            // process group, so intentionally detached grandchildren survive.
            // Drain any leftover bytes (rare: process exited but pipe still has data).
            use tokio::io::AsyncReadExt;
            loop {
                match stdout.read(&mut read_buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => output_buf.extend_from_slice(&read_buf[..n]),
                }
            }
            let combined = String::from_utf8_lossy(&output_buf);
            let exit_code = status.code().unwrap_or(-1);
            Ok(format_shell_output(&combined, combined.len(), exit_code))
        }
        Ok(Ok(Err(e))) => Err(anyhow!("Failed to run shell command: {}", e)),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => {
            // Timeout — kill process tree, drain remaining pipe content.
            kill_process_group(pgid);
            // Drain whatever the process wrote before the kill took effect.
            use tokio::io::AsyncReadExt;
            loop {
                match stdout.read(&mut read_buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => output_buf.extend_from_slice(&read_buf[..n]),
                }
            }
            let combined = String::from_utf8_lossy(&output_buf);
            let total = combined.len();
            if total == 0 {
                return Err(anyhow!(
                    "Shell command timed out after {} seconds (no output captured)",
                    timeout_secs.max(1)
                ));
            }
            let formatted = format_shell_output(&combined, total, -1);
            Err(anyhow!(
                "Shell command timed out after {} seconds.\nPartial output ({} total):\n{}",
                timeout_secs.max(1),
                human_size(total),
                formatted,
            ))
        }
    }
}

/// Drop the CLIXML noise Windows PowerShell serializes onto its stderr when it
/// is a redirected pipe (each block starts with a `#< CLIXML` marker line
/// followed by a `<Objs …>…</Objs>` XML payload). Line-based so it never eats
/// genuine error text. No-op when there is no CLIXML marker.
#[cfg(windows)]
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
    let trimmed = result.trim_end().to_string();
    if trimmed.is_empty() {
        result
    } else {
        trimmed
    }
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
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
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
        sandbox.available = false;
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
        // roots (SANDBOX_PLAN.md §2.2).
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
        sandbox.available = available;
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
}
