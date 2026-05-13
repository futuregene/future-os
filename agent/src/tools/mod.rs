//! Tools — 1:1 compatible with Go internal/tools/

use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

// ─── Tool definitions ────────────────────────────────────────────────────────

use crate::types::{AgentTool, FunctionDef, ToolDef};

fn make_tool(
    name: &str,
    description: &str,
    parameters: serde_json::Value,
    handler: fn(serde_json::Value) -> Result<String, anyhow::Error>,
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

pub fn bash_tool() -> AgentTool {
    make_tool("bash", "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 50000 bytes.", bash_schema(), |args| {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BashParams {
            command: String,
            timeout: Option<u64>,
        }
        let params: BashParams = serde_json::from_value(args)?;
        let output = tokio::runtime::Handle::current()
            .block_on(run_bash(&params.command, params.timeout.unwrap_or(120)))?;
        Ok(output)
    }, vec!["Prefer one bash command per turn"])
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

pub fn read_tool() -> AgentTool {
    make_tool("read", "Read a file from the filesystem.", read_schema(), |args| {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ReadParams {
            path: String,
            offset: Option<usize>,
            limit: Option<usize>,
        }
        let params: ReadParams = serde_json::from_value(args)?;
        let content = tokio::runtime::Handle::current()
            .block_on(run_read(&params.path, params.offset, params.limit))?;
        Ok(content)
    }, vec![])
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

pub fn write_tool() -> AgentTool {
    make_tool("write", "Write content to a file, creating or overwriting.", write_schema(), |args| {
        #[derive(serde::Deserialize)]
        struct WriteParams {
            path: String,
            content: String,
        }
        let params: WriteParams = serde_json::from_value(args)?;
        tokio::runtime::Handle::current()
            .block_on(run_write(&params.path, &params.content))?;
        Ok(format!("Written to {}", params.path))
    }, vec![])
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
                "description": "Array of {oldText, newText} for multi-edit mode"
            }
        },
        "required": ["path"]
    })
}

pub fn edit_tool() -> AgentTool {
    make_tool("edit", "Edit a file using exact text replacement. Supports multi-edit via edits array.", edit_schema(), |args| {
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
        let edits: Option<Vec<EditOp>> = params.edits.map(|es| es.into_iter().map(|e| EditOp { old_text: e.old_text, new_text: e.new_text }).collect());
        tokio::runtime::Handle::current()
            .block_on(run_edit(&params.path, old_text.as_deref(), new_text.as_deref(), edits.as_deref()))?;
        Ok(format!("Edited {}", params.path))
    }, vec!["Include enough context for unique matching"])
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

pub fn grep_tool() -> AgentTool {
    make_tool("grep", "Search for a pattern in files.", grep_schema(), |args| {
        let params: GrepParams = serde_json::from_value(args)?;
        let output = tokio::runtime::Handle::current()
            .block_on(run_grep(&params))?;
        Ok(output)
    }, vec![])
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

pub fn ls_tool() -> AgentTool {
    make_tool("ls", "List directory contents.", ls_schema(), |args| {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LsParams { path: Option<String>, limit: Option<usize> }
        let params: LsParams = serde_json::from_value(args)?;
        let output = tokio::runtime::Handle::current()
            .block_on(run_ls(params.path.as_deref(), params.limit.unwrap_or(500)))?;
        Ok(output)
    }, vec![])
}

// ─── All tools ─────────────────────────────────────────────────────────────

pub fn all_tools() -> Vec<AgentTool> {
    vec![
        bash_tool(),
        read_tool(),
        write_tool(),
        edit_tool(),
        grep_tool(),
        ls_tool(),
    ]
}

// ─── Tool runners ─────────────────────────────────────────────────────────

async fn run_bash(command: &str, timeout_secs: u64) -> Result<String> {
    let output = Command::new("bash")
        .args(["-c", command])
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    let combined = if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{}\n{}", stdout, stderr)
    };

    // Truncate to last 50000 bytes
    let combined = if combined.len() > 50000 {
        format!("...(truncated, showing last 50000 chars)\n{}", &combined[combined.len() - 50000..])
    } else {
        combined
    };

    Ok(format!("[exit code: {}]\n{}", exit_code, combined))
}

async fn run_read(path: &str, offset: Option<usize>, limit: Option<usize>) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut content = String::new();
    file.read_to_string(&mut content).await?;

    let offset = offset.unwrap_or(1).saturating_sub(1); // 1-indexed → 0-indexed
    let limit = limit.unwrap_or(usize::MAX);

    let lines: Vec<&str> = content.lines().skip(offset).take(limit).collect();
    let result = lines.join("\n");

    Ok(result)
}

async fn run_write(path: &str, content: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(content.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

async fn run_edit(path: &str, old_text: Option<&str>, new_text: Option<&str>, edits: Option<&[EditOp]>) -> Result<()> {
    let current = tokio::fs::read_to_string(path).await?;

    let final_content = if let Some(edits) = edits {
        // Multi-edit mode
        let mut result = current.clone();
        for edit in edits.iter().rev() {
            if let Some(pos) = result.rfind(&edit.old_text) {
                result = format!("{}{}{}", &result[..pos], edit.new_text, &result[pos + edit.old_text.len()..]);
            }
        }
        result
    } else if let (Some(old), Some(new)) = (old_text, new_text) {
        if let Some(pos) = current.find(old) {
            format!("{}{}{}", &current[..pos], new, &current[pos + old.len()..])
        } else {
            return Err(anyhow!("Pattern not found: {}", old));
        }
    } else {
        return Err(anyhow!("Either oldText+newText or edits array required"));
    };

    tokio::fs::write(path, &final_content).await?;
    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct EditOp { old_text: String, new_text: String }

async fn run_grep(params: &GrepParams) -> Result<String> {
    let mut args = vec![];
    if params.ignore_case.unwrap_or(false) { args.push("-i".to_string()); }
    if params.literal.unwrap_or(false) { args.push("-F".to_string()); }
    if let Some(c) = params.context { args.push(format!("-{}", c)); }
    args.push("-n".to_string());
    args.push(params.pattern.clone());
    if let Some(ref p) = params.path {
        if let Some(ref g) = params.glob {
            let include_pat = format!("--include={}", g);
            let output = Command::new("grep")
                .args(&["-r", &include_pat, &params.pattern, p])
                .output()
                .await?;
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }
        args.push(p.clone());
    }

    let output = Command::new("grep").args(&args).output().await?;
    let result = String::from_utf8_lossy(&output.stdout).to_string();

    let limit = params.limit.unwrap_or(100);
    let lines: Vec<&str> = result.lines().take(limit).collect();
    Ok(lines.join("\n"))
}

async fn run_ls(path: Option<&str>, limit: usize) -> Result<String> {
    let path = path.unwrap_or(".");
    let mut entries = tokio::fs::read_dir(path).await?;
    let mut result = Vec::new();
    let mut count = 0;
    while let Some(entry) = entries.next_entry().await? {
        if count >= limit { break; }
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().await?.is_dir();
        let suffix = if is_dir { "/" } else { "" };
        result.push(format!("{}{}", name, suffix));
        count += 1;
    }
    Ok(result.join("\n"))
}
