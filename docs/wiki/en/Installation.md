# Install FutureOS

The desktop app runs on **macOS, Windows and Linux**. Android/iOS clients connect
to a running desktop through [[Remote]].

## Download

Use the official download channel or [GitHub Releases](https://github.com/futuregene/future-os/releases).
Choose the version and architecture that match your machine:

| System | Release artifacts |
|---|---|
| macOS arm64 / Intel | `.dmg` disk image |
| Windows x64 | Installer `.exe`, or portable `.zip` |
| Linux x86_64 / aarch64 | `.deb` (`amd64` / `arm64`), desktop portable tarball, or headless CLI tarball |

Linux release names include version and architecture:
`FutureOS_<version>_amd64.deb`, `FutureOS_<version>_arm64.deb`,
`FutureOS_<version>_linux_<arch>-portable.tar.gz`, and
`FutureOS_<version>_linux_<arch>-cli.tar.gz` (`arch` is `x86_64` or `aarch64`).
Use the actual downloaded filename, not the placeholders above. Local developer
builds can use different names.

Every desktop download includes the unified `future` CLI, which also runs the
agent, TUI and channel bridge. Linux CLI-only downloads do not include the GUI.
See [[CLI]] for usage.

### One-line installation

macOS/Linux:

```bash
curl -fsSL https://dl.future-os.cn/install.sh | bash
```

Windows PowerShell:

```powershell
iex (irm https://dl.future-os.cn/install.ps1)
```

On Debian/Ubuntu the Linux installer uses a matching `.deb`; other Linux systems
use the portable desktop package. The scripts finish with `future init`. For a
headless server, use the CLI-only tarball instead of installing desktop libraries.

## First launch and signing

Official release workflows sign macOS/Windows installers and notarize macOS
artifacts. Unsigned test builds are a separate channel and may trigger first-launch
warnings. Check the artifact/channel you downloaded; do not assume every build is
unsigned or treat an unexpected signature warning as harmless. See [[FAQ]].

### macOS

Open the `.dmg`, drag **FutureOS** into **Applications**, then launch it. If a
trusted unsigned test build is blocked, follow its included first-launch note.
For an unexpected warning on an official signed release, verify the source and
redownload before overriding security controls.

### Windows

- **Installer:** run the `.exe` and follow the prompts.
- **Portable:** extract the whole folder and run `FutureOS.exe`. Keep it beside
  `future.exe`; the app starts the background agent through that executable.
- The GUI requires **Microsoft Edge WebView2 Runtime**. Recent Windows 10/11
  systems usually have it; otherwise install Microsoft's Evergreen runtime.
- SmartScreen reputation warnings can occur even with a signed build. Verify the
  publisher and source before proceeding. For a trusted portable zip whose
  background service is blocked, use Properties → **Unblock** before extraction.

### Linux

- **Debian/Ubuntu:** install the downloaded package with
  `sudo apt install ./<downloaded-file>.deb`, then launch FutureOS from the app menu.
- **Portable desktop:** extract the downloaded tarball, keep `futureos` and
  `future` together, and run `./futureos`.
- **Headless CLI:** extract the CLI-only tarball and run `./future config`, then
  `./future tui` (starts its agent when needed), or start `./future agent` for CLI
  commands that require an agent. Add the executable directory to PATH as needed.

The published GUI packages require a recent glibc-based distribution (glibc ≥ 2.39,
roughly Ubuntu 24.04+) and WebKitGTK 4.1. Portable users may need
`sudo apt install libwebkit2gtk-4.1-0` or `sudo dnf install webkit2gtk4.1`.
Official Linux CI builds the CLI with static musl, so the CLI does not require
those GUI libraries. Local source builds depend on their selected target.

For sandbox mode, install **system Bubblewrap ≥ 0.9.0**, restart the app and run
`future agent --probe-sandbox` / `future doctor`. It is not bundled, and an older
distribution package may need a trusted upgrade. See [[Sandbox]] for requirements,
namespace restrictions and fallback behavior.

## Configure and use

Sign in inside the app to use FutureOS-hosted models, or configure your own
provider/key. See [[Quick Start|Quick-Start]]. The desktop defaults to
**Unrestricted**; select Manual/Sandboxed if needed before running tasks.

## Data, updates and uninstalling

Persistent data lives under `~/.future` on macOS/Linux or
`C:\Users\<you>\.future` on Windows. Models and online features still transmit
requests to their services; local storage does not mean offline-only processing.

Installer builds support **Settings → Check for updates** with signature-verified
updates. Linux `.deb` updates use the system package manager after verification.
Portable builds can be updated by replacing the extracted files. User data is
kept separately.

To uninstall, delete the macOS app, use Windows Settings (or delete its portable
folder), or run `sudo apt remove futureos` for Linux deb installs (delete the
portable/CLI files for manual installs). This does not require uninstalling system
Bubblewrap. Remove `.future` separately only if you intend to delete conversations,
credentials and settings; back up anything you need first.

See [[FAQ]], [[CLI]], [[Sandbox]] and [[Remote]].
