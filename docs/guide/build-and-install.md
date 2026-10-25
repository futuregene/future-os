# Build & Install

All Rust programs share the root Cargo workspace, lockfile, `target/` directory,
and release profile (Thin LTO). Desktop builds compile the CLI and GUI in one
Cargo invocation, then stage the CLI and package the outputs. `make build-cli`
remains independent and does not compile GUI dependencies. On Linux, official
releases build the static musl CLI including `future host`; the GNU GUI
uses that prebuilt CLI via `FUTURE_DESKTOP_CLI`.

How to build and install FutureOS on macOS, Linux, and Windows — the agent
backend, the TUI/CLI frontends, the desktop GUI, the channel bridge, and the
goal control plane (`future-goal`).

> For *using* FutureOS (models, skills, running the agent), see the
> [README](../README.md).

## Prerequisites

Required on every platform for a full build (agent + TUI + CLI + GUI):

- **Rust** 1.97+ (pinned via `rust-toolchain.toml`)
- **Node.js** 24+ (see `.nvmrc`) — for the GUI frontend
- Optional: **Python 3** — for the manual/documentation targets (`make generate-models`, `make check-docs` / `make test-docs-check`, the `make test-cli-diff` / `make test-tui-tmux` harnesses) and the profiling targets (`make profile-agent` / `profile-quick` / `profile-heap`)
- Optional: **protoc** (Protocol Buffers compiler) — only for `make generate-proto`; generated code is checked in so normal builds don't need it

The TUI and CLI are Rust (`cargo build`) and no longer need Bun or Node.

## Clone

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
make setup    # JS deps (desktop/mobile) + skills submodule
```

`make setup` is a one-time bootstrap: it installs the shared `thread-projection`
package and the desktop/mobile JavaScript dependencies, initializes the `skills`
submodule. Rust checks do not require a staged CLI sidecar; the desktop build
stages the real CLI after compilation. Setup prepares the repository-level
dependencies; app run targets still
need the platform SDKs and a suitable emulator or device.

Local packaging uses the same public filenames as Actions. `<version>` is the artifact
version from `scripts/version.mjs` (for example `0.0.2-<hash>`), while installer
metadata uses its numeric bundle version. Windows outputs
`FutureOS_<version>_x64-setup[-unsigned].exe` and
`FutureOS_<version>_windows_x64-portable[-unsigned].zip`; `-Sign` removes
`-unsigned`. The installer and portable scripts copy final packages to the
repository root by default (`-OutDir` overrides it). `make package-desktop`
uses these platform scripts; on Linux it builds the host CLI before bundling.

### Distribution filenames

These are the final public filenames. `<V>` is the artifact version, `<B>` the
numeric bundle version, and `<N>` the build number. `<S>` is empty for signed
packages and `-unsigned` when platform code signing is absent. An architecture
written with “or” means either value, not literal filename text.

| Artifact | Filename | Producer |
|---|---|---|
| macOS DMG | `FutureOS_<V>_<aarch64 or x64><S>.dmg` | Actions, local platform script, Make |
| macOS updater | `FutureOS_<V>_<aarch64 or x64><S>.app.tar.gz` | Actions, with matching `.sig` |
| Windows installer | `FutureOS_<V>_x64-setup<S>.exe` | Actions, local platform script, Make; Actions also produces `.sig` |
| Windows portable | `FutureOS_<V>_windows_x64-portable<S>.zip` | Platform Actions, local portable script; excluded from coordinated publishing |
| Linux Debian | `FutureOS_<V>_<amd64 or arm64>.deb` | Actions, local platform script, Make; coordinated publishing also produces `.sig` |
| Linux portable | `FutureOS_<V>_linux_<x86_64 or aarch64>-portable.tar.gz` | Actions, local platform script, Make |
| Linux CLI | `FutureOS_<V>_linux_<x86_64 or aarch64>-cli.tar.gz` | Actions |
| Android APK | `FutureOS_<V>_android-universal.apk` | Actions |
| iOS IPA | `FutureOS_<B>_<N>.ipa` | Actions, uploaded only to TestFlight |
| Update manifest | `latest.json` | Release/test/nightly coordinators |

`.sig` is an updater signature, separate from macOS Developer ID / Windows
Authenticode code signing. Linux does not use `-unsigned`. Ordinary local
packaging does not generate updater archives or `.sig` files by default.

`npm run tauri:build`, `npm run desktop:package`, and the Tauri CLI are lower-level
build entrypoints. Their raw packages under `target/release/bundle/` use `<B>`;
they do not perform the platform scripts' final renaming and copying. Use
`make package-desktop` or a platform script for Actions-compatible final names.
Actions likewise builds raw bundles before staging the public names above.

Standalone desktop and Android Actions upload to `dev/<V>/`. Coordinated builds
publish to `test/<V>/`, `nightly/<V>/`, or `releases/<V>/`. Test/nightly `<V>` is
`0.0.2-<N>`, without the display version's `+test` / `+nightly` suffix.

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
make install        # GUI + unified `future` CLI (including `future host`) + skills (agent/tui/channel/goal are embedded) → /opt/homebrew/bin
make install-cli    # unified `future` CLI only
make install-desktop    # desktop app + matching CLI
make install-desktop-headless   # GUI-free CLI, including `future host`
make install-skills # built-in skills + the /future-goal skill
make package-desktop    # named DMG in the repository root; raw bundles in target/release/bundle/
scripts/build/build-desktop-macos.sh  # local DMG; auto-signs when a Developer ID certificate is available
```

`scripts/build/build-desktop-macos.sh` builds the unified `future` CLI sidecar together
with the GUI. It automatically uses a single `Developer ID Application` identity
from the macOS Keychain. Signed output is `FutureOS_<version>_<arch>.dmg`;
if no unambiguous identity is available, output ends in `-unsigned.dmg`. Run it with `--help` for
certificate selection, output-directory and Apple notarization options.

## Linux (Debian/Ubuntu)

### End users — prebuilt packages

The one-line installer downloads the prebuilt release (no local build):

```bash
curl -fsSL https://dl.future-os.cn/install.sh | bash
```

The script auto-detects the platform, installs the matching package from the
release manifest, verifies its SHA-256, then runs `future init`:

- **Debian/Ubuntu** — `FutureOS_<version>_amd64.deb` or `FutureOS_<version>_arm64.deb`, installed with `apt` (resolves dependencies). It installs graphical `futureos` and the unified `future` CLI (including `future host`).
- **Other Linux** — `FutureOS_<version>_linux_<arch>-portable.tar.gz` (`futureos` + unified `future` (including `future host`)), extracted to `/usr/local/bin` (or `~/.local/bin` when not writable). Architectures: `x86_64`, `aarch64`.
- **Host machines** — download `FutureOS_<version>_linux_<arch>-cli.tar.gz` from the official release channel, extract it, then run `./future config`, start the host with `./future host` (it needs no sign-in), and attach with `./future tui` in another terminal (`./future agent` also works as the host). No GUI libraries are needed for the official static musl CLI.

The published GUI needs glibc ≥ 2.39 (roughly Ubuntu 24.04+) and WebKitGTK 4.1.
The `future` CLI, including `future host`, is fully static
musl binaries with no glibc requirement, so they also run on older enterprise/HPC
systems (CentOS 7 / Rocky 8 era). Local developer builds use the selected host
target and need not be static.

Pin a specific release with `FUTUREOS_VERSION` (e.g. `FUTUREOS_VERSION=1.2.0`),
or point at a mirror with `FUTUREOS_BASE`. A pinned install reads that release's
own manifest (`releases/<version>/latest.json`) and verifies the artifact's
SHA-256 exactly like the latest-release path. Verification is mandatory: when
the pinned version publishes no manifest entry for the platform, or the host has
neither `shasum` nor `sha256sum`, the installer stops instead of installing an
unverified binary. `FUTUREOS_SKIP_VERIFY=1` skips the check for that run; use it
only when you already know the artifact is the one you want, because the script
then installs whatever the mirror served.

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
scripts/build/build-desktop-linux.sh --out-dir ./dist   # → ./dist/FutureOS_<version>_amd64.deb + FutureOS_<version>_linux_<arch>-portable.tar.gz
scripts/dev/start-desktop-linux.sh                    # local Linux Desktop + agent development session
make install        # or install straight from source: GUI + unified `future` CLI (including `future host`) + skills → /usr/local/bin (sudo)
make install-cli    # unified `future` CLI only
make install-desktop    # desktop app + matching CLI
make install-desktop-headless   # GUI-free CLI, including `future host`
make install-skills # built-in skills + the /future-goal skill
make package-desktop    # named .deb and portable archive in the repository root
```

`scripts/dev/start-desktop-linux.sh` starts GUI development with an embedded
Agent or attaches to a compatible external service. The launcher does not start
or stop a separate Agent. The existing Agent probe and execution path still
determine Bubblewrap availability and sandbox boundaries. See the
[Agent lifecycle/development guide](desktop-agent.md).

### Optional OS sandbox

Install trusted system Bubblewrap with runtime-probed capabilities (`sudo apt install bubblewrap`, or
`sudo dnf install bubblewrap` on Fedora), then fully restart FutureOS. Older
repository packages may offer write protection only; the selector suggests upgrading
for full protection. Basic write protection is required. It is not bundled with the app.
Check `future agent --probe-sandbox` and `future doctor`; user namespaces and fresh
`/proc` mounts must be permitted by host policy. See the [sandbox guide](../wiki/en/Sandbox.md)
for fallback and snapshot/credential/network limitations. Availability is not
certification of all distributions or architectures.

## Windows

Install the toolchain:

1. **Visual Studio Build Tools** with the *Desktop development with C++* workload (MSVC + Windows SDK) — required by the Rust MSVC toolchain and Tauri. `winget install Microsoft.VisualStudio.2022.BuildTools`, then select the C++ workload in the installer (or install from [visualstudio.com](https://visualstudio.microsoft.com/downloads/)).
2. **Rust**: `winget install Rustlang.Rustup` (host triple `x86_64-pc-windows-msvc`)
3. **Node.js 24+**: `winget install OpenJS.NodeJS` or [nodejs.org](https://nodejs.org)
4. **WebView2 Runtime**: ships with Windows 10/11 — a GUI *runtime* dependency, nothing to install on current systems

No `make` needed — the PowerShell commands below mirror the make targets step for step. Run them from the repo root.

**Terminal stack** — equivalent to `make install-cli install-skills`: only the unified
`future` CLI is needed (agent/tui/channel/goal are embedded in it); skills are
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

**Desktop app** — equivalent to `make install-desktop`. Build the CLI and GUI
together; the build script stages the sidecar after compilation:

```powershell
Push-Location desktop; npm install; npm run tauri:build -- --no-bundle; Pop-Location
$bin = "$env:USERPROFILE\.future\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item target\release\future.exe $bin
Copy-Item target\release\futureos.exe "$bin\futureos.exe"
```

**Host mode / CLI-only** — equivalent to `make install-desktop-headless`. The GUI-free CLI including the shared backend, installed in `%USERPROFILE%\.future\bin`:

```powershell
cargo build --release -p future-cli --bin future
$bin = "$env:USERPROFILE\.future\bin"
Copy-Item target\release\future.exe $bin
```

**Installer package** — equivalent to `make package-desktop`:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build/build-desktop-windows-installer.ps1   # → named NSIS setup .exe in the repository root
```

Notes:

- `scripts\dev\start-desktop-windows.bat` delegates to the PowerShell launcher; Desktop embeds Agent or connects to a compatible external service.
- The scripts under `scripts/build/` (`build-desktop-macos.sh`, `build-desktop-windows-portable.ps1`, `build-desktop-windows-installer.ps1`) wrap these same steps into a single command and replicate the CI packaging pipeline (DMG / portable zip / NSIS installer). They check the toolchain up front and require `protoc` (`brew install protobuf` / `choco install protoc`). Their artifacts contain the GUI and the unified `future` CLI (agent/TUI/channel/goal embedded) — not a separate TUI.

## Goal control plane (`future-goal`)

The goal control plane lives in `orchestration/goal` and builds as a normal
workspace member:

```bash
cargo build -p future-goal                 # debug build → target/debug/future-goal
cargo build -p future-goal --release       # release build → target/release/future-goal
```

To use it with the agent, link the `/future-goal` skill (no build needed —
the control plane runs through the unified `future` CLI):

```bash
make install-skills                    # built-in skills + the /future-goal skill → ~/.future/agent/skills/
```

Optionally install the standalone `future-goal` binary as well (dev use):

```bash
bash scripts/release/install-future-goal.sh        # CLI → ~/.local/bin/future-goal, skill → ~/.future/agent/skills/
bash scripts/release/install-future-goal.sh --release
```

Verify:

```bash
future goal status        # primary entry (same code as `future-goal status`)
```

> All Rust components are also reachable through the unified `future` CLI:
> `future agent`, `future tui`, `future channel`, `future goal` — each runs
> the same code as its standalone binary (`future-agent`, `future-tui`,
> `future-channel`, `future-goal`). The standalone binaries remain buildable
> with `cargo build -p <crate>` and runnable via `make run-*` (dev use); the
> standalone future-goal binary installs via scripts/release/install-future-goal.sh.

See the [goal control plane guide](../architecture/goal-control-plane.md) for what it does
and how to use it.

## Install skills (optional)

FutureOS includes a set of curated skills — specialized instructions for
common tasks like deep research, browser automation, document processing,
long-running goal orchestration (`/future-goal`), and more. These are
maintained in the [future-skills](https://github.com/futuregene/future-skills)
repository:

```bash
make install-skills                          # symlink from the bundled skills/ submodule
# or install from the platform catalog:
future skills install                        # install all current built-in future-* skills
future init                                  # install skills and, on macOS/Linux, link local commands
```

> Skills are symlinked into `~/.future/agent/skills/` where the agent
> discovers them automatically. Use `future skills list` to see available
> skills and `future skills update` to upgrade.

## Verify

```bash
make test        # all 8 suites: agent, channels, CLI, app-settings, TUI, GUI, GUI Rust, mobile
make lint        # lint all: agent, channels, TUI, CLI, GUI (+stylelint), mobile
```

## Development (from source)

Source builds use the repo Makefile from the repo root:

```bash
make build          # build GUI + unified CLI (no system install; agent/tui/channel/goal embedded in future, GUI stages its own sidecars)
make lint           # lint all: agent, channels, TUI, CLI, GUI (+stylelint), mobile
make fmt            # cargo fmt --all (the whole Rust workspace, desktop/src-tauri included)
make test           # all 8 suites: agent, channels, CLI, app-settings, TUI, GUI, GUI Rust, mobile
make clean          # remove build artifacts (use make uninstall for installed binaries)
```

### Start the desktop app from source

After the prerequisites and clone above, run:

```bash
make setup
make run-desktop
```

`make setup` installs the JavaScript workspaces and initializes built-in skills.
`make run-desktop` uses the platform launcher to build/stage the real CLI and
start Desktop in development mode; when no CLI artifact exists the launcher
writes a compile-only placeholder, which runtime tool discovery rejects.
Desktop embeds Agent when absent, so no
separate `future agent` is needed. (If a compatible
agent is already reachable through per-user local IPC or the explicit
`FUTURE_AGENT_GRPC_ADDR` override, the app reuses it instead and does not own its
shutdown.) Set `FUTURE_DESKTOP_AGENT_MODE=external` for attach-only debugging;
start the matching Agent separately with the same FutureOS home. Launchers do
not cancel old database rows or kill agents recorded by stale PID files.
`BUILD_CLI=0` skips the CLI build, `DESKTOP_DEV_PORT` selects the Vite port,
`RUN_CHECKS=1` runs static checks, and `DRY_RUN=1` only prints configuration.
Standalone Agent logs remain under `agent/logs/agent.log`; embedded Agent/host
logs use `app/logs/desktop.log`. See [directory layout](directory-layout.md) for socket selection.
Android and iOS need their respective SDKs and an emulator or device; see the
[mobile guide](../internals/mobile/README.md).

### Proto

The canonical API is `packages/rpc/proto/future.proto`. Generated Rust code is
checked into the repo — normal builds don't touch it. After editing a `.proto`
file, regenerate:

```bash
make generate-proto          # future-rpc + channels
```
