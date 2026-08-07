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

FutureOS gives you a unified AI agent experience across a terminal UI (TUI),
desktop app (GUI), CLI, and IM bots — on macOS, Linux, and Windows.
Write code, run research, manage files — from the terminal, from a chat app,
or from a native desktop window.

## Features

| Category | Details |
|---|---|
| **Multi-Interface** | Terminal UI (TUI), desktop app (GUI), CLI, IM bots — one agent, everywhere |
| **Model Flexibility** | 3800+ built-in models across 140+ providers ([catalog](docs/wiki/en/Models.md)); custom providers via `models.json`; scoped model lists |
| **Streaming & Thinking** | Real-time token streaming with collapsible reasoning-content blocks; configurable thinking levels (off ↔ xhigh) |
| **Tool Execution** | read, write, edit, shell with approval gating; sandbox tiers (off / manual / macOS Seatbelt) |
| **Session Persistence** | JSONL-based sessions with fork, clone, tree navigation, and query-count tracking ([using](docs/wiki/en/Using-FutureOS.md)) |
| **Skills System** | Pluggable YAML-defined skill bundles discovered from multiple directories ([guide](docs/wiki/en/Skills.md)) |
| **Compaction & Retry** | Automatic context compaction; exponential-backoff retry on context-length errors |
| **Loop Control Plane** | `future-loop`: durable goals/todos/gates/monitors, quota should-run kernel, event-sourced state, validators, extensions & multi-agent ([guide](docs/loop-control-plane.md)) — a Rust rewrite of the [loopx](https://github.com/huangruiteng/loopx) control plane, customized for FutureOS |
| **Rust Core** | Agent, IM channel bridge, loop control plane, CLI, and TUI are all written in Rust — high performance with memory safety |

## Quick Start

### Install

One line, no source build required:

**macOS** — installs the official signed app (arm64 / Intel auto-detected), then builds the `future-loop` control plane (CLI + skill) from source:

```bash
curl -fsSL https://raw.githubusercontent.com/futuregene/future-os/main/scripts/install.sh | bash
```

**Windows** (PowerShell) — runs the signed installer silently:

```powershell
iex (irm https://raw.githubusercontent.com/futuregene/future-os/main/scripts/install.ps1)
```

**Linux** — no prebuilt binaries yet; the script bootstraps the toolchain (apt deps + Rust + Node 24 + Bun) and builds the terminal stack (agent, TUI, CLI, IM channels, loop) from source:

```bash
curl -fsSL https://raw.githubusercontent.com/futuregene/future-os/main/scripts/install.sh | bash
```

Step-by-step installation for every platform (desktop app, toolchains, GUI
packaging) is in the **[Build & Install](docs/build-and-install.md)** guide.

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

The terminal and CLI clients are thin gRPC clients. **The agent must be running
first**, listening on `127.0.0.1:50051`:

```bash
future-agent      # start the agent in the terminal (logs to stdout; Ctrl-C to stop)
```

Then launch the terminal UI:

```bash
future-tui        # terminal UI
```

> A client that exits with a connection / gRPC error almost always means the agent isn't running yet — see [Troubleshooting](#troubleshooting).

### Essential Slash Commands (TUI)

| Command | Purpose |
|---|---|
| `/help` | Show all commands and shortcuts |
| `/model [name]` | Select / switch model |
| `/new` | Start a new session |
| `/sessions` | Browse and switch sessions |
| `/compact` | Compress conversation context |
| `/scoped-models` | Configure model enable/disable list |
| `/clone` | Clone the current session |
| `/fork` | Fork the current session |
| `/tree` | Session tree with fork/clone hierarchy |
| `/name [n]` | Set the session name |
| `/status` | Session state, token usage, cost |
| `/stop` | Abort current generation |
| `/cwd` | Change the working directory |
| `/approve` | Approve pending tool execution |
| `/reject` | Reject pending tool execution |
| `/cancel <run-id>` | Cancel a queued run |
| `/reload` | Reload skills and context |

### Keyboard Shortcuts (TUI)

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

## Troubleshooting

| Symptom | Fix |
|---|---|
| Client exits with a connection / gRPC error | The agent isn't running. Start it (`future-agent`) and check nothing else holds the port: `lsof -i :50051`. |
| Agent replies with an auth / "no model" error | No model configured yet. Run `future auth login`, or add a provider to `models.json` — see [Configure a model](#configure-a-model). |
| Build / install problems | See [Build & Install](docs/build-and-install.md) (platform toolchains, linker, GUI packaging). |

## License

MIT
