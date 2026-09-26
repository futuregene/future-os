//! CLI entry point — port of `tui/src/index.ts` (777 lines).
//!
//!   - `parse_args` — 1:1 arg parsing (incl. the `--model model:thinking`
//!     colon split, `-p` message capture, `@file` args, `--help` exiting
//!     during scanning, unknown-option exit 1)
//!   - `list_models` — `get_available_models` table (provider/model/context/
//!     max-out/thinking/images, K/M suffixes, 100-row cap)
//!   - `run_print_mode` — non-interactive `-p` prompt via raw gRPC
//!   - `run_interactive` — the App event loop (input/events/cmds/timers)

use crate::app::{App, CliOptions, UiCmd, UiInput};
use crate::rpc::grpc_client::GrpcClient;
use crate::version::VERSION;
use future_rpc::proto::{RpcCommand, RpcResponse, StreamRequest};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

const GRPC_DEADLINE_SEC: u64 = 30;

// ─── CLI Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct CliArgs {
    pub grpc_addr: String,
    pub session: Option<String>,
    pub r#continue: bool,
    pub resume: bool,
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
            "--resume" | "-r" => result.resume = true,
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
    let connected = future_rpc::transport::connect_channel(
        Some(addr),
        Duration::from_secs(timeout_secs.min(5)),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut client = future_rpc::transport::agent_client(connected.channel);
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
async fn dial_channel(addr: &str) -> Result<tonic::transport::Channel, String> {
    future_rpc::transport::connect_channel(Some(addr), Duration::from_secs(5), None)
        .await
        .map(|connected| connected.channel)
        .map_err(|e| e.to_string())
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
    if args.resume {
        return Err(
            "--resume opens a session picker and needs an interactive terminal; use --continue or --session"
                .to_string(),
        );
    }
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
    let channel = dial_channel(grpc_addr).await?;
    let mut client = future_rpc::transport::agent_client(channel);
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

/// `listModels(grpcAddr, search?)` — `get_available_models` table.
async fn list_models(grpc_addr: &str, search: Option<&str>) -> Result<(), String> {
    let cmd = RpcCommand {
        id: "1".to_string(),
        r#type: "get_available_models".to_string(),
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
    let result: serde_json::Value = serde_json::from_str(&resp.data).map_err(|e| e.to_string())?;

    struct Row {
        provider: String,
        id: String,
        name: String,
        reasoning: bool,
        image: bool,
        context_window: f64,
        max_tokens: f64,
    }
    let mut models: Vec<Row> = result
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let obj = m.as_object()?;
                    Some(Row {
                        provider: obj
                            .get("provider")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        id: obj
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: obj
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        reasoning: obj
                            .get("reasoning")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                        image: obj
                            .get("image")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                        context_window: obj
                            .get("contextWindow")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0),
                        max_tokens: obj
                            .get("maxTokens")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if let Some(search) = search {
        let search_lower = search.to_lowercase();
        models.retain(|m| {
            m.id.to_lowercase().contains(&search_lower)
                || m.name.to_lowercase().contains(&search_lower)
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
    let out_w = "max-out".len();
    let think_w = "thinking".len();
    let img_w = "images".len();

    let header = format!(
        "{}  {}  {}  {}  {}  {}",
        pad_end("provider", max_provider),
        pad_end("model", max_model),
        pad_start("context", ctx_w),
        pad_start("max-out", out_w),
        pad_start("thinking", think_w),
        pad_start("images", img_w),
    );
    println!("{header}");

    for model in models.iter().take(100) {
        let row = format!(
            "{}  {}  {}  {}  {}  {}",
            pad_end(&model.provider, max_provider),
            pad_end(&model.id, max_model),
            pad_start(&fmt_num(model.context_window), ctx_w),
            pad_start(&fmt_num(model.max_tokens), out_w),
            pad_start(if model.reasoning { "yes" } else { "no" }, think_w),
            pad_start(if model.image { "yes" } else { "no" }, img_w),
        );
        println!("{row}");
    }

    println!("\n{} model(s)", models.len());
    Ok(())
}

// ─── Interactive mode ───────────────────────────────────────────────────────

fn tui_settings_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".future")
        .join("tui")
        .join("settings.json")
}

/// Build the interactive App and run its event loop until exit.
async fn run_interactive(args: &CliArgs) -> u8 {
    let (op_tx, mut op_rx) = mpsc::unbounded_channel::<UiCmd>();
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<UiInput>();

    let terminal = match crate::terminal::Terminal::new() {
        Ok(t) => t,
        Err(err) => {
            eprintln!("future-tui: failed to initialize terminal: {err}");
            return 1;
        }
    };

    // Wire SIGINT/SIGTERM to the graceful stop path (TS process.on).
    let mut terminal = terminal;
    let sig_tx = input_tx.clone();
    terminal.set_exit_signal_callback(Some(Box::new(move || {
        let _ = sig_tx.send(UiInput::ExitSignal);
    })));

    let (client, mut event_rx, mut conn_rx) = GrpcClient::new(&args.grpc_addr);
    let client = Arc::new(client.with_no_context_files(args.no_context_files));

    let cli_options = CliOptions {
        session: args.session.clone(),
        r#continue: args.r#continue,
        resume: args.resume,
        fork: args.fork.clone(),
        initial_prompt: if !args.print && (!args.messages.is_empty() || !args.file_args.is_empty())
        {
            build_initial_prompt(&args.file_args, &args.messages)
        } else {
            None
        },
    };

    let mut app = App::new(
        terminal,
        client.clone(),
        op_tx,
        &cli_options,
        tui_settings_path(),
    );

    // Startup: enter raw mode, wait for the agent, establish the session.
    // The startup future borrows `app`; an interrupt (SIGINT / Ctrl+C during
    // the agent handshake) is signalled through a watch so the loop does not
    // touch the app while it is borrowed. The whole phase lives in a nested
    // scope so the future (and its `&mut app` borrow) is dropped before the
    // main loop uses the app.
    let startup_result: Result<(), std::io::Error> = {
        let (interrupt_tx, mut interrupt_rx) = tokio::sync::watch::channel(false);
        let startup = async {
            tokio::select! {
                r = app.start(input_tx.clone()) => r,
                _ = interrupt_rx.changed() => Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "interrupted during startup",
                )),
            }
        };
        let mut startup = std::pin::pin!(startup);
        loop {
            tokio::select! {
                r = &mut startup => break r,
                Some(ui) = input_rx.recv() => {
                    match ui {
                        UiInput::ExitSignal => {
                            let _ = interrupt_tx.send(true);
                        }
                        UiInput::Input(d) => {
                            if d == "\x03" {
                                // Interrupt during startup — exit like the TS app.
                                let _ = interrupt_tx.send(true);
                            }
                            // Other input during startup is dropped (the agent
                            // handshake owns the screen).
                        }
                        UiInput::Resize => {}
                    }
                }
            }
        }
    };
    // The startup future is dropped with the scope above; `app` is free now.
    match startup_result {
        Ok(()) => {}
        Err(err) => {
            eprintln!("future-tui: {err}");
            app.stop();
            return 1;
        }
    }
    // (No is_running gate here: the main loop's own check handles it.)

    // Poll once alongside input/events, never during the startup handshake.
    // Dropping this future on exit cancels an in-flight check.
    let update_check = async {
        if args.offline {
            None
        } else {
            crate::update::check(VERSION).await
        }
    };
    tokio::pin!(update_check);
    let mut update_checked = false;

    // ── Main event loop ────────────────────────────────────────────────
    loop {
        let next = app.next_deadline();
        let sleep = match next {
            Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)),
            None => tokio::time::sleep(Duration::from_millis(50)),
        };
        tokio::pin!(sleep);
        tokio::select! {
            biased;
            Some(ui) = input_rx.recv() => {
                match ui {
                    UiInput::Input(d) => app.handle_input(&d),
                    UiInput::Resize => app.request_resize_render(),
                    UiInput::ExitSignal => {
                        app.stop();
                        break;
                    }
                }
            }
            notice = &mut update_check, if !update_checked => {
                update_checked = true;
                if let Some(notice) = notice {
                    app.handle_cmd(UiCmd::UpdateAvailable(notice));
                }
            }
            Some(cmd) = op_rx.recv() => app.handle_cmd(cmd),
            Some(event) = event_rx.recv() => app.handle_agent_event(&event),
            changed = conn_rx.changed() => {
                if changed.is_ok() {
                    app.on_connection_change(*conn_rx.borrow());
                }
            }
            _ = &mut sleep => app.on_tick(),
        }
        if !app.is_running() {
            app.stop();
            break;
        }
    }
    0
}

// ─── Main ───────────────────────────────────────────────────────────────────

/// `main()` equivalent — mirrors index.ts's top-level flow and exit codes.
pub fn run(args: &[String]) -> ExitCode {
    // Restore the terminal + capture evidence on panic, before anything can
    // enter raw mode / the alternate screen.
    crate::crash::install();

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

    // Desktop-style ownership: attach to an existing Agent when possible;
    // otherwise launch a sidecar and keep it alive for the TUI lifetime.
    let _owned_agent = match runtime.block_on(crate::agent_supervisor::ensure_agent_running(
        &args.grpc_addr,
    )) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("future-tui: {error}");
            return ExitCode::from(1);
        }
    };

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

    // Interactive mode (TUI).
    let code = runtime.block_on(run_interactive(&args));
    ExitCode::from(code)
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

    #[test]
    fn parse_version_flag() {
        let a = args(&["--version"]);
        assert!(a.version);
        let a = args(&["-v"]);
        assert!(a.version);
    }

    #[test]
    fn parse_help_exits_during_scanning() {
        // --help terminates parsing immediately (TS process.exit inside the
        // loop) — even a later unknown option is never reached.
        assert!(matches!(
            parse_args(&["--help".into(), "--unknown-flag".into()]),
            ParseOutcome::Help
        ));
        assert!(matches!(parse_args(&["-h".into()]), ParseOutcome::Help));
    }

    #[test]
    fn parse_unknown_option_rejected() {
        assert!(matches!(
            parse_args(&["--bogus".into()]),
            ParseOutcome::UnknownOption(_)
        ));
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
            "--resume",
            "-r",
        ]);
        assert!(booleans.no_tools && booleans.no_builtin_tools && booleans.no_session);
        assert!(booleans.no_prompt_templates && booleans.no_context_files);
        assert!(booleans.offline && booleans.verbose && booleans.no_skills);
        assert!(booleans.r#continue && booleans.resume);
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

    /// With no mode flags at all, `run` falls through to the interactive TUI.
    /// A test must **not** drive that: the event loop would talk to a real agent
    /// and never return. Only the pre-connection path is asserted, and only where
    /// no console exists — in a redirected runner like CI, `Terminal::new` fails
    /// and `run` reports 1 instead of entering the loop. Where a console *is*
    /// available the test declines rather than risk the user's session.
    #[test]
    fn run_without_flags_reports_a_missing_console_instead_of_entering_the_loop() {
        // Drain the injected-failure seam so this probe reflects reality.
        crate::terminal::FORCE_NEW_FAILURE.swap(false, std::sync::atomic::Ordering::SeqCst);
        if crate::terminal::Terminal::new().is_ok() {
            eprintln!(
                "[skip] a console is available here; the interactive loop must not be \
                 driven from a test"
            );
            return;
        }
        assert!(exit_code_is_one(run(&[])));
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
    fn parse_value_options() {
        let a = args(&[
            "--session",
            "s1",
            "--fork",
            "e9",
            "--provider",
            "openai",
            "--api-key",
            "sk-test",
            "--thinking",
            "high",
            "--system-prompt",
            "be nice",
            "--mode",
            "json",
            "--prompt-template",
            "t1",
            "--skill",
            "review",
        ]);
        assert_eq!(a.session.as_deref(), Some("s1"));
        assert_eq!(a.fork.as_deref(), Some("e9"));
        assert_eq!(a.provider.as_deref(), Some("openai"));
        assert_eq!(a.api_key.as_deref(), Some("sk-test"));
        assert_eq!(a.thinking.as_deref(), Some("high"));
        assert_eq!(a.system_prompt.as_deref(), Some("be nice"));
        assert_eq!(a.mode.as_deref(), Some("json"));
        assert_eq!(a.prompt_template.as_deref(), Some(&["t1".to_string()][..]));
        assert_eq!(a.skill.as_deref(), Some(&["review".to_string()][..]));
    }

    #[test]
    fn parse_model_colon_edge_cases() {
        // Colon at position 0 → no model/thinking split happens.
        let a = args(&["--model", ":high"]);
        assert!(a.model.is_none());
        assert!(a.thinking.is_none());
        // Trailing colon → kept whole (empty suffix is not a level).
        let a = args(&["--model", "sonnet:"]);
        assert_eq!(a.model.as_deref(), Some("sonnet:"));
        // No colon at all.
        let a = args(&["--model", "sonnet"]);
        assert_eq!(a.model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn parse_flags_without_trailing_values_are_ignored() {
        // A value option as the last argument simply has no value.
        let a = args(&["--session"]);
        assert!(a.session.is_none());
        let a = args(&["--model"]);
        assert!(a.model.is_none());
        let a = args(&["--grpc-addr"]);
        assert_eq!(a.grpc_addr, "auto");
    }

    #[test]
    fn build_initial_prompt_uses_absolute_path_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("abs.txt");
        std::fs::write(&file, "contents").unwrap();
        let abs = file.to_str().unwrap().to_string();
        let prompt = build_initial_prompt(std::slice::from_ref(&abs), &[]).expect("prompt built");
        assert!(prompt.contains(&format!("<file name=\"{abs}\">")));
        assert!(prompt.contains("contents"));
    }

    #[test]
    fn now_id_is_epoch_millis() {
        let id = now_id();
        let v: u128 = id.parse().expect("numeric id");
        assert!(v > 1_000_000_000_000); // after 2001
    }

    #[test]
    fn tui_settings_path_lives_under_home() {
        let path = tui_settings_path();
        assert!(path.ends_with(".future/tui/settings.json"));
    }

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

    /// Configurable mock: canned data for get_state/get_available_models,
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
    }

    #[tonic::async_trait]
    impl FutureAgent for MockAgent {
        async fn execute_command(
            &self,
            request: tonic::Request<RpcCommand>,
        ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
            let cmd = request.into_inner();
            self.seen_commands.lock().unwrap().push(cmd.r#type.clone());
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
                "get_available_models" => self.models_data.clone(),
                other => self
                    .command_data
                    .get(other)
                    .cloned()
                    .unwrap_or_else(|| "{}".to_string()),
            };
            let success = !self.fail_types.contains(&cmd.r#type)
                && !self.fail_silent_types.contains(&cmd.r#type);
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
        let resp = execute_unary(&addr, RpcCommand::default(), 5)
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

    fn models_json() -> String {
        serde_json::json!({
            "models": [
                {"provider": "openai", "id": "gpt-4o", "name": "GPT-4o",
                 "reasoning": false, "image": true,
                 "contextWindow": 128000, "maxTokens": 4096},
                {"provider": "anthropic", "id": "claude-sonnet-4", "name": "Claude Sonnet 4",
                 "reasoning": true, "image": false,
                 "contextWindow": 200000, "maxTokens": 8192}
            ]
        })
        .to_string()
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
            fail_types: HashSet::from(["get_available_models".to_string()]),
            ..Default::default()
        })
        .await;
        assert_eq!(list_models(&addr, None).await, Err("boom".into()));
        // …and with an empty error → "unknown error".
        let addr = spawn_mock(MockAgent {
            fail_silent_types: HashSet::from(["get_available_models".to_string()]),
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

        // --resume needs a terminal; it never reaches the agent.
        seen.lock().unwrap().clear();
        assert!(
            resolve_print_session(&addr, &args(&["-p", "hi", "--resume"]))
                .await
                .is_err()
        );
        assert!(commands().is_empty());
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

    #[test]
    fn run_help_version_and_unknown_option() {
        assert_eq!(run(&["--help".to_string()]), ExitCode::SUCCESS);
        assert_eq!(run(&["--version".to_string()]), ExitCode::SUCCESS);
        assert_eq!(run(&["--bogus".to_string()]), ExitCode::from(1));
    }

    #[test]
    fn run_list_models_unreachable_agent() {
        let code = run(&[
            "--list-models".to_string(),
            "--grpc-addr".to_string(),
            "127.0.0.1:1".to_string(),
        ]);
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn run_list_models_against_mock_succeeds() {
        let mock_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let addr = mock_rt.block_on(spawn_mock(MockAgent {
            models_data: models_json(),
            ..Default::default()
        }));
        // With and without a search term.
        let code = run(&[
            "--list-models".to_string(),
            "--grpc-addr".to_string(),
            addr.clone(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
        let code = run(&[
            "--list-models".to_string(),
            "gpt".to_string(),
            "--grpc-addr".to_string(),
            addr,
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn run_print_failure_json_mode_stays_quiet() {
        // The error message is suppressed in json mode.
        let code = run(&[
            "-p".to_string(),
            "hi".to_string(),
            "--mode".to_string(),
            "json".to_string(),
            "--grpc-addr".to_string(),
            "127.0.0.1:1".to_string(),
        ]);
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn run_print_failure_text_mode_prints_error() {
        let code = run(&[
            "-p".to_string(),
            "hi".to_string(),
            "--grpc-addr".to_string(),
            "127.0.0.1:1".to_string(),
        ]);
        assert_eq!(code, ExitCode::from(1));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env-var serialization across awaits
    async fn run_interactive_terminal_init_failure() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());
        // Call the interactive path directly: run() can fail at agent discovery
        // before Terminal::new, leaking the injected failure into another test.
        crate::terminal::FORCE_NEW_FAILURE.store(true, std::sync::atomic::Ordering::SeqCst);
        let code = run_interactive(&args(&["--offline"])).await;
        restore_env("HOME", old_home);
        assert_eq!(code, 1);
    }

    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn restore_env_handles_set_and_unset() {
        let _guard = crate::test_env::lock();
        let old = std::env::var_os("FUTURE_TUI_INDEX_PROBE");
        restore_env("FUTURE_TUI_INDEX_PROBE", Some("1".into()));
        assert_eq!(std::env::var("FUTURE_TUI_INDEX_PROBE").as_deref(), Ok("1"));
        restore_env("FUTURE_TUI_INDEX_PROBE", None);
        assert!(std::env::var_os("FUTURE_TUI_INDEX_PROBE").is_none());
        restore_env("FUTURE_TUI_INDEX_PROBE", old);
    }

    #[test]
    fn run_print_without_message_fails() {
        let code = run(&["-p".to_string()]);
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn run_print_against_mock_succeeds() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());
        // Multi-thread runtime: the mock server keeps being driven on worker
        // threads while run() blocks on its own runtime.
        let mock_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let addr = mock_rt.block_on(spawn_mock(MockAgent {
            command_data: new_session_data(),
            events: vec![
                stream_event("text_chunk", "{\"text\":\"Hi\"}"),
                stream_event("agent_end", "{}"),
            ],
            ..Default::default()
        }));
        let code = run(&[
            "-p".to_string(),
            "hello".to_string(),
            "--grpc-addr".to_string(),
            addr,
        ]);
        restore_env("HOME", old_home);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[cfg(unix)]
    #[test]
    fn run_interactive_fails_fast_without_tty() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());
        // Make stdin deterministically not-a-TTY (a developer shell may have
        // a real one): swap fd 0 for /dev/null for the duration.
        let code = with_null_stdin(|| run(&["--grpc-addr".to_string(), "127.0.0.1:1".to_string()]));
        restore_env("HOME", old_home);
        assert_eq!(code, ExitCode::from(1));
    }

    /// Run `f` with fd 0 redirected from /dev/null (restored afterwards).
    #[cfg(unix)]
    fn with_null_stdin<F: FnOnce() -> std::process::ExitCode>(f: F) -> std::process::ExitCode {
        use std::os::unix::io::AsRawFd;
        let devnull = std::fs::File::open("/dev/null").unwrap();
        let null_fd = devnull.as_raw_fd();
        unsafe {
            let saved = libc::dup(0);
            libc::dup2(null_fd, 0);
            let code = f();
            libc::dup2(saved, 0);
            libc::close(saved);
            code
        }
    }

    /// A PTY pair with fd 0 redirected to the slave (restored on drop).
    #[cfg(unix)]
    struct PtyStdin {
        master: i32,
        slave: i32,
        saved: i32,
    }

    #[cfg(unix)]
    impl PtyStdin {
        fn install() -> Self {
            unsafe {
                let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
                assert!(master >= 0);
                assert_eq!(libc::grantpt(master), 0);
                assert_eq!(libc::unlockpt(master), 0);
                let slave_name = libc::ptsname(master);
                assert!(!slave_name.is_null());
                let slave = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
                assert!(slave >= 0);
                let saved = libc::dup(0);
                assert!(saved >= 0);
                assert_ne!(libc::dup2(slave, 0), -1);
                Self {
                    master,
                    slave,
                    saved,
                }
            }
        }

        fn write_input(&self, data: &str) {
            unsafe {
                libc::write(
                    self.master,
                    data.as_ptr() as *const libc::c_void,
                    data.len(),
                );
            }
        }
    }

    #[cfg(unix)]
    impl Drop for PtyStdin {
        fn drop(&mut self) {
            unsafe {
                libc::dup2(self.saved, 0);
                libc::close(self.saved);
                libc::close(self.slave);
                libc::close(self.master);
            }
        }
    }

    /// Full interactive loop against a PTY + mock agent: startup completes,
    /// the main event loop runs, and a ctrl+c byte through the PTY quits.
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env-var serialization across awaits
    async fn interactive_loop_runs_and_ctrl_c_quits() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let addr = spawn_mock(MockAgent {
            state_data: "{\"sessionId\":\"s1\"}".into(),
            events: vec![stream_event("ping", "{}")],
            hold_open: true,
            seen_commands: seen.clone(),
            ..Default::default()
        })
        .await;
        let pty = PtyStdin::install();

        let args = CliArgs {
            grpc_addr: addr,
            offline: true,
            ..Default::default()
        };
        let driver = async {
            // Wait for startup to finish (the last startup RPC is
            // new_session), then deliver ctrl+c through the terminal input.
            for _ in 0..1200 {
                if seen.lock().unwrap().iter().any(|t| t == "new_session") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            // Grace for the welcome render.
            tokio::time::sleep(Duration::from_millis(300)).await;
            pty.write_input("\x03");
        };
        let (code, ()) = tokio::join!(run_interactive(&args), driver);

        drop(pty);
        restore_env("HOME", old_home);
        assert_eq!(code, 0);
    }

    /// Shared setup for the PTY interactive scenarios: mock agent + scratch
    /// HOME + PTY stdin. Returns the pieces the drivers need.
    #[cfg(unix)]
    struct InteractiveFixture {
        addr: String,
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        /// Kept for drop (cleanup); never read.
        _home: tempfile::TempDir,
        old_home: Option<std::ffi::OsString>,
        pty: PtyStdin,
        unary_delay_ms: u64,
    }

    #[cfg(unix)]
    impl InteractiveFixture {
        async fn new() -> Self {
            Self::with_delay(0).await
        }

        async fn with_delay(unary_delay_ms: u64) -> Self {
            let home = tempfile::tempdir().unwrap();
            let old_home = std::env::var_os("HOME");
            std::env::set_var("HOME", home.path());
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let addr = spawn_mock(MockAgent {
                state_data: "{\"sessionId\":\"s1\"}".into(),
                events: vec![stream_event("ping", "{}")],
                hold_open: true,
                seen_commands: seen.clone(),
                unary_delay_ms,
                ..Default::default()
            })
            .await;
            let pty = PtyStdin::install();
            Self {
                addr,
                seen,
                _home: home,
                old_home,
                pty,
                unary_delay_ms,
            }
        }

        fn args(&self) -> CliArgs {
            CliArgs {
                grpc_addr: self.addr.clone(),
                offline: true,
                ..Default::default()
            }
        }

        /// Wait until the mock sees `what` (bounded), then a grace beat.
        async fn await_command(&self, what: &str) {
            for _ in 0..400 {
                if self.seen.lock().unwrap().iter().any(|t| t == what) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        /// Wait until startup has fully completed: the last startup RPCs
        /// are `new_session` followed by apply_tui_defaults' `get_state`
        /// (the second one). Commands are recorded when the mock RECEIVES
        /// them, so the grace must outlast the unary delay.
        async fn await_startup_complete(&self) {
            for _ in 0..400 {
                let done = {
                    let seen = self.seen.lock().unwrap();
                    let states = seen.iter().filter(|t| *t == "get_state").count();
                    seen.iter().any(|t| t == "new_session") && states >= 2
                };
                if done {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            tokio::time::sleep(Duration::from_millis(self.unary_delay_ms + 400)).await;
        }

        fn finish(self) {
            let Self { old_home, pty, .. } = self;
            drop(pty);
            restore_env("HOME", old_home);
        }
    }

    /// A real SIGINT quits via the exit-signal callback + ExitSignal arm.
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env-var serialization across awaits
    async fn interactive_sigint_quits_via_exit_signal() {
        let _guard = crate::test_env::lock();
        let fx = InteractiveFixture::new().await;
        let driver = async {
            fx.await_startup_complete().await;
            unsafe { libc::raise(libc::SIGINT) };
        };
        let args = fx.args();
        let (code, ()) = tokio::join!(run_interactive(&args), driver);
        fx.finish();
        assert_eq!(code, 0);
    }

    /// ctrl+c DURING startup interrupts it (exit 1 like the TS app). The
    /// mock answers slowly so startup is guaranteed to be in flight.
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env-var serialization across awaits
    async fn interactive_ctrl_c_during_startup() {
        let _guard = crate::test_env::lock();
        let fx = InteractiveFixture::with_delay(500).await;
        let driver = async {
            // The byte sits in the PTY buffer until the reader starts.
            tokio::time::sleep(Duration::from_millis(50)).await;
            fx.pty.write_input("\x03");
        };
        let args = fx.args();
        let (code, ()) = tokio::join!(run_interactive(&args), driver);
        fx.finish();
        assert_eq!(code, 1);
    }

    /// SIGINT during startup drives the ExitSignal startup arm.
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env-var serialization across awaits
    async fn interactive_sigint_during_startup() {
        let _guard = crate::test_env::lock();
        let fx = InteractiveFixture::with_delay(500).await;
        let driver = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            unsafe { libc::raise(libc::SIGINT) };
        };
        let args = fx.args();
        let (code, ()) = tokio::join!(run_interactive(&args), driver);
        fx.finish();
        assert_eq!(code, 1);
    }

    /// SIGWINCH during startup AND in the main loop (both Resize arms).
    /// The mock is slow so startup is still in flight for the first raise.
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env-var serialization across awaits
    async fn interactive_resize_during_and_after_startup() {
        let _guard = crate::test_env::lock();
        let fx = InteractiveFixture::with_delay(400).await;
        let driver = async {
            // Resize during startup.
            tokio::time::sleep(Duration::from_millis(50)).await;
            unsafe { libc::raise(libc::SIGWINCH) };
            // Resize in the main loop, then quit.
            fx.await_startup_complete().await;
            unsafe { libc::raise(libc::SIGWINCH) };
            tokio::time::sleep(Duration::from_millis(100)).await;
            fx.pty.write_input("\x03");
        };
        let args = fx.args();
        let (code, ()) = tokio::join!(run_interactive(&args), driver);
        fx.finish();
        assert_eq!(code, 0);
    }

    /// CLI messages become the initial prompt (sent after startup).
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env-var serialization across awaits
    async fn interactive_with_initial_prompt_message() {
        let _guard = crate::test_env::lock();
        let fx = InteractiveFixture::new().await;
        let driver = async {
            // The initial prompt goes out as a `prompt` command ~100 ms
            // after startup.
            fx.await_command("prompt").await;
            fx.pty.write_input("\x03");
        };
        let mut args = fx.args();
        args.messages = vec!["hello agent".to_string()];
        let (code, ()) = tokio::join!(run_interactive(&args), driver);
        fx.finish();
        assert_eq!(code, 0);
    }
}
