#!/usr/bin/env bash
# Bubblewrap availability is decided by the unchanged Agent sandbox probe.
# Missing bwrap disables sandbox tools; it never selects an unconfined fallback.
exec bash "$(dirname -- "${BASH_SOURCE[0]}")/start-desktop-common.sh" "$@"
