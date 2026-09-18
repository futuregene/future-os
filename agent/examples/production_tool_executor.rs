//! Native Future tool executor for isolated replay.
//!
//! Calls the production handler for the named tool and exports that tool's actual
//! definition from `coding_tools()`. A shell command is never tokenized or rewritten into a
//! single CLI argv. Explicit path denials are trusted harness configuration, not model
//! arguments; the native default read/network policy is not, on its own, a study-data
//! isolation guarantee.
//!
//! Any tool in production's installed set can be executed here, so a replay can hand the
//! model production's real definitions and then run whatever it calls, through the same
//! handlers, scope and sandbox a session uses.
use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use future_agent::sandbox::rules::parse_rule_file;
use future_agent::sandbox::{ResolvedSandbox, SandboxPolicy, SandboxTier};
use future_agent::tools::{tool_end_semantics, with_tool_scope, ScopeOptions};

#[derive(Deserialize)]
struct Input {
    mode: String,
    /// Which installed tool to describe or execute. Defaults to `shell`, the tool through
    /// which a session reaches the history CLI.
    #[serde(default)]
    tool: Option<String>,
    workspace: Option<String>,
    #[serde(default)]
    denied_reads: Vec<String>,
    #[serde(default)]
    denied_writes: Vec<String>,
    /// Paths this replay may touch. Production's `read`/`write`/`edit` resolve a relative
    /// path against the workspace but accept an absolute one as-is, so the rule-based
    /// denials only reach `shell` through the OS sandbox. A replay that hands the model
    /// production's whole tool set therefore has to bound those tools itself; this is
    /// harness isolation layered on production handlers, not a production guarantee.
    #[serde(default)]
    allowed_roots: Vec<String>,
    arguments: Option<Value>,
}

/// Refuse a path argument outside the replay's own roots, for every tool.
///
/// `shell` is bounded by the OS sandbox; the file tools are not, so the check has to happen
/// here or a replay that exposes production's full tool set can read whatever the process
/// can. Deny-by-default: with no `allowed_roots` only the workspace is reachable.
fn enforce_allowed_roots(input: &Input, tool: &str, args: &Value) -> Result<()> {
    let workspace = input.workspace.as_deref().context("workspace required")?;
    let mut roots: Vec<std::path::PathBuf> = input
        .allowed_roots
        .iter()
        .map(|root| crate::resolved(root))
        .collect();
    roots.push(crate::resolved(workspace));
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let paths: Vec<String> = ["path", "file_path", "filePath"]
        .iter()
        .filter_map(|key| args.get(key).and_then(Value::as_str).map(str::to_string))
        .collect();
    for raw in paths {
        // `~` resolves to the real home, which production does deliberately; a replay must
        // not inherit that, so an explicit root is required for it to be reachable.
        let expanded = if let Some(rest) = raw.strip_prefix("~/") {
            match &home {
                Some(home) => home.join(rest),
                None => anyhow::bail!("refusing {tool} path {raw}: no HOME to resolve against"),
            }
        } else {
            std::path::PathBuf::from(&raw)
        };
        let resolved = if expanded.is_absolute() {
            crate::resolved_path(&expanded)
        } else {
            crate::resolved_path(&crate::path_join(workspace, &expanded))
        };
        if !roots.iter().any(|root| resolved.starts_with(root)) {
            anyhow::bail!(
                "STUDY_SCOPE_DENIED: {tool} path {} is outside this replay's roots",
                resolved.display()
            );
        }
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let input: Input = serde_json::from_str(&input)?;
    let name = input.tool.clone().unwrap_or_else(|| "shell".to_string());
    let name = name.as_str();
    // Look the tool up in the set production installs, so a replay cannot execute a
    // capability the runtime does not have.
    let tool = future_agent::tools::coding_tools()
        .into_iter()
        .find(|tool| tool.def.function.name == name)
        .with_context(|| format!("tool {name} is not in production's installed set"))?;
    if input.mode == "describe" {
        println!("{}", serde_json::to_string(&tool.def)?);
        return Ok(());
    }
    if input.mode != "execute" {
        bail!("expected describe or execute");
    }
    // Windows' current native restricted backend protects writes, not the
    // read isolation this study requires. Do not silently run less isolated.
    if cfg!(windows) {
        bail!("this replay requires a verified native read-isolating sandbox");
    }
    let workspace = input.workspace.clone().context("workspace required")?;
    // These are explicit native path rules, not a claimed global read allowlist.
    // Preserve the backend's platform/pseudo-device rules rather than denying
    // the filesystem root and assuming a later allow can undo that denial.
    let mut rules = vec![json!({"path": workspace, "access":"write", "action":"deny"})];
    for root in &input.denied_reads {
        if !Path::new(&root).is_absolute() {
            bail!("deny paths must be absolute");
        }
        rules.push(json!({"path":root,"access":"read","action":"deny"}));
    }
    for root in &input.denied_writes {
        if !Path::new(&root).is_absolute() {
            bail!("deny paths must be absolute");
        }
        rules.push(json!({"path":root,"access":"write","action":"deny"}));
    }
    let compiled = parse_rule_file(&json!({"rules":rules}).to_string(), Path::new(&workspace))
        .context("invalid replay rules")?;
    let sandbox = ResolvedSandbox::resolve_with_session(
        &SandboxPolicy {
            tier: SandboxTier::Sandbox,
        },
        &workspace,
        Arc::new(Mutex::new(compiled)),
    );
    if !sandbox.wraps_shell() {
        bail!("native OS sandbox unavailable; replay refuses to downgrade");
    }
    let args = input
        .arguments
        .clone()
        .context("native tool arguments required")?;
    enforce_allowed_roots(&input, name, &args)?;
    if args.get("escalated").and_then(Value::as_bool) == Some(true)
        || args.get("additional_permissions").is_some()
        || args.get("additionalPermissions").is_some()
    {
        bail!("replay does not permit escalation or added writes");
    }
    let result = with_tool_scope(
        ScopeOptions {
            workspace,
            permission_level: "workspace".into(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: None,
            on_sandboxed: None,
        },
        (tool.handler)(args.clone()),
    )
    .await?;
    let semantics = tool_end_semantics(name, &args, &result);
    let terminated_by_signal = result.lines().any(|line| line == "[exit: signal]");
    println!(
        "{}",
        json!({"output":result,"exit_code":semantics.exit_code,
        "is_soft_fail":semantics.is_soft_fail,"terminated_by_signal":terminated_by_signal,"native_tool":name})
    );
    Ok(())
}

/// Lexically normalized absolute path, matching how the file tools resolve one.
fn resolved_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn resolved(path: &str) -> std::path::PathBuf {
    resolved_path(&std::path::PathBuf::from(path))
}

fn path_join(base: &str, rest: &std::path::Path) -> std::path::PathBuf {
    std::path::PathBuf::from(base).join(rest)
}
