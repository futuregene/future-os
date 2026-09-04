# FutureOS Terminal UI (TUI)

The TUI is the terminal client: `future-tui`. It is a thin gRPC client that
connects over **per-user local IPC** by default (Unix-domain socket
`~/.future/run/agent.sock` on macOS/Linux, a current-user-only named pipe on
Windows). If no agent is reachable, the TUI launches one as a sidecar and
shuts it down on exit — no manual startup needed. You can still run the
agent yourself:

```bash
future agent      # terminal 1: the agent
future tui        # terminal 2: the terminal UI
```

For remote/development setups pass `--grpc-addr <host:port>` to the agent to
serve TCP instead, and set `FUTURE_AGENT_GRPC_ADDR=<host:port>` on clients
(explicit TCP is tried first, local IPC remains the fallback).

`future tui <args>` runs the TUI in-process; the standalone `future-tui`
binary is equivalent but no longer installed by default (build it with
`cargo build -p future-tui` if you need it). `future tui --help` lists all
options (print mode, `--list-models`, `--session`, ...).

- Build / install: see [Build & Install](build-and-install.md).
- Session persistence, model config and tool approval all run through the
  agent; the TUI is a front-end.

## Slash commands

All commands below are handled locally by the TUI (they are not sent to the
model). Command names are case-insensitive; `arg` is everything after the
command name.

| Command | Purpose |
|---|---|
| `/help` | Show the help overlay (shortcuts + core commands) |
| `/model [name]` | Set the model directly, or open the model selector with no arg |
| `/sessions` | Browse and switch sessions |
| `/new` | Start a new session |
| `/clone` | Clone the current session (continue in a new branch) |
| `/fork` | Fork from a chosen message |
| `/tree` | Session tree with fork/clone hierarchy |
| `/name <name>` | Set the session name |
| `/scoped-models` | Configure the model enable/disable list |
| `/compact` | Compress the conversation context |
| `/status` | Session state, model, token usage, cost |
| `/stop` | Stop the current generation |
| `/cwd <dir>` | Change the working directory |
| `/approve <request-id>` | Approve a pending tool execution |
| `/reject <request-id>` | Reject a pending tool execution |
| `/cancel <run-id>` | Cancel a queued run |
| `/reload` | Reload skills + context files |
| `/export` | *Not available in the TUI* (stub, replies with a notice) |
| `/import` | *Not available in the TUI* (stub, replies with a notice) |

> The in-app help overlay (`/help`) lists a subset of these commands; the full
> dispatch set above is authoritative (tui/src/app.rs `handle_submit`).

## Keyboard shortcuts

| Key | Action |
|---|---|
| `ctrl+p` | Cycle model |
| `ctrl+t` | Cycle thinking level |
| `ctrl+o` | Expand / collapse thinking |
| `ctrl+r` | Browse sessions |
| `ctrl+c` | Interrupt / exit |
| `tab` | Autocomplete |
| `enter` | Submit / accept |
| `escape` | Close popup |
| `↑↓` | Scroll / navigate lists |

## Settings & local files

The TUI persists client-side settings to `~/.future/tui/settings.json`
(e.g. `defaultModel`, `defaultThinkingLevel`, `defaultPermissionLevel`,
`enabledModelIds`). Optional user keybinding overrides can be placed at
`~/.future/tui/keybindings.json`. Logs: `PI_DEBUG_REDRAW=1` writes debug
redraw logging to `~/.future/tui/debug.log`; `PI_TUI_WRITE_LOG=1` logs raw
screen writes to `~/.future/tui/write.log`.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Connection / gRPC error on startup | No agent could be found or started. Check the sidecar error, or start `future agent` yourself. In TCP mode, check nothing else holds the port: `lsof -i :<port>`. |
| Auth / "no model" error | No model configured. Run `future auth login`, or add a provider to `~/.future/agent/models.json` — see the repo README "Configure a model". |

See also: [Directory layout](directory-layout.md) for what lives under
`~/.future/`, and the wiki [CLI](wiki/en/CLI.md) / [Settings](wiki/en/Settings.md)
pages for the desktop-app equivalents.
