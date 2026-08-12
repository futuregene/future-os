#!/usr/bin/env bash
# FutureOS one-line installer
#
#   macOS / Linux:  curl -fsSL https://dl.future-os.cn/install.sh | bash
#   Windows:        iex (irm https://dl.future-os.cn/install.ps1)
#
# Auto-detects the OS and installs the prebuilt release — no local build:
#   - macOS            downloads the signed DMG, verifies its SHA-256 against
#                      the release manifest, and copies it to /Applications.
#   - Linux (Debian)   downloads the .deb and installs it with apt/dpkg.
#   - Linux (other)    downloads the portable tarball and extracts it to
#                      /usr/local/bin (or ~/.local/bin when not writable).
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
  Darwin) OS=macos ;;
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

# Resolve the version to install: FUTUREOS_VERSION wins, otherwise read the
# pointer from latest.json. A leading "v" is stripped — releases publish under
# the plain semver (releases/1.0.2/…), matching version.mjs tag handling.
VERSION=""
resolve_latest() {
  if [[ -z "${FUTUREOS_VERSION:-}" ]]; then
    fetch "$LATEST" > "$TMP/latest.json"
    VERSION="$(sed -n 's/^[[:space:]]*"version":[[:space:]]*"\([^"]*\)".*/\1/p' "$TMP/latest.json" | head -n1)"
    [[ -n "$VERSION" ]] || die "could not resolve the latest version from $LATEST"
  else
    VERSION="${FUTUREOS_VERSION#v}"
  fi
}

# Resolve the download URL+SHA-256 for one asset key (e.g. "darwin-aarch64").
# The pinned-version path constructs the URL directly and skips verification,
# since there is no manifest to check against.
ASSET_URL=""
ASSET_SHA=""
resolve_asset() {
  local key="$1" filename="$2"
  if [[ -z "${FUTUREOS_VERSION:-}" ]]; then
    read -r ASSET_URL ASSET_SHA <<< "$(manifest_asset "$filename")" || true
    [[ -n "$ASSET_URL" ]] || die "no release asset '$key' in $LATEST"
  else
    ASSET_URL="$BASE/$VERSION/$filename"
    ASSET_SHA=""
    warn "FUTUREOS_VERSION is pinned — skipping SHA-256 verification (no manifest lookup)"
  fi
}

install_macos() {
  local filename dmg mnt dmg_arch
  resolve_latest
  # Release artifacts use the short "x64" for Intel (FutureOS_<v>_x64-sign.dmg),
  # while latest.json asset keys use the Rust-style "x86_64".
  case "$ARCH" in
    x86_64)  dmg_arch="x64" ;;
    aarch64) dmg_arch="aarch64" ;;
    *) die "unsupported architecture: $ARCH" ;;
  esac
  filename="FutureOS_${VERSION}_${dmg_arch}-sign.dmg"
  resolve_asset "darwin-$ARCH" "$filename"
  say "Installing FutureOS $VERSION ($OS-$ARCH)"
  say "Downloading $ASSET_URL"
  dmg="$TMP/FutureOS.dmg"
  fetch "$ASSET_URL" > "$dmg"
  [[ -n "$ASSET_SHA" ]] && verify_sha256 "$dmg" "$ASSET_SHA"

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
  say "Done — FutureOS $VERSION installed"
  say "Launch the app with: open -a FutureOS"
  say "Terminal users: the unified 'future' CLI is bundled with the app (run 'future init' to link it into ~/.future/bin, then use 'future agent|tui|channel|loop')."
}

install_linux() {
  local filename key pkg pkg_type prefix DEB_ARCH
  resolve_latest

  if command -v dpkg >/dev/null 2>&1; then
    # Debian/Ubuntu — .deb package. Debian arch convention (amd64/arm64),
    # independent of the Rust host triple naming.
    case "$ARCH" in
      x86_64)  DEB_ARCH="amd64" ;;
      aarch64) DEB_ARCH="arm64" ;;
      *) die "unsupported architecture: $ARCH" ;;
    esac
    pkg_type="deb"
    key="linux-$ARCH"
    filename="FutureOS_${VERSION}_${DEB_ARCH}.deb"
    say "Debian-based system detected — installing the .deb package"
  else
    pkg_type="portable"
    key="linux-$ARCH-portable"
    filename="FutureOS-portable-linux.tar.gz"
    say "Non-Debian system detected — installing the portable tarball"
  fi

  resolve_asset "$key" "$filename"
  say "Installing FutureOS $VERSION ($OS-$ARCH, $pkg_type)"
  say "Downloading $ASSET_URL"
  pkg="$TMP/FutureOS.$pkg_type"
  fetch "$ASSET_URL" > "$pkg"
  [[ -n "$ASSET_SHA" ]] && verify_sha256 "$pkg" "$ASSET_SHA"

  if [[ "$pkg_type" == "deb" ]]; then
    if command -v apt-get >/dev/null 2>&1; then
      say "Installing with apt (resolves dependencies)"
      sudo apt-get install -y "$pkg"
    else
      say "Installing with dpkg"
      sudo dpkg -i "$pkg"
    fi
    say "Done — FutureOS $VERSION installed"
    say "Launch 'FutureOS' from your application menu"
  else
    if [[ -w /usr/local/bin ]]; then
      prefix="/usr/local/bin"
    else
      prefix="$HOME/.local/bin"
      mkdir -p "$prefix"
    fi
    say "Extracting portable tarball to $prefix"
    tar -xzf "$pkg" -C "$prefix"
    chmod +x "$prefix/futureos" "$prefix/future"
    if [[ ":$PATH:" != *":$prefix:"* ]]; then
      warn "Add $prefix to your PATH: export PATH=\"$prefix:\$PATH\""
    fi
    say "Done — FutureOS $VERSION installed"
    say "Run 'futureos' for the desktop app, 'future' for the CLI (agent|tui|channel|loop)"
  fi
}

case "$OS" in
  macos) install_macos ;;
  linux) install_linux ;;
esac
