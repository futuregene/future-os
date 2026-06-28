//! Tools — 1:1 compatible with Go internal/tools/

use anyhow::{anyhow, Result};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::process::Command;

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
}

tokio::task_local! {
    static TOOL_SCOPE: ToolExecutionScope;
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
    let scope = ToolExecutionScope {
        workspace: normalize_path(&PathBuf::from(workspace)),
        approved_outside_paths: Arc::new(Mutex::new(vec![])),
        permission_level,
        interrupt_flag,
    };
    TOOL_SCOPE.scope(scope, future).await
}

pub fn approve_outside_path(path: &str) {
    let path = normalize_path(&PathBuf::from(path));
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
        }
        let params: BashParams = serde_json::from_value(args)?;
        run_bash(&params.command, params.timeout.unwrap_or(120)).await
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
            old_text: Option<String>,
            new_text: Option<String>,
            old_string: Option<String>,
            new_string: Option<String>,
            edits: Option<Vec<EditOp>>,
        }
        let params: EditParams = serde_json::from_value(args)?;
        let old_text = params.old_text.or(params.old_string);
        let new_text = params.new_text.or(params.new_string);
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

// ─── Grep Tool ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrepParams {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    ignore_case: Option<bool>,
    literal: Option<bool>,
    context: Option<usize>,
    limit: Option<usize>,
}

fn grep_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string" },
            "path": { "type": "string" },
            "glob": { "type": "string" },
            "ignore_case": { "type": "boolean" },
            "literal": { "type": "boolean" },
            "context": { "type": "integer" },
            "limit": { "type": "integer" }
        },
        "required": ["pattern"]
    })
}

fn grep_handler(args: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        let params: GrepParams = serde_json::from_value(args)?;
        run_grep(&params).await
    })
}

pub fn grep_tool() -> AgentTool {
    make_tool(
        "grep",
        "Search for a pattern in files.",
        grep_schema(),
        grep_handler,
        vec![],
    )
}

// ─── Ls Tool ────────────────────────────────────────────────────────────────

fn ls_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "limit": { "type": "integer" }
        }
    })
}

fn ls_handler(args: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LsParams {
            path: Option<String>,
            limit: Option<usize>,
        }
        let params: LsParams = serde_json::from_value(args)?;
        run_ls(params.path.as_deref(), params.limit.unwrap_or(500)).await
    })
}

pub fn ls_tool() -> AgentTool {
    make_tool(
        "ls",
        "List directory contents.",
        ls_schema(),
        ls_handler,
        vec![],
    )
}

// ─── Tool sets ─────────────────────────────────────────────────────────────

/// Core coding tools (default set): read, write, edit, bash
pub fn coding_tools() -> Vec<AgentTool> {
    vec![read_tool(), write_tool(), edit_tool(), bash_tool()]
}

/// Read-only tools: read, grep, ls
pub fn readonly_tools() -> Vec<AgentTool> {
    vec![read_tool(), grep_tool(), ls_tool()]
}

/// All built-in tools: read, write, edit, bash, grep, ls
pub fn all_tools() -> Vec<AgentTool> {
    vec![
        read_tool(),
        write_tool(),
        edit_tool(),
        bash_tool(),
        grep_tool(),
        ls_tool(),
    ]
}

// ─── Tool runners (async, using tokio) ─────────────────────────────────────

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

async fn run_bash(command: &str, timeout_secs: u64) -> Result<String> {
    let cwd = active_workspace()?;
    let mut child = Command::new("bash");
    child
        .args(["-c", command])
        .current_dir(&cwd)
        .env("PWD", &cwd);
    child.kill_on_drop(true);

    // Get interrupt flag from task-local scope
    let interrupt_flag = TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.clone())
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));

    // Use tokio::select! to race between:
    // 1. Command completion
    // 2. Timeout
    // 3. Interrupt signal (abort)
    let output = tokio::select! {
        result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs.max(1)),
            child.output(),
        ) => {
            match result {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => return Err(anyhow!("Failed to run bash command: {}", e)),
                Err(_) => return Err(anyhow!(
                    "Bash command timed out after {} seconds",
                    timeout_secs.max(1)
                )),
            }
        }
        _ = wait_for_interrupt(interrupt_flag.clone()) => {
            // Interrupt received, child will be dropped and killed via kill_on_drop
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
struct EditOp {
    old_text: String,
    new_text: String,
}

async fn run_grep(params: &GrepParams) -> Result<String> {
    let cwd = active_workspace()?;
    let mut args: Vec<String> = vec![];
    if params.ignore_case.unwrap_or(false) {
        args.push("-i".to_string());
    }
    if params.literal.unwrap_or(false) {
        args.push("-F".to_string());
    }
    if let Some(c) = params.context {
        args.push(format!("-{}", c));
    }
    args.push("-n".to_string());
    args.push(params.pattern.clone());

    let output = if let Some(ref p) = params.path {
        let path = workspace_path(p)?;
        let path = path.to_string_lossy().to_string();
        if let Some(ref g) = params.glob {
            let include_pat = format!("--include={}", g);
            Command::new("grep")
                .args(["-r", &include_pat, &params.pattern, &path])
                .current_dir(&cwd)
                .output()
                .await
        } else {
            args.push(path);
            Command::new("grep")
                .args(&args)
                .current_dir(&cwd)
                .output()
                .await
        }
    } else {
        Command::new("grep")
            .args(&args)
            .current_dir(&cwd)
            .output()
            .await
    }
    .map_err(|e| anyhow!("Failed to run grep: {}", e))?;

    let result = String::from_utf8_lossy(&output.stdout).to_string();

    let limit = params.limit.unwrap_or(100);
    let lines: Vec<&str> = result.lines().take(limit).collect();
    Ok(lines.join("\n"))
}

async fn run_ls(path: Option<&str>, limit: usize) -> Result<String> {
    let path = path.unwrap_or(".");
    let path = workspace_path(path)?;
    let mut entries = tokio::fs::read_dir(path).await?;
    let mut result = Vec::new();
    let mut count = 0;
    while let Some(entry) = entries.next_entry().await? {
        if count >= limit {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().await?.is_dir();
        let suffix = if is_dir { "/" } else { "" };
        result.push(format!("{}{}", name, suffix));
        count += 1;
    }
    Ok(result.join("\n"))
}

fn workspace_path(path: &str) -> Result<PathBuf> {
    let cwd = active_workspace()?;
    let path = if let Some(relative) = path.strip_prefix("~/") {
        cwd.join(relative)
    } else {
        PathBuf::from(path)
    };
    let raw_path = path.as_path();
    let absolute_path = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        cwd.join(raw_path)
    };
    let normalized_path = normalize_path(&absolute_path);
    Ok(normalized_path)
}

fn active_workspace() -> Result<PathBuf> {
    if let Ok(workspace) = TOOL_SCOPE.try_with(|scope| scope.workspace.clone()) {
        return Ok(workspace);
    }
    Ok(std::env::current_dir()?)
}

fn ensure_workspace_access(workspace: &Path, path: &Path) -> Result<()> {
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

    let workspace = normalize_path(workspace);
    let path = normalize_path(path);
    if path.starts_with(&workspace) || is_approved_outside_path(&path) {
        return Ok(());
    }

    Err(anyhow!(
        "Path is outside the current workspace and requires approval: {}",
        path.display()
    ))
}

fn is_approved_outside_path(path: &Path) -> bool {
    TOOL_SCOPE
        .try_with(|scope| {
            scope
                .approved_outside_paths
                .lock()
                .map(|approved| {
                    approved.iter().any(|approved_path| {
                        path == approved_path || path.starts_with(approved_path)
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

    #[tokio::test]
    async fn scoped_workspace_rejects_unapproved_absolute_outside_write() {
        let workspace = test_path("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let outside = test_path("outside.txt");
        let workspace_string = workspace.to_string_lossy().to_string();
        let outside_string = outside.to_string_lossy().to_string();

        let result = with_workspace_scope(workspace_string, "workspace".to_string(), async {
            run_write(&outside_string, "no").await
        })
        .await;

        assert!(result.is_err());
        assert!(!outside.exists());
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
                async { run_bash("sleep 30", 60).await },
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
}
