#!/bin/bash
set -euo pipefail

echo "FutureOS mobile Android dev"

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MOBILE_DIR="$ROOT_DIR/mobile"

ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
ANDROID_SDK_ROOT="$ANDROID_HOME"

AVD_NAME="FutureOS"
SYSTEM_IMAGE="system-images;android-36.1;google_apis;arm64-v8a"
API_LEVEL="36"
BUILD_TOOLS_VERSION="36.0.0"

JAVA_HOME="${JAVA_HOME:-$(/usr/libexec/java_home -v 17 2>/dev/null || echo '')}"

MODE="${1:-dev}"
REBUILD_PREBUILD="${REBUILD_PREBUILD:-0}"
WARM_START="${WARM_START:-0}"
alias_on_exit=""

cleanup() {
  if [[ -n "$alias_on_exit" ]]; then
    echo ""
    echo "=========================================="
    echo "  FutureOS Android ($MODE)"
    echo "=========================================="
    echo "  ANDROID_HOME=$ANDROID_HOME"
    echo "  JAVA_HOME=$JAVA_HOME"
    echo "  AVD:      $AVD_NAME"
    if [[ "$MODE" == "dev" ]]; then
      echo "  Metro:    http://localhost:8081"
      echo "  Reload:   adb shell input keyevent 82"
    fi
    echo "  Logs:     cd $MOBILE_DIR && npx react-native log-android"
    echo "=========================================="
  fi
}

trap cleanup EXIT

# ── checks ──────────────────────────────────────────────────────────────────

if [[ -z "$ANDROID_HOME" ]]; then
  echo "ANDROID_HOME is not set. Please export ANDROID_HOME=/path/to/Android/sdk"
  exit 1
fi

if [[ ! -d "$ANDROID_HOME" ]]; then
  echo "Android SDK not found at $ANDROID_HOME"
  exit 1
fi

if [[ -z "$JAVA_HOME" ]] || [[ ! -d "$JAVA_HOME" ]]; then
  echo "Java 17 not found. Install it or export JAVA_HOME."
  exit 1
fi

export ANDROID_HOME
export ANDROID_SDK_ROOT
export JAVA_HOME

PLATFORM_TOOLS="$ANDROID_HOME/platform-tools"
EMULATOR="$ANDROID_HOME/emulator/emulator"
CMDLINE_TOOLS="$ANDROID_HOME/cmdline-tools/latest/bin"

if [[ ! -x "$PLATFORM_TOOLS/adb" ]]; then
  echo "adb not found at $PLATFORM_TOOLS/adb"
  exit 1
fi
export PATH="$PLATFORM_TOOLS:$PATH"

if [[ ! -x "$EMULATOR" ]]; then
  echo "Android emulator not found at $EMULATOR"
  exit 1
fi

echo "ANDROID_HOME=$ANDROID_HOME"
echo "JAVA_HOME=$JAVA_HOME"
echo "AVD: $AVD_NAME"
echo "Mode: $MODE"

# ── dependencies ────────────────────────────────────────────────────────────

if [[ ! -d "$MOBILE_DIR/node_modules" ]]; then
  echo "Installing mobile dependencies..."
  (cd "$MOBILE_DIR" && npm ci)
fi

# ── SDK image & tools ────────────────────────────────────────────────────────

IMAGE_PATH="$ANDROID_HOME/${SYSTEM_IMAGE//;//}"

if [[ ! -d "$IMAGE_PATH" ]]; then
  echo "Installing system image $SYSTEM_IMAGE..."
  yes | "$CMDLINE_TOOLS/sdkmanager" --sdk_root="$ANDROID_HOME" \
    "$SYSTEM_IMAGE" "platforms;android-$API_LEVEL" "build-tools;$BUILD_TOOLS_VERSION"
fi

# ── AVD ──────────────────────────────────────────────────────────────────────

if ! "$EMULATOR" -list-avds | grep -qxF "$AVD_NAME"; then
  echo "Creating AVD $AVD_NAME..."
  rm -rf "$HOME/.android/avd/${AVD_NAME}.avd" "$HOME/.android/avd/${AVD_NAME}.ini" 2>/dev/null || true
  echo "no" | "$CMDLINE_TOOLS/avdmanager" create avd \
    -n "$AVD_NAME" \
    -k "$SYSTEM_IMAGE" \
    --force
  echo "AVD $AVD_NAME created."
fi

# ── emulator ─────────────────────────────────────────────────────────────────

emulator_running() {
  adb devices 2>/dev/null | grep -qE '^emulator-.*\bdevice\b'
}

emulator_ready() {
  adb -e shell getprop sys.boot_completed 2>/dev/null | tr -d '\r\n' | grep -q '^1$'
}

if emulator_running && emulator_ready; then
  echo "Emulator $AVD_NAME is already running and ready."
else
  if emulator_running; then
    echo "Emulator is online but not fully booted. Waiting..."
  else
    if [[ "$WARM_START" == "1" ]]; then
      echo "WARM_START=1 — assuming emulator is already running. If not, drop WARM_START."
    else
      echo "Starting emulator $AVD_NAME..."
      "$EMULATOR" -avd "$AVD_NAME" -no-boot-anim -netdelay none -netspeed full &
    fi
  fi

  echo "Waiting for emulator to boot..."
  for i in $(seq 1 60); do
    sleep 5
    if emulator_ready; then
      echo "Emulator ready."
      break
    fi
    if [[ $((i % 6)) -eq 0 ]]; then
      echo "  still booting... (${i}x5s)"
    fi
  done

  if ! emulator_ready; then
    echo "Emulator did not finish booting in time."
    exit 1
  fi
fi

# ── prebuild ─────────────────────────────────────────────────────────────────

if [[ "$REBUILD_PREBUILD" == "1" ]] || [[ ! -d "$MOBILE_DIR/android/app/build/outputs" ]]; then
  echo "Running expo prebuild..."
  (cd "$MOBILE_DIR" && npx expo prebuild --platform android)
fi

# ── gen version ──────────────────────────────────────────────────────────────

(cd "$MOBILE_DIR" && npm run gen-version)

alias_on_exit="1"

if [[ "$MODE" == "release" ]]; then
  # ── release APK ───────────────────────────────────────────────────────────

  # Kill any lingering Metro so the emulator doesn't try to connect to it
  adb -e shell am force-stop cn.futureos.mobile 2>/dev/null || true

  # Ensure gradle can find the SDK even when ANDROID_HOME isn't inherited
  echo "sdk.dir=$ANDROID_HOME" > "$MOBILE_DIR/android/local.properties"

  echo ""
  echo "Building release APK..."
  echo ""

  (cd "$MOBILE_DIR/android" && ANDROID_HOME="$ANDROID_HOME" JAVA_HOME="$JAVA_HOME" ./gradlew assembleRelease)

  APK="$MOBILE_DIR/android/app/build/outputs/apk/release/app-release.apk"
  if [[ ! -f "$APK" ]]; then
    echo "Release APK not found at $APK"
    exit 1
  fi

  echo ""
  echo "Installing $APK..."
  adb -e install -r "$APK"

  echo "Starting app..."
  adb -e shell am start -n cn.futureos.mobile/.MainActivity

  APK_SIZE=$(du -h "$APK" | cut -f1)
  echo ""
  echo "Release APK installed — $APK_SIZE"
  echo "No Metro needed; app runs standalone."

  # Keep script alive so the cleanup hint prints on Ctrl-C
  # (BSD sleep on macOS rejects "infinity", so use the max numeric value)
  echo "Press Ctrl-C to exit."
  sleep 2147483647

else
  # ── dev (Metro + debug APK) ────────────────────────────────────────────────

  echo ""
  echo "Building + installing debug APK + starting Metro..."
  echo ""

  cd "$MOBILE_DIR" && exec env ANDROID_HOME="$ANDROID_HOME" JAVA_HOME="$JAVA_HOME" \
    npx expo run:android
fi
