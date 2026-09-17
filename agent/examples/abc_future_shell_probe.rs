//! Native Future shell adapter for isolated retrieval replay.
//!
//! Calls the production shell tool handler and exports its actual definition.
//! The command is never tokenized or rewritten into a single CLI argv. Explicit
//! path denials are trusted harness configuration, not model arguments. Native
//! default read/network policy is not a global study-data isolation guarantee.
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
use future_agent::tools::{shell_tool, tool_end_semantics, with_tool_scope, ScopeOptions};

#[derive(Deserialize)]
struct Input {
    mode: String,
    workspace: Option<String>,
    #[serde(default)]
    denied_reads: Vec<String>,
    #[serde(default)]
    denied_writes: Vec<String>,
    arguments: Option<Value>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let input: Input = serde_json::from_str(&input)?;
    let tool = shell_tool();
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
    let workspace = input.workspace.context("workspace required")?;
    // These are explicit native path rules, not a claimed global read allowlist.
    // Preserve the backend's platform/pseudo-device rules rather than denying
    // the filesystem root and assuming a later allow can undo that denial.
    let mut rules = vec![json!({"path": workspace, "access":"write", "action":"deny"})];
    for root in input.denied_reads {
        if !Path::new(&root).is_absolute() {
            bail!("deny paths must be absolute");
        }
        rules.push(json!({"path":root,"access":"read","action":"deny"}));
    }
    for root in input.denied_writes {
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
    let args = input.arguments.context("native shell arguments required")?;
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
    let semantics = tool_end_semantics("shell", &args, &result);
    let terminated_by_signal = result.lines().any(|line| line == "[exit: signal]");
    println!(
        "{}",
        json!({"output":result,"exit_code":semantics.exit_code,
        "is_soft_fail":semantics.is_soft_fail,"terminated_by_signal":terminated_by_signal,"native_tool":"shell"})
    );
    Ok(())
}
