#!/usr/bin/env bash
#
# Build the FutureOS Linux packages locally: the Tauri .deb plus the portable
# tarball. Linux packages are unsigned — there is no code-signing equivalent —
# so this script has none of the macOS signing/notarization machinery.
#
# Produces:
#   FutureOS_<artifact-version>_<arch>.deb  (futureos + future)
#   FutureOS_<artifact-version>_linux_<arch>-portable.tar.gz

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SKIP_DEPS=false
OUT_DIR="$ROOT"

usage() {
  cat <<'EOF'
Usage: scripts/build/build-desktop-linux.sh [options]

Build the unified `future` CLI, graphical Desktop, and standalone headless
Desktop, then produce the Linux distribution packages (.deb + portable tarball).

Options:
  --skip-deps              Skip npm ci in desktop/.
  --out-dir DIR            Copy the final packages to DIR (default: repository root).
  -h, --help               Show this help.
EOF
}

fail() {
  echo "error: $*" >&2
  exit 1
}

require_tool() {
  local command_name="$1"
  local hint="$2"
  command -v "$command_name" >/dev/null 2>&1 || fail "missing '$command_name'. $hint"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-deps)
      SKIP_DEPS=true
      shift
      ;;
    --out-dir)
      [[ $# -ge 2 ]] || fail "--out-dir requires a directory"
      OUT_DIR="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown option '$1' (use --help)"
      ;;
  esac
done

[[ "$(uname -s)" == "Linux" ]] || fail "this script must run on Linux"

cd "$ROOT"

echo "==> Checking prerequisites"
require_tool node "Install Node.js 24+ (https://nodejs.org)."
require_tool npm "npm is included with Node.js."
require_tool cargo "Install Rust (https://rustup.rs)."
require_tool rustc "Install Rust (https://rustup.rs)."
require_tool dpkg-deb "Install the Debian package tools."

TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
[[ -n "$TRIPLE" ]] || fail "could not read the host triple from rustc -Vv"

VERSIONS="$(node scripts/version.mjs --json)"
FUTURE_VERSION="$(node -p 'JSON.parse(process.argv[1]).version' "$VERSIONS")"
ARTIFACT_VERSION="$(node -p 'JSON.parse(process.argv[1]).artifactVersion' "$VERSIONS")"
export FUTURE_VERSION

echo "    host triple: $TRIPLE"
echo "    version    : $FUTURE_VERSION"

if [[ "$SKIP_DEPS" != true ]]; then
  echo "==> Installing npm dependencies (desktop)"
  (cd desktop && npm ci)
fi

# Unlike official CI's static musl release, this local build uses the host target.
# The shared portable Readme.txt describes both variants; do not promise static linkage here.
echo "==> Preparing unified CLI/Desktop build"

# `--set-bundle` rewrites the tracked desktop/src-tauri/tauri.conf.json, which is
# deliberately pinned at 0.0.0 (the real version is injected via FUTURE_VERSION).
# Snapshot it before the rewrite and put it back on exit, including on failure:
# otherwise every local package leaves a modified tracked file behind, and a
# dirty tree blocks the merges other sessions run in this checkout.
TAURI_CONF="$ROOT/desktop/src-tauri/tauri.conf.json"
TAURI_CONF_SNAPSHOT="$(mktemp)"
cp "$TAURI_CONF" "$TAURI_CONF_SNAPSHOT"
restore_tauri_conf() {
  cp -f "$TAURI_CONF_SNAPSHOT" "$TAURI_CONF"
  rm -f "$TAURI_CONF_SNAPSHOT"
}
trap restore_tauri_conf EXIT

echo "==> Setting Tauri bundle version"
node scripts/version.mjs --set-bundle

echo "==> Building unified CLI/headless (release, no GUI dependencies)"
cargo build --release -p future-cli --bin future

export FUTURE_DESKTOP_CLI="target/release/future"

echo "==> Building desktop app and .deb (Tauri)"
(cd desktop && npm run tauri:build)

DEB="$(find target/release/bundle/deb -maxdepth 1 -name '*.deb' -print -quit)"
[[ -n "$DEB" ]] || fail "Tauri produced no .deb bundle"

# Debian arch convention (Tauri uses amd64/arm64), independent of the Rust
# host triple naming.
case "$TRIPLE" in
  x86_64-*)  DEB_ARCH="amd64"; ASSET_ARCH="x86_64" ;;
  aarch64-*) DEB_ARCH="arm64"; ASSET_ARCH="aarch64" ;;
  *) fail "unsupported architecture in host triple: $TRIPLE" ;;
esac

echo "==> Assembling portable tarball"
dir="futureos-portable-linux"
mkdir -p "$dir"
cp "target/release/futureos" "$dir/futureos"
cp "target/release/future" "$dir/future"
chmod +x "$dir"/*
cp "docs/dist/readme-linux.txt" "$dir/Readme.txt"
PORTABLE="FutureOS_${ARTIFACT_VERSION}_linux_${ASSET_ARCH}-portable.tar.gz"
tar -czf "$PORTABLE" -C "$dir" .

deb_extract_dir="$(mktemp -d)"
dpkg-deb --extract "$DEB" "$deb_extract_dir"
if [[ ! -x "$deb_extract_dir/usr/bin/future" ]]; then
  dpkg-deb --contents "$DEB" >&2
  fail "the .deb does not contain executable /usr/bin/future"
fi
rm -rf -- "$deb_extract_dir"
tar -tzf "$PORTABLE" ./future >/dev/null \
  || fail "the portable archive does not contain ./future"

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
DEB_NAME="FutureOS_${ARTIFACT_VERSION}_${DEB_ARCH}.deb"
# Skip the copies when the package already lives in OUT_DIR (the default is
# the repo root, where the tarball was just assembled) — `cp -f a a` fails.
[[ "$OUT_DIR/$DEB_NAME" -ef "$DEB" ]] || cp -f "$DEB" "$OUT_DIR/$DEB_NAME"
[[ "$OUT_DIR/$PORTABLE" -ef "$PORTABLE" ]] \
  || cp -f "$PORTABLE" "$OUT_DIR/$PORTABLE"

echo
echo "Done:"
echo "  $OUT_DIR/$DEB_NAME"
echo "  $OUT_DIR/$PORTABLE"
