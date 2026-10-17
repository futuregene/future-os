//! `future mcp` — configure Model Context Protocol servers and call their tools.
//!
//! FutureOS has no MCP client inside the agent, so this group is the one place
//! an MCP server can be reached: it owns the server list (`~/.future/mcp.json`)
//! and the four steps of using one — list servers, page their tools, read one
//! tool's schema, then call it.
//!
//! Every step is bounded, because the servers this exists for are large: REA
//! advertises 207 tools (a 3.1 MB `tools/list`, 690 KB of JSON Schema) and can
//! answer a call with hundreds of megabytes. So `tools` pages by default,
//! `describe` takes one exact name, and `call` caps what it prints and points at
//! `--out` for the rest. Nothing here writes an unbounded amount of text to a
//! terminal an agent is reading.

use crate::commands::mcp_client::{
    max_frame_bytes_from_env, ServerProcess, ServerSpec, DEFAULT_TIMEOUT,
};
use crate::output::Output;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

/// Tools one `future mcp tools` page shows unless `--limit` says otherwise.
const DEFAULT_TOOL_LIMIT: usize = 50;

/// Ceiling on `--limit`: the paging exists to protect the reader's context, so
/// one flag must not be able to undo it.
const MAX_TOOL_LIMIT: usize = 500;

/// Longest tool description printed on one `tools` row.
const TOOL_ROW_DESCRIPTION: usize = 96;

/// What `future mcp call` prints unless `--max-bytes` says otherwise.
const DEFAULT_MAX_BYTES: usize = 64 * 1024;

/// Suggestions offered when a tool name does not match.
const MAX_SUGGESTIONS: usize = 5;

// ── Configuration ──────────────────────────────────────────────────────────

/// `~/.future/mcp.json`.
///
/// It sits at the FutureOS home root (beside `agent/`), not inside it: these
/// servers belong to the CLI, and moving `FUTURE_HOME` must move them with it
/// so an isolated instance cannot reach the real one's servers.
pub fn config_path() -> PathBuf {
    future_agent::utils::future_home().join("mcp.json")
}

/// The configured MCP servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, ServerSpec>,
}

/// Read the configuration. A missing or empty file is an empty configuration,
/// not an error — that is the state before the first `future mcp add`.
pub fn load_config() -> Result<McpConfig, String> {
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(McpConfig::default())
        }
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(McpConfig::default());
    }
    serde_json::from_str(&text).map_err(|error| format!("{} is not valid: {error}", path.display()))
}

/// Write the configuration, replacing the file atomically so a crash cannot
/// leave a half-written server list behind.
pub fn save_config(config: &McpConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut body = serde_json::to_string_pretty(config)
        .map_err(|error| format!("could not encode the MCP configuration: {error}"))?;
    body.push('\n');
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, body)
        .map_err(|error| format!("could not write {}: {error}", temp.display()))?;
    std::fs::rename(&temp, &path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        format!("could not replace {}: {error}", path.display())
    })
}

/// The named server, or an error that says what the alternatives are.
fn resolve<'config>(config: &'config McpConfig, name: &str) -> Result<&'config ServerSpec, String> {
    match config.servers.get(name) {
        Some(spec) if spec.enabled => Ok(spec),
        Some(_) => Err(format!(
            "MCP server '{name}' is disabled in {} — set \"enabled\": true to use it",
            config_path().display()
        )),
        None => {
            let mut message = format!("unknown MCP server '{name}'");
            if config.servers.is_empty() {
                message.push_str(&format!(
                    ": none are configured. Add one with `future mcp add`, which writes {}",
                    config_path().display()
                ));
            } else {
                let names: Vec<&str> = config.servers.keys().map(String::as_str).collect();
                message.push_str(&format!(" — configured: {}", names.join(", ")));
            }
            Err(message)
        }
    }
}

// ── Argument parsing ───────────────────────────────────────────────────────

/// Parsed `future mcp` arguments: positional values, `--flag value` pairs,
/// repeatable flags and bare switches.
#[derive(Debug)]
struct Flags {
    positional: Vec<String>,
    values: BTreeMap<String, String>,
    lists: BTreeMap<String, Vec<String>>,
    switches: Vec<String>,
}

/// Parse `--key value` pairs, `--key=value`, repeatable flags into `lists`, and
/// bare switches into `switches`.
fn parse_flags(
    args: &[String],
    taking_value: &[&str],
    repeatable: &[&str],
    switches: &[&str],
) -> Result<Flags, String> {
    let mut parsed = Flags {
        positional: Vec::new(),
        values: BTreeMap::new(),
        lists: BTreeMap::new(),
        switches: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if !arg.starts_with("--") {
            parsed.positional.push(arg.to_string());
            index += 1;
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg, None),
        };
        if switches.contains(&flag) {
            if inline.is_some() {
                return Err(format!("{flag} does not take a value"));
            }
            parsed.switches.push(flag.to_string());
            index += 1;
            continue;
        }
        if !taking_value.contains(&flag) && !repeatable.contains(&flag) {
            return Err(format!("unknown option: {flag}"));
        }
        let value = match inline {
            Some(value) => value,
            None => {
                // Repeatable flags take their value verbatim: `--arg --mcp` is
                // how an MCP server is spawned, so a leading `--` is data here,
                // not the next option. Single-value flags keep the guard, where
                // a following `--flag` means the value was forgotten.
                let next = args.get(index + 1);
                let value = if repeatable.contains(&flag) {
                    next.ok_or_else(|| format!("missing value for {flag}"))?
                } else {
                    next.filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| format!("missing value for {flag}"))?
                };
                index += 1;
                value.clone()
            }
        };
        if repeatable.contains(&flag) {
            parsed
                .lists
                .entry(flag.to_string())
                .or_default()
                .push(value);
        } else {
            parsed.values.insert(flag.to_string(), value);
        }
        index += 1;
    }
    Ok(parsed)
}

impl Flags {
    fn has(&self, flag: &str) -> bool {
        self.switches.iter().any(|switch| switch == flag)
    }

    fn value(&self, flag: &str) -> Option<&str> {
        self.values.get(flag).map(String::as_str)
    }

    fn list(&self, flag: &str) -> &[String] {
        self.lists.get(flag).map(Vec::as_slice).unwrap_or(&[])
    }

    fn usize_value(&self, flag: &str, default: usize, max: usize) -> Result<usize, String> {
        let Some(raw) = self.value(flag) else {
            return Ok(default);
        };
        let parsed: usize = raw
            .parse()
            .map_err(|_| format!("{flag} must be a non-negative integer"))?;
        if parsed > max {
            return Err(format!("{flag} must be at most {max}"));
        }
        Ok(parsed)
    }

    fn timeout(&self) -> Result<Duration, String> {
        match self.value("--timeout") {
            None => Ok(DEFAULT_TIMEOUT),
            Some(raw) => {
                let seconds: u64 = raw
                    .parse()
                    .map_err(|_| "--timeout must be a whole number of seconds".to_string())?;
                if seconds == 0 {
                    return Err("--timeout must be at least 1 second".to_string());
                }
                Ok(Duration::from_secs(seconds))
            }
        }
    }

    /// The frame cap for this run: `--max-frame-bytes` if given, otherwise
    /// `FUTURE_MCP_MAX_FRAME_BYTES`, otherwise the default.
    fn max_frame_bytes(&self) -> Result<usize, String> {
        match self.value("--max-frame-bytes") {
            None => Ok(max_frame_bytes_from_env()),
            Some(raw) => {
                let bytes: usize = raw
                    .parse()
                    .map_err(|_| "--max-frame-bytes must be a whole number of bytes".to_string())?;
                if bytes == 0 {
                    return Err("--max-frame-bytes must be at least 1 byte".to_string());
                }
                Ok(bytes)
            }
        }
    }

    fn exactly_one_positional(&self, what: &str, usage: &str) -> Result<String, String> {
        match self.positional.len() {
            1 => Ok(self.positional[0].clone()),
            0 => Err(format!("missing {what}\n\n{usage}")),
            _ => Err(format!(
                "unexpected argument: {}\n\n{usage}",
                self.positional[1]
            )),
        }
    }
}

// ── Group dispatch ─────────────────────────────────────────────────────────

pub fn is_mcp_command(command: Option<&str>) -> bool {
    matches!(
        command,
        Some("list" | "tools" | "describe" | "call" | "add" | "remove" | "status" | "serve")
    )
}

/// `mcp(command, args)` — the body behind each `future mcp` subcommand.
pub async fn mcp(command: &str, args: &[String], out: &Output) -> Result<(), String> {
    match command {
        "list" => mcp_list(args, out),
        "status" => mcp_status(args, out).await,
        "tools" => mcp_tools(args, out).await,
        "describe" => mcp_describe(args, out).await,
        "call" => mcp_call(args, out).await,
        "serve" => mcp_serve(args, out).await,
        "add" => mcp_add(args, out),
        "remove" => mcp_remove(args, out),
        other => Err(format!("unknown mcp command: {other}")),
    }
}

// ── status ─────────────────────────────────────────────────────────────────

const STATUS_USAGE: &str =
    "Usage: future mcp status [<server>] [--timeout <secs>] [--max-frame-bytes <n>] [--json]";

/// `future mcp status [server]` — is each server actually usable?
///
/// The handshake is the whole test: start it, initialize, count tools, stop.
/// That is one cheap round trip (no tool is called), and it is the question
/// `list` cannot answer, since a configured server may have no interpreter, no
/// engine, or a missing credential. Measured in the same shape `future doctor`
/// uses: a per-server verdict plus, on failure, the server's own stderr, which
/// is usually the real explanation.
async fn mcp_status(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &["--timeout", "--max-frame-bytes"], &[], &["--json"])?;
    if flags.positional.len() > 1 {
        return Err(format!(
            "expected at most one server name ({})\n\n{STATUS_USAGE}",
            flags.positional.len()
        ));
    }
    let timeout = flags.timeout()?;
    let max_frame_bytes = flags.max_frame_bytes()?;

    let config = load_config()?;
    let targets: Vec<String> = match flags.positional.first() {
        Some(name) => {
            resolve(&config, name)?;
            vec![name.clone()]
        }
        None => config.servers.keys().cloned().collect(),
    };
    if targets.is_empty() {
        out.log(&format!(
            "No MCP servers are configured.\n\nConfiguration file: {}\nAdd one:            future mcp add rea --command rea --arg --mcp",
            config_path().display()
        ));
        return Ok(());
    }

    let mut reports: Vec<Value> = Vec::new();
    let mut failed = 0usize;
    for name in &targets {
        // A disabled server is a configuration state, not a failure to reach it.
        let Some(spec) = config.servers.get(name) else {
            continue;
        };
        if !spec.enabled {
            reports.push(json!({ "name": name, "state": "disabled" }));
            continue;
        }
        let started = std::time::Instant::now();
        match ServerProcess::start_with_frame_cap(name, spec, timeout, max_frame_bytes).await {
            Err(error) => {
                failed += 1;
                reports.push(json!({
                    "name": name,
                    "state": "failed",
                    "error": error,
                    "handshakeMs": started.elapsed().as_millis() as u64,
                    "command": spec.command_line(),
                }));
            }
            Ok(mut process) => {
                let handshaked = started.elapsed();
                let listed = process.session().list_tools().await;
                let stderr = process.stderr_tail();
                process.shutdown().await;
                match listed {
                    Ok(tools) => reports.push(json!({
                        "name": name,
                        "state": "ok",
                        "toolCount": tools.len(),
                        "handshakeMs": handshaked.as_millis() as u64,
                        "totalMs": started.elapsed().as_millis() as u64,
                        "command": spec.command_line(),
                    })),
                    Err(error) => {
                        failed += 1;
                        reports.push(json!({
                            "name": name,
                            "state": "failed",
                            "error": attach_stderr(error, &stderr),
                            "handshakeMs": handshaked.as_millis() as u64,
                            "command": spec.command_line(),
                        }));
                    }
                }
            }
        }
    }

    if flags.has("--json") {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "configPath": config_path().display().to_string(),
                "servers": reports,
            }))
            .unwrap_or_default(),
        );
    } else {
        for report in &reports {
            let name = report["name"].as_str().unwrap_or_default();
            match report["state"].as_str().unwrap_or_default() {
                "ok" => out.log(&format!(
                    "{name:<16} ok      {} tool(s), handshake {} ms",
                    report["toolCount"], report["handshakeMs"]
                )),
                "disabled" => out.log(&format!("{name:<16} disabled")),
                _ => {
                    out.log(&format!("{name:<16} FAILED  {}", report["command"]));
                    if let Some(error) = report["error"].as_str() {
                        for line in error.lines() {
                            out.log(&format!("                 {line}"));
                        }
                    }
                }
            }
        }
        let ok = reports
            .iter()
            .filter(|report| report["state"] == "ok")
            .count();
        out.log(&format!("\n{ok}/{} server(s) reachable", targets.len()));
    }

    // Unreachable servers are a failed check, so this is scriptable; a disabled
    // one is a deliberate configuration and does not fail the command.
    if failed > 0 {
        return Err(crate::HANDLED_EXIT.to_string());
    }
    Ok(())
}

// ── serve ──────────────────────────────────────────────────────────────────

const SERVE_USAGE: &str =
    "Usage: future mcp serve <server> [--timeout <secs>] [--max-frame-bytes <n>]";

/// `future mcp serve <server>` — one long-lived session, driven by stdin.
///
/// Every other subcommand starts the server, uses it once and stops it, which is
/// free for a fast server but not for an engine that must load and analyse the
/// target on launch: with Ghidra that cold start is about four minutes, so five
/// expensive calls cost five cold starts. `serve` pays it once and reuses the
/// session.
///
/// The protocol is newline-delimited JSON in both directions, so it composes with
/// `jq` and with a shell that keeps one process alive:
///
/// ```text
/// -> {"tool":"inspect_go_binary","args":{"path":"/usr/bin/x"}}
/// <- {"ok":true,"isError":false,"result":{...}}
/// ```
///
/// The first line out is a ready ack, so a caller can wait for it instead of
/// guessing how long analysis takes. Requests are answered in order, one at a
/// time: a stateful engine session is not something to drive concurrently.
async fn mcp_serve(args: &[String], out: &Output) -> Result<(), String> {
    mcp_serve_with_stdin(args, out, tokio::io::BufReader::new(tokio::io::stdin())).await
}

/// [`mcp_serve`] over an explicit reader.
///
/// Production passes the process's own stdin. Taking the reader as a parameter
/// is what makes this command's *own* paths reachable: a stdin whose reads fail
/// (rather than ending at EOF) and the diagnostics-and-shutdown that follow the
/// request loop both run after the happy path, on a real spawned server, which
/// an in-process test cannot provide through a pipe it does not own.
async fn mcp_serve_with_stdin<R: tokio::io::AsyncBufRead + Unpin>(
    args: &[String],
    out: &Output,
    reader: R,
) -> Result<(), String> {
    let flags = parse_flags(args, &["--timeout", "--max-frame-bytes"], &[], &[])?;
    let name = flags.exactly_one_positional("a server name", SERVE_USAGE)?;
    let timeout = flags.timeout()?;
    let max_frame_bytes = flags.max_frame_bytes()?;

    let config = load_config()?;
    let spec = resolve(&config, &name)?;
    let mut process =
        ServerProcess::start_with_frame_cap(&name, spec, timeout, max_frame_bytes).await?;

    // Announce readiness on stdout, and only then start answering: the caller can
    // block on this line rather than on a timeout.
    let ready = process.session().list_tools().await;
    let stderr = process.stderr_tail();
    let tools = match ready {
        Ok(tools) => tools,
        Err(error) => {
            process.shutdown().await;
            return Err(attach_stderr(error, &stderr));
        }
    };
    out.log(
        &json!({
            "ok": true,
            "ready": true,
            "server": name,
            "toolCount": tools.len(),
        })
        .to_string(),
    );

    serve_requests(&mut process, out, reader).await?;

    let stderr = process.stderr_tail();
    process.shutdown().await;
    // Diagnostics go to stderr, so stdout stays a clean JSON stream.
    let tail = stderr.trim();
    if !tail.is_empty() {
        out.log_err(tail);
    }
    Ok(())
}

/// The `serve` request loop, over any reader (stdin in production).
///
/// Split out so the protocol is testable without a pipe: malformed JSON, a
/// request with no `tool`, a call the server fails, and ordering are all part of
/// this contract, and all of them must leave the session running.
async fn serve_requests<R: tokio::io::AsyncBufRead + Unpin>(
    process: &mut ServerProcess,
    out: &Output,
    reader: R,
) -> Result<(), String> {
    let mut lines = reader.lines();
    loop {
        let next = lines.next_line().await;
        let line = match next {
            Ok(Some(line)) => line,
            // EOF: the caller is done with this session.
            Ok(None) => break,
            Err(error) => {
                out.log(&json!({ "ok": false, "error": format!("stdin: {error}") }).to_string());
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                out.log(
                    &json!({
                        "ok": false,
                        "error": format!("request is not valid JSON: {error}"),
                    })
                    .to_string(),
                );
                continue;
            }
        };
        let Some(tool) = request.get("tool").and_then(Value::as_str) else {
            out.log(
                &json!({
                    "ok": false,
                    "error": "request needs a 'tool' string; see future mcp serve --help",
                })
                .to_string(),
            );
            continue;
        };
        let arguments = request.get("args").cloned().unwrap_or_else(|| json!({}));
        match process.session().call_tool(tool, arguments).await {
            Ok(result) => {
                let reported = result
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                out.log(&json!({ "ok": true, "isError": reported, "result": result }).to_string());
            }
            Err(error) => {
                // A failed call does not end the session: the other requests may
                // still be fine, and the point of `serve` is to stay up. The
                // server's own stderr is attached because it is usually the real
                // reason (a missing engine, a missing credential).
                let detail = attach_stderr(error, &process.stderr_tail());
                out.log(&json!({ "ok": false, "error": detail }).to_string());
            }
        }
    }
    Ok(())
}

// ── list ───────────────────────────────────────────────────────────────────

const LIST_USAGE: &str = "Usage: future mcp list [--json]";

fn mcp_list(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &[], &[], &["--json"])?;
    if let Some(extra) = flags.positional.first() {
        return Err(format!("unexpected argument: {extra}\n\n{LIST_USAGE}"));
    }
    let config = load_config()?;

    if flags.has("--json") {
        let servers: Vec<Value> = config
            .servers
            .iter()
            .map(|(name, spec)| {
                json!({
                    "name": name,
                    "command": spec.command,
                    "args": spec.args,
                    "cwd": spec.cwd,
                    "enabled": spec.enabled,
                })
            })
            .collect();
        out.log(
            &serde_json::to_string_pretty(&json!({
                "configPath": config_path().display().to_string(),
                "servers": servers,
            }))
            .unwrap_or_default(),
        );
        return Ok(());
    }

    if config.servers.is_empty() {
        out.log(&format!(
            "No MCP servers are configured.\n\nConfiguration file: {}\nAdd one:            future mcp add rea --command rea --arg --mcp",
            config_path().display()
        ));
        return Ok(());
    }
    for (name, spec) in &config.servers {
        let state = if spec.enabled { "" } else { "  [disabled]" };
        out.log(&format!("{name:<16} {}{state}", spec.command_line()));
    }
    out.log(&format!(
        "\n{} server(s) configured in {}",
        config.servers.len(),
        config_path().display()
    ));
    Ok(())
}

// ── tools ──────────────────────────────────────────────────────────────────

const TOOLS_USAGE: &str = "Usage: future mcp tools <server> [--query <text>] [--limit <n>] [--offset <n>] [--timeout <secs>] [--max-frame-bytes <n>] [--json]";

async fn mcp_tools(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(
        args,
        &[
            "--query",
            "--limit",
            "--offset",
            "--timeout",
            "--max-frame-bytes",
        ],
        &[],
        &["--json"],
    )?;
    let name = flags.exactly_one_positional("a server name", TOOLS_USAGE)?;
    let limit = flags.usize_value("--limit", DEFAULT_TOOL_LIMIT, MAX_TOOL_LIMIT)?;
    let offset = flags.usize_value("--offset", 0, usize::MAX)?;
    let timeout = flags.timeout()?;
    let max_frame_bytes = flags.max_frame_bytes()?;
    let query = flags.value("--query").map(str::to_lowercase);

    let config = load_config()?;
    let spec = resolve(&config, &name)?;
    let mut process =
        ServerProcess::start_with_frame_cap(&name, spec, timeout, max_frame_bytes).await?;
    let listed = process.session().list_tools().await;
    let stderr = process.stderr_tail();
    process.shutdown().await;
    let tools = listed.map_err(|error| attach_stderr(error, &stderr))?;

    let matching: Vec<&Value> = tools
        .iter()
        .filter(|tool| match query.as_deref() {
            None => true,
            Some(query) => tool_matches(tool, query),
        })
        .collect();
    let total = matching.len();
    let page: Vec<&&Value> = matching.iter().skip(offset).take(limit).collect();
    let next_offset = offset + page.len();
    let has_more = next_offset < total;

    if flags.has("--json") {
        let rows: Vec<Value> = page
            .iter()
            .map(|tool| {
                json!({
                    "name": tool_name(tool),
                    "description": tool_description(tool),
                })
            })
            .collect();
        out.log(
            &serde_json::to_string_pretty(&json!({
                "server": name,
                "total": total,
                "offset": offset,
                "limit": limit,
                "tools": rows,
                "nextOffset": if has_more { json!(next_offset) } else { Value::Null },
            }))
            .unwrap_or_default(),
        );
        return Ok(());
    }

    match query {
        Some(query) => out.log(&format!(
            "Tools of '{name}' matching \"{query}\" ({total} of {}):\n",
            tools.len()
        )),
        None => out.log(&format!("Tools of '{name}' ({total}):\n")),
    }
    for tool in &page {
        out.log(&format!(
            "{:<28} {}",
            tool_name(tool),
            shorten(&one_line(&tool_description(tool)), TOOL_ROW_DESCRIPTION)
        ));
    }
    out.log(&format!("\nShowing {} of {total} tools.", page.len()));
    if page.is_empty() && total > 0 {
        out.log(&format!(
            "--offset {offset} is past the end of this list; the last page starts at --offset {}.",
            total - 1
        ));
    }
    if has_more {
        out.log(&format!(
            "More tools exist: raise --limit or narrow with --query (next: --offset {next_offset})."
        ));
    }
    out.log(&format!(
        "Full schema for one: future mcp describe {name} <tool>"
    ));
    Ok(())
}

fn tool_name(tool: &Value) -> String {
    tool.get("name")
        .and_then(Value::as_str)
        .unwrap_or("<unnamed>")
        .to_string()
}

fn tool_description(tool: &Value) -> String {
    tool.get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Case-insensitive substring match over the name and the description.
fn tool_matches(tool: &Value, lowercase_query: &str) -> bool {
    tool_name(tool).to_lowercase().contains(lowercase_query)
        || tool_description(tool)
            .to_lowercase()
            .contains(lowercase_query)
}

// ── describe ───────────────────────────────────────────────────────────────

const DESCRIBE_USAGE: &str =
    "Usage: future mcp describe <server> <tool> [--timeout <secs>] [--max-frame-bytes <n>] [--json]";

async fn mcp_describe(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &["--timeout", "--max-frame-bytes"], &[], &["--json"])?;
    if flags.positional.len() != 2 {
        return Err(format!(
            "expected a server and a tool name ({})\n\n{DESCRIBE_USAGE}",
            flags.positional.len()
        ));
    }
    let name = flags.positional[0].clone();
    let wanted = flags.positional[1].clone();
    let timeout = flags.timeout()?;
    let max_frame_bytes = flags.max_frame_bytes()?;

    let config = load_config()?;
    let spec = resolve(&config, &name)?;
    let mut process =
        ServerProcess::start_with_frame_cap(&name, spec, timeout, max_frame_bytes).await?;
    let listed = process.session().list_tools().await;
    let stderr = process.stderr_tail();
    process.shutdown().await;
    let tools = listed.map_err(|error| attach_stderr(error, &stderr))?;

    let Some(tool) = tools.iter().find(|tool| tool_name(tool) == wanted) else {
        return Err(missing_tool_error(&name, &wanted, &tools));
    };

    if flags.has("--json") {
        out.log(&serde_json::to_string_pretty(tool).unwrap_or_default());
        return Ok(());
    }
    out.log(&format!("{}\n", tool_name(tool)));
    let description = tool_description(tool);
    if !description.trim().is_empty() {
        out.log(description.trim());
        out.log("");
    }
    let schema = tool.get("inputSchema").cloned().unwrap_or(Value::Null);
    out.log("Arguments (JSON Schema):");
    out.log(&serde_json::to_string_pretty(&schema).unwrap_or_default());
    Ok(())
}

/// "no such tool" with the nearest alternatives — shared by `describe` and
/// `call`, so a typo reads the same either way.
fn missing_tool_error(server: &str, wanted: &str, tools: &[Value]) -> String {
    let mut message = format!("server '{server}' has no tool named '{wanted}'");
    let suggestions = suggest_tools(tools, wanted);
    if suggestions.is_empty() {
        message.push_str(&format!(
            "\nList what it has: future mcp tools {server} --query <text>"
        ));
    } else {
        message.push_str(&format!("\nDid you mean: {}", suggestions.join(", ")));
    }
    message
}

/// Up to [`MAX_SUGGESTIONS`] tool names resembling `wanted`, for the error text.
///
/// Scored by token *rarity*, not just containment: `inspect_ghidra_binary` must not
/// be answered with `inspect_adb_client`, which shares only the prefix every other
/// inspection tool also has. A token found in fewer names identifies one better,
/// which is what separates "binary" from "inspect".
fn suggest_tools(tools: &[Value], wanted: &str) -> Vec<String> {
    let wanted = wanted.to_lowercase();
    let names: Vec<String> = tools.iter().map(tool_name).collect();
    let lowercase: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();

    let wanted_tokens: Vec<&str> = wanted.split('_').filter(|token| token.len() >= 3).collect();
    // How many tool names contain each wanted token; a rare one scores higher.
    let mut rarity = vec![0.0f64; wanted_tokens.len()];
    for (index, token) in wanted_tokens.iter().enumerate() {
        let documents = lowercase.iter().filter(|name| name.contains(token)).count();
        rarity[index] = 1.0 / documents.max(1) as f64;
    }

    let mut scored: Vec<(f64, usize, &str)> = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let lower = &lowercase[index];
        let mut score = 0.0;
        if lower.contains(&wanted) || wanted.contains(lower.as_str()) {
            // One name is a prefix/extension of the other: the strongest signal.
            score += 1_000.0;
        }
        for (position, token) in wanted_tokens.iter().enumerate() {
            if lower.contains(token) {
                score += rarity[position];
            }
        }
        if score > 0.0 {
            scored.push((score, name.len(), name.as_str()));
        }
    }
    // Best score, then the shortest name, then alphabetical: stable and readable.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(b.2))
    });
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, name)| name.to_string())
        .collect()
}

// ── call ───────────────────────────────────────────────────────────────────

const CALL_USAGE: &str = "Usage: future mcp call <server> <tool> [--args '<json>'] [--out <file>] [--max-bytes <n>] [--timeout <secs>] [--max-frame-bytes <n>] [--json]";

async fn mcp_call(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(
        args,
        &[
            "--args",
            "--out",
            "--max-bytes",
            "--timeout",
            "--max-frame-bytes",
        ],
        &[],
        &["--json"],
    )?;
    if flags.positional.len() != 2 {
        return Err(format!(
            "expected a server and a tool name ({})\n\n{CALL_USAGE}",
            flags.positional.len()
        ));
    }
    let name = flags.positional[0].clone();
    let tool = flags.positional[1].clone();
    let timeout = flags.timeout()?;
    let max_frame_bytes = flags.max_frame_bytes()?;
    let max_bytes = flags.usize_value("--max-bytes", DEFAULT_MAX_BYTES, usize::MAX)?;
    let arguments = match flags.value("--args") {
        None => json!({}),
        Some(raw) => {
            let parsed: Value = serde_json::from_str(raw)
                .map_err(|error| format!("--args is not valid JSON: {error}"))?;
            if !parsed.is_object() {
                return Err("--args must be a JSON object of tool arguments".to_string());
            }
            parsed
        }
    };

    let config = load_config()?;
    let spec = resolve(&config, &name)?;
    let mut process =
        ServerProcess::start_with_frame_cap(&name, spec, timeout, max_frame_bytes).await?;
    let called = process.session().call_tool(&tool, arguments).await;

    // A failed call may simply be a mistyped tool name, which the server reports
    // in its own words. Only then is the tool list fetched, so the happy path
    // pays nothing for the check (REA's list is 3.1 MB).
    let called = match called {
        Ok(result) => Ok(result),
        Err(error) => match process.session().list_tools().await {
            Ok(tools) if !tools.iter().any(|entry| tool_name(entry) == tool) => {
                Err(missing_tool_error(&name, &tool, &tools))
            }
            _ => Err(error),
        },
    };
    let stderr = process.stderr_tail();
    process.shutdown().await;
    let result = called.map_err(|error| attach_stderr(error, &stderr))?;

    let rendered = if flags.has("--json") {
        serde_json::to_string_pretty(&result).unwrap_or_default()
    } else {
        render_tool_result(&result)
    };
    let reported_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut header: Vec<String> = Vec::new();
    if reported_error {
        header.push("[the tool reported an error]".to_string());
    }
    let mut body = rendered;
    if body.len() > max_bytes {
        let full_len = body.len();
        if let Some(path) = flags.value("--out") {
            std::fs::write(path, body.as_bytes())
                .map_err(|error| format!("could not write {path}: {error}"))?;
            header.push(format!(
                "[first {max_bytes} bytes of {full_len}; the whole result is in {path}]"
            ));
        } else {
            header.push(format!(
                "[first {max_bytes} bytes of {full_len} — rerun with --out <file> to write it all]"
            ));
        }
        body = truncate_at_char_boundary(&body, max_bytes);
    } else if let Some(path) = flags.value("--out") {
        std::fs::write(path, body.as_bytes())
            .map_err(|error| format!("could not write {path}: {error}"))?;
        header.push(format!("[the whole result is also in {path}]"));
    }

    // A tool that reported an error is a failed command, exactly as it is for
    // `future tools call`: the payload goes to stderr and the exit code is 1, so
    // a script or an agent does not have to parse a marker out of stdout to
    // learn that the call did not work. `--out` still wrote the full text.
    if reported_error {
        let mut message = header.join("\n");
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(&body);
        out.log_err(&message);
        return Err(crate::HANDLED_EXIT.to_string());
    }

    for line in header {
        out.log(&line);
    }
    out.log(&body);
    Ok(())
}

/// The text of an MCP `tools/call` result.
///
/// MCP carries content as typed blocks. Text is what a terminal can show, so
/// text blocks are joined and everything else is named rather than dumped; a
/// result with no `content` at all falls back to its own JSON.
fn render_tool_result(result: &Value) -> String {
    let Some(blocks) = result.get("content").and_then(Value::as_array) else {
        if result.is_null() {
            return "(the tool returned nothing)".to_string();
        }
        return serde_json::to_string_pretty(result).unwrap_or_default();
    };
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => parts.push(
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            ),
            Some(kind) => {
                let detail = block
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .or_else(|| block.get("uri").and_then(Value::as_str))
                    .unwrap_or("");
                parts.push(if detail.is_empty() {
                    format!("[{kind} content]")
                } else {
                    format!("[{kind} content: {detail}]")
                });
            }
            None => parts.push("[unrecognized content block omitted]".to_string()),
        }
    }
    if parts.is_empty() {
        return "(the tool returned no content)".to_string();
    }
    parts.join("\n")
}

fn truncate_at_char_boundary(text: &str, max_bytes: usize) -> String {
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

// ── add / remove ───────────────────────────────────────────────────────────

const ADD_USAGE: &str = "Usage: future mcp add <name> --command <program> [--arg <value>]... [--env KEY=VALUE]... [--cwd <dir>]";

fn mcp_add(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &["--command", "--cwd"], &["--arg", "--env"], &[])?;
    let name = flags.exactly_one_positional("a server name", ADD_USAGE)?;
    if !valid_server_name(&name) {
        return Err(format!(
            "'{name}' is not a usable server name — use letters, digits, '.', '_' or '-'"
        ));
    }
    let command = flags
        .value("--command")
        .ok_or_else(|| format!("missing --command\n\n{ADD_USAGE}"))?
        .to_string();

    let mut env = Map::new();
    for pair in flags.list("--env") {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("--env expects KEY=VALUE, got '{pair}'"))?;
        if key.is_empty() {
            return Err(format!("--env expects KEY=VALUE, got '{pair}'"));
        }
        env.insert(key.to_string(), Value::String(value.to_string()));
    }

    let mut config = load_config()?;
    if config.servers.contains_key(&name) {
        return Err(format!(
            "MCP server '{name}' already exists in {} — remove it first with `future mcp remove {name}`",
            config_path().display()
        ));
    }
    config.servers.insert(
        name.clone(),
        ServerSpec {
            command,
            args: flags.list("--arg").to_vec(),
            env,
            cwd: flags.value("--cwd").map(str::to_string),
            enabled: true,
        },
    );
    save_config(&config)?;

    let spec = &config.servers[&name];
    out.log(&format!(
        "Added MCP server '{name}': {}\nWritten to {}",
        spec.command_line(),
        config_path().display()
    ));
    out.log(&format!("Check it: future mcp tools {name}"));
    Ok(())
}

const REMOVE_USAGE: &str = "Usage: future mcp remove <name>";

fn mcp_remove(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &[], &[], &[])?;
    let name = flags.exactly_one_positional("a server name", REMOVE_USAGE)?;
    let mut config = load_config()?;
    if config.servers.remove(&name).is_none() {
        return Err(format!(
            "no MCP server named '{name}' in {}",
            config_path().display()
        ));
    }
    save_config(&config)?;
    out.log(&format!(
        "Removed MCP server '{name}' from {}",
        config_path().display()
    ));
    Ok(())
}

fn valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

// ── Shared helpers ─────────────────────────────────────────────────────────

/// Append a server's stderr to a failure, when it wrote any. The last lines are
/// usually the whole explanation (a missing engine, a bad key, a stack trace).
fn attach_stderr(error: String, stderr: &str) -> String {
    if stderr.trim().is_empty() {
        error
    } else {
        format!(
            "{error}\n--- server stderr (last lines) ---\n{}",
            stderr.trim_end()
        )
    }
}

/// Collapse a description to one line, so a listing row stays one row.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Shorten to `max` characters, marking that it was shortened.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn is_mcp_command_covers_every_documented_subcommand() {
        for command in [
            "list", "tools", "describe", "call", "add", "remove", "status", "serve",
        ] {
            assert!(is_mcp_command(Some(command)), "{command}");
        }
        assert!(!is_mcp_command(None));
        assert!(!is_mcp_command(Some("bogus")));
    }

    #[test]
    fn flags_split_values_from_switches_and_repeats() {
        let parsed = parse_flags(
            &args(&[
                "rea", "--query", "ipc", "--json", "--arg", "a", "--arg", "b",
            ]),
            &["--query"],
            &["--arg"],
            &["--json"],
        )
        .unwrap();
        assert_eq!(parsed.positional, vec!["rea"]);
        assert_eq!(parsed.value("--query"), Some("ipc"));
        assert!(parsed.has("--json"));
        assert_eq!(parsed.list("--arg"), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn flags_accept_equals_form_and_reject_unknown_options() {
        let parsed = parse_flags(&args(&["--query=ipc"]), &["--query"], &[], &[]).unwrap();
        assert_eq!(parsed.value("--query"), Some("ipc"));

        let error = parse_flags(&args(&["--bogus"]), &[], &[], &[]).unwrap_err();
        assert!(error.contains("unknown option: --bogus"), "{error}");
    }

    #[test]
    fn repeatable_flags_take_a_leading_dash_as_data() {
        // `future mcp add rea --command rea --arg --mcp` is the documented way
        // to configure a server, so `--arg` must accept `--mcp`.
        let parsed = parse_flags(
            &args(&["rea", "--command", "rea", "--arg", "--mcp"]),
            &["--command"],
            &["--arg"],
            &[],
        )
        .unwrap();
        assert_eq!(parsed.value("--command"), Some("rea"));
        assert_eq!(parsed.list("--arg"), vec!["--mcp".to_string()]);
    }

    #[test]
    fn a_flag_without_its_value_is_an_error() {
        let error = parse_flags(&args(&["--query"]), &["--query"], &[], &[]).unwrap_err();
        assert!(error.contains("missing value for --query"), "{error}");
        // A following flag is not a value.
        let error = parse_flags(
            &args(&["--query", "--json"]),
            &["--query"],
            &[],
            &["--json"],
        )
        .unwrap_err();
        assert!(error.contains("missing value for --query"), "{error}");
    }

    #[test]
    fn limit_is_capped_so_paging_cannot_be_undone() {
        let parsed = parse_flags(&args(&["--limit", "500"]), &["--limit"], &[], &[]).unwrap();
        assert_eq!(
            parsed
                .usize_value("--limit", DEFAULT_TOOL_LIMIT, MAX_TOOL_LIMIT)
                .unwrap(),
            500
        );
        let parsed = parse_flags(&args(&["--limit", "5000"]), &["--limit"], &[], &[]).unwrap();
        let error = parsed
            .usize_value("--limit", DEFAULT_TOOL_LIMIT, MAX_TOOL_LIMIT)
            .unwrap_err();
        assert!(error.contains("at most 500"), "{error}");
    }

    #[test]
    fn a_non_numeric_timeout_is_rejected() {
        let parsed = parse_flags(&args(&["--timeout", "soon"]), &["--timeout"], &[], &[]).unwrap();
        assert!(parsed.timeout().unwrap_err().contains("whole number"));
    }

    #[test]
    fn describe_suggestions_prefer_the_closest_names() {
        let tools = vec![
            json!({ "name": "inspect_go_binary" }),
            json!({ "name": "inspect_rust_binary" }),
            json!({ "name": "analyze_javascript_application" }),
        ];
        let suggestions = suggest_tools(&tools, "inspect_go");
        assert_eq!(suggestions[0], "inspect_go_binary");
        assert!(!suggestions
            .iter()
            .any(|name| name == "analyze_javascript_application"));
    }

    /// A rare token must beat a shared prefix. Against real REA,
    /// `inspect_ghidra_binary` used to be answered with `inspect_adb_client`,
    /// which shares only "inspect" — the prefix of most of the 207 tools.
    #[test]
    fn suggestions_prefer_a_distinctive_token_over_a_common_prefix() {
        let tools = vec![
            json!({ "name": "inspect_adb_client" }),
            json!({ "name": "inspect_adb_device" }),
            json!({ "name": "inspect_adb_package" }),
            json!({ "name": "inspect_go_binary" }),
            json!({ "name": "inspect_pe_binary" }),
        ];
        let suggestions = suggest_tools(&tools, "inspect_ghidra_binary");
        // "binary" is shared by two names, "inspect" by all five: the two binary
        // tools must come first.
        assert!(
            suggestions[0].contains("binary") && suggestions[1].contains("binary"),
            "{suggestions:?}"
        );
        assert!(
            !suggestions[..2].iter().any(|name| name.contains("adb")),
            "{suggestions:?}"
        );
    }

    #[test]
    fn text_content_is_joined_and_other_blocks_are_named() {
        let result = json!({
            "content": [
                { "type": "text", "text": "line one" },
                { "type": "image", "mimeType": "image/png" },
                { "type": "text", "text": "line two" },
            ],
        });
        assert_eq!(
            render_tool_result(&result),
            "line one\n[image content: image/png]\nline two"
        );
    }

    #[test]
    fn a_result_without_content_falls_back_to_its_own_json() {
        assert_eq!(
            render_tool_result(&json!({ "structuredContent": { "ok": true } })),
            "{\n  \"structuredContent\": {\n    \"ok\": true\n  }\n}"
        );
        assert_eq!(
            render_tool_result(&Value::Null),
            "(the tool returned nothing)"
        );
        assert_eq!(
            render_tool_result(&json!({ "content": [] })),
            "(the tool returned no content)"
        );
    }

    #[test]
    fn truncation_lands_on_a_character_boundary() {
        let text = "日本語テキスト";
        let cut = truncate_at_char_boundary(text, 4);
        assert!(cut.len() <= 4);
        assert!(text.starts_with(&cut));
        assert_eq!(truncate_at_char_boundary("short", 40), "short");
    }

    #[test]
    fn listing_rows_stay_on_one_line() {
        assert_eq!(one_line("a\nb\tc  d"), "a b c d");
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(shorten("abc", 4), "abc");
    }

    #[test]
    fn server_names_are_restricted_to_safe_characters() {
        assert!(valid_server_name("rea"));
        assert!(valid_server_name("my.server_2"));
        assert!(!valid_server_name(""));
        assert!(!valid_server_name("has space"));
        assert!(!valid_server_name("has/slash"));
    }

    #[test]
    fn config_round_trips_through_the_documented_shape() {
        let config: McpConfig = serde_json::from_str(
            r#"{
              "servers": {
                "rea": { "command": "rea", "args": ["--mcp"] },
                "off": { "command": "x", "enabled": false }
              }
            }"#,
        )
        .unwrap();
        assert_eq!(config.servers["rea"].command_line(), "rea --mcp");
        assert!(config.servers["rea"].enabled, "enabled defaults to true");
        assert!(!config.servers["off"].enabled);
    }

    #[test]
    fn an_unknown_key_in_the_config_is_an_error_not_a_silent_ignore() {
        let error = serde_json::from_str::<McpConfig>(
            r#"{ "servers": { "rea": { "command": "rea", "url": "http://x" } } }"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("url"), "{error}");
    }

    #[test]
    fn an_empty_document_is_an_empty_configuration() {
        let config: McpConfig = serde_json::from_str("{}").unwrap();
        assert!(config.servers.is_empty());
    }

    // ── The configuration file ─────────────────────────────────────────────

    /// A throwaway FutureOS home, optionally holding `body` as its `mcp.json`.
    fn home_with_config(body: Option<&str>) -> (tempfile::TempDir, crate::test_env::EnvGuard) {
        let home = tempfile::tempdir().expect("tempdir");
        if let Some(body) = body {
            std::fs::write(home.path().join("mcp.json"), body).unwrap();
        }
        let guard =
            crate::test_env::EnvGuard::set(&[("FUTURE_HOME", home.path().as_os_str().to_owned())]);
        (home, guard)
    }

    fn rendered(captured: &crate::output::Captured) -> (String, String) {
        let out = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
        let err = String::from_utf8(captured.err.lock().unwrap().clone()).unwrap();
        (out, err)
    }

    fn a_spec(command: &str) -> ServerSpec {
        ServerSpec {
            command: command.to_string(),
            args: Vec::new(),
            env: Map::new(),
            cwd: None,
            enabled: true,
        }
    }

    /// A missing file, a blank file and a file that cannot be read are three
    /// different states, and only the last one is an error: a blank file is
    /// what `save_config` leaves behind, not a broken configuration.
    #[tokio::test]
    async fn config_reads_an_absent_blank_and_unreadable_file() {
        let _lock = crate::test_env::lock_env().await;

        // No file at all: the state before the first `future mcp add`.
        {
            let (_home, _env) = home_with_config(None);
            assert!(load_config().unwrap().servers.is_empty());
        }

        // A file that exists but says nothing is the same state.
        {
            let (_home, _env) = home_with_config(Some("  \n"));
            assert!(load_config().unwrap().servers.is_empty());
        }

        // Text that is not JSON names the file it came from.
        {
            let (_home, _env) = home_with_config(Some("{ not json"));
            let error = load_config().unwrap_err();
            assert!(error.contains("mcp.json is not valid"), "{error}");
        }

        // A path that cannot be read as a file (here: a directory) is reported
        // rather than silently read as "nothing configured".
        {
            let home = tempfile::tempdir().unwrap();
            std::fs::create_dir(home.path().join("mcp.json")).unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let error = load_config().unwrap_err();
            assert!(error.contains("could not read"), "{error}");
            assert!(error.contains("mcp.json"), "{error}");
        }
    }

    /// `save_config` creates the directory it needs, replaces the file
    /// atomically (no temp file survives), and reports both ways it can fail
    /// instead of leaving a half-written server list behind.
    #[tokio::test]
    async fn save_config_creates_its_parent_and_reports_its_failures() {
        let _lock = crate::test_env::lock_env().await;
        let mut config = McpConfig::default();
        config.servers.insert("rea".to_string(), a_spec("rea"));

        // A home that does not exist yet: the directory is created.
        {
            let outer = tempfile::tempdir().unwrap();
            let home = outer.path().join("fresh");
            let _env =
                crate::test_env::EnvGuard::set(&[("FUTURE_HOME", home.as_os_str().to_owned())]);
            save_config(&config).unwrap();
            let written: McpConfig =
                serde_json::from_str(&std::fs::read_to_string(home.join("mcp.json")).unwrap())
                    .unwrap();
            assert_eq!(written.servers["rea"].command, "rea");
            assert!(
                !home.join("mcp.json.tmp").exists(),
                "the temp file is renamed away, not left behind"
            );
        }

        // A directory that cannot be created (a regular file is in the way) is
        // reported: the caller must not be told the list was saved.
        {
            let blocked = tempfile::tempdir().unwrap();
            let file = blocked.path().join("file");
            std::fs::write(&file, "not a directory").unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                file.join("home").as_os_str().to_owned(),
            )]);
            let error = save_config(&config).unwrap_err();
            assert!(error.contains("could not create"), "{error}");
        }

        // A destination that cannot be replaced (a directory sits at
        // `mcp.json`) is reported too, and the temp file is cleaned up.
        {
            let home = tempfile::tempdir().unwrap();
            std::fs::create_dir(home.path().join("mcp.json")).unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let error = save_config(&config).unwrap_err();
            assert!(error.contains("could not replace"), "{error}");
            assert!(
                !home.path().join("mcp.json.tmp").exists(),
                "the temp file must not survive a failed replace"
            );
        }
    }

    /// `resolve` distinguishes the three ways a name can fail: configured but
    /// disabled, unknown with alternatives, unknown with nothing configured.
    #[tokio::test]
    async fn resolve_reports_a_disabled_server_and_names_the_alternatives() {
        let _lock = crate::test_env::lock_env().await;
        let (_home, _env) = home_with_config(None);
        let config: McpConfig = serde_json::from_str(
            r#"{ "servers": {
                   "off":  { "command": "x", "enabled": false },
                   "live": { "command": "y" }
                 } }"#,
        )
        .unwrap();

        let error = resolve(&config, "off").unwrap_err();
        assert!(error.contains("'off' is disabled"), "{error}");
        assert!(error.contains("mcp.json"), "{error}");
        assert_eq!(resolve(&config, "live").unwrap().command, "y");

        // A typo is answered with the names that do exist, so it can be fixed
        // from the message alone.
        let error = resolve(&config, "typo").unwrap_err();
        assert!(error.contains("unknown MCP server 'typo'"), "{error}");
        assert!(error.contains("configured: live, off"), "{error}");

        // With nothing configured there is nothing to list, so the message
        // points at the command that creates the first entry.
        let error = resolve(&McpConfig::default(), "typo").unwrap_err();
        assert!(error.contains("none are configured"), "{error}");
        assert!(error.contains("future mcp add"), "{error}");
    }

    // ── Argument parsing ───────────────────────────────────────────────────

    /// A switch is a switch: `--json=1` is a mistake, not a value to ignore.
    #[test]
    fn a_switch_with_a_value_is_rejected() {
        let error = parse_flags(&args(&["--json=1"]), &[], &[], &["--json"]).unwrap_err();
        assert_eq!(error, "--json does not take a value");
    }

    /// The two numeric budgets are bounded at both ends: zero is refused in
    /// each, because zero means "no budget at all".
    #[test]
    fn the_timeout_and_frame_cap_boundaries_are_enforced() {
        let flags = parse_flags(&args(&["--timeout", "5"]), &["--timeout"], &[], &[]).unwrap();
        assert_eq!(flags.timeout().unwrap(), Duration::from_secs(5));

        let flags = parse_flags(&args(&["--timeout", "0"]), &["--timeout"], &[], &[]).unwrap();
        assert_eq!(
            flags.timeout().unwrap_err(),
            "--timeout must be at least 1 second"
        );

        let flags = parse_flags(
            &args(&["--max-frame-bytes", "0"]),
            &["--max-frame-bytes"],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            flags.max_frame_bytes().unwrap_err(),
            "--max-frame-bytes must be at least 1 byte"
        );
        let flags = parse_flags(
            &args(&["--max-frame-bytes", "4096"]),
            &["--max-frame-bytes"],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(flags.max_frame_bytes().unwrap(), 4096);
    }

    /// Every subcommand that takes one name reports both ways of getting the
    /// count wrong, and both messages end with the usage line.
    #[test]
    fn a_missing_or_extra_positional_reports_the_usage() {
        let flags = parse_flags(&args(&[]), &[], &[], &[]).unwrap();
        assert_eq!(
            flags
                .exactly_one_positional("a server name", REMOVE_USAGE)
                .unwrap_err(),
            format!("missing a server name\n\n{REMOVE_USAGE}")
        );

        let flags = parse_flags(&args(&["one", "two"]), &[], &[], &[]).unwrap();
        assert_eq!(
            flags
                .exactly_one_positional("a server name", REMOVE_USAGE)
                .unwrap_err(),
            format!("unexpected argument: two\n\n{REMOVE_USAGE}")
        );
    }

    /// `lib.rs` only reaches the dispatch table for the eight documented
    /// subcommands, so this pins the fallback the table still owns.
    #[tokio::test]
    async fn an_unknown_subcommand_is_rejected_with_its_name() {
        let (out, _sink) = Output::memory();
        let error = mcp("bogus", &args(&[]), &out).await.unwrap_err();
        assert_eq!(error, "unknown mcp command: bogus");
    }

    /// A usage error is found before the configuration is read, so a mistyped
    /// flag costs nothing and cannot be masked by a broken `mcp.json`.
    #[tokio::test]
    async fn tools_and_call_reject_an_unknown_option_before_reading_any_config() {
        let (out, _sink) = Output::memory();
        assert_eq!(
            mcp_tools(&args(&["fake", "--bogus"]), &out)
                .await
                .unwrap_err(),
            "unknown option: --bogus"
        );
        assert_eq!(
            mcp_call(&args(&["fake", "plain", "--bogus"]), &out)
                .await
                .unwrap_err(),
            "unknown option: --bogus"
        );
    }

    /// `call` needs exactly two names, and `--args` must be a JSON *object*:
    /// an array is a common mistake and is refused before any server starts.
    #[tokio::test]
    async fn call_validates_its_names_and_its_args_flag() {
        let (out, _sink) = Output::memory();

        let error = mcp_call(&args(&["fake"]), &out).await.unwrap_err();
        assert!(
            error.contains("expected a server and a tool name (1)"),
            "{error}"
        );
        let error = mcp_call(&args(&["fake", "plain", "extra"]), &out)
            .await
            .unwrap_err();
        assert!(
            error.contains("expected a server and a tool name (3)"),
            "{error}"
        );

        let error = mcp_call(&args(&["fake", "plain", "--args", "{not json}"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("--args is not valid JSON"), "{error}");
        let error = mcp_call(&args(&["fake", "plain", "--args", "[1,2]"]), &out)
            .await
            .unwrap_err();
        assert_eq!(error, "--args must be a JSON object of tool arguments");
    }

    /// `status` takes at most one name; two is a usage error, not "check the
    /// first one and ignore the rest".
    #[tokio::test]
    async fn status_takes_at_most_one_server_name() {
        let (out, _sink) = Output::memory();
        let error = mcp_status(&args(&["one", "two"]), &out).await.unwrap_err();
        assert!(
            error.contains("expected at most one server name (2)"),
            "{error}"
        );
    }

    // ── list / add / remove on the configuration file ──────────────────────

    #[tokio::test]
    async fn list_rejects_an_extra_argument_and_prints_the_document_on_request() {
        let _lock = crate::test_env::lock_env().await;
        let (_home, _env) = home_with_config(Some(
            r#"{ "servers": {
                   "off": { "command": "x", "enabled": false },
                   "rea": { "command": "rea", "args": ["--mcp"], "cwd": "/tmp/work" }
                 } }"#,
        ));

        let (out, _sink) = Output::memory();
        let error = mcp_list(&args(&["extra"]), &out).unwrap_err();
        assert!(error.contains("unexpected argument: extra"), "{error}");

        let (out, sink) = Output::memory();
        mcp_list(&args(&["--json"]), &out).unwrap();
        let (stdout, stderr) = rendered(&sink);
        assert_eq!(stderr, "");
        let document: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            document["configPath"],
            json!(config_path().display().to_string())
        );
        assert_eq!(document["servers"][0]["name"], "off");
        assert_eq!(document["servers"][0]["command"], "x");
        assert_eq!(document["servers"][0]["enabled"], json!(false));
        assert_eq!(document["servers"][1]["name"], "rea");
        assert_eq!(document["servers"][1]["args"], json!(["--mcp"]));
        assert_eq!(document["servers"][1]["cwd"], json!("/tmp/work"));
        assert_eq!(document["servers"][1]["enabled"], json!(true));
    }

    #[tokio::test]
    async fn add_stores_the_arguments_environment_and_working_directory() {
        let _lock = crate::test_env::lock_env().await;
        let (_home, _env) = home_with_config(None);
        let (out, sink) = Output::memory();

        mcp_add(
            &args(&[
                "rea",
                "--command",
                "rea",
                "--arg",
                "--mcp",
                "--env",
                "TOKEN=secret",
                "--env",
                "EMPTY=",
                "--cwd",
                "/tmp/work",
            ]),
            &out,
        )
        .unwrap();

        let (stdout, stderr) = rendered(&sink);
        assert_eq!(stderr, "");
        assert!(
            stdout.contains("Added MCP server 'rea': rea --mcp"),
            "{stdout}"
        );
        assert!(stdout.contains("future mcp tools rea"), "{stdout}");

        // The file is the contract: arguments verbatim (dash and all), the
        // environment as strings, the working directory, enabled by default.
        let config = load_config().unwrap();
        let spec = &config.servers["rea"];
        assert_eq!(spec.command_line(), "rea --mcp");
        assert_eq!(spec.env["TOKEN"], json!("secret"));
        assert_eq!(spec.env["EMPTY"], json!(""));
        assert_eq!(spec.cwd.as_deref(), Some("/tmp/work"));
        assert!(spec.enabled);

        // Adding a name twice must not quietly overwrite what is there.
        let (out, _sink) = Output::memory();
        let error = mcp_add(&args(&["rea", "--command", "other"]), &out).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert!(error.contains("future mcp remove rea"), "{error}");
        assert_eq!(load_config().unwrap().servers["rea"].command, "rea");
    }

    #[tokio::test]
    async fn add_rejects_an_unusable_name_and_a_malformed_environment_pair() {
        let _lock = crate::test_env::lock_env().await;
        let (_home, _env) = home_with_config(None);
        let (out, _sink) = Output::memory();

        let error = mcp_add(&args(&["has space", "--command", "x"]), &out).unwrap_err();
        assert!(error.contains("is not a usable server name"), "{error}");

        let error = mcp_add(&args(&["srv", "--command", "x", "--env", "TOKEN"]), &out).unwrap_err();
        assert_eq!(error, "--env expects KEY=VALUE, got 'TOKEN'");

        let error =
            mcp_add(&args(&["srv", "--command", "x", "--env", "=secret"]), &out).unwrap_err();
        assert_eq!(error, "--env expects KEY=VALUE, got '=secret'");

        // None of the three refusals wrote anything.
        assert!(load_config().unwrap().servers.is_empty());
    }

    #[tokio::test]
    async fn remove_reports_an_unknown_server_and_deletes_a_known_one() {
        let _lock = crate::test_env::lock_env().await;
        let (_home, _env) =
            home_with_config(Some(r#"{ "servers": { "rea": { "command": "rea" } } }"#));

        let (out, _sink) = Output::memory();
        let error = mcp_remove(&args(&["ghost"]), &out).unwrap_err();
        assert!(error.contains("no MCP server named 'ghost'"), "{error}");
        assert!(
            load_config().unwrap().servers.contains_key("rea"),
            "a refused removal changes nothing"
        );

        let (out, sink) = Output::memory();
        mcp_remove(&args(&["rea"]), &out).unwrap();
        let (stdout, _) = rendered(&sink);
        assert!(stdout.contains("Removed MCP server 'rea'"), "{stdout}");
        assert!(load_config().unwrap().servers.is_empty());
    }

    // ── The two shared helpers this file renders with ──────────────────────

    /// A content block that is neither text nor detailed, and one that does not
    /// say what it is at all, are both named rather than dropped.
    #[test]
    fn content_blocks_without_a_detail_are_named() {
        assert_eq!(
            render_tool_result(&json!({ "content": [{ "type": "image" }] })),
            "[image content]"
        );
        assert_eq!(
            render_tool_result(&json!({ "content": [{ "data": "x" }] })),
            "[unrecognized content block omitted]"
        );
        assert_eq!(
            render_tool_result(&json!({ "content": [{ "type": "resource", "uri": "file:///a" }] })),
            "[resource content: file:///a]"
        );
    }

    #[test]
    fn stderr_is_attached_to_a_failure_only_when_the_server_wrote_some() {
        assert_eq!(attach_stderr("boom".to_string(), "   \n"), "boom");
        assert_eq!(
            attach_stderr("boom".to_string(), "line one\nline two\n"),
            "boom\n--- server stderr (last lines) ---\nline one\nline two"
        );
    }

    // ── End to end against a real spawned server ───────────────────────────

    /// These spawn `/bin/sh` as an MCP server over real pipes, so they cover
    /// what the duplex tests cannot: the configuration file, process spawning,
    /// the handshake against a real child, and the stdout/stderr split a
    /// caller actually sees.
    #[cfg(unix)]
    mod process {
        use super::*;
        use crate::output::Output;

        /// A minimal MCP server in POSIX shell: answers `initialize`,
        /// `tools/list` and `tools/call`, echoing the request id back.
        ///
        /// `tools/call boom` answers `isError: true`; anything else succeeds.
        const FAKE_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"boom","description":"always fails"},{"name":"plain","description":"always works"}]}}\n' "$id" ;;
    *'"tools/call"'*)
      case "$line" in
        *'"name":"boom"'*)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"isError":true,"content":[{"type":"text","text":"engine missing"}]}}\n' "$id" ;;
        *)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"fine"}]}}\n' "$id" ;;
      esac ;;
  esac
done
"#;

        /// A throwaway FutureOS home whose `mcp.json` runs the fake server.
        fn with_fake_server() -> (tempfile::TempDir, crate::test_env::EnvGuard) {
            let home = tempfile::tempdir().expect("tempdir");
            std::fs::write(
                home.path().join("mcp.json"),
                serde_json::to_string(&json!({
                    "servers": {
                        "fake": { "command": "/bin/sh", "args": ["-c", FAKE_SERVER] },
                    },
                }))
                .unwrap(),
            )
            .unwrap();
            let paths = std::env::var_os("PATH").unwrap_or_default();
            let guard = crate::test_env::EnvGuard::set(&[
                ("FUTURE_HOME", home.path().as_os_str().to_owned()),
                ("PATH", paths),
            ]);
            (home, guard)
        }

        fn split(captured: &crate::output::Captured) -> (String, String) {
            let out = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
            let err = String::from_utf8(captured.err.lock().unwrap().clone()).unwrap();
            (out, err)
        }

        #[tokio::test]
        async fn a_successful_call_prints_the_tool_text_on_stdout() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();

            mcp_call(&args(&["fake", "plain"]), &out)
                .await
                .expect("a normal tool result is a successful command");
            let (stdout, stderr) = split(&sink);
            assert_eq!(stdout, "fine\n");
            assert_eq!(stderr, "");
        }

        #[tokio::test]
        async fn a_tool_reported_error_goes_to_stderr_and_fails_the_command() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();

            let error = mcp_call(&args(&["fake", "boom"]), &out)
                .await
                .expect_err("a tool error is a failed command");
            assert_eq!(
                error,
                crate::HANDLED_EXIT,
                "exit code 1, not a printed error"
            );
            let (stdout, stderr) = split(&sink);
            assert_eq!(stdout, "", "a failure must not fill stdout");
            assert!(stderr.contains("[the tool reported an error]"), "{stderr}");
            assert!(stderr.contains("engine missing"), "{stderr}");
        }

        #[tokio::test]
        async fn a_tool_reported_error_still_honours_out() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();
            let path = std::env::temp_dir().join("future-mcp-call-error-out.txt");
            let _ = std::fs::remove_file(&path);
            let path_arg = path.display().to_string();

            mcp_call(
                &args(&["fake", "boom", "--max-bytes", "4", "--out", &path_arg]),
                &out,
            )
            .await
            .expect_err("a tool error is a failed command");
            let (_, stderr) = split(&sink);
            assert!(stderr.contains("first 4 bytes of 14"), "{stderr}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "engine missing");
            let _ = std::fs::remove_file(&path);
        }

        #[tokio::test]
        async fn tools_lists_the_spawned_servers_tools() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();

            mcp_tools(&args(&["fake"]), &out).await.unwrap();
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("boom"), "{stdout}");
            assert!(stdout.contains("plain"), "{stdout}");
            assert!(stdout.contains("Showing 2 of 2 tools."), "{stdout}");
        }

        #[tokio::test]
        async fn a_server_that_dies_reports_its_stderr() {
            let _lock = crate::test_env::lock_env().await;
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("mcp.json"),
                json!({
                    "servers": { "broken": {
                        "command": "/bin/sh",
                        "args": ["-c", "echo 'no engine here' >&2; exit 3"],
                    }},
                })
                .to_string(),
            )
            .unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let (out, _sink) = Output::memory();

            let error = mcp_tools(&args(&["broken"]), &out)
                .await
                .expect_err("a server that dies during its handshake fails the command");
            assert!(error.contains("failed its handshake"), "{error}");
            assert!(error.contains("no engine here"), "{error}");
        }

        // ── status ─────────────────────────────────────────────────────────

        #[tokio::test]
        async fn status_reports_a_reachable_server_with_its_tool_count() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();

            mcp_status(&args(&[]), &out).await.unwrap();
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("fake"), "{stdout}");
            assert!(stdout.contains("ok"), "{stdout}");
            assert!(stdout.contains("2 tool(s)"), "{stdout}");
            assert!(stdout.contains("1/1 server(s) reachable"), "{stdout}");
        }

        /// The point of `status` over `list`: an unreachable server fails, and
        /// its own stderr is what explains why.
        #[tokio::test]
        async fn status_fails_and_shows_stderr_for_an_unreachable_server() {
            let _lock = crate::test_env::lock_env().await;
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("mcp.json"),
                json!({
                    "servers": {
                        "broken": {
                            "command": "/bin/sh",
                            "args": ["-c", "echo 'JAVA_HOME is not set' >&2; exit 4"],
                        },
                        "fine": { "command": "/bin/sh", "args": ["-c", FAKE_SERVER] },
                    },
                })
                .to_string(),
            )
            .unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let (out, sink) = Output::memory();

            let error = mcp_status(&args(&[]), &out)
                .await
                .expect_err("one unreachable server fails the check");
            assert_eq!(error, crate::HANDLED_EXIT.to_string());
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("JAVA_HOME is not set"), "{stdout}");
            assert!(stdout.contains("1/2 server(s) reachable"), "{stdout}");
        }

        #[tokio::test]
        async fn a_disabled_server_is_reported_but_does_not_fail_status() {
            let _lock = crate::test_env::lock_env().await;
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("mcp.json"),
                json!({
                    "servers": {
                        "off": { "command": "/nonexistent", "enabled": false },
                        "fake": { "command": "/bin/sh", "args": ["-c", FAKE_SERVER] },
                    },
                })
                .to_string(),
            )
            .unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let (out, sink) = Output::memory();

            mcp_status(&args(&[]), &out).await.unwrap();
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("disabled"), "{stdout}");
            assert!(stdout.contains("1/2 server(s) reachable"), "{stdout}");
        }

        // ── a mistyped tool name ───────────────────────────────────────────

        /// `describe` has always suggested alternatives; `call` used to hand the
        /// typo to the server and print whatever it said back.
        #[tokio::test]
        async fn call_suggests_the_tool_the_name_was_meant_to_be() {
            let _lock = crate::test_env::lock_env().await;
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("mcp.json"),
                json!({
                    "servers": { "fake": {
                        "command": "/bin/sh",
                        "args": ["-c", REJECTING_SERVER],
                    }},
                })
                .to_string(),
            )
            .unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let (out, _sink) = Output::memory();

            let error = mcp_call(&args(&["fake", "plai"]), &out)
                .await
                .expect_err("an unknown tool fails the command");
            assert!(error.contains("has no tool named 'plai'"), "{error}");
            assert!(error.contains("plain"), "{error}");
            assert!(error.contains("Did you mean"), "{error}");
        }

        /// A failing call whose tool *does* exist keeps the server's own error —
        /// the suggestion path must not swallow a genuine failure.
        #[tokio::test]
        async fn call_keeps_the_servers_error_when_the_tool_exists() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();

            let error = mcp_call(&args(&["fake", "boom"]), &out)
                .await
                .expect_err("a tool-reported error fails the command");
            assert_eq!(error, crate::HANDLED_EXIT.to_string());
            let (stdout, stderr) = split(&sink);
            assert!(stderr.contains("engine missing"), "{stderr}");
            assert!(stdout.is_empty(), "{stdout}");
        }

        // ── the frame budget ──────────────────────────────────────────────

        /// A frame past the cap must say which budget it is and how to raise it;
        /// `--max-bytes` is a print cap and cannot help here.
        #[tokio::test]
        async fn a_frame_over_the_cap_explains_the_read_budget() {
            let _lock = crate::test_env::lock_env().await;
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("mcp.json"),
                json!({
                    "servers": { "big": {
                        "command": "/bin/sh",
                        "args": ["-c", BIG_FRAME_SERVER],
                    }},
                })
                .to_string(),
            )
            .unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let (out, _sink) = Output::memory();

            // A cap this small is under any real handshake, so the failure is
            // deterministic and does not need a multi-megabyte fixture.
            let error = mcp_tools(&args(&["big", "--max-frame-bytes", "64"]), &out)
                .await
                .expect_err("a frame past the cap fails the command");
            assert!(error.contains("larger than 64 bytes"), "{error}");
            assert!(error.contains("--max-frame-bytes"), "{error}");
            assert!(error.contains("FUTURE_MCP_MAX_FRAME_BYTES"), "{error}");
            assert!(error.contains("--max-bytes"), "{error}");

            // The same server passes at the default cap.
            mcp_tools(&args(&["big"]), &out)
                .await
                .expect("the default cap fits a handshake");
        }

        #[tokio::test]
        async fn a_non_numeric_frame_budget_is_rejected() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, _sink) = Output::memory();

            let error = mcp_tools(&args(&["fake", "--max-frame-bytes", "lots"]), &out)
                .await
                .expect_err("a non-numeric cap is a usage error");
            assert!(error.contains("whole number of bytes"), "{error}");
        }

        /// A server that refuses `tools/call` the way a real one refuses an
        /// unknown name, but still answers `tools/list`.
        const REJECTING_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"plain","description":"works"}]}}\n' "$id" ;;
    *'"tools/call"'*)
      printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"unknown tool"}}\n' "$id" ;;
  esac
done
"#;

        /// The big-frame server: a normal handshake, plus a `tools/list` answer
        /// far larger than a deliberately tiny cap.
        const BIG_FRAME_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*)
      pad=$(printf 'x%.0s' $(seq 1 4000))
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"only","description":"%s"}]}}\n' "$id" "$pad" ;;
  esac
done
"#;

        // ── serve ──────────────────────────────────────────────────────────

        /// The whole point of `serve`: one cold start for many calls. The fake
        /// appends a line to `$MCP_START_LOG` on `initialize`, so the count of
        /// handshakes is observable — with `call` there would be one per request.
        /// `tools/call plain` succeeds, `boom` reports a tool error, and any
        /// other name is a protocol-level error.
        const COUNTING_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf 'start\n' >> "$MCP_START_LOG"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"plain","description":"works"}]}}\n' "$id" ;;
    *'"tools/call"'*)
      case "$line" in
        *'"name":"plain"'*)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"called"}]}}\n' "$id" ;;
        *'"name":"boom"'*)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"isError":true,"content":[{"type":"text","text":"engine missing"}]}}\n' "$id" ;;
        *)
          printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"unknown tool"}}\n' "$id" ;;
      esac ;;
  esac
done
"#;

        /// A FutureOS home configured with `server`, plus a start log path.
        fn home_with(
            server: &str,
            log: &std::path::Path,
        ) -> (tempfile::TempDir, crate::test_env::EnvGuard) {
            let home = tempfile::tempdir().expect("tempdir");
            std::fs::write(
                home.path().join("mcp.json"),
                serde_json::to_string(&json!({
                    "servers": { "fake": { "command": "/bin/sh", "args": ["-c", server] } },
                }))
                .unwrap(),
            )
            .unwrap();
            let guard = crate::test_env::EnvGuard::set(&[
                ("FUTURE_HOME", home.path().as_os_str().to_owned()),
                ("MCP_START_LOG", log.as_os_str().to_owned()),
            ]);
            (home, guard)
        }

        fn starts_in(log: &std::path::Path) -> usize {
            std::fs::read_to_string(log)
                .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
                .unwrap_or(0)
        }

        /// `serve` takes its requests from stdin, so the test drives the loop with
        /// an in-memory reader — the same code path, without a pipe.
        #[tokio::test]
        async fn serve_answers_many_requests_over_one_session() {
            let _lock = crate::test_env::lock_env().await;
            let log = tempfile::tempdir().unwrap();
            let log_file = log.path().join("starts.log");
            let _home = home_with(COUNTING_SERVER, &log_file);

            let config = load_config().unwrap();
            let spec = resolve(&config, "fake").unwrap();
            let mut process = ServerProcess::start("fake", spec, Duration::from_secs(10))
                .await
                .unwrap();
            assert_eq!(starts_in(&log_file), 1, "one handshake so far");

            let requests = concat!(
                "{\"tool\":\"plain\"}\n",
                "{\"tool\":\"plain\",\"args\":{}}\n",
                "{\"tool\":\"plain\"}\n",
            );
            let (out, sink) = Output::memory();
            serve_requests(&mut process, &out, requests.as_bytes())
                .await
                .unwrap();

            let (stdout, _) = split(&sink);
            let answers: Vec<Value> = stdout
                .lines()
                .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
                .collect();
            assert_eq!(answers.len(), 3, "one answer per request: {stdout}");
            for answer in &answers {
                assert_eq!(answer["ok"], json!(true));
                assert_eq!(answer["isError"], json!(false));
                assert_eq!(answer["result"]["content"][0]["text"], "called");
            }
            // Three calls, still one cold start: that is the feature.
            assert_eq!(starts_in(&log_file), 1, "still one handshake after 3 calls");
            process.shutdown().await;
        }

        /// A bad request must be answered and skipped, and a call the server
        /// fails must not end the session — a caller sends a batch, and one
        /// mistake should not cost the remaining requests.
        #[tokio::test]
        async fn serve_survives_bad_and_failing_requests() {
            let _lock = crate::test_env::lock_env().await;
            let log = tempfile::tempdir().unwrap();
            let _home = home_with(COUNTING_SERVER, &log.path().join("starts.log"));

            let config = load_config().unwrap();
            let spec = resolve(&config, "fake").unwrap();
            let mut process = ServerProcess::start("fake", spec, Duration::from_secs(10))
                .await
                .unwrap();

            let requests = concat!(
                "not json at all\n",
                "\n",
                "{\"args\":{}}\n",
                "{\"tool\":\"boom\"}\n",
                "{\"tool\":\"no_such_tool\"}\n",
                "{\"tool\":\"plain\"}\n",
            );
            let (out, sink) = Output::memory();
            serve_requests(&mut process, &out, requests.as_bytes())
                .await
                .unwrap();

            let (stdout, _) = split(&sink);
            let answers: Vec<Value> = stdout
                .lines()
                .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
                .collect();
            // The blank line is skipped, so five answers for six lines.
            assert_eq!(answers.len(), 5, "{stdout}");
            assert!(answers[0]["error"]
                .as_str()
                .unwrap()
                .contains("not valid JSON"));
            assert!(answers[1]["error"]
                .as_str()
                .unwrap()
                .contains("needs a 'tool'"));
            // A tool that reports its own error is a *successful* call carrying
            // `isError`, exactly as `call` reports it on stderr.
            assert_eq!(answers[2]["ok"], json!(true));
            assert_eq!(answers[2]["isError"], json!(true));
            // A protocol-level failure is reported, and the last request still
            // runs: the session outlived both failures.
            assert_eq!(answers[3]["ok"], json!(false));
            assert!(answers[3]["error"]
                .as_str()
                .unwrap()
                .contains("unknown tool"));
            assert_eq!(answers[4]["ok"], json!(true));
            assert_eq!(answers[4]["result"]["content"][0]["text"], "called");
            process.shutdown().await;
        }

        // ── status: the JSON form and the handshake that succeeds alone ────

        /// The `--json` form is what a script reads: one record per configured
        /// server, in configuration order, with its state and tool count.
        #[tokio::test]
        async fn status_json_reports_every_server_with_its_state() {
            let _lock = crate::test_env::lock_env().await;
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("mcp.json"),
                json!({
                    "servers": {
                        "off": { "command": "/nonexistent", "enabled": false },
                        "fake": { "command": "/bin/sh", "args": ["-c", FAKE_SERVER] },
                    },
                })
                .to_string(),
            )
            .unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let (out, sink) = Output::memory();

            mcp_status(&args(&["--json"]), &out).await.unwrap();
            let (stdout, _) = split(&sink);
            let document: Value = serde_json::from_str(&stdout).unwrap();
            assert_eq!(
                document["configPath"],
                json!(config_path().display().to_string())
            );
            let servers = document["servers"].as_array().unwrap();
            assert_eq!(servers.len(), 2, "{document}");
            assert_eq!(servers[0]["name"], "fake");
            assert_eq!(servers[0]["state"], "ok");
            assert_eq!(servers[0]["toolCount"], json!(2));
            assert!(
                servers[0]["command"]
                    .as_str()
                    .unwrap()
                    .starts_with("/bin/sh -c"),
                "{document}"
            );
            assert!(servers[0]["handshakeMs"].is_u64(), "{document}");
            assert!(servers[0]["totalMs"].is_u64(), "{document}");
            // A disabled server is a configuration state with no error of its
            // own; it is reported, and it does not fail the command.
            assert_eq!(servers[1]["name"], "off");
            assert_eq!(servers[1]["state"], "disabled");
            assert!(servers[1].get("error").is_none(), "{document}");
        }

        /// The handshake can succeed and the next request still fail — the
        /// server's own stderr is the only explanation of why.
        #[tokio::test]
        async fn status_reports_a_server_that_fails_after_a_successful_handshake() {
            let _lock = crate::test_env::lock_env().await;
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("mcp.json"),
                json!({
                    "servers": { "quiet": {
                        "command": "/bin/sh",
                        "args": ["-c", HANDSHAKE_ONLY_SERVER],
                    }},
                })
                .to_string(),
            )
            .unwrap();
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_HOME",
                home.path().as_os_str().to_owned(),
            )]);
            let (out, sink) = Output::memory();

            let error = mcp_status(&args(&["--timeout", "1"]), &out)
                .await
                .expect_err("a server that stops answering is a failed check");
            assert_eq!(error, crate::HANDLED_EXIT.to_string());
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("quiet"), "{stdout}");
            assert!(stdout.contains("FAILED"), "{stdout}");
            assert!(stdout.contains("timed out after 1s"), "{stdout}");
            assert!(stdout.contains("JAVA_HOME is not set"), "{stdout}");
            assert!(stdout.contains("0/1 server(s) reachable"), "{stdout}");
        }

        // ── tools: the query, the page and the JSON form ────────────────────

        /// A query matches the name or the description, case-insensitively, and
        /// the header says how much of the list survived it.
        #[tokio::test]
        async fn tools_filters_by_query_and_reports_the_totals() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();

            let (out, sink) = Output::memory();
            mcp_tools(&args(&["fake", "--query", "BOOM"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            assert!(
                stdout.contains("Tools of 'fake' matching \"boom\" (1 of 2):"),
                "{stdout}"
            );
            assert!(stdout.contains("boom"), "{stdout}");
            assert!(!stdout.contains("plain"), "{stdout}");

            // The description matches too, not only the name.
            let (out, sink) = Output::memory();
            mcp_tools(&args(&["fake", "--query", "always works"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("(1 of 2)"), "{stdout}");
            assert!(stdout.contains("plain"), "{stdout}");
            assert!(!stdout.contains("boom"), "{stdout}");
        }

        #[tokio::test]
        async fn tools_json_page_carries_the_progress_through_the_list() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();

            let (out, sink) = Output::memory();
            mcp_tools(&args(&["fake", "--json", "--limit", "1"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            let document: Value = serde_json::from_str(&stdout).unwrap();
            assert_eq!(document["server"], "fake");
            assert_eq!(document["total"], json!(2));
            assert_eq!(document["offset"], json!(0));
            assert_eq!(document["limit"], json!(1));
            assert_eq!(document["tools"].as_array().unwrap().len(), 1);
            assert_eq!(document["tools"][0]["name"], "boom");
            assert_eq!(document["tools"][0]["description"], "always fails");
            assert_eq!(document["nextOffset"], json!(1));

            // The last page says so with an explicit null rather than by
            // leaving the field out, so a caller can always read it.
            let (out, sink) = Output::memory();
            mcp_tools(
                &args(&["fake", "--json", "--offset", "1", "--limit", "1"]),
                &out,
            )
            .await
            .unwrap();
            let (stdout, _) = split(&sink);
            let document: Value = serde_json::from_str(&stdout).unwrap();
            assert_eq!(document["tools"][0]["name"], "plain");
            assert_eq!(document["nextOffset"], Value::Null);
        }

        #[tokio::test]
        async fn tools_names_the_next_offset_and_the_offset_that_is_past_the_end() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();

            // A page that is not the last one points at the next offset.
            let (out, sink) = Output::memory();
            mcp_tools(&args(&["fake", "--limit", "1"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("Showing 1 of 2 tools."), "{stdout}");
            assert!(stdout.contains("More tools exist"), "{stdout}");
            assert!(stdout.contains("--offset 1"), "{stdout}");

            // An offset past the end is a page that cannot exist, and the
            // message gives the last offset that can.
            let (out, sink) = Output::memory();
            mcp_tools(&args(&["fake", "--offset", "9"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            assert!(stdout.contains("Showing 0 of 2 tools."), "{stdout}");
            assert!(stdout.contains("--offset 9 is past the end"), "{stdout}");
            assert!(
                stdout.contains("the last page starts at --offset 1"),
                "{stdout}"
            );
        }

        // ── describe ───────────────────────────────────────────────────────

        #[tokio::test]
        async fn describe_prints_the_description_and_the_schema() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_server(DESCRIBE_SERVER);
            let (out, sink) = Output::memory();

            mcp_describe(&args(&["fake", "full"]), &out).await.unwrap();
            let (stdout, _) = split(&sink);
            assert!(stdout.starts_with("full\n"), "{stdout}");
            assert!(stdout.contains("does a thing"), "{stdout}");
            assert!(stdout.contains("Arguments (JSON Schema):"), "{stdout}");
            assert!(stdout.contains("\"path\""), "{stdout}");

            // A tool with neither a description nor a schema is still
            // describable: the schema falls back to `null`, and no empty
            // description line is printed.
            let (out, sink) = Output::memory();
            mcp_describe(&args(&["fake", "bare"]), &out).await.unwrap();
            let (stdout, _) = split(&sink);
            assert!(
                stdout.starts_with("bare\n\nArguments (JSON Schema):"),
                "{stdout}"
            );
            assert!(
                stdout.ends_with("Arguments (JSON Schema):\nnull\n"),
                "{stdout}"
            );
        }

        #[tokio::test]
        async fn describe_json_prints_the_tool_verbatim() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_server(DESCRIBE_SERVER);
            let (out, sink) = Output::memory();

            mcp_describe(&args(&["fake", "full", "--json"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            let document: Value = serde_json::from_str(&stdout).unwrap();
            assert_eq!(document["name"], "full");
            assert_eq!(document["description"], "does a thing");
            assert_eq!(
                document["inputSchema"]["properties"]["path"]["type"],
                "string"
            );
        }

        #[tokio::test]
        async fn describe_requires_two_names_and_suggests_alternatives() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_server(DESCRIBE_SERVER);
            let (out, _sink) = Output::memory();

            let error = mcp_describe(&args(&[]), &out).await.unwrap_err();
            assert!(
                error.contains("expected a server and a tool name (0)"),
                "{error}"
            );
            let error = mcp_describe(&args(&["fake"]), &out).await.unwrap_err();
            assert!(
                error.contains("expected a server and a tool name (1)"),
                "{error}"
            );

            // A name that resembles nothing is answered with how to list what
            // the server does have, rather than with a bad guess...
            let error = mcp_describe(&args(&["fake", "zzzz"]), &out)
                .await
                .unwrap_err();
            assert!(error.contains("has no tool named 'zzzz'"), "{error}");
            assert!(
                error.contains("future mcp tools fake --query <text>"),
                "{error}"
            );
            assert!(!error.contains("Did you mean"), "{error}");

            // ...and a near miss with the name it was probably meant to be.
            let error = mcp_describe(&args(&["fake", "ful"]), &out)
                .await
                .unwrap_err();
            assert!(error.contains("Did you mean: full"), "{error}");
        }

        // ── call: --args, --json and --out ──────────────────────────────────

        #[tokio::test]
        async fn call_passes_a_json_args_object_to_the_tool() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_server(ECHO_PATH_SERVER);
            let (out, sink) = Output::memory();

            mcp_call(
                &args(&["fake", "plain", "--args", r#"{"path":"/bin/ls"}"#]),
                &out,
            )
            .await
            .unwrap();
            let (stdout, _) = split(&sink);
            assert_eq!(stdout, "the tool got /bin/ls\n", "{stdout}");
        }

        #[tokio::test]
        async fn call_json_prints_the_raw_result_envelope() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();

            mcp_call(&args(&["fake", "plain", "--json"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            let document: Value = serde_json::from_str(&stdout).unwrap();
            assert_eq!(document["content"][0]["type"], "text");
            assert_eq!(document["content"][0]["text"], "fine");
        }

        /// A result that fits is written whole, and the caller is told where.
        #[tokio::test]
        async fn call_writes_a_short_result_to_out_and_says_where_it_went() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let path = std::env::temp_dir().join("future-mcp-call-out-small.txt");
            let _ = std::fs::remove_file(&path);
            let path_arg = path.display().to_string();
            let (out, sink) = Output::memory();

            mcp_call(&args(&["fake", "plain", "--out", &path_arg]), &out)
                .await
                .unwrap();
            let (stdout, stderr) = split(&sink);
            assert_eq!(stderr, "");
            assert!(stdout.contains("[the whole result is also in"), "{stdout}");
            assert!(stdout.contains(&path_arg), "{stdout}");
            assert!(stdout.ends_with("fine\n"), "{stdout}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "fine");
            let _ = std::fs::remove_file(&path);
        }

        /// A result past the print cap is truncated, and the header gives both
        /// numbers and the flag that writes the whole thing.
        #[tokio::test]
        async fn call_truncates_a_large_result_and_names_the_rerun_flag() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_fake_server();
            let (out, sink) = Output::memory();

            mcp_call(&args(&["fake", "plain", "--max-bytes", "3"]), &out)
                .await
                .unwrap();
            let (stdout, _) = split(&sink);
            assert!(
                stdout.contains("[first 3 bytes of 4 — rerun with --out <file> to write it all]"),
                "{stdout}"
            );
            assert!(stdout.ends_with("fin\n"), "{stdout}");
        }

        /// A protocol-level failure for a tool that *is* in the list keeps the
        /// server's own error: replacing it with "no such tool" would hide what
        /// actually went wrong.
        #[tokio::test]
        async fn call_keeps_the_servers_error_when_the_tool_is_in_the_list() {
            let _lock = crate::test_env::lock_env().await;
            let _home = with_server(REJECTING_SERVER);
            let (out, _sink) = Output::memory();

            let error = mcp_call(&args(&["fake", "plain"]), &out).await.unwrap_err();
            assert!(error.contains("MCP 'tools/call' failed"), "{error}");
            assert!(error.contains("unknown tool"), "{error}");
            assert!(!error.contains("has no tool named"), "{error}");
        }

        // ── serve over a real stdin ────────────────────────────────────────

        /// `serve` reads its requests from the process's own stdin, which no
        /// in-process test can replace (fd 0 belongs to the whole test binary).
        /// This re-runs this same instrumented test binary as a child with a
        /// piped stdin — the technique `commands::tools` uses for the same
        /// shape — so the production stdin path runs for real and its coverage
        /// merges with the parent's.
        #[tokio::test(flavor = "multi_thread")]
        async fn serve_drives_a_real_stdin_session() {
            const CHILD: &str = "FUTURE_MCP_SERVE_CHILD";
            const MODE: &str = "FUTURE_MCP_SERVE_MODE";
            const NAME: &str =
                "commands::mcp_servers::tests::process::serve_drives_a_real_stdin_session";

            if let Some(home) = std::env::var_os(CHILD) {
                let mode = std::env::var(MODE).expect("the parent sets the mode");
                let _env = crate::test_env::EnvGuard::set(&[("FUTURE_HOME", home)]);
                let out = Output::stdio();
                match mode.as_str() {
                    // The server answers `initialize` and then stops
                    // answering, so the command fails *after* a successful
                    // handshake and its last words are the explanation.
                    "fails" => {
                        let error = mcp_serve(&args(&["fake", "--timeout", "1"]), &out)
                            .await
                            .expect_err("a server that stops answering fails the command");
                        assert!(error.contains("timed out after 1s"), "{error}");
                        assert!(error.contains("JAVA_HOME is not set"), "{error}");
                    }
                    // A stdin whose reads *fail* rather than reaching EOF: the
                    // request loop reports it, and the session still closes
                    // with the server's diagnostics on stderr. A directory
                    // cannot be read as lines, so this is a real broken stdin.
                    "shutdown" => {
                        let broken = tokio::fs::File::open(".")
                            .await
                            .expect("a directory to read");
                        mcp_serve_with_stdin(
                            &args(&["fake"]),
                            &out,
                            tokio::io::BufReader::new(broken),
                        )
                        .await
                        .expect("a stdin that fails to read is reported, not fatal");
                    }
                    _ => {
                        mcp_serve(&args(&["fake"]), &out)
                            .await
                            .expect("the session runs to its end");
                    }
                }
                return;
            }

            for (mode, server, requests) in [
                (
                    "serves",
                    SERVE_SERVER,
                    "{\"tool\":\"plain\"}\n{\"tool\":\"boom\"}\n",
                ),
                ("fails", HANDSHAKE_ONLY_SERVER, ""),
                ("shutdown", SERVE_SERVER, ""),
            ] {
                let home = tempfile::tempdir().unwrap();
                let log = home.path().join("starts.log");
                std::fs::write(
                    home.path().join("mcp.json"),
                    serde_json::to_string(&json!({
                        "servers": { "fake": { "command": "/bin/sh", "args": ["-c", server] } },
                    }))
                    .unwrap(),
                )
                .unwrap();

                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", NAME, "--nocapture"])
                    .env(CHILD, home.path())
                    .env(MODE, mode)
                    .env("MCP_START_LOG", &log)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .expect("spawn the child test");
                use std::io::Write;
                child
                    .stdin
                    .as_mut()
                    .expect("stdin")
                    .write_all(requests.as_bytes())
                    .expect("write the requests");

                let output = child.wait_with_output().expect("wait for the child");
                let stdout = String::from_utf8(output.stdout).unwrap();
                let stderr = String::from_utf8(output.stderr).unwrap();
                assert!(
                    output.status.success(),
                    "{mode} child failed: {stdout}\n{stderr}"
                );
                assert!(
                    stdout.contains("running 1 test"),
                    "the child filter must select this test: {stdout}"
                );
                let answers: Vec<Value> = stdout
                    .lines()
                    .filter(|line| line.starts_with('{'))
                    .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
                    .collect();

                if mode == "serves" {
                    // The ready ack comes first, so a caller can wait for it
                    // instead of guessing how long the engine takes to load.
                    assert_eq!(answers.len(), 3, "{stdout}");
                    assert_eq!(answers[0]["ok"], json!(true));
                    assert_eq!(answers[0]["ready"], json!(true));
                    assert_eq!(answers[0]["server"], "fake");
                    assert_eq!(answers[0]["toolCount"], json!(2));
                    assert_eq!(answers[1]["result"]["content"][0]["text"], "called");
                    assert_eq!(answers[2]["isError"], json!(true));
                    assert_eq!(answers[2]["result"]["content"][0]["text"], "engine missing");
                    // One cold start for two calls: what `serve` exists for.
                    assert_eq!(
                        std::fs::read_to_string(&log)
                            .unwrap_or_default()
                            .lines()
                            .filter(|line| !line.trim().is_empty())
                            .count(),
                        1,
                        "the session is reused, not restarted"
                    );
                    // The server's diagnostics go to stderr, so stdout stays a
                    // clean JSON stream.
                    assert!(stderr.contains("engine warm"), "{stderr}");
                } else if mode == "shutdown" {
                    // The ack was still printed, and the request loop reported
                    // the failing stdin rather than treating it as end-of-input.
                    assert_eq!(answers.len(), 2, "{stdout}");
                    assert_eq!(answers[0]["ready"], json!(true));
                    assert_eq!(answers[1]["ok"], json!(false));
                    assert!(
                        answers[1]["error"].as_str().unwrap().starts_with("stdin: "),
                        "{stdout}"
                    );
                    assert!(stderr.contains("engine warm"), "{stderr}");
                } else {
                    assert!(answers.is_empty(), "{stdout}");
                }
            }
        }

        /// A reader that fails instead of delivering a line: the loop must
        /// report it and stop, not treat it as EOF or spin.
        struct FailingReader;

        impl tokio::io::AsyncRead for FailingReader {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                _buf: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Err(std::io::Error::other("the reader went away")))
            }
        }

        #[tokio::test]
        async fn serve_reports_a_stdin_that_fails_to_read() {
            let _lock = crate::test_env::lock_env().await;
            let log = tempfile::tempdir().unwrap();
            let _home = home_with(COUNTING_SERVER, &log.path().join("starts.log"));

            let config = load_config().unwrap();
            let spec = resolve(&config, "fake").unwrap();
            let mut process = ServerProcess::start("fake", spec, Duration::from_secs(10))
                .await
                .unwrap();
            let (out, sink) = Output::memory();
            serve_requests(&mut process, &out, tokio::io::BufReader::new(FailingReader))
                .await
                .unwrap();

            let (stdout, _) = split(&sink);
            assert!(stdout.contains("stdin: the reader went away"), "{stdout}");
            assert!(stdout.contains("\"ok\":false"), "{stdout}");
            process.shutdown().await;
        }

        /// Answers `initialize`, writes one line to stderr, then reads and
        /// ignores everything else: the handshake succeeds and every later
        /// request stops being answered.
        const HANDSHAKE_ONLY_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf 'JAVA_HOME is not set\n' >&2
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
  esac
done
"#;

        /// `full` has a description and an input schema; `bare` has neither.
        const DESCRIBE_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"full","description":"does a thing","inputSchema":{"type":"object","properties":{"path":{"type":"string"}}}},{"name":"bare"}]}}\n' "$id" ;;
    *'"tools/call"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"fine"}]}}\n' "$id" ;;
  esac
done
"#;

        /// Reports back the `path` member of the arguments object, so a test can
        /// see what actually reached the server.
        const ECHO_PATH_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"plain"}]}}\n' "$id" ;;
    *'"tools/call"'*)
      path=$(printf '%s' "$line" | sed -n 's/.*"path":"\([^"]*\)".*/\1/p')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"the tool got %s"}]}}\n' "$id" "$path" ;;
  esac
done
"#;

        /// The `serve` server: logs every `initialize` to `$MCP_START_LOG` (so a
        /// test can count cold starts), writes one line to stderr (so the
        /// hand-off at the end of `serve` has something to print), and reports
        /// `boom` as a tool error.
        const SERVE_SERVER: &str = r#"
printf 'engine warm\n' >&2
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf 'start\n' >> "$MCP_START_LOG"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"plain"},{"name":"boom"}]}}\n' "$id" ;;
    *'"tools/call"'*)
      case "$line" in
        *'"name":"boom"'*)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"isError":true,"content":[{"type":"text","text":"engine missing"}]}}\n' "$id" ;;
        *)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"called"}]}}\n' "$id" ;;
      esac ;;
  esac
done
"#;

        /// A throwaway FutureOS home whose `mcp.json` runs `script` as `fake`.
        fn with_server(script: &str) -> (tempfile::TempDir, crate::test_env::EnvGuard) {
            let home = tempfile::tempdir().expect("tempdir");
            std::fs::write(
                home.path().join("mcp.json"),
                serde_json::to_string(&json!({
                    "servers": { "fake": { "command": "/bin/sh", "args": ["-c", script] } },
                }))
                .unwrap(),
            )
            .unwrap();
            let paths = std::env::var_os("PATH").unwrap_or_default();
            let guard = crate::test_env::EnvGuard::set(&[
                ("FUTURE_HOME", home.path().as_os_str().to_owned()),
                ("PATH", paths),
            ]);
            (home, guard)
        }
    }
}
