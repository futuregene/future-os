//! `future run` — port of cli/src/commands/run.ts (P2: full command body).
//!
//! Non-interactive agent execution over gRPC. Arg parsing, help text, prompt
//! building, and error output are byte-identical to the TS original; the
//! actual orchestration (session resolution → config → stream → prompt) runs
//! in `rpc.rs::RunClient::run`.

use crate::output::Output;
use crate::rpc::RunConfig;
use std::path::Path;

const VALID_THINKING_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh"];
const VALID_PERMISSION_LEVELS: &[&str] = &["all", "workspace", "none"];

// ─── CLI Types ────────────────────────────────────────────────────────

struct RunArgs {
    grpc_addr: String,
    fork: Option<String>,
    session: Option<String>,
    continue_last: bool,
    model: Option<String>,
    thinking: Option<String>,
    tools: Option<Vec<String>>,
    no_tools: bool,
    no_builtin_tools: bool,
    system_prompt: Option<String>,
    append_system_prompt: Option<Vec<String>>,
    permission: Option<String>,
    no_session: bool,
    mode: String,
    cwd: Option<String>,
    verbose: bool,
    file_args: Vec<String>,
    messages: Vec<String>,
}

// ─── Help Text ─────────────────────────────────────────────────────────

fn print_run_help(out: &Output) {
    out.log(
        r#"future run — send a prompt to the Future Agent (one-shot, non-interactive)

Connects to the agent gRPC server, configures the session, sends a prompt,
streams the response to stdout, and exits.

Usage:
  future run [options] [@files...] [message...]

Session options:
  --session <id>           Connect to an existing session by ID
  --continue, -c           Continue the most recent session (by updated_at)
  --fork <entry-id>        Fork a new session from a specific entry in the current session
  --no-session             Ephemeral mode: do not persist the session to disk

Model & behavior:
  --model <id>             Model ID. Only affects this run; subsequent runs use the default.
                           Supports model:thinking shorthand, e.g. "sonnet:high".
  --thinking <level>       Thinking/reasoning level: off, minimal, low, medium, high, xhigh
  --permission <level>     File access permission: all (no restrictions), workspace
                           (workspace + temp only), none (read-only outside workspace)

Tool control:
  --tools, -t <names>      Comma-separated tool names to enable (e.g. "read,shell")
  --no-tools, -nt          Disable all tools
  --no-builtin-tools, -nbt Disable built-in tools only (keep MCP extensions active)

Prompt control:
  --system-prompt <text>   Replace the system prompt
  --append-system-prompt <text>  Append to the system prompt (can repeat)

Output:
  --mode <mode>            text (default): stream to stdout; json: one JSON object on exit
  --verbose                Write progress and tool calls to stderr

Other:
  --grpc-addr <addr>       gRPC server address (default 127.0.0.1:50051).
                           Override with env FUTURE_AGENT_GRPC_ADDR.
  --cwd <dir>              Working directory for the agent (default: current directory)
  --help, -h               Show this help

Arguments:
  @files...    Read each file's content and wrap it in <file name="<abs-path>"> tags
               before the message text. Files are read before the prompt is sent.
  message...   The text prompt. Joined with spaces. If empty, @files must provide content.

Examples:
  future run "Explain this codebase"
  future run --model sonnet:high "Review the changes"
  future run --fork abc123 "Continue from this fork point"
  future run --continue "Pick up where we left off"
  future run --tools read,shell "Read the README and list files"
  future run --permission workspace "Summarize AGENTS.md"
  future run --mode json "What is 2+2?"
  future run @README.md @src/main.rs "Summarize these files""#,
    );
}

// ── Arg Parser ────────────────────────────────────────────────────────

fn parse_run_args(args: &[String], out: &Output) -> Option<RunArgs> {
    let default_addr =
        std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string());

    let mut result = RunArgs {
        grpc_addr: default_addr,
        fork: None,
        session: None,
        continue_last: false,
        model: None,
        thinking: None,
        tools: None,
        no_tools: false,
        no_builtin_tools: false,
        system_prompt: None,
        append_system_prompt: None,
        permission: None,
        no_session: false,
        mode: "text".to_string(),
        cwd: None,
        verbose: false,
        file_args: Vec::new(),
        messages: Vec::new(),
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
            "--fork" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.fork = Some(args[i].clone());
                }
            }
            "--session" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.session = Some(args[i].clone());
                }
            }
            "--continue" | "-c" => result.continue_last = true,
            "--model" => {
                if i + 1 < args.len() {
                    i += 1;
                    let model_arg = args[i].as_str();
                    if let Some(colon_index) = model_arg.rfind(':') {
                        let potential_thinking = &model_arg[colon_index + 1..];
                        if VALID_THINKING_LEVELS.contains(&potential_thinking) {
                            result.model = Some(model_arg[..colon_index].to_string());
                            result.thinking = Some(potential_thinking.to_string());
                        } else {
                            result.model = Some(model_arg.to_string());
                        }
                    } else {
                        result.model = Some(model_arg.to_string());
                    }
                }
            }
            "--thinking" => {
                if i + 1 < args.len() {
                    i += 1;
                    let level = args[i].as_str();
                    if !VALID_THINKING_LEVELS.contains(&level) {
                        out.log_err(&format!(
                            "Invalid thinking level: {level}. Valid: {}",
                            VALID_THINKING_LEVELS.join(", ")
                        ));
                        return None;
                    }
                    result.thinking = Some(level.to_string());
                }
            }
            "--tools" | "-t" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.tools = Some(args[i].split(',').map(|s| s.trim().to_string()).collect());
                }
            }
            "--no-tools" | "-nt" => result.no_tools = true,
            "--no-builtin-tools" | "-nbt" => result.no_builtin_tools = true,
            "--system-prompt" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.system_prompt = Some(args[i].clone());
                }
            }
            "--append-system-prompt" => {
                result.append_system_prompt.get_or_insert_with(Vec::new);
                if i + 1 < args.len() {
                    i += 1;
                    result
                        .append_system_prompt
                        .as_mut()
                        .unwrap()
                        .push(args[i].clone());
                }
            }
            "--permission" => {
                if i + 1 < args.len() {
                    i += 1;
                    let level = args[i].as_str();
                    if !VALID_PERMISSION_LEVELS.contains(&level) {
                        out.log_err(&format!(
                            "Invalid permission level: {level}. Valid: {}",
                            VALID_PERMISSION_LEVELS.join(", ")
                        ));
                        return None;
                    }
                    result.permission = Some(level.to_string());
                }
            }
            "--no-session" => result.no_session = true,
            "--mode" => {
                if i + 1 < args.len() {
                    i += 1;
                    let mode = args[i].as_str();
                    if mode != "text" && mode != "json" {
                        out.log_err(&format!("Invalid mode: {mode}. Valid: text, json"));
                        return None;
                    }
                    result.mode = mode.to_string();
                }
            }
            "--cwd" => {
                if i + 1 < args.len() {
                    i += 1;
                    result.cwd = Some(args[i].clone());
                }
            }
            "--verbose" => result.verbose = true,
            "--help" | "-h" => {
                print_run_help(out);
                return None;
            }
            _ => {
                if let Some(rest) = arg.strip_prefix('@') {
                    result.file_args.push(rest.to_string());
                } else if arg.starts_with('-') {
                    out.log_err(&format!("Unknown option: {arg}"));
                    return None;
                } else {
                    result.messages.push(arg.to_string());
                }
            }
        }
        i += 1;
    }

    Some(result)
}

// ─── Prompt Builder ────────────────────────────────────────────────────

async fn build_prompt(file_args: &[String], messages: &[String], out: &Output) -> Option<String> {
    if file_args.is_empty() && messages.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for file_path in file_args {
        let abs_path = absolute_path(file_path);
        match tokio::fs::read_to_string(&abs_path).await {
            Ok(content) => {
                parts.push(format!("<file name=\"{abs_path}\">\n{content}\n</file>"));
            }
            Err(_) => {
                out.log_err(&format!("Failed to read file: {file_path}"));
                return None;
            }
        }
    }
    parts.extend(messages.iter().cloned());
    Some(parts.join("\n"))
}

/// `path.resolve(filePath)` — absolute, normalized, platform-native.
fn absolute_path(file_path: &str) -> String {
    let path = Path::new(file_path);
    if path.is_absolute() {
        normalize_abs(path.to_path_buf())
    } else {
        match std::env::current_dir() {
            Ok(cwd) => normalize_abs(cwd.join(path)),
            Err(_) => path.display().to_string(),
        }
    }
}

fn normalize_abs(path: std::path::PathBuf) -> String {
    // Node's path.resolve normalizes `.`/`..` segments without resolving
    // symlinks; std canonicalize would resolve symlinks, so do a lexical
    // normalization instead.
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.display().to_string()
}

// ── Main Command ──────────────────────────────────────────────────────

/// `run(args)` — args are everything after `future run`.
pub async fn run_command(args: &[String], out: &Output) -> Result<(), String> {
    let parsed = parse_run_args(args, out);

    // null means help was printed or parse error
    let Some(parsed) = parsed else {
        return Ok(());
    };

    // Build prompt
    let Some(prompt) = build_prompt(&parsed.file_args, &parsed.messages, out).await else {
        out.log_err("No prompt provided. Usage: future run [options] [@files...] [message...]");
        return Err(crate::HANDLED_EXIT.to_string());
    };

    // Build RunConfig
    let run_config = RunConfig {
        fork: parsed.fork,
        session: parsed.session,
        continue_last: parsed.continue_last,
        model: parsed.model,
        thinking: parsed.thinking,
        tools: parsed.tools,
        no_tools: parsed.no_tools,
        no_builtin_tools: parsed.no_builtin_tools,
        system_prompt: parsed.system_prompt,
        append_system_prompt: parsed.append_system_prompt.map(|lines| lines.join("\n")),
        permission: parsed.permission,
        no_session: parsed.no_session,
        mode: parsed.mode,
        cwd: parsed.cwd.unwrap_or_else(|| {
            std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default()
        }),
        verbose: parsed.verbose,
        message: prompt,
    };

    // Execute
    let client = crate::rpc::RunClient::new(&parsed.grpc_addr);
    let result = client.run(&run_config, out).await;
    match result {
        Ok(_) => Ok(()),
        Err(msg) => {
            if run_config.mode == "json" {
                out.log(
                    &serde_json::to_string(&serde_json::json!({ "error": msg }))
                        .unwrap_or_default(),
                );
            } else {
                out.log_err(&format!("Error: {msg}"));
            }
            Err(crate::HANDLED_EXIT.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<RunArgs> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (out, _cap) = Output::memory();
        parse_run_args(&args, &out)
    }

    #[tokio::test]
    async fn parse_basic_message() {
        // The default grpc addr reads FUTURE_AGENT_GRPC_ADDR (global env) —
        // hold the shared env lock so doctor/session tests cannot race us.
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_AGENT_GRPC_ADDR"]);
        let parsed = parse(&["hello", "world"]).unwrap();
        assert_eq!(parsed.messages, vec!["hello", "world"]);
        assert_eq!(parsed.mode, "text");
        assert_eq!(parsed.grpc_addr, "127.0.0.1:50051");
    }

    #[test]
    fn parse_model_thinking_shorthand() {
        let parsed = parse(&["--model", "sonnet:high", "hi"]).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("sonnet"));
        assert_eq!(parsed.thinking.as_deref(), Some("high"));

        // unknown suffix → whole string is the model id
        let parsed = parse(&["--model", "sonnet:notathing", "hi"]).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("sonnet:notathing"));
        assert!(parsed.thinking.is_none());
    }

    #[test]
    fn parse_invalid_thinking_level() {
        // invalid → prints error + None (exit 0, matches TS quirk)
        assert!(parse(&["--thinking", "bogus", "hi"]).is_none());
    }

    #[test]
    fn parse_invalid_mode() {
        assert!(parse(&["--mode", "xml", "hi"]).is_none());
    }

    #[test]
    fn parse_invalid_permission() {
        assert!(parse(&["--permission", "everywhere", "hi"]).is_none());
    }

    #[test]
    fn parse_unknown_option() {
        assert!(parse(&["--frobnicate", "hi"]).is_none());
    }

    #[test]
    fn parse_tools_and_flags() {
        let parsed = parse(&[
            "--tools",
            "read, shell",
            "-nt",
            "--no-session",
            "--verbose",
            "go",
        ])
        .unwrap();
        assert_eq!(
            parsed.tools,
            Some(vec!["read".to_string(), "shell".to_string()])
        );
        assert!(parsed.no_tools);
        assert!(parsed.no_session);
        assert!(parsed.verbose);
    }

    #[test]
    fn parse_file_args_and_messages() {
        let parsed = parse(&["@README.md", "@src/main.rs", "summarize"]).unwrap();
        assert_eq!(parsed.file_args, vec!["README.md", "src/main.rs"]);
        assert_eq!(parsed.messages, vec!["summarize"]);
    }

    #[test]
    fn parse_append_system_prompt_repeats() {
        let parsed = parse(&[
            "--append-system-prompt",
            "a",
            "--append-system-prompt",
            "b",
            "hi",
        ])
        .unwrap();
        assert_eq!(
            parsed.append_system_prompt,
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn absolute_path_normalization() {
        assert_eq!(absolute_path("/a/b/../c"), "/a/c");
        assert_eq!(absolute_path("/a/./b"), "/a/b");
        assert_eq!(absolute_path("/a//b"), "/a/b");
    }

    #[test]
    fn parse_value_flags() {
        let parsed = parse(&[
            "--grpc-addr", "10.0.0.1:9999",
            "--fork", "entry-1",
            "--session", "sess-1",
            "--system-prompt", "sys",
            "--cwd", "/work",
            "-nbt",
            "--mode", "json",
            "-c",
            "hi",
        ])
        .unwrap();
        assert_eq!(parsed.grpc_addr, "10.0.0.1:9999");
        assert_eq!(parsed.fork.as_deref(), Some("entry-1"));
        assert_eq!(parsed.session.as_deref(), Some("sess-1"));
        assert_eq!(parsed.system_prompt.as_deref(), Some("sys"));
        assert_eq!(parsed.cwd.as_deref(), Some("/work"));
        assert!(parsed.no_builtin_tools);
        assert!(parsed.continue_last);
        assert_eq!(parsed.mode, "json");
        assert_eq!(parsed.messages, vec!["hi"]);
    }

    #[test]
    fn parse_trailing_flags_without_values_are_ignored() {
        // A flag as the LAST arg has no value → option stays unset (JS: undefined).
        let parsed = parse(&["--model", "m1", "hi", "--fork"]).unwrap();
        assert!(parsed.fork.is_none());
        assert_eq!(parsed.model.as_deref(), Some("m1"));
        let parsed = parse(&["hi", "--session"]).unwrap();
        assert!(parsed.session.is_none());
        let parsed = parse(&["hi", "--system-prompt"]).unwrap();
        assert!(parsed.system_prompt.is_none());
        let parsed = parse(&["hi", "--cwd"]).unwrap();
        assert!(parsed.cwd.is_none());
        let parsed = parse(&["hi", "--grpc-addr"]).unwrap();
        assert!(parsed.grpc_addr.ends_with("50051") || parsed.grpc_addr.contains(':'));
        let parsed = parse(&["hi", "--tools"]).unwrap();
        assert!(parsed.tools.is_none());
        let parsed = parse(&["hi", "--mode"]).unwrap();
        assert_eq!(parsed.mode, "text");
        // --append-system-prompt with no value: vec created, nothing pushed.
        let parsed = parse(&["hi", "--append-system-prompt"]).unwrap();
        assert_eq!(parsed.append_system_prompt, Some(vec![]));
        // A bare "-" is an unknown OPTION (starts with '-'), not a message.
        assert!(parse(&["-"]).is_none());
    }

    #[tokio::test]
    async fn run_command_help_and_no_prompt() {
        let (out, cap) = Output::memory();
        run_command(&["--help".to_string()], &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("future run — send a prompt"), "stdout: {stdout}");
        assert!(stdout.contains("--fork <entry-id>"), "stdout: {stdout}");

        // No message and no @files → usage error.
        let (out, cap) = Output::memory();
        let result = run_command(&[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("No prompt provided."), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn run_command_file_args_and_read_failure() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        tokio::fs::write(&file, "FILE-CONTENTS").await.unwrap();

        // @file content wraps in <file> tags; unreadable file → error.
        let (out, cap) = Output::memory();
        let result = run_command(
            &["@/no/such/file.txt".to_string(), "hi".to_string()],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Failed to read file: /no/such/file.txt"), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn run_command_end_to_end_against_mock() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        tokio::fs::write(&file, "FILE-CONTENTS").await.unwrap();
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .responses
            .insert("new_session".into(), "{\"sessionId\":\"s-run\"}".into());
        agent.events = vec![
            crate::test_server::stream_event("text_chunk", "{\"text\":\"hi there\"}"),
            crate::test_server::stream_event("agent_end", "{}"),
        ];
        let addr = crate::test_server::spawn_mock(agent.clone()).await;

        // Success: json mode + @file prompt assembly + append-system-prompt join.
        let (out, cap) = Output::memory();
        run_command(
            &[
                "--grpc-addr".to_string(),
                addr.clone(),
                "--mode".to_string(),
                "json".to_string(),
                "--append-system-prompt".to_string(),
                "line1".to_string(),
                "--append-system-prompt".to_string(),
                "line2".to_string(),
                "--cwd".to_string(),
                "/tmp".to_string(),
                format!("@{}", file.display()),
                "summarize".to_string(),
            ],
            &out,
        )
        .await
        .expect("run");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessionId"], "s-run");
        assert_eq!(parsed["text"], "hi there");
        // The prompt carried the wrapped file + message.
        let prompts = agent.seen_of("prompt");
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].message.contains("<file name=\""), "msg: {}", prompts[0].message);
        assert!(prompts[0].message.contains("FILE-CONTENTS"));
        assert!(prompts[0].message.ends_with("summarize"));
        // Repeated append-system-prompt joined with newline.
        let appends = agent.seen_of("append_system_prompt");
        assert_eq!(appends[0].system_prompt, "line1\nline2");
    }

    #[tokio::test]
    async fn run_command_error_output_modes() {
        let _guard = crate::test_env::lock_env().await;
        // Dead port → error printed per mode.
        let (out, cap) = Output::memory();
        let result = run_command(
            &["--grpc-addr".to_string(), "127.0.0.1:1".to_string(), "hi".to_string()],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.starts_with("Error: "), "stderr: {stderr}");

        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                "127.0.0.1:1".to_string(),
                "--mode".to_string(),
                "json".to_string(),
                "hi".to_string(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("{\"error\":\""), "stdout: {stdout}");
    }
}
