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

> One AI agent, everywhere you work — terminal, desktop, mobile, and your chat apps.

FutureOS gives you a unified AI agent experience across a terminal UI (TUI),
desktop app (GUI), mobile apps (Android & iOS), CLI, and IM bots — on
macOS, Linux, Windows, and your phone. Write code, run research, manage files —
from the terminal, from a chat app, from a native desktop window, or from a
phone in your pocket.

## Features

| Category | Details |
|---|---|
| **Multi-Interface** | Terminal UI (TUI), desktop app (GUI), mobile apps (Android & iOS), CLI, IM bots — one agent, everywhere |
| **Model Flexibility** | 3800+ built-in models across 140+ providers ([catalog](docs/wiki/en/Models.md)); custom providers via `models.json`; scoped model lists |
| **Agent Service** | The agent runs as a standalone gRPC service — the runtime is decoupled from the TUI, desktop app, mobile app, channel bridge, and loop control plane, leaving room for new clients and extensions |
| **Minimalist Tool Execution** | read, write, edit, shell with approval gating; sandbox tiers (off / manual / macOS Seatbelt) — Pi-style minimalism: a lean tool set, no prompt bloat |
| **Forkable Sessions** | Branch any conversation like a repo — fork, clone, and tree navigation over JSONL session history |
| **Powerful Built-in Skills** | 15+ skills out of the box for everyday agent work — image read & generation, PDF/Word parsing, web search, browser control, slides, software install, and the `/future-loop` long-run goal orchestrator ([builtin](https://github.com/futuregene/future-skills/tree/main/builtin)) |
| **Loop Engineering** | Durable goals/todos/gates/monitors for long-horizon runs of 24+ hours — deterministic should-run kernel, event-sourced state, hard checks (evidence floor / acceptance contracts / verify gates), lease liveness, multi-agent ([guide](docs/loop-control-plane.md)) |
| **Rust Core** | Agent, IM channel bridge, loop control plane, CLI, and TUI are all written in Rust — high performance with memory safety |

## Quick Start

### Install

One line, no source build required:

**macOS / Linux** — one script, auto-detects the platform: macOS gets the official signed app (arm64 / Intel); Linux gets the `.deb` on Debian/Ubuntu (desktop app + unified `future` CLI) or the portable tarball everywhere else:

```bash
curl -fsSL https://dl.future-os.cn/install.sh | bash
```

**Windows** (PowerShell) — runs the signed installer silently:

```powershell
iex (irm https://dl.future-os.cn/install.ps1)
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
future agent     # start the agent in the terminal (logs to stdout; Ctrl-C to stop)
```

Then launch the terminal UI from the same `future` command:

```bash
future tui       # terminal UI
```

> `future <cmd>` is the unified entry point for every Rust component: `future agent`,
> `future tui`, `future channel`, `future loop`. Each runs the same code as the
> standalone binary of the same name (the `future-*` binaries still exist as
> build targets — `cargo build -p future-tui` etc. — but are no longer
> installed by default).
>
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
| `ctrl+o` | Expand / collapse thinking |
| `ctrl+r` | Browse sessions |
| `ctrl+c` | Interrupt / exit |
| `tab` | Autocomplete |
| `enter` | Submit / accept |
| `escape` | Close popup |
| `↑↓` | Scroll / navigate lists |

## Troubleshooting

| Symptom | Fix |
|---|---|
| Client exits with a connection / gRPC error | The agent isn't running. Start it (`future agent`) and check nothing else holds the port: `lsof -i :50051`. |
| Agent replies with an auth / "no model" error | No model configured yet. Run `future auth login`, or add a provider to `models.json` — see [Configure a model](#configure-a-model). |
| Build / install problems | See [Build & Install](docs/build-and-install.md) (platform toolchains, linker, GUI packaging). |

## License

FutureOS is distributed under the [MIT License](LICENSE), **except**
[`orchestration/loop/`](orchestration/loop/) (Future Loop), which contains
code derived from [LoopX](https://github.com/huangruiteng/loopx) and is
distributed under the Apache License, Version 2.0 — see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
