#!/usr/bin/env bash
# FutureOS one-line installer
#
#   macOS / Linux:  curl -fsSL https://dl.future-os.cn/install.sh | bash
#   Windows:        iex (irm https://dl.future-os.cn/install.ps1)
#
# Auto-detects the OS, installs the prebuilt release, then runs `future init`
# — no local build:
#   - macOS            downloads the signed DMG, verifies its SHA-256 against
#                      the release manifest, and copies it to /Applications.
#   - Linux (Debian)   downloads the .deb and installs it with apt/dpkg.
#   - Linux (other)    downloads the portable tarball and extracts it to
#                      /usr/local/bin (or ~/.local/bin when not writable).
#
# Env overrides:
#   FUTUREOS_VERSION  pin a specific release (e.g. v0.1.2); default = latest
#   FUTUREOS_BASE     release mirror base URL; default https://dl.future-os.cn/releases
#   FUTUREOS_SKIP_VERIFY=1  install without checksum verification (not recommended;
#                     needed only when a release publishes no manifest or the
#                     host has neither shasum nor sha256sum)
set -euo pipefail

BASE="${FUTUREOS_BASE:-https://dl.future-os.cn/releases}"
LATEST="$BASE/latest.json"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/futureos.XXXXXX")"
MANIFEST="$TMP/latest.json"
trap 'rm -rf "$TMP"' EXIT

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m==>\033[0m %s\n' "$*" >&2; exit 1; }

# This script runs straight from `curl | bash`, so an unverified download has
# nothing left but TLS to the mirror (which FUTUREOS_BASE can repoint) and the
# CDN in front of it. Verification therefore fails closed: a release we cannot
# checksum is refused, and skipping the check is an explicit opt-in.
skip_verify_enabled() {
  case "${FUTUREOS_SKIP_VERIFY:-}" in
    ""|0|false|no) return 1 ;;
    *) return 0 ;;
  esac
}

require_verification_unless_skipped() { # reason
  if skip_verify_enabled; then
    warn "$1 — installing WITHOUT checksum verification (FUTUREOS_SKIP_VERIFY=1)"
    return 0
  fi
  die "$1 — refusing to install unverified content; set FUTUREOS_SKIP_VERIFY=1 to accept the risk"
}

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

# Read the version (no argument) or assets[key] URL+SHA-256 from the release
# manifest in $MANIFEST.
# Tokenize strings and nesting rather than relying on lines or field order.
# This only reads the release manifest string fields; no jq/Python is required.
manifest_entry() {
  awk -v asset_key="${1:-}" '
    {
      while (match($0, /"([^"\\]|\\.)*"|[][{}:,]/)) {
        token = substr($0, RSTART, RLENGTH)
        $0 = substr($0, RSTART + RLENGTH)
        if (token == "{" || token == "[") {
          path[++depth] = key
          key = ""
        } else if (token == "}" || token == "]") {
          delete path[depth--]
          key = ""
        } else if (token == ":") {
          key = value
        } else if (token == ",") {
          key = ""
        } else {
          value = substr(token, 2, length(token) - 2)
          gsub(/\\\//, "/", value)
          if (depth == 1 && key == "version") version = value
          if (depth == 3 && path[2] == "assets" && path[3] == asset_key) {
            if (key == "url") url = value
            if (key == "sha256") sha = value
          }
        }
      }
    }
    END {
      if (asset_key == "") print version
      else if (url != "") print url, sha
    }
  ' "$MANIFEST"
}

verify_sha256() { # file expected
  local file="$1" expected="$2" got=""
  if [[ -z "$expected" ]]; then
    require_verification_unless_skipped "no SHA-256 is published for $(basename "$file")"
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a 256 "$file" | awk '{print $1}')"
  elif command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$file" | awk '{print $1}')"
  else
    require_verification_unless_skipped "need shasum or sha256sum to verify the download"
    return 0
  fi
  if [[ "$got" != "$expected" ]]; then
    die "checksum mismatch — aborting (got $got, expected $expected)"
  fi
  say "Checksum verified"
}

# Initialize bundled skills/CLI links. Interactive provider setup
# (`future config`) is intentionally not run here yet — add it back right
# before the release ships so installers stay non-interactive until then.
run_future_setup() {
  local future_bin="$1"
  if [[ ! -x "$future_bin" ]]; then
    warn "bundled future CLI not found at $future_bin — run 'future init' and 'future config' manually"
    return 0
  fi

  say "Initializing FutureOS"
  if ! "$future_bin" init; then
    warn "future init did not complete — retry with: $future_bin init"
  fi
}

# Resolve the version to install and read its release manifest into $MANIFEST.
# FUTUREOS_VERSION wins, otherwise latest.json names the current version. A
# leading "v" is stripped — releases publish under the plain semver
# (releases/1.0.2/…), matching version.mjs tag handling. A pinned version reads
# that version's own manifest (releases/<version>/latest.json, published beside
# its artifacts), because a constructed URL carries no expected checksum.
VERSION=""
MANIFEST_READY=0
resolve_latest() {
  local url
  if [[ -z "${FUTUREOS_VERSION:-}" ]]; then
    url="$LATEST"
  else
    VERSION="${FUTUREOS_VERSION#v}"
    url="$BASE/$VERSION/latest.json"
  fi
  MANIFEST_READY=0
  if fetch "$url" > "$MANIFEST"; then
    MANIFEST_READY=1
    if [[ -z "$VERSION" ]]; then
      VERSION="$(manifest_entry)"
      [[ -n "$VERSION" ]] || die "could not resolve the latest version from $url"
    fi
  else
    [[ -n "$VERSION" ]] || die "could not resolve the latest version from $url"
  fi
}

# Resolve the download URL and its expected SHA-256 for one asset key (e.g.
# "darwin-aarch64") from the release manifest, falling back to the historical
# key when asked. A pinned release that publishes no manifest (or no entry for
# this platform) has no checksum to compare against, so it is refused unless
# FUTUREOS_SKIP_VERIFY=1 was set: an unverifiable install must be a deliberate
# choice, never the default.
ASSET_URL=""
ASSET_SHA=""
resolve_asset() {
  local key="$1" filename="$2" fallback_key="${3:-}" manifest_value=""
  if [[ "$MANIFEST_READY" -eq 1 ]]; then
    manifest_value="$(manifest_entry "$key")"
    if [[ -z "$manifest_value" && -n "$fallback_key" ]]; then
      manifest_value="$(manifest_entry "$fallback_key")"
    fi
    read -r ASSET_URL ASSET_SHA <<< "$manifest_value" || true
  fi
  if [[ -z "$ASSET_URL" ]]; then
    # A latest.json without the platform is a broken release, not something to
    # paper over with a guessed filename.
    [[ -z "${FUTUREOS_VERSION:-}" ]] && die "no release asset '$key' in $LATEST"
    require_verification_unless_skipped "$BASE/$VERSION/latest.json publishes no '$key' asset"
    ASSET_URL="$BASE/$VERSION/$filename"
    ASSET_SHA=""
    return 0
  fi
  [[ -n "$ASSET_SHA" ]] || require_verification_unless_skipped "the release manifest publishes no SHA-256 for '$key'"
}

# Preserve macOS bundle metadata when ditto is available. The fallback keeps
# this helper usable in the installer's offline shell regression tests.
copy_macos_app() { # source-app staged-app
  if command -v ditto >/dev/null 2>&1; then
    ditto "$1" "$2"
  else
    cp -R "$1" "$2"
  fi
}

# Stage a complete app bundle beside its destination, then swap it into place.
# The old bundle is moved only after staging succeeds and is restored if the
# final rename fails, avoiding a partially merged or half-copied application.
replace_macos_app() { # source-app target-app
  local source_app="$1" target_app="$2" target_parent stage_dir staged_app backup_app had_current=0
  target_parent="$(dirname "$target_app")"
  stage_dir="$(mktemp -d "$target_parent/.futureos-install.XXXXXX")" || return 1
  staged_app="$stage_dir/FutureOS.app"
  backup_app="$stage_dir/FutureOS.previous.app"

  if ! copy_macos_app "$source_app" "$staged_app"; then
    rm -rf "$stage_dir"
    return 1
  fi
  if [[ -e "$target_app" || -L "$target_app" ]]; then
    if ! mv "$target_app" "$backup_app"; then
      rm -rf "$stage_dir"
      return 1
    fi
    had_current=1
  fi
  if mv "$staged_app" "$target_app"; then
    rm -rf "$stage_dir"
    return 0
  fi

  if [[ "$had_current" -eq 1 ]]; then
    if mv "$backup_app" "$target_app"; then
      rm -rf "$stage_dir"
    else
      warn "failed to restore the previous app; it remains at $backup_app"
    fi
  else
    rm -rf "$stage_dir"
  fi
  return 1
}

install_macos() {
  local filename dmg mnt dmg_arch
  resolve_latest
  # Release artifacts use the short "x64" for Intel (FutureOS_<v>_x64.dmg),
  # while latest.json asset keys use the Rust-style "x86_64".
  case "$ARCH" in
    x86_64)  dmg_arch="x64" ;;
    aarch64) dmg_arch="aarch64" ;;
    *) die "unsupported architecture: $ARCH" ;;
  esac
  filename="FutureOS_${VERSION}_${dmg_arch}.dmg"
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
  if ! replace_macos_app "$mnt/FutureOS.app" "/Applications/FutureOS.app"; then
    hdiutil detach "$mnt" >/dev/null 2>&1 || true
    die "failed to replace FutureOS.app in /Applications; the previous app was preserved"
  fi
  hdiutil detach "$mnt" >/dev/null 2>&1 || true
  run_future_setup "/Applications/FutureOS.app/Contents/MacOS/future"
  say "Done — FutureOS $VERSION installed"
  say "Launch the app with: open -a FutureOS"
  say "Terminal users: add ~/.future/bin to PATH, then use 'future agent|tui|channel|goal'."
}

install_linux() {
  local filename key pkg pkg_type prefix DEB_ARCH deb_package future_bin
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
    key="linux-$ARCH-deb"
    filename="FutureOS_${VERSION}_${DEB_ARCH}.deb"
    say "Debian-based system detected — installing the .deb package"
  else
    pkg_type="portable"
    key="linux-$ARCH-portable"
    # Release filenames always carry the normalized Linux architecture.
    case "$ARCH" in
      x86_64)  filename="FutureOS_${VERSION}_linux_x86_64-portable.tar.gz" ;;
      aarch64) filename="FutureOS_${VERSION}_linux_aarch64-portable.tar.gz" ;;
      *) die "unsupported architecture: $ARCH" ;;
    esac
    say "Non-Debian system detected — installing the portable tarball"
  fi

  if [[ "$pkg_type" == "deb" ]]; then
    # Prefer the explicit package-format key. Fall back to the historical key
    # while old manifests may still be served by mirrors or pinned caches.
    resolve_asset "$key" "$filename" "linux-$ARCH"
  else
    resolve_asset "$key" "$filename"
  fi
  say "Installing FutureOS $VERSION ($OS-$ARCH, $pkg_type)"
  say "Downloading $ASSET_URL"
  pkg="$TMP/FutureOS.$pkg_type"
  fetch "$ASSET_URL" > "$pkg"
  [[ -n "$ASSET_SHA" ]] && verify_sha256 "$pkg" "$ASSET_SHA"

  if [[ "$pkg_type" == "deb" ]]; then
    deb_package="$(dpkg-deb -f "$pkg" Package 2>/dev/null || true)"
    if command -v apt-get >/dev/null 2>&1; then
      say "Installing with apt (resolves dependencies)"
      sudo apt-get install -y "$pkg"
    else
      say "Installing with dpkg"
      sudo dpkg -i "$pkg"
    fi
    future_bin=""
    if [[ -n "$deb_package" ]]; then
      future_bin="$(dpkg-query -L "$deb_package" 2>/dev/null | awk '/\/future$/ { print; exit }')"
    fi
    if [[ -z "$future_bin" ]] && command -v future >/dev/null 2>&1; then
      future_bin="$(command -v future)"
    fi
    run_future_setup "${future_bin:-/usr/lib/FutureOS/future}"
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
    # A portable prefix has no ownership manifest. Preserve any pre-existing
    # same-name file rather than guessing whether it belongs to this product.
    # The new archive and CLI do not use the retired separate headless entry.
    run_future_setup "$prefix/future"
    if [[ ":$PATH:" != *":$prefix:"* ]]; then
      warn "Add $prefix to your PATH: export PATH=\"$prefix:\$PATH\""
    fi
    say "Done — FutureOS $VERSION installed"
    say "Run 'futureos' for the desktop app, 'future' for the CLI (headless|agent|tui|channel|goal)"
  fi
}

case "$OS" in
  macos) install_macos ;;
  linux) install_linux ;;
esac
