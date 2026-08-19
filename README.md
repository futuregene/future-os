<p align="center">
  <img src="docs/banner.png" alt="FutureOS" width="600">
</p>

<h3 align="center">One AI agent, everywhere you work.</h3>
<p align="center">
  Terminal, desktop, mobile, and your chat apps — one Rust core, one agent, 3,800+ models.<br>
  Every tool call gated by your approval. Local-first. Open source.
</p>

<p align="center">
  <img src="docs/tui-screenshot.png" alt="FutureOS terminal UI — built-in skills loaded, /help command palette" width="800">
</p>

<!-- TODO(demo): replace the static screenshot with a 60-90s demo GIF (TUI approval gate →
     desktop GUI → IM-bot progress → session fork tree → /future-loop board), saved as docs/demo.gif -->

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#features">Features</a> •
  <a href="#configure-a-model">3,800+ Models</a> •
  <a href="#essential-slash-commands-tui">Commands</a> •
  <a href="#troubleshooting">Troubleshooting</a>
</p>

<p align="center">⭐ If FutureOS is useful to you, a star helps others find it.</p>

<p align="center">
  <img src="https://img.shields.io/badge/Core-Rust-orange?style=for-the-badge&logo=rust" alt="Rust core">
  <a href="https://github.com/futuregene/future-os/blob/main/THIRD_PARTY_NOTICES.md"><img src="https://img.shields.io/badge/License-MIT_%2B_Apache--2.0-green?style=for-the-badge" alt="License: MIT + Apache-2.0"></a>
  <a href="README.zh-CN.md"><img src="https://img.shields.io/badge/Lang-中文-red?style=for-the-badge" alt="中文"></a>
</p>

---

## Why FutureOS

- **Trust before capability.** Every tool call — read, write, edit, shell — is gated by your approval by default. Nothing writes to your filesystem or runs a command silently. When an agent holds your credentials, trust can't be a config option.
- **One backend, every surface.** A single gRPC agent drives the terminal UI, desktop app, mobile apps, CLI, and IM bots — same sessions, same memory, same skills, wherever you happen to be.
- **Long runs need engineering, not prompting.** The built-in loop control plane gives durable goals, event-sourced state, and verification gates to runs of 24+ hours — start a research task at night, review the results from your phone in the morning.

## Features

| Category | Details |
|---|---|
| **Multi-Interface** | Terminal UI (TUI), desktop app (GUI), mobile apps (Android & iOS), CLI, IM bots — one agent, everywhere |
| **Trust-First Tool Execution** | read, write, edit, shell — every call gated by your approval; sandbox tiers (off / manual / macOS Seatbelt); a lean tool set, no prompt bloat |
| **Model Flexibility** | 3800+ built-in models across 140+ providers ([catalog](docs/wiki/en/Models.md)); custom providers via `models.json`; scoped model lists |
| **Loop Engineering** | Durable goals/todos/gates/monitors for long-horizon runs of 24+ hours — deterministic should-run kernel, event-sourced state, hard checks (evidence floor / acceptance contracts / verify gates), lease liveness, multi-agent ([guide](docs/loop-control-plane.md)) |
| **Powerful Built-in Skills** | 15+ skills out of the box for everyday agent work — image read & generation, PDF/Word parsing, web search, browser control, slides, software install, and the `/future-loop` long-run goal orchestrator ([builtin](https://github.com/futuregene/future-skills/tree/main/builtin)) |
| **Forkable Sessions** | Branch any conversation like a repo — fork, clone, and tree navigation over JSONL session history |
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

The agent needs at least one model with an API key before it can answer.

**FutureOS hosted models** — device-flow sign-in provisions keys and a model list automatically:

```bash
future auth login
```

<details>
<summary><strong>Prefer your own keys? (BYOK &amp; custom providers)</strong></summary>

**Use a known provider.** Put your API key in `~/.future/agent/auth.json`, keyed by provider name. See the [built-in model catalog](docs/wiki/en/Models.md) — most providers have built-in base URLs and auto-discover their models:

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

**Custom provider.** For providers not in the built-in catalog, specify everything in `~/.future/agent/models.json`:

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

</details>

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

## Community

[💬 Discussions](https://github.com/futuregene/future-os/discussions) • [🐛 Issues](https://github.com/futuregene/future-os/issues) • [🔒 Security](SECURITY.md) • [📖 Wiki](https://github.com/futuregene/future-os/wiki) • [Third-Party Notices](THIRD_PARTY_NOTICES.md)
