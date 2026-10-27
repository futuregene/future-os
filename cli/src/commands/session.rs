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
  future session list [--parent <id>] [--json]        List all sessions
  future session new [options]                       Create a session
  future session set <id> [options]                  Change settings on an existing session
  future session info <id> [--json]                  Show session details + stats
  future session status <id> [--json] [--metrics] [--system-prompt]
                                                     Live state: permission, context, runs
  future session transcript --help                   Filter/window one session's records
  future session history --help                      Search/read original history
  future session compact --help                      Request manual compaction
  future session reset <id> [selectors] [--yes]      Delete part of the history in place
  future session rename <id> <name>                  Give a session a readable name
  future session fork <id> --entry <entry-id>        Branch a new session at a user turn
  future session forks <id> [--json]                 List the turns a session can fork at
  future session clone <id>                          Copy a session at its latest settled point
  future session title <id>                          Generate a title (a model call)
  future session export <id> [--out <path>]          Write the session to an HTML file
  future session abort <id>                          Stop the active run and clear the queue
  future session cancel <id> --run <run-id>            Cancel one queued run
  future session wait <id> [--run <run-id>]            Wait for a run to finish and print its answer
  future session approvals <id> [--json]             Approval requests the session waits on
  future session approve <id> <request-id> [opts]      Allow a pending approval
  future session reject <id> <request-id> [opts]       Deny a pending approval
  future session delete <id>                           Delete a session
  future session prune --older-than <age> [--yes]      Delete sessions idle longer than <age>
  future session journal [--reclaim [--yes]] [--json]  Raw run-journal cost + reclamation

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
  --thinking <level>      Thinking level: default (provider default), off, minimal,
                          low, medium, high, xhigh, max
  --system-prompt <text>  Replace this session's system prompt: the text
                          becomes the prompt's opening section instead of the
                          built-in identity. Stored with the session, so it
                          survives an agent restart and applies to runs started
                          later by any client. An empty string restores the
                          built-in prompt. The project context, workspace
                          memory and --append-system-prompt are still added
                          after it. Read it back with `future session status
                          <id>`.
  --append-system-prompt <text>
                          Rules appended to this session's system prompt on
                          every run, after the project context and workspace
                          memory. Stored with the session, so it survives an
                          agent restart and applies to runs started later by
                          any client — unlike the live options below. An empty
                          string clears it. Read it back with `future session
                          status <id>`; the whole built prompt with `future
                          session status <id> --system-prompt`.

Applied to the live session (until this agent stops):
  --tools <names>         Enable exactly these tools (comma-separated).
  --no-tools              Disable every tool.
  --no-builtin-tools      Disable built-in tools only (keep extensions).
  --permission <level>    Tool permission: all|workspace|none. This is the
                          approval gate; it is not the OS sandbox.
  --sandbox <tier>        Approval mode: off|manual|sandbox|auto. `sandbox` is
                          refused when the platform cannot provide one.
  --context-files <on|off>
                          Load AGENTS.md and friends into the prompt.
  --auto-compact <on|off> Compact autonomously when context fills.
  --auto-retry <on|off>   Retry a failed request automatically.

Output:
  --json                  Print the update as a JSON object
  --help, -h              Show this help

Only the options you pass are changed; everything else is left alone, and one
failure does not roll back the others. A parent records lineage only — unlike
`fork`, no history is copied, and the parent must be an existing session.

The model and thinking level take effect at once and are stored with the
session's next run (they are part of the run snapshot). `future run` only
applies its own `--cwd` when that flag is given, or when it creates the session
(a new session starts in the caller's directory); attaching to an existing
session with `--session`/`--continue` leaves the cwd set here alone. Read the
current values back with `future session status <id>`."#;

fn help(out: &Output) {
    out.log(SESSION_HELP);
}

/// `truncate(s, n)` — cut to n chars (last char replaced with "…").
pub(super) fn truncate(s: &str, n: usize) -> String {
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

pub(super) fn relative_age(updated_ms: i64, now_ms: i64) -> String {
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
pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ─── List ─────────────────────────────────────────────────────────────────

/// `listSessions(jsonFlag)`.
async fn list_sessions(json_flag: bool, parent: Option<&str>, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    // Not wrapped in try/catch — errors propagate to main().catch.
    let data = client.list_sessions().await?;
    // `const { sessions } = await client.listSessions();`
    let sessions: Vec<Value> = data
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Filtered here rather than server-side: the list is already in memory, and
    // a parent's children are a handful of rows inside it.
    let sessions: Vec<Value> = match parent {
        Some(parent) => sessions
            .into_iter()
            .filter(|s| s.get("parentSessionId").and_then(Value::as_str) == Some(parent))
            .collect(),
        None => sessions,
    };
    if json_flag {
        // Serializing plain JSON values is infallible.
        out.log(&serde_json::to_string_pretty(&json_value(&sessions)).expect("json serializes"));
        return Ok(());
    }

    if sessions.is_empty() {
        match parent {
            Some(parent) => out.log(&format!("No sessions found under {parent}.")),
            None => out.log("No sessions found."),
        }
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
pub(super) fn pad_end(s: &str, n: usize) -> String {
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
    // The session's system-prompt overrides are stored in the record this
    // command already reads, so report them here: this is the persisted view,
    // and a rule the user set is part of what the session is. An empty append
    // prints nothing (unlike `session status`, whose layout is fixed) — most
    // sessions have neither, and a two-line "(none)" on every session would be
    // noise in the command whose job is a quick overview.
    let append = content["append_system_prompt"].as_str().unwrap_or("");
    if !append.is_empty() {
        out.log(&format!(
            "  Append prompt: {}",
            super::session_status::first_line_summary(append)
        ));
    }
    let custom = content["system_prompt"].as_str().unwrap_or("");
    if !custom.is_empty() {
        out.log(&format!(
            "  System prompt: REPLACED — {}",
            super::session_status::first_line_summary(custom)
        ));
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
    let mut append_system_prompt = None;
    let mut parent = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        if !matches!(
            flag,
            "--cwd" | "--name" | "--append-system-prompt" | "--parent"
        ) {
            out.log_err(&format!("Unknown option: {flag}"));
            return Err(crate::HANDLED_EXIT.to_string());
        }
        let Some(value) = args.get(index + 1).filter(|v| !v.starts_with("--")) else {
            out.log_err(&format!("Missing value for {flag}"));
            return Err(crate::HANDLED_EXIT.to_string());
        };
        match flag {
            "--cwd" => cwd = Some(value.clone()),
            "--name" => name = value.clone(),
            "--parent" => parent = Some(value.clone()),
            _ => append_system_prompt = Some(value.clone()),
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
        .new_named_session(
            &cwd,
            &name,
            append_system_prompt.as_deref(),
            parent.as_deref(),
        )
        .await?;
    let session_id = result["sessionId"].as_str().unwrap_or("").to_string();
    if json {
        let mut document = json!({
            "sessionId": session_id,
            "cwd": cwd,
            "sessionName": name,
        });
        if let Some(append) = &append_system_prompt {
            document["appendSystemPrompt"] = json!(append);
        }
        if let Some(parent) = &parent {
            document["parentSessionId"] = json!(parent);
        }
        out.log(&serde_json::to_string(&document).expect("json serializes"));
        return Ok(());
    }
    out.log(&format!("Created session {session_id}"));
    out.log(&format!("  CWD:  {cwd}"));
    if !name.is_empty() {
        out.log(&format!("  Name: {name}"));
    }
    if let Some(append) = &append_system_prompt {
        out.log(&format!("  Append prompt: {append}"));
    }
    if let Some(parent) = &parent {
        out.log(&format!("  Parent: {parent}"));
    }
    out.log(
        "\nThe session is listed right away — `future session list` finds it, and \
         `future session list --parent <id>` finds it under its parent when it has \
         one. Start it with `future run --session <id> <prompt>`.",
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
  future session new [--cwd <dir>] [--name <text>] [--parent <session-id>] [--append-system-prompt <text>] [--json]

Options:
  --cwd <dir>    Working directory for the new session (default: the current
                 directory). The agent resolves ~ and relative paths.
  --name <text>  Initial title.
  --parent <session-id>
                 Record another session as this session's parent — what makes a
                 session a sub-session of the one that created it. The parent
                 must exist. Lineage only: no history is copied (that is
                 `fork`), but the parent's conversation will list this session
                 under it, and this session's prompt will not offer to delegate
                 further — one jump is as far as delegation goes, and the agent
                 refuses a deeper chain. A user can move a session out of such a
                 chain with `future session set <id> --parent \"\"`.
                 `future session list --parent <id>` finds its children.
  --append-system-prompt <text>
                 Rules appended to the end of this session's system prompt on
                 every run. Stored with the session, so it survives an agent
                 restart. Same storage as `future session set <id>
                 --append-system-prompt`; the environment block stays after it.
  --json         Print {sessionId, cwd, sessionName, parentSessionId?} instead
                 of prose.

A newly created session is persisted at once, so `future session list` shows it
immediately (with no messages yet); `future run --session <id> <prompt>` starts
it.";

pub const LIST_HELP: &str = "future session list — list sessions

Usage:
  future session list [--parent <session-id>] [--json]

Options:
  --parent <session-id>  Only the sessions whose recorded parent is this one —
                         the sub-sessions it created, each with its
                         isStreaming flag. Reads the same parent `session
                         status` reports, so a later
                         `session set <id> --parent` moves a session between
                         lists.
  --json                 Machine-readable output.
  -h, --help             Show this help";

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
            // Same distinction as the append segment: an empty value restores
            // the built-in prompt, which is not a "replacement".
            Ok(()) => applied.push((
                "systemPrompt",
                if prompt.is_empty() {
                    "(built-in restored)".into()
                } else {
                    super::session_status::first_line_summary(prompt)
                },
            )),
            Err(err) => failures.push(("systemPrompt", missing_session_error(err, &target_id))),
        }
    }
    if let Some(prompt) = &parsed.append_system_prompt {
        match client.append_system_prompt(prompt, &target_id).await {
            // An empty value is not a failure: it is how a caller clears the
            // segment, so the report has to distinguish "cleared" from
            // "set to something".
            Ok(()) => applied.push((
                "appendSystemPrompt",
                if prompt.is_empty() {
                    "(cleared)".into()
                } else {
                    prompt.clone()
                },
            )),
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
            Ok(_) => {
                applied.push(("sandboxTier", tier.clone()));
                // `manual`/`sandbox`/`auto` only ask while the approval gate is
                // armed: at the legacy `all` level the agent returns from
                // `before_tool_call` before any approval rule is consulted, so
                // reporting the mode alone would promise prompts that never
                // come. The permission block above already ran, so a
                // `--permission` value is the post-state.
                if matches!(tier.as_str(), "manual" | "sandbox" | "auto") {
                    let level = match &parsed.permission {
                        Some(level) => Some(level.clone()),
                        None => client
                            .get_state(Some(&target_id))
                            .await
                            .ok()
                            .and_then(|state| {
                                state
                                    .get("permissionLevel")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                            }),
                    };
                    // Only a level we actually know to be `all` earns the
                    // warning: a failed `get_state` (or an older agent that
                    // omits the field) leaves the level unknown, and the note
                    // would then assert a fact this command never learned.
                    if level.as_deref() == Some("all") {
                        notes.push(format!(
                            "approval mode `{tier}` cannot ask while the tool permission is `all` — add `--permission workspace`"
                        ));
                    }
                }
            }
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
        "sandboxTier" => "Approval",
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

    if subcommand == "reset" {
        return super::session_reset::run(args, out).await;
    }

    if subcommand == "prune" {
        return super::session_prune::run(args, out).await;
    }

    if subcommand == "journal" {
        return super::session_journal::run(args, out).await;
    }

    if subcommand == "wait" {
        return super::session_wait::run(args, out).await;
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
        // `--parent` narrows the list to one session's children, so an agent can
        // see what it already started without filtering the whole list itself.
        let mut json_flag = false;
        let mut parent: Option<String> = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--json" => {
                    json_flag = true;
                    index += 1;
                }
                "--parent" => {
                    let Some(value) = args.get(index + 1).filter(|v| !v.starts_with("--")) else {
                        out.log_err("Missing value for --parent");
                        return Err(crate::HANDLED_EXIT.to_string());
                    };
                    parent = Some(value.clone());
                    index += 2;
                }
                "--help" | "-h" => {
                    out.log(LIST_HELP);
                    return Ok(());
                }
                other => {
                    out.log_err(&format!("Unknown option: {other}"));
                    return Err(crate::HANDLED_EXIT.to_string());
                }
            }
        }
        list_sessions(json_flag, parent.as_deref(), out).await?;
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
        let result = list_sessions(false, None, &out).await;
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
        )])
        .with_temp_home();
        // `session set --cwd` files the conversation in the desktop app's own
        // store (`~/.future/app/app.db`), so every one of these tests must run
        // against a throwaway home: without this they would read — and, on a
        // machine where the desktop app has imported the session, *write* — the
        // real app database.
        (agent, env)
    }

    /// `--parent` in the create step is what makes a child a child: without it
    /// the new session is an unrelated conversation that merely happens to be
    /// busy, and neither the rail nor `session list --parent` would find it.
    #[tokio::test]
    async fn session_new_records_the_parent_in_the_same_step() {
        let _guard = crate::test_env::lock_env().await;
        let agent =
            crate::test_server::MockAgent::respond("new_session", "{\"sessionId\":\"child-1\"}");
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("new"),
            &[
                "--cwd".into(),
                "/tmp/proj".into(),
                "--parent".into(),
                "parent-9".into(),
                "--json".into(),
            ],
            &out,
        )
        .await
        .expect("new --parent");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessionId"], "child-1");
        assert_eq!(
            parsed["parentSessionId"], "parent-9",
            "a script needs the recorded lineage back: {stdout}"
        );
        // The lineage travels on the create command itself, so the link exists
        // even if the child's first run never happens.
        assert_eq!(agent.seen_of("new_session")[0].parent_session, "parent-9");

        // Prose output reports it too; without it the caller cannot tell a
        // child from a root session.
        let (out, cap) = Output::memory();
        session(
            Some("new"),
            &[
                "--cwd".into(),
                "/tmp/proj".into(),
                "--parent".into(),
                "parent-9".into(),
            ],
            &out,
        )
        .await
        .expect("new --parent");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("  Parent: parent-9"), "{stdout}");
    }

    /// `list --parent` answers "what did I already start" without making the
    /// caller filter the whole list (and without jq, which Windows lacks).
    #[tokio::test]
    async fn session_list_filters_by_parent() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "list_sessions",
            "{\"sessions\":[{\"id\":\"child-a\",\"parentSessionId\":\"me\",\"isStreaming\":true},\
                            {\"id\":\"child-b\",\"parentSessionId\":\"me\"},\
                            {\"id\":\"unrelated\",\"parentSessionId\":\"other\"},\
                            {\"id\":\"root\"}]}",
        );
        let (_agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(Some("list"), &["--parent".into(), "me".into()], &out)
            .await
            .expect("list --parent");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("child-a"), "{stdout}");
        assert!(stdout.contains("child-b"), "{stdout}");
        // Neither a sibling under another parent nor a root session belongs in
        // the answer, however similar its row looks.
        assert!(!stdout.contains("unrelated"), "{stdout}");
        assert!(!stdout.contains("root  "), "{stdout}");

        // JSON carries the same filter, so a script sees only children.
        let (out, cap) = Output::memory();
        session(
            Some("list"),
            &["--parent".into(), "me".into(), "--json".into()],
            &out,
        )
        .await
        .expect("list --parent --json");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        let ids: Vec<&str> = parsed["sessions"]
            .as_array()
            .expect("array")
            .iter()
            .map(|s| s["id"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(ids, vec!["child-a", "child-b"]);

        // A parent with no children says so instead of printing an empty table.
        let (out, cap) = Output::memory();
        session(Some("list"), &["--parent".into(), "lonely".into()], &out)
            .await
            .expect("list --parent");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "No sessions found under lonely.\n");
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

    /// A session whose recorded prompt settings exist reports them; the default
    /// (neither set) adds no lines, because `info` is the quick overview.
    #[tokio::test]
    async fn info_reports_the_recorded_system_prompt_settings() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            r#"{"entries":[{"kind":"session_info","role":"system","session":{"sessionName":"Named","cwd":"/work","append_system_prompt":"Always answer in haiku.","system_prompt":"You are terse."}},{"kind":"user","role":"user"}]}"#,
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["sess-1".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Append prompt: Always answer in haiku."),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("System prompt: REPLACED — You are terse."),
            "stdout: {stdout}"
        );

        // Neither set: no lines at all.
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            r#"{"entries":[{"kind":"session_info","role":"system","session":{"sessionName":"Plain"}},{"kind":"user","role":"user"}]}"#,
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["sess-2".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(!stdout.contains("Append prompt:"), "stdout: {stdout}");
        assert!(!stdout.contains("System prompt:"), "stdout: {stdout}");
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
            r#"{"entries":[{"kind":"session_info","role":"system","session":{"sessionName":"Named","cwd":"/work","model":"m1","thinkingLevel":"high","append_system_prompt":"Rule.","usage":{"inputTokens":10,"outputTokens":2,"costCny":0.5}}},{"kind":"user","role":"user"},{"kind":"assistant","role":"assistant","blocks":[{"kind":"tool_call"}]}]}"#,
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
        // `--json` passes the recorded session object through, so a prompt
        // setting stored in the record is machine-readable here without a
        // second command.
        assert_eq!(parsed["session"]["append_system_prompt"], json!("Rule."));
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
            future_app_workspaces::normalize_path(directory.path())
                .display()
                .to_string()
        );
        assert_eq!(thread_workspace_id(&conn, "thread-1"), workspace.id);
        // The conversation left Chat: both clients group by `mode`, so filing
        // the workspace alone would have kept it under Chat for good.
        assert_eq!(thread_mode(&conn, "thread-1"), "workspace");

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

    /// A thread bound to an Agent session, plus the columns the filing rule and
    /// the workspace ordering read — what the desktop app's import creates when
    /// it adopts a session. The ordering ranks a workspace by
    /// `COALESCE(last_message_at, updated_at, created_at)` of its newest
    /// conversation, so a fixture that omits those columns is not a database the
    /// app could have written.
    fn seed_desktop_thread(session_id: &str, thread_id: &str) {
        let conn = open_app_db();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS threads (
                 id TEXT PRIMARY KEY,
                 workspace_id TEXT NOT NULL,
                 mode TEXT NOT NULL DEFAULT 'chat',
                 agent_session_id TEXT,
                 status TEXT NOT NULL DEFAULT 'active',
                 last_message_at INTEGER,
                 created_at INTEGER NOT NULL DEFAULT 0,
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

    fn thread_mode(conn: &rusqlite::Connection, thread_id: &str) -> String {
        conn.query_row(
            "SELECT mode FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
            |row| row.get(0),
        )
        .expect("thread mode")
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
        // The report echoes the text that is now in force, not a fixed word:
        // the same flag clears the segment with an empty value.
        assert_eq!(parsed["updated"]["appendSystemPrompt"], "be terse");

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

    /// A protected approval mode only asks while the tool permission level is
    /// `workspace`; at the legacy `all` the agent never consults the approval
    /// rules. The CLI says so instead of reporting a mode that will not prompt.
    #[tokio::test]
    async fn set_warns_that_a_protected_mode_needs_the_workspace_level() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .responses
            .insert("get_state".into(), r#"{"permissionLevel":"all"}"#.into());
        let (_agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--sandbox".into(),
                "manual".into(),
                "--json".into(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["updated"]["sandboxTier"], "manual");
        let notes = parsed["notes"].as_array().expect("a note");
        assert!(
            notes.iter().any(|note| note
                .as_str()
                .is_some_and(|note| note.contains("--permission workspace"))),
            "{notes:?}"
        );

        // The explicit level is the post-state: no note once it gates.
        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--sandbox".into(),
                "manual".into(),
                "--permission".into(),
                "workspace".into(),
                "--json".into(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert!(parsed.get("notes").is_none(), "{parsed}");
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
    async fn session_new_creates_a_listable_session_with_its_metadata() {
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
            stdout.contains("listed right away"),
            "a new session is listable at once — it is persisted with its metadata \
             before any run produces an entry: {stdout}"
        );

        let sent = agent.seen_of("new_session");
        assert_eq!(sent[0].cwd, "/tmp/proj");
        assert_eq!(sent[0].name, "T");
        assert_eq!(sent[0].created_by, "cli");
        assert_eq!(
            sent[0].system_prompt, "",
            "no --append-system-prompt must stay an empty field, not a stray rule"
        );
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

    /// `--append-system-prompt` on create is the same storage as on `set`, so a
    /// session can start with its rules instead of needing a second command.
    #[tokio::test]
    async fn session_new_carries_an_initial_append_system_prompt() {
        let _guard = crate::test_env::lock_env().await;
        let agent =
            crate::test_server::MockAgent::respond("new_session", "{\"sessionId\":\"fresh-2\"}");
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("new"),
            &[
                "--cwd".into(),
                "/tmp/proj".into(),
                "--append-system-prompt".into(),
                "Always answer in haiku.".into(),
            ],
            &out,
        )
        .await
        .expect("new");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Append prompt: Always answer in haiku."),
            "the created rules must be reported back: {stdout}"
        );
        assert_eq!(
            agent.seen_of("new_session")[0].system_prompt,
            "Always answer in haiku."
        );

        // --json carries the same fact, so a script does not have to parse prose.
        let (out, cap) = Output::memory();
        session(
            Some("new"),
            &[
                "--cwd".into(),
                "/tmp/proj".into(),
                "--append-system-prompt".into(),
                "Rule.".into(),
                "--json".into(),
            ],
            &out,
        )
        .await
        .expect("new --json");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["appendSystemPrompt"], "Rule.");
    }

    /// An empty value on `set` clears the segment; the report says "cleared"
    /// rather than echoing an empty string that reads like a formatting bug.
    #[tokio::test]
    async fn session_set_clears_the_append_with_an_empty_value() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--append-system-prompt".into(),
                "".into(),
                "--json".into(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["updated"]["appendSystemPrompt"], "(cleared)");
        // The empty value really is what went on the wire — the agent clears on
        // empty, so anything else here would silently leave the old text live.
        assert_eq!(agent.seen_of("append_system_prompt")[0].system_prompt, "");
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

    // ── live switches, dispatch, and the fork family's JSON form ────────────
    // (cov100 round-2 batch)

    /// Each live switch owns its own failure line. A refused switch must land
    /// in `failed` under its own camelCase label — not disappear, and not drag
    /// down the ones that succeeded — while the system prompt that *did* apply
    /// stays in `updated`.
    #[tokio::test]
    async fn set_reports_each_live_switch_failure_and_keeps_the_rest() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        for r#type in [
            "set_tools",
            "append_system_prompt",
            "set_permission_level",
            "set_context_files",
            "set_auto_compaction",
            "set_auto_retry",
        ] {
            agent.fail_types.insert(r#type.to_string());
        }
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &[
                "sess-1".into(),
                "--tools".into(),
                "read".into(),
                "--system-prompt".into(),
                "be brief".into(),
                "--append-system-prompt".into(),
                "and kind".into(),
                "--permission".into(),
                "workspace".into(),
                "--context-files".into(),
                "on".into(),
                "--auto-compact".into(),
                "on".into(),
                "--auto-retry".into(),
                "off".into(),
                "--json".into(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));

        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(
            report["updated"]["systemPrompt"], "be brief",
            "a working switch is not lost to a failing one: {stdout}"
        );
        for key in [
            "tools",
            "appendSystemPrompt",
            "permissionLevel",
            "contextFiles",
            "autoCompaction",
            "autoRetry",
        ] {
            assert!(
                report["failed"][key].is_string(),
                "{key} missing from failed: {stdout}"
            );
            assert!(
                report["updated"].get(key).is_none(),
                "{key} must not also be reported as applied: {stdout}"
            );
        }
        assert_eq!(agent.seen_of("set_permission_level")[0].level, "workspace");

        // The empty value is the restore path: it is reported as such and it is
        // what went on the wire, because the agent clears on empty.
        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--system-prompt".into(),
                "".into(),
                "--json".into(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["updated"]["systemPrompt"], "(built-in restored)");
        assert_eq!(
            agent
                .seen_of("set_system_prompt")
                .last()
                .unwrap()
                .system_prompt,
            ""
        );
    }

    /// The text form labels every switch it applied; the four labels below are
    /// reachable only on the paths that apply those switches.
    #[tokio::test]
    async fn set_text_labels_each_live_switch_it_applied() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        let (_agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("set"),
            &[
                "sess-1".into(),
                "--sandbox".into(),
                "auto".into(),
                "--context-files".into(),
                "on".into(),
                "--auto-compact".into(),
                "off".into(),
                "--auto-retry".into(),
                "on".into(),
            ],
            &out,
        )
        .await
        .expect("set");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(
            stdout,
            "Updated session sess-1\n  Approval:     auto\n  CtxFiles:     on\n  AutoCompact:  off\n  AutoRetry:    on\n"
        );
    }

    /// The subcommands that parse their own arguments are dispatched from
    /// `session`, and the live-session actions reach their own RPC instead of
    /// the journal path below.
    #[tokio::test]
    async fn session_dispatches_the_self_parsing_subcommands() {
        for name in ["transcript", "status", "reset", "wait"] {
            let (out, cap) = Output::memory();
            session(Some(name), &["--help".into()], &out)
                .await
                .unwrap_or_else(|err| panic!("{name}: {err}"));
            let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
            assert!(stdout.starts_with("future session"), "{name}: {stdout}");
        }

        // `new` has its own help (and its own parser) rather than the shared one.
        let (out, cap) = Output::memory();
        session(Some("new"), &["--help".into()], &out)
            .await
            .expect("new help");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("future session new"), "{stdout}");

        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::default();
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(Some("abort"), &["--help".into()], &out)
            .await
            .expect("abort help");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("future session abort <session-id>"),
            "{stdout}"
        );

        let (out, _cap) = Output::memory();
        session(Some("abort"), &["sess-1".into()], &out)
            .await
            .expect("abort");
        let seen = agent.seen_of("abort_session");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].session_id, "sess-1");
    }

    /// `--json` is the machine-readable form of the fork family too, and the
    /// fork call carries the entry id and target the caller named.
    #[tokio::test]
    async fn the_fork_family_reports_json_when_asked() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert(
            "get_fork_messages".into(),
            "{\"messages\":[{\"id\":\"e1\",\"blocks\":[{\"text\":\"hi\"}]}]}".into(),
        );
        agent
            .responses
            .insert("fork".into(), "{\"sessionId\":\"child-1\"}".into());
        agent
            .responses
            .insert("clone".into(), "{\"sessionId\":\"child-2\"}".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(Some("forks"), &["s1".into(), "--json".into()], &out)
            .await
            .expect("forks");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["sessionId"], "s1");
        assert_eq!(report["forks"][0]["id"], "e1");

        let (out, cap) = Output::memory();
        session(
            Some("fork"),
            &["s1".into(), "--entry".into(), "e1".into(), "--json".into()],
            &out,
        )
        .await
        .expect("fork");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout.trim(), "{\"sessionId\":\"child-1\"}");
        let fork = &agent.seen_of("fork")[0];
        assert_eq!(fork.entry_id, "e1");
        assert_eq!(fork.session_id, "s1");

        let (out, cap) = Output::memory();
        session(Some("clone"), &["s1".into(), "--json".into()], &out)
            .await
            .expect("clone");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout.trim(), "{\"sessionId\":\"child-2\"}");

        // The text form names the child too: a caller who did not ask for JSON
        // still needs the id to continue with.
        let (out, cap) = Output::memory();
        session(Some("clone"), &["s1".into()], &out)
            .await
            .expect("clone");
        assert_eq!(
            String::from_utf8(cap.out.lock().unwrap().clone()).unwrap(),
            "Cloned session s1 → child-2\n"
        );
    }

    /// A refused system prompt lands in `failed` under its own label, like every
    /// other switch — it is not silently dropped because the key is not a switch.
    #[tokio::test]
    async fn a_refused_system_prompt_is_reported_by_its_own_label() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("get_state".into(), "{}".into());
        agent.fail_types.insert("set_system_prompt".into());
        let (_agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        let result = session(
            Some("set"),
            &[
                "sess-1".into(),
                "--system-prompt".into(),
                "x".into(),
                "--json".into(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert!(report["failed"]["systemPrompt"].is_string(), "{stdout}");
        assert!(report["updated"].get("systemPrompt").is_none(), "{stdout}");
    }

    /// `title` and `export` report JSON with the fields the caller needs to
    /// continue — the model that answered, whether the rename was applied, and
    /// where the exported file ended up.
    #[tokio::test]
    async fn title_and_export_report_json() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert(
            "generate_session_title".into(),
            "{\"title\":\"Budget review\",\"model\":\"m1\"}".into(),
        );
        agent
            .responses
            .insert("export_html".into(), "{\"path\":\"/tmp/x.html\"}".into());
        let (agent, _env) = mock_env(agent).await;

        let (out, cap) = Output::memory();
        session(
            Some("title"),
            &["s1".into(), "--lang".into(), "zh".into(), "--json".into()],
            &out,
        )
        .await
        .expect("title");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["title"], "Budget review");
        assert_eq!(report["model"], "m1");
        assert_eq!(report["applied"], json!(false));
        assert_eq!(agent.seen_of("generate_session_title")[0].mode, "zh");

        let (out, cap) = Output::memory();
        session(Some("export"), &["s1".into(), "--json".into()], &out)
            .await
            .expect("export");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["sessionId"], "s1");
        assert_eq!(report["path"], "/tmp/x.html");
    }

    /// The fork family rejects an unknown or valueless option before it calls
    /// the agent, so a typo cannot silently fork at the wrong entry.
    #[tokio::test]
    async fn the_fork_family_rejects_unknown_and_valueless_options() {
        let cases: [(&str, &[&str], &str); 5] = [
            (
                "fork",
                &["s1", "--bogus"],
                "Unknown option: --bogus; see future session forks s1\n",
            ),
            ("fork", &["s1", "--entry"], "Missing value for --entry\n"),
            ("title", &["s1", "--lang"], "Missing value for --lang\n"),
            ("export", &["s1", "--out"], "Missing value for --out\n"),
            ("export", &["s1", "--bogus"], "Unknown option: --bogus\n"),
        ];
        for (name, argv, expected) in cases {
            let (out, cap) = Output::memory();
            let args: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
            let result = session(Some(name), &args, &out).await;
            assert_eq!(
                result,
                Err(crate::HANDLED_EXIT.to_string()),
                "{name} {argv:?}"
            );
            let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
            assert_eq!(stderr, expected, "{name} {argv:?}");
        }
    }

    /// An export the agent could not write has no path to report: saying so is
    /// the only useful outcome, because the alternative is printing an empty
    /// destination the caller would try to open.
    #[tokio::test]
    async fn export_refuses_an_agent_that_reported_no_path() {
        let _guard = crate::test_env::lock_env().await;
        let (_agent, _env) =
            mock_env(crate::test_server::MockAgent::respond("export_html", "{}")).await;
        let (out, cap) = Output::memory();
        let result = session(Some("export"), &["s1".into()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "The agent reported no export path.\n");
    }
}
