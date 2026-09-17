#!/usr/bin/env bash
# Check the actual ELF, then start it in a clean Linux image with no GUI stack.
# Manual check on native x86_64/aarch64 Linux; not wired into any workflow.
set -euo pipefail

binary="$(realpath "${1:?Usage: bash scripts/ci/check-headless-linux.sh <futureos-headless>}")"
dependencies="$(ldd "$binary")"
printf '%s\n' "$dependencies"
if grep -Eiq 'not found|lib(webkit|javascriptcore|gtk|gdk|soup|X11|wayland)' <<< "$dependencies"; then
  echo "Headless binary has missing or graphical runtime dependencies." >&2
  exit 1
fi

# No network, real user data, Agent or TTY: first setup must refuse to start
# before contacting an Agent or emitting login/pairing secrets. Unlike --help
# alone, this also exercises the runtime and shared Desktop instance guard.
docker run --rm --network none \
  --mount "type=bind,src=$binary,dst=/usr/local/bin/futureos-headless,readonly" \
  --env HOME=/tmp/headless-home --env USERPROFILE=/tmp/headless-home \
  ubuntu:24.04 sh -eu -c '
    mkdir -p "$HOME"
    futureos-headless --help
    test ! -e "$HOME/.future"
    set +e
    output=$(futureos-headless </dev/null 2>&1)
    status=$?
    set -e
    printf "%s\n" "$output"
    test "$status" -eq 1
    printf "%s\n" "$output" | grep -F "interactive terminal"
    # Dev builds persist their default test-environment base_url before the
    # interactive-terminal gate. That configuration is expected and contains
    # no credential; reject an actual saved API key instead of requiring the
    # whole auth file to be absent. Release builds normally leave it absent.
    auth="$HOME/.future/agent/auth.json"
    if test -e "$auth"; then
      ! grep -Eq '"key"[[:space:]]*:' "$auth"
    fi
    test ! -e "$HOME/.future/remote_pairing.json"
    test ! -e "$HOME/.future/app/app.db"
  '
