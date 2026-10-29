#!/usr/bin/env bash
# shot.sh — capture one workbench frame as a real PNG.
#
#   tui/scripts/shot.sh <scenario> [COLSxROWS] [out.png]
#   tui/scripts/shot.sh <scenario> <out.png>          (the spec's argument form)
#
# A thin wrapper over `scripts/screenshots/capture.py tui`, which is the only
# implementation: the program under test runs in a real pty of the requested
# size and the bytes it writes there are replayed into the picture. There is no
# second path and nothing outside the repository to install.
#
# Two data sources, chosen by the environment:
#
#   * default — `demo_agent`, a scripted gRPC server. No agent, no model, no
#     credentials, no network: `shot.sh chat 120x38` on a fresh clone works, and
#     two runs are byte-identical. These frames land in `.screenshots/tui/`
#     (gitignored), because they are illustrative — not the baseline.
#   * `FUTURE_AGENT_GRPC_ADDR` set (and not `auto`) — that agent, i.e. a *real*
#     session: its transcript, its goal todos, its usage. This is the path the
#     verification recipe uses, and its frames default into `docs/tui/assets/`,
#     the baseline directory. Build and start it first (see below); `SESSION_ID`
#     selects one session.
#
#     export FUTURE_HOME=/tmp/wb-shot
#     FUTURE_VERSION=$(git describe --tags --always)+local \
#       cargo build -q -p future-tui -p future-cli
#     FUTURE_HOME=$FUTURE_HOME ./target/debug/future agent --grpc-addr 127.0.0.1:5511 &
#     export FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5511
#
# Scenarios: chat help palette todo usage skills sessions launchpad scrolled
#            palette-filter full-panel drag light command-usage command-skills
#            rail-search git
# (the table lives in scripts/screenshots/capture.py; an unknown name is an
# error rather than a silently different frame. `git` is the one scenario that
# reads the checkout, so it needs `FUTURE_DEMO_CWD=$PWD` for the scripted
# session to have a repository.)
#
# `SHOT_LANG=en` captures the frame in English: the interface *and* the scripted
# session (its list, its transcript), which the demo agent carries in both
# languages. Default `zh`, the language every committed asset was shot in.
# `SHOT_SIDEBAR_WIDTH=<cols>` pins the rail wider than the surface's own default,
# which is what an English frame usually wants (see `--sidebar-width`).
# Exit codes: 2 = usage or missing binary, 1 = capture failure.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)

usage() {
  sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
}

case "${1:-}" in
  -h|--help) usage; exit 0;;
esac
# A mis-typed size or a flag this script does not take must not be ignored: it
# would silently produce the default frame instead of the one asked for.
if [ "$#" -gt 3 ]; then
  echo "shot.sh: unexpected arguments: ${*:4}" >&2
  exit 2
fi

SCENARIO=${1:-chat}
case "${2:-}" in
  ""|*x*) ;;                                    # size, or nothing at all
  *.png) SCENARIO_OUT=$2; set -- "$1" "" "$2" ;; # the spec's <scenario> <out.png>
esac
SIZE=${2:-120x38}
if [ -n "${FUTURE_AGENT_GRPC_ADDR:-}" ] && [ "$FUTURE_AGENT_GRPC_ADDR" != "auto" ]; then
  LIVE=1
else
  LIVE=0
fi

# Names follow the assets that are already here: the plain chat frame is
# `workbench-<WxH>.png`, every other scenario carries its name. Only a live
# capture defaults into `docs/tui/assets/` — those frames are the visual
# baseline, and letting the scripted demo overwrite them would silently trade a
# real session's record for demo data. A demo frame is illustrative, so it goes
# to the gitignored `.screenshots/`.
if [ "$SCENARIO" = chat ]; then
  NAME=workbench-$SIZE.png
else
  NAME=workbench-$SCENARIO-$SIZE.png
fi
if [ "$LIVE" = 1 ]; then
  DEFAULT_OUT=$ROOT/docs/tui/assets/$NAME
else
  DEFAULT_OUT=$ROOT/.screenshots/tui/$NAME
fi
OUT=${3:-${SCENARIO_OUT:-$DEFAULT_OUT}}

case "$SIZE" in
  [0-9]*x[0-9]*) COLS=${SIZE%x*}; ROWS=${SIZE#*x};;
  *) echo "shot.sh: size must look like 120x38, got '$SIZE'" >&2; exit 2;;
esac

# Where the TUI comes from: an explicit override, else the checkout this script
# lives in (a task worktree has its own target/), else the main checkout — where
# the built binaries are when this runs from a worktree.
TUI=${SHOT_TUI:-}
if [ -z "$TUI" ]; then
  for candidate in "$ROOT/target/debug/future" \
                   "$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir | xargs dirname)/target/debug/future"; do
    if [ -x "$candidate" ]; then TUI=$candidate; break; fi
  done
fi
[ -n "$TUI" ] && [ -x "$TUI" ] || {
  echo "shot.sh: no future binary (set SHOT_TUI, or: cargo build -p future-cli --bin future)" >&2
  exit 2
}

ARGS=(tui "$OUT" --cols "$COLS" --rows "$ROWS" --scenario "$SCENARIO" --bin "$TUI" --lang "${SHOT_LANG:-zh}")
# Only when asked for: unset leaves the rail at the surface's own width, which is
# the layout every committed frame was captured with.
if [ -n "${SHOT_SIDEBAR_WIDTH:-}" ]; then
  ARGS+=(--sidebar-width "$SHOT_SIDEBAR_WIDTH")
fi
if [ "$LIVE" = 1 ]; then
  ARGS+=(--addr "$FUTURE_AGENT_GRPC_ADDR")
  if [ -n "${SESSION_ID:-}" ]; then
    ARGS+=(--session "$SESSION_ID")
  fi
else
  # The scripted agent ships as a cargo example, so it is built from the same
  # checkout as the TUI rather than being committed as a binary.
  AGENT_BIN=${SHOT_AGENT_BIN:-$(dirname "$TUI")/examples/demo_agent}
  [ -x "$AGENT_BIN" ] || {
    echo "shot.sh: no demo agent at $AGENT_BIN (cargo build -p future-tui --example demo_agent)" >&2
    exit 2
  }
  ARGS+=(--agent-bin "$AGENT_BIN")
fi

mkdir -p "$(dirname "$OUT")"
python3 "$ROOT/scripts/screenshots/capture.py" --out "$(dirname "$OUT")" "${ARGS[@]}"
