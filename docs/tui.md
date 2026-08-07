# FutureOS Terminal UI (TUI)

The TUI is the terminal client: `future-tui`. It is a thin gRPC client — the
**agent must be running first** (`future-agent`, listening on
`127.0.0.1:50051`). A client that exits with a connection / gRPC error almost
always means the agent isn't running yet.

```bash
future-agent      # terminal 1: the agent
future-tui        # terminal 2: the terminal UI
```

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
`~/.future/tui/keybindings.json`. Logs: `~/.future/tui/debug.log` (runtime
log, always written); with `PI_TUI_WRITE_LOG=1`, raw screen writes are
additionally logged to `~/.future/tui/write.log`.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Connection / gRPC error on startup | The agent isn't running. Start `future-agent` and check nothing else holds the port: `lsof -i :50051`. |
| Auth / "no model" error | No model configured. Run `future auth login`, or add a provider to `~/.future/agent/models.json` — see the repo README "Configure a model". |

See also: [Directory layout](directory-layout.md) for what lives under
`~/.future/`, and the wiki [CLI](wiki/en/CLI.md) / [Settings](wiki/en/Settings.md)
pages for the desktop-app equivalents.
