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
use crate::generated::proto::future_agent_client::FutureAgentClient;
use crate::generated::proto::{RpcCommand, RpcResponse, StreamRequest};
use crate::rpc::grpc_client::GrpcClient;
use crate::version::VERSION;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tonic::transport::Endpoint;

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
        grpc_addr: "localhost:50051".to_string(),
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
    let endpoint = Endpoint::from_shared(format!("http://{addr}"))
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(timeout_secs));
    let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
    let mut client = FutureAgentClient::new(channel);
    client
        .execute_command(cmd)
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
    let cfg = |id: &str, mut cmd: RpcCommand| {
        cmd.id = id.to_string();
        cmd.session_id = session_id.to_string();
        cmd
    };

    if let Some(model) = &args.model {
        let cmd = cfg(
            "cfg1",
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
        let cmd = cfg("cfg5", RpcCommand::default());
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
        let cmd = cfg("cfg7", RpcCommand::default());
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
    Ok(())
}

// ─── Print Mode (Non-Interactive) ──────────────────────────────────────────

/// `runPrintMode` — connect, apply CLI options, stream events, prompt, output.
async fn run_print_mode(grpc_addr: &str, args: &CliArgs) -> Result<(), String> {
    let prompt = build_initial_prompt(&args.file_args, &args.messages)
        .ok_or_else(|| "No prompt provided".to_string())?;

    // Get initial state to get the session ID.
    let state_cmd = RpcCommand {
        id: now_id(),
        r#type: "get_state".to_string(),
        session_id: String::new(),
        ..Default::default()
    };
    let resp = execute_unary(grpc_addr, state_cmd, GRPC_DEADLINE_SEC).await?;
    if !resp.success {
        let err = if resp.error.is_empty() {
            "get_state failed".to_string()
        } else {
            resp.error.clone()
        };
        return Err(err);
    }
    let state: serde_json::Value = serde_json::from_str(&resp.data).map_err(|e| e.to_string())?;
    let session_id = state
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let is_json_mode = args.mode.as_deref() == Some("json");

    // Apply CLI options.
    apply_cli_options(grpc_addr, &session_id, args).await?;

    // Subscribe to events BEFORE sending the prompt.
    let endpoint =
        Endpoint::from_shared(format!("http://{grpc_addr}")).map_err(|e| e.to_string())?;
    let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
    let mut client = FutureAgentClient::new(channel);
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

    // Wait for the event stream to complete.
    let (stream_result, json_messages, text) = events_task
        .await
        .unwrap_or_else(|_| (Ok(()), Vec::new(), String::new()));
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
    let client = Arc::new(client);

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
    if !app.is_running() {
        app.stop();
        return 0;
    }

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

    // Build the runtime (print mode + interactive both need one).
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!("future-tui: failed to start runtime: {err}");
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
        assert_eq!(a.grpc_addr, "localhost:50051");
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
}
