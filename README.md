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
| **Model Flexibility** | 906+ built-in models (OpenAI, Anthropic, DeepSeek, Qwen, …); custom providers via `models.json`; scoped model lists |
| **Streaming & Thinking** | Real-time token streaming with collapsible reasoning-content blocks; configurable thinking levels (off ↔ xhigh) |
| **Tool Execution** | read, write, edit, bash with approval gating; sandbox tiers (off / manual / macOS Seatbelt); auto-compaction at 90% context |
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
- **protoc** (Protocol Buffers compiler) — required by the GUI and channels proto codegen
  - macOS: `brew install protobuf`
  - Linux (Debian/Ubuntu): `sudo apt install protobuf-compiler`
  - Windows: `choco install protoc` (or download from the protobuf releases)
- **Tauri system dependencies** (for the GUI):
  - macOS: `xcode-select --install`
  - Linux (Debian/Ubuntu): `sudo apt install build-essential libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev libayatana-appindicator3-dev patchelf`
  - Windows: WebView2 Runtime (ships with Windows 10/11) + MSVC build tools
- Optional: **Python 3** — only for `make generate-models`
- Platform: macOS, Linux, or Windows

> **Terminal-only build?** To skip the GUI/Tauri toolchain, build just the terminal stack:
> `make build-agent && make build-tui && make build-cli`.
>
> **Note:** `make install` builds standalone binaries and installs them to your system path:
> macOS `/opt/homebrew/bin`, Linux `/usr/local/bin`, Windows `%USERPROFILE%\.future\bin`.

### Build

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
make install   # build & install standalone binaries (future, future-tui, future-gui, future-channel)
make build     # dev build — agent + TUI + CLI + GUI (no system install)
```

### Run the agent (start this first)

Every client — TUI, GUI, CLI, channels — is a thin gRPC client. **The agent must be running first**, listening on `127.0.0.1:50051`. There are two ways to start it, for two different situations:

| Mode | Command | Use when |
|---|---|---|
| **Dev / foreground** | `make run-agent` | Hacking on the agent. Rebuilds from source, runs in your terminal, logs to stdout, stops on Ctrl-C. |
| **Background service** | `future agent start` | Daily use. Installed as a managed service (macOS launchctl / Linux systemd / Windows sc), survives reboots, started once. Manage with `future agent stop \| restart \| status`. |

Pick one — you don't need both. Then launch a client:

```bash
make run-tui     # terminal interface   (or: future tui)
make run-gui     # desktop app
```

> A client that exits with a connection / gRPC error almost always means the agent isn't running yet — see [Troubleshooting](#troubleshooting).

### Configure a model

The agent needs at least one model with an API key before it can answer. Two options:

**A — FutureOS hosted models.** Device-flow sign-in provisions keys and a model list automatically:

```bash
future auth login
```

**B — Bring your own key.** Point the agent at any OpenAI-compatible provider via `~/.future/agent/models.json`:

```json
{
  "providers": {
    "openai": {
      "apiKey": "sk-...",
      "baseUrl": "https://api.openai.com/v1",
      "models": [
        { "id": "gpt-4o", "name": "GPT-4o", "contextWindow": 128000 }
      ]
    }
  }
}
```

`baseUrl` has built-in defaults for `openai`, `anthropic`, `google`, `deepseek`, `openrouter`, and `dashscope`, so you can omit it for those. To keep secrets out of `models.json`, put keys in `~/.future/agent/auth.json` instead, keyed by provider:

```json
{
  "openai": { "type": "api_key", "key": "sk-..." }
}
```

Switch the active model any time with `/model <id>` in the TUI, or `ctrl+p` to cycle.

### CLI Quick Start

```bash
future auth login                            # sign in to hosted models
future run "Write a Python sort function"    # one-shot prompt (needs the agent running)
future tui                                   # open the TUI
future gui                                   # launch the desktop app
future agent start                           # run the agent as a background service
future channel start                         # start the channel bridge
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
          ┌─────────────────────────┼─────────────────────────┐
          │                         │                         │
   TypeScript TUI           Channel Bridge             TypeScript CLI
   (terminal, bun)          Feishu · DingTalk          auth · MCP · skills · run
                             
   Tauri / React GUI
   (desktop, WebView)
```

All clients connect to the agent independently over gRPC — no client depends on another.

- **Agent** (`agent/`) — Rust, tokio, tonic. LLM client (OpenAI-compatible HTTP+SSE), tool execution, session JSONL persistence, gRPC server.
- **TUI** (`tui/`) — TypeScript, bun. Differential rendering, markdown (via marked), Kitty image protocol, 14 UI components.
- **GUI** (`gui/`) — Tauri 2 + React + TypeScript. Standalone gRPC client. Three-panel layout (nav / chat / context), approval prompts, skill browser, settings.
- **CLI** (`cli/`) — TypeScript. Auth (device-flow OAuth), service management, MCP tool calls, TUI launcher.
- **Channel Bridge** (`channels/`) — Rust. Feishu (pbbp2 WebSocket + CardKit streaming) and DingTalk (Stream Mode).

## Configuration

All config under `~/.future/`:

| Path | Purpose |
|---|---|
| `agent/settings.json` | Steering/follow-up mode, compaction, retry, permission level |
| `agent/auth.json` | API keys (FutureOS + custom providers) |
| `agent/models.json` | Custom model overrides (base URL, API key, compat) |
| `agent/sessions/` | JSONL session files (one per session) |
| `tui/settings.json` | Default model, thinking level, scoped model list |
| `channels/config.json` | Feishu/DingTalk credentials, agent address, channel defaults |

## Development

```bash
make build-channels  # channel bridge — built separately; `make install` includes it
make lint     # lint all (agent clippy + channels clippy + TUI tsc + CLI tsc + GUI eslint)
make fmt      # cargo fmt (agent + channels)
make test     # cargo test (agent)
make clean    # remove all build artifacts
```

### Proto

The canonical API is `proto/future.proto`. Generated code updates automatically on build via `build.rs` (tonic-build). For manual regeneration:

```bash
make generate-proto          # agent + channels
cd tui && npm run generate-proto  # TUI embedded proto
```

## Troubleshooting

| Symptom | Fix |
|---|---|
| Client exits with a connection / gRPC error | The agent isn't running. Start it (`make run-agent` or `future agent start`) and check nothing else holds the port: `lsof -i :50051`. |
| Build fails with a `protoc` not-found error | Install the Protocol Buffers compiler — see [Prerequisites](#prerequisites). |
| Agent replies with an auth / "no model" error | No model configured yet. Run `future auth login`, or add a provider to `models.json` — see [Configure a model](#configure-a-model). |
| GUI can't find the agent binary | `make install-gui` copies the agent sidecar using your host target triple. If your triple differs from the auto-detected one, copy it manually: `cp agent/target/debug/future-agent gui/src-tauri/binaries/future-agent-$(rustc -vV | sed -n 's/^host: //p')`. |
| GUI build fails on Linux (webkit / gtk errors) | Install the Tauri system dependencies — see [Prerequisites](#prerequisites). |

## License

MIT
