#!/usr/bin/env bash

set -euo pipefail

simulator="${FUTURE_IOS_SIMULATOR:-booted}"
bundle_id="${FUTURE_IOS_BUNDLE_ID:-cn.futureos.mobile}"

# `terminate` also fails when the app is already stopped; that is safe to ignore.
xcrun simctl terminate "$simulator" "$bundle_id" >/dev/null 2>&1 || true
xcrun simctl privacy "$simulator" reset all "$bundle_id"

echo "Reset simulator permissions for $bundle_id on $simulator."
