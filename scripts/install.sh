#!/usr/bin/env bash
# FutureOS one-line installer
#
#   macOS / Linux:  curl -fsSL https://raw.githubusercontent.com/futuregene/future-os/main/scripts/install.sh | bash
#   Windows:        iex (irm https://raw.githubusercontent.com/futuregene/future-os/main/scripts/install.ps1)
#
# - macOS  installs the official signed GUI app (DMG) for the detected arch,
#          verifies its SHA-256 against the release manifest, copies it to
#          /Applications, then builds and installs the `future-loop` control
#          plane (CLI + skill) from source.
# - Windows uses install.ps1 instead (iex (irm ...)); this script targets
#          macOS and Linux only.
# - Linux  has no prebuilt release binaries yet, so this script bootstraps the
#          toolchain (apt deps + Rust + Node 24 + Bun) and builds the terminal
#          stack (agent, TUI, CLI, channels, skills, loop) from source via
#          `make install-nogui`.
#
# Env overrides:
#   FUTUREOS_VERSION  pin a specific release (e.g. v0.1.2); default = latest
#   FUTUREOS_BASE     release mirror base URL; default https://dl.future-os.cn/releases
set -euo pipefail

BASE="${FUTUREOS_BASE:-https://dl.future-os.cn/releases}"
LATEST="$BASE/latest.json"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/futureos.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m==>\033[0m %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "curl is required (https://curl.se)"

case "$(uname -s)" in
  Darwin) OS=darwin ;;
  Linux)  OS=linux ;;
  *) die "unsupported OS: $(uname -s)" ;;
esac
case "$(uname -m)" in
  arm64|aarch64) ARCH=aarch64 ;;
  x86_64|amd64)  ARCH=x86_64 ;;
  *) die "unsupported architecture: $(uname -m)" ;;
esac

fetch() { curl -fsSL --proto '=https' --tlsv1.2 "$1"; }

# Asset URL+SHA-256 for one release file, read from the `assets` section of
# the pretty-printed release manifest (latest.json). Filename-keyed (e.g.
# "FutureOS_v0.1.2_aarch64-sign.dmg") -> "URL SHA256".
manifest_asset() {
  awk -v f="$1" '
    /"assets":/ { in_assets=1; next }
    in_assets && $0 ~ f {
      u=$0; getline; s=$0;
      gsub(/^.*"url": *"/,"",u); gsub(/".*$/,"",u);
      gsub(/^.*"sha256": *"/,"",s); gsub(/".*$/,"",s);
      print u, s; exit
    }
  ' "$TMP/latest.json"
}

verify_sha256() { # file expected
  local file="$1" expected="$2" got=""
  if command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a 256 "$file" | awk '{print $1}')"
  elif command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$file" | awk '{print $1}')"
  else
    warn "no sha256 tool found — skipping checksum verification"
    return 0
  fi
  if [[ "$got" != "$expected" ]]; then
    die "checksum mismatch — aborting (got $got, expected $expected)"
  fi
  say "Checksum verified"
}

install_macos() {
  local version="${FUTUREOS_VERSION:-}" url="" sha="" dmg="" mnt=""
  if [[ -z "$version" ]]; then
    fetch "$LATEST" > "$TMP/latest.json"
    version="$(sed -n 's/^[[:space:]]*"version":[[:space:]]*"\([^"]*\)".*/\1/p' "$TMP/latest.json" | head -n1)"
    [[ -n "$version" ]] || die "could not resolve the latest version from $LATEST"
  fi
  local filename="FutureOS_${version}_${ARCH}-sign.dmg"
  url="$BASE/$version/$filename"
  if [[ -z "${FUTUREOS_VERSION:-}" ]]; then
    read -r url sha <<< "$(manifest_asset "$filename")" || true
    [[ -n "$url" ]] || die "no release asset $filename in $LATEST"
  else
    warn "FUTUREOS_VERSION is pinned — skipping SHA-256 verification (no manifest lookup)"
  fi
  say "Installing FutureOS $version ($OS-$ARCH)"
  say "Downloading $url"
  dmg="$TMP/FutureOS.dmg"
  fetch "$url" > "$dmg"
  [[ -n "$sha" ]] && verify_sha256 "$dmg" "$sha"

  mnt="$TMP/mnt"
  mkdir -p "$mnt"
  hdiutil attach -nobrowse -quiet -mountpoint "$mnt" "$dmg" || die "failed to mount the DMG"
  if [[ ! -d "$mnt/FutureOS.app" ]]; then
    hdiutil detach "$mnt" >/dev/null 2>&1 || true
    die "the DMG does not contain FutureOS.app"
  fi
  say "Copying FutureOS.app to /Applications"
  if ! cp -R "$mnt/FutureOS.app" /Applications/; then
    hdiutil detach "$mnt" >/dev/null 2>&1 || true
    die "failed to copy FutureOS.app to /Applications"
  fi
  hdiutil detach "$mnt" >/dev/null 2>&1 || true
  say "FutureOS.app installed in /Applications"
  install_future_loop
  say "Done — FutureOS $version installed"
  say "Launch the app with: open -a FutureOS"
}

# The GUI app bundles the agent + CLI sidecars but not the loop control plane;
# build `future-loop` (CLI + skill) from source like the Linux path does.
install_future_loop() {
  command -v git >/dev/null 2>&1 || die "git is required (Xcode Command Line Tools: xcode-select --install)"
  if ! command -v cargo >/dev/null 2>&1; then
    say "Installing Rust via rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
  local src="$HOME/future-os"
  if [[ ! -d "$src/.git" ]]; then
    say "Cloning https://github.com/futuregene/future-os -> $src"
    git clone --depth 1 https://github.com/futuregene/future-os.git "$src"
  else
    say "Updating $src"
    git -C "$src" fetch --depth 1 origin
    git -C "$src" reset --hard origin/main
  fi
  say "Building future-loop (cargo build -p future-loop) — this can take a few minutes"
  bash "$src/scripts/install-future-loop.sh"
  say "future-loop installed: $HOME/.local/bin/future-loop (add ~/.local/bin to PATH if needed)"
}

install_linux() {
  say "Linux has no prebuilt release binaries yet — building the terminal stack from source"
  say "This installs: agent (future-agent), TUI (future-tui), CLI (future), channel bridge, skills, loop control plane (future-loop)"

  # 1. System build dependencies (Debian/Ubuntu).
  if command -v apt-get >/dev/null 2>&1; then
    local pkgs="build-essential pkg-config libssl-dev curl git"
    [[ "$ARCH" == "x86_64" ]] && pkgs="$pkgs mold"   # .cargo/config.toml pins mold on x86_64
    say "Installing build dependencies: $pkgs"
    sudo apt-get update -qq
    sudo apt-get install -y -qq $pkgs
  else
    die "source install currently supports Debian/Ubuntu only — see docs/build-and-install.md for other distros"
  fi

  # 2. Rust (repo pins the exact toolchain via rust-toolchain.toml).
  if ! command -v cargo >/dev/null 2>&1; then
    say "Installing Rust via rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    export PATH="$HOME/.cargo/bin:$PATH"
  fi

  # 3. Node.js 24 (repo pins .nvmrc = 24) via nvm when missing or too old.
  if ! command -v node >/dev/null 2>&1 || [[ "$(node -v 2>/dev/null | tr -dc '0-9' | cut -c1-2)" -lt 24 ]]; then
    if [[ ! -s "$HOME/.nvm/nvm.sh" ]]; then
      say "Installing nvm"
      curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
    fi
    export NVM_DIR="$HOME/.nvm"
    # shellcheck disable=SC1091
    \. "$NVM_DIR/nvm.sh"
    say "Installing Node.js 24"
    nvm install 24 >/dev/null
    nvm alias default 24 >/dev/null
    nvm use default >/dev/null
  fi

  # 4. Bun (required for the TUI/CLI single-binary builds).
  if ! command -v bun >/dev/null 2>&1; then
    say "Installing Bun"
    curl -fsSL https://bun.sh/install | bash
    export PATH="$HOME/.bun/bin:$PATH"
  fi

  # 5. Clone (or update) and build the terminal stack.
  local src="$HOME/future-os"
  if [[ ! -d "$src/.git" ]]; then
    say "Cloning https://github.com/futuregene/future-os -> $src"
    git clone --depth 1 https://github.com/futuregene/future-os.git "$src"
  else
    say "Updating $src"
    git -C "$src" fetch --depth 1 origin
    git -C "$src" reset --hard origin/main
  fi

  say "Building and installing (make install-nogui) — this can take a while"
  ( cd "$src" && make install-nogui )

  say "Done — FutureOS terminal stack installed"
  say "Run: future-agent (agent)  ·  future-tui (terminal UI)  ·  future (CLI)  ·  future-loop (control plane)"
}

case "$OS" in
  darwin) install_macos ;;
  linux)  install_linux ;;
esac
