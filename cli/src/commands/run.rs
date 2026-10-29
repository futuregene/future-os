//! `future run` — port of cli/src/commands/run.ts (P2: full command body).
//!
//! Non-interactive agent execution over gRPC. Arg parsing, help text, prompt
//! building, and error output are byte-identical to the TS original; the
//! actual orchestration (session resolution → config → stream → prompt) runs
//! in `rpc.rs::RunClient::run`.

use crate::output::Output;
use crate::rpc::RunConfig;
use std::path::Path;

/// Level names `--thinking` / `--model id:level` accept.
///
/// `default` asks for the provider default (no thinking parameter);
/// `max` is a wire value models may declare. A model can still offer any other
/// string — the agent clamps a level it does not offer to `default`, and the
/// pickers send the declared values verbatim — these are just the names a user
/// can type on the command line.
pub(crate) const VALID_THINKING_LEVELS: &[&str] = &[
    "default", "off", "minimal", "low", "medium", "high", "xhigh", "max",
];
pub(crate) const VALID_PERMISSION_LEVELS: &[&str] = &["all", "workspace", "none"];
/// Product approval modes; auto maps to tier sandbox plus reviewer model. The OS probe is the agent's job, not ours — a tier
/// this list allows can still be refused at apply time.
pub(crate) const VALID_SANDBOX_TIERS: &[&str] = &["off", "manual", "sandbox", "auto"];

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
    auto_title: bool,
    busy_policy: String,
    no_wait: bool,
    file_args: Vec<String>,
    messages: Vec<String>,
    /// A file whose contents ARE the message, verbatim (no `<file>` wrapper).
    /// Programmatic dispatch uses this; `@<file>` stays the human shape.
    message_file: Option<String>,
}

// ─── Help Text ─────────────────────────────────────────────────────────

fn print_run_help(out: &Output) {
    out.log(
        r#"future run — send a prompt to the Future Agent (one-shot, non-interactive)

Connects to the agent gRPC server, configures the session, sends a prompt,
streams the response to stdout, and exits.

Usage:
  future run [options] [@files...] [message...]
  future run '/fusion <task>'   Enable a thinking-only Fusion tool for this run.
                                Requires a running FutureOS Host. The main Agent
                                decides when to consult, then continues the task.

Session options:
  --session <id>           Connect to an existing session by ID
  --continue, -c           Continue the most recent session (by updated_at)
  --fork <entry-id>        Fork a new session from a specific entry in the current session
  --steer                  Interrupt the session's in-progress run and ask
                           immediately (supersede_session). Default: append the
                           prompt behind any in-progress run (enqueue_if_busy).
  --no-wait                Submit the prompt, print an ack, and exit without
                           waiting for the answer. Useful for scripting and
                           fire-and-forget runs. Default without this flag is
                           to stream the response and exit when the run ends.
  --no-session             Ephemeral mode: do not persist the session to disk

Model & behavior:
  --model <id>             Model ID. Only affects this run; subsequent runs use the default.
                           Supports model:thinking shorthand, e.g. "sonnet:high" or
                           "sonnet:max".
  --thinking <level>       Thinking/reasoning level: default (send no thinking
                           parameter), off, minimal, low, medium, high, xhigh, max
  --permission <level>     Tool permission: all (unrestricted; fresh-session default),
                           workspace (approval-gated access), none (deny all tools).
                           Separate from the desktop OS sandbox policy.

Tool control:
  --tools, -t <names>      Comma-separated tool names to enable (e.g. "read,shell")
  --no-tools, -nt          Disable all tools
  --no-builtin-tools, -nbt Disable built-in tools only (keep MCP extensions active)

Prompt control:
  --system-prompt <text>   Replace this session's system prompt. Recorded with
                           the session, so it persists and applies to later runs
                           too (an empty string restores the built-in prompt).
  --append-system-prompt <text>
                           Append rules to the end of this session's system
                           prompt (can repeat). Recorded with the session, so it
                           persists and applies to later runs too (an empty
                           string clears it). Read either back with
                           `future session status <id>`.
                           On a run that creates a new session these are
                           one-shot in practice; with --session/--continue they
                           change that session from now on.
  --message-file <path>    Send the file's contents as the message, verbatim:
                           no <file> wrapper, no joining, nothing trimmed.
                           Mutually exclusive with @files and a message
                           argument. Use this for scripted or agent-to-agent
                           dispatch, where the other side must see a bare user
                           turn; use @<file> when a human attaches a file.

Output:
  --mode <mode>            text (default): stream to stdout; json: one JSON object on exit
  --verbose                Write progress and tool calls to stderr

Other:
  --no-title               Do not name a session this run creates. By default
                           the first answer of a NEW session is used to generate
                           and save a title (one extra model call). Sessions
                           that already existed (--session/--continue/--fork)
                           are never renamed.
  --grpc-addr <addr>       Explicit TCP address (default: auto, per-user local IPC).
                           Also configurable via FUTURE_AGENT_GRPC_ADDR.
                           Explicit TCP is tried first, then local IPC.
  --cwd <dir>              Working directory for the agent. Defaults to the
                           current directory for a session this run creates;
                           an existing session (--session/--continue) keeps its
                           recorded cwd unless --cwd is given.
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
  future run --steer --continue "Stop that, answer this instead"
  future run --session abc123 "Queue this behind the current run"
  future run --tools read,shell "Read the README and list files"
  future run --permission workspace "Summarize AGENTS.md"
  future run --message-file /tmp/prompt.md --session abc123
  future run --mode json "What is 2+2?"
  future run @README.md @src/main.rs "Summarize these files""#,
    );
}

// ── Arg Parser ────────────────────────────────────────────────────────

fn parse_run_args(args: &[String], out: &Output) -> Option<RunArgs> {
    let default_addr = std::env::var("FUTURE_AGENT_GRPC_ADDR")
        .unwrap_or_else(|_| future_rpc::transport::AUTO_ENDPOINT.to_string());

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
        auto_title: true,
        busy_policy: String::new(),
        no_wait: false,
        file_args: Vec::new(),
        messages: Vec::new(),
        message_file: None,
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
            "--no-title" => result.auto_title = false,
            "--steer" => {
                result.busy_policy = "supersede_session".to_string();
            }
            "--no-wait" => result.no_wait = true,
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
            "--message-file" => {
                if i + 1 >= args.len() {
                    out.log_err("--message-file requires a path");
                    return None;
                }
                i += 1;
                result.message_file = Some(args[i].clone());
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

    // `--message-file` IS the turn; a positional message or an `@file` would
    // say it a second time. Refuse rather than let one silently win (checked
    // in `run_command`, which can report a handled exit; a parse-level
    // rejection would exit 0).

    Some(result)
}

// ─── Prompt Builder ────────────────────────────────────────────────────

async fn build_prompt(
    file_args: &[String],
    messages: &[String],
    message_file: Option<&str>,
    out: &Output,
) -> Option<String> {
    // The two shapes are deliberately different: `@<file>` wraps the content
    // in a `<file name="…">` block (a human attaching a file), while
    // `--message-file` sends the bytes as the message itself so the other side
    // sees a bare user turn (programmatic dispatch).
    if let Some(path) = message_file {
        return read_message_file(path, out).await;
    }
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

/// Read `--message-file` as the message: bytes in, bytes out, nothing added.
///
/// Every rejection is explicit — an empty turn, a typo'd path and a binary
/// blob are different mistakes and each names itself. Nothing is trimmed or
/// truncated: the caller's bytes are what the other conversation receives.
async fn read_message_file(path: &str, out: &Output) -> Option<String> {
    let bytes = match tokio::fs::read(&absolute_path(path)).await {
        Ok(bytes) => bytes,
        Err(_) => {
            out.log_err(&format!("Failed to read message file: {path}"));
            return None;
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            out.log_err(&format!(
                "Failed to read message file: {path} (message file is not valid UTF-8)"
            ));
            return None;
        }
    };
    // NUL bytes are valid UTF-8, so a binary file passes the decode above.
    if text.contains('\0') {
        out.log_err(&format!(
            "Failed to read message file: {path} (message file is binary: contains NUL bytes)"
        ));
        return None;
    }
    if text.trim().is_empty() {
        out.log_err(&format!(
            "Failed to read message file: {path} (message file is empty)"
        ));
        return None;
    }
    Some(text)
}

/// `path.resolve(filePath)` — absolute, normalized, platform-native.
fn absolute_path(file_path: &str) -> String {
    let path = Path::new(file_path);
    if path.is_absolute() {
        return normalize_abs(path.to_path_buf());
    }
    // current_dir is practically infallible; on the off chance it fails the
    // bare path is still better than a panic (TS path.resolve never fails).
    let cwd = std::env::current_dir().unwrap_or_default();
    normalize_abs(cwd.join(path))
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

    // `--message-file` carries the whole turn, so a positional message or an
    // `@file` would state it twice. Checked here rather than in the parser:
    // this path reports a handled exit, while a parse-level rejection exits 0.
    if parsed.message_file.is_some()
        && (!parsed.messages.is_empty() || !parsed.file_args.is_empty())
    {
        out.log_err(
            "--message-file cannot be combined with a message argument or @files \
             (the file's contents are the whole message)",
        );
        return Err(crate::HANDLED_EXIT.to_string());
    }

    // Build prompt. An input was given but rejected (unreadable file, binary
    // payload, …) — that error is already reported, so do not add the
    // "no prompt" usage line on top of it.
    if parsed.message_file.is_none() && parsed.file_args.is_empty() && parsed.messages.is_empty() {
        out.log_err("No prompt provided. Usage: future run [options] [@files...] [message...]");
        return Err(crate::HANDLED_EXIT.to_string());
    }
    let Some(prompt) = build_prompt(
        &parsed.file_args,
        &parsed.messages,
        parsed.message_file.as_deref(),
        out,
    )
    .await
    else {
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
        no_wait: parsed.no_wait,
        // Passed through untouched: `None` (no --cwd) must stay distinguishable
        // from an explicit cwd, so attaching to an existing session leaves its
        // recorded working directory alone.
        cwd: parsed.cwd,
        verbose: parsed.verbose,
        auto_title: parsed.auto_title,
        busy_policy: parsed.busy_policy,
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
        assert_eq!(parsed.grpc_addr, "auto");
    }

    #[test]
    fn parse_model_thinking_shorthand() {
        let parsed = parse(&["--model", "sonnet:high", "hi"]).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("sonnet"));
        assert_eq!(parsed.thinking.as_deref(), Some("high"));

        // `default` (provider default) and `max` (a wire value) split too.
        let parsed = parse(&["--model", "sonnet:default", "hi"]).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("sonnet"));
        assert_eq!(parsed.thinking.as_deref(), Some("default"));
        let parsed = parse(&["--model", "sonnet:max", "hi"]).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("sonnet"));
        assert_eq!(parsed.thinking.as_deref(), Some("max"));

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
    fn parse_valid_thinking_and_permission_levels() {
        let parsed = parse(&["--thinking", "high", "hi"]).unwrap();
        assert_eq!(parsed.thinking.as_deref(), Some("high"));
        for level in ["default", "max"] {
            let parsed = parse(&["--thinking", level, "hi"]).unwrap();
            assert_eq!(parsed.thinking.as_deref(), Some(level));
        }
        let parsed = parse(&["--permission", "workspace", "hi"]).unwrap();
        assert_eq!(parsed.permission.as_deref(), Some("workspace"));
        // Trailing flags without values are ignored.
        assert!(parse(&["--model"]).is_some());
        assert!(parse(&["--thinking"]).is_some());
        assert!(parse(&["--permission"]).is_some());
    }

    #[test]
    fn normalize_abs_resolves_curdir_segments() {
        // Platform-spelled: the normalized path is joined with the host
        // separator (and a bare `/a` is not absolute on Windows).
        let root = if cfg!(windows) { r"C:\a" } else { "/a" };
        let sep = std::path::MAIN_SEPARATOR;
        let out = normalize_abs(std::path::PathBuf::from(format!(
            "{root}{sep}b{sep}..{sep}c"
        )));
        assert_eq!(out, format!("{root}{sep}c"));
        // A leading ./ produces an actual CurDir component.
        let out = normalize_abs(std::path::PathBuf::from("./rel/path"));
        assert_eq!(out, format!("rel{sep}path"));
    }

    #[test]
    fn parse_invalid_mode() {
        assert!(parse(&["--mode", "xml", "hi"]).is_none());
    }

    #[test]
    fn parse_steer_flag() {
        let parsed = parse(&["--steer", "hi"]).unwrap();
        assert_eq!(parsed.busy_policy, "supersede_session");
        // No flag → empty (the default follow-up policy at the RPC layer).
        assert_eq!(parse(&["hi"]).unwrap().busy_policy, "");
    }

    #[test]
    fn parse_no_wait_flag() {
        let parsed = parse(&["--no-wait", "hi"]).unwrap();
        assert!(parsed.no_wait);
        // No flag → false (the default wait-for-completion path).
        assert!(!parse(&["hi"]).unwrap().no_wait);
    }

    #[test]
    fn parse_no_title_flag() {
        // Titling a session this run creates is on by default (matching the
        // desktop app); `--no-title` is the opt-out.
        assert!(parse(&["hi"]).unwrap().auto_title);
        assert!(!parse(&["--no-title", "hi"]).unwrap().auto_title);
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

    /// `--message-file` takes the turn on its own: it is not a message
    /// argument and not an `@file`, so it lands in neither list.
    #[test]
    fn parse_message_file_flag() {
        let parsed = parse(&["--message-file", "/tmp/prompt.md"]).unwrap();
        assert_eq!(parsed.message_file.as_deref(), Some("/tmp/prompt.md"));
        assert!(parsed.messages.is_empty());
        assert!(parsed.file_args.is_empty());

        // Without a path it is a usage error, not a message-less run.
        let (out, cap) = Output::memory();
        let args: Vec<String> = ["--message-file"].iter().map(|s| s.to_string()).collect();
        assert!(parse_run_args(&args, &out).is_none());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("--message-file requires a path"),
            "{stderr}"
        );
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
        // Platform-spelled: normalization joins with the host separator.
        let root = if cfg!(windows) { r"C:\a" } else { "/a" };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            absolute_path(&format!("{root}{sep}b{sep}..{sep}c")),
            format!("{root}{sep}c")
        );
        assert_eq!(
            absolute_path(&format!("{root}{sep}.{sep}b")),
            format!("{root}{sep}b")
        );
        assert_eq!(
            absolute_path(&format!("{root}{sep}{sep}b")),
            format!("{root}{sep}b")
        );
    }

    #[test]
    fn absolute_path_relative_input_joins_cwd() {
        // Relative input is resolved against the process CWD (TS
        // path.resolve semantics).
        let cwd = std::env::current_dir().unwrap();
        let expected = normalize_abs(cwd.join("rel/file.txt"));
        assert_eq!(absolute_path("rel/file.txt"), expected);
        assert_eq!(absolute_path("./rel"), normalize_abs(cwd.join("rel")));
    }

    #[test]
    fn parse_value_flags() {
        let parsed = parse(&[
            "--grpc-addr",
            "10.0.0.1:9999",
            "--fork",
            "entry-1",
            "--session",
            "sess-1",
            "--system-prompt",
            "sys",
            "--cwd",
            "/work",
            "-nbt",
            "--mode",
            "json",
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

    #[tokio::test]
    async fn parse_trailing_flags_without_values_are_ignored() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_AGENT_GRPC_ADDR"]);
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
        assert_eq!(parsed.grpc_addr, "auto");
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
        assert!(
            stdout.contains("future run — send a prompt"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("--fork <entry-id>"), "stdout: {stdout}");
        assert!(
            stdout.contains("auto, per-user local IPC"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("none (deny all tools)"), "stdout: {stdout}");
        assert!(!stdout.contains("default 127.0.0.1:50051"));
        assert!(!stdout.contains("read-only outside workspace"));
        // The opt-out and its cost (one extra model call) are documented in
        // the help the user actually reads.
        assert!(stdout.contains("--no-title"), "stdout: {stdout}");
        assert!(stdout.contains("one extra model call"), "stdout: {stdout}");
        assert!(
            stdout.contains("never renamed"),
            "help must say existing sessions are left alone: {stdout}"
        );
        // --no-wait is documented and points at the wait/default distinction.
        assert!(stdout.contains("--no-wait"), "stdout: {stdout}");
        // The prompt flags record with the session, which is why a one-shot run
        // can change later runs too — the help has to say so, or the effect is
        // invisible to the caller.
        assert!(
            stdout.contains("Recorded with\n                           the session"),
            "the run help must state that --system-prompt persists: {stdout}"
        );
        assert!(
            stdout.contains("future session status <id>"),
            "the run help must point at the read-back: {stdout}"
        );
        // The two file forms look alike and are not: `--message-file` sends the
        // bytes as the message, `@<file>` wraps them. The help has to say which
        // is which, or the difference is only discoverable by experiment.
        assert!(stdout.contains("--message-file <path>"), "stdout: {stdout}");
        assert!(
            stdout.contains("no <file> wrapper"),
            "the help must contrast --message-file with @<file>: {stdout}"
        );
        assert!(
            stdout.contains("Mutually exclusive with @files"),
            "the help must state the exclusion: {stdout}"
        );

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
        let result = run_command(&["@/no/such/file.txt".to_string(), "hi".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Failed to read file: /no/such/file.txt"),
            "stderr: {stderr}"
        );
    }

    // ── --message-file ────────────────────────────────────────────────
    //
    // The flag exists for PROGRAMMATIC dispatch (agent → agent, model
    // fan-out, goal workers): the file IS the message, so the child's first
    // user turn is the caller's text and nothing else. `@<file>` keeps its
    // own shape — a `<file name="…">` wrapper — for a human attaching a file
    // in a terminal.

    /// A mock agent that answers one run, for `--message-file` cases.
    async fn message_file_mock() -> (crate::test_server::MockAgent, String) {
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert(
            "new_session".into(),
            "{\"sessionId\":\"s-msg-file\"}".into(),
        );
        agent.events = vec![
            crate::test_server::stream_event("text_chunk", "{\"text\":\"ok\"}"),
            crate::test_server::stream_event("agent_end", "{}"),
        ];
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        (agent, addr)
    }

    /// The message is the file's bytes, verbatim: no `<file>` wrapper, no
    /// joining, no trimming, no re-encoding of non-ASCII content.
    #[tokio::test]
    async fn run_command_message_file_sends_verbatim_message() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("prompt.md");
        // CJK + Latin + emoji + a trailing newline the caller wrote on purpose.
        let sample = "第一行：中英混排 mixed text 😀🎯\n\n第二轮 · 结论\n结尾无空行\n";
        tokio::fs::write(&file, sample).await.unwrap();
        let (agent, addr) = message_file_mock().await;

        let (out, cap) = Output::memory();
        run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--message-file".to_string(),
                file.display().to_string(),
            ],
            &out,
        )
        .await
        .expect("run --message-file");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.is_empty(), "stderr: {stderr}");

        let prompts = agent.seen_of("prompt");
        assert_eq!(prompts.len(), 1);
        assert_eq!(
            prompts[0].message.as_bytes(),
            sample.as_bytes(),
            "message must be the file's bytes: {:?}",
            prompts[0].message
        );
        assert!(
            !prompts[0].message.contains("<file"),
            "@file's wrapper must not appear: {:?}",
            prompts[0].message
        );
        assert!(
            !prompts[0].message.contains(file.to_str().unwrap()),
            "the path must not be part of the message: {:?}",
            prompts[0].message
        );
    }

    /// `--message-file` and a positional message are two ways to say what the
    /// turn is; accepting both would silently pick one.
    #[tokio::test]
    async fn run_command_message_file_rejects_positional_message() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("prompt.md");
        tokio::fs::write(&file, "hello").await.unwrap();
        let (agent, addr) = message_file_mock().await;

        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--message-file".to_string(),
                file.display().to_string(),
                "extra message".to_string(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("--message-file cannot be combined"),
            "stderr: {stderr}"
        );
        assert!(agent.seen_of("prompt").is_empty(), "nothing may be sent");
    }

    /// Same for `@files`: the wrapper plus a verbatim message has no defined
    /// meaning, so it is a usage error rather than a guess.
    #[tokio::test]
    async fn run_command_message_file_rejects_at_files() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("prompt.md");
        tokio::fs::write(&file, "hello").await.unwrap();
        let attached = dir.path().join("note.txt");
        tokio::fs::write(&attached, "FILE-CONTENTS").await.unwrap();
        let (agent, addr) = message_file_mock().await;

        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--message-file".to_string(),
                file.display().to_string(),
                format!("@{}", attached.display()),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("--message-file cannot be combined"),
            "stderr: {stderr}"
        );
        assert!(agent.seen_of("prompt").is_empty(), "nothing may be sent");
    }

    /// An empty (or whitespace-only) file is a mistake, not an empty turn:
    /// say so instead of sending nothing.
    #[tokio::test]
    async fn run_command_message_file_rejects_empty_file() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("empty.md");
        tokio::fs::write(&file, "  \n\n\t").await.unwrap();
        let (agent, addr) = message_file_mock().await;

        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--message-file".to_string(),
                file.display().to_string(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("message file is empty"), "stderr: {stderr}");
        assert!(agent.seen_of("prompt").is_empty(), "nothing may be sent");
    }

    /// A missing file names the path and sends nothing.
    #[tokio::test]
    async fn run_command_message_file_rejects_missing_file() {
        let _guard = crate::test_env::lock_env().await;
        let (agent, addr) = message_file_mock().await;

        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--message-file".to_string(),
                "/no/such/prompt.md".to_string(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Failed to read message file: /no/such/prompt.md"),
            "stderr: {stderr}"
        );
        assert!(agent.seen_of("prompt").is_empty(), "nothing may be sent");
    }

    /// NUL bytes are legal UTF-8, so a binary file would otherwise sail
    /// through as a message with control characters in it. Refuse it.
    #[tokio::test]
    async fn run_command_message_file_rejects_binary_file() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("blob.bin");
        tokio::fs::write(&file, b"prompt\0\x01\x02tail")
            .await
            .unwrap();
        let (agent, addr) = message_file_mock().await;

        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--message-file".to_string(),
                file.display().to_string(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("message file is binary"),
            "stderr: {stderr}"
        );
        assert!(agent.seen_of("prompt").is_empty(), "nothing may be sent");
    }

    /// Invalid UTF-8 gets its own message (not "binary") so the caller knows
    /// to fix the encoding rather than the content.
    #[tokio::test]
    async fn run_command_message_file_rejects_non_utf8_file() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gbk.md");
        tokio::fs::write(&file, [0xB0, 0xA1, 0xBA, 0xC3])
            .await
            .unwrap(); // GBK "啊好"
        let (agent, addr) = message_file_mock().await;

        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--message-file".to_string(),
                file.display().to_string(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("message file is not valid UTF-8"),
            "stderr: {stderr}"
        );
        assert!(agent.seen_of("prompt").is_empty(), "nothing may be sent");
    }

    #[tokio::test]
    async fn run_command_no_wait_against_mock() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .responses
            .insert("new_session".into(), "{\"sessionId\":\"s-no-wait\"}".into());
        agent.responses.insert(
            "prompt".into(),
            "{\"run_id\":\"r-no-wait\",\"run_epoch\":1,\"accepted_state\":\"queued\",\"queue_position\":2}"
                .into(),
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;

        // text mode: prints an ack line and exits without streaming.
        let (out, cap) = Output::memory();
        run_command(
            &[
                "--grpc-addr".to_string(),
                addr.clone(),
                "--no-wait".to_string(),
                "hi".to_string(),
            ],
            &out,
        )
        .await
        .expect("run --no-wait");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Run r-no-wait accepted (queued) in session s-no-wait"));
        assert!(stdout.contains("queue position: 2"));
        let prompts = agent.seen_of("prompt");
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].busy_policy, "enqueue_coalescing");

        // json mode: emits the ack fields.
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .responses
            .insert("new_session".into(), "{\"sessionId\":\"s-no-wait\"}".into());
        agent.responses.insert(
            "prompt".into(),
            "{\"run_id\":\"r-no-wait\",\"run_epoch\":1,\"accepted_state\":\"queued\",\"queue_position\":2}"
                .into(),
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let (out, cap) = Output::memory();
        run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--no-wait".to_string(),
                "--mode".to_string(),
                "json".to_string(),
                "hi".to_string(),
            ],
            &out,
        )
        .await
        .expect("run --no-wait --mode json");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessionId"], "s-no-wait");
        assert_eq!(parsed["runId"], "r-no-wait");
        assert_eq!(parsed["acceptedState"], "queued");
        assert_eq!(parsed["queuePosition"], 2);
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
        let first_message = &prompts[0].message;
        assert!(
            first_message.contains("<file name=\""),
            "msg: {first_message}"
        );
        assert!(prompts[0].message.contains("FILE-CONTENTS"));
        assert!(prompts[0].message.ends_with("summarize"));
        // Repeated append-system-prompt joined with newline.
        let appends = agent.seen_of("append_system_prompt");
        assert_eq!(appends[0].system_prompt, "line1\nline2");
    }

    /// Regression: `future run --session <id> "..."` without `--cwd` must not
    /// send set_cwd — the session keeps the cwd it already has. Covers the CLI
    /// layer specifically: this is where the missing `--cwd` used to be turned
    /// into the caller's directory, which then rewrote the session.
    #[tokio::test]
    async fn run_command_existing_session_without_cwd_sends_no_set_cwd() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent {
            events: vec![
                crate::test_server::stream_event("text_chunk", "{\"text\":\"hi there\"}"),
                crate::test_server::stream_event("agent_end", "{}"),
            ],
            ..Default::default()
        };
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let (out, _cap) = Output::memory();
        run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--session".to_string(),
                "s-existing".to_string(),
                "hi".to_string(),
            ],
            &out,
        )
        .await
        .expect("run");
        assert!(agent.seen_of("new_session").is_empty());
        assert_eq!(agent.seen_of("prompt")[0].session_id, "s-existing");
        assert!(
            agent.seen_of("set_cwd").is_empty(),
            "set_cwd must not fire without --cwd: {:?}",
            agent.seen_of("set_cwd")
        );
    }

    /// End to end through the CLI entry: a new session is titled, `--no-title`
    /// suppresses it, and `--session` leaves an existing conversation alone.
    #[tokio::test]
    async fn run_command_titles_only_sessions_it_creates() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let state = "{\"queryCount\":1,\"isStreaming\":false,\"sessionName\":\"\"}";
        let events = vec![
            crate::test_server::stream_event("text_chunk", "{\"text\":\"hi there\"}"),
            crate::test_server::stream_event("agent_end", "{}"),
        ];

        // ① a session this run created → titled
        let mut agent = crate::test_server::MockAgent {
            events: events.clone(),
            ..Default::default()
        };
        agent
            .responses
            .insert("new_session".into(), "{\"sessionId\":\"s-run\"}".into());
        agent.responses.insert("get_state".into(), state.into());
        agent.responses.insert(
            "generate_session_title".into(),
            "{\"title\":\"Fresh title\"}".into(),
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let (out, _) = Output::memory();
        run_command(
            &["--grpc-addr".to_string(), addr.clone(), "hi".to_string()],
            &out,
        )
        .await
        .expect("run");
        assert_eq!(agent.seen_of("set_session_name").len(), 1);
        assert_eq!(agent.seen_of("set_session_name")[0].name, "Fresh title");

        // ② `--no-title` → the same setup generates nothing
        let mut agent = crate::test_server::MockAgent {
            events: events.clone(),
            ..Default::default()
        };
        agent
            .responses
            .insert("new_session".into(), "{\"sessionId\":\"s-run\"}".into());
        agent.responses.insert("get_state".into(), state.into());
        agent.responses.insert(
            "generate_session_title".into(),
            "{\"title\":\"Fresh title\"}".into(),
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let (out, _) = Output::memory();
        run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--no-title".to_string(),
                "hi".to_string(),
            ],
            &out,
        )
        .await
        .expect("run");
        assert!(agent.seen_of("generate_session_title").is_empty());
        assert!(agent.seen_of("set_session_name").is_empty());

        // ③ an existing session, even a nameless one-answer one, is untouched
        let mut agent = crate::test_server::MockAgent {
            events,
            ..Default::default()
        };
        agent.responses.insert("get_state".into(), state.into());
        agent.responses.insert(
            "generate_session_title".into(),
            "{\"title\":\"Fresh title\"}".into(),
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let (out, _) = Output::memory();
        run_command(
            &[
                "--grpc-addr".to_string(),
                addr,
                "--session".to_string(),
                "s-existing".to_string(),
                "hi".to_string(),
            ],
            &out,
        )
        .await
        .expect("run");
        assert!(agent.seen_of("new_session").is_empty());
        assert!(agent.seen_of("generate_session_title").is_empty());
        assert!(agent.seen_of("set_session_name").is_empty());
    }

    #[tokio::test]
    async fn run_command_error_output_modes() {
        let _guard = crate::test_env::lock_env().await;
        // Dead port → error printed per mode.
        let (out, cap) = Output::memory();
        let result = run_command(
            &[
                "--grpc-addr".to_string(),
                "127.0.0.1:1".to_string(),
                "hi".to_string(),
            ],
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
