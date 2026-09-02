# Build & Install

How to build and install FutureOS on macOS, Linux, and Windows — the agent
backend, the TUI/CLI frontends, the desktop GUI, the channel bridge, and the
loop control plane (`future-loop`).

> For *using* FutureOS (models, skills, running the agent), see the
> [README](../README.md).

## Prerequisites

Required on every platform for a full build (agent + TUI + CLI + GUI):

- **Rust** 1.97+ (pinned via `rust-toolchain.toml`)
- **Node.js** 24+ (see `.nvmrc`) — for the GUI frontend
- Optional: **Python 3** — only for `make generate-models` and the CLI golden-diff harness (`make test-cli-diff`)
- Optional: **protoc** (Protocol Buffers compiler) — only for `make generate-proto`; generated code is checked in so normal builds don't need it

The TUI and CLI are Rust (`cargo build`) and no longer need Bun or Node.

## Clone

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
make setup    # JS deps (desktop/mobile) + skills submodule + sidecar placeholder
```

`make setup` is a one-time bootstrap: it installs the shared `thread-projection`
package and the desktop/mobile JavaScript dependencies, initializes the `skills`
submodule, and creates an empty Tauri sidecar placeholder so `cargo check`/
`clippy`/`test` under `desktop/src-tauri` run even before the first `cargo
build`. It prepares the repository-level dependencies; app run targets still
need the platform SDKs and a suitable emulator or device.

## macOS

Install dependencies:

```bash
xcode-select --install                                            # system toolchain (Tauri)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
source "$HOME/.cargo/env"                                         # make cargo/rustc available in this shell
brew install node                                                 # Node.js 24+ (nvm works too — see .nvmrc)
brew install protobuf                                             # optional — only for make generate-proto
```

Build:

```bash
make install        # GUI + unified `future` CLI + skills (agent/tui/channel/loop are embedded) → /opt/homebrew/bin
make install-cli    # unified `future` CLI only
make install-desktop    # desktop app only (stages its own agent/CLI sidecars)
make install-skills # built-in skills + the /future-loop skill
make package-desktop    # desktop bundle → .app + .dmg in desktop/src-tauri/target/release/bundle/
scripts/build-desktop-macos.sh  # local DMG; auto-signs when a Developer ID certificate is available
```

`scripts/build-desktop-macos.sh` builds the unified `future` CLI sidecar together
with the GUI. It automatically uses a single `Developer ID Application` identity
from the macOS Keychain and writes a `*-sign.dmg`; if no unambiguous identity
is available, it falls back to the normal DMG. Run it with `--help` for
certificate selection, output-directory and Apple notarization options.

## Linux (Debian/Ubuntu)

### End users — prebuilt packages

The one-line installer downloads the prebuilt release (no local build):

```bash
curl -fsSL https://dl.future-os.cn/install.sh | bash
```

The script auto-detects the platform, installs the matching package from the
release manifest, verifies its SHA-256, then runs `future init`:

- **Debian/Ubuntu** — `FutureOS_<version>_amd64.deb`, installed with `apt` (resolves dependencies).
- **Every other Linux** — `FutureOS-portable-linux.tar.gz` (`futureos` desktop app + unified `future` CLI), extracted to `/usr/local/bin` (or `~/.local/bin` when not writable).

Pin a specific release with `FUTUREOS_VERSION` (e.g. `FUTUREOS_VERSION=1.2.0`),
or point at a mirror with `FUTUREOS_BASE`.

### Building from source (developers)

Install the toolchain:

```bash
sudo apt update
sudo apt install -y build-essential mold libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust (rust-toolchain.toml pins 1.97.0)
source "$HOME/.cargo/env"                                         # make cargo/rustc available in this shell
sudo apt install -y protobuf-compiler                             # optional — only for make generate-proto
```

> `mold` is required on x86_64 — `.cargo/config.toml` passes `-fuse-ld=mold` to the linker. ARM Linux doesn't need it.

Build and install:

```bash
scripts/build-desktop-linux.sh --out-dir ./dist   # → ./dist/FutureOS_<version>_amd64.deb + FutureOS-portable-linux.tar.gz
make install        # or install straight from source: GUI + unified `future` CLI + skills → /usr/local/bin (sudo)
make install-cli    # unified `future` CLI only
make install-desktop    # desktop app only (stages its own agent/CLI sidecars)
make install-skills # built-in skills + the /future-loop skill
make package-desktop    # desktop bundle → .deb in desktop/src-tauri/target/release/bundle/
```

## Windows

Install the toolchain:

1. **Visual Studio Build Tools** with the *Desktop development with C++* workload (MSVC + Windows SDK) — required by the Rust MSVC toolchain and Tauri. `winget install Microsoft.VisualStudio.2022.BuildTools`, then select the C++ workload in the installer (or install from [visualstudio.com](https://visualstudio.microsoft.com/downloads/)).
2. **Rust**: `winget install Rustlang.Rustup` (host triple `x86_64-pc-windows-msvc`)
3. **Node.js 24+**: `winget install OpenJS.NodeJS` or [nodejs.org](https://nodejs.org)
4. **WebView2 Runtime**: ships with Windows 10/11 — a GUI *runtime* dependency, nothing to install on current systems

No `make` needed — the PowerShell commands below mirror the make targets step for step. Run them from the repo root.

**Terminal stack** — equivalent to `make install-cli install-skills`: only the unified
`future` CLI is needed (agent/tui/channel/loop are embedded in it); skills are
installed by the CLI itself:

```powershell
# Rust CLI — the unified binary (make build-cli)
cargo build --release --manifest-path cli/Cargo.toml

# Install to %USERPROFILE%\.future\bin             (the install-cli copy step)
$bin = "$env:USERPROFILE\.future\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item target\release\future.exe $bin

# Built-in skills — make install-skills uses symlinks; on Windows use the CLI instead
& "$bin\future.exe" skills install
```

**Desktop app** — the GUI half of `make install` (stages its own sidecar,
`make desktop-sidecars` — only the unified `future` CLI; the desktop app starts the agent
via `future agent`):

```powershell
# Stage the unified CLI as the Tauri sidecar, named with the host triple
$triple = (rustc -Vv | Select-String '^host:').Line.Split(' ')[1]
New-Item -ItemType Directory -Force -Path desktop\src-tauri\binaries | Out-Null
Copy-Item target\release\future.exe "desktop\src-tauri\binaries\future-$triple.exe"

# Build the app and install it as future-desktop.exe   (make install-desktop)
Push-Location desktop; npm install; npx tauri build --no-bundle; Pop-Location
Copy-Item desktop\src-tauri\target\release\futureos.exe "$env:USERPROFILE\.future\bin\future-desktop.exe"
```

**Installer package** — equivalent to `make package-desktop`, once the sidecars are staged:

```powershell
node scripts\version.mjs --set-bundle
Push-Location desktop; npm run tauri:build; Pop-Location   # → NSIS setup .exe under desktop\src-tauri\target\release\bundle\nsis\
```

Notes:

- `scripts\start-desktop-windows.bat` runs the GUI in dev mode against a locally built agent.
- The scripts under `scripts/` (`build-desktop-macos.sh`, `build-desktop-windows-portable.ps1`, `build-desktop-windows-installer.ps1`) wrap these same steps into a single command and replicate the CI packaging pipeline (DMG / portable zip / NSIS installer). They check the toolchain up front and require `protoc` (`brew install protobuf` / `choco install protoc`). Their artifacts contain the GUI and the unified `future` CLI (agent/TUI/channel/loop embedded) — not a separate TUI.

## Loop control plane (`future-loop`)

The loop control plane lives in `orchestration/loop` and builds as a normal
workspace member:

```bash
cargo build -p future-loop                 # debug build → target/debug/future-loop
cargo build -p future-loop --release       # release build → target/release/future-loop
```

To use it with the agent, link the `/future-loop` skill (no build needed —
the control plane runs through the unified `future` CLI):

```bash
make install-skills                    # built-in skills + the /future-loop skill → ~/.future/agent/skills/
```

Optionally install the standalone `future-loop` binary as well (dev use):

```bash
bash scripts/install-future-loop.sh        # CLI → ~/.local/bin/future-loop, skill → ~/.future/agent/skills/
bash scripts/install-future-loop.sh --release
```

Verify:

```bash
future loop status        # primary entry (same code as `future-loop status`)
```

> All Rust components are also reachable through the unified `future` CLI:
> `future agent`, `future tui`, `future channel`, `future loop` — each runs
> the same code as its standalone binary (`future-agent`, `future-tui`,
> `future-channel`, `future-loop`). The standalone binaries remain buildable
> with `cargo build -p <crate>` and runnable via `make run-*` (dev use); the
> standalone future-loop binary installs via scripts/install-future-loop.sh.

See the [loop control plane guide](loop-control-plane.md) for what it does
and how to use it.

## Install skills (optional)

FutureOS includes a set of curated skills — specialized instructions for
common tasks like deep research, browser automation, document processing,
long-running goal orchestration (`/future-loop`), and more. These are
maintained in the [future-skills](https://github.com/futuregene/future-skills)
repository:

```bash
make install-skills                          # symlink from the bundled skills/ submodule
# or install from the platform catalog:
future skills install                        # install all future-* skills (15)
future init                                  # install skills and, on macOS/Linux, link local commands
```

> Skills are symlinked into `~/.future/agent/skills/` where the agent
> discovers them automatically. Use `future skills list` to see available
> skills and `future skills update` to upgrade.

## Verify

```bash
make test        # all 7 suites: agent, channels, CLI, TUI, GUI, GUI Rust, mobile
make lint        # lint all: agent, channels, TUI, CLI, GUI (+stylelint), mobile
```

## Development (from source)

Source builds use the repo Makefile from the repo root:

```bash
make build          # build GUI + unified CLI (no system install; agent/tui/channel/loop embedded in future, GUI stages its own sidecars)
make lint           # lint all: agent, channels, TUI, CLI, GUI (+stylelint), mobile
make fmt            # cargo fmt --all (workspace) + desktop/src-tauri + mobile formatting
make test           # all 7 suites: agent, channels, CLI, TUI, GUI, GUI Rust, mobile
make clean          # remove build artifacts (use make uninstall for installed binaries)
```

### Start the desktop app from source

After the prerequisites and clone above, run:

```bash
make setup
make run-desktop
```

`make setup` installs the JavaScript workspaces, initializes built-in skills, and
creates the Tauri sidecar placeholder. `make run-desktop` builds the local CLI
sidecar and starts the desktop app in development mode — the app auto-starts the
bundled agent sidecar on launch, so no separate `future agent` is needed. (If an
agent is already running on `127.0.0.1:50051`, the app reuses it instead.)
Android and iOS need their respective SDKs and an emulator or device; see the
[mobile guide](../mobile/README.md).

### Proto

The canonical API is `packages/rpc/proto/future.proto`. Generated Rust code is
checked into the repo — normal builds don't touch it. After editing a `.proto`
file, regenerate:

```bash
make generate-proto          # future-rpc + channels
```
