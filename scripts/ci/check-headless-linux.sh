#!/usr/bin/env bash
# Check the actual ELF, then start it in a clean Linux image with no GUI stack
# and an old glibc — the enterprise/HPC baseline the portable bundle must run
# on. Invoked from .github/workflows/build-linux.yaml; also usable directly on
# native x86_64/aarch64 Linux.
set -euo pipefail

binary="$(realpath "${1:?Usage: bash scripts/ci/check-headless-linux.sh <futureos-headless>}")"

# The release must be statically linked. A gnu build inherits the build host's
# glibc (2.39 on ubuntu-latest) and dies on older systems with the classic
#   /lib64/libc.so.6: version `GLIBC_2.29' not found
# readelf decides, not ldd: its output is stable for static binaries, and ldd
# may execute static-PIE inputs instead of inspecting them.
dynamic_section="$(readelf -d "$binary")"
if grep -q NEEDED <<<"$dynamic_section"; then
  grep NEEDED <<<"$dynamic_section" >&2
  echo "Headless binary is dynamically linked; build it for <arch>-unknown-linux-musl." >&2
  exit 1
fi
file -b "$binary" 2>/dev/null || true

# No network, real user data, Agent or TTY: first setup must refuse to start
# before contacting an Agent or emitting login/pairing secrets. Unlike --help
# alone, this also exercises the runtime and shared Desktop instance guard.
# Rocky 8 ships glibc 2.28 — older than every GLIBC_x the build host could
# stamp onto a gnu binary — so a linkage regression fails here, not on a
# user's HPC login node.
docker run --rm --network none \
  --mount "type=bind,src=$binary,dst=/usr/local/bin/futureos-headless,readonly" \
  --env HOME=/tmp/headless-home --env USERPROFILE=/tmp/headless-home \
  rockylinux:8 sh -eu -c '
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
