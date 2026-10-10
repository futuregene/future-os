#!/usr/bin/env bash
# Measure per-line coverage for the channels crate and report uncovered lines.
#
# Usage:
#   scripts/measure/chan-cov.sh                          # whole crate: report + missed list
#   scripts/measure/chan-cov.sh --check FILE [FILE...]   # fail if any listed file has an
#                                                # uncovered line (exit 1), print them
#
# FILE is a repo-relative path (`channels/src/foo.rs`). With --check the
# coverage of every file is measured as usual, but only the listed files are
# judged — a peer's in-flight file cannot fail your gate.
#
# Why the environment is sanitised: a leaked `CARGO_HOME` / `CARGO_TARGET_DIR`
# changes the instrumented unit hash and llvm-cov then reports every line as
# "never executed". An isolated HOME keeps a real `~/.future` out of the
# channel tests.
#
# Outputs under coverage/chan/: lcov.info, missed.txt (grouped list), summary.txt.
set -euo pipefail

cd "$(dirname "$0")/../.."

check=0
files=()
while [ $# -gt 0 ]; do
  case "$1" in
    --check) check=1 ;;
    *) files+=("$1") ;;
  esac
  shift
done

if [ "$check" = 1 ] && [ "${#files[@]}" = 0 ]; then
  echo "usage: scripts/measure/chan-cov.sh --check FILE [FILE...]" >&2
  exit 2
fi

# One instrumented build per machine. Parallel workers share this checkout, and
# several concurrent llvm-cov runs thrash the CPU and have been observed to get
# the test binary killed mid-run. Take an exclusive lock and wait our turn.
#
# The lock records its owner's pid so a run that was killed outright (SIGKILL
# skips the trap) cannot block everyone else until the wait timeout.
lock_dir="${TMPDIR:-/tmp}/future-chan-cov.lock"

# Directory age in seconds, portable across GNU and BSD `stat`.
lock_age() {
  local mtime
  mtime=$(stat -f %m "$lock_dir" 2>/dev/null || stat -c %Y "$lock_dir" 2>/dev/null || echo 0)
  [ "$mtime" = 0 ] && { echo 0; return; }
  echo $(( $(date +%s) - mtime ))
}

lock_wait=0
while ! mkdir "$lock_dir" 2>/dev/null; do
  holder=$(cat "$lock_dir/pid" 2>/dev/null || true)
  stale=0
  if [ -n "$holder" ]; then
    kill -0 "$holder" 2>/dev/null || stale=1
  elif [ "$(lock_age)" -gt 60 ]; then
    # No owner recorded and nothing has touched it for a minute: left behind by
    # an older version of this script or a hard kill.
    stale=1
  fi
  if [ "$stale" = 1 ]; then
    echo "chan-cov: removing a stale measurement lock (owner: ${holder:-unknown})"
    rm -rf "$lock_dir"
    continue
  fi
  lock_wait=$((lock_wait + 1))
  if [ "$lock_wait" -gt 7200 ]; then
    echo "chan-cov: gave up waiting for the measurement lock at $lock_dir" >&2
    exit 3
  fi
  if [ "$lock_wait" = 1 ]; then
    echo "chan-cov: another measurement is running; waiting for the lock"
  fi
  sleep 5
done
echo "$$" > "$lock_dir/pid"
cleanup_lock() { rm -rf "$lock_dir"; }
trap cleanup_lock EXIT INT TERM

unset CARGO_HOME CARGO_TARGET_DIR CARGO_BUILD_TARGET 2>/dev/null || true
export PATH="$HOME/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH"
export HOME="$PWD/target/chan-cov-home"
mkdir -p "$HOME"

out_dir=coverage/chan
mkdir -p "$out_dir"

cargo llvm-cov clean --workspace
cargo llvm-cov --no-report -p future-channel --all-targets

cargo llvm-cov report --lcov --output-path "$out_dir/lcov.info" -p future-channel
cargo llvm-cov report --text --output-path "$out_dir/summary.txt" -p future-channel

python3 - "$out_dir/lcov.info" "$out_dir/missed.txt" <<'PY'
import collections
import sys

lcov, out = sys.argv[1], sys.argv[2]
current = None
missed = collections.defaultdict(list)
for raw in open(lcov):
    line = raw.strip()
    if line.startswith("SF:"):
        current = line[3:]
    elif line.startswith("DA:") and current:
        number, _, count = line[3:].partition(",")
        if count.split(",")[0] == "0":
            missed[current].append(int(number))

total = sum(len(v) for v in missed.values())
with open(out, "w") as handle:
    for path in sorted(missed):
        for number in sorted(missed[path]):
            handle.write(f"{path}:{number}\n")
    handle.write("\n")
    for path in sorted(missed):
        handle.write(f"{len(missed[path]):5d}  {path}\n")
    handle.write(f"{total:5d}  TOTAL\n")
print(f"crate uncovered lines: {total} in {len(missed)} file(s) -> {out}")
PY

if [ "$check" = 0 ]; then
  python3 scripts/measure/chan-missed.py
  exit 0
fi

failed=0
for relative in "${files[@]}"; do
  # Match on the repo-relative suffix so the absolute lcov paths line up.
  hits=$(awk -v needle="/$relative:" 'index($0, needle) > 0' "$out_dir/missed.txt")
  count=$(printf '%s' "$hits" | grep -c . || true)
  if [ "$count" != 0 ]; then
    echo "FAIL $relative: $count uncovered line(s)"
    printf '%s\n' "$hits" | sed "s|.*/$relative:|  line |"
    failed=1
  else
    echo "ok   $relative: 0 uncovered lines"
  fi
done
exit "$failed"
