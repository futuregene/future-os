//! CLI entry point — port of `tui/src/index.ts` (777 lines).
//!
//!   - `parse_args` — 1:1 arg parsing (incl. the `--model model:thinking`
//!     colon split, `-p` message capture, `@file` args, `--help` exiting
//!     during scanning, unknown-option exit 1)
//!   - `list_models` — `list_models` table (provider/model/context/thinking/
//!     images, K/M suffixes, 100-row cap)
//!   - `run_print_mode` — non-interactive `-p` prompt via raw gRPC
//!   - `run` — the entry point: the workbench for interactive use

use crate::rpc::types::ModelInfo;
use crate::version::VERSION;
use future_rpc::proto::{RpcCommand, RpcResponse, StreamRequest};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GRPC_DEADLINE_SEC: u64 = 30;

// ─── CLI Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct CliArgs {
    pub grpc_addr: String,
    pub session: Option<String>,
    pub r#continue: bool,
    pub fork: Option<String>,
    pub print: bool,
    pub file_args: Vec<String>,
    pub messages: Vec<String>,
    pub model: Option<String>,
    pub models: Option<Vec<String>>,
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub list_models: Option<String>, // None = off; Some("") = no search
    pub thinking: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<Vec<String>>,
    pub tools: Option<Vec<String>>,
    pub no_tools: bool,
    pub no_builtin_tools: bool,
    pub no_session: bool,
    pub mode: Option<String>,
    pub prompt_template: Option<Vec<String>>,
    pub no_prompt_templates: bool,
    pub no_context_files: bool,
    pub offline: bool,
    pub verbose: bool,
    pub skill: Option<Vec<String>>,
    pub no_skills: bool,
    pub version: bool,
}

/// Result of argument scanning: help/unknown-option terminate during parsing.
#[allow(clippy::large_enum_variant)] // `Args(CliArgs)` is returned by value for TS parity
#[derive(Debug)]
pub enum ParseOutcome {
    Args(CliArgs),
    Help,
    UnknownOption(String),
}

// ─── CLI Parsing ────────────────────────────────────────────────────────────

/// `split(",").map(s => s.trim())` for the list-valued options.
fn split_csv(s: &str) -> Vec<String> {
    s.split(',').map(|part| part.trim().to_string()).collect()
}

/// Port of `parseArgs` from index.ts — `--help` exits during scanning.
pub fn parse_args(args: &[String]) -> ParseOutcome {
    let mut result = CliArgs {
        grpc_addr: std::env::var("FUTURE_AGENT_GRPC_ADDR")
            .unwrap_or_else(|_| future_rpc::transport::AUTO_ENDPOINT.to_string()),
        ..Default::default()
    };

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--grpc-addr" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.grpc_addr = args[i].clone();
                }
            }
            "--session" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.session = Some(args[i].clone());
                }
            }
            "--continue" | "-c" => result.r#continue = true,
            "--fork" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.fork = Some(args[i].clone());
                }
            }
            "--print" | "-p" => {
                result.print = true;
                // Check if next arg is a message (not a flag or file arg).
                if i + 1 < args.len()
                    && !args[i + 1].starts_with('@')
                    && !args[i + 1].starts_with('-')
                {
                    i += 1;
                    result.messages.push(args[i].clone());
                }
            }
            "--model" => {
                if i + 1 < args.len() {
                    i += 1;
                    let model_arg = &args[i];
                    // Support model:thinking format (e.g. sonnet:high).
                    if let Some(colon_index) = model_arg.rfind(':') {
                        if colon_index > 0 {
                            let potential_thinking = &model_arg[colon_index + 1..];
                            const LEVELS: [&str; 6] =
                                ["off", "minimal", "low", "medium", "high", "xhigh"];
                            if LEVELS.contains(&potential_thinking) {
                                result.model = Some(model_arg[..colon_index].to_string());
                                result.thinking = Some(potential_thinking.to_string());
                            } else {
                                result.model = Some(model_arg.clone());
                            }
                        }
                    } else {
                        result.model = Some(model_arg.clone());
                    }
                }
            }
            "--models" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.models = Some(split_csv(&args[i]));
                }
            }
            "--provider" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.provider = Some(args[i].clone());
                }
            }
            "--api-key" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.api_key = Some(args[i].clone());
                }
            }
            "--append-system-prompt" => {
                let v = result.append_system_prompt.get_or_insert_with(Vec::new);
                if i + 1 < args.len() {
                    i += 1;
                    v.push(args[i].clone());
                }
            }
            "--list-models" => {
                result.list_models = Some(String::new());
                if i + 1 < args.len()
                    && !args[i + 1].starts_with('-')
                    && !args[i + 1].starts_with('@')
                {
                    i += 1;
                    result.list_models = Some(args[i].clone());
                }
            }
            "--thinking" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.thinking = Some(args[i].clone());
                }
            }
            "--system-prompt" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.system_prompt = Some(args[i].clone());
                }
            }
            "--tools" | "-t" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.tools = Some(split_csv(&args[i]));
                }
            }
            "--no-tools" | "-nt" => result.no_tools = true,
            "--no-builtin-tools" | "-nbt" => result.no_builtin_tools = true,
            "--no-session" => result.no_session = true,
            "--mode" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.mode = Some(args[i].clone());
                }
            }
            "--prompt-template" => {
                let v = result.prompt_template.get_or_insert_with(Vec::new);
                if i + 1 < args.len() {
                    i += 1;
                    v.push(args[i].clone());
                }
            }
            "--no-prompt-templates" | "-np" => result.no_prompt_templates = true,
            "--no-context-files" | "-nc" => result.no_context_files = true,
            "--offline" => result.offline = true,
            "--verbose" => result.verbose = true,
            "--skill" => {
                let v = result.skill.get_or_insert_with(Vec::new);
                if i + 1 < args.len() {
                    i += 1;
                    v.push(args[i].clone());
                }
            }
            "--no-skills" | "-ns" => result.no_skills = true,
            "--version" | "-v" => result.version = true,
            "--help" | "-h" => return ParseOutcome::Help,
            _ => {
                if let Some(rest) = arg.strip_prefix('@') {
                    result.file_args.push(rest.to_string());
                } else if arg.starts_with('-') {
                    return ParseOutcome::UnknownOption(arg.to_string());
                } else {
                    result.messages.push(arg.to_string());
                }
            }
        }
        i += 1;
    }

    ParseOutcome::Args(result)
}

// ─── Build Initial Prompt ───────────────────────────────────────────────────

/// `buildInitialPrompt` — wraps @files in `<file name="...">` blocks.
fn build_initial_prompt(file_args: &[String], messages: &[String]) -> Option<String> {
    if file_args.is_empty() && messages.is_empty() {
        return None;
    }
    let mut prompt_parts: Vec<String> = Vec::new();
    for file_path in file_args {
        let abs_path = std::path::Path::new(file_path);
        let display = if abs_path.is_absolute() {
            file_path.clone()
        } else {
            let cwd = std::env::current_dir().unwrap_or_default();
            cwd.join(file_path).display().to_string()
        };
        match std::fs::read_to_string(abs_path) {
            Ok(content) => {
                prompt_parts.push(format!("<file name=\"{display}\">\n{content}\n</file>"));
            }
            Err(_) => {
                eprintln!("Failed to read file: {file_path}");
                return None;
            }
        }
    }
    prompt_parts.extend(messages.iter().cloned());
    Some(prompt_parts.join("\n"))
}

// ─── Raw gRPC helpers (print mode / list-models) ───────────────────────────

fn now_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

async fn execute_unary(
    addr: &str,
    cmd: RpcCommand,
    timeout_secs: u64,
) -> Result<RpcResponse, String> {
    let mut client = future_rpc::connection::connect_verified(
        addr,
        future_rpc::connection::EXPECTED_AGENT_VERSION,
        Duration::from_secs(timeout_secs.min(5)),
        Duration::from_secs(5),
    )
    .await
    .map_err(|error| error.to_string())?;
    client
        .execute_command(future_rpc::command_policy::request_with_timeout(cmd))
        .await
        .map(|r| r.into_inner())
        .map_err(|status| {
            let msg = status.message();
            if msg.is_empty() {
                status.to_string()
            } else {
                msg.to_string()
            }
        })
}

// ─── Apply CLI Options to Server (print mode) ──────────────────────────────

/// `applyCliOptions` — the eight cfgN command blocks (set_model, thinking,
/// system prompt, tools, disable_tools, ephemeral, disable_builtin_tools,
/// append system prompt).
async fn apply_cli_options(addr: &str, session_id: &str, args: &CliArgs) -> Result<(), String> {
    // The command name is part of the payload, not implied by the call site:
    // the Agent dispatches on `type` and rejects an unnamed command.
    let cfg = |id: &str, cmd_type: &str, mut cmd: RpcCommand| {
        cmd.id = id.to_string();
        cmd.r#type = cmd_type.to_string();
        cmd.session_id = session_id.to_string();
        cmd
    };

    if let Some(model) = &args.model {
        let cmd = cfg(
            "cfg1",
            "set_model",
            RpcCommand {
                model_id: model.clone(),
                ..Default::default()
            },
        );
        let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
        if !resp.success {
            return Err(if resp.error.is_empty() {
                "unknown error".to_string()
            } else {
                resp.error.clone()
            });
        }
    }
    if let Some(thinking) = &args.thinking {
        let cmd = cfg(
            "cfg2",
            "set_thinking_level",
            RpcCommand {
                level: thinking.clone(),
                ..Default::default()
            },
        );
        let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
        if !resp.success {
            return Err(if resp.error.is_empty() {
                "unknown error".to_string()
            } else {
                resp.error.clone()
            });
        }
    }
    if let Some(sp) = &args.system_prompt {
        let cmd = cfg(
            "cfg3",
            "set_system_prompt",
            RpcCommand {
                system_prompt: sp.clone(),
                ..Default::default()
            },
        );
        let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
        if !resp.success {
            return Err(if resp.error.is_empty() {
                "unknown error".to_string()
            } else {
                resp.error.clone()
            });
        }
    }
    if let Some(tools) = &args.tools {
        if !tools.is_empty() {
            let cmd = cfg(
                "cfg4",
                "set_tools",
                RpcCommand {
                    tools: tools.clone(),
                    ..Default::default()
                },
            );
            let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
            if !resp.success {
                return Err(if resp.error.is_empty() {
                    "unknown error".to_string()
                } else {
                    resp.error.clone()
                });
            }
        }
    }
    if args.no_tools {
        let cmd = cfg("cfg5", "disable_tools", RpcCommand::default());
        let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
        if !resp.success {
            return Err(if resp.error.is_empty() {
                "unknown error".to_string()
            } else {
                resp.error.clone()
            });
        }
    }
    if args.no_session {
        let cmd = cfg(
            "cfg6",
            "set_ephemeral",
            RpcCommand {
                ephemeral: true,
                ..Default::default()
            },
        );
        let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
        if !resp.success {
            return Err(if resp.error.is_empty() {
                "unknown error".to_string()
            } else {
                resp.error.clone()
            });
        }
    }
    if args.no_builtin_tools {
        let cmd = cfg("cfg7", "disable_builtin_tools", RpcCommand::default());
        let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
        if !resp.success {
            return Err(if resp.error.is_empty() {
                "unknown error".to_string()
            } else {
                resp.error.clone()
            });
        }
    }
    if let Some(append) = &args.append_system_prompt {
        if !append.is_empty() {
            let prompt = append.join("\n");
            let cmd = cfg(
                "cfg8",
                "append_system_prompt",
                RpcCommand {
                    system_prompt: prompt,
                    ..Default::default()
                },
            );
            let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
            if !resp.success {
                return Err(if resp.error.is_empty() {
                    "unknown error".to_string()
                } else {
                    resp.error.clone()
                });
            }
        }
    }
    if args.no_context_files {
        let cmd = cfg(
            "cfg9",
            "set_context_files",
            RpcCommand {
                enabled: false,
                ..Default::default()
            },
        );
        let resp = execute_unary(addr, cmd, GRPC_DEADLINE_SEC).await?;
        if !resp.success {
            return Err(if resp.error.is_empty() {
                "unknown error".to_string()
            } else {
                resp.error.clone()
            });
        }
    }
    Ok(())
}

// ─── Print Mode (Non-Interactive) ──────────────────────────────────────────

/// Dial a channel for the event-stream subscription (print mode).
async fn dial_channel(addr: &str) -> Result<future_rpc::connection::VerifiedAgentClient, String> {
    future_rpc::connection::connect_verified(
        addr,
        future_rpc::connection::EXPECTED_AGENT_VERSION,
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
    .await
    .map_err(|error| error.to_string())
}

/// Resolve the session one non-interactive run talks to.
///
/// A print-mode process is a brand-new client, not an attach to some
/// process-wide "current" session: the Agent keeps no default session
/// (sessions are equal peers), so `get_state` with an empty id is rejected.
/// Mirror the interactive startup path instead — an explicit `--session` is
/// validated and adopted, `--continue`/`--fork` resolve the most recently
/// updated session, and anything else mints a fresh session (tagged
/// `createdBy: "tui"`, so other clients discover it).
async fn resolve_print_session(grpc_addr: &str, args: &CliArgs) -> Result<String, String> {
    if let Some(entry_id) = &args.fork {
        let parent = match &args.session {
            Some(session) => session.clone(),
            None => most_recent_session_id(grpc_addr).await?,
        };
        return fork_print_session(grpc_addr, &parent, entry_id).await;
    }
    if let Some(session) = &args.session {
        // Validate now: `switch_session` is the one command that reports a
        // missing session without touching the model or the journal.
        let cmd = switch_session_command(session);
        let resp = execute_unary(grpc_addr, cmd, GRPC_DEADLINE_SEC).await?;
        require_success(&resp, "switch_session")?;
        return Ok(session.clone());
    }
    if args.r#continue {
        return most_recent_session_id(grpc_addr).await;
    }
    new_print_session(grpc_addr).await
}

/// Mint a session for this run, exactly like the interactive client does at
/// startup (its cwd defaults to the process working directory).
async fn new_print_session(grpc_addr: &str) -> Result<String, String> {
    let cmd = RpcCommand {
        id: now_id(),
        r#type: "new_session".to_string(),
        cwd: std::env::current_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default(),
        created_by: "tui".to_string(),
        ..Default::default()
    };
    let resp = execute_unary(grpc_addr, cmd, GRPC_DEADLINE_SEC).await?;
    require_success(&resp, "new_session")?;
    future_rpc::decode::response_data(&resp)
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Agent returned no session for the new conversation".to_string())
}

/// Fork `entry_id` out of `parent`, the same three fields the CLI sends.
async fn fork_print_session(
    grpc_addr: &str,
    parent: &str,
    entry_id: &str,
) -> Result<String, String> {
    let cmd = RpcCommand {
        id: now_id(),
        r#type: "fork".to_string(),
        session_id: parent.to_string(),
        parent_session: parent.to_string(),
        entry_id: entry_id.to_string(),
        created_by: "tui".to_string(),
        ..Default::default()
    };
    let resp = execute_unary(grpc_addr, cmd, GRPC_DEADLINE_SEC).await?;
    require_success(&resp, "fork")?;
    future_rpc::decode::response_data(&resp)
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Fork returned no session".to_string())
}

/// Most recently updated session, by the canonical `updatedAtMs` the Agent
/// already sorts by — the client must not re-derive it from a stale shape.
async fn most_recent_session_id(grpc_addr: &str) -> Result<String, String> {
    let cmd = RpcCommand {
        id: now_id(),
        r#type: "list_sessions".to_string(),
        ..Default::default()
    };
    let resp = execute_unary(grpc_addr, cmd, GRPC_DEADLINE_SEC).await?;
    require_success(&resp, "list_sessions")?;
    let data = future_rpc::decode::response_data(&resp);
    let sessions = data
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut rows = sessions
        .iter()
        .filter_map(|session| {
            let id = session.get("id")?.as_str()?.to_string();
            let updated_at = session
                .get("updatedAtMs")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            Some((updated_at, id))
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(updated_at, _)| std::cmp::Reverse(*updated_at));
    rows.into_iter().next().map(|(_, id)| id).ok_or_else(|| {
        "No previous session to continue; run without --continue to start a new one.".to_string()
    })
}

fn switch_session_command(session_id: &str) -> RpcCommand {
    RpcCommand {
        id: now_id(),
        r#type: "switch_session".to_string(),
        session_id: session_id.to_string(),
        ..Default::default()
    }
}

/// Unary outcome → `Err`, using the Agent's own message when it supplied one.
fn require_success(resp: &RpcResponse, command: &str) -> Result<(), String> {
    if resp.success {
        return Ok(());
    }
    Err(if resp.error.is_empty() {
        format!("{command} failed")
    } else {
        resp.error.clone()
    })
}

/// `runPrintMode` — connect, apply CLI options, stream events, prompt, output.
async fn run_print_mode(grpc_addr: &str, args: &CliArgs) -> Result<(), String> {
    let prompt = build_initial_prompt(&args.file_args, &args.messages)
        .ok_or_else(|| "No prompt provided".to_string())?;

    let session_id = resolve_print_session(grpc_addr, args).await?;
    let is_json_mode = args.mode.as_deref() == Some("json");

    // Apply CLI options.
    apply_cli_options(grpc_addr, &session_id, args).await?;

    // Subscribe to events BEFORE sending the prompt.
    let mut client = dial_channel(grpc_addr).await?;
    let mut stream = client
        .stream_events(StreamRequest {
            session_id: session_id.clone(),
            ..Default::default()
        })
        .await
        .map_err(|status| {
            let msg = status.message();
            if msg.is_empty() {
                status.to_string()
            } else {
                msg.to_string()
            }
        })?
        .into_inner();

    let mut json_messages: Vec<serde_json::Value> = Vec::new();
    let mut text = String::new();

    // Consume stream events until agent_end (bounded by the prompt below).
    let events_task = tokio::spawn(async move {
        let mut result: Result<(), String> = Ok(());
        loop {
            match stream.message().await {
                Ok(Some(event)) => {
                    if is_json_mode {
                        match serde_json::from_str::<serde_json::Value>(&event.data) {
                            Ok(v) => json_messages.push(v),
                            Err(_) => continue,
                        }
                        if event.r#type == "agent_end" {
                            break;
                        }
                    } else {
                        if event.r#type == "text_chunk" {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&event.data) {
                                if let Some(t) = v.get("text").and_then(serde_json::Value::as_str) {
                                    text.push_str(t);
                                }
                            }
                        } else if event.r#type == "error" {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&event.data) {
                                eprintln!(
                                    "{}",
                                    v.get("error")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("unknown error")
                                );
                            }
                        } else if event.r#type == "agent_end" {
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    result = Err("stream error".to_string());
                    break;
                }
            }
        }
        (result, json_messages, text)
    });

    // Send prompt.
    let prompt_cmd = RpcCommand {
        id: now_id(),
        r#type: "prompt".to_string(),
        session_id: session_id.clone(),
        message: prompt.clone(),
        ..Default::default()
    };
    let resp = execute_unary(grpc_addr, prompt_cmd, GRPC_DEADLINE_SEC).await?;
    if !resp.success {
        events_task.abort();
        let err = if resp.error.is_empty() {
            "prompt failed".to_string()
        } else {
            resp.error.clone()
        };
        return Err(err);
    }

    // Wait for the event stream to complete. (The task has no panic paths:
    // every stream outcome is mapped to a value above.)
    let (stream_result, json_messages, text) = events_task.await.expect("events task");
    stream_result?;

    // Output result.
    if is_json_mode {
        let result = serde_json::json!({
            "sessionId": session_id,
            "messages": json_messages,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else if !text.is_empty() {
        println!("{text}");
    }
    Ok(())
}

// ─── List Models ────────────────────────────────────────────────────────────

/// `padEnd` / `padStart` (JS String methods) for the models table.
fn pad_end(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

fn pad_start(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - len), s)
    }
}

/// `fmtNum` — 1.5M / 2.3K / raw.
fn fmt_num(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}K", n / 1_000.0)
    } else {
        format!("{n}")
    }
}

/// `listModels(grpcAddr, search?)` — `list_models` table.
async fn list_models(grpc_addr: &str, search: Option<&str>) -> Result<(), String> {
    let cmd = RpcCommand {
        id: "1".to_string(),
        r#type: "list_models".to_string(),
        ..Default::default()
    };
    let resp = execute_unary(grpc_addr, cmd, GRPC_DEADLINE_SEC).await?;
    if !resp.success {
        let err = if resp.error.is_empty() {
            "unknown error".to_string()
        } else {
            resp.error.clone()
        };
        return Err(err);
    }
    // Typed-first decode, like every sibling print-mode call site: command
    // -response dual-write is retired, so a typed command answers with an
    // empty `data` string and the payload only exists in the typed `payload`.
    let result = future_rpc::decode::response_data(&resp);

    // Same wire shape the interactive client parses (`ModelInfo`): `label`
    // is the display name, and the agent sends no `maxTokens`, so the table
    // has no max-out column.
    //
    // A response without a `models` array is a broken answer, not an empty
    // catalogue: reporting "0 model(s)" would hide a payload the client cannot
    // read, which is the failure this table exists to surface.
    let rows = result
        .get("models")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Agent returned no model list".to_string())?;
    let mut models: Vec<ModelInfo> = rows
        .iter()
        .filter_map(|m| serde_json::from_value(m.clone()).ok())
        .collect();

    if let Some(search) = search {
        let search_lower = search.to_lowercase();
        models.retain(|m| {
            m.id.to_lowercase().contains(&search_lower)
                || m.label.to_lowercase().contains(&search_lower)
                || m.provider.to_lowercase().contains(&search_lower)
        });
    }

    // Compute column widths.
    let max_provider = models
        .iter()
        .map(|m| m.provider.len())
        .chain(std::iter::once("provider".len()))
        .max()
        .unwrap_or(0);
    let max_model = models
        .iter()
        .map(|m| m.id.len())
        .chain(std::iter::once("model".len()))
        .max()
        .unwrap_or(0);
    let ctx_w = "context".len();
    let think_w = "thinking".len();
    let img_w = "images".len();

    let header = format!(
        "{}  {}  {}  {}  {}",
        pad_end("provider", max_provider),
        pad_end("model", max_model),
        pad_start("context", ctx_w),
        pad_start("thinking", think_w),
        pad_start("images", img_w),
    );
    println!("{header}");

    for model in models.iter().take(100) {
        // Absent thinkingLevel means "off" (the agent always sends high/off).
        let thinking = if model.thinking_level == "off" || model.thinking_level.is_empty() {
            "no"
        } else {
            "yes"
        };
        let row = format!(
            "{}  {}  {}  {}  {}",
            pad_end(&model.provider, max_provider),
            pad_end(&model.id, max_model),
            pad_start(&fmt_num(model.context_window as f64), ctx_w),
            pad_start(thinking, think_w),
            pad_start(if model.supports_images { "yes" } else { "no" }, img_w,),
        );
        println!("{row}");
    }

    println!("\n{} model(s)", models.len());
    Ok(())
}

// ─── Main ───────────────────────────────────────────────────────────────────

/// `main()` equivalent — mirrors index.ts's top-level flow and exit codes.
pub fn run(args: &[String]) -> ExitCode {
    // Restore the terminal + capture evidence on panic, before anything can
    // enter raw mode / the alternate screen. Test builds install nothing: the
    // hook is a single process-global slot, so a test-build install would replace
    // libtest's capturing hook and every later panic in the suite would lose its
    // message — see `crash::install_scoped_for_entry_point`.
    let _hook = crate::crash::install_scoped_for_entry_point();

    let args = match parse_args(args) {
        ParseOutcome::Args(a) => a,
        ParseOutcome::Help => {
            println!("{}", crate::help::help_text());
            return ExitCode::SUCCESS;
        }
        ParseOutcome::UnknownOption(arg) => {
            eprintln!("Unknown option: {arg}");
            return ExitCode::from(1);
        }
    };

    // Handle --version.
    if args.version {
        println!("future-tui v{VERSION}");
        return ExitCode::SUCCESS;
    }

    // Build the runtime (print mode + interactive both need one). A build
    // failure means OS resource exhaustion — panic with the reason.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");

    // Attach only. The Agent belongs to its host (`future host`, the desktop
    // app, or an explicit `future agent`); without one this says what to start
    // rather than starting it, so several terminals cannot race over the
    // instance lock. Losing the Agent later keeps showing the client's own
    // "connection lost" state — the TUI does not restart it either.
    if let Err(error) =
        runtime.block_on(crate::agent_attach::ensure_agent_attached(&args.grpc_addr))
    {
        eprintln!("future-tui: {error}");
        return ExitCode::from(1);
    }

    // Handle --list-models.
    if let Some(search) = &args.list_models {
        eprintln!("Connecting to gRPC server at {}", args.grpc_addr);
        let addr = args.grpc_addr.clone();
        let search = if search.is_empty() {
            None
        } else {
            Some(search.clone())
        };
        let result = runtime.block_on(list_models(&addr, search.as_deref()));
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("Error: {err}");
                ExitCode::from(1)
            }
        };
    }

    // Print mode: non-interactive.
    if args.print {
        if args.messages.is_empty() && args.file_args.is_empty() {
            if args.mode.as_deref() != Some("json") {
                eprintln!("No prompt provided. Usage: future-tui -p \"message\"");
            }
            return ExitCode::from(1);
        }
        let addr = args.grpc_addr.clone();
        let result = runtime.block_on(run_print_mode(&addr, &args));
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                if args.mode.as_deref() != Some("json") {
                    eprintln!("Error: {err}");
                }
                ExitCode::from(1)
            }
        };
    }

    // Interactive mode: the workbench, the only terminal surface there is.
    // Attach-only (see `agent_attach`): the Agent belongs to its host, so there
    // is nothing of ours to stop on the way out.
    let options = crate::workbench::run::Options {
        addr: args.grpc_addr.clone(),
        session: args.session.clone(),
        no_context_files: args.no_context_files,
        initial_prompt: build_initial_prompt(&args.file_args, &args.messages),
        continue_recent: args.r#continue,
        offline: args.offline,
        settings_path: crate::settings::path(),
    };
    match runtime.block_on(crate::workbench::run::run(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("future-tui: {error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(rest: &[&str]) -> CliArgs {
        let v: Vec<String> = rest.iter().map(|s| s.to_string()).collect();
        match parse_args(&v) {
            ParseOutcome::Args(a) => a,
            other => panic!("expected Args, got {other:?}"),
        }
    }

    /// Every value-taking flag has a `if i + 1 < args.len()` guard in front of
    /// its assignment. A test that only ever passes the flag *with* a value (or
    /// only bare) leaves the other side of that guard unexecuted — and the bare
    /// side is exactly the "user typed `--model` at the end of the line" case,
    /// where the flag must be accepted and simply stay unset rather than panic.
    ///
    /// Table-driven over the whole flag set, asserting the parsed field both
    /// ways. Note the two families, because they differ on purpose (TS parity):
    /// most flags take the next argument **unconditionally** (so `--session
    /// --verbose` stores `"--verbose"`), while `--print` refuses a following flag
    /// or `@file`. `--list-models` has its own tri-state and is asserted
    /// separately below.
    #[test]
    fn every_value_taking_flag_accepts_its_value_and_tolerates_none() {
        // (flag, value, getter, next-arg-is-consumed-as-the-value)
        #[allow(clippy::type_complexity)]
        let cases: Vec<(&str, &str, fn(&CliArgs) -> Option<String>, bool)> = vec![
            ("--session", "s-1", |a| a.session.clone(), true),
            ("--fork", "s-2", |a| a.fork.clone(), true),
            ("--model", "m-1", |a| a.model.clone(), true),
            ("--provider", "p-1", |a| a.provider.clone(), true),
            ("--api-key", "k-1", |a| a.api_key.clone(), true),
            ("--thinking", "high", |a| a.thinking.clone(), true),
            (
                "--system-prompt",
                "be terse",
                |a| a.system_prompt.clone(),
                true,
            ),
            ("--mode", "json", |a| a.mode.clone(), true),
            // `-p`/`--print` refuses a following flag or `@file` as its message,
            // so it is the guarded case: the flag after it still parses.
            ("-p", "hello", |a| a.messages.first().cloned(), false),
        ];
        for (flag, value, get, consumes_next) in cases {
            let with = args(&[flag, value]);
            assert_eq!(
                get(&with).as_deref(),
                Some(value),
                "{flag} must store its value"
            );

            // Bare, as the last argument: accepted, field left unset.
            let bare = args(&[flag]);
            assert_eq!(
                get(&bare),
                None,
                "{flag} with no value must leave the field unset"
            );

            // A following flag: consumed as the value by the unconditional
            // flags, refused by the guarded ones.
            let followed = args(&[flag, "--verbose"]);
            if consumes_next {
                assert_eq!(
                    get(&followed).as_deref(),
                    Some("--verbose"),
                    "{flag} takes the next argument verbatim"
                );
                assert!(!followed.verbose, "so the flag it swallowed is not set");
            } else {
                assert_eq!(
                    get(&followed),
                    None,
                    "{flag} must not swallow a following flag"
                );
                assert!(followed.verbose, "the following flag still parsed");
            }
        }

        // The flags whose value is a list or a repeatable entry. `--models` and
        // `--tools` split on commas; the repeatable ones store the argument
        // verbatim and accumulate per occurrence.
        #[allow(clippy::type_complexity)]
        let list_cases: Vec<(&str, &str, fn(&CliArgs) -> Option<Vec<String>>, bool, bool)> = vec![
            // (flag, value, getter, splits on comma, allocates an empty list when bare)
            ("--models", "a,b", |a| a.models.clone(), true, false),
            ("--tools", "read,write", |a| a.tools.clone(), true, false),
            (
                "--append-system-prompt",
                "extra",
                |a| a.append_system_prompt.clone(),
                false,
                true,
            ),
            (
                "--prompt-template",
                "tpl",
                |a| a.prompt_template.clone(),
                false,
                true,
            ),
            ("--skill", "sk", |a| a.skill.clone(), false, true),
        ];
        for (flag, value, get, splits, allocates_empty_when_bare) in list_cases {
            let expected: Vec<String> = if splits {
                value.split(',').map(|s| s.to_string()).collect()
            } else {
                vec![value.to_string()]
            };
            let with = args(&[flag, value]);
            assert_eq!(
                get(&with).as_deref(),
                Some(expected.as_slice()),
                "{flag} must store its value"
            );

            // `--models`/`--tools` only call `split_csv` after the guard, so a
            // bare flag leaves `None`; the repeatable flags call
            // `get_or_insert_with(Vec::new)` first, so a bare flag leaves an
            // empty list (the TS `?? []` behaviour).
            let bare = args(&[flag]);
            if allocates_empty_when_bare {
                assert_eq!(
                    get(&bare),
                    Some(Vec::new()),
                    "{flag} with no value allocates an empty list"
                );
            } else {
                assert_eq!(get(&bare), None, "{flag} with no value must stay unset");
            }

            // These are unconditional too: the next argument is the value.
            let followed = args(&[flag, "--verbose"]);
            assert_eq!(
                get(&followed).as_deref(),
                Some(vec!["--verbose".to_string()].as_slice()),
                "{flag} takes the next argument verbatim"
            );
        }

        // `--print` is the guarded one: a flag or `@file` after it is not a
        // message, so the message list stays empty.
        let printed = args(&["-p", "--verbose"]);
        assert!(printed.print);
        assert!(printed.messages.is_empty());
        assert!(printed.verbose);
        assert_eq!(
            args(&["-p", "@notes.md"]).messages.len(),
            0,
            "an @file is not a message"
        );
        assert_eq!(args(&["-p", "hello"]).messages.as_slice(), ["hello"]);

        // A repeatable flag accumulates across occurrences, in order.
        let repeated = args(&[
            "--append-system-prompt",
            "one",
            "--append-system-prompt",
            "two",
            "--skill",
            "s1",
            "--skill",
            "s2",
            "--models",
            "a,b,c",
        ]);
        assert_eq!(
            repeated.append_system_prompt.as_deref(),
            Some(vec!["one".to_string(), "two".to_string()].as_slice())
        );
        assert_eq!(
            repeated.skill.as_deref(),
            Some(vec!["s1".to_string(), "s2".to_string()].as_slice())
        );
        assert_eq!(
            repeated.models.as_deref(),
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()].as_slice()),
            "a comma list splits, and a single entry is still a list"
        );

        // `--list-models` keeps its documented tri-state: absent (None), present
        // with no search (Some("")), present with a search (Some(query)).
        assert_eq!(args(&[]).list_models, None);
        assert_eq!(args(&["--list-models"]).list_models.as_deref(), Some(""));
        assert_eq!(
            args(&["--list-models", "gpt"]).list_models.as_deref(),
            Some("gpt")
        );
        // An `@file` after it is not a search term — it parses as a file arg.
        let with_file = args(&["--list-models", "@notes.md"]);
        assert_eq!(with_file.list_models.as_deref(), Some(""));
        assert_eq!(with_file.file_args.as_slice(), ["notes.md"]);
        // A flag after it is not swallowed either: it is parsed on its own and,
        // being unknown, rejects the whole command line.
        assert!(matches!(
            parse_args(&["--list-models".into(), "-x".into()]),
            ParseOutcome::UnknownOption(option) if option == "-x"
        ));

        // The boolean flags need no value and are not affected by the guards.
        let booleans = args(&[
            "--no-tools",
            "-nt",
            "--no-builtin-tools",
            "-nbt",
            "--no-session",
            "--no-prompt-templates",
            "-np",
            "--no-context-files",
            "-nc",
            "--offline",
            "--verbose",
            "--no-skills",
            "-ns",
            "--continue",
            "-c",
        ]);
        assert!(booleans.no_tools && booleans.no_builtin_tools && booleans.no_session);
        assert!(booleans.no_prompt_templates && booleans.no_context_files);
        assert!(booleans.offline && booleans.verbose && booleans.no_skills);
        assert!(booleans.r#continue);
    }

    /// `ExitCode` exposes no accessor, so the only in-process way to compare it
    /// is its `Debug` form — which does distinguish the success variant from a
    /// failure one, which is the claim being asserted.
    fn exit_code_is_one(code: ExitCode) -> bool {
        format!("{code:?}") == format!("{:?}", ExitCode::from(1))
    }

    /// `run` with a flag but no prompt exits 1 and touches no network — the
    /// branch that has to be right before any gRPC call is attempted.
    #[test]
    fn run_without_a_prompt_exits_one_and_reports_usage() {
        assert!(exit_code_is_one(run(&["-p".to_string()])));
        assert!(
            !exit_code_is_one(run(&["--version".to_string()])),
            "a successful run is not exit 1"
        );
    }

    /// The `--list-models` failure tail: an endpoint with nothing listening is
    /// reported as an error and exits 1. Port 1 is used explicitly so no real
    /// agent can be reached — the test must never talk to a running one.
    #[test]
    fn run_reports_a_list_models_failure() {
        assert!(exit_code_is_one(run(&[
            "--list-models".to_string(),
            "--grpc-addr".to_string(),
            "127.0.0.1:1".to_string(),
        ])));
    }

    /// Print mode with a prompt but an unreachable agent exits 1 through the
    /// `Err` arm of `run_print_mode` (silently in `json` mode, with the error on
    /// stderr otherwise). Port 1 is explicit for the same reason as above.
    #[test]
    fn run_reports_a_print_mode_failure() {
        for extra in [vec![], vec!["--mode".to_string(), "json".to_string()]] {
            let mut argv = vec![
                "-p".to_string(),
                "hello".to_string(),
                "--grpc-addr".to_string(),
                "127.0.0.1:1".to_string(),
            ];
            argv.extend(extra.iter().cloned());
            assert!(
                exit_code_is_one(run(&argv)),
                "an unreachable agent must exit 1: {argv:?}"
            );
        }
    }

    /// `-p --mode json` with no prompt is the machine-readable path: it must
    /// still exit 1, but *silently* — the caller parses the output as JSON, so a
    /// usage line would corrupt it. Covered here by taking the false side of the
    /// `mode != json` guard on the usage line.
    #[test]
    fn run_without_a_prompt_stays_silent_in_json_mode() {
        assert!(exit_code_is_one(run(&[
            "-p".to_string(),
            "--mode".to_string(),
            "json".to_string()
        ])));
    }

    #[test]
    fn parse_model_thinking_colon_split() {
        let a = args(&["--model", "sonnet:high"]);
        assert_eq!(a.model.as_deref(), Some("sonnet"));
        assert_eq!(a.thinking.as_deref(), Some("high"));
    }

    #[test]
    fn parse_model_non_thinking_colon_kept_whole() {
        let a = args(&["--model", "deepseek-v4:beta"]);
        assert_eq!(a.model.as_deref(), Some("deepseek-v4:beta"));
        assert_eq!(a.thinking, None);
    }

    #[test]
    fn parse_print_captures_following_message_only() {
        let a = args(&["-p", "hello world"]);
        assert!(a.print);
        assert_eq!(a.messages, vec!["hello world"]);

        // A flag after -p is not consumed as a message.
        let a = args(&["-p", "--model", "x"]);
        assert!(a.print);
        assert!(a.messages.is_empty());

        // An @file after -p is not consumed as a message.
        let a = args(&["-p", "@notes.txt"]);
        assert!(a.print);
        assert!(a.messages.is_empty());
        assert_eq!(a.file_args, vec!["notes.txt"]);
    }

    #[test]
    fn parse_file_and_message_args() {
        let a = args(&["@a.md", "plain message", "-c"]);
        assert_eq!(a.file_args, vec!["a.md"]);
        assert_eq!(a.messages, vec!["plain message"]);
        assert!(a.r#continue);
    }

    #[test]
    fn parse_list_models_with_search() {
        let a = args(&["--list-models"]);
        assert_eq!(a.list_models.as_deref(), Some(""));
        let a = args(&["--list-models", "deepseek"]);
        assert_eq!(a.list_models.as_deref(), Some("deepseek"));
        // A flag after --list-models is not consumed as the search term.
        let a = args(&["--list-models", "--verbose"]);
        assert_eq!(a.list_models.as_deref(), Some(""));
        assert!(a.verbose);
    }

    #[test]
    fn parse_csv_lists() {
        let a = args(&["--tools", "read, shell", "--models", "a,b"]);
        assert_eq!(a.tools, Some(vec!["read".to_string(), "shell".to_string()]));
        assert_eq!(a.models, Some(vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn parse_append_system_prompt_accumulates() {
        let a = args(&[
            "--append-system-prompt",
            "one",
            "--append-system-prompt",
            "two",
        ]);
        assert_eq!(
            a.append_system_prompt,
            Some(vec!["one".to_string(), "two".to_string()])
        );
    }

    #[test]
    fn parse_grpc_addr() {
        let a = args(&["--grpc-addr", "10.0.0.5:50051"]);
        assert_eq!(a.grpc_addr, "10.0.0.5:50051");
        let a = args(&[]);
        assert_eq!(a.grpc_addr, "auto");
    }

    #[test]
    fn build_initial_prompt_wraps_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        std::fs::write(&file, "line one").unwrap();
        let abs = file.display().to_string();
        let prompt = build_initial_prompt(&[abs], &["question".to_string()]);
        let prompt = prompt.unwrap();
        assert!(prompt.starts_with("<file name=\""));
        assert!(prompt.contains("line one"));
        assert!(prompt.ends_with("</file>\nquestion"));
    }

    #[test]
    fn build_initial_prompt_none_without_args() {
        assert!(build_initial_prompt(&[], &[]).is_none());
    }

    #[test]
    fn build_initial_prompt_returns_none_on_missing_file() {
        assert!(
            build_initial_prompt(&["/nonexistent/definitely-missing.txt".to_string()], &[])
                .is_none()
        );
    }

    #[test]
    fn fmt_num_suffixes() {
        assert_eq!(fmt_num(1_500_000.0), "1.5M");
        assert_eq!(fmt_num(2_300.0), "2.3K");
        assert_eq!(fmt_num(42.0), "42");
        assert_eq!(fmt_num(999.0), "999");
    }

    #[test]
    fn pad_end_and_start() {
        assert_eq!(pad_end("provider", 10), "provider  ");
        assert_eq!(pad_end("longer-than-width", 4), "longer-than-width");
        assert_eq!(pad_start("context", 10), "   context");
        assert_eq!(pad_start("wide", 3), "wide");
    }

    // ─── parse_args gaps ──────────────────────────────────────────────

    #[test]
    fn build_initial_prompt_relative_path_uses_cwd() {
        let _guard = crate::test_env::lock();
        let dir = tempfile::tempdir().unwrap();
        // Canonicalize: current_dir resolves symlinks (/var → /private/var).
        let dir_path = dir.path().canonicalize().unwrap();
        std::fs::write(dir_path.join("rel.txt"), "relative contents").unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir_path).unwrap();
        let result = build_initial_prompt(&["rel.txt".to_string()], &[]);
        std::env::set_current_dir(old_cwd).unwrap();
        let prompt = result.unwrap();
        let expected = dir_path.join("rel.txt").display().to_string();
        assert!(prompt.contains(&format!("<file name=\"{expected}\">")));
        assert!(prompt.contains("relative contents"));
    }

    #[test]
    fn args_helper_panics_on_non_args_outcome() {
        let r = std::panic::catch_unwind(|| args(&["--help"]));
        assert!(r.is_err());
    }

    // ─── In-process mock agent ────────────────────────────────────────

    use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
    use futures_util::stream;
    use futures_util::StreamExt as _;
    use std::collections::{HashMap, HashSet};
    use std::pin::Pin;
    use tonic::transport::server::TcpIncoming;
    use tonic::transport::Server;

    /// Configurable mock: canned data for get_state/list_models,
    /// canned stream events, and command types answered with success=false.
    #[derive(Clone, Default)]
    struct MockAgent {
        state_data: String,
        models_data: String,
        /// Canned `data` for every other command type
        /// (new_session / switch_session / list_sessions / fork).
        command_data: HashMap<String, String>,
        events: Vec<future_rpc::proto::StreamEvent>,
        fail_types: HashSet<String>,
        /// Keep the event stream open (idle) after the canned events instead
        /// of ending it — avoids reconnect flapping in long-running flows.
        hold_open: bool,
        /// Every received command type, for synchronization in tests.
        seen_commands: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        /// Return success=false with an EMPTY error for these types.
        fail_silent_types: HashSet<String>,
        /// stream_events fails with this tonic Status.
        stream_status_error: Option<tonic::Status>,
        /// After the canned events, emit this stream-level error.
        stream_error_after: bool,
        /// Unary calls with these types fail with a tonic Status (empty
        /// message variant for the to_string rendering).
        unary_status_types: HashSet<String>,
        /// Unary calls with these types fail with a message-bearing Status.
        unary_status_message_types: HashSet<String>,
        /// Delay before answering unary calls (slow-agent scenarios).
        unary_delay_ms: u64,
        /// Answer these command types the way the real dispatcher does now:
        /// a typed `payload` and an EMPTY `data` string. Command-response
        /// dual-write is retired, so a mock that only ever sets `data` reports
        /// success for clients that read `data` and breaks on the real Agent.
        typed_only_types: HashSet<String>,
    }

    #[tonic::async_trait]
    impl FutureAgent for MockAgent {
        async fn execute_command(
            &self,
            request: tonic::Request<RpcCommand>,
        ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
            let cmd = request.into_inner();
            if cmd.r#type == "get_agent_readiness" {
                return Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
                id: cmd.id, r#type: "response".into(), command: "get_agent_readiness".into(), success: true,
                data: serde_json::json!({"version": future_rpc::connection::EXPECTED_AGENT_VERSION, "agentInstanceId":"mock-agent", "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION, "dataRootId":"0".repeat(64), "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION}).to_string(),
                ..Default::default()
            }));
            }
            self.seen_commands.lock().unwrap().push(cmd.r#type.clone());
            // Mirror the real dispatcher: commands outside the shared
            // registry are answered with `unknown command`, never a canned
            // success — otherwise a client-side rename stays green against
            // this mock while failing against the real agent (exactly the
            // `get_available_models` drift this guard was added for).
            if future_rpc::command_policy::command_policy(&cmd.r#type).is_none() {
                return Ok(tonic::Response::new(RpcResponse {
                    id: cmd.id,
                    r#type: "response".into(),
                    command: cmd.r#type.clone(),
                    success: false,
                    data: String::new(),
                    error: format!("unknown command: {}", cmd.r#type),
                    error_code: String::new(),
                    error_data: String::new(),
                    payload: None,
                }));
            }
            if self.unary_status_types.contains(&cmd.r#type) {
                return Err(tonic::Status::new(tonic::Code::Unknown, ""));
            }
            if self.unary_status_message_types.contains(&cmd.r#type) {
                return Err(tonic::Status::unavailable("transport down"));
            }
            if self.unary_delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.unary_delay_ms)).await;
            }
            let data = match cmd.r#type.as_str() {
                "get_state" => self.state_data.clone(),
                "list_models" => self.models_data.clone(),
                other => self
                    .command_data
                    .get(other)
                    .cloned()
                    .unwrap_or_else(|| "{}".to_string()),
            };
            let success = !self.fail_types.contains(&cmd.r#type)
                && !self.fail_silent_types.contains(&cmd.r#type);
            if success && self.typed_only_types.contains(&cmd.r#type) {
                // The real Agent's shape: typed payload, no `data` at all. The
                // encode must succeed — a shape drift here would otherwise let
                // the mock fall back to `data` and quietly stop being faithful.
                let value: serde_json::Value = serde_json::from_str(&data).unwrap();
                let payload = future_rpc::encode::response_payload(&cmd.r#type, &value)
                    .unwrap_or_else(|| {
                        panic!(
                            "mock cannot encode a typed payload for {}; the fixture no longer \
                             mirrors the wire",
                            cmd.r#type
                        )
                    });
                return Ok(tonic::Response::new(RpcResponse {
                    id: cmd.id,
                    r#type: "response".into(),
                    command: cmd.r#type.clone(),
                    success: true,
                    data: String::new(),
                    error: String::new(),
                    error_code: String::new(),
                    error_data: String::new(),
                    payload: Some(payload),
                }));
            }
            Ok(tonic::Response::new(RpcResponse {
                id: cmd.id,
                r#type: "response".into(),
                command: cmd.r#type.clone(),
                success,
                data,
                error: if success || self.fail_silent_types.contains(&cmd.r#type) {
                    String::new()
                } else {
                    "boom".into()
                },
                error_code: String::new(),
                error_data: String::new(),
                payload: None,
            }))
        }

        type StreamEventsStream = Pin<
            Box<
                dyn tokio_stream::Stream<
                        Item = Result<future_rpc::proto::StreamEvent, tonic::Status>,
                    > + Send,
            >,
        >;

        async fn stream_events(
            &self,
            _request: tonic::Request<future_rpc::proto::StreamRequest>,
        ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
            if let Some(status) = &self.stream_status_error {
                return Err(status.clone());
            }
            let events = self.events.clone();
            let canned = stream::iter(events.into_iter().map(Ok));
            if self.stream_error_after {
                let err = stream::once(async { Err(tonic::Status::internal("mid-stream boom")) });
                return Ok(tonic::Response::new(Box::pin(canned.chain(err))));
            }
            if self.hold_open {
                let idle =
                    stream::pending::<Result<future_rpc::proto::StreamEvent, tonic::Status>>();
                Ok(tonic::Response::new(Box::pin(canned.chain(idle))))
            } else {
                Ok(tonic::Response::new(Box::pin(canned)))
            }
        }
    }

    async fn spawn_mock(agent: MockAgent) -> String {
        // Serve on the listener bound here: probe-bind → drop → `serve(addr)`
        // left a window where a concurrent mock could take the freed port
        // (observed as an intermittent "transport error" failure).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
        // Spawn the serve future directly — no async-block tail that never
        // completes.
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(agent))
                .serve_with_incoming(incoming),
        );
        format!("127.0.0.1:{}", addr.port())
    }

    fn stream_event(t: &str, data: &str) -> future_rpc::proto::StreamEvent {
        future_rpc::proto::StreamEvent {
            r#type: t.into(),
            data: data.into(),
            ..Default::default()
        }
    }

    // ─── execute_unary / apply_cli_options ────────────────────────────

    #[tokio::test]
    async fn execute_unary_success_error_and_connect_failure() {
        let addr = spawn_mock(MockAgent {
            state_data: "{\"sessionId\":\"s1\"}".into(),
            ..Default::default()
        })
        .await;
        // The success case needs a registered command type — the mock (like
        // the real dispatcher) rejects unknown/empty types.
        let resp = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_state".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap();
        assert!(resp.success);

        // Server-reported failure surfaces the error string.
        let failing = spawn_mock(MockAgent {
            fail_types: HashSet::from(["get_state".to_string()]),
            ..Default::default()
        })
        .await;
        let err = execute_unary(
            &failing,
            RpcCommand {
                r#type: "get_state".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap();
        assert!(!err.success);
        assert_eq!(err.error, "boom");

        // Nothing listening → connect error.
        assert!(execute_unary("127.0.0.1:1", RpcCommand::default(), 5)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn apply_cli_options_sends_all_blocks() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let addr = spawn_mock(MockAgent {
            seen_commands: seen.clone(),
            ..Default::default()
        })
        .await;
        let a = args(&[
            "--model",
            "sonnet",
            "--thinking",
            "high",
            "--system-prompt",
            "sp",
            "--tools",
            "read,shell",
            "--no-tools",
            "--no-session",
            "--no-builtin-tools",
            "--append-system-prompt",
            "extra",
            "--no-context-files",
        ]);
        apply_cli_options(&addr, "s1", &a).await.unwrap();
        // Every block must name its command: an unnamed one reaches the Agent
        // as `unknown command` and the whole print run fails.
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "set_model",
                "set_thinking_level",
                "set_system_prompt",
                "set_tools",
                "disable_tools",
                "set_ephemeral",
                "disable_builtin_tools",
                "append_system_prompt",
                "set_context_files",
            ]
        );
    }

    #[tokio::test]
    async fn apply_cli_options_each_block_error_arm() {
        // Each cfg block has its own !success arm; walk all of them with
        // both the "boom" and the empty-error (→ "unknown error") mocks.
        let loud = spawn_mock(MockAgent {
            fail_types: option_command_types(),
            ..Default::default()
        })
        .await;
        let silent = spawn_mock(MockAgent {
            fail_silent_types: option_command_types(),
            ..Default::default()
        })
        .await;
        let arg_sets: &[&[&str]] = &[
            &["--model", "m"],
            &["--thinking", "high"],
            &["--system-prompt", "sp"],
            &["--tools", "read"],
            &["--no-tools"],
            &["--no-session"],
            &["--no-builtin-tools"],
            &["--append-system-prompt", "x"],
            &["--no-context-files"],
            &["-nc"],
        ];
        for arg_set in arg_sets {
            let a = args(arg_set);
            assert_eq!(apply_cli_options(&loud, "s1", &a).await, Err("boom".into()));
            assert_eq!(
                apply_cli_options(&silent, "s1", &a).await,
                Err("unknown error".into())
            );
        }
        // Empty-list options skip their blocks entirely (white-box).
        let a = CliArgs {
            tools: Some(vec![]),
            append_system_prompt: Some(vec![]),
            ..Default::default()
        };
        apply_cli_options(&loud, "s1", &a).await.unwrap();
    }

    /// The commands `apply_cli_options` can send.
    fn option_command_types() -> HashSet<String> {
        [
            "set_model",
            "set_thinking_level",
            "set_system_prompt",
            "set_tools",
            "disable_tools",
            "set_ephemeral",
            "disable_builtin_tools",
            "append_system_prompt",
            "set_context_files",
        ]
        .iter()
        .map(|name| name.to_string())
        .collect()
    }

    #[tokio::test]
    async fn print_no_context_files_rejection_prevents_prompt() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let addr = spawn_mock(MockAgent {
            command_data: new_session_data(),
            seen_commands: seen.clone(),
            fail_types: HashSet::from(["set_context_files".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            run_print_mode(&addr, &args(&["-p", "hello", "-nc"])).await,
            Err("boom".into())
        );
        assert_eq!(*seen.lock().unwrap(), ["new_session", "set_context_files"]);
    }

    #[tokio::test]
    async fn apply_cli_options_propagates_server_errors() {
        // A rejected option command errors out.
        let addr = spawn_mock(MockAgent {
            fail_types: HashSet::from(["set_model".to_string()]),
            ..Default::default()
        })
        .await;
        let a = args(&["--model", "sonnet"]);
        assert_eq!(apply_cli_options(&addr, "s1", &a).await, Err("boom".into()));
        // No options at all → nothing sent, always Ok.
        let a = args(&[]);
        assert!(apply_cli_options(&addr, "s1", &a).await.is_ok());
    }

    // ─── list_models ──────────────────────────────────────────────────

    /// The wire shape the Agent actually sends for `list_models`: the
    /// `ListModelsPayload` camelCase fields, which `ModelEntryPayload` requires
    /// in full. A partial fixture only survives on the legacy `data` path,
    /// because `ModelInfo` defaults every optional field.
    fn models_json() -> String {
        serde_json::json!({
            "models": [
                {"provider": "openai", "id": "gpt-4o", "label": "GPT-4o",
                 "supportsImages": true, "thinkingLevel": "off",
                 "contextWindow": 128000, "isDefault": false,
                 "reasoning": true, "description": null, "descriptionEn": null,
                 "recommended": false},
                {"provider": "anthropic", "id": "claude-sonnet-4", "label": "Claude Sonnet 4",
                 "supportsImages": false, "thinkingLevel": "high",
                 "contextWindow": 200000, "isDefault": true,
                 "reasoning": true, "description": null, "descriptionEn": null,
                 "recommended": false}
            ],
            "defaultModel": "anthropic/claude-sonnet-4",
            "isScoped": false
        })
        .to_string()
    }

    #[tokio::test]
    async fn list_models_reads_the_typed_payload_the_agent_actually_sends() {
        // Command-response dual-write is retired: the real dispatcher answers
        // `list_models` with a typed `payload` and an empty `data` string. The
        // plain fixture above sets `data`, so it stayed green while the real
        // `future tui --list-models` failed to parse an empty payload.
        let addr = spawn_mock(MockAgent {
            models_data: models_json(),
            typed_only_types: HashSet::from(["list_models".to_string()]),
            ..Default::default()
        })
        .await;
        list_models(&addr, None).await.unwrap();
        list_models(&addr, Some("gpt")).await.unwrap();
    }

    #[tokio::test]
    async fn list_models_prints_table_and_filters() {
        let addr = spawn_mock(MockAgent {
            models_data: models_json(),
            ..Default::default()
        })
        .await;
        list_models(&addr, None).await.unwrap();
        list_models(&addr, Some("gpt")).await.unwrap();
        list_models(&addr, Some("no-such-model")).await.unwrap();
    }

    #[tokio::test]
    async fn list_models_error_paths() {
        // Server-side failure.
        let addr = spawn_mock(MockAgent {
            fail_types: HashSet::from(["list_models".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(list_models(&addr, None).await, Err("boom".into()));
        // …and with an empty error → "unknown error".
        let addr = spawn_mock(MockAgent {
            fail_silent_types: HashSet::from(["list_models".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(list_models(&addr, None).await, Err("unknown error".into()));
        // Invalid JSON payload.
        let addr = spawn_mock(MockAgent {
            models_data: "not json".into(),
            ..Default::default()
        })
        .await;
        assert!(list_models(&addr, None).await.is_err());
        // Unreachable agent.
        assert!(list_models("127.0.0.1:1", None).await.is_err());
    }

    // ─── run_print_mode ───────────────────────────────────────────────

    /// Canned reply for the session a non-interactive run mints by default.
    fn new_session_data() -> HashMap<String, String> {
        HashMap::from([(
            "new_session".to_string(),
            "{\"sessionId\":\"s1\"}".to_string(),
        )])
    }

    #[tokio::test]
    async fn print_mode_text_stream() {
        let addr = spawn_mock(MockAgent {
            command_data: new_session_data(),
            events: vec![
                stream_event("text_chunk", "{\"text\":\"Hello\"}"),
                stream_event("error", "{\"error\":\"transient\"}"),
                stream_event("error", "not json"),
                stream_event("agent_end", "{}"),
            ],
            ..Default::default()
        })
        .await;
        let a = args(&["-p", "hi"]);
        run_print_mode(&addr, &a).await.unwrap();
    }

    #[tokio::test]
    async fn print_mode_json_stream() {
        let addr = spawn_mock(MockAgent {
            command_data: new_session_data(),
            events: vec![
                stream_event("text_chunk", "{\"text\":\"Hello\"}"),
                stream_event("bogus", "not json"), // skipped (continue)
                stream_event("agent_end", "{}"),
            ],
            ..Default::default()
        })
        .await;
        let a = args(&["-p", "hi", "--mode", "json"]);
        run_print_mode(&addr, &a).await.unwrap();
    }

    /// A non-interactive run resolves its own session: the Agent has no
    /// default session, so `get_state` with an empty id is never an option.
    #[tokio::test]
    async fn print_mode_resolves_its_session() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let addr = spawn_mock(MockAgent {
            command_data: HashMap::from([
                (
                    "new_session".to_string(),
                    "{\"sessionId\":\"s-new\"}".to_string(),
                ),
                (
                    "switch_session".to_string(),
                    "{\"cancelled\":false}".to_string(),
                ),
                (
                    "list_sessions".to_string(),
                    serde_json::json!({"sessions": [
                        {"id": "s-old", "updatedAtMs": 100},
                        {"id": "s-recent", "updatedAtMs": 900},
                        {"id": "s-untimed"}
                    ]})
                    .to_string(),
                ),
                ("fork".to_string(), "{\"sessionId\":\"s-fork\"}".to_string()),
            ]),
            seen_commands: seen.clone(),
            ..Default::default()
        })
        .await;
        let commands = || seen.lock().unwrap().clone();

        // No session option → mint one (and never read "current" state).
        assert_eq!(
            resolve_print_session(&addr, &args(&["-p", "hi"])).await,
            Ok("s-new".to_string())
        );
        assert_eq!(commands(), vec!["new_session".to_string()]);

        // --session → adopted after validation, no new session.
        seen.lock().unwrap().clear();
        assert_eq!(
            resolve_print_session(&addr, &args(&["-p", "hi", "--session", "s-given"])).await,
            Ok("s-given".to_string())
        );
        assert_eq!(commands(), vec!["switch_session".to_string()]);

        // --continue → the session with the newest updatedAtMs.
        seen.lock().unwrap().clear();
        assert_eq!(
            resolve_print_session(&addr, &args(&["-p", "hi", "--continue"])).await,
            Ok("s-recent".to_string())
        );
        assert_eq!(commands(), vec!["list_sessions".to_string()]);

        // --fork → forked from the explicit parent.
        seen.lock().unwrap().clear();
        assert_eq!(
            resolve_print_session(
                &addr,
                &args(&["-p", "hi", "--session", "s-parent", "--fork", "entry-1"])
            )
            .await,
            Ok("s-fork".to_string())
        );
        assert_eq!(commands(), vec!["fork".to_string()]);

        // --resume is no longer a flag at all: the picker it opened went with
        // the inline UI, so print mode cannot be asked for it.
        assert!(matches!(
            parse_args(&["-p".to_string(), "hi".to_string(), "--resume".to_string()]),
            ParseOutcome::UnknownOption(_)
        ));
    }

    #[tokio::test]
    async fn print_mode_session_resolution_failures() {
        // An unknown --session fails on validation, before any prompt.
        let missing = spawn_mock(MockAgent {
            fail_types: HashSet::from(["switch_session".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            resolve_print_session(&missing, &args(&["-p", "hi", "--session", "gone"])).await,
            Err("boom".to_string())
        );

        // --continue with nothing to continue.
        let empty = spawn_mock(MockAgent {
            command_data: HashMap::from([(
                "list_sessions".to_string(),
                "{\"sessions\": []}".to_string(),
            )]),
            ..Default::default()
        })
        .await;
        let err = resolve_print_session(&empty, &args(&["-p", "hi", "--continue"]))
            .await
            .unwrap_err();
        assert!(err.contains("No previous session to continue"), "{err}");

        // --continue when the list itself fails (empty error → default label).
        let failing = spawn_mock(MockAgent {
            fail_silent_types: HashSet::from(["list_sessions".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            resolve_print_session(&failing, &args(&["-p", "hi", "--continue"])).await,
            Err("list_sessions failed".to_string())
        );

        // --fork without --session needs a parent to fork from.
        let err = resolve_print_session(&empty, &args(&["-p", "hi", "--fork", "entry-1"]))
            .await
            .unwrap_err();
        assert!(err.contains("No previous session to continue"), "{err}");

        // A fork reply without a session id is an error.
        let bad_fork = spawn_mock(MockAgent {
            command_data: HashMap::from([("fork".to_string(), "{}".to_string())]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            resolve_print_session(
                &bad_fork,
                &args(&["-p", "hi", "--fork", "e", "--session", "p"])
            )
            .await,
            Err("Fork returned no session".to_string())
        );
    }

    #[tokio::test]
    async fn execute_unary_status_error_message_forms() {
        let mock = MockAgent {
            unary_status_types: HashSet::from(["get_state".to_string()]),
            unary_status_message_types: HashSet::from(["get_messages".to_string()]),
            ..Default::default()
        };
        let addr = spawn_mock(mock).await;
        // Status with an empty message → to_string rendering.
        let err = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_state".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Unknown"));
        // Status with a message → the message itself.
        let err = execute_unary(
            &addr,
            RpcCommand {
                r#type: "get_messages".into(),
                ..Default::default()
            },
            5,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "transport down");
    }

    #[tokio::test]
    async fn dial_and_unary_address_arms() {
        // Unparseable address → the from_shared map_err arms.
        assert!(dial_channel("bad addr with spaces").await.is_err());
        assert!(
            execute_unary("bad addr with spaces", RpcCommand::default(), 1)
                .await
                .is_err()
        );
        // Parseable but nothing listening → the connect map_err arm.
        assert!(dial_channel("127.0.0.1:1").await.is_err());
    }

    #[tokio::test]
    async fn print_mode_error_paths() {
        // No prompt content.
        let a = args(&["-p"]);
        assert_eq!(
            run_print_mode("127.0.0.1:1", &a).await,
            Err("No prompt provided".to_string())
        );
        // new_session failure.
        let failing_new_session = spawn_mock(MockAgent {
            fail_types: HashSet::from(["new_session".to_string()]),
            ..Default::default()
        })
        .await;
        let a = args(&["-p", "hi"]);
        assert_eq!(
            run_print_mode(&failing_new_session, &a).await,
            Err("boom".to_string())
        );
        // …with an empty error → the "new_session failed" default.
        let silent_new_session = spawn_mock(MockAgent {
            fail_silent_types: HashSet::from(["new_session".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            run_print_mode(&silent_new_session, &a).await,
            Err("new_session failed".to_string())
        );
        // A reply without a session id (non-JSON data decodes to null).
        let bad_json = spawn_mock(MockAgent {
            command_data: HashMap::from([("new_session".to_string(), "nope".to_string())]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            run_print_mode(&bad_json, &a).await,
            Err("Agent returned no session for the new conversation".to_string())
        );
        // Prompt command failure.
        let failing_prompt = spawn_mock(MockAgent {
            command_data: new_session_data(),
            fail_types: HashSet::from(["prompt".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            run_print_mode(&failing_prompt, &a).await,
            Err("boom".to_string())
        );
        // …with an empty error → the "prompt failed" default.
        let silent_prompt = spawn_mock(MockAgent {
            command_data: new_session_data(),
            fail_silent_types: HashSet::from(["prompt".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(
            run_print_mode(&silent_prompt, &a).await,
            Err("prompt failed".to_string())
        );
    }

    #[tokio::test]
    async fn print_mode_stream_failures() {
        // stream_events fails at subscribe time (message-bearing status).
        let a = args(&["-p", "hi"]);
        let mock = MockAgent {
            command_data: new_session_data(),
            stream_status_error: Some(tonic::Status::internal("stream boom")),
            ..Default::default()
        };
        let addr = spawn_mock(mock).await;
        assert!(run_print_mode(&addr, &a).await.is_err());

        // Same failure with an empty status message.
        let mock = MockAgent {
            command_data: new_session_data(),
            stream_status_error: Some(tonic::Status::new(tonic::Code::Unknown, "")),
            ..Default::default()
        };
        let addr = spawn_mock(mock).await;
        assert!(run_print_mode(&addr, &a).await.is_err());

        // A stream-level error mid-run → "stream error".
        let mock = MockAgent {
            command_data: new_session_data(),
            events: vec![stream_event("text_chunk", "{\"text\":\"partial\"}")],
            stream_error_after: true,
            ..Default::default()
        };
        let addr = spawn_mock(mock).await;
        assert_eq!(
            run_print_mode(&addr, &a).await,
            Err("stream error".to_string())
        );
    }

    #[tokio::test]
    async fn print_mode_event_data_edge_cases() {
        // text_chunk with non-JSON data is skipped; an error event without
        // an "error" key renders the default; empty text prints nothing.
        let mock = MockAgent {
            command_data: new_session_data(),
            events: vec![
                stream_event("text_chunk", "not json"),
                stream_event("text_chunk", "{\"wrong\":1}"),
                stream_event("tool_start", "{}"), // unrecognized type is skipped
                stream_event("error", "{\"no_error_key\":true}"),
                stream_event("agent_end", "{}"),
            ],
            ..Default::default()
        };
        let addr = spawn_mock(mock).await;
        let a = args(&["-p", "hi"]);
        run_print_mode(&addr, &a).await.unwrap();
    }

    // ─── run() top-level flow ─────────────────────────────────────────
}
