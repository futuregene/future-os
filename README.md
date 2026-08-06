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
| **Loop Control Plane** | `future-loop`: durable goals/todos/gates/monitors, quota should-run kernel, event-sourced state, validators, extensions & multi-agent ([guide](docs/loop-control-plane.md)) — a Rust rewrite of the [loopx](https://github.com/huangruiteng/loopx) control plane, customized for FutureOS |
| **Cross-Platform** | macOS, Linux, Windows (GUI via Tauri + WebView2) |

## Quick Start

### Install

Install FutureOS from the prebuilt installers or package scripts — no source
build required. Step-by-step installation for every platform (macOS / Linux /
Windows, desktop app, the `future-loop` control plane) is in the
**[Build & Install](docs/build-and-install.md)** guide.

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

Every client — TUI, GUI, CLI, channels — is a thin gRPC client. **The agent must be running first**, listening on `127.0.0.1:50051`:

```bash
future-agent      # start the agent in the terminal (logs to stdout; Ctrl-C to stop)
```

Then launch a client:

```bash
future-tui        # terminal
future-gui        # desktop
future-channel    # channel bridge
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

## Troubleshooting

| Symptom | Fix |
|---|---|
| Client exits with a connection / gRPC error | The agent isn't running. Start it (`future-agent`) and check nothing else holds the port: `lsof -i :50051`. |
| Agent replies with an auth / "no model" error | No model configured yet. Run `future auth login`, or add a provider to `models.json` — see [Configure a model](#configure-a-model). |
| Build / install problems | See [Build & Install](docs/build-and-install.md) (platform toolchains, linker, GUI packaging). |

## License

MIT
