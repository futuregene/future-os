<p align="center">
  <a href="https://github.com/futuregene/future-os/tree/main/docs/wiki/en"><img src="https://img.shields.io/badge/Docs-Wiki-FFD700?style=for-the-badge" alt="Documentation"></a>
  <a href="https://github.com/futuregene/future-os/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
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

- **Rust** 1.80+
- **Node.js** 22+ / **Bun** (TUI runtime)
- macOS, Linux, or Windows

### Build & Run (60 seconds)

```bash
# Clone and build
git clone https://github.com/futuregene/future-os.git
cd future-os
make install   # install all dependencies
make build     # build agent + TUI + CLI + GUI

# Start the agent (gRPC server on 127.0.0.1:50051)
make run-agent &

# Launch a client
make run-tui    # terminal interface
make run-gui    # desktop app
```

### CLI Quick Start

```bash
future auth login                            # sign in
future run "Write a Python sort function"    # one-shot prompt
future tui                                   # open TUI
future agent start                           # start agent as a service (macOS launchctl / Linux systemd)
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
| `agent/auth.json` | API keys (FutureGene + custom providers) |
| `agent/models.json` | Custom model overrides (base URL, API key, compat) |
| `agent/sessions/` | JSONL session files (one per session) |
| `tui/settings.json` | Default model, thinking level, scoped model list |
| `channels/config.json` | Feishu/DingTalk credentials, agent address, channel defaults |

## Development

```bash
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

## License

MIT
