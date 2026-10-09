//! `future session` — 1:1 port of cli/src/commands/session.ts.
//!
//! list / set / info / rename / delete agent sessions via gRPC.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
#[cfg(test)]
use chrono::TimeZone;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// `help()` from session.ts — printed to stdout.
pub const SESSION_HELP: &str = "future session — manage agent sessions

Usage:
  future session list [--json]                       List all sessions
  future session new [options]                       Create a session
  future session set <id> [options]                  Change settings on an existing session
  future session info <id> [--json]                  Show session details + stats
  future session status <id> [--json] [--metrics]    Live state: permission, context, runs
  future session transcript --help                   Filter/window one session's records
  future session history --help                      Search/read original history
  future session compact --help                      Request manual compaction
  future session rename <id> <name>                  Give a session a readable name
  future session fork <id> --entry <entry-id>        Branch a new session at a user turn
  future session forks <id> [--json]                 List the turns a session can fork at
  future session clone <id>                          Copy a session at its latest settled point
  future session title <id>                          Generate a title (a model call)
  future session export <id> [--out <path>]          Write the session to an HTML file
  future session abort <id>                          Stop the active run and clear the queue
  future session cancel <id> --run <run-id>          Cancel one queued run
  future session approvals <id> [--json]             Approval requests the session waits on
  future session approve <id> <request-id> [opts]    Allow a pending approval
  future session reject <id> <request-id> [opts]     Deny a pending approval
  future session delete <id>                         Delete a session

Session data is stored in ~/.future/agent/agent.db";

/// `future session set --help`.
pub const SESSION_SET_HELP: &str = r#"future session set — change an existing session's settings

Usage:
  future session set <session-id> [options]

Recorded with the session:
  --parent <session-id>   Record another session as this session's parent;
                          pass an empty string to detach
  --title <name>          Session title
  --cwd <dir>             Working directory. The conversation is also filed in
                          the desktop app's workspace list under this directory
                          (see `future workspace`), the same way the app files
                          it when it observes the change itself — so it does not
                          depend on the app being open. A directory that does
                          not exist is reported as a note, after the change.
  --model <id>            Model ID
  --thinking <level>      Thinking level: off, minimal, low, medium, high, xhigh

Applied to the live session (until this agent stops):
  --tools <names>         Enable exactly these tools (comma-separated).
  --no-tools              Disable every tool.
  --no-builtin-tools      Disable built-in tools only (keep extensions).
  --system-prompt <text>  Replace the system prompt.
  --append-system-prompt <text>
                          Append to the system prompt.
  --permission <level>    Tool permission: all|workspace|none. This is the
                          approval gate; it is not the OS sandbox.
  --sandbox <tier>        Approval mode: off|manual|sandbox|auto. `sandbox` is
                          refused when the platform cannot provide one.
  --context-files <on|off>
                          Load CLAUDE.md and friends into the prompt.
  --auto-compact <on|off> Compact autonomously when context fills.
  --auto-retry <on|off>   Retry a failed request automatically.

Output:
  --json                  Print the update as a JSON object
  --help, -h              Show this help

Only the options you pass are changed; everything else is left alone, and one
failure does not roll back the others. A parent records lineage only — unlike
`fork`, no history is copied, and the parent must be an existing session.

The model and thinking level take effect at once and are stored with the
session's next run (they are part of the run snapshot). `future run` applies its
own --cwd (the current directory by default) when a run starts, so a cwd set
here stays in effect only until the next run overrides it. Read the current
values back with `future session status <id>`."#;

fn help(out: &Output) {
    out.log(SESSION_HELP);
}

/// `truncate(s, n)` — cut to n chars (last char replaced with "…").
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n - 1).collect();
        out.push('…');
        out
    }
}

/// `humanTokens(n)` from session.ts.
pub(super) fn human_tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", (n as f64 / 1_000.0).round() as i64)
    } else {
        n.to_string()
    }
}

/// `ago(iso)` — relative time from `now_ms` (injected for testability).
#[cfg(test)]
fn ago(iso: &str, now_ms: i64) -> String {
    relative_age(parse_timestamp_ms(iso), now_ms)
}

fn relative_age(updated_ms: i64, now_ms: i64) -> String {
    let ms = now_ms - updated_ms;
    let mins = ms / 60_000;
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else {
        let hrs = mins / 60;
        if hrs < 24 {
            format!("{hrs}h ago")
        } else {
            format!("{}d ago", hrs / 24)
        }
    }
}

/// `new Date(iso).getTime()` — RFC3339, or local "YYYY-MM-DD HH:MM:SS"
/// (the `updated_at` format), else 0 (JS gives NaN).
#[cfg(test)]
fn parse_timestamp_ms(iso: &str) -> i64 {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        return dt.timestamp_millis();
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%d %H:%M:%S") {
        // Ambiguous/nonexistent local times (DST gaps) fall back to 0 (NaN).
        return chrono::Local
            .from_local_datetime(&naive)
            .single()
            .map(|local| local.timestamp_millis())
            .unwrap_or(0);
    }
    0
}

/// `Date.now()`.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ─── List ─────────────────────────────────────────────────────────────────

/// `listSessions(jsonFlag)`.
async fn list_sessions(json_flag: bool, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    // Not wrapped in try/catch — errors propagate to main().catch.
    let data = client.list_sessions().await?;
    // `const { sessions } = await client.listSessions();`
    let sessions: Vec<Value> = data
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if json_flag {
        // Serializing plain JSON values is infallible.
        out.log(&serde_json::to_string_pretty(&json_value(&sessions)).expect("json serializes"));
        return Ok(());
    }

    if sessions.is_empty() {
        out.log("No sessions found.");
        return Ok(());
    }

    let mut sessions = sessions;
    sessions.sort_by_key(|s| std::cmp::Reverse(s["updatedAtMs"].as_i64().unwrap_or(0)));

    // Header.
    out.log(&format!(
        "  {} {} {} {} QUERIES",
        pad_end("SESSION ID", 24),
        pad_end("TITLE", 38),
        pad_end("UPDATED", 10),
        pad_end("MODEL", 28)
    ));
    out.log(&format!(
        "  {} {} {} {} ———————",
        "—".repeat(24),
        "—".repeat(38),
        "—".repeat(10),
        "—".repeat(28)
    ));

    for s in &sessions {
        let session_name = s.get("sessionName").and_then(Value::as_str).unwrap_or("");
        let first_message = s.get("firstMessage").and_then(Value::as_str).unwrap_or("");
        // `s.session_name || s.first_message ? truncate(...) : "(untitled)"`
        let title = if !session_name.is_empty() || !first_message.is_empty() {
            truncate(
                if !session_name.is_empty() {
                    session_name
                } else {
                    first_message
                },
                42,
            )
        } else {
            "(untitled)".to_string()
        };
        // `s.model.length > 28 ? s.model.slice(0, 27) + "…" : s.model`
        let model_raw = s.get("model").and_then(Value::as_str).unwrap_or("");
        let model = if model_raw.chars().count() > 28 {
            let mut m: String = model_raw.chars().take(27).collect();
            m.push('…');
            m
        } else {
            model_raw.to_string()
        };
        // `s.query_count ? \`${s.query_count}\` : "—"`
        let q = match s.get("queryCount").and_then(Value::as_i64) {
            Some(n) if n != 0 => n.to_string(),
            _ => "—".to_string(),
        };
        let id = s.get("id").and_then(Value::as_str).unwrap_or("");
        let updated = s["updatedAtMs"].as_i64().unwrap_or(0);
        out.log(&format!(
            "  {} {} {} {} {}",
            pad_end(id, 24),
            pad_end(&title, 38),
            pad_end(&relative_age(updated, now_ms()), 10),
            pad_end(&model, 28),
            q
        ));
    }
    out.log(&format!("\n{} sessions.", sessions.len()));
    Ok(())
}

/// `s.padEnd(n)`.
fn pad_end(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count >= n {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(n - count))
    }
}

// ─── Info ─────────────────────────────────────────────────────────────────

/// `info(sessionId)`.
async fn info(session_id: &str, json: bool, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    let data = client.get_session_entries(session_id).await?;
    // `const { entries } = ...; if (!data.entries || data.entries.length === 0)`
    let entries: Vec<Value> = data
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        out.log_err(&format!("Session not found: {session_id}"));
        return Err(crate::HANDLED_EXIT.to_string());
    }

    let content = entries
        .iter()
        .find(|e| e["kind"] == "session_info")
        .and_then(|e| e.get("session"))
        .cloned()
        .unwrap_or_default();
    let model = content["model"].as_str().unwrap_or("?");
    let thinking_level = content["thinkingLevel"].as_str().unwrap_or("?");
    let session_name = content["sessionName"].as_str().unwrap_or("(untitled)");
    let cwd = content["cwd"].as_str().unwrap_or("");
    let mut roles: HashMap<String, i64> = HashMap::new();
    let mut tool_calls = 0i64;
    for entry in &entries {
        *roles
            .entry(entry["kind"].as_str().unwrap_or("?").to_owned())
            .or_insert(0) += 1;
        tool_calls += entry["blocks"]
            .as_array()
            .map(|blocks| blocks.iter().filter(|b| b["kind"] == "tool_call").count() as i64)
            .unwrap_or(0);
    }
    let users = roles.get("user").copied().unwrap_or(0);
    let assistants = roles.get("assistant").copied().unwrap_or(0);
    let tools = roles.get("tool").copied().unwrap_or(0);
    // `roles.get("session_info") ?? roles.get("system") ?? 0`
    let system = roles
        .get("session_info")
        .copied()
        .or_else(|| roles.get("system").copied())
        .unwrap_or(0);
    let compacted = roles.get("compaction").copied().unwrap_or(0);

    // `Number(content?.tokens_in ?? 0)` etc.
    let counts = InfoCounts {
        messages: entries.len(),
        users,
        assistants,
        tools,
        system,
        compacted,
        tool_calls,
        tokens_in: content["usage"]
            .get("inputTokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        tokens_out: content["usage"]
            .get("outputTokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        tokens_cache_r: content["usage"]
            .get("cacheReadTokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        tokens_cache_w: content["usage"]
            .get("cacheWriteTokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        total_cost: content["usage"]
            .get("costCny")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    };

    if json {
        let value = info_json(
            session_id,
            &content,
            model,
            thinking_level,
            session_name,
            cwd,
            &counts,
        );
        out.log(&serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?);
        return Ok(());
    }

    out.log(&format!("Session:  {session_id}"));
    out.log(&format!("  Name:        {session_name}"));
    out.log(&format!("  Model:       {model}"));
    out.log(&format!("  Thinking:    {thinking_level}"));
    if !cwd.is_empty() {
        out.log(&format!("  CWD:         {cwd}"));
    }
    out.log(&format!(
        "  Messages:    {} ({} user, {} assistant, {} tool{}{})",
        counts.messages,
        counts.users,
        counts.assistants,
        counts.tools,
        if counts.system > 0 {
            format!(", {} system", counts.system)
        } else {
            String::new()
        },
        if counts.compacted > 0 {
            format!(", {} compacted", counts.compacted)
        } else {
            String::new()
        }
    ));
    out.log(&format!("  Tool calls:  {}", counts.tool_calls));
    if counts.tokens_in + counts.tokens_out > 0 {
        out.log(&format!(
            "  Tokens:      in={} out={}",
            human_tokens(counts.tokens_in),
            human_tokens(counts.tokens_out)
        ));
        if counts.tokens_cache_r + counts.tokens_cache_w > 0 {
            out.log(&format!(
                "  Cache:       r={} w={}",
                human_tokens(counts.tokens_cache_r),
                human_tokens(counts.tokens_cache_w)
            ));
        }
        if counts.total_cost > 0.0 {
            out.log(&format!("  Cost:        ¥{:.6}", counts.total_cost));
        }
    }
    Ok(())
}

/// The machine-readable form of `session info`: identity and working directory
/// at the top level, the raw session metadata alongside the computed stats, so
/// an agent does not have to re-derive either.
fn info_json(
    session_id: &str,
    session: &Value,
    model: &str,
    thinking_level: &str,
    session_name: &str,
    cwd: &str,
    counts: &InfoCounts,
) -> Value {
    json!({
        "id": session_id,
        "sessionName": session_name,
        "model": model,
        "thinkingLevel": thinking_level,
        "cwd": cwd,
        "stats": {
            "messages": counts.messages,
            "userMessages": counts.users,
            "assistantMessages": counts.assistants,
            "toolMessages": counts.tools,
            "systemMessages": counts.system,
            "compactions": counts.compacted,
            "toolCalls": counts.tool_calls,
            "tokens": {
                "input": counts.tokens_in,
                "output": counts.tokens_out,
                "cacheRead": counts.tokens_cache_r,
                "cacheWrite": counts.tokens_cache_w,
            },
            "costCny": counts.total_cost,
        },
        "session": session,
    })
}

/// The aggregate counts `session info` reports, text or JSON.
struct InfoCounts {
    messages: usize,
    users: i64,
    assistants: i64,
    tools: i64,
    system: i64,
    compacted: i64,
    tool_calls: i64,
    tokens_in: i64,
    tokens_out: i64,
    tokens_cache_r: i64,
    tokens_cache_w: i64,
    total_cost: f64,
}

// ─── Rename ──────────────────────────────────────────────────────────────

/// `rename(sessionId, name)`.
async fn rename(session_id: &str, name: &str, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    client.rename_session(session_id, name).await?;
    out.log(&format!("Renamed session {session_id} → \"{name}\""));
    Ok(())
}

// ─── Delete ───────────────────────────────────────────────────────────────

/// `deleteSession(sessionId)`.
async fn delete_session(session_id: &str, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    let data = match client.delete_session(session_id).await {
        Ok(data) => data,
        Err(msg) => {
            // `msg.startsWith("failed to delete") ? msg : \`Failed to delete: ${msg}\``
            let msg = if msg.starts_with("failed to delete") {
                msg
            } else {
                format!("Failed to delete: {msg}")
            };
            out.log_err(&msg);
            return Err(crate::HANDLED_EXIT.to_string());
        }
    };
    // `const { deleted } = await client.deleteSession(sessionId);`
    let deleted = data
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if deleted {
        out.log(&format!("Deleted session {session_id}"));
    } else {
        out.log_err(&format!("Session not found: {session_id}"));
        return Err(crate::HANDLED_EXIT.to_string());
    }
    Ok(())
}

// ─── Set options ─────────────────────────────────────────────────────────

/// Options accepted by `future session set` (all optional; unset means "leave it
/// alone", empty string means "clear" for `--parent`).
#[derive(Debug, Default, PartialEq, Eq)]
struct SessionOptions {
    parent: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    thinking: Option<String>,
    /// `--tools` (a list) and the two disable switches are mutually exclusive.
    tools: Option<ToolsChoice>,
    system_prompt: Option<String>,
    append_system_prompt: Option<String>,
    permission: Option<String>,
    sandbox: Option<String>,
    context_files: Option<bool>,
    auto_compaction: Option<bool>,
    auto_retry: Option<bool>,
    json: bool,
}

/// Which tool set to apply. `--tools a,b` names one; the two switches say "all
/// off" and "built-ins off, keep extensions", which the agent spells as two
/// different commands rather than an empty list.
#[derive(Debug, PartialEq, Eq)]
enum ToolsChoice {
    Enable(Vec<String>),
    DisableAll,
    DisableBuiltin,
}

impl SessionOptions {
    /// Whether any setting was requested.
    fn is_empty(&self) -> bool {
        self.parent.is_none()
            && self.title.is_none()
            && self.cwd.is_none()
            && self.model.is_none()
            && self.thinking.is_none()
            && self.tools.is_none()
            && self.system_prompt.is_none()
            && self.append_system_prompt.is_none()
            && self.permission.is_none()
            && self.sandbox.is_none()
            && self.context_files.is_none()
            && self.auto_compaction.is_none()
            && self.auto_retry.is_none()
    }
}

/// An on/off value. Accepts the same spellings as the booleans elsewhere in the
/// CLI (`true`/`false`, `on`/`off`, `yes`/`no`, `1`/`0`).
fn parse_flag_value(flag: &str, raw: &str) -> Result<bool, String> {
    match raw.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(format!("{flag} takes on|off (got {raw})")),
    }
}

/// Parse `future session set <id>` options (`args` excludes the target id).
///
/// `Err` is an already-reported usage error: unlike `future run`, an unknown
/// option is a hard failure here — silently ignoring `--parent` would record the
/// wrong lineage. `--help` is handled by the caller before the target id is
/// known, so it never reaches this parser.
fn parse_session_options(args: &[String], out: &Output) -> Result<SessionOptions, String> {
    let mut result = SessionOptions::default();
    let mut i = 0;
    // A usage error is reported on stderr and turned into the CLI's handled
    // exit, so callers only ever see `Err(HANDLED_EXIT)`.
    macro_rules! fail {
        ($($arg:tt)*) => {{
            out.log_err(&format!($($arg)*));
            return Err(crate::HANDLED_EXIT.to_string());
        }};
    }
    while i < args.len() {
        let arg = args[i].as_str();
        // Value-taking options first; the rest are switches.
        let takes_value = matches!(
            arg,
            "--parent"
                | "--title"
                | "--cwd"
                | "--model"
                | "--thinking"
                | "--tools"
                | "--system-prompt"
                | "--append-system-prompt"
                | "--permission"
                | "--sandbox"
                | "--context-files"
                | "--auto-compact"
                | "--auto-retry"
        );
        if !takes_value {
            match arg {
                "--json" => result.json = true,
                "--no-tools" => result.tools = Some(ToolsChoice::DisableAll),
                "--no-builtin-tools" => result.tools = Some(ToolsChoice::DisableBuiltin),
                _ => fail!("Unknown option: {arg}"),
            }
            i += 1;
            continue;
        }
        if i + 1 >= args.len() || args[i + 1].starts_with("--") {
            fail!("Missing value for {arg}");
        }
        i += 1;
        let value = args[i].clone();
        match arg {
            "--parent" => result.parent = Some(value),
            "--title" => result.title = Some(value),
            "--cwd" => result.cwd = Some(value),
            "--model" => result.model = Some(value),
            "--thinking" => result.thinking = Some(value),
            "--tools" => {
                let names: Vec<String> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect();
                if names.is_empty() {
                    fail!("--tools needs at least one tool name (use --no-tools for none)");
                }
                result.tools = Some(ToolsChoice::Enable(names));
            }
            "--system-prompt" => result.system_prompt = Some(value),
            "--append-system-prompt" => result.append_system_prompt = Some(value),
            "--permission" => {
                if !crate::commands::run::VALID_PERMISSION_LEVELS.contains(&value.as_str()) {
                    fail!(
                        "Invalid permission level: {value}. Valid: {}",
                        crate::commands::run::VALID_PERMISSION_LEVELS.join(", ")
                    );
                }
                result.permission = Some(value);
            }
            "--sandbox" => {
                if !crate::commands::run::VALID_SANDBOX_TIERS.contains(&value.as_str()) {
                    fail!(
                        "Invalid sandbox tier: {value}. Valid: {}",
                        crate::commands::run::VALID_SANDBOX_TIERS.join(", ")
                    );
                }
                result.sandbox = Some(value);
            }
            _ => {
                let parsed = match parse_flag_value(arg, &value) {
                    Ok(parsed) => parsed,
                    Err(message) => fail!("{message}"),
                };
                match arg {
                    "--context-files" => result.context_files = Some(parsed),
                    "--auto-compact" => result.auto_compaction = Some(parsed),
                    _ => result.auto_retry = Some(parsed),
                }
            }
        }
        i += 1;
    }

    // The three tool options are contradictory, whichever order they arrive in:
    // last-one-wins would make `--no-tools --tools read` mean something the
    // caller almost certainly did not intend.
    let tool_flags = args
        .iter()
        .filter(|a| matches!(a.as_str(), "--tools" | "--no-tools" | "--no-builtin-tools"))
        .count();
    if tool_flags > 1 {
        fail!("--tools, --no-tools and --no-builtin-tools are mutually exclusive");
    }
    if let Some(level) = &result.thinking {
        if !crate::commands::run::VALID_THINKING_LEVELS.contains(&level.as_str()) {
            out.log_err(&format!(
                "Invalid thinking level: {level}. Valid: {}",
                crate::commands::run::VALID_THINKING_LEVELS.join(", ")
            ));
            return Err(crate::HANDLED_EXIT.to_string());
        }
    }
    Ok(result)
}

// ─── Create ──────────────────────────────────────────────────────────────

/// `future session new [--cwd <dir>] [--name <text>] [--json]`.
///
/// The agent records a brand-new session only when it first produces an entry,
/// so a freshly created one is deliberately absent from `session list` until it
/// runs — the help and the output both say so rather than looking broken.
async fn new_session_command(args: &[String], out: &Output) -> Result<(), String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        out.log(NEW_HELP);
        return Ok(());
    }
    let mut cwd = None;
    let mut name = String::new();
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        if !matches!(flag, "--cwd" | "--name") {
            out.log_err(&format!("Unknown option: {flag}"));
            return Err(crate::HANDLED_EXIT.to_string());
        }
        let Some(value) = args.get(index + 1).filter(|v| !v.starts_with("--")) else {
            out.log_err(&format!("Missing value for {flag}"));
            return Err(crate::HANDLED_EXIT.to_string());
        };
        if flag == "--cwd" {
            cwd = Some(value.clone());
        } else {
            name = value.clone();
        }
        index += 2;
    }
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()
            .map_err(|e| format!("cannot determine the current directory: {e}"))?
            .display()
            .to_string(),
    };
    let result = RunClient::new(&grpc_addr())
        .new_named_session(&cwd, &name)
        .await?;
    let session_id = result["sessionId"].as_str().unwrap_or("").to_string();
    if json {
        out.log(
            &serde_json::to_string(&json!({
                "sessionId": session_id,
                "cwd": cwd,
                "sessionName": name,
            }))
            .expect("json serializes"),
        );
        return Ok(());
    }
    out.log(&format!("Created session {session_id}"));
    out.log(&format!("  CWD:  {cwd}"));
    if !name.is_empty() {
        out.log(&format!("  Name: {name}"));
    }
    out.log(
        "\nThe session is not listed until its first run produces an entry; \
         use `future run --session <id> <prompt>` to start it.",
    );
    Ok(())
}

/// `--help` for fork/forks/clone/title/export, which share the
/// `<session-id> [options]` shape.
pub const SUBCOMMAND_HELP: &str = "future session — fork, clone, title and export

Usage:
  future session forks <session-id> [--json]              List forkable user turns
  future session fork <session-id> --entry <entry-id> [--json]
  future session clone <session-id> [--json]
  future session title <session-id> [--lang en|zh] [--apply] [--json]
  future session export <session-id> [--out <path>] [--json]

  forks     The user turns a session can branch at, with their entry ids.
  fork      Branch a new session at one of them (its own history, up to there).
  clone     Branch at the latest settled point — the \"continue from here\"
            copy, with no entry to name.
  title     Ask the session's model for a title. This spends credits and fails
            without a provider; the language defaults to the desktop app's
            titleLanguage. It only prints the title unless --apply is given.
  export    Write the session to a standalone HTML file. The agent writes its
            own temporary copy; --out copies it to a path you choose.

Forks and clones are new sessions: a fork keeps the parent's lineage, and
`--json` prints the child's id as `sessionId`. None of these touches the parent.";

pub const NEW_HELP: &str = "future session new — create a session

Usage:
  future session new [--cwd <dir>] [--name <text>] [--json]

Options:
  --cwd <dir>    Working directory for the new session (default: the current
                 directory). The agent resolves ~ and relative paths.
  --name <text>  Initial title.
  --json         Print {sessionId, cwd, sessionName} instead of prose.

A newly created session has no records, so it does not appear in
`future session list` until its first run produces an entry. Start it with
`future run --session <id> <prompt>`.";

// ─── Fork / clone / title / export ───────────────────────────────────────

/// `future session forks <id> [--json]` — the user turns a session can fork at.
async fn forks(session_id: &str, json: bool, out: &Output) -> Result<(), String> {
    let result = RunClient::new(&grpc_addr())
        .get_fork_messages(session_id)
        .await?;
    let messages = result["messages"].as_array().cloned().unwrap_or_default();
    if json {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "sessionId": session_id,
                "forks": messages,
            }))
            .map_err(|e| e.to_string())?,
        );
        return Ok(());
    }
    if messages.is_empty() {
        out.log(&format!(
            "Session {session_id} has no user turns to fork at."
        ));
        return Ok(());
    }
    out.log(&format!(
        "Session {session_id} can fork at {} user turn(s):\n",
        messages.len()
    ));
    for message in &messages {
        let id = message["id"].as_str().unwrap_or("");
        let text = message["blocks"][0]["text"]
            .as_str()
            .unwrap_or("")
            .replace('\n', " ");
        let preview: String = text.chars().take(70).collect();
        let ellipsis = if text.chars().count() > 70 { "…" } else { "" };
        out.log(&format!("  {}  {preview}{ellipsis}", pad_end(id, 34)));
    }
    out.log(&format!(
        "\nFork one with: future session fork {session_id} --entry <entry-id>"
    ));
    Ok(())
}

/// `future session fork <id> --entry <entry-id> [--json]`.
async fn fork_session(session_id: &str, args: &[String], out: &Output) -> Result<(), String> {
    let mut entry = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        if flag != "--entry" {
            out.log_err(&format!(
                "Unknown option: {flag}; see future session forks {session_id}"
            ));
            return Err(crate::HANDLED_EXIT.to_string());
        }
        let Some(value) = args.get(index + 1).filter(|v| !v.starts_with("--")) else {
            out.log_err("Missing value for --entry");
            return Err(crate::HANDLED_EXIT.to_string());
        };
        entry = Some(value.clone());
        index += 2;
    }
    let Some(entry) = entry else {
        out.log_err(&format!(
            "future session fork requires --entry; list candidates with \
             `future session forks {session_id}`"
        ));
        return Err(crate::HANDLED_EXIT.to_string());
    };
    let result = RunClient::new(&grpc_addr())
        .fork(&entry, session_id)
        .await?;
    let child = result["sessionId"].as_str().unwrap_or("").to_string();
    if json {
        out.log(&serde_json::to_string(&result).map_err(|e| e.to_string())?);
        return Ok(());
    }
    out.log(&format!("Forked session {session_id} at {entry} → {child}"));
    Ok(())
}

/// `future session clone <id> [--json]` — fork at the latest settled point.
async fn clone_session(session_id: &str, json: bool, out: &Output) -> Result<(), String> {
    let result = RunClient::new(&grpc_addr())
        .clone_session(session_id)
        .await?;
    let child = result["sessionId"].as_str().unwrap_or("").to_string();
    if json {
        out.log(&serde_json::to_string(&result).map_err(|e| e.to_string())?);
        return Ok(());
    }
    out.log(&format!("Cloned session {session_id} → {child}"));
    Ok(())
}

/// `future session title <id> [--lang en|zh] [--apply] [--json]`.
///
/// This is a model call: it spends credits and fails without a provider. The
/// language defaults to the desktop app's `titleLanguage` so a terminal title
/// matches the app's, and `--apply` is what renames — generating a title is a
/// read, applying it changes the session.
async fn title_session(session_id: &str, args: &[String], out: &Output) -> Result<(), String> {
    let mut language = None;
    let mut apply = false;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--json" => {
                json = true;
                index += 1;
                continue;
            }
            "--apply" => {
                apply = true;
                index += 1;
                continue;
            }
            "--lang" => {}
            _ => {
                out.log_err(&format!("Unknown option: {flag}"));
                return Err(crate::HANDLED_EXIT.to_string());
            }
        }
        let Some(value) = args.get(index + 1).filter(|v| !v.starts_with("--")) else {
            out.log_err("Missing value for --lang");
            return Err(crate::HANDLED_EXIT.to_string());
        };
        if !matches!(value.as_str(), "en" | "zh") {
            out.log_err("--lang must be en or zh");
            return Err(crate::HANDLED_EXIT.to_string());
        }
        language = Some(value.clone());
        index += 2;
    }
    let language = language.unwrap_or_else(|| {
        // The desktop app owns this preference; fall back to its own default
        // when the store has never been written.
        super::desktop::load()
            .map(|settings| settings.title_language)
            .unwrap_or_else(|_| "en".to_string())
    });
    let client = RunClient::new(&grpc_addr());
    let result = client.generate_session_title(session_id, &language).await?;
    let title = result["title"].as_str().unwrap_or("").to_string();
    if apply {
        client.rename_session(session_id, &title).await?;
    }
    if json {
        out.log(
            &serde_json::to_string(&json!({
                "sessionId": session_id,
                "title": title,
                "model": result["model"],
                "applied": apply,
            }))
            .map_err(|e| e.to_string())?,
        );
        return Ok(());
    }
    out.log(&title);
    if !apply {
        out.log(&format!(
            "\nNot applied. Run `future session rename {session_id} <name>` or add --apply."
        ));
    }
    Ok(())
}

/// `future session export <id> [--out <path>] [--json]` — the agent writes the
/// HTML to its own temporary path; `--out` copies it somewhere durable.
async fn export_session(session_id: &str, args: &[String], out: &Output) -> Result<(), String> {
    let mut destination = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        if flag != "--out" {
            out.log_err(&format!("Unknown option: {flag}"));
            return Err(crate::HANDLED_EXIT.to_string());
        }
        let Some(value) = args.get(index + 1).filter(|v| !v.starts_with("--")) else {
            out.log_err("Missing value for --out");
            return Err(crate::HANDLED_EXIT.to_string());
        };
        destination = Some(value.clone());
        index += 2;
    }
    let result = RunClient::new(&grpc_addr()).export_html(session_id).await?;
    let source = result["path"].as_str().unwrap_or("").to_string();
    if source.is_empty() {
        out.log_err("The agent reported no export path.");
        return Err(crate::HANDLED_EXIT.to_string());
    }
    let final_path = match &destination {
        Some(destination) => {
            std::fs::copy(&source, destination)
                .map_err(|e| format!("cannot write {destination}: {e}"))?;
            destination.clone()
        }
        None => source.clone(),
    };
    if json {
        out.log(
            &serde_json::to_string(&json!({
                "sessionId": session_id,
                "path": final_path,
            }))
            .expect("json serializes"),
        );
        return Ok(());
    }
    out.log(&format!("Exported session {session_id} to {final_path}"));
    Ok(())
}

// ─── Set ─────────────────────────────────────────────────────────────────

/// Map the agent's missing-session failures onto the CLI's own wording
/// (`session info` prints `Session not found: <id>`).
pub(super) fn missing_session_error(err: String, session_id: &str) -> String {
    if err.starts_with("session not found") {
        format!("Session not found: {session_id}")
    } else if let Some(parent) = err.strip_prefix("parent session not found: ") {
        format!("Session not found: {parent}")
    } else {
        err
    }
}

/// `future session set <id> [options]` — change settings on an existing
/// session. Each option is applied on its own, so one failure (e.g. a model the
/// agent no longer serves) does not roll back the others; every failure is
/// reported and the command exits 1.
async fn set_session_command(args: &[String], out: &Output) -> Result<(), String> {
    // `set --help` carries no target id, so help is resolved before the target
    // is used (`sess-1 --help` shows it too — `--help` is never a value here).
    if args.iter().any(|a| a == "--help" || a == "-h") {
        out.log(SESSION_SET_HELP);
        return Ok(());
    }
    let Some(target_id) = args.first() else {
        out.log_err("Usage: future session set <session-id> [options]");
        return Err(crate::HANDLED_EXIT.to_string());
    };
    let parsed = parse_session_options(&args[1..], out)?;
    if parsed.is_empty() {
        out.log_err(
            "Nothing to set. Pass at least one of --parent, --title, --cwd, --model, --thinking",
        );
        return Err(crate::HANDLED_EXIT.to_string());
    }
    let target_id = target_id.clone();
    let client = RunClient::new(&grpc_addr());

    // Resolve the target once, before mutating anything: the agent's own
    // "session not found" message is about `new_session`, which makes no sense
    // here, and every later call would report the same failure.
    if let Err(err) = client.get_state(Some(&target_id)).await {
        let err = missing_session_error(err, &target_id);
        out.log_err(&err);
        return Err(crate::HANDLED_EXIT.to_string());
    }

    // No client-side parent probe: the agent accepts a parent that exists only
    // in memory (created, not yet prompted), which `list_sessions` cannot see.
    // `--parent` is applied first and aborts the rest — a parent is a reference
    // to another session, so a typo there makes the whole update meaningless.
    let mut applied: Vec<(&str, String)> = Vec::new();
    let mut failures: Vec<(&str, String)> = Vec::new();

    if let Some(parent) = &parsed.parent {
        if !parent.is_empty() && parent == &target_id {
            out.log_err("A session cannot be its own parent");
            return Err(crate::HANDLED_EXIT.to_string());
        }
        match client.set_parent_session(&target_id, parent).await {
            Ok(()) => applied.push((
                "parent",
                if parent.is_empty() {
                    "(none)".to_string()
                } else {
                    parent.clone()
                },
            )),
            Err(err) => {
                out.log_err(&missing_session_error(err, &target_id));
                return Err(crate::HANDLED_EXIT.to_string());
            }
        }
    }
    if let Some(title) = &parsed.title {
        match client.rename_session(&target_id, title).await {
            Ok(()) => applied.push(("title", title.clone())),
            Err(err) => failures.push(("title", missing_session_error(err, &target_id))),
        }
    }
    // The cwd is an agent-side property and the workspace a desktop-side record;
    // changing one has always filed the conversation under the other when the
    // desktop app was there to notice. Doing it here too is what makes a cwd
    // changed from the terminal land the same way with the app closed — and it
    // is the same code (`future_app_workspaces::file_session_for_cwd`), so the
    // two writers cannot disagree about where a directory's conversation goes.
    let mut notes: Vec<String> = Vec::new();
    if let Some(cwd) = &parsed.cwd {
        match client.set_cwd(cwd, &target_id).await {
            Ok(()) => {
                applied.push(("cwd", cwd.clone()));
                notes.extend(crate::commands::workspace::file_session_cwd(&target_id, cwd).note());
            }
            Err(err) => failures.push(("cwd", missing_session_error(err, &target_id))),
        }
    }
    if let Some(model) = &parsed.model {
        match client.set_model(model, &target_id).await {
            Ok(()) => applied.push(("model", model.clone())),
            Err(err) => failures.push(("model", missing_session_error(err, &target_id))),
        }
    }
    if let Some(level) = &parsed.thinking {
        match client.set_thinking_level(level, &target_id).await {
            Ok(()) => applied.push(("thinkingLevel", level.clone())),
            Err(err) => failures.push(("thinkingLevel", missing_session_error(err, &target_id))),
        }
    }
    // The remaining options are session-scoped switches the agent applies to
    // its live session; unlike the ones above they are not persisted into the
    // session record, so they hold for this agent's lifetime.
    if let Some(choice) = &parsed.tools {
        let outcome = match choice {
            ToolsChoice::Enable(names) => client
                .set_tools(names, &target_id)
                .await
                .map(|()| names.join(",")),
            ToolsChoice::DisableAll => client
                .disable_tools(&target_id)
                .await
                .map(|()| "none".into()),
            ToolsChoice::DisableBuiltin => client
                .disable_builtin_tools(&target_id)
                .await
                .map(|()| "builtin disabled".into()),
        };
        match outcome {
            Ok(description) => applied.push(("tools", description)),
            Err(err) => failures.push(("tools", missing_session_error(err, &target_id))),
        }
    }
    if let Some(prompt) = &parsed.system_prompt {
        match client.set_system_prompt(prompt, &target_id).await {
            Ok(()) => applied.push(("systemPrompt", "(replaced)".into())),
            Err(err) => failures.push(("systemPrompt", missing_session_error(err, &target_id))),
        }
    }
    if let Some(prompt) = &parsed.append_system_prompt {
        match client.append_system_prompt(prompt, &target_id).await {
            Ok(()) => applied.push(("appendSystemPrompt", "(appended)".into())),
            Err(err) => {
                failures.push(("appendSystemPrompt", missing_session_error(err, &target_id)))
            }
        }
    }
    if let Some(level) = &parsed.permission {
        match client.set_permission_level(level, &target_id).await {
            Ok(()) => applied.push(("permissionLevel", level.clone())),
            Err(err) => failures.push(("permissionLevel", missing_session_error(err, &target_id))),
        }
    }
    if let Some(tier) = &parsed.sandbox {
        match client.set_sandbox_policy(tier, &target_id).await {
            Ok(_) => applied.push(("sandboxTier", tier.clone())),
            Err(err) => failures.push(("sandboxTier", missing_session_error(err, &target_id))),
        }
    }
    if let Some(enabled) = parsed.context_files {
        match client.set_context_files(enabled, &target_id).await {
            Ok(()) => applied.push(("contextFiles", on_off(enabled))),
            Err(err) => failures.push(("contextFiles", missing_session_error(err, &target_id))),
        }
    }
    if let Some(enabled) = parsed.auto_compaction {
        match client.set_auto_compaction(enabled, &target_id).await {
            Ok(()) => applied.push(("autoCompaction", on_off(enabled))),
            Err(err) => failures.push(("autoCompaction", missing_session_error(err, &target_id))),
        }
    }
    if let Some(enabled) = parsed.auto_retry {
        match client.set_auto_retry(enabled, &target_id).await {
            Ok(()) => applied.push(("autoRetry", on_off(enabled))),
            Err(err) => failures.push(("autoRetry", missing_session_error(err, &target_id))),
        }
    }

    if parsed.json {
        // Same camelCase field names as `session info`.
        let updated: serde_json::Map<String, Value> = applied
            .iter()
            .map(|(key, value)| (key.to_string(), Value::String(value.clone())))
            .collect();
        let failed: serde_json::Map<String, Value> = failures
            .iter()
            .map(|(key, value)| (key.to_string(), Value::String(value.clone())))
            .collect();
        let mut document = json!({
            "sessionId": target_id,
            "updated": updated,
            "failed": failed,
        });
        if !notes.is_empty() {
            document["notes"] = json!(notes);
        }
        out.log(&serde_json::to_string(&document).expect("json serializes"));
    } else {
        if !applied.is_empty() {
            out.log(&format!("Updated session {target_id}"));
        }
        for (key, value) in &applied {
            out.log(&format!("  {}{value}", option_label(key)));
        }
        for note in &notes {
            out.log(&format!("  {note}"));
        }
    }

    if failures.is_empty() {
        return Ok(());
    }
    for (key, err) in &failures {
        out.log_err(&format!("Failed to set {}: {err}", key));
    }
    Err(crate::HANDLED_EXIT.to_string())
}

/// `on`/`off` for a boolean setting, matching the CLI's boolean spellings.
fn on_off(enabled: bool) -> String {
    if enabled {
        "on".to_string()
    } else {
        "off".to_string()
    }
}

/// Padded label matching the `session info` layout.
fn option_label(key: &str) -> String {
    let label = match key {
        "parent" => "Parent",
        "title" => "Title",
        "cwd" => "CWD",
        "model" => "Model",
        "thinkingLevel" => "Thinking",
        "tools" => "Tools",
        "systemPrompt" => "SystemPro",
        "appendSystemPrompt" => "AppendSys",
        "permissionLevel" => "Permission",
        "sandboxTier" => "Sandbox",
        "contextFiles" => "CtxFiles",
        "autoCompaction" => "AutoCompact",
        "autoRetry" => "AutoRetry",
        _ => "Setting",
    };
    pad_end(&format!("{label}:"), 14)
}

// ─── Entry ────────────────────────────────────────────────────────────────

/// `session(subcommand, args)`.
pub async fn session(
    subcommand: Option<&str>,
    args: &[String],
    out: &Output,
) -> Result<(), String> {
    // `if (subcommand === "--help" || subcommand === "-h" || !subcommand)`
    let Some(subcommand) = subcommand else {
        help(out);
        return Ok(());
    };
    if subcommand == "--help" || subcommand == "-h" {
        help(out);
        return Ok(());
    }

    if subcommand == "compact" {
        return super::session_compact::run(args, out).await;
    }

    if subcommand == "history" {
        return super::session_history::run(args, out).await;
    }

    if subcommand == "transcript" {
        return super::session_transcript::run(args, out).await;
    }

    if subcommand == "status" {
        return super::session_status::run(args, out).await;
    }

    // Stop/approve actions own their own argument shapes (`abort <id>` takes no
    // target, `approve <id> <request-id>` takes two), so they parse here rather
    // than through the `<session-id>` path below.
    if let Some((_, action)) = super::session_control::SUBCOMMANDS
        .iter()
        .find(|(name, _)| *name == subcommand)
    {
        if args.iter().any(|a| a == "--help" || a == "-h") {
            out.log(super::session_control::HELP);
            return Ok(());
        }
        return super::session_control::run(*action, args, out).await;
    }

    if subcommand == "new" {
        return new_session_command(args, out).await;
    }

    if subcommand == "list" {
        list_sessions(args.iter().any(|a| a == "--json"), out).await?;
        return Ok(());
    }

    // `set` takes its options after the target id, so it does its own argument
    // handling instead of the plain `<session-id>` path below.
    if subcommand == "set" {
        return set_session_command(args, out).await;
    }

    // These also carry options after the target id, so they parse their own
    // arguments rather than going through the bare `<session-id>` path.
    if matches!(subcommand, "fork" | "forks" | "clone" | "title" | "export") {
        // `--help` is resolved before the target id, so `session fork --help`
        // works as well as the target-bearing form.
        if args.iter().any(|a| a == "--help" || a == "-h") {
            out.log(SUBCOMMAND_HELP);
            return Ok(());
        }
        let Some(target) = args.first().filter(|a| !a.starts_with("--")) else {
            out.log_err(&format!("Usage: future session {subcommand} <session-id>"));
            return Err(crate::HANDLED_EXIT.to_string());
        };
        let rest = &args[1..];
        return match subcommand {
            "fork" => fork_session(target, rest, out).await,
            "forks" => forks(target, rest.iter().any(|a| a == "--json"), out).await,
            "clone" => clone_session(target, rest.iter().any(|a| a == "--json"), out).await,
            "title" => title_session(target, rest, out).await,
            _ => export_session(target, rest, out).await,
        };
    }

    // `const targetId = args[0]; if (!targetId)`
    let target_id = args.first().cloned().unwrap_or_default();
    if target_id.is_empty() {
        out.log_err(&format!(
            "Usage: future session {subcommand} <session-id>{}",
            if subcommand == "rename" {
                " <name>"
            } else {
                ""
            }
        ));
        return Err(crate::HANDLED_EXIT.to_string());
    }

    match subcommand {
        "info" => {
            let json = args[1..].iter().any(|arg| arg == "--json");
            info(&target_id, json, out).await?;
        }
        "rename" => {
            // `const name = args.slice(1).join(" ");`
            let name = args[1..].join(" ");
            if name.is_empty() {
                out.log_err("Usage: future session rename <session-id> <name>");
                return Err(crate::HANDLED_EXIT.to_string());
            }
            rename(&target_id, &name, out).await?;
        }
        "delete" => {
            delete_session(&target_id, out).await?;
        }
        _ => {
            out.log_err(&format!("Unknown command: {subcommand}"));
            help(out);
            return Err(crate::HANDLED_EXIT.to_string());
        }
    }
    Ok(())
}

/// Small helper for the JSON list output (avoids a json! round-trip).
fn json_value(sessions: &[Value]) -> Value {
    Value::Object(
        [("sessions".to_string(), Value::Array(sessions.to_vec()))]
            .into_iter()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Titles are still never generated *automatically*: the model is asked
    /// only when a caller names a session explicitly, and the result is a
    /// suggestion unless `--apply` is passed. The old form — `session title
    /// <id> summary`, which read as "summarize this for me" — stays rejected.
    #[tokio::test]
    async fn title_is_explicit_and_never_automatic() {
        let (out, cap) = Output::memory();
        assert!(
            session(Some("title"), &["s1".into(), "summary".into()], &out)
                .await
                .is_err()
        );
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Unknown option: summary"), "{stderr}");
        assert!(SESSION_HELP.contains("future session title <id>"));

        // And a missing target is a usage error, not a silent no-op.
        let (out, _cap) = Output::memory();
        assert!(session(Some("title"), &[], &out).await.is_err());
    }

    #[test]
    fn truncate_behavior() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("", 5), "");
        // UTF-8 safe.
        assert_eq!(truncate("你好世界", 3), "你好…");
    }

    #[test]
    fn human_tokens_behavior() {
        assert_eq!(human_tokens(2_500_000), "2.5M");
        assert_eq!(human_tokens(1_000_000), "1.0M");
        assert_eq!(human_tokens(128_000), "128K");
        assert_eq!(human_tokens(999), "999");
        assert_eq!(human_tokens(0), "0");
    }

    #[test]
    fn ago_behavior() {
        let now = 1_000_000_000_000i64; // fixed epoch (2001-09-09)
                                        // ~30s ago.
        let iso = format_ts(now - 30_000);
        assert_eq!(ago(&iso, now), "just now");
        // 5m ago.
        let iso = format_ts(now - 5 * 60_000);
        assert_eq!(ago(&iso, now), "5m ago");
        // 3h ago.
        let iso = format_ts(now - 3 * 3_600_000);
        assert_eq!(ago(&iso, now), "3h ago");
        // 2d ago.
        let iso = format_ts(now - 2 * 86_400_000);
        assert_eq!(ago(&iso, now), "2d ago");
        // Unparseable → treated as epoch → far past.
        assert_eq!(ago("bogus", now), "11574d ago");
    }

    /// Render `now - delta` as local "YYYY-MM-DD HH:MM:SS".
    fn format_ts(ms: i64) -> String {
        use chrono::TimeZone;
        chrono::Local
            .timestamp_millis_opt(ms)
            .single()
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    #[test]
    fn parse_timestamp_rfc3339() {
        // "2026-08-06T12:00:00Z" — 12:00 UTC on 2026-08-06.
        let ms = parse_timestamp_ms("2026-08-06T12:00:00Z");
        assert_eq!(ms, 1786017600000);
    }

    #[tokio::test]
    async fn session_list_agent_down_propagates() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, cap) = Output::memory();
        let result = list_sessions(false, &out).await;
        assert!(result.is_err());
        assert_eq!(
            String::from_utf8(cap.out.lock().unwrap().clone()).unwrap(),
            ""
        );
    }

    #[tokio::test]
    async fn session_unknown_subcommand_usage_and_help() {
        // TS prints the missing-target usage BEFORE dispatching subcommands,
        // so `bogus` with no target shows the usage line.
        let (out, cap) = Output::memory();
        let result = session(Some("bogus"), &[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future session bogus <session-id>\n");

        // With a target id, the unknown-command branch fires: stderr error +
        // help on stdout.
        let (out, cap) = Output::memory();
        let result = session(Some("bogus"), &["sess-1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Unknown command: bogus\n");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("future session — manage agent sessions"));
    }

    #[tokio::test]
    async fn session_missing_target_id() {
        let (out, cap) = Output::memory();
        let result = session(Some("info"), &[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future session info <session-id>\n");
    }

    #[tokio::test]
    async fn session_rename_missing_name() {
        let (out, cap) = Output::memory();
        let result = session(Some("rename"), &["sess-1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future session rename <session-id> <name>\n");
    }

    // ── Mock-agent backed flows ─────────────────────────────────────

    /// Point FUTURE_AGENT_GRPC_ADDR at a spawned mock (caller holds ENV_LOCK).
    async fn mock_env(
        agent: crate::test_server::MockAgent,
    ) -> (crate::test_server::MockAgent, crate::test_env::EnvGuard) {
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        // `session set --cwd` files the conversation in the desktop app's own
        // store (`~/.future/app/app.db`), so every one of these tests must run
        // against a throwaway home: without this they would read — and, on a
        // machine where the desktop app has imported the session, *write* — the
        // real app database.
        isolate_home();
        (agent, env)
    }

    /// Point HOME at a fresh temporary directory for the rest of this test,
    /// keeping the directory alive in a thread-local so the guard's drop leaves
    /// nothing behind and no test deletes a directory another one is using.
    fn isolate_home() {
        use std::cell::RefCell;
        thread_local! {
            static HOMES: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
        }
        let home = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", home.path());
        HOMES.with(|homes| homes.borrow_mut().push(home));
    }

    #[tokio::test]
    async fn list_empty_sessions() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond("list_sessions", "{\"sessions\":[]}");
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("list"), &[], &out).await.expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "No sessions found.\n");
    }

    #[tokio::test]
    async fn list_json_passthrough() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "list_sessions",
            "{\"sessions\":[{\"id\":\"s1\",\"sessionName\":\"one\"}]}",
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("list"), &["--json".to_string()], &out)
            .await
            .expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessions"][0]["id"], "s1");
    }

    #[tokio::test]
    async fn list_table_rendering_and_sorting() {
        let _guard = crate::test_env::lock_env().await;
        let long_model = "a".repeat(30);
        let body = format!(
            "{{\"sessions\":[\
                {{\"id\":\"untitled-old\",\"updatedAtMs\":1000}},\
                {{\"id\":\"named\",\"sessionName\":\"My Session\",\"model\":\"k3\",\"queryCount\":3,\"updatedAtMs\":3000}},\
                {{\"id\":\"first-msg\",\"firstMessage\":\"hello world this is the first message of the session\",\"model\":\"{long_model}\",\"queryCount\":0,\"updatedAtMs\":2000}}\
            ]}}"
        );
        let agent = crate::test_server::MockAgent::respond("list_sessions", &body);
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("list"), &[], &out).await.expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // Header + rule + 3 rows + count.
        assert!(stdout.contains("SESSION ID"), "stdout: {stdout}");
        assert!(stdout.contains("QUERIES"), "stdout: {stdout}");
        // Sorted by updated_at desc: named (2099) before first-msg (2098).
        let named = stdout.find("My Session").expect("named row");
        let first_msg = stdout.find("hello world").expect("first-msg row");
        assert!(named < first_msg);
        // Untitled row + zero query count → "—", long model truncated.
        assert!(stdout.contains("(untitled)"), "stdout: {stdout}");
        assert!(
            stdout.contains(&format!("{}…", "a".repeat(27))),
            "stdout: {stdout}"
        );
        assert!(stdout.ends_with("\n3 sessions.\n"), "stdout: {stdout}");
        // A row with a real query count renders the number (single line: a
        // short-circuit chain split across lines leaves the tail uncovered).
        let has_three =
            stdout.contains(" 3\n") || stdout.contains(" 3 \n") || stdout.contains("3\n");
        assert!(has_three, "stdout: {stdout}");
    }

    #[tokio::test]
    async fn list_agent_down_propagates_raw_error() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, _cap) = Output::memory();
        // Not wrapped in HANDLED_EXIT — the transport error propagates.
        let err = session(Some("list"), &[], &out).await.unwrap_err();
        assert_ne!(err, crate::HANDLED_EXIT);
    }

    #[tokio::test]
    async fn info_full_rendering() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            r#"{"entries":[{"kind":"session_info","role":"system","session":{"sessionName":"Named","cwd":"/work","usage":{"inputTokens":2500000,"outputTokens":128000,"cacheReadTokens":5000,"cacheWriteTokens":1500,"costCny":0.012345}}},{"kind":"user","role":"user"},{"kind":"assistant","role":"assistant","blocks":[{"kind":"tool_call"},{"kind":"tool_call"}]},{"kind":"assistant","role":"assistant"},{"kind":"tool","role":"tool"},{"kind":"compaction"},{}]}"#,
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["sess-1".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Session:  sess-1"), "stdout: {stdout}");
        assert!(stdout.contains("Name:        Named"));
        // model falls back to "?" (no model anywhere), thinking "?" too.
        assert!(stdout.contains("Model:       ?"), "stdout: {stdout}");
        assert!(stdout.contains("Thinking:    ?"), "stdout: {stdout}");
        assert!(stdout.contains("CWD:         /work"));
        // 7 entries: 1 user, 2 assistant, 1 tool, 1 system, 1 compaction,
        // and the {} entry counts as "?".
        assert!(
            stdout.contains("Messages:    7 (1 user, 2 assistant, 1 tool, 1 system, 1 compacted)"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("Tool calls:  2"));
        assert!(
            stdout.contains("Tokens:      in=2.5M out=128K"),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("Cache:       r=5K w=2K"),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("Cost:        ¥0.012345"),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn info_json_reports_cwd_and_stats() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            r#"{"entries":[{"kind":"session_info","role":"system","session":{"sessionName":"Named","cwd":"/work","model":"m1","thinkingLevel":"high","usage":{"inputTokens":10,"outputTokens":2,"costCny":0.5}}},{"kind":"user","role":"user"},{"kind":"assistant","role":"assistant","blocks":[{"kind":"tool_call"}]}]}"#,
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(
            Some("info"),
            &["sess-1".to_string(), "--json".to_string()],
            &out,
        )
        .await
        .expect("info --json");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["id"], "sess-1");
        assert_eq!(parsed["cwd"], "/work");
        assert_eq!(parsed["model"], "m1");
        assert_eq!(parsed["thinkingLevel"], "high");
        assert_eq!(parsed["stats"]["messages"], json!(3));
        assert_eq!(parsed["stats"]["toolCalls"], json!(1));
        assert_eq!(parsed["stats"]["tokens"]["input"], json!(10));
        assert_eq!(parsed["stats"]["costCny"], json!(0.5));
        assert_eq!(parsed["session"]["cwd"], json!("/work"));
    }

    #[tokio::test]
    async fn info_minimal_entry_and_model_fallbacks() {
        let _guard = crate::test_env::lock_env().await;
        // Model from the entry itself, thinking from content; no cwd/tokens.
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            r#"{"entries":[{"kind":"session_info","role":"system","session":{"model":"m-entry","thinkingLevel":"high"}}]}"#,
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["s1".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Model:       m-entry"), "stdout: {stdout}");
        assert!(stdout.contains("Thinking:    high"), "stdout: {stdout}");
        assert!(
            stdout.contains("Name:        (untitled)"),
            "stdout: {stdout}"
        );
        assert!(!stdout.contains("CWD:"), "stdout: {stdout}");
        assert!(!stdout.contains("Tokens:"), "stdout: {stdout}");
        assert!(!stdout.contains("Cache:"), "stdout: {stdout}");
        assert!(!stdout.contains("Cost:"), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn info_session_not_found() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "list_sessions",
            "{\"sessions\":[{\"id\":\"other\"}]}",
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("info"), &["ghost".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Session not found: ghost\n");
    }

    #[tokio::test]
    async fn info_and_rename_rpc_failures_propagate() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.fail_types.insert("get_session_entries".into());
        agent.fail_types.insert("set_session_name".into());
        let (_agent, _env) = mock_env(agent).await;
        let (out, _cap) = Output::memory();
        let result = session(Some("info"), &["s1".to_string()], &out).await;
        assert!(result.is_err());
        let result = session(Some("rename"), &["s1".to_string(), "x".to_string()], &out).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn info_sparse_entries_take_default_arms() {
        let _guard = crate::test_env::lock_env().await;
        // No system-role entry, non-array tool_calls, zero cost: all the
        // default/false arms in the info renderer.
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            "{\"entries\":[{\"role\":\"user\",\"tool_calls\":5,\"content\":null}]}",
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["s1".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Model:       ?"), "stdout: {stdout}");
        assert!(stdout.contains("user"), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn rename_success() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::default();
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(
            Some("rename"),
            &["sess-1".to_string(), "new".to_string(), "name".to_string()],
            &out,
        )
        .await
        .expect("rename");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "Renamed session sess-1 → \"new name\"\n");
        let seen = agent.seen_of("set_session_name");
        assert_eq!(seen[0].name, "new name");
        assert_eq!(seen[0].session_id, "sess-1");
    }

    #[tokio::test]
    async fn delete_outcomes() {
        let _guard = crate::test_env::lock_env().await;
        // deleted: true.
        let agent = crate::test_server::MockAgent::respond("delete_session", "{\"deleted\":true}");
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("delete"), &["s1".to_string()], &out)
            .await
            .expect("delete");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "Deleted session s1\n");
        drop(_env);

        // deleted: false → not found.
        let agent = crate::test_server::MockAgent::respond("delete_session", "{\"deleted\":false}");
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("delete"), &["ghost".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Session not found: ghost\n");
        drop(_env);

        // RPC failure → "Failed to delete: <msg>".
        let mut agent = crate::test_server::MockAgent::default();
        agent.fail_types.insert("delete_session".into());
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("delete"), &["s1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Failed to delete: boom\n");
        drop(_env);

        // Failure already prefixed passes through untouched.
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .fail_with
            .insert("delete_session".into(), "failed to delete: busy".into());
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("delete"), &["s1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "failed to delete: busy\n");
    }

    #[test]
    fn parse_timestamp_ms_naive_local() {
        // Naive "YYYY-MM-DD HH:MM:SS" parses in LOCAL time — compare against
        // the UTC epoch allowing any timezone offset (±14h max).
        let naive = parse_timestamp_ms("2001-09-09 01:46:40");
        let utc = parse_timestamp_ms("2001-09-09T01:46:40Z");
        assert!(naive != 0);
        assert!(
            (naive - utc).abs() <= 14 * 3_600_000,
            "naive={naive} utc={utc}"
        );
        assert_eq!(parse_timestamp_ms("junk"), 0);
    }

    #[test]
    fn json_value_wraps_sessions() {
        let value = json_value(&[serde_json::json!({"id": "s1"})]);
        assert_eq!(value["sessions"][0]["id"], "s1");
        assert!(json_value(&[])["sessions"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn session_no_subcommand_prints_help() {
        let (out, cap) = Output::memory();
        session(None, &[], &out).await.expect("ok");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, format!("{}\n", SESSION_HELP));
    }

    // ── `session set` ───────────────────────────────────────────────

    #[tokio::test]
    async fn set_applies_every_option_to_an_existing_session() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .responses
            .insert("get_state".into(), "{\"model\":\"m0\"}".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--parent".into(),
                "parent-1".into(),
                "--title".into(),
                "Renamed".into(),
                "--cwd".into(),
                "/work".into(),
                "--model".into(),
                "m1".into(),
                "--thinking".into(),
                "high".into(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // The label column widened from 10 to 14 to fit `--auto-compact` and
        // friends; the values and their order are unchanged.
        assert_eq!(
            stdout,
            "Updated session sess-1\n  Parent:       parent-1\n  Title:        Renamed\n  CWD:          /work\n  Model:        m1\n  Thinking:     high\n"
        );

        // Target resolved once, then one command per option — each scoped to
        // the target session.
        let state = agent.seen_of("get_state");
        assert_eq!(state.len(), 1);
        assert_eq!(state[0].session_id, "sess-1");
        for (cmd, session_id) in [
            ("set_parent_session", "sess-1"),
            ("set_session_name", "sess-1"),
            ("set_cwd", "sess-1"),
            ("set_model", "sess-1"),
            ("set_thinking_level", "sess-1"),
        ] {
            let seen = agent.seen_of(cmd);
            assert_eq!(seen.len(), 1, "{cmd}");
            assert_eq!(seen[0].session_id, session_id, "{cmd}");
        }
        assert_eq!(
            agent.seen_of("set_parent_session")[0].parent_session,
            "parent-1"
        );
        assert_eq!(agent.seen_of("set_session_name")[0].name, "Renamed");
        assert_eq!(agent.seen_of("set_cwd")[0].cwd, "/work");
        assert_eq!(agent.seen_of("set_model")[0].model_id, "m1");
        assert_eq!(agent.seen_of("set_thinking_level")[0].level, "high");
    }

    /// One option failing does not roll back the others: the title, cwd and
    /// thinking-level failures land in `failed` (with the agent's
    /// "session not found" message mapped to the target id) while the model
    /// that succeeded stays in `updated`, the JSON report carries both maps,
    /// and the command exits 1.
    #[tokio::test]
    async fn set_reports_each_failed_option_and_keeps_the_successful_ones() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        agent.fail_types.insert("set_session_name".into());
        agent.fail_types.insert("set_thinking_level".into());
        agent
            .fail_with
            .insert("set_cwd".into(), "session not found".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &[
                "sess-1".into(),
                "--title".into(),
                "T".into(),
                "--cwd".into(),
                "/work".into(),
                "--model".into(),
                "m1".into(),
                "--thinking".into(),
                "high".into(),
                "--json".into(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));

        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let doc: Value = serde_json::from_str(&stdout).expect("json report");
        assert_eq!(doc["sessionId"], "sess-1");
        assert_eq!(doc["updated"]["model"], "m1", "{doc}");
        assert_eq!(doc["failed"]["title"], "boom", "{doc}");
        assert_eq!(doc["failed"]["cwd"], "Session not found: sess-1", "{doc}");
        assert_eq!(doc["failed"]["thinkingLevel"], "boom", "{doc}");
        assert!(
            doc["updated"].get("title").is_none() && doc["updated"].get("cwd").is_none(),
            "a failed option is not reported as applied: {doc}"
        );

        // Every option was still attempted (a failure is per-option, not fatal).
        for cmd in [
            "set_session_name",
            "set_cwd",
            "set_model",
            "set_thinking_level",
        ] {
            assert_eq!(agent.seen_of(cmd).len(), 1, "{cmd}");
        }
        // The human report names each failure on stderr, one line per option.
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        for expected in [
            "Failed to set title: boom",
            "Failed to set cwd: Session not found: sess-1",
            "Failed to set thinkingLevel: boom",
        ] {
            assert!(
                stderr.contains(expected),
                "missing {expected:?} in {stderr}"
            );
        }
        assert!(
            !stderr.contains("Failed to set model"),
            "the successful option is not reported as failed: {stderr}"
        );
    }

    #[tokio::test]
    async fn set_only_touches_the_options_given() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &["sess-1".into(), "--title".into(), "Only".into()],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "Updated session sess-1\n  Title:        Only\n");
        for cmd in [
            "set_parent_session",
            "set_cwd",
            "set_model",
            "set_thinking_level",
        ] {
            assert!(agent.seen_of(cmd).is_empty(), "{cmd}");
        }
        // No --parent → no session list probe.
        assert!(agent.seen_of("list_sessions").is_empty());
    }

    /// `--cwd` also files the conversation in the desktop app's store: the
    /// workspace record follows the new directory, the same way the desktop app
    /// files it when it observes the change itself. A cwd changed here with the
    /// app closed must not wait for the app to be opened later.
    #[tokio::test]
    async fn set_cwd_files_the_conversation_in_the_desktop_store() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (_agent, _env) = mock_env(agent).await;

        let directory = tempfile::tempdir().expect("project directory");
        seed_desktop_thread("sess-1", "thread-1");
        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--cwd".into(),
                directory.path().display().to_string(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("workspace: created"), "{stdout}");

        // The workspace is the app's own row, and the thread now points at it.
        let conn = open_app_db();
        let workspace = future_app_workspaces::list(&conn).expect("list").remove(0);
        assert_eq!(workspace.kind, "user");
        assert_eq!(
            workspace.path,
            directory
                .path()
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(thread_workspace_id(&conn, "thread-1"), workspace.id);

        // A second session pointing at the same directory reuses that row.
        // Same home: a second `mock_env` would point HOME at a fresh directory
        // and this would be a different desktop store.
        seed_desktop_thread("sess-2", "thread-2");
        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-2".into(),
                "--cwd".into(),
                directory.path().display().to_string(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("workspace: filed under"), "{stdout}");
        let conn = open_app_db();
        assert_eq!(future_app_workspaces::list(&conn).expect("list").len(), 1);
        assert_eq!(thread_workspace_id(&conn, "thread-2"), workspace.id);
    }

    /// A directory that does not exist is reported as a note, not as a failed
    /// `--cwd`: the agent-side change really did happen, and the fix is the one
    /// the note names.
    #[tokio::test]
    async fn set_cwd_reports_a_directory_it_cannot_file() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (_agent, _env) = mock_env(agent).await;
        seed_desktop_thread("sess-1", "thread-1");

        let missing = tempfile::tempdir()
            .expect("parent")
            .path()
            .join("not-there");
        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--json".into(),
                "--cwd".into(),
                missing.display().to_string(),
            ],
            &out,
        )
        .await
        .expect("the cwd change itself succeeded");
        let parsed: Value =
            serde_json::from_str(&String::from_utf8(cap.out.lock().unwrap().clone()).unwrap())
                .expect("json");
        assert_eq!(parsed["updated"]["cwd"], missing.display().to_string());
        let notes = parsed["notes"].as_array().expect("notes");
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].as_str().unwrap().contains("does not exist"),
            "got: {notes:?}"
        );
        assert!(
            notes[0].as_str().unwrap().contains("future workspace add"),
            "the note names the fix: {notes:?}"
        );
    }

    /// With no desktop app store there is nothing to file and nothing to say:
    /// the plain `--cwd` output is unchanged on a machine that has never run
    /// the app.
    #[tokio::test]
    async fn set_cwd_says_nothing_without_a_desktop_store() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (_agent, _env) = mock_env(agent).await;

        let directory = tempfile::tempdir().expect("directory");
        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--cwd".into(),
                directory.path().display().to_string(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(
            stdout,
            format!(
                "Updated session sess-1\n  CWD:          {}\n",
                directory.path().display()
            )
        );
    }

    /// The desktop app's database in the isolated home, opened for a test to
    /// seed and inspect. It uses the same schema the CLI writes.
    fn open_app_db() -> rusqlite::Connection {
        let path = future_app_settings::app_db_path().expect("app db path");
        std::fs::create_dir_all(path.parent().expect("app dir")).expect("app dir");
        let conn = rusqlite::Connection::open(&path).expect("open app db");
        future_app_workspaces::ensure_table(&conn).expect("workspaces table");
        conn
    }

    /// A thread bound to an Agent session, plus the tables the filing rule reads
    /// — what the desktop app's import creates when it adopts a session.
    fn seed_desktop_thread(session_id: &str, thread_id: &str) {
        let conn = open_app_db();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS threads (
                 id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 agent_session_id TEXT,
                 status TEXT NOT NULL DEFAULT 'active',
                 updated_at INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("threads table");
        conn.execute(
            "INSERT OR REPLACE INTO threads (id, workspace_id, agent_session_id, status, updated_at)
             VALUES (?1, '', ?2, 'active', 0)",
            rusqlite::params![thread_id, session_id],
        )
        .expect("seed thread");
    }

    fn thread_workspace_id(conn: &rusqlite::Connection, thread_id: &str) -> String {
        conn.query_row(
            "SELECT workspace_id FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
            |row| row.get(0),
        )
        .expect("thread workspace")
    }

    #[tokio::test]
    async fn set_json_reports_applied_and_failed_fields() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        // The model is gone from the registry: that field fails, the rest apply.
        agent.fail_with.insert(
            "set_model".into(),
            "model `gone` is no longer available".into(),
        );
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &[
                "sess-1".into(),
                "--json".into(),
                "--title".into(),
                "New".into(),
                "--model".into(),
                "gone".into(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessionId"], "sess-1");
        assert_eq!(parsed["updated"]["title"], "New");
        assert_eq!(
            parsed["failed"]["model"],
            "model `gone` is no longer available"
        );
        assert!(parsed["updated"]["model"].is_null());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(
            stderr,
            "Failed to set model: model `gone` is no longer available\n"
        );
        // The title was still applied (no rollback).
        assert_eq!(agent.seen_of("set_session_name").len(), 1);
    }

    #[tokio::test]
    async fn set_parent_empty_detaches() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &["sess-1".into(), "--parent".into(), String::new()],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "Updated session sess-1\n  Parent:       (none)\n");
        let seen = agent.seen_of("set_parent_session");
        assert_eq!(seen[0].parent_session, "");
        // No existence probe for a detach.
        assert!(agent.seen_of("list_sessions").is_empty());
    }

    #[tokio::test]
    async fn set_usage_errors() {
        // No target id.
        let (out, cap) = Output::memory();
        let result = session(Some("set"), &[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future session set <session-id> [options]\n");

        // A target with no options at all.
        let (out, cap) = Output::memory();
        let result = session(Some("set"), &["sess-1".into()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(
            stderr,
            "Nothing to set. Pass at least one of --parent, --title, --cwd, --model, --thinking\n"
        );

        // Invalid thinking level and an unknown option are hard errors.
        let (out, cap) = Output::memory();
        assert!(session(
            Some("set"),
            &["sess-1".into(), "--thinking".into(), "bogus".into()],
            &out
        )
        .await
        .is_err());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Invalid thinking level: bogus"), "{stderr}");

        let (out, cap) = Output::memory();
        assert!(
            session(Some("set"), &["sess-1".into(), "--frobnicate".into()], &out)
                .await
                .is_err()
        );
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Unknown option: --frobnicate\n");
    }

    /// The live-session options the CLI now owns, one RPC each. They are the
    /// only way to set these from a terminal — `--tools` on `future run` lasts
    /// one run, and none of the rest had a CLI spelling at all.
    #[tokio::test]
    async fn set_applies_the_live_session_options() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--tools".into(),
                "read, shell".into(),
                "--append-system-prompt".into(),
                "be terse".into(),
                "--permission".into(),
                "workspace".into(),
                "--sandbox".into(),
                "manual".into(),
                "--context-files".into(),
                "off".into(),
                "--auto-compact".into(),
                "off".into(),
                "--auto-retry".into(),
                "on".into(),
                "--json".into(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["updated"]["tools"], "read,shell");
        assert_eq!(parsed["updated"]["permissionLevel"], "workspace");
        assert_eq!(parsed["updated"]["sandboxTier"], "manual");
        assert_eq!(parsed["updated"]["contextFiles"], "off");
        assert_eq!(parsed["updated"]["autoCompaction"], "off");
        assert_eq!(parsed["updated"]["autoRetry"], "on");
        assert_eq!(parsed["updated"]["appendSystemPrompt"], "(appended)");

        // The tool list is trimmed of the spaces the user typed.
        assert_eq!(agent.seen_of("set_tools")[0].tools, vec!["read", "shell"]);
        assert_eq!(
            agent.seen_of("append_system_prompt")[0].system_prompt,
            "be terse"
        );
        assert_eq!(agent.seen_of("set_permission_level")[0].level, "workspace");
        let policy = agent.seen_of("set_sandbox_policy")[0]
            .sandbox_policy
            .clone()
            .expect("sandbox policy sent");
        assert_eq!(policy.tier, "manual");
        // `--context-files off` inverts into the session's `no_context_files`.
        assert!(!agent.seen_of("set_context_files")[0].enabled);
        assert!(!agent.seen_of("set_auto_compaction")[0].enabled);
        assert!(agent.seen_of("set_auto_retry")[0].enabled);
        // Every one is scoped to the target session.
        for cmd in [
            "set_tools",
            "append_system_prompt",
            "set_permission_level",
            "set_sandbox_policy",
            "set_context_files",
            "set_auto_compaction",
            "set_auto_retry",
        ] {
            assert_eq!(agent.seen_of(cmd)[0].session_id, "sess-1", "{cmd}");
        }
    }

    #[tokio::test]
    async fn set_tool_choices_map_to_their_own_rpc() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (agent, _env) = mock_env(agent).await;

        // `--no-tools` is `disable_tools`, not `set_tools` with an empty list.
        let (out, _cap) = Output::memory();
        session(Some("set"), &["sess-1".into(), "--no-tools".into()], &out)
            .await
            .expect("set");
        assert_eq!(agent.seen_of("disable_tools").len(), 1);
        assert!(agent.seen_of("set_tools").is_empty());

        // `--no-builtin-tools` keeps extensions, so it is its own command.
        let (out, _cap) = Output::memory();
        session(
            Some("set"),
            &["sess-1".into(), "--no-builtin-tools".into()],
            &out,
        )
        .await
        .expect("set");
        assert_eq!(agent.seen_of("disable_builtin_tools").len(), 1);
    }

    #[tokio::test]
    async fn set_rejects_contradictory_or_malformed_live_options() {
        let cases: &[(&[&str], &str)] = &[
            (&["--tools", "read", "--no-tools"], "mutually exclusive"),
            (&["--no-tools", "--tools", "read"], "mutually exclusive"),
            (&["--tools", " , "], "at least one tool name"),
            (&["--permission", "sometimes"], "Invalid permission level"),
            (&["--sandbox", "on"], "Invalid sandbox tier"),
            (&["--auto-compact", "maybe"], "--auto-compact takes on|off"),
            (&["--context-files", "2"], "--context-files takes on|off"),
        ];
        for (options, expected) in cases {
            let mut argv = vec!["sess-1".to_string()];
            argv.extend(options.iter().map(|s| s.to_string()));
            let (out, cap) = Output::memory();
            let result = session(Some("set"), &argv, &out).await;
            assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()), "{options:?}");
            let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
            assert!(
                stderr.contains(expected),
                "{options:?} should mention {expected:?}, got {stderr:?}"
            );
        }
    }

    #[tokio::test]
    async fn set_accepts_the_boolean_spellings() {
        for value in ["on", "true", "yes", "1"] {
            let parsed = parse_session_options(
                &["--auto-compact".to_string(), value.to_string()],
                &Output::memory().0,
            )
            .expect("parse");
            assert_eq!(parsed.auto_compaction, Some(true), "{value}");
        }
        for value in ["off", "false", "no", "0"] {
            let parsed = parse_session_options(
                &["--auto-compact".to_string(), value.to_string()],
                &Output::memory().0,
            )
            .expect("parse");
            assert_eq!(parsed.auto_compaction, Some(false), "{value}");
        }
    }

    #[tokio::test]
    async fn set_reports_an_option_the_agent_refuses_without_losing_the_others() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        agent.fail_with.insert(
            "set_sandbox_policy".into(),
            "Sandbox availability could not be determined".into(),
        );
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &[
                "sess-1".into(),
                "--permission".into(),
                "none".into(),
                "--sandbox".into(),
                "sandbox".into(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // The one that worked is reported; the refusal is not swallowed.
        assert!(stdout.contains("Permission:   none"), "{stdout}");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Failed to set sandboxTier"), "{stderr}");
        assert!(stderr.contains("Sandbox availability"), "{stderr}");
        assert_eq!(agent.seen_of("set_permission_level").len(), 1);
    }

    #[tokio::test]
    async fn set_help_needs_no_agent_or_target() {
        for args in [
            vec!["--help".to_string()],
            vec!["sess-1".to_string(), "--help".to_string()],
            vec!["-h".to_string()],
        ] {
            let (out, cap) = Output::memory();
            session(Some("set"), &args, &out).await.expect("help");
            let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
            assert_eq!(stdout, format!("{}\n", SESSION_SET_HELP), "args {args:?}");
        }
    }

    #[tokio::test]
    async fn set_unknown_target_reports_session_not_found() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        let session_gate =
            "session not found — pass a valid session_id (new_session creates one)".to_string();
        agent.fail_with.insert("get_state".into(), session_gate);
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &["ghost".into(), "--title".into(), "x".into()],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Session not found: ghost\n");
        // Nothing was mutated.
        assert!(agent.seen_of("set_session_name").is_empty());
    }

    #[tokio::test]
    async fn set_parent_must_exist_and_not_be_self() {
        let _guard = crate::test_env::lock_env().await;
        // Self-parent is rejected before any RPC beyond target resolution.
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &["sess-1".into(), "--parent".into(), "sess-1".into()],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert_eq!(
            String::from_utf8(cap.err.lock().unwrap().clone()).unwrap(),
            "A session cannot be its own parent\n"
        );
        assert!(agent.seen_of("set_parent_session").is_empty());
        drop(_env);

        // An unknown parent is reported by the agent (it also accepts a live
        // session with no entries, which the CLI cannot see) and aborts the
        // other options.
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        agent.fail_with.insert(
            "set_parent_session".into(),
            "parent session not found: ghost".into(),
        );
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &[
                "sess-1".into(),
                "--parent".into(),
                "ghost".into(),
                "--title".into(),
                "x".into(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert_eq!(
            String::from_utf8(cap.err.lock().unwrap().clone()).unwrap(),
            "Session not found: ghost\n"
        );
        // The bad parent aborts before the other options are applied.
        assert!(agent.seen_of("set_session_name").is_empty());
    }

    #[test]
    fn parse_session_options_requires_values_and_rejects_unknowns() {
        // A flag with no value is a hard error (not silently ignored).
        for flag in ["--parent", "--title", "--cwd", "--model", "--thinking"] {
            let (out, cap) = Output::memory();
            let result = parse_session_options(&[flag.to_string()], &out);
            assert!(result.is_err(), "flag {flag}");
            let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
            assert_eq!(stderr, format!("Missing value for {flag}\n"), "flag {flag}");
        }
    }

    #[tokio::test]
    async fn session_help_documents_set() {
        assert!(SESSION_HELP.contains("future session set <id> [options]"));
        assert!(SESSION_HELP.contains("Change settings on an existing session"));
        assert!(SESSION_SET_HELP.contains("future session set <session-id> [options]"));
    }

    /// `session new` was dropped in #774 because "a create path whose result no
    /// command consumes": it printed an id and nothing could act on it. That
    /// premise is gone — `session set` now configures a session and
    /// `session status` reads it back — so the create step is back, and the
    /// command has to hand the agent the cwd and name.
    #[tokio::test]
    async fn session_new_creates_a_session_and_warns_it_is_unlisted() {
        let _guard = crate::test_env::lock_env().await;
        let agent =
            crate::test_server::MockAgent::respond("new_session", "{\"sessionId\":\"fresh-1\"}");
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("new"),
            &[
                "--cwd".into(),
                "/tmp/proj".into(),
                "--name".into(),
                "T".into(),
            ],
            &out,
        )
        .await
        .expect("new");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Created session fresh-1"), "{stdout}");
        assert!(stdout.contains("/tmp/proj"));
        assert!(stdout.contains("  Name: T"));
        assert!(
            stdout.contains("not listed until its first run"),
            "a new session is absent from `list` until it runs: {stdout}"
        );

        let sent = agent.seen_of("new_session");
        assert_eq!(sent[0].cwd, "/tmp/proj");
        assert_eq!(sent[0].name, "T");
        assert_eq!(sent[0].created_by, "cli");
        // No output flag: the same facts, machine-readable.
        let (out, cap) = Output::memory();
        session(
            Some("new"),
            &["--cwd".into(), "/tmp/p2".into(), "--json".into()],
            &out,
        )
        .await
        .expect("new --json");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessionId"], "fresh-1");
        assert_eq!(parsed["cwd"], "/tmp/p2");
    }

    #[tokio::test]
    async fn session_new_defaults_to_the_current_directory() {
        let _guard = crate::test_env::lock_env().await;
        let agent =
            crate::test_server::MockAgent::respond("new_session", "{\"sessionId\":\"fresh-2\"}");
        let (agent, _env) = mock_env(agent).await;
        let (out, _cap) = Output::memory();
        session(Some("new"), &[], &out).await.expect("new");
        let cwd = std::env::current_dir().unwrap().display().to_string();
        assert_eq!(agent.seen_of("new_session")[0].cwd, cwd);
        assert!(agent.seen_of("new_session")[0].name.is_empty());
    }

    #[tokio::test]
    async fn session_new_rejects_unknown_options() {
        let (out, cap) = Output::memory();
        let result = session(Some("new"), &["--bogus".into(), "x".into()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Unknown option: --bogus\n");
        // A missing value is caught before any agent call.
        let (out, cap) = Output::memory();
        assert!(session(Some("new"), &["--cwd".into()], &out).await.is_err());
        assert_eq!(
            String::from_utf8(cap.err.lock().unwrap().clone()).unwrap(),
            "Missing value for --cwd\n"
        );
    }

    // ── fork / clone / title / export ───────────────────────────────

    #[tokio::test]
    async fn forks_lists_the_turns_with_copy_pasteable_ids() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "get_fork_messages",
            r#"{"messages":[
                {"id":"e-1","kind":"user","blocks":[{"kind":"text","text":"first question"}]},
                {"id":"e-2","kind":"user","blocks":[{"kind":"text","text":"second one"}]}
            ]}"#,
        );
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("forks"), &["sess-1".into()], &out)
            .await
            .expect("forks");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("2 user turn(s)"), "{stdout}");
        assert!(stdout.contains("e-1"), "{stdout}");
        assert!(stdout.contains("first question"), "{stdout}");
        assert!(
            stdout.contains("future session fork sess-1 --entry <entry-id>"),
            "the next command must be copy-pasteable: {stdout}"
        );
        assert_eq!(agent.seen_of("get_fork_messages")[0].session_id, "sess-1");

        // An empty history is a statement, not a blank table.
        let agent =
            crate::test_server::MockAgent::respond("get_fork_messages", r#"{"messages":[]}"#);
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("forks"), &["sess-1".into()], &out)
            .await
            .expect("forks");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("no user turns to fork at"), "{stdout}");
    }

    #[tokio::test]
    async fn fork_requires_an_entry_and_reports_the_child() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "fork",
            r#"{"cancelled":false,"sessionId":"child-1"}"#,
        );
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("fork"),
            &["sess-1".into(), "--entry".into(), "e-2".into()],
            &out,
        )
        .await
        .expect("fork");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Forked session sess-1 at e-2 → child-1"),
            "{stdout}"
        );
        assert_eq!(agent.seen_of("fork")[0].entry_id, "e-2");
        assert_eq!(agent.seen_of("fork")[0].session_id, "sess-1");

        // Without --entry the caller is told where to get one.
        let (out, cap) = Output::memory();
        let result = session(Some("fork"), &["sess-1".into()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("requires --entry"), "{stderr}");
        assert!(stderr.contains("future session forks sess-1"), "{stderr}");
    }

    #[tokio::test]
    async fn clone_branches_at_the_latest_settled_point() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "clone",
            r#"{"cancelled":false,"sessionId":"copy-1","created":true}"#,
        );
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("clone"), &["sess-1".into(), "--json".into()], &out)
            .await
            .expect("clone");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessionId"], "copy-1");
        assert_eq!(parsed["created"], true);
        assert_eq!(agent.seen_of("clone")[0].session_id, "sess-1");
    }

    #[tokio::test]
    async fn title_generates_with_the_desktop_language_and_only_applies_on_request() {
        let _guard = crate::test_env::lock_env().await;
        // An isolated HOME: `title --lang` is not given, so the language comes
        // from the desktop store, which must not be the real one.
        let home = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[
            ("HOME", home.path().as_os_str().to_owned()),
            ("FUTURE_HOME", home.path().as_os_str().to_owned()),
        ]);
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert(
            "generate_session_title".into(),
            r#"{"title":"Fix the flaky test","model":"m1"}"#.to_string(),
        );
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(Some("title"), &["sess-1".into()], &out)
            .await
            .expect("title");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("Fix the flaky test"), "{stdout}");
        assert!(
            stdout.contains("Not applied"),
            "generating a title must not rename the session: {stdout}"
        );
        // The app-store default is `en`.
        assert_eq!(agent.seen_of("generate_session_title")[0].mode, "en");
        // No rename happened.
        assert!(agent.seen_of("set_session_name").is_empty());

        // `--apply` is what renames, and `--lang` overrides the store.
        let (out, _cap) = Output::memory();
        session(
            Some("title"),
            &[
                "sess-1".into(),
                "--lang".into(),
                "zh".into(),
                "--apply".into(),
            ],
            &out,
        )
        .await
        .expect("title --apply");
        let sent = agent.seen_of("generate_session_title");
        assert_eq!(sent[1].mode, "zh");
        assert_eq!(
            agent.seen_of("set_session_name")[0].name,
            "Fix the flaky test"
        );
    }

    #[tokio::test]
    async fn title_rejects_an_unknown_language() {
        let (out, cap) = Output::memory();
        let result = session(
            Some("title"),
            &["sess-1".into(), "--lang".into(), "fr".into()],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert_eq!(
            String::from_utf8(cap.err.lock().unwrap().clone()).unwrap(),
            "--lang must be en or zh\n"
        );
    }

    #[tokio::test]
    async fn export_copies_the_agents_file_when_asked() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("session.html");
        std::fs::write(&source, "<html>transcript</html>").expect("seed");
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert(
            "export_html".into(),
            serde_json::json!({"path": source.display().to_string()}).to_string(),
        );
        let (agent, _env) = mock_env(agent).await;

        // No --out: the agent's own temporary path is reported so the caller
        // can read it, but nothing is copied.
        let (out, cap) = Output::memory();
        session(Some("export"), &["sess-1".into()], &out)
            .await
            .expect("export");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains(&source.display().to_string()), "{stdout}");

        // --out is a real copy with the same content.
        let destination = dir.path().join("kept.html");
        let (out, cap) = Output::memory();
        session(
            Some("export"),
            &[
                "sess-1".into(),
                "--out".into(),
                destination.display().to_string(),
            ],
            &out,
        )
        .await
        .expect("export --out");
        assert_eq!(
            std::fs::read_to_string(&destination).expect("copy"),
            "<html>transcript</html>"
        );
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains(&destination.display().to_string()),
            "{stdout}"
        );
        assert_eq!(agent.seen_of("export_html").len(), 2);
    }

    #[tokio::test]
    async fn export_fails_when_the_destination_cannot_be_written() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("s.html");
        std::fs::write(&source, "x").expect("seed");
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert(
            "export_html".into(),
            serde_json::json!({"path": source.display().to_string()}).to_string(),
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, _cap) = Output::memory();
        let error = session(
            Some("export"),
            &[
                "sess-1".into(),
                "--out".into(),
                dir.path().join("nope").join("x.html").display().to_string(),
            ],
            &out,
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("cannot write"), "{error}");
    }

    #[tokio::test]
    async fn the_new_subcommands_report_a_missing_session_id() {
        for name in ["fork", "forks", "clone", "title", "export"] {
            let (out, cap) = Output::memory();
            let result = session(Some(name), &[], &out).await;
            assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()), "{name}");
            let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
            assert_eq!(
                stderr,
                format!("Usage: future session {name} <session-id>\n")
            );
        }
    }

    #[tokio::test]
    async fn the_new_subcommands_have_their_own_help() {
        for name in ["fork", "forks", "clone", "title", "export"] {
            let (out, cap) = Output::memory();
            session(Some(name), &["--help".into()], &out)
                .await
                .expect("help");
            let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
            assert!(
                stdout.starts_with("future session — fork, clone"),
                "{name}: {stdout}"
            );
        }
    }

    #[tokio::test]
    async fn session_rename_usage_includes_name_placeholder() {
        let (out, cap) = Output::memory();
        let result = session(Some("rename"), &[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Usage: future session rename <session-id> <name>"),
            "stderr: {stderr}"
        );
    }
}
