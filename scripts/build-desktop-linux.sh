#!/usr/bin/env bash
#
# Build the FutureOS Linux packages locally: the Tauri .deb plus the portable
# tarball. Linux packages are unsigned — there is no code-signing equivalent —
# so this script has none of the macOS signing/notarization machinery.
#
# Produces:
#   FutureOS_<bundle-version>_<arch>.deb  (Tauri .deb: desktop app + CLI sidecar)
#   FutureOS-portable-linux.tar.gz        (portable: futureos + future + Readme.txt)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

SKIP_DEPS=false
OUT_DIR="$ROOT"

usage() {
  cat <<'EOF'
Usage: scripts/build-desktop-linux.sh [options]

Build the unified `future` CLI and Tauri desktop app, then produce the Linux
distribution packages (.deb + portable tarball).

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

TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
[[ -n "$TRIPLE" ]] || fail "could not read the host triple from rustc -Vv"

if [[ -z "${FUTURE_VERSION:-}" ]]; then
  FUTURE_VERSION="$(node scripts/version.mjs)"
  export FUTURE_VERSION
fi

echo "    host triple: $TRIPLE"
echo "    version    : $FUTURE_VERSION"

if [[ "$SKIP_DEPS" != true ]]; then
  echo "==> Installing npm dependencies (desktop)"
  (cd desktop && npm ci)
fi

echo "==> Building CLI (release) and staging as Tauri sidecar"
cargo build --release --manifest-path cli/Cargo.toml
mkdir -p desktop/src-tauri/binaries
cp target/release/future "desktop/src-tauri/binaries/future-$TRIPLE"

echo "==> Setting Tauri bundle version"
node scripts/version.mjs --set-bundle
BUNDLE_VERSION="$(node -e \
  "process.stdout.write(require('./desktop/src-tauri/tauri.conf.json').version)")"

echo "==> Building desktop app and .deb (Tauri)"
(cd desktop && npm run tauri:build)

DEB="$(find desktop/src-tauri/target/release/bundle/deb -maxdepth 1 -name '*.deb' -print -quit)"
[[ -n "$DEB" ]] || fail "Tauri produced no .deb bundle"

# Debian arch convention (Tauri uses amd64/arm64), independent of the Rust
# host triple naming.
case "$TRIPLE" in
  x86_64-*)  DEB_ARCH="amd64" ;;
  aarch64-*) DEB_ARCH="arm64" ;;
  *) fail "unsupported architecture in host triple: $TRIPLE" ;;
esac

echo "==> Assembling portable tarball"
dir="futureos-portable-linux"
mkdir -p "$dir"
cp "desktop/src-tauri/target/release/futureos" "$dir/futureos"
cp "target/release/future" "$dir/future"
chmod +x "$dir"/*
cp "docs/dist/readme-linux.txt" "$dir/Readme.txt"
tar -czf FutureOS-portable-linux.tar.gz -C "$dir" .

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
DEB_NAME="FutureOS_${BUNDLE_VERSION}_${DEB_ARCH}.deb"
# Skip the copies when the package already lives in OUT_DIR (the default is
# the repo root, where the tarball was just assembled) — `cp -f a a` fails.
[[ "$OUT_DIR/$DEB_NAME" -ef "$DEB" ]] || cp -f "$DEB" "$OUT_DIR/$DEB_NAME"
[[ "$OUT_DIR/FutureOS-portable-linux.tar.gz" -ef FutureOS-portable-linux.tar.gz ]] \
  || cp -f FutureOS-portable-linux.tar.gz "$OUT_DIR/FutureOS-portable-linux.tar.gz"

echo
echo "Done:"
echo "  $OUT_DIR/$DEB_NAME"
echo "  $OUT_DIR/FutureOS-portable-linux.tar.gz"
