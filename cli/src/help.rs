//! Help text — verbatim port of `cli/src/help.ts` (`printHelp`) plus the
//! per-group help strings inlined in `cli/src/index.ts`. Bytes must match the
//! TypeScript CLI exactly (golden-tested in P4).

/// `printHelp()` from cli/src/help.ts.
pub const MAIN_HELP: &str = r#"Future OS CLI — agent gateway for the Future Agent gRPC server (per-user local IPC by default).

Usage:
  future <group> <command> [options] [args...]

Groups:
  init      Install built-in skills and initialize local commands
  config    Configure a model provider, and read/write global settings
  desktop   Read and change the desktop app's settings
  workspace List, and add, the desktop app's workspaces
  auth      Authentication & API key management
  account   Platform account info
  run       Send a prompt to the agent (one-shot, non-interactive)
  skills    Install & manage agent skills
  task      Manage FutureOS tasks (reusable prompt + trigger + runs)
  tools     List, describe, and call platform & browser tools
  models    List available AI models from the agent
  session   List, inspect, update, rename, and delete agent sessions
  doctor    Environment diagnostic
  version   Print the build identity (version, commit, target)

Apps (run the FutureOS components — same as their standalone binaries):
  agent     Start the agent gRPC server (future-agent)
  tui       Launch the terminal UI (future-tui)
  channel   Start the IM channel bridge (future-channel)
  loop      Loop control plane: goals/todos/gates (future-loop)

Quick start:
  future init                                Initialize Future OS
  future config                              Configure a model provider
  future auth login                          Sign in to the Future platform
  future agent                               Start the agent server
  future tui                                 Launch the terminal UI
  future run "Explain this project"          One-shot agent prompt
  future run @README.md "Summarize this"     Include files in prompt
  future skills install-builtin              Install all built-in skills
  future doctor                              Check everything is working

Run 'future <group> --help' for per-group details.
  future init --help         Initialization behavior
  future config --help       Interactive model-provider setup
  future desktop --help      Desktop app settings
  future workspace --help    Desktop app workspaces (list/add)
  future run --help          All run options (model, fork, thinking, tools, etc.)
  future auth --help         Auth subcommands
  future account --help      Account subcommands
  future skills --help       Skills subcommands
  future task --help         Task management (list/show/add/run/runs)
  future tools --help        Tool subcommands
  future models --help       Model listing options
  future session --help      Session management options
  future agent --help        Agent server options (gRPC addr, logging, profiling)
  future tui --help          TUI options (print mode, list models, etc.)
  future loop --help         Loop control plane commands
  future version --json      Build identity: version, commit, target
  future --version           Print version and exit"#;

/// `future init --help` output (index.ts).
pub const INIT_HELP: &str = r#"future init — initialize Future OS

Usage:
  future init

Installs all built-in skills. On macOS and Linux, also links future and, when
available, its sibling future-agent into ~/.future/bin/ and prints a PATH setup hint."#;

/// `future config --help` output.
pub const CONFIG_HELP: &str = r#"future config — configure a model provider, or read/write global settings

Usage:
  future config                       Interactive model-provider setup
  future config get [<key>] [--json]  Show the effective agent settings
  future config set <key> <value>     Change one setting
  future config get --help            All settings keys and their defaults

Subcommands:
  get       With no key, prints the settings file path and every effective
            setting value (defaults included). With a key, prints that value
            alone; --json prints it typed, or the whole document.
  set       Writes one key into ~/.future/agent/settings.json, creating the
            file if needed. Other keys, and any key this build does not know
            about, are left untouched.

Interactive setup (no subcommand):
  FutureOS  Reuse the device-code login flow. If a token is already configured,
            asks before replacing it.
  Custom    Prompt for provider ID, API protocol, base URL and API key, then
            keep, edit, delete or add models with their own token limits, image
            support and per-1M-token prices; then update
            ~/.future/agent/models.json and ~/.future/agent/auth.json.

API keys are read without terminal echo. If Future Agent is running, custom
provider changes take effect immediately; otherwise they apply on its next start.
Settings reads and writes never need the Agent: settings are read from disk when
they are used, so a `set` applies to the next new session (or the next Agent
start, for the compaction and retry policy).

Settable keys (the dotted names are the exact keys in settings.json):
  compaction.enabled                  true|false  Auto-compaction on/off
  compaction.reserve_tokens           integer     Context reserved for the reply
  compaction.keep_recent_tokens       integer     Recent tokens kept verbatim
  retry.enabled                       true|false  Automatic retry on/off
  retry.max_retries                   integer     Retries per request
  retry.base_delay_ms                 integer     Base backoff delay (ms)
  retry.provider.max_retry_delay_ms   integer     Provider-level retry cap (ms)
  maxTurns                            integer     Model+tool turns per prompt (0 = unlimited)
  defaultPermissionLevel              all|workspace|none
  defaultModel                        model id    Global default model ("provider/id"; "" = none)"#;

/// `future config get --help` output.
///
/// This is the canonical list of settable keys: `CONFIG_HELP` and
/// `CONFIG_SET_HELP` both point readers here ("Run `future config get --help`
/// for the settable keys"), so it has to carry them. `config_keys_are_documented`
/// pins that every key the command accepts appears here.
pub const CONFIG_GET_HELP: &str = r#"future config get — show the effective agent settings

Usage:
  future config get [<key>] [--json]

With no key, prints the settings file path and every effective value, including
the defaults for keys the file omits. With a key, prints that value alone, which
makes it usable in a script. --json prints the value typed (or, with no key, the
whole effective document as JSON).

Reads ~/.future/agent/settings.json and never writes it, so it works with the
Agent stopped. It does not read auth.json: credentials are never part of the
settings document.

Settable keys (the dotted names are the exact keys in settings.json, which nests
snake_case inside the camelCase top level):
  compaction.enabled                  true|false  Auto-compaction on/off
  compaction.reserve_tokens           integer     Context reserved for the reply
  compaction.keep_recent_tokens       integer     Recent tokens kept verbatim
  retry.enabled                       true|false  Automatic retry on/off
  retry.max_retries                   integer     Retries per request
  retry.base_delay_ms                 integer     Base backoff delay (ms)
  retry.provider.max_retry_delay_ms   integer     Provider-level retry cap (ms)
  maxTurns                            integer     Model+tool turns per prompt (0 = unlimited)
  defaultPermissionLevel              all|workspace|none
  defaultModel                        model id    Global default model ("provider/id"; "" = none)

When a change takes effect:
  defaultModel, defaultPermissionLevel   the next new session
  compaction.*, retry.*, maxTurns        the next Agent start

Change a value with `future config set <key> <value>`."#;

/// `future config set --help` output.
pub const CONFIG_SET_HELP: &str = r#"future config set — change one global agent setting

Usage:
  future config set <key> <value> [--json]

Writes one key into ~/.future/agent/settings.json (created if missing) and
prints the new value. Other keys are preserved, including keys this build does
not know about, and the file keeps the Agent's own formatting.

Run `future config get --help` for the settable keys, their accepted values and
their defaults.

Values are validated before the file is touched, and an invalid value or an
unknown key leaves the file exactly as it was.

When a change takes effect:
  defaultModel, defaultPermissionLevel   the next new session
  compaction.*, retry.*, maxTurns        the next Agent start

No Agent is required; a running one is unaffected, since these settings are read
from disk when they are used."#;

/// `future desktop --help` output.
pub const DESKTOP_HELP: &str = r#"future desktop — read and change the desktop app's settings

Usage:
  future desktop settings [<key>] [--json]   Show the effective settings
  future desktop settings get [<key>] [--json]
  future desktop settings set <key> <value>

The desktop app keeps its own preferences in an `app_settings` table in
~/.future/app/app.db. They are separate from the agent settings document that
`future config` writes, and from models/providers/auth, which the app and the
agent share.

With no key, prints the database path and every effective value (defaults
included). A database the desktop app has never written reports the defaults and
is not created. With a key, prints that value alone, which makes it usable in a
script; --json prints it typed, or the whole document.

No running desktop app is required. A running one picks the change up the next
time it reads the settings.

Settable keys (camelCase, as the desktop API spells them):
  approvalTier            off|manual|sandbox|auto  Approval tier for file access and shell
  hiddenModels            list               Model ids hidden from the model picker
  autoUpgradeSkills       true|false         Upgrade installed skills on app open
  autoConnectRemote       true|false         Auto-connect the paired remote device
  skillGuideDismissed     true|false         Skills onboarding banner dismissed
  skillIntroDismissed     true|false         Skills intro bubble acknowledged
  bellOnComplete          true|false         Bell when a run finishes
  autoTitleFirstTurn      true|false         Generate a title after the first answer
  titleLanguage           en|zh              Language used for generated titles
  communityEdition        true|false         Use the community-edition UI
  skillRecommend          true|false         Recommend one uninstalled skill

A list value is a JSON array or a comma-separated list; an empty value clears it."#;

/// `future workspace --help` output.
pub const WORKSPACE_HELP: &str = r#"future workspace — manage the desktop app's workspaces

Usage:
  future workspace list [--json]                    List the user's workspaces
  future workspace add <path> [--name <name>] [--json]
                                                   Add a directory as a workspace

A workspace is a directory on this machine that workspace conversations are
filed under. The records live in the desktop app's own database
(~/.future/app/app.db) — the same store the app's new-conversation dialog and a
paired phone write — so a workspace added here appears in the desktop app's
sidebar and on the phone once the app publishes its catalogue. No running
desktop app is required for the write.

The directory must already exist: a workspace points at a directory, it does not
create one. Adding a path that already has a workspace reopens that workspace
instead of creating a second one for the same directory, however the path is
spelled (~, a symlinked path, a trailing separator) — one directory is always
one workspace.

Workspaces also appear on their own: `future session set <id> --cwd <dir>` files
that conversation under the directory's workspace, and the desktop app does the
same whenever a cwd changes. `future workspace list` is then the place to see
where conversations are filed.

Examples:
  future workspace list
  future workspace add ~/projects/demo
  future workspace add /srv/app --name "Service"

With --json, list prints an array of workspace records and add prints the
record it created or reopened, with "created" saying which."#;

/// `future desktop settings get --help` output.
pub const DESKTOP_GET_HELP: &str = r#"future desktop settings get — show the desktop app's effective settings

Usage:
  future desktop settings [<key>] [--json]
  future desktop settings get [<key>] [--json]

With no key, prints the database path (default ~/.future/app/app.db) and every
effective value, including the defaults for keys never written. With a key,
prints that value alone, which makes it usable in a script. --json prints the
value typed (or, with no key, the whole effective document as JSON).

Reads only: a database the desktop app has never written reports the defaults
rather than creating one. Credentials are never part of this document."#;

/// `future desktop settings set --help` output.
pub const DESKTOP_SET_HELP: &str = r#"future desktop settings set — change one desktop app setting

Usage:
  future desktop settings set <key> <value> [--json]

Writes one key into ~/.future/app/app.db (created if missing) and prints the new
value. Other keys are preserved. Values are validated before the database is
touched, so an invalid value or an unknown key leaves it exactly as it was.

Run `future desktop --help` for the settable keys, their accepted values and
their defaults.

The running desktop app is unaffected until it next reads the settings; a change
made in the app's own Settings screen applies immediately."#;

/// `future task --help` output.
pub const TASK_HELP: &str = r#"future task — manage FutureOS tasks (reusable prompt + trigger + full-permission runs)

A task is a reusable work unit: prompt + working directory + model/thinking
level, run at full permission when its trigger fires. Each run produces an
ordinary conversation and a durable run-ledger entry.

Usage:
  future task list [--all] [--json]                 List tasks
  future task show <id|name> [--json] [--prompt]    Show one task
  future task add --name N --prompt P|--prompt-file F --cwd D
        [--model M] [--thinking L] [--session new|existing]
        [--session-retention keep|delete]
        [--conversation workspace|chat] [--disabled] [--json]
        [--depends-on A[:success|failure|completed]]… [--join-any]
        (--manual | --at "YYYY-MM-DD HH:MM" | --every 30m
         | --daily [--time 09:00]
         | --weekly --days mon,wed,fri [--time 10:00]
         | --monthly --day 31 [--time 09:00]
         | --yearly --month 12 --day 31 [--time 22:00])
  future task edit <id|name> [any add flag] [--enable|--disable]
        [--join-all|--join-any] [--depends-on …] [--json]
  future task enable|disable <id|name>
  future task remove <id|name> [--yes]              Soft-delete; runs are kept
  future task run <id|name> [--wait] [--timeout 15m] [--json]
  future task runs <id|name> [--limit N] [--json]
  future task output <run-id> [--tail N] [--json]   A run's full answer
  future task feedback <run-id> good|bad [--note "…"]
  future task upstream|deps <id|name> [--json]     Dependency edges + progress
  future task prompt log <id|name> [--json]         The prompt version history
  future task prompt apply <id|name> <revision-id>  Put a stored version back in force
  future task prompt revert <id|name>               Back to the previous version

Triggers:
  --manual    Only runs when triggered: `future task run`, the desktop panel,
              the phone, or a dependency of another task. This is also what an
              add without any trigger flag produces.
  --at        One-shot on a calendar date/time (local time).
  --every     Every N minutes (30m, 2h, 1d). Anchored to a grid; no drift.
  --daily     Every day at --time (default 09:00).
  --weekly    Every selected weekday at --time (default 09:00).
  --monthly   Every month on --day (1-31) at --time (default 09:00).
              Months without that day run on the last day of the month.
  --yearly    Once a year on --month (1-12) and --day (1-31) at --time
              (default 09:00). A day the month cannot hold runs on the last
              day of that month, so 2/29 runs on the 28th in a common year.

Dependencies:
  --depends-on A            Run when A finishes successfully.
  --depends-on A:failure    …when A fails. Also :completed for either.
  --join-any                Run when any one upstream has finished (default is
                            to wait for all of them).
  The upstream's result summary is injected into the run's prompt, and
  `future task upstream` shows which edges have fired.

Session:
  --session new       Each run opens a new conversation (default).
  --session existing  Reuse one conversation, so each run continues from what
                      the previous ones did.
  --session-retention keep    Keep the conversation (default).
  --session-retention delete  Delete it once the run settles, whether it
                      succeeded or failed. Only valid with `--session new`: a
                      conversation that is deleted cannot be the one the next
                      run continues.
                      What the run *did* is not lost — its status, its result
                      summary and its whole answer are recorded on the run, so
                      `future task runs` and `future task output <run-id>`
                      still answer, and a dependent task still receives the
                      summary. Only the conversation (its reasoning, its tools,
                      its transcript) goes.
  --conversation workspace  File the conversation under the working directory
                            (default).
  --conversation chat       Open it as a chat conversation instead (it runs in
                            the conversation's own temporary workspace, so
                            --cwd is optional).

Prompt versions:
  Editing a prompt (here or in the desktop) records a new version and keeps the
  one it replaced, so `prompt revert` can walk all the way back to the task's
  first prompt. Version source reads as: user (manual edit), rollback (a version
  re-applied), reflection (a version accepted from an older build's prompt
  suggestions), superseded (an outgoing version kept for the history).

Reading a run:
  `future task runs` lists the ledger (statuses, versions, summaries).
  `future task output <run-id>` prints the full answer behind a summary (the
  answer saved on the run; a run recorded before that was stored falls back to
  the conversation), and `feedback` records your verdict on a run.

Execution:
  Runs are executed by the desktop (or headless desktop) tick loop, not by
  this CLI. `future task run` queues a request; the next tick picks it up.
  `--wait` blocks until the run reaches a terminal state and prints its
  status, thread/session ids, and result summary.

Not to be confused with `future loop todo` (long-running goals with evidence
and gates). Tasks are fixed actions with triggers."#;

/// `future version --help` output.
pub const VERSION_HELP: &str = r#"future version — print the build identity of this CLI

Usage:
  future version [--json]
  future --version | -v | version      Same thing, plain output

Plain output is the display version (`future v0.0.2-479c8fee+local`), the same
string `future --version` has always printed. `--json` adds the facts that
string cannot carry:

  version         Display version
  isRelease       true when the version is a release (its first component is
                  non-zero); `0.*` is a dev build
  bundleVersion   Plain semver core, what installers use (they reject suffixes)
  gitCommit       Full commit this binary was built from, or null when the build
                  had no git checkout (tarball/vendored build)
  gitCommitShort  Abbreviated form
  gitDirty        Whether the tree had uncommitted changes at build time; null
                  when gitCommit is null
  buildTarget     Target triple the binary was compiled for
  buildProfile    Cargo profile (`debug` or `release`)

`gitCommit` is present even for release and coordinated test/nightly builds,
whose version string carries no hash at all. Use it to check whether the binary
you are running is the commit you are reading."#;

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
  install-builtin         Install platform skills classified as builtin (from builtin/).
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

Requires a running agent (connects over per-user local IPC by default).
Set FUTURE_AGENT_GRPC_ADDR to an explicit TCP address for remote/development use."#;
