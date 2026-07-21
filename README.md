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

Required on every platform for a full build (agent + TUI + CLI + GUI):

- **Rust** 1.96+ (pinned via `rust-toolchain.toml`)
- **Node.js** 24+ (see `.nvmrc`)
- **Bun** — required, not optional: the TUI build and CLI/GUI packaging use `bun build`
- Optional: **Python 3** — only for `make generate-models`
- Optional: **protoc** (Protocol Buffers compiler) — only for `make generate-proto`; generated code is checked in so normal builds don't need it

### Platform setup and build

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
```

#### macOS

Install dependencies:

```bash
xcode-select --install                                            # system toolchain (Tauri)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
brew install node oven-sh/bun/bun                                 # Node.js 24+ / Bun (nvm works too — see .nvmrc)
brew install protobuf                                             # optional — only for make generate-proto
```

Build:

```bash
make install        # build everything, install to /opt/homebrew/bin
make install-nogui  # terminal stack only (skip the Tauri GUI)
make package-gui    # desktop bundle → .app + .dmg in gui/src-tauri/target/release/bundle/
scripts/build-macos-dmg.sh  # local DMG; auto-signs when a Developer ID certificate is available
```

`scripts/build-macos-dmg.sh` builds the agent and CLI sidecars together with the
GUI. It automatically uses a single `Developer ID Application` identity from
the macOS Keychain and writes a `*-sign.dmg`; if no unambiguous identity is
available, it falls back to the normal DMG. Run it with `--help` for certificate
selection, output-directory and Apple notarization options.

#### Linux (Debian/Ubuntu)

Install dependencies:

```bash
sudo apt update
sudo apt install -y build-essential mold libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
curl -fsSL https://bun.sh/install | bash                          # Bun
# Node.js 24+ — `nvm install` reads the repo's .nvmrc
sudo apt install -y protobuf-compiler                             # optional — only for make generate-proto
```

> `mold` is required on x86_64 — `.cargo/config.toml` passes `-fuse-ld=mold` to the linker. ARM Linux doesn't need it.

Build:

```bash
make install        # build everything, install to /usr/local/bin (sudo)
make install-nogui  # terminal stack only (skip the Tauri GUI)
make package-gui    # desktop bundle → .deb in gui/src-tauri/target/release/bundle/
```

#### Windows

Install the toolchain:

1. **Visual Studio Build Tools** with the *Desktop development with C++* workload (MSVC + Windows SDK) — required by the Rust MSVC toolchain and Tauri. `winget install Microsoft.VisualStudio.2022.BuildTools`, then select the C++ workload in the installer (or install from [visualstudio.com](https://visualstudio.microsoft.com/downloads/)).
2. **Rust**: `winget install Rustlang.Rustup` (host triple `x86_64-pc-windows-msvc`)
3. **Node.js 24+**: `winget install OpenJS.NodeJS` or [nodejs.org](https://nodejs.org)
4. **Bun**: `winget install Oven-sh.Bun` (or `powershell -c "irm bun.sh/install.ps1 | iex"`)
5. **WebView2 Runtime**: ships with Windows 10/11 — a GUI *runtime* dependency, nothing to install on current systems

No `make` needed — the PowerShell commands below mirror the make targets step for step. Run them from the repo root.

**Terminal stack** — equivalent to `make install-nogui`:

```powershell
# Rust components: agent + channel bridge          (make build-agent / build-channels)
cargo build --release --manifest-path agent/Cargo.toml
cargo build --release --manifest-path channels/Cargo.toml

# TypeScript components: TUI + CLI                 (make build-tui / build-cli)
Push-Location tui; npm install; npm run gen-version; npm run build; bun build --compile dist/index.js --outfile dist/future-tui.exe; Pop-Location
Push-Location cli; npm install; npm run gen-version; npm run build; bun build --compile dist/index.js --outfile dist/future.exe --external chromium-bidi; Pop-Location

# Install to %USERPROFILE%\.future\bin             (the install-* copy steps)
$bin = "$env:USERPROFILE\.future\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item target\release\future-agent.exe, target\release\future-channel.exe, tui\dist\future-tui.exe, cli\dist\future.exe $bin

# Built-in skills — make install-skills uses symlinks; on Windows use the CLI instead
& "$bin\future.exe" skills install
```

**Desktop app** — the GUI half of `make install` (run after the terminal stack block above, which produces the sidecars):

```powershell
# Stage agent + CLI as Tauri sidecars, named with the host triple
$triple = (rustc -Vv | Select-String '^host:').Line.Split(' ')[1]
New-Item -ItemType Directory -Force -Path gui\src-tauri\binaries | Out-Null
Copy-Item target\release\future-agent.exe "gui\src-tauri\binaries\future-agent-$triple.exe"
Copy-Item cli\dist\future.exe "gui\src-tauri\binaries\future-$triple.exe"

# Build the app and install it as future-gui.exe   (make install-gui)
Push-Location gui; npm install; npx tauri build --no-bundle; Pop-Location
Copy-Item gui\src-tauri\target\release\futureos.exe "$env:USERPROFILE\.future\bin\future-gui.exe"
```

**Installer package** — equivalent to `make package-gui`, once the sidecars are staged:

```powershell
node scripts\version.mjs --set-bundle
Push-Location gui; npm run tauri:build; Pop-Location   # → NSIS setup .exe under gui\src-tauri\target\release\bundle\nsis\
```

Notes:

- `scripts\start-gui-test.bat` runs the GUI in dev mode against a locally built agent.
- The scripts under `scripts/` (`build-macos-dmg.sh`, `build-windows-portable.ps1`, `build-windows-installer.ps1`) wrap these same steps into a single command and replicate the CI packaging pipeline (DMG / portable zip / NSIS installer). They check the toolchain up front and require `protoc` (`brew install protobuf` / `choco install protoc`). Their artifacts contain the GUI, agent, and CLI — not the TUI.

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

### Install skills (optional)

FutureOS includes a set of curated skills — specialized instructions for common tasks like deep research, browser automation, document processing, and more. These are maintained in the [future-skills](https://github.com/futuregene/future-skills) repository and are our recommended defaults.

```bash
make install-skills                          # symlink from the bundled skills/ submodule
# or install from the platform catalog:
future skills install                        # install all future-* skills (~13)
```

> Skills are symlinked into `~/.future/agent/skills/` where the agent discovers them automatically. Use `future skills list` to see available skills and `future skills update` to upgrade.

### Run the agent

Every client — TUI, GUI, CLI, channels — is a thin gRPC client. **The agent must be running first**, listening on `127.0.0.1:50051`:

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
- **CLI** (`cli/`) — TypeScript. Auth (device-flow OAuth), one-shot prompts (`run`), MCP tool calls, skills management, environment diagnostics (`doctor`).
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
| GUI can't find the agent binary | `make install-gui` copies the agent sidecar using your host target triple. If your triple differs from the auto-detected one, copy it manually: `cp target/debug/future-agent gui/src-tauri/binaries/future-agent-$(rustc -vV | sed -n 's/^host: //p')`. |
| Build fails with "unable to find linker 'mold'" | Install mold: `sudo apt install mold` (Linux x86_64 only). ARM Linux doesn't need it. |
| GUI build fails on Linux (webkit / gtk errors) | Install the Tauri system dependencies — see [Linux setup](#linux-debianubuntu). |

## License

MIT
