#!/usr/bin/env bash
# Default embedded hosting; FUTURE_DESKTOP_AGENT_MODE=external is attach-only.
exec bash "$(dirname -- "${BASH_SOURCE[0]}")/start-desktop-common.sh" "$@"
