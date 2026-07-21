#!/usr/bin/env bash
#
# Build the FutureOS macOS DMG locally, using a Developer ID Application
# certificate when one can be resolved from the current user's keychains.
#
# The script intentionally falls back to the normal ad-hoc Tauri package when
# no unambiguous Developer ID identity is available. A signing failure after an
# identity has been selected is still fatal: silently replacing a broken signed
# release with an ordinary package would hide a real certificate/build problem.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

SKIP_DEPS=false
OUT_DIR="$ROOT"
IDENTITY_FILTER=""
NOTARY_PROFILE="${APPLE_NOTARY_PROFILE:-}"
FORCE_UNSIGNED=false

usage() {
  cat <<'EOF'
Usage: scripts/build-macos-dmg.sh [options]

Build the agent, standalone CLI and Tauri GUI, then produce a macOS DMG.
Signing is automatic when exactly one Developer ID Application identity is
available. Without a usable identity the script falls back to a normal package.

Options:
  --skip-deps              Skip npm ci in gui/ and cli/.
  --out-dir DIR            Copy the final DMG to DIR (default: repository root).
  --identity TEXT          Select a Developer ID identity containing TEXT.
  --notary-profile NAME    notarytool keychain profile used for notarization.
  --unsigned               Do not look for a certificate; build a normal DMG.
  -h, --help               Show this help.

Notarization credentials, in priority order:
  1. --notary-profile or APPLE_NOTARY_PROFILE
  2. APPLE_API_KEY_PATH + APPLE_API_KEY (key ID) + APPLE_API_ISSUER
  3. APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID

Examples:
  scripts/build-macos-dmg.sh
  scripts/build-macos-dmg.sh --skip-deps --out-dir ./dist
  scripts/build-macos-dmg.sh --identity "Developer ID Application: Example"
  APPLE_NOTARY_PROFILE=futureos scripts/build-macos-dmg.sh

Create a reusable local notarization profile with:
  xcrun notarytool store-credentials futureos
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
    --identity)
      [[ $# -ge 2 ]] || fail "--identity requires text to match"
      IDENTITY_FILTER="$2"
      shift 2
      ;;
    --notary-profile)
      [[ $# -ge 2 ]] || fail "--notary-profile requires a profile name"
      NOTARY_PROFILE="$2"
      shift 2
      ;;
    --unsigned)
      FORCE_UNSIGNED=true
      shift
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

[[ "$(uname -s)" == "Darwin" ]] || fail "this script must run on macOS"

cd "$ROOT"

echo "==> Checking prerequisites"
require_tool node "Install Node.js 24+ (https://nodejs.org)."
require_tool npm "npm is included with Node.js."
require_tool bun "Install Bun (https://bun.sh)."
require_tool cargo "Install Rust (https://rustup.rs)."
require_tool rustc "Install Rust (https://rustup.rs)."
require_tool protoc "Install protobuf with 'brew install protobuf'."
require_tool security "security is provided by macOS."
require_tool codesign "Install the Xcode command line tools."
require_tool xcrun "Install the Xcode command line tools."

TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
[[ -n "$TRIPLE" ]] || fail "could not read the host triple from rustc -Vv"

if [[ -z "${FUTURE_VERSION:-}" ]]; then
  FUTURE_VERSION="$(node scripts/version.mjs)"
  export FUTURE_VERSION
fi

echo "    host triple: $TRIPLE"
echo "    version    : $FUTURE_VERSION"

# Read only valid code-signing identities. The quoted field is the full name
# accepted by both Tauri's signingIdentity setting and /usr/bin/codesign.
IDENTITIES=()
while IFS= read -r identity; do
  [[ -n "$identity" ]] && IDENTITIES[${#IDENTITIES[@]}]="$identity"
done < <(security find-identity -v -p codesigning 2>/dev/null \
  | awk -F'"' '/Developer ID Application:/ { print $2 }')

SIGNING_IDENTITY=""
if [[ "$FORCE_UNSIGNED" == true ]]; then
  echo "==> Signing disabled by --unsigned; building a normal DMG"
elif [[ -n "$IDENTITY_FILTER" ]]; then
  MATCHES=()
  if [[ ${#IDENTITIES[@]} -gt 0 ]]; then
    for identity in "${IDENTITIES[@]}"; do
      if [[ "$identity" == *"$IDENTITY_FILTER"* ]]; then
        MATCHES[${#MATCHES[@]}]="$identity"
      fi
    done
  fi
  if [[ ${#MATCHES[@]} -eq 1 ]]; then
    SIGNING_IDENTITY="${MATCHES[0]}"
  elif [[ ${#MATCHES[@]} -eq 0 ]]; then
    echo "warning: no Developer ID Application identity matched '$IDENTITY_FILTER'." >&2
    echo "         Falling back to a normal DMG." >&2
  else
    echo "warning: --identity '$IDENTITY_FILTER' matched more than one identity:" >&2
    printf '         %s\n' "${MATCHES[@]}" >&2
    echo "         Falling back to a normal DMG; use a more specific value." >&2
  fi
elif [[ ${#IDENTITIES[@]} -eq 1 ]]; then
  SIGNING_IDENTITY="${IDENTITIES[0]}"
elif [[ ${#IDENTITIES[@]} -eq 0 ]]; then
  echo "==> No Developer ID Application certificate found; building a normal DMG"
else
  echo "warning: more than one Developer ID Application identity is available:" >&2
  printf '         %s\n' "${IDENTITIES[@]}" >&2
  echo "         Falling back to a normal DMG; rerun with --identity to select one." >&2
fi

if [[ -n "$SIGNING_IDENTITY" ]]; then
  echo "==> Signing identity"
  echo "    $SIGNING_IDENTITY"
fi

if [[ "$SKIP_DEPS" != true ]]; then
  echo "==> Installing npm dependencies (gui, cli)"
  (cd gui && npm ci)
  (cd cli && npm ci)
fi

echo "==> Building agent (release)"
cargo build --release --manifest-path agent/Cargo.toml
mkdir -p gui/src-tauri/binaries
cp target/release/future-agent "gui/src-tauri/binaries/future-agent-$TRIPLE"

echo "==> Building CLI (standalone binary)"
(
  cd cli
  npm run build
  bun build --compile dist/index.js --outfile dist/future --external chromium-bidi
)
cp cli/dist/future "gui/src-tauri/binaries/future-$TRIPLE"

echo "==> Setting Tauri bundle version"
node scripts/version.mjs --set-bundle

OVERLAY=""
OVERLAY_DIR=""
cleanup() {
  if [[ -n "$OVERLAY" && -f "$OVERLAY" ]]; then
    rm -f "$OVERLAY"
  fi
  if [[ -n "$OVERLAY_DIR" && -d "$OVERLAY_DIR" ]]; then
    rmdir "$OVERLAY_DIR" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ -n "$SIGNING_IDENTITY" ]]; then
  OVERLAY_DIR="$(mktemp -d "${TMPDIR:-/tmp}/futureos-macos-sign.XXXXXX")"
  OVERLAY="$OVERLAY_DIR/tauri.macos-sign.json"
  SIGNING_IDENTITY="$SIGNING_IDENTITY" node -e '
    const fs = require("node:fs");
    fs.writeFileSync(process.argv[1], JSON.stringify({
      bundle: { macOS: { signingIdentity: process.env.SIGNING_IDENTITY } }
    }));
  ' "$OVERLAY"
fi

echo "==> Building macOS app and DMG (Tauri)"
# Notarize the final DMG ourselves below. Remove Tauri's notarization variables
# for this command so a configured local environment does not submit the app
# once here and then submit the containing DMG a second time.
if [[ -n "$SIGNING_IDENTITY" ]]; then
  (
    cd gui
    env -u APPLE_SIGNING_IDENTITY \
        -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
        -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
        npm run tauri:build -- --config "$OVERLAY"
  )
else
  (
    cd gui
    env -u APPLE_SIGNING_IDENTITY \
        -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
        -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
        npm run tauri:build
  )
fi

APP="$(find gui/src-tauri/target/release/bundle/macos -maxdepth 1 -name '*.app' -print -quit)"
DMG="$(find gui/src-tauri/target/release/bundle/dmg -maxdepth 1 -name '*.dmg' -print -quit)"
[[ -n "$APP" ]] || fail "Tauri produced no .app bundle"
[[ -n "$DMG" ]] || fail "Tauri produced no DMG"

SIGNED=false
NOTARIZED=false
if [[ -n "$SIGNING_IDENTITY" ]]; then
  echo "==> Verifying app signature"
  codesign --verify --deep --strict --verbose=2 "$APP"
  SIGNATURE_INFO="$(codesign --display --verbose=4 "$APP" 2>&1)"
  printf '%s\n' "$SIGNATURE_INFO"
  grep -q 'flags=.*runtime' <<<"$SIGNATURE_INFO" \
    || fail "the app signature does not enable hardened runtime"

  # Tauri signs the DMG after creating it. Do not force-sign it again: replacing
  # an embedded signature on an already compressed image can leave a signature
  # that passes the current process's cache but fails a fresh verification.
  echo "==> Verifying DMG signature"
  codesign --verify --strict --verbose=2 "$DMG"
  SIGNED=true

  NOTARY_ARGS=()
  if [[ -n "$NOTARY_PROFILE" ]]; then
    NOTARY_ARGS=(--keychain-profile "$NOTARY_PROFILE")
  elif [[ -n "${APPLE_API_KEY_PATH:-}" \
       && -n "${APPLE_API_KEY:-}" \
       && -n "${APPLE_API_ISSUER:-}" ]]; then
    [[ -f "$APPLE_API_KEY_PATH" ]] \
      || fail "APPLE_API_KEY_PATH does not exist: $APPLE_API_KEY_PATH"
    NOTARY_ARGS=(
      --key "$APPLE_API_KEY_PATH"
      --key-id "$APPLE_API_KEY"
      --issuer "$APPLE_API_ISSUER"
    )
  elif [[ -n "${APPLE_ID:-}" \
       && -n "${APPLE_PASSWORD:-}" \
       && -n "${APPLE_TEAM_ID:-}" ]]; then
    NOTARY_ARGS=(
      --apple-id "$APPLE_ID"
      --password "$APPLE_PASSWORD"
      --team-id "$APPLE_TEAM_ID"
    )
  fi

  if [[ ${#NOTARY_ARGS[@]} -gt 0 ]]; then
    echo "==> Submitting DMG for Apple notarization"
    xcrun notarytool submit "$DMG" "${NOTARY_ARGS[@]}" --wait
    xcrun stapler staple "$DMG"
    xcrun stapler validate "$DMG"
    spctl --assess --type open \
      --context context:primary-signature --verbose=2 "$DMG"
    NOTARIZED=true
  else
    echo "warning: the DMG is signed but not notarized because no notarization" >&2
    echo "         credentials were found. See --help for supported options." >&2
  fi
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
BASE="$(basename "$DMG" .dmg)"
if [[ "$SIGNED" == true ]]; then
  OUTPUT="$OUT_DIR/${BASE}-sign.dmg"
else
  OUTPUT="$OUT_DIR/${BASE}.dmg"
fi
cp -f "$DMG" "$OUTPUT"

echo
echo "Done: $OUTPUT"
if [[ "$NOTARIZED" == true ]]; then
  echo "  status: signed and notarized"
elif [[ "$SIGNED" == true ]]; then
  echo "  status: signed, not notarized"
else
  echo "  status: normal package (no Developer ID signature)"
fi
