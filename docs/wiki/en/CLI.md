# Command line tool (`future`)

FutureOS ships with an **optional** command-line tool called `future`. It comes with every download.

> **You probably don't need this.** The desktop app already handles everyday use. Reach for the CLI only when you want to script things, automate a task, or work purely in a terminal. **If you're not comfortable with a terminal, you can skip this page.**

---

## Where it is

The tool travels with the app:

| System | Location |
|---|---|
| **macOS** (`.dmg`) | Inside the app: `/Applications/FutureOS.app/Contents/MacOS/future` |
| **Windows** (installer or portable `.zip`) | `future.exe` in the app folder |
| **Linux** (`.deb`) | `future` on PATH |
| **Linux** (portable / CLI-only tarball) | `future` in the extracted folder; use `./future` until it is on PATH |

The CLI ships in **every** download — both the installer and the portable package include it, sitting next to the app.

---

## Running it

Open a terminal in the folder that contains the binary, then run it with `--help` to see everything:

```bash
future --help
```

To make it easier to run from anywhere, add its folder to your `PATH`, or set up an alias. For example, on macOS:

```bash
alias future="/Applications/FutureOS.app/Contents/MacOS/future"
```

### The agent must be running

Most commands connect to the FutureOS agent (the background service). If the **desktop app is open**, the agent is already running. Otherwise, start the agent with `future agent` (or the `future-agent` binary directly — both are the same code), or open the desktop app, which starts it automatically.

`future config`, `future auth login`, and `future auth logout` also work with the agent stopped, writing configuration under `~/.future/agent/`. Skill catalog/install commands do not require an agent either. The TUI retains independent Agent startup; Desktop hosts the complete Agent in process or attaches to a compatible external service. Exiting an embedded owner also stops CLI/TUI work; see the [Desktop Agent guide](../../guide/desktop-agent.md). `future doctor` diagnoses connectivity; default discovery is per-user local IPC, not an always-open TCP port.

---

## Command groups

### `init` — first-time setup

```bash
future init
```

Installs all built-in skills. On macOS and Linux, also links `future` (and `future-agent` when available) into `~/.future/bin/` and prints a PATH setup hint.

### `config` — configure a model provider, and read/write global settings

```bash
future config
```

Interactively select **FutureOS** or a **custom provider**:

- FutureOS: if a token is already configured, asks whether to sign in again; otherwise starts the device-code login flow.
- Custom: prompts for the provider ID, API protocol, base URL, API key, model ID, context window, and related settings. API key input is not echoed.

Configuration is written to `~/.future/agent/models.json` and `~/.future/agent/auth.json`. If the agent is running, the command writes through its RPC and refreshes the model registry immediately; otherwise the changes take effect the next time the agent starts.

The same group reads and changes the agent's global settings, with no subcommand
needed to stay interactive:

```bash
future config get                                   # effective settings, defaults included
future config get defaultPermissionLevel            # one value, for scripts
future config set defaultPermissionLevel workspace  # change one key
future config set compaction.reserve_tokens 8192
```

`future config get --help` lists every settable key and its accepted values.
Writes are validated first, so a bad value or an unknown key leaves
`settings.json` untouched. Neither direction needs a running agent; a change
applies to the next new session (`defaultModel`, `defaultPermissionLevel`) or the
next agent start (`compaction.*`, `retry.*`, `maxTurns`). See
[Self-inspection](../../guide/self-inspection.md) for the full picture.

### `auth` — sign in and out

```bash
future auth login       # sign in via your browser (device-code flow)
future auth status      # show whether you're signed in
future auth credential  # print the API key + endpoint for shell scripts
future auth logout      # sign out
```

### `account` — your account

```bash
future account profile  # email, user ID, verification status, creation date
future account balance  # credit balance (--json for machine output)
```

Both commands are free. They are the only CLI reads that leave the machine: they
call the Future platform over the network and need a login (`future auth login`).
The CLI reads the stored API key itself, so no key ever needs to be passed — and
seeing a low balance is information to report, not a reason to create a recharge
order.

### `version` — which build is this

```bash
future version            # same string as `future --version`
future version --json     # version, commit, dirty state, target, profile
```

`--json` reports what the version string cannot: `gitCommit` (the full object
name this binary was built from), `gitDirty`, `buildTarget` and `buildProfile`.
Release and coordinated test/nightly builds carry **no** commit in their version
at all, so this is the only way to check whether the binary you are running is
the commit you are reading.

### `run` — send a one-off prompt and print the answer

```bash
future run "Explain this project"
```

Useful options and forms:

| Form | What it does |
|---|---|
| `--model <model>` | Choose the model. Supports `model:thinking`, e.g. `sonnet:high`. |
| `--thinking <level>` | Thinking level: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`. |
| `@<path>` | Include a file's contents in the prompt. |
| `--continue`, `-c` | Continue the most recent session. |
| `--session <id>` | Connect to an existing session by ID. |
| `--fork <entry-id>` | Fork a new session from a specific entry in the current session. |
| `--permission <level>` | `all` (fresh-session default, unrestricted), `workspace` (approval-gated access), or `none` (deny all tool calls). This does not select an OS sandbox; see [[Sandbox]]. |
| `--tools, -t <names>` | Comma-separated tool names to enable (e.g. `read,shell`); `--no-tools` / `--no-builtin-tools` disable them. |
| `--steer` | Interrupt the session's current run; without it, the prompt queues behind a busy run. |
| `--no-wait` | Submit the prompt, print an ack, and exit; retrieve the result later with `future session wait <id>`. |
| `--system-prompt`, `--append-system-prompt` | Replace or extend the system prompt. Both are recorded with the session, so they persist. |
| `--cwd <dir>` | Set the working directory. |
| `--mode json` | Print the answer as JSON instead of text. |
| `--verbose` | Write progress and tool calls to stderr. |
| `--no-title` | Do not name a session this run creates. By default the first answer of a NEW session generates and saves a title; an existing session (`--session`/`--continue`/`--fork`) is never renamed. |
| `--grpc-addr <addr>` | Explicit agent TCP address (default: per-user local IPC; also `FUTURE_AGENT_GRPC_ADDR`). |
| `--no-session` | Don't save this exchange as a session. |

Examples:

```bash
future run --model sonnet:high "Review the changes"
future run @README.md "Summarize this file"
future run --tools read,shell "Read the README and list files"
```

Run `future run --help` for every option.

After the first answer of a session `future run` **created**, the run generates and saves a
title: one extra model call (it can take tens of seconds and spend credits) using the same
generator as the desktop app and `future session title`, in the desktop app's `titleLanguage`.
A session attached through `--session`, `--continue` or `--fork` is **never** renamed. If you
rename the conversation, send another turn, or the session starts running again while the title
is being generated, the stale suggestion is discarded rather than written over your name. A
title failure is reported on stderr and ignored — the run still succeeds and the answer is
printed in full. Use `--no-title` to turn the behaviour off.

### `skills` — manage capability packs

```bash
future skills list             # list catalog skills (installed + available)
future skills install <name>   # install a specific skill
future skills install-builtin  # install skills from builtin/ (catalogue classification)
future skills uninstall <name> # remove an installed skill
future skills update           # upgrade all installed skills
```

### `tools` — list and call tools

```bash
future tools list
future tools describe <name>
future tools call <name> --args '<json>'
future tools call <name> --stdin
future tools call <name> --args '<json>' --output result.png
```

File-path arguments are converted automatically where a tool expects file content.

### `mcp` — configure MCP servers and call their tools

```bash
future mcp list                          # configured servers, and the file they come from
future mcp status                        # start each, shake hands, report tool counts
future mcp add rea --command rea --arg --mcp
future mcp tools rea --query binary      # that server's tools, one line each
future mcp describe rea inspect_go_binary  # one tool's full argument schema
future mcp call rea inspect_go_binary --args '{"path":"/usr/bin/foo"}'
future mcp call rea inspect_go_binary --args '{...}' --out full.json  # write all of it
future mcp serve rea                     # hold one session, take requests on stdin
future mcp remove rea
```

Servers live in `~/.future/mcp.json` (beside `agent/`, so `FUTURE_HOME` moves them
with the instance) and run as local processes speaking MCP over stdio. The agent
has no MCP client of its own, so this group is how an agent session reaches one:
the skill or the user calls `future mcp` through the ordinary `shell` tool.

`list` answers "what is configured"; `status` answers "is it usable" — it starts
each server, completes the handshake, reports the tool count and timing, and stops
it again, without calling a tool. On failure it appends the server's own stderr
(usually the real explanation: a missing engine, a missing credential), and any
unreachable server makes the command exit 1; `enabled: false` is reported as
`disabled` and does not fail the check.

Every step is bounded because the servers this exists for are large. `tools`
shows one line per tool and pages (`--limit`, default 50; `--offset`
continues). `describe` takes one exact name and prints its full JSON Schema.
`call` prints at most `--max-bytes` (default 64 KB) and tells you to rerun with
`--out <file>` for the rest, so a tool that returns hundreds of megabytes cannot
flood the terminal it was asked from. Add `--timeout <secs>` (default 60) when a
server is slow to start.

There are two budgets with a flag each: `--max-bytes` caps what is **printed**,
`--max-frame-bytes` caps one **read** frame (default 32 MiB, or
`FUTURE_MCP_MAX_FRAME_BYTES`). `--max-bytes` cannot help when a frame does not
fit at all, so the over-cap error names the method being awaited, says which
budget it is, and gives both knobs; many servers also have their own response
budget (REA's is `REA_MCP_MAX_RESPONSE_BYTES`). A mistyped tool name gets the
same "Did you mean" hint from `call` as from `describe`.

`call` starts the server on every invocation, which costs nothing for a fast one
and a lot for an engine that loads and analyses the target at launch: a Ghidra
cold start is about four minutes, so five calls cost five of them. `serve` pays it
once:

```bash
printf '%s\n' '{"tool":"open_binary","args":{"path":"/bin/ls"}}' \
              '{"tool":"open_binary","args":{"path":"/bin/cat"}}' \
  | future mcp serve rea
# {"ok":true,"ready":true,"server":"rea","toolCount":207}
# {"ok":true,"isError":false,"result":{...}}
# {"ok":true,"isError":false,"result":{...}}
```

Both directions are newline-delimited JSON: a request is `{"tool":...,"args":{...}}`
and an answer is `{"ok":...,"result"/"error":...}`. The first line is a ready ack
(the handshake is done and the tool count is known), so a caller waits on that
line instead of guessing how long analysis takes. Requests are answered in order,
one at a time: a stateful engine session is not something to drive concurrently.
A request that fails on its own — malformed JSON, no `tool`, a server rejection,
a tool error — only affects itself; the session continues. stderr is left to the
server's own logs, so stdout stays a clean JSON stream. A tool that reports its own error (`isError`) prints its
payload on **stderr** and exits 1, the same way `future tools call` reports a
failed tool — a script or an agent does not have to parse a marker out of stdout
to learn the call did not work.

### `models` — list available models

```bash
future models            # list models from the running agent
future models --json     # machine-readable output
```

### `agent` — run the agent server

```bash
future agent              # start the agent gRPC server
future agent --help       # agent options: transport override, logging, profiling
future agent --probe-sandbox # one-shot OS sandbox diagnostic; no server startup
future agent --home /tmp/futureos-b  # second instance with its own state and IPC endpoint
```

`future agent <args>` runs the agent backend directly with the same arguments
as the standalone `future-agent` binary — including `--home DIR`, which makes
the FutureOS home (`~/.future` by default, or `$FUTURE_HOME`) something else
for this instance, so several Agents can run beside each other without sharing
sessions, a lock or a socket (see
[Directory layout](../../guide/directory-layout.md#running-several-isolated-instances-future_home)).
Clients join a specific instance by setting the same `FUTURE_HOME`.

### `tui` / `channel` / `loop` — run the other components

`future` is the unified entry point for every Rust component — each runs the
same code as its standalone binary (the `future-*` binaries still exist as
build targets but no longer ship by default):

```bash
future tui                # terminal UI
future channel            # IM channel bridge: Feishu / DingTalk
future goal status        # goal control plane: goals/todos/gates
```

### `session` — manage sessions

```bash
future session list
future session set <id> [--parent <id>] [--title <name>] [--cwd <dir>]
                        [--model <id>] [--thinking <level>]
future session info <id> [--json]
future session status <id> [--json]     # live state: permission, sandbox, context, runs
future session transcript --help # filter and window one session's records
future session history --help   # search and read original history
future session compact --help   # request manual context compaction
future session rename <id> <name>
future session new [--cwd <dir>] [--name <text>]
future session forks <id>                # turns this session can be branched at
future session fork <id> --entry <id>
future session clone <id>
future session title <id> [--apply]      # generate a title (a model call)
future session export <id> [--out <path>]
future session abort <id>                # stop the active run, clear the queue
future session cancel <id> --run <run-id>
future session wait <id> [--run <run-id>] [--timeout <duration>]  # wait for a run and print its answer
future session approvals <id>
future session approve <id> <request-id> [--allow <glob>]
future session reject <id> <request-id>
future session delete <id>
```

`session info` reads the persisted journal (what the conversation contains and
what it has cost); `session status` reads the live agent (how the session is
configured *now*: the effective tool permission, the sandbox tier, context
occupancy, loaded context files and skills, active and queued runs, and pending
approvals). Only the second can answer a question about a session whose settings
differ from the global defaults. `session set` writes both groups — the recorded
options (`--model`, `--thinking`, `--cwd`, `--title`, `--parent`,
`--system-prompt`, `--append-system-prompt`) and the live ones (`--tools`,
`--no-tools`, `--no-builtin-tools`, `--permission`, `--sandbox`,
`--context-files`, `--auto-compact`, `--auto-retry`) — and `session status` reads
them back.
`abort`/`cancel`/`approve`/`reject` are the CLI counterpart of the TUI's
`/stop`, `/cancel` and `/approve`, so a run started from a script can also be
stopped from one.

Both system-prompt flags are stored with the session record rather than held
live: the run boundary rebuilds the prompt, so a value written only to the live
loop is gone before a request is sent. `--append-system-prompt` adds text after
the project context and workspace memory; `--system-prompt` replaces the
prompt's opening section (the built-in identity and tool rules), with the
context, memory and append still following it. Either is set at creation with
`future session new`, and an empty value restores the built-in prompt.
`session status` reports both plus whether project context is folded in,
`session status <id> --system-prompt` prints the whole prompt the next run would
send (add `--json` for it as an object), and `session info` reads the recorded
values back.

`future session history search --all --query "<text>"` searches across sessions
instead of one, so "have we ever discussed this" does not require knowing which
session first. It scans the most recently updated `--sessions` sessions (default
50) and reports `scannedSessions` and `truncated`, so a partial scan is visible
rather than silently reported as "never discussed". See
[Self-inspection](../../guide/self-inspection.md).

`future session transcript --session <id>` processes a whole conversation rather
than finding one passage: `--select` picks the slices (user, assistant, thinking,
tool-call, tool-result, session, compaction), `--tool` a single tool, `--input` /
`--output` a tool's arguments versus its result, `--paths` just the file paths
either mentions, and `--cursor` / `--limit` / `--max-bytes` window the output for
paging. `--counts` is the cheap first look at an unfamiliar session. Both
`session info` and `transcript` answer `--json`.

`set` changes those settings on an existing session — only the options you pass are
touched (`--parent ""` detaches). A parent records lineage only: no history is
copied (that is `fork`), and the parent must be an existing session.

A title or cwd set on a session that already has a record is written immediately;
for a session that has never run, it is stored with the session's first run. The
model and thinking level take effect on the running session at once and reach the
session record with the next run (they are part of the run snapshot). `future run`
only applies a cwd of its own when `--cwd` is passed, or when it creates the
session (a new session starts in the caller's directory); attaching to an existing
session with `--session`/`--continue` leaves the cwd set here as it is.

Session data is stored in the agent's SQLite database, `~/.future/agent/agent.db`.
The `~/.future/agent/sessions/` directory is retained only as a legacy JSONL import
source.

### `doctor` — environment diagnostics

```bash
future doctor
```

Checks login status, component installation, agent connectivity, OS sandbox availability, configuration, providers/models, sessions, and skills in one pass.

---

## Tips

- **macOS blocked it?** Verify the download/signature first. Trusted unsigned test builds have separate first-launch instructions; see [[FAQ]].
- **Connection failed?** Run `future doctor`; check Agent startup/compatibility/instance-lock errors and matching user/IPC environment. Open the desktop or start `future agent` if needed.

---

## See also

- [[Install FutureOS|Installation]] — where the tool ships.
- [[Skills]] — the same skills, managed from the app.
- [[FAQ]] — common issues.
