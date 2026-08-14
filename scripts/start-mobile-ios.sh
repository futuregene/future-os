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
APP_SCHEME="futureos"

MODE="${1:-dev}"
REBUILD_PREBUILD="${REBUILD_PREBUILD:-0}"
WARM_START="${WARM_START:-0}"
CLEAN_NATIVE_BUILD="${CLEAN_NATIVE_BUILD:-0}"
FORCE_NATIVE_BUILD="${FORCE_NATIVE_BUILD:-0}"
METRO_PORT="${METRO_PORT:-8081}"
alias_on_exit=""
IOS_NATIVE_LOCK=""
ios_native_lock_held=0

release_native_lock() {
  if [[ "$ios_native_lock_held" == "1" ]]; then
    rm -f "$IOS_NATIVE_LOCK"
    ios_native_lock_held=0
  fi
}

cleanup() {
  release_native_lock
  if [[ -n "$alias_on_exit" ]]; then
    echo ""
    echo "=========================================="
    echo "  FutureOS iOS ($MODE)"
    echo "=========================================="
    echo "  Simulator: $DEVICE_NAME ($RUNTIME)"
    if [[ "$MODE" == "dev" ]]; then
      echo "  Metro:     http://localhost:$METRO_PORT"
      echo "  Reload:    press Cmd+R in the simulator"
    fi
    echo "  Logs:      cd $MOBILE_DIR && npx react-native log-ios"
    echo "=========================================="
  fi
}

trap cleanup EXIT

acquire_native_lock() {
  IOS_NATIVE_LOCK="$MOBILE_DIR/build/.futureos-ios-native.lock"
  mkdir -p "$MOBILE_DIR/build"

  local waited=0
  until /usr/bin/shlock -f "$IOS_NATIVE_LOCK" -p "$$"; do
    local owner_pid
    owner_pid="$(cat "$IOS_NATIVE_LOCK" 2>/dev/null || echo "unknown")"
    if [[ "$waited" == "0" ]]; then
      echo "Another iOS native build is using the shared cache (PID $owner_pid). Waiting..."
    fi
    if [[ "$waited" -ge 120 ]]; then
      echo "The other iOS native build is still running (PID $owner_pid)."
      echo "Wait for it to finish, or stop that build before retrying."
      return 1
    fi
    sleep 2
    waited=$((waited + 2))
  done
  ios_native_lock_held=1
}

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

# `xcodebuild` stores a SQLite database in the persistent DerivedData cache.
# Serializing native work prevents a second terminal from failing with
# "build.db: database is locked" while the first build is still running.
acquire_native_lock

# ── prebuild ─────────────────────────────────────────────────────────────────

if [[ "$REBUILD_PREBUILD" == "1" ]] || [[ ! -d "$MOBILE_DIR/ios" ]]; then
  echo "Running expo prebuild..."
  (cd "$MOBILE_DIR" && npx expo prebuild --platform ios)
fi

# Keep Pods intact between runs. Synchronize only when the dependency manifests
# actually changed; comparing mtimes made every fresh checkout run CocoaPods
# again, even when the lockfile contents were unchanged.
POD_LOCK="$MOBILE_DIR/ios/Podfile.lock"
POD_STAMP="$MOBILE_DIR/ios/Pods/.futureos-dependencies.sha256"
POD_INPUT_HASH="$({
  shasum -a 256 "$MOBILE_DIR/package.json"
  shasum -a 256 "$ROOT_DIR/package-lock.json"
  shasum -a 256 "$MOBILE_DIR/ios/Podfile"
  shasum -a 256 "$POD_LOCK"
} | shasum -a 256 | awk '{print $1}')"
INSTALLED_POD_HASH="$(cat "$POD_STAMP" 2>/dev/null || true)"
if [[ ! -f "$POD_LOCK" ]] || [[ ! -d "$MOBILE_DIR/ios/Pods" ]] || \
  [[ "$POD_INPUT_HASH" != "$INSTALLED_POD_HASH" ]]; then
  echo "Synchronizing updated iOS native dependencies..."
  (cd "$MOBILE_DIR/ios" && pod install --no-repo-update)
  printf '%s\n' "$POD_INPUT_HASH" > "$POD_STAMP"
else
  echo "iOS native dependencies unchanged; reusing Pods."
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
    -derivedDataPath "$MOBILE_DIR/build/ios-release" build 2>&1 | tail -5)

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
  release_native_lock

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
  echo "Preparing debug app with persistent Xcode cache..."
  echo ""

  # Keep DerivedData inside mobile/build so every run reuses compiled pods,
  # Swift modules, and object files. `expo run:ios` uses Xcode's implicit cache
  # location and then treats a transient simctl openurl timeout as a build
  # failure, even when the app was built and installed successfully.
  DEV_DERIVED_DATA="$MOBILE_DIR/build/ios-debug"
  APP="$DEV_DERIVED_DATA/Build/Products/Debug-iphonesimulator/FutureOS.app"
  NATIVE_BUILD_STAMP="$DEV_DERIVED_DATA/.futureos-native-inputs.sha256"
  NATIVE_BUILD_HASH="$({
    shasum -a 256 "$ROOT_DIR/package-lock.json"
    shasum -a 256 "$MOBILE_DIR/package.json"
    shasum -a 256 "$MOBILE_DIR/app.config.ts"
    find "$MOBILE_DIR/ios" -type f \
      ! -path "$MOBILE_DIR/ios/Pods/*" \
      ! -path "$MOBILE_DIR/ios/build/*" \
      ! -path "*/xcuserdata/*" \
      ! -name ".xcode.env.local" -print | LC_ALL=C sort | while IFS= read -r native_file; do
        shasum -a 256 "$native_file"
      done
  } | shasum -a 256 | awk '{print $1}')"
  CACHED_NATIVE_BUILD_HASH="$(cat "$NATIVE_BUILD_STAMP" 2>/dev/null || true)"
  APP_INSTALLED=0
  if xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" app >/dev/null 2>&1; then
    APP_INSTALLED=1
  fi

  build_actions=(build)
  if [[ "$CLEAN_NATIVE_BUILD" == "1" ]]; then
    echo "CLEAN_NATIVE_BUILD=1 — cleaning the persistent debug build first."
    build_actions=(clean build)
  fi

  if [[ "$FORCE_NATIVE_BUILD" == "1" ]] || [[ "$CLEAN_NATIVE_BUILD" == "1" ]] || \
    [[ ! -d "$APP" ]] || [[ "$NATIVE_BUILD_HASH" != "$CACHED_NATIVE_BUILD_HASH" ]]; then
    (cd "$MOBILE_DIR/ios" && xcodebuild -workspace FutureOS.xcworkspace \
      -scheme FutureOS -configuration Debug \
      -destination "id=$DEVICE_UDID" \
      -derivedDataPath "$DEV_DERIVED_DATA" "${build_actions[@]}")

    if [[ ! -d "$APP" ]]; then
      echo "Debug app not found at $APP"
      exit 1
    fi

    printf '%s\n' "$NATIVE_BUILD_HASH" > "$NATIVE_BUILD_STAMP"
    APP_INSTALLED=0
  else
    echo "Native inputs unchanged; skipping Xcode build."
  fi

  if [[ "$APP_INSTALLED" != "1" ]]; then
    echo "Installing cached debug app..."
    xcrun simctl install "$DEVICE_UDID" "$APP"
  else
    echo "Reusing the installed debug app."
  fi

  release_native_lock

  # Start Metro in the foreground, but wait for it in a helper before opening
  # the development-client URL. Launching the app first avoids the common
  # CoreSimulator code-60 timeout. If SpringBoard is still unhealthy, restart
  # the app and finally the simulator before asking the developer to intervene.
  DEV_CLIENT_URL="${APP_SCHEME}://expo-development-client/?url=http%3A%2F%2F127.0.0.1%3A${METRO_PORT}"
  launch_dev_client() {
    local metro_ready=0
    local attempt

    for attempt in $(seq 1 120); do
      # Prefer the ::1 listener used by current macOS/Node releases, with an
      # IPv4 fallback for environments where localhost resolves differently.
      if { curl -6 --fail --silent "http://[::1]:${METRO_PORT}/status" || \
        curl -4 --fail --silent "http://127.0.0.1:${METRO_PORT}/status"; \
      } | grep -q "packager-status:running"; then
        metro_ready=1
        break
      fi
      sleep 0.5
    done

    if [[ "$metro_ready" != "1" ]]; then
      echo "Metro did not become ready on port $METRO_PORT."
      return 1
    fi

    echo "Starting development client..."
    xcrun simctl launch --terminate-running-process "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
    if xcrun simctl openurl "$DEVICE_UDID" "$DEV_CLIENT_URL"; then
      return 0
    fi

    echo "Development-client link timed out; restarting the app and retrying..."
    xcrun simctl terminate "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
    xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
    if xcrun simctl openurl "$DEVICE_UDID" "$DEV_CLIENT_URL"; then
      return 0
    fi

    echo "Simulator is not responding; rebooting it and retrying once..."
    xcrun simctl shutdown "$DEVICE_UDID" >/dev/null 2>&1 || true
    xcrun simctl boot "$DEVICE_UDID"
    xcrun simctl bootstatus "$DEVICE_UDID" -b
    xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
    if ! xcrun simctl openurl "$DEVICE_UDID" "$DEV_CLIENT_URL"; then
      echo "Could not reconnect the development client automatically."
      echo "Close and reopen Simulator, then run this script again."
      return 1
    fi
  }

  launch_dev_client &
  cd "$MOBILE_DIR"
  exec npx expo start --dev-client --localhost --port "$METRO_PORT"
fi
