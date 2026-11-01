#!/bin/bash
# Split a coverage lane's gate into a *build* step and a *timed run* step.
#
# Why this exists (2026-11-02, agent lane): the gate was one
# `cargo llvm-cov -p <pkg> --no-report`, which builds and runs in a single
# process. Its wall clock could not tell a slow suite from a cold instrumented
# build — on the agent lane the two are the same order of magnitude (a 40 s
# single-target rebuild and a 3 m 08 s cold dependency build against a ~237 s
# suite) — so a per-round budget could only be stated as "the whole thing",
# which is what made a 480 s gate look tight. Measured on that lane: the same
# tree, minutes apart, both at 8 threads, split 1 s + 237 s = 238 s against
# 291 s for the one-process form.
#
# The build step is **not** `cargo llvm-cov --no-run`: that is deprecated and is
# rejected next to `--no-report` (`error: --no-report may not be used together
# with --no-run`) — it means "report from the profraw already on disk", not
# "build only". So step 1 sources the instrumented environment that
# `cargo llvm-cov show-env --sh` prints and runs `cargo test --no-run` inside
# it: same RUSTC_WRAPPER, same target dir, so step 2 recompiles nothing. That is
# measured, not assumed — the script counts step 2's `Compiling` lines.
#
# Usage:
#   scripts/coverage/measure-split.sh PACKAGE OUT.info [options]
#
#   PACKAGE            cargo package to measure (e.g. future-agent)
#   OUT.info           where the lcov goes; logs are written beside it
#
# Options:
#   --threads N        libtest threads for the run step (default 8; `default`
#                       leaves RUST_TEST_THREADS unset)
#   --budget S         wall-clock budget for the run step only (default 480)
#   --expect-targets N how many test targets must run. Omit and the script says
#                       it cannot detect a truncated run — a truncated `cargo
#                       llvm-cov` still writes an lcov, and on the agent lane
#                       that lcov read ~100 rows *better* than the tree.
#   --filter PATH      argument passed to lcov-missed.py (default: agent/src)
#   --test-home DIR    HOME for the run (default: <worktree>/target/test-home)
#
# Guard: the run log is judged, never the exit status alone — `cargo test`
# stops at the first failing test *target*, so a red run leaves later targets
# unrun while `cargo llvm-cov report` still succeeds.
set -uo pipefail

PACKAGE="${1:?usage: measure-split.sh PACKAGE OUT.info [--threads N] [--budget S] [--expect-targets N]}"
OUT_BASE="${2:?usage: measure-split.sh PACKAGE OUT.info [--threads N] [--budget S] [--expect-targets N]}"
shift 2

THREADS=8
BUDGET_SECONDS=480
EXPECTED_TARGETS=0
FILTER=agent/src
TEST_HOME=""

while [ $# -gt 0 ]; do
    case "$1" in
        --threads) THREADS="$2"; shift 2 ;;
        --budget) BUDGET_SECONDS="$2"; shift 2 ;;
        --expect-targets) EXPECTED_TARGETS="$2"; shift 2 ;;
        --filter) FILTER="$2"; shift 2 ;;
        --test-home) TEST_HOME="$2"; shift 2 ;;
        *) echo "[split] unknown argument: $1" >&2; exit 64 ;;
    esac
done

WORKTREE=$(git rev-parse --show-toplevel) || exit 1
OUT=$(cd "$(dirname "$OUT_BASE")" && pwd)/$(basename "$OUT_BASE")
PREFIX="${OUT%.info}"

cd "$WORKTREE" || exit 1
ulimit -n 8192
unset CARGO_BUILD_TARGET CARGO_TARGET_DIR CARGO_INCREMENTAL CARGO_PROFILE_DEV_DEBUG
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_UNSTABLE_LLVM_PROFILE_FILE RUST_TEST_THREADS
if [ -z "$TEST_HOME" ]; then
    TEST_HOME="$WORKTREE/target/test-home"
fi
export HOME="$TEST_HOME"
mkdir -p "$HOME"

now() { date +%s; }
load1() { uptime | sed -E 's/.*load averages?: ([0-9.]+).*/\1/'; }
TSV="$PREFIX.timings.tsv"

# The wall clock is only comparable across rounds when the host is not carrying
# another lane: on this box a concurrent instrumented build moved a 200 s suite
# past 400 s. So the 1-minute load goes next to every measurement.
load_start=$(load1)
cores=$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo '?')
echo "[split] package=$PACKAGE cargo=$(command -v cargo) HOME=$HOME threads=$THREADS budget=${BUDGET_SECONDS}s cores=$cores load=$load_start"

# ---- step 1: compile every test target, run nothing -------------------------
t0=$(now)
cargo llvm-cov show-env --sh -p "$PACKAGE" >"$PREFIX.env.sh" 2>/dev/null
# shellcheck disable=SC1090
source "$PREFIX.env.sh"
cargo test -p "$PACKAGE" --no-run >"$PREFIX.compile.log" 2>&1
build_rc=$?
t1=$(now)
build_seconds=$((t1 - t0))
compiled_units=$(grep -c '^   Compiling ' "$PREFIX.compile.log")
echo "[split] compile rc=$build_rc ${build_seconds}s units=$compiled_units"
if [ "$build_rc" -ne 0 ]; then
    echo "[split] REFUSING: the build failed; see $PREFIX.compile.log"
    tail -20 "$PREFIX.compile.log"
    exit 2
fi

# ---- step 2: run the suite, which is the only thing the budget governs ------
if [ "$THREADS" != "default" ]; then
    export RUST_TEST_THREADS="$THREADS"
fi
t2=$(now)
timeout "$BUDGET_SECONDS" cargo llvm-cov -p "$PACKAGE" --no-report --no-fail-fast \
    >"$PREFIX.run.log" 2>&1
run_rc=$?
t3=$(now)
run_seconds=$((t3 - t2))
echo "[split] run rc=$run_rc ${run_seconds}s (budget ${BUDGET_SECONDS}s)"
if [ "$run_rc" -eq 124 ]; then
    echo "[split] BUDGET EXCEEDED: the suite did not finish in ${BUDGET_SECONDS}s"
    exit 4
fi

targets=$(grep -c '^     Running ' "$PREFIX.run.log")
recompiled=$(grep -c '^   Compiling ' "$PREFIX.run.log")
failed_summaries=$(grep -c 'test result: FAILED' "$PREFIX.run.log")
failed_total=$(grep -oE 'test result: (ok|FAILED)\. [0-9]+ passed; [0-9]+ failed' "$PREFIX.run.log" \
    | grep -oE '[0-9]+ failed' | awk '{sum += $1} END {printf "%d", sum + 0}')
passed_total=$(grep -oE 'test result: (ok|FAILED)\. [0-9]+ passed' "$PREFIX.run.log" \
    | grep -oE '[0-9]+ passed' | awk '{sum += $1} END {printf "%d", sum + 0}')
panics=$(grep -c 'panicked at' "$PREFIX.run.log")
echo "[split] targets=$targets recompiled_units=$recompiled passed=${passed_total:-0} failed_summaries=$failed_summaries failed_total=${failed_total:-0} panics=$panics"

if [ "$EXPECTED_TARGETS" -gt 0 ] && [ "$targets" -lt "$EXPECTED_TARGETS" ]; then
    echo "[split] REFUSING: only $targets of $EXPECTED_TARGETS targets ran; the lcov would overstate missed lines"
    grep '^     Running ' "$PREFIX.run.log"
    exit 2
fi
if [ "$EXPECTED_TARGETS" -eq 0 ]; then
    echo "[split] NOTE: no --expect-targets given, so a truncated run cannot be detected from this log"
fi
if [ "$failed_summaries" -gt 0 ] || [ "${failed_total:-1}" -ne 0 ]; then
    echo "[split] RED RUN: refusing to quote this lcov"
    grep -A3 '^failures:$' "$PREFIX.run.log" | head -30
    exit 3
fi
if [ "$run_rc" -ne 0 ]; then
    echo "[split] RED RUN: rc=$run_rc although the log shows no failure"
    exit 3
fi

# ---- step 3: the report, from the profraw the run step just wrote -----------
cargo llvm-cov report -p "$PACKAGE" --lcov --output-path "$OUT" >/dev/null
report_rc=$?
load_end=$(load1)
echo "[split] report rc=$report_rc -> $OUT"
echo "[split] wall: compile=${build_seconds}s run=${run_seconds}s total=$((t3 - t0))s load=${load_start}->${load_end}"

if [ ! -s "$TSV" ]; then
    printf 'started\tform\tcompile_s\trun_s\ttotal_s\tthreads\tload_start\tload_end\ttargets\n' >"$TSV"
fi
printf '%s\tsplit\t%d\t%d\t%d\t%s\t%s\t%s\t%s\n' \
    "$(date '+%Y-%m-%dT%H:%M:%S')" "$build_seconds" "$run_seconds" "$((t3 - t0))" \
    "$THREADS" "$load_start" "$load_end" "$targets" >>"$TSV"
echo "[split] timing row appended to $TSV"

MISSED_TOOL="$WORKTREE/coverage/2026-11/tools/lcov-missed.py"
if [ -f "$MISSED_TOOL" ]; then
    python3 "$MISSED_TOOL" "$OUT" --filter "$FILTER" --top 5
fi
exit "$report_rc"
