#!/bin/bash
set -euo pipefail

echo "FutureOS mobile iOS dev"

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MOBILE_DIR="$ROOT_DIR/mobile"

RUNTIME="com.apple.CoreSimulator.SimRuntime.iOS-26-5"
DEVICE_TYPE="com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro"
DEVICE_NAME="iPhone 17 Pro"
BUNDLE_ID="cn.futureos.mobile"

MODE="${1:-dev}"
REBUILD_PREBUILD="${REBUILD_PREBUILD:-0}"
WARM_START="${WARM_START:-0}"
CLEAN_NATIVE_BUILD="${CLEAN_NATIVE_BUILD:-0}"
alias_on_exit=""

cleanup() {
  if [[ -n "$alias_on_exit" ]]; then
    echo ""
    echo "=========================================="
    echo "  FutureOS iOS ($MODE)"
    echo "=========================================="
    echo "  Simulator: $DEVICE_NAME ($RUNTIME)"
    if [[ "$MODE" == "dev" ]]; then
      echo "  Metro:     http://localhost:8081"
      echo "  Reload:    press Cmd+R in the simulator"
    fi
    echo "  Logs:      cd $MOBILE_DIR && npx react-native log-ios"
    echo "=========================================="
  fi
}

trap cleanup EXIT

# ── checks ──────────────────────────────────────────────────────────────────

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "iOS simulator is only available on macOS. Aborting."
  exit 1
fi

if ! command -v xcrun >/dev/null 2>&1 || ! xcode-select -p >/dev/null 2>&1; then
  echo "Xcode command line tools not found. Install Xcode first."
  exit 1
fi

if ! xcrun simctl list runtimes | grep -q "$RUNTIME"; then
  echo "iOS simulator runtime $RUNTIME is not installed."
  echo "Install it with:  xcodebuild -downloadPlatform iOS"
  exit 1
fi

echo "Simulator: $DEVICE_NAME ($RUNTIME)"
echo "Mode: $MODE"

# ── dependencies ────────────────────────────────────────────────────────────

if [[ ! -d "$MOBILE_DIR/node_modules" ]]; then
  echo "Installing mobile dependencies..."
  (cd "$MOBILE_DIR" && npm ci)
fi

# ── simulator ────────────────────────────────────────────────────────────────

sim_running() {
  xcrun simctl list devices | grep -qE "^ *${DEVICE_NAME} \(" && \
    xcrun simctl list devices | grep -E "^ *${DEVICE_NAME} \(" | grep -q "Booted"
}

sim_ready() {
  xcrun simctl bootstatus "${DEVICE_NAME}" -b >/dev/null 2>&1
}

DEVICE_UDID="$(xcrun simctl list devices | grep -E "^ *${DEVICE_NAME} \(" | grep -oE '[0-9A-F-]{36}' | head -1 || true)"
if [[ -z "$DEVICE_UDID" ]]; then
  echo "Creating simulator $DEVICE_NAME..."
  DEVICE_UDID="$(xcrun simctl create "$DEVICE_NAME" "$DEVICE_TYPE" "$RUNTIME")"
  echo "Created simulator $DEVICE_NAME ($DEVICE_UDID)"
fi

if sim_running && sim_ready; then
  echo "Simulator $DEVICE_NAME is already running and ready."
else
  if sim_running; then
    echo "Simulator is online but not fully booted. Waiting..."
  else
    if [[ "$WARM_START" == "1" ]]; then
      echo "WARM_START=1 — assuming simulator is already running. If not, drop WARM_START."
    else
      echo "Booting simulator $DEVICE_NAME..."
      xcrun simctl boot "$DEVICE_UDID" 2>/dev/null || true
      open -a Simulator
    fi
  fi

  echo "Waiting for simulator to boot..."
  for i in $(seq 1 60); do
    sleep 5
    if sim_ready; then
      echo "Simulator ready."
      break
    fi
    if [[ $((i % 6)) -eq 0 ]]; then
      echo "  still booting... (${i}x5s)"
    fi
  done

  if ! sim_ready; then
    echo "Simulator did not finish booting in time."
    exit 1
  fi
fi

# ── prebuild ─────────────────────────────────────────────────────────────────

if [[ "$REBUILD_PREBUILD" == "1" ]] || [[ ! -d "$MOBILE_DIR/ios" ]]; then
  echo "Running expo prebuild..."
  (cd "$MOBILE_DIR" && npx expo prebuild --platform ios)
fi

# expo run:ios reuses an existing Pods directory. When a package adds an Expo
# native module, Metro can load its JS while the installed development client
# still lacks the corresponding native code. A regular `pod install` refuses
# stale local podspec snapshots, so refresh Pods without updating remote specs
# whenever the JavaScript dependency manifests changed.
POD_LOCK="$MOBILE_DIR/ios/Podfile.lock"
if [[ ! -f "$POD_LOCK" ]] || [[ "$MOBILE_DIR/package.json" -nt "$POD_LOCK" ]] || \
  [[ "$MOBILE_DIR/package-lock.json" -nt "$POD_LOCK" ]]; then
  echo "Synchronizing updated iOS native dependencies..."
  (cd "$MOBILE_DIR/ios" && pod update --no-repo-update)
fi

# ── gen version ──────────────────────────────────────────────────────────────

(cd "$MOBILE_DIR" && npm run gen-version)

alias_on_exit="1"

if [[ "$MODE" == "release" ]]; then
  # ── release build ───────────────────────────────────────────────────────────

  # Kill any lingering Metro so the simulator doesn't try to connect to it
  xcrun simctl terminate "$DEVICE_UDID" "$BUNDLE_ID" 2>/dev/null || true

  echo ""
  echo "Building release app..."
  echo ""

  (cd "$MOBILE_DIR/ios" && xcodebuild -workspace FutureOS.xcworkspace \
    -scheme FutureOS -configuration Release \
    -destination "id=$DEVICE_UDID" \
    -derivedDataPath "$MOBILE_DIR/build/ios-release" build \
    CODE_SIGNING_ALLOWED=NO 2>&1 | tail -5)

  APP="$MOBILE_DIR/build/ios-release/Build/Products/Release-iphonesimulator/FutureOS.app"
  if [[ ! -d "$APP" ]]; then
    echo "Release app not found at $APP"
    exit 1
  fi

  echo ""
  echo "Installing $APP..."
  xcrun simctl install "$DEVICE_UDID" "$APP"

  echo "Starting app..."
  xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID"

  APP_SIZE=$(du -h -d 0 "$APP" | cut -f1)
  echo ""
  echo "Release app installed — $APP_SIZE"
  echo "No Metro needed; app runs standalone."

  # Keep script alive so the cleanup hint prints on Ctrl-C
  # (BSD sleep on macOS rejects "infinity", so use the max numeric value)
  echo "Press Ctrl-C to exit."
  sleep 2147483647

else
  # ── dev (Metro + debug build) ───────────────────────────────────────────────

  echo ""
  echo "Building + installing debug app + starting Metro..."
  echo ""

  run_args=()
  if [[ "$CLEAN_NATIVE_BUILD" == "1" ]]; then
    echo "CLEAN_NATIVE_BUILD=1 — clearing Xcode DerivedData before building."
    run_args+=(--no-build-cache)
  fi
  cd "$MOBILE_DIR" && exec npx expo run:ios "${run_args[@]}"
fi
