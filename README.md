<p align="center">
  <a href="https://github.com/futuregene/future-os/wiki"><img src="https://img.shields.io/badge/Docs-Wiki-FFD700?style=for-the-badge" alt="Documentation"></a>
  <a href="https://github.com/futuregene/future-os/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="https://github.com/futuregene/future-skills"><img src="https://img.shields.io/badge/Skills-future--skills-blue?style=for-the-badge" alt="Skills"></a>
  <a href="README.zh-CN.md"><img src="https://img.shields.io/badge/Lang-中文-red?style=for-the-badge" alt="中文"></a>
</p>

<p align="center">
  <img src="docs/banner.png" alt="FutureOS" width="600">
</p>

# FutureOS

> A local-first AI agent workspace — terminal, desktop, messaging platforms, all through one backend.

FutureOS gives you a unified AI agent experience across TUI, GUI, CLI, Feishu, and DingTalk. The Rust backend handles LLM orchestration, tool execution, and persistent sessions. TypeScript frontends and a Tauri/React desktop app connect over gRPC. Write code, run research, manage files — from the terminal, from a chat app, or from a native desktop window.

## Features

| Category | Details |
|---|---|
| **Multi-Interface** | Terminal UI (TUI), Desktop app (GUI), CLI, Feishu bot, DingTalk bot — one agent, everywhere |
| **Model Flexibility** | 1000+ built-in models across 100+ providers ([full catalog](docs/wiki/en/Models.md)); custom providers via `models.json`; scoped model lists |
| **Streaming & Thinking** | Real-time token streaming with collapsible reasoning-content blocks; configurable thinking levels (off ↔ xhigh) |
| **Tool Execution** | read, write, edit, shell with approval gating; sandbox tiers (off / manual / macOS Seatbelt); auto-compaction at 90% context |
| **Session Persistence** | JSONL-based sessions with fork, clone, tree navigation, and query-count tracking |
| **Compaction & Retry** | Automatic context compaction; exponential-backoff retry on context-length errors |
| **Channel Bridge** | Feishu (Lark) and DingTalk bots — markdown streaming, slash commands, session management via chat |
| **Skills System** | Pluggable YAML-defined skill bundles discovered from multiple directories |
| **Cross-Platform** | macOS, Linux, Windows (GUI via Tauri + WebView2) |

## Quick Start

### Prerequisites

Required for a full `make build` (agent + TUI + CLI + GUI):

- **Rust** 1.96+ (pinned via `rust-toolchain.toml`)
- **Node.js** 24+ (see `.nvmrc`)
- **Bun** — required, not optional: the TUI build and CLI/GUI packaging use `bun build`
- **Linux only** (required for all builds):
  - `sudo apt install build-essential mold`
- **Tauri system dependencies** (for the GUI):
  - macOS: `xcode-select --install`
  - Linux (Debian/Ubuntu): `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf`
  - Windows: WebView2 Runtime (ships with Windows 10/11) + MSVC build tools
- Optional: **Python 3** — only for `make generate-models`
- Optional: **protoc** (Protocol Buffers compiler) — only for `make generate-proto`; generated code is checked in so normal builds don't need it
- Platform: macOS, Linux, or Windows

### Build and Install

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
make install   # build everything and install to system path
```

Binaries are installed to: macOS `/opt/homebrew/bin`, Linux `/usr/local/bin`, Windows `%USERPROFILE%\.future\bin`.

> **Terminal-only?** Skip the GUI toolchain: `make install-nogui`

### Configure a model

The agent needs at least one model with an API key before it can answer. Three options:

**A — FutureOS hosted models.** Device-flow sign-in provisions keys and a model list automatically:

```bash
future auth login
```

**B — Use a known provider.** Put your API key in `~/.future/agent/auth.json`, keyed by provider name. See the [built-in model catalog](docs/wiki/en/Models.md) for all supported providers — most have built-in base URLs and auto-discover their models:

```json
{
  "openai": { "type": "api_key", "key": "sk-..." }
}
```

For providers with user-specific base URLs (e.g. Azure's `YOUR_RESOURCE`), add a `baseUrl` field in `auth.json`:

```json
{
  "azure": { "type": "api_key", "key": "sk-...", "baseUrl": "https://my-resource.openai.azure.com/openai/v1" }
}
```

**C — Custom provider.** For providers not in the built-in catalog, specify everything in `~/.future/agent/models.json`:

```json
{
  "providers": {
    "my-provider": {
      "apiKey": "sk-...",
      "baseUrl": "https://my-api.example.com/v1",
      "models": [
        { "id": "my-model", "name": "My Model", "contextWindow": 128000 }
      ]
    }
  }
}
```

### Run the agent

Every client — TUI, GUI, CLI, channels — is a thin gRPC client. **The agent must be running first**, listening on `127.0.0.1:50051`. Two options:

| Mode | Command | Use when |
|---|---|---|
| **Foreground** | `make run-agent` | Builds and runs agent in terminal. Logs to stdout. Stop with Ctrl-C. |
| **Foreground** | `future-agent`  | Runs pre-built agent. Logs to stdout. Stop with Ctrl-C. |
Then launch a client:

```bash
future-tui           # terminal, after make install
future-gui           # desktop, after make install
# or in dev mode (builds first):
make run-tui         # terminal
make run-gui         # desktop
```

> A client that exits with a connection / gRPC error almost always means the agent isn't running yet — see [Troubleshooting](#troubleshooting).

### CLI Quick Start

```bash
future run "Write a Python sort function"    # one-shot prompt
future-tui                                   # open the TUI
future-gui                                   # launch the desktop app
future-channel                               # start the channel bridge
future --help                                # full command list
```

### Essential Slash Commands (TUI)

| Command | Purpose |
|---|---|
| `/help` | Show all commands and shortcuts |
| `/model <id>` | Switch model (e.g. `deepseek-v4-pro`) |
| `/status` | Session state, token usage, cost |
| `/sessions` | Browse and switch sessions |
| `/new` | Start a new session |
| `/stop` | Abort current generation |
| `/compact` | Compress conversation context |
| `/scoped-models` | Configure model enable/disable list |
| `/tree` | Session tree with fork/clone hierarchy |

### Keyboard Shortcuts (TUI)

| Key | Action |
|---|---|
| `ctrl+p` | Cycle model |
| `ctrl+t` | Cycle thinking level |
| `ctrl+r` | Browse sessions |
| `ctrl+c` | Interrupt / exit |
| `↑↓` | Scroll chat / navigate lists |
| `Tab` | Autocomplete |

## Architecture

```
                         ┌──────────────────────────┐
                         │   Rust Agent (gRPC)      │
                         │   LLM · tools · session  │
                         │   127.0.0.1:50051        │
                         └──────────┬───────────────┘
                                    │
        ┌───────────────┬───────────┴───────────┬───────────────┐
        │               │                       │               │
 TypeScript TUI   Tauri/React GUI       TypeScript CLI   Channel Bridge
 (terminal, bun) (desktop, WebView)     auth · MCP      Feishu · DingTalk
```

All clients connect to the agent independently over gRPC — no client depends on another.

- **Agent** (`agent/`) — Rust, tokio, tonic. LLM client (OpenAI-compatible HTTP+SSE), tool execution, session JSONL persistence, gRPC server.
- **TUI** (`tui/`) — TypeScript, bun. Differential rendering, markdown, Kitty image protocol, 14 UI components.
- **GUI** (`gui/`) — Tauri 2 + React + TypeScript. Three-panel layout (nav / chat / context), approval prompts, skill browser, settings.
- **CLI** (`cli/`) — TypeScript. Auth (device-flow OAuth), service management, MCP tool calls, TUI/GUI launcher.
- **Channel Bridge** (`channels/`) — Rust. Feishu (pbbp2 WebSocket + CardKit streaming) and DingTalk (Stream Mode).

## Configuration

All config under `~/.future/`:

| Path | Component | Purpose |
|---|---|---|
| `agent/settings.json` | Agent | Steering/follow-up mode, compaction, retry, max turns |
| `agent/auth.json` | Agent | API keys by provider (FutureOS + custom) |
| `agent/models.json` | Agent | Custom model overrides (base URL, API key, compat) |
| `agent/sessions/` | Agent | JSONL session files |
| `tui/settings.json` | TUI | Default model, thinking level, enabled model IDs |
| `app/app.db` | GUI | SQLite — threads, runs, artifacts, approvals, settings |
| `channels/config.json` | Channels | Agent gRPC address, Feishu/DingTalk credentials |

## Development

```bash
make build    # build all components (no system install)
make lint     # lint all (agent + channels + TUI + CLI + GUI)
make fmt      # cargo fmt (agent + channels)
make test     # cargo test (agent)
make clean    # remove build artifacts + installed binaries
```

### Proto

The canonical API is `proto/future.proto`. Generated Rust/TS code is checked into the repo — normal builds don't touch it. After editing a `.proto` file, regenerate:

```bash
make generate-proto          # agent + channels + TUI
```

## Troubleshooting

| Symptom | Fix |
|---|---|
| Client exits with a connection / gRPC error | The agent isn't running. Start it (`future-agent` or `make run-agent`) and check nothing else holds the port: `lsof -i :50051`. |
| Agent replies with an auth / "no model" error | No model configured yet. Run `future auth login`, or add a provider to `models.json` — see [Configure a model](#configure-a-model). |
| GUI can't find the agent binary | `make install-gui` copies the agent sidecar using your host target triple. If your triple differs from the auto-detected one, copy it manually: `cp agent/target/debug/future-agent gui/src-tauri/binaries/future-agent-$(rustc -vV | sed -n 's/^host: //p')`. |
| Build fails with "unable to find linker 'mold'" | Install mold: `sudo apt install mold` (Linux x86_64 only). ARM Linux doesn't need it. |
| GUI build fails on Linux (webkit / gtk errors) | Install the Tauri system dependencies — see [Prerequisites](#prerequisites). |

## License

MIT
