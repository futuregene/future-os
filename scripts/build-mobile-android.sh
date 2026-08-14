#!/bin/bash
set -euo pipefail

# ── Local APK build script ──────────────────────────────────────────────────
# Builds a release APK from the current source tree.
# Output: mobile/android/app/build/outputs/apk/release/app-release.apk

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MOBILE_DIR="$ROOT_DIR/mobile"

ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
JAVA_HOME="${JAVA_HOME:-$(/usr/libexec/java_home -v 17 2>/dev/null || echo '')}"

# ── checks ──────────────────────────────────────────────────────────────────

if [[ ! -d "$ANDROID_HOME" ]]; then
  echo "ANDROID_HOME not found at $ANDROID_HOME"
  echo "Export ANDROID_HOME=/path/to/Android/sdk or install Android Studio."
  exit 1
fi

if [[ -z "$JAVA_HOME" ]] || [[ ! -d "$JAVA_HOME" ]]; then
  echo "Java 17 not found. Install it (brew install openjdk@17) or export JAVA_HOME."
  exit 1
fi

export ANDROID_HOME
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export JAVA_HOME

echo "ANDROID_HOME=$ANDROID_HOME"
echo "JAVA_HOME=$JAVA_HOME"

# ── dependencies ────────────────────────────────────────────────────────────

if [[ ! -d "$MOBILE_DIR/node_modules" ]]; then
  echo "Installing mobile dependencies..."
  (cd "$MOBILE_DIR" && npm ci)
fi


# ── version ─────────────────────────────────────────────────────────────────

echo "Generating version..."
(cd "$MOBILE_DIR" && npm run gen-version)

# ── prebuild ────────────────────────────────────────────────────────────────

if [[ ! -d "$MOBILE_DIR/android/app/build" ]]; then
  echo "Running expo prebuild..."
  (cd "$MOBILE_DIR" && npx expo prebuild --platform android)
fi

# ── local.properties ────────────────────────────────────────────────────────

echo "sdk.dir=$ANDROID_HOME" > "$MOBILE_DIR/android/local.properties"

# ── build ───────────────────────────────────────────────────────────────────

echo ""
echo "Building release APK..."
echo ""

(cd "$MOBILE_DIR/android" && ./gradlew assembleRelease)

APK="$MOBILE_DIR/android/app/build/outputs/apk/release/app-release.apk"
if [[ ! -f "$APK" ]]; then
  echo "Build failed: APK not found at $APK"
  exit 1
fi

APK_SIZE=$(du -h "$APK" | cut -f1)
echo ""
echo "Done — $APK ($APK_SIZE)"
