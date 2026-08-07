//! Help text — verbatim port of `cli/src/help.ts` (`printHelp`) plus the
//! per-group help strings inlined in `cli/src/index.ts`. Bytes must match the
//! TypeScript CLI exactly (golden-tested in P4).

/// `printHelp()` from cli/src/help.ts.
pub const MAIN_HELP: &str = r#"Future OS CLI — agent gateway for the Future Agent gRPC server (default 127.0.0.1:50051).

Usage:
  future <group> <command> [options] [args...]

Groups:
  init      Install built-in skills and initialize local commands
  auth      Authentication & API key management
  account   Platform account info
  run       Send a prompt to the agent (one-shot, non-interactive)
  skills    Install & manage agent skills
  tools     List, describe, and call platform & browser tools
  models    List available AI models from the agent
  session   List, inspect, rename, and delete agent sessions
  doctor    Environment diagnostic

Apps (run the FutureOS components — same as their standalone binaries):
  agent     Start the agent gRPC server (future-agent)
  tui       Launch the terminal UI (future-tui)
  channel   Start the IM channel bridge (future-channel)
  loop      Loop control plane: goals/todos/gates (future-loop)

Quick start:
  future init                                Initialize Future OS
  future auth login                          Sign in to the Future platform
  future agent                               Start the agent server
  future tui                                 Launch the terminal UI
  future run "Explain this project"          One-shot agent prompt
  future run @README.md "Summarize this"     Include files in prompt
  future skills install-builtin              Install all built-in skills
  future doctor                              Check everything is working

Run 'future <group> --help' for per-group details.
  future init --help         Initialization behavior
  future run --help          All run options (model, fork, thinking, tools, etc.)
  future auth --help         Auth subcommands
  future account --help      Account subcommands
  future skills --help       Skills subcommands
  future tools --help        Tool subcommands
  future models --help       Model listing options
  future session --help      Session management options
  future agent --help        Agent server options (gRPC addr, logging, profiling)
  future tui --help          TUI options (print mode, list models, etc.)
  future loop --help         Loop control plane commands
  future --version           Print version and exit"#;

/// `future init --help` output (index.ts).
pub const INIT_HELP: &str = r#"future init — initialize Future OS

Usage:
  future init

Installs all built-in skills. On macOS and Linux, also links future and, when
available, its sibling future-agent into ~/.future/bin/ and prints a PATH setup hint."#;

/// `future auth` group help (index.ts, no-command / --help branch).
pub const AUTH_GROUP_HELP: &str = r#"future auth — authenticate with the Future platform

Usage:
  future auth <command>

Commands:
  login       Device-code OAuth flow; saves API key to ~/.future/agent/auth.json
  status      Show whether logged in, and the platform URL in use
  credential  Output the API key + endpoint for shell scripts. Output is always JSON
              on success; use --json for consistent JSON error output when not logged in.
  logout      Remove the stored API key from auth.json

API key file: ~/.future/agent/auth.json
Environment override: FUTURE_API_KEY (takes precedence over auth.json)"#;

/// `future auth` group help shown after an unknown subcommand (index.ts) —
/// note the `credential` line differs from AUTH_GROUP_HELP.
pub const AUTH_GROUP_HELP_UNKNOWN: &str = r#"future auth — authenticate with the Future platform

Usage:
  future auth <command>

Commands:
  login       Device-code OAuth flow; saves API key to ~/.future/agent/auth.json
  status      Show whether logged in, and the platform URL in use
  credential  Output the raw API key + endpoint for shell scripts (--json not needed;
              output is always JSON: {"api_key":"...","endpoint":"..."})
  logout      Remove the stored API key from auth.json

API key file: ~/.future/agent/auth.json
Environment override: FUTURE_API_KEY (takes precedence over auth.json)"#;

/// `future auth login --help` output (index.ts).
pub const AUTH_LOGIN_HELP: &str = r#"future auth login — device-code OAuth flow

Usage:
  future auth login [--url <url>]

  --url <url>   Override the platform URL (default from DNS TXT record or built-in)
  --help, -h    Show this help

Opens a browser for you to sign in and authorize this CLI device.
Saves the resulting API key to ~/.future/agent/auth.json."#;

/// `future auth status --help` output (index.ts).
pub const AUTH_STATUS_HELP: &str = "future auth status — check current login state\n\nShows the platform URL and indicates whether an API key is stored.\nDoes not validate the key against the server.";

/// `future auth credential --help` output (index.ts).
pub const AUTH_CREDENTIAL_HELP: &str = r#"future auth credential — output API key for scripting

Usage:
  future auth credential [--json]

Output (always JSON on success):
  {"api_key":"...","endpoint":"..."}

  --json    When not logged in, emit JSON error instead of plain text.
            On success the output is always JSON regardless of this flag.

Useful for piping into other tools or CI/CD scripts."#;

/// `future auth logout --help` output (index.ts).
pub const AUTH_LOGOUT_HELP: &str = "future auth logout — remove stored API key\n\nDeletes the Future provider key from ~/.future/agent/auth.json.\nOther provider keys in the file are left untouched.";

/// `future tools` group help (index.ts).
pub const TOOLS_GROUP_HELP: &str = r#"future tools — list, describe, and call platform & browser tools

Usage:
  future tools list [--json]
  future tools describe <name>
  future tools call <name> --key1 val1 --key2 val2 [...]

Commands:
  list               Show available tools. --json for machine output.
  describe <name>    Show a tool's arguments and usage example.
  call <name>        Invoke a tool. Args as --key value. Use describe first to see
                     what arguments each tool accepts.

Requires authentication: future auth login, or set the FUTURE_API_KEY environment variable."#;

/// `future skills` group help (index.ts).
pub const SKILLS_GROUP_HELP: &str = r#"future skills — install & manage agent skills

Skills are markdown instruction files the agent loads to handle specific tasks.
They live under ~/.future/agent/skills/<name>/SKILL.md.

Usage:
  future skills <command> [args]

Commands:
  list                    Show all skills available in the catalog (name, latest version,
                          installed version, description).
  install <name>          Install a specific skill by name. Use --version <ver> for a
                          specific version; omit for latest.
  install                 With no name argument, same as install-builtin.
  install-builtin         Install all built-in platform skills (names prefixed "future-").
  uninstall <name>        Remove an installed skill.
  update                  Upgrade all installed skills to their latest versions.

Skills directory: ~/.future/agent/skills/
Catalog source: fetched from the Future platform API."#;

/// `future account` group help (index.ts).
pub const ACCOUNT_GROUP_HELP: &str = r#"future account — view platform account information

Usage:
  future account <command>

Commands:
  profile     Show account profile (email, user ID, verification status, creation date)
  balance     Show account credit balance. Use --json for machine-readable output.

Requires authentication: future auth login first."#;

/// `future models --help` output (index.ts).
pub const MODELS_HELP: &str = r#"future models — list available models from the running agent

Usage:
  future models [--json]

  --json    Output as JSON array with id, label, provider, contextWindow,
            supportsImages, thinkingLevel, and isDefault fields.
  --help    Show this help.

Requires a running agent (connects to 127.0.0.1:50051 by default).
Override with FUTURE_AGENT_GRPC_ADDR environment variable."#;
