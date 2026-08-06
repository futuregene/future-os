# Build & Install

How to build and install FutureOS on macOS, Linux, and Windows — the agent
backend, the TUI/CLI frontends, the desktop GUI, the channel bridge, and the
loop control plane (`future-loop`).

> For *using* FutureOS (models, skills, running the agent), see the
> [README](../README.md).

## Prerequisites

Required on every platform for a full build (agent + TUI + CLI + GUI):

- **Rust** 1.97+ (pinned via `rust-toolchain.toml`)
- **Node.js** 24+ (see `.nvmrc`)
- **Bun** — required, not optional: the TUI build and CLI/GUI packaging use `bun build`
- Optional: **Python 3** — only for `make generate-models`
- Optional: **protoc** (Protocol Buffers compiler) — only for `make generate-proto`; generated code is checked in so normal builds don't need it

## Clone

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
```

## macOS

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

`scripts/build-macos-dmg.sh` builds the agent and CLI sidecars together with
the GUI. It automatically uses a single `Developer ID Application` identity
from the macOS Keychain and writes a `*-sign.dmg`; if no unambiguous identity
is available, it falls back to the normal DMG. Run it with `--help` for
certificate selection, output-directory and Apple notarization options.

## Linux (Debian/Ubuntu)

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

## Windows

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

## Loop control plane (`future-loop`)

The loop control plane lives in `orchestration/loop` and builds as a normal
workspace member:

```bash
cargo build -p future-loop                 # debug build → target/debug/future-loop
cargo build -p future-loop --release       # release build → target/release/future-loop
```

To install the CLI plus the `/future-loop` agent skill locally:

```bash
bash scripts/install-future-loop.sh        # CLI → ~/.local/bin/future-loop, skill → ~/.future/agent/skills/
bash scripts/install-future-loop.sh --release
```

Add `~/.local/bin` to your `PATH` if it isn't already, then verify:

```bash
future-loop status
```

See the [loop control plane guide](loop-control-plane.md) for what it does
and how to use it.

## Install skills (optional)

FutureOS includes a set of curated skills — specialized instructions for
common tasks like deep research, browser automation, document processing,
and more. These are maintained in the
[future-skills](https://github.com/futuregene/future-skills) repository:

```bash
make install-skills                          # symlink from the bundled skills/ submodule
# or install from the platform catalog:
future skills install                        # install all future-* skills (~13)
future init                                  # install skills and, on macOS/Linux, link local commands
```

> Skills are symlinked into `~/.future/agent/skills/` where the agent
> discovers them automatically. Use `future skills list` to see available
> skills and `future skills update` to upgrade.

## Verify

```bash
make test        # cargo test (agent + loop control plane)
make lint        # lint all (agent + channels + TUI + CLI + GUI)
```

## Development (from source)

Source builds use the repo Makefile from the repo root:

```bash
make build          # build all components (no system install)
make lint           # lint all (agent + channels + TUI + CLI + GUI)
make fmt            # cargo fmt (agent + channels)
make test           # cargo test (agent)
make clean          # remove build artifacts + installed binaries
```

### Proto

The canonical API is `proto/future.proto`. Generated Rust/TS code is checked
into the repo — normal builds don't touch it. After editing a `.proto` file,
regenerate:

```bash
make generate-proto          # agent + channels + TUI
```
