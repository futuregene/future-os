//! Tools — 1:1 compatible with Go internal/tools/

use anyhow::{anyhow, Result};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::sandbox::{EscalationDecision, EscalationRequest, EscalationRequester, ResolvedSandbox};

/// Callback invoked when a bash command is about to run inside the OS
/// sandbox (the RPC layer wires this to a `tool_sandboxed` SSE event).
pub type SandboxedNotifier = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct ToolExecutionScope {
    workspace: PathBuf,
    approved_outside_paths: Arc<Mutex<Vec<PathBuf>>>,
    /// "all" | "workspace" | "none" — controls workspace boundary enforcement
    permission_level: String,
    /// Interrupt flag for cooperative cancellation of long-running tool operations
    /// (e.g., bash commands). When set, in-flight tool work returns an "interrupted"
    /// error promptly and child processes are dropped (kill_on_drop).
    interrupt_flag: Arc<AtomicBool>,
    /// Resolved sandbox boundary: OS sandbox wrapping for bash, writable-roots
    /// boundary for write/edit. Shared with the approval layer so both reach
    /// the same verdicts.
    sandbox: Arc<ResolvedSandbox>,
    /// Post-hoc approval hook for escalated (out-of-sandbox) bash runs.
    /// Injected by the RPC layer; None means escalation is unavailable.
    escalation: Option<EscalationRequester>,
    /// Notifier for sandboxed bash executions (progress/event plumbing).
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
        workspace: normalize_path(&PathBuf::from(options.workspace)),
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
        if let Ok(mut approved) = scope.approved_outside_paths.lock() {
            approved.push(path);
        }
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

// ─── Bash Tool ───────────────────────────────────────────────────────────────

fn bash_schema() -> serde_json::Value {
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

fn bash_handler(args: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BashParams {
            command: String,
            timeout: Option<u64>,
            escalated: Option<bool>,
            justification: Option<String>,
        }
        let params: BashParams = serde_json::from_value(args)?;
        run_bash(
            &params.command,
            params.timeout.unwrap_or(120),
            params.escalated.unwrap_or(false),
            params.justification.as_deref().unwrap_or(""),
        )
        .await
    })
}

pub fn bash_tool() -> AgentTool {
    make_tool(
        "bash",
        "Execute a shell command in the current working directory. Use this for exploration and command-line programs. For ordinary file creation or edits, prefer write/edit tools, but shell redirection and heredocs may be used when they are the better fit. Returns stdout and stderr. Output is truncated to last 500000 bytes.",
        bash_schema(),
        bash_handler,
        vec![
            "Prefer one bash command per turn",
            "Prefer write/edit for ordinary file writes; use shell redirection, heredocs, tee, or cat > file only when they are more appropriate for the task.",
        ],
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

/// Core coding tools (default set): read, write, edit, bash
pub fn coding_tools() -> Vec<AgentTool> {
    vec![read_tool(), write_tool(), edit_tool(), bash_tool()]
}

/// All built-in tools
pub fn all_tools() -> Vec<AgentTool> {
    vec![read_tool(), write_tool(), edit_tool(), bash_tool()]
}

// ─── Tool runners (async, using tokio) ─────────────────────────────────────

/// SIGKILL an entire process group by its group-leader PID. Used to tear down a
/// bash command's full process tree on abort/timeout, since `kill_on_drop` only
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

async fn run_bash(
    command: &str,
    timeout_secs: u64,
    escalated: bool,
    justification: &str,
) -> Result<String> {
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
    if escalated && sandbox.wraps_bash() {
        if let Some(requester) = &escalation {
            let request = EscalationRequest {
                command: command.to_string(),
                justification: justification.to_string(),
                failure_summary: String::new(),
            };
            match requester(&request) {
                EscalationDecision::Approved => {
                    return spawn_bash(command, timeout_secs, &sandbox, true).await;
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

    let sandboxed = sandbox.wraps_bash();
    if sandboxed {
        if let Ok(Some(notify)) = TOOL_SCOPE.try_with(|scope| scope.on_sandboxed.clone()) {
            notify(command);
        }
    }
    let result = spawn_bash(command, timeout_secs, &sandbox, false).await?;

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
                        return spawn_bash(command, timeout_secs, &sandbox, true).await;
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

/// Extract the exit code and output tail from a formatted run_bash result, for
/// the sandbox-denial heuristic. Results are formatted "[exit code: N]\n...".
fn parse_result_failure(result: &str) -> (i32, String) {
    let exit_code = result
        .strip_prefix("[exit code: ")
        .and_then(|rest| rest.split(']').next())
        .and_then(|code| code.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let tail_start = result.len().saturating_sub(2000);
    let tail = result.get(tail_start..).unwrap_or(result).to_string();
    (exit_code, tail)
}

/// Spawn a bash command (sandbox-wrapped unless `escalated`) and wait for it
/// with timeout + interrupt handling. Returns the formatted combined output.
async fn spawn_bash(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    escalated: bool,
) -> Result<String> {
    let cwd = active_workspace()?;
    let mut child = sandbox.build_bash_command(command, escalated);
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
    child
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    child.kill_on_drop(true);
    // Run bash as the leader of its own process group so abort/timeout can kill
    // the whole tree. kill_on_drop alone only SIGKILLs bash itself, leaving
    // grandchildren (e.g. a `sleep` spawned by the command) running as orphans.
    // sandbox-exec execs its child, so the group covers the wrapped tree too.
    #[cfg(unix)]
    child.process_group(0);

    // Get interrupt flag from task-local scope
    let interrupt_flag = TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.clone())
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));

    // Spawn (rather than `output()`) so we hold the PID for group teardown.
    // With process_group(0) the child's PID equals its process-group ID.
    let spawned = child
        .spawn()
        .map_err(|e| anyhow!("Failed to run bash command: {}", e))?;
    #[cfg(unix)]
    let pgid = spawned.id().map(|id| id as i32);
    // Windows has no process groups; a Job Object with KILL_ON_JOB_CLOSE is the
    // equivalent tree-teardown. Assign the fresh PID so abort/timeout kills bash
    // and its grandchildren (e.g. a background `sleep`), not just bash itself.
    #[cfg(windows)]
    let job = {
        let job = crate::sandbox::windows::Job::create().ok();
        if let (Some(job), Some(pid)) = (&job, spawned.id()) {
            let _ = job.assign(pid);
        }
        job
    };

    // Use tokio::select! to race between:
    // 1. Command completion
    // 2. Timeout
    // 3. Interrupt signal (abort)
    let output = tokio::select! {
        result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs.max(1)),
            spawned.wait_with_output(),
        ) => {
            match result {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => return Err(anyhow!("Failed to run bash command: {}", e)),
                Err(_) => {
                    #[cfg(unix)]
                    kill_process_group(pgid);
                    #[cfg(windows)]
                    if let Some(job) = &job {
                        job.terminate();
                    }
                    return Err(anyhow!(
                        "Bash command timed out after {} seconds",
                        timeout_secs.max(1)
                    ));
                }
            }
        }
        _ = wait_for_interrupt(interrupt_flag.clone()) => {
            // Kill the whole group; dropping the wait future also kills bash via
            // kill_on_drop, but only killpg reaches grandchildren.
            #[cfg(unix)]
            kill_process_group(pgid);
            #[cfg(windows)]
            if let Some(job) = &job {
                job.terminate();
            }
            return Err(anyhow!("Bash command interrupted by abort"));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    let combined = if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{}\n{}", stdout, stderr)
    };

    // Truncate to last 500000 bytes, respecting UTF-8 char boundaries
    let combined = if combined.len() > 500000 {
        let start = combined.ceil_char_boundary(combined.len() - 500000);
        format!(
            "...(truncated, showing last 500000 chars)\n{}",
            &combined[start..]
        )
    } else {
        combined
    };

    let result = if exit_code != 0 {
        format!("[exit code: {}]\n{}", exit_code, combined)
    } else {
        combined
    };
    // Strip trailing blank lines
    let trimmed = result.trim_end().to_string();
    Ok(if trimmed.is_empty() { result } else { trimmed })
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
        // Multi-edit mode
        let mut result = current.clone();
        for edit in edits.iter().rev() {
            if let Some(pos) = result.rfind(&edit.old_text) {
                result = format!(
                    "{}{}{}",
                    &result[..pos],
                    edit.new_text,
                    &result[pos + edit.old_text.len()..]
                );
            }
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
    let normalized_path = normalize_path(&absolute_path);
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
                .map(|approved| {
                    approved.iter().any(|approved_path| {
                        crate::sandbox::paths::path_within(path, approved_path)
                    })
                })
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("futureos-tools-{name}-{stamp}"))
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
            calls.lock().unwrap().push(request.clone());
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
    async fn escalated_bash_denied_returns_error_without_running() {
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
                run_bash(
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
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].justification, "test needs it");
    }

    #[tokio::test]
    async fn escalated_bash_approved_runs_unsandboxed() {
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
            async { run_bash("echo escalated-ok", 30, true, "why").await },
        )
        .await;

        assert!(result.unwrap().contains("escalated-ok"));
        assert_eq!(calls.lock().unwrap().len(), 1);
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
            async { run_bash("echo degraded-ok", 30, true, "why").await },
        )
        .await;

        assert!(result.unwrap().contains("degraded-ok"));
        assert!(
            calls.lock().unwrap().is_empty(),
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
    async fn run_bash_abort_interrupt() {
        let workspace = test_path("abort-test");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_string = workspace.to_string_lossy().to_string();
        let interrupt_flag = Arc::new(AtomicBool::new(false));

        let flag_clone = interrupt_flag.clone();
        let bash_task = tokio::spawn(async move {
            with_workspace_scope_with_interrupt(
                workspace_string,
                "all".to_string(),
                flag_clone,
                async { run_bash("sleep 30", 60, false, "").await },
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        interrupt_flag.store(true, Ordering::SeqCst);

        let result = bash_task.await.unwrap();
        assert!(
            result.is_err(),
            "run_bash should return Err when interrupted"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("interrupted") || err.contains("Interrupted") || err.contains("abort"),
            "Error message should mention interruption: got '{err}'"
        );
    }

    // Aborting a bash command must kill its whole process group, not just bash.
    // The command backgrounds a `sleep` that writes a marker file after it wakes;
    // if the grandchild survived the abort, the marker would appear.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_bash_abort_kills_grandchildren() {
        let workspace = test_path("abort-grandchild");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace_string = workspace.to_string_lossy().to_string();
        let marker = workspace.join("survived.marker");
        let marker_string = marker.to_string_lossy().to_string();
        let interrupt_flag = Arc::new(AtomicBool::new(false));

        // `sh -c 'sleep 2; touch MARKER' &` — a grandchild that outlives bash's
        // own exit unless the process group is killed.
        let command = format!("sh -c 'sleep 2; touch {marker_string}' & wait");
        let flag_clone = interrupt_flag.clone();
        let bash_task = tokio::spawn(async move {
            with_workspace_scope_with_interrupt(
                workspace_string,
                "all".to_string(),
                flag_clone,
                async move { run_bash(&command, 60, false, "").await },
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        interrupt_flag.store(true, Ordering::SeqCst);
        let result = bash_task.await.unwrap();
        assert!(
            result.is_err(),
            "run_bash should return Err when interrupted"
        );

        // Wait past the grandchild's sleep; if the group was killed, no marker.
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        assert!(
            !marker.exists(),
            "grandchild process survived abort and wrote the marker file"
        );
    }
}
