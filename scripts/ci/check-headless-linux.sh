#!/usr/bin/env bash
# Check the actual ELF, then start it in a clean Linux image with no GUI stack
# and an old glibc — the enterprise/HPC baseline the portable bundle must run
# on. Invoked from .github/workflows/build-linux.yaml; also usable directly on
# native x86_64/aarch64 Linux.
set -euo pipefail

binary="$(realpath "${1:?Usage: bash scripts/ci/check-headless-linux.sh <future>}")"

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

# No network, real user data or TTY: a signed-out host must serve locally
# without emitting login/pairing secrets. Unlike --help alone, this exercises
# the embedded Agent, local store and shared Desktop instance guard.
# Rocky 8 ships glibc 2.28 — older than every GLIBC_x the build host could
# stamp onto a gnu binary — so a linkage regression fails here, not on a
# user's HPC login node.
container_name="future-headless-probe-$$-$RANDOM"
trap 'docker rm -f "$container_name" >/dev/null 2>&1 || true' EXIT
timeout --signal=TERM --kill-after=10s 180s docker run --rm --name "$container_name" --network none \
  --mount "type=bind,src=$binary,dst=/usr/local/bin/future,readonly" \
  --env HOME=/tmp/headless-home --env USERPROFILE=/tmp/headless-home \
  --env FUTURE_HOME=/tmp/headless-home/.future \
  rockylinux:8 sh -eu -c '
    mkdir -p "$HOME"
    timeout 20s future host --help
    test ! -e "$HOME/.future"
    set +e
    output=$(timeout --kill-after=10s 20s future host </dev/null 2>&1)
    status=$?
    set -e
    printf "%s\n" "$output"
    # timeout sends SIGTERM to the serving host; 124 proves it stayed alive.
    test "$status" -eq 124
    printf "%s\n" "$output" | grep -F "FutureOS: embedded Agent ready"
    printf "%s\n" "$output" | grep -F "Not signed in to FutureOS: serving local"
    printf "%s\n" "$output" | grep -F "FutureOS: cleanup stage=verification"
    if printf "%s\n" "$output" | grep -E "PHONE PAIRING|user code"; then
      echo "Signed-out startup printed authorization material." >&2
      exit 1
    fi
    # Dev builds persist their default test-environment base_url during
    # startup. That configuration is expected and contains
    # no credential; reject an actual saved API key instead of requiring the
    # whole auth file to be absent. Release builds normally leave it absent.
    auth="$HOME/.future/agent/auth.json"
    if test -e "$auth"; then
      if grep -Eq "\"key\"[[:space:]]*:" "$auth"; then
        echo "Signed-out startup persisted credentials." >&2
        exit 1
      fi
    fi
    test ! -e "$HOME/.future/remote_pairing.json"
    test -s "$HOME/.future/app/app.db"
  '
