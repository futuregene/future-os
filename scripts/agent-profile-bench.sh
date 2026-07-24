#!/usr/bin/env bash
# agent-profile-bench.sh — start a profiled agent on port 50052, generate
# load with one-shot CLI prompts, and stop after PROFILE_DURATION seconds
# so the agent writes its flamegraph on shutdown.
#
# Usage:
#   PROFILE_DURATION=90 bash scripts/agent-profile-bench.sh
#
# Expects the profiled binary to already exist (the Makefile `profile-agent`
# target builds it).  Outputs:
#   profile-results/agent-profile-<ts>.svg   flamegraph
#   profile-results/agent-profile-<ts>.log   agent stdout/stderr
set -euo pipefail

DURATION="${PROFILE_DURATION:-90}"
PORT="${PROFILE_PORT:-50052}"
ADDR="127.0.0.1:${PORT}"
BIN="./target/release/future-agent"
TS="$(date +%Y%m%d-%H%M%S)"
SVG="profile-results/agent-profile-${TS}.svg"
LOG="profile-results/agent-profile-${TS}.log"

mkdir -p profile-results

if [[ ! -x "${BIN}" ]]; then
    echo "error: ${BIN} not found — run 'make profile-agent' (it builds first)" >&2
    exit 1
fi

echo "Starting profiled agent on ${ADDR} for ${DURATION}s ..."
"${BIN}" \
    --grpc-addr "${ADDR}" \
    --profile "${SVG}" \
    --profile-seconds "${DURATION}" \
    --verbose >"${LOG}" 2>&1 &
AGENT_PID=$!

# Make sure we never leave the profiled agent running if the script dies.
cleanup() {
    if kill -0 "${AGENT_PID}" 2>/dev/null; then
        kill "${AGENT_PID}" 2>/dev/null || true
        wait "${AGENT_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Wait for the gRPC port to accept connections (max ~15s).
echo -n "Waiting for agent to come up"
for _ in $(seq 1 150); do
    if (echo >"/dev/tcp/127.0.0.1/${PORT}") 2>/dev/null; then
        echo " — up."
        break
    fi
    if ! kill -0 "${AGENT_PID}" 2>/dev/null; then
        echo ""
        echo "error: agent exited during startup — see ${LOG}" >&2
        exit 1
    fi
    echo -n "."
    sleep 0.1
done

# Generate load: a series of one-shot prompts through the profiled agent.
# Skipped silently when the `future` CLI is not installed — the agent still
# profiles idle/shutdown behaviour, and you can drive load manually.
if command -v future >/dev/null 2>&1; then
    echo "Driving load with 'future run' one-shot prompts ..."
    PROMPTS=(
        "Summarise what this repository does in three sentences."
        "List the main entry points of the Rust agent crate."
        "Explain how session persistence works, briefly."
    )
    # Spread the prompts across the profiling window, leaving headroom for
    # startup and the final flamegraph write.
    N=${#PROMPTS[@]}
    STEP=$(( (DURATION - 10) / N ))
    (( STEP < 3 )) && STEP=3
    for p in "${PROMPTS[@]}"; do
        # Stop driving load once the profile timer has expired — a prompt cut
        # short by shutdown is expected, not a failure.
        kill -0 "${AGENT_PID}" 2>/dev/null || break
        echo "  → ${p}"
        if ! future run --grpc-addr "${ADDR}" "${p}" >/dev/null 2>&1; then
            if kill -0 "${AGENT_PID}" 2>/dev/null; then
                echo "    (prompt failed — continuing; see ${LOG})"
            else
                echo "    (cut short by profile timer — expected)"
            fi
        fi
        sleep "${STEP}"
    done
else
    echo "'future' CLI not found — agent will profile idle/shutdown only."
    echo "Install it with 'make install' or drive load manually against ${ADDR}."
fi

echo "Waiting for profile timer to expire and flamegraph to be written ..."
wait "${AGENT_PID}" 2>/dev/null || true
trap - EXIT

if [[ -s "${SVG}" ]]; then
    echo "Done: ${SVG}"
else
    echo "warning: flamegraph not found at ${SVG} — check ${LOG}" >&2
    exit 1
fi
