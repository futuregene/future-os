#!/usr/bin/env bash
#
# tmux screen-consistency harness: Rust TUI vs committed goldens.
#
# Requirements include tmux >= 3.5 for `paste-buffer -p` (bracketed paste) —
# see `paste_file`.
#
# The TypeScript TUI was retired (2026-08, same as the CLI); the golden files
# under `tui/tests/golden/` started as captures of the TS pane (carried into
# this tree by the port commit 1467a1cd) and are now recorded from the Rust
# pane. This harness runs the Rust TUI in a tmux pane connected to a
# deterministic mock agent (examples/mock_agent), drives it with keystrokes,
# and byte-compares `capture-pane -e` screens against the goldens — a
# divergence in the port (or an intentional screen change, which must be
# committed together with re-recorded goldens) is caught.
#
# This is the P4 screen-consistency gate for the Rust TUI port. It covers the
# chat itself (welcome banner, typing, the streamed reply with its tool call,
# ctrl+g tool-body expand/collapse), the footer/status readouts, and every
# panel the port added: the help card (clipped with its fold row at 80x36, the
# whole card at 80x72, and scrolled to the end at 80x36), the model selector,
# the session list, the model-scope and tool menus, the providers list (both
# tabs and the edit form), the skill browser (list, search-filtered, the filter
# cleared by an escape, catalogue rows), the sandbox/permission panel (before
# and after applying a tier and a permission level), the theme picker (before
# and after `/theme light`), and the `/usage`, `/transcript` (+ a real
# PageDown/PageUp and the search editor), `/agent`, `/metrics`,
# `/snapshot`, `/tool-output` (list and diff body) and `/history` readouts.
#
# Beyond byte-comparing screens, the scenarios assert the *transitions* a
# golden cannot state — that the first escape kept a filtered panel open and
# cleared its query, that PageDown moved the pager's viewport and PageUp
# brought it back, that the second escape closed them. Those assertions run in
# both modes and are reported as `ASSERT-FAIL`; `--record` refuses to freeze a
# screen whose own assertion failed.
#
# Requirements:
#   - tmux (panes provide the PTYs)
#   - rustup with the pinned toolchain (rust-toolchain.toml) — builds the
#     Rust TUI (future-tui) and the mock agent example
#
# Usage:
#   make test-tui-tmux          # verify mode (goldens must match)
#   tui/tests/tmux-diff.sh
#   tui/tests/tmux-diff.sh --record     # rewrite goldens from the Rust pane
#   tui/tests/tmux-diff.sh --verbose    # show failing diffs
#   tui/tests/tmux-diff.sh --keep       # keep /tmp/future-tui-tmux-* artifacts
#
# Golden files: tui/tests/golden/<scenario>.txt

set -euo pipefail

VERBOSE=0
KEEP=0
RECORD=0
for arg in "$@"; do
  case "$arg" in
    --record) RECORD=1 ;;
    --verbose) VERBOSE=1 ;;
    --keep) KEEP=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# Resolve repo root from this script's location: tui/tests/.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TUI_DIR="$ROOT/tui"
GOLDEN_DIR="$TUI_DIR/tests/golden"

RUST_TUI="$ROOT/target/debug/future-tui"
MOCK_AGENT="$ROOT/target/debug/examples/mock_agent"

SESSION="tui-diff-$$"
WINDOW="${SESSION}:0"
RUST_PANE="${WINDOW}.0"

WORK="$(mktemp -d /tmp/future-tui-tmux-XXXXXX)"
RUST_HOME="$WORK/home-rust"
BIN_DIR="$WORK/bin"
mkdir -p "$RUST_HOME"
RUST_OUT="$WORK/rust.out"
PANE_PID=""
TUI_PID=""
# Set once the mock's port is picked; the cleanup sweeps for it (see
# `reap_pane_tui`). Empty until then so a preflight exit cannot trip `set -u`.
PORT_PATTERN=""
# Killing the tmux session closes the pane and SIGHUPs its process group, but a
# TUI whose terminal is gone keeps running — it ignores SIGHUP *and*
# SIGINT/SIGTERM (its exit path needs the event loop, which stops progressing)
# — so it would leak one orphan per run, still holding its gRPC connection.
# The pane's child is therefore SIGKILLed explicitly (see `TUI_PID` below).
#
# The kill is *scoped*: first the pane's own child (an exact ppid lookup), then a
# sweep for this run's gRPC address. The port is picked per run, so that pattern
# can never match a concurrent worker's TUI — several sessions work in this repo
# at once and a blanket `pkill future-tui` would take theirs down.
reap_pane_tui() {
  local pids
  if [ -z "$PORT_PATTERN" ]; then
    # The run died before the mock was started (a preflight failure): there is
    # nothing to reap and no port to scope the sweep to.
    return 0
  fi
  pids="$(pgrep -f "$PORT_PATTERN" 2>/dev/null || true)"
  if [ -n "$pids" ]; then
    kill -9 $pids 2>/dev/null || true
  fi
  pids="$(pgrep -f "$PORT_PATTERN" 2>/dev/null || true)"
  if [ -n "$pids" ]; then
    echo "cleanup: WARNING: still alive after SIGKILL: $pids" >&2
  else
    echo "cleanup: nothing left on '$PORT_PATTERN' (pane TUI and mock both gone)"
  fi
}
cleanup() {
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  [ -n "$TUI_PID" ] && kill -9 "$TUI_PID" 2>/dev/null || true
  reap_pane_tui
  kill "${MOCK_A_PID:-}" 2>/dev/null || true
  # Reap the mock so the shell does not print its own "Terminated: 15" job
  # notice after the result line (it looks like a failure in a CI log).
  wait "${MOCK_A_PID:-}" 2>/dev/null || true
}
if [ "$KEEP" -eq 0 ]; then
  trap 'cleanup; rm -rf "$WORK"' EXIT
else
  trap 'cleanup; echo "kept $WORK"' EXIT
fi

echo "== TUI screen consistency: Rust vs golden (tmux) =="
echo "work:   $WORK"
[ "$RECORD" -eq 1 ] && echo "mode:   RECORD (goldens <- Rust pane)"
echo "golden: $GOLDEN_DIR"

# ── Preflight ───────────────────────────────────────────────────────────────
if ! command -v tmux >/dev/null 2>&1; then
  echo "SKIP: tmux not found — tmux screen-consistency test requires an interactive"
  echo "      terminal server (run locally, not in headless CI)."
  exit 0
fi
mkdir -p "$GOLDEN_DIR"

# ── Build Rust TUI + mock agent ─────────────────────────────────────────────
echo "-- build future-tui + mock_agent --"
(cd "$ROOT" && rustup run 1.97.0 cargo build -q -p future-tui --bin future-tui)
(cd "$ROOT" && rustup run 1.97.0 cargo build -q -p future-tui --example mock_agent)
[ -x "$RUST_TUI" ] || { echo "FATAL: $RUST_TUI missing" >&2; exit 1; }
[ -x "$MOCK_AGENT" ] || { echo "FATAL: $MOCK_AGENT missing" >&2; exit 1; }

# ── The pane's binary + the one subprocess stand-in it needs ────────────────
#
# The pane runs a *copy* of the TUI from $BIN_DIR rather than $RUST_TUI,
# because `/skills` shells out to `future skills list --json` and that binary
# is resolved as `<exe_dir>/future` first (tui/src/skills_cli.rs,
# `future_binary_candidates`). Next to $RUST_TUI a developer box usually has
# either `target/debug/future` (built by another session) or the installed
# `future` on PATH; either would win, run against the isolated HOME, hang
# until the skill-op timeout and then drop a wall-clock-dependent error line
# into the chat — which the panel goldens (a 76-column card at 80 columns, so
# the two left columns show the chat behind it) would capture differently on
# every run. Beside the copy, the stub below is the first candidate on every
# host, so the catalogue is a fixed document.
#
# The sidecar agent uses the same `<exe_dir>/future` lookup, but the TUI is
# launched with an explicit `--grpc-addr` onto the running mock, so it never
# reaches for one.
mkdir -p "$BIN_DIR"
cp "$RUST_TUI" "$BIN_DIR/future-tui"
cat > "$BIN_DIR/future" <<'FUTURE_STUB'
#!/usr/bin/env bash
# Deterministic stand-in for the unified `future` binary. The only subprocess
# the harnessed TUI runs is the skills catalogue, and it gets a fixed document:
# one row with a newer catalogue version (the ⬆ upgrade row), one current row
# and one catalogue-only row (`not installed`), with no network, disk or clock
# involved. Anything else is a harness bug and fails loudly.
if [ "${1:-}" = "skills" ] && [ "${2:-}" = "list" ] && [ "${3:-}" = "--json" ]; then
  printf '%s\n' '{"skills":[{"id":"code-review","name":"code-review","latestVersion":"1.0","installedVersion":"1.0","description":"Review code and diffs","descriptionZh":"审查代码差异的正确性"},{"id":"future-web","name":"future-web","latestVersion":"1.3","installedVersion":"1.2","description":"Search and fetch web pages","descriptionZh":"搜索并抓取网页"},{"id":"pdf-tools","name":"pdf-tools","latestVersion":"1.4","description":"Extract text from PDF files","descriptionZh":"从 PDF 文件提取文本"}],"count":3}'
  exit 0
fi
echo "future stub: not implemented: $*" >&2
exit 2
FUTURE_STUB
chmod +x "$BIN_DIR/future"
# Preflight the stub itself: a stub that cannot answer would leave `/skills`
# without a catalogue (or, worse, let the real binary through on another
# host), and the failure would only show up as a mysterious drift later.
STUB_CATALOGUE="$("$BIN_DIR/future" skills list --json)" || {
  echo "FATAL: the $BIN_DIR/future stub did not answer 'skills list --json'" >&2
  exit 1
}
case "$STUB_CATALOGUE" in
  *'"count":3'*) ;;
  *) echo "FATAL: unexpected stub catalogue: $STUB_CATALOGUE" >&2; exit 1 ;;
esac

# ── Free port for the mock agent ────────────────────────────────────────────
pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}
PORT_A="$(pick_port)"
PORT_PATTERN="grpc-addr 127.0.0.1:$PORT_A"

# ── Start mock agent (deterministic) ────────────────────────────────────────
"$MOCK_AGENT" --port "$PORT_A" > "$WORK/mock-a.log" 2>&1 &
MOCK_A_PID=$!
wait_port() {
  local port="$1" i
  for i in $(seq 1 50); do
    if python3 -c "import socket,sys; s=socket.socket(); s.settimeout(0.2); sys.exit(0 if s.connect_ex(('127.0.0.1',$port))==0 else 1)" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}
wait_port "$PORT_A" || { echo "FATAL: mock agent (port $PORT_A) did not start" >&2; exit 1; }

# ── Start tmux session: one 80x36 pane ─────────────────────────────────────
#
# `TZ=UTC` is part of the fixture, not a debug convenience: `/history` stamps
# every match with its local `HH:MM:SS` (the agent sends epoch millis), and the
# mock's timestamps are fixed, so the *only* thing that could move that readout
# between two runs — this machine and CI — is the box's time zone. Pinning the
# zone for the pane keeps the golden byte-comparable on any host. The product
# still renders the user's own zone: nothing in the TUI reads `TZ`.
tmux kill-session -t "$SESSION" 2>/dev/null || true
tmux new-session -d -s "$SESSION" -x 80 -y 36 \
  "cd $ROOT && HOME=$RUST_HOME TZ=UTC PATH=$BIN_DIR:\$PATH $BIN_DIR/future-tui --grpc-addr 127.0.0.1:$PORT_A; echo RUST_EXIT=\$? > $WORK/rust.exit"

# Sanity: the pane must be exactly 80x36.
PANE_SIZES="$(tmux list-panes -t "$WINDOW" -F '#{pane_width}x#{pane_height}' | sort -u)"
if [ "$PANE_SIZES" != "80x36" ]; then
  echo "FATAL: unexpected pane sizes: $PANE_SIZES (want 80x36)" >&2
  exit 1
fi

# The pane's shell is `sh -c '… future-tui …'`; note the TUI below it so the
# cleanup can kill it (see `cleanup`). SIGKILL, not SIGTERM: the TUI installs
# handlers for SIGINT/SIGTERM/SIGHUP, but they only take effect through the
# event loop, which stops progressing once the pane's terminal is gone — a TUI
# whose session was killed ignores all three and has to be SIGKILLed (checked
# by hand against a leaked 80x36 pane).
PANE_PID="$(tmux list-panes -t "$RUST_PANE" -F '#{pane_pid}')"
for _ in $(seq 1 50); do
  TUI_PID="$(pgrep -P "$PANE_PID" -f 'future-tui' 2>/dev/null | head -1 || true)"
  if [ -n "$TUI_PID" ]; then
    break
  fi
  sleep 0.1
done

capture() { # $1 = out file
  tmux capture-pane -t "$RUST_PANE" -p -e > "$1"
}

# Wait for the welcome screen (banner "future-tui v0.0.0-mock").
wait_welcome() {
  local i
  for i in $(seq 1 300); do
    if tmux capture-pane -t "$RUST_PANE" -p 2>/dev/null | grep -q "future-tui v"; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

# Wait for a literal substring to appear in the pane (15 s). Panels and the
# streamed reply arrive asynchronously, so a scenario polls for its own text
# instead of guessing a sleep.
wait_text() { # $1 = literal substring
  local i
  for i in $(seq 1 150); do
    if tmux capture-pane -t "$RUST_PANE" -p 2>/dev/null | grep -qF "$1"; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

echo "-- starting TUI --"
wait_welcome || { echo "FATAL: Rust TUI never showed welcome" >&2; exit 1; }
sleep 1   # let deferred renders settle

# ── Paste / image fixtures ──────────────────────────────────────────────────
#
# The two things this harness has to paste for real: a log-like block past the
# fold threshold, and a path to an actual image file. Both are generated here
# (no fixture files in the tree) and both are byte-deterministic — the paste's
# *character count* is part of the golden.
#
# Python is already a harness requirement (`pick_port`, `pane_fingerprint`), and
# it is also the portable way to write a real PNG: `base64 -d` spells its
# decode flag differently on BSD and GNU.
PASTE_FILE="$WORK/paste.txt"
python3 - "$PASTE_FILE" "$WORK/shot.png" "$WORK/other.png" <<'PY'
import struct, sys, zlib

body = "".join(f"line-{i:03d} {'x' * 30}\n" for i in range(40))
body += "PASTE-TAIL-SENTINEL"          # no trailing newline: nothing to strip
with open(sys.argv[1], "w") as handle:
    handle.write(body)
assert len(body) > 1000 and len(body) == len(body.encode())


def chunk(tag, data):
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data))
    )


ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
for target, colour in ((sys.argv[2], b"\xff\x00\x00"), (sys.argv[3], b"\x00\x00\xff")):
    raw = b"\x00" + colour
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )
    with open(target, "wb") as handle:
        handle.write(png)
PY
PASTE_CHARS="$(python3 -c 'import sys; print(len(open(sys.argv[1]).read()))' "$PASTE_FILE")"
echo "fixtures: paste=$PASTE_CHARS chars, images=$WORK/shot.png $WORK/other.png"

# ── Scenario runner ─────────────────────────────────────────────────────────
PASS=0
FAIL=0
# Assertion failures that are not a golden comparison (see `expect_*`).
AFAIL=0

# Recording must never freeze a screen that is known-bad. `step`, `step_when`
# and the assertions all bail out through here instead of writing a golden that
# would then pass forever; the EXIT trap reaps the session, the pane's TUI and
# the mock.
die_record() { # $1 = scenario, $2 = reason
  echo "FATAL[$1]: $2" >&2
  exit 1
}

step() { # $1 = scenario name
  local name="$1" golden="$GOLDEN_DIR/$1.txt"
  sleep 1
  capture "$RUST_OUT"

  if [ "$RECORD" -eq 1 ]; then
    # A screen that reports a failure must never become a golden: the gate
    # would then pass forever on a broken panel. The patterns are the TUI's own
    # failure phrasings — `Failed to …` / `Error: …` chat messages (both are
    # their own line, so an `eventJournalError:` counter in the metrics pager
    # does not match), a Rust panic, and the stub's unimplemented branch. The
    # mock's own data may say "failed" (the `call_mock_2 shell failed` row).
    if grep -qE '^ ?(Failed to|Error: )|panicked at|not implemented:' "$RUST_OUT"; then
      grep -nE '^ ?(Failed to|Error: )|panicked at|not implemented:' "$RUST_OUT" | head -3 >&2
      die_record "$name" "the pane reports a failure — refusing to record it"
    fi
    cp "$RUST_OUT" "$golden"
    echo "OK[$name]: recorded golden ($(wc -c < "$golden") bytes)"
    PASS=$((PASS+1))
    return
  fi

  mkdir -p "$WORK/actual"
  cp "$RUST_OUT" "$WORK/actual/$name.txt"

  if [ ! -f "$golden" ]; then
    echo "FAIL[$name]: golden missing: $golden (run with --record first)"
    FAIL=$((FAIL+1))
    return
  fi
  if ! diff -q "$RUST_OUT" "$golden" >/dev/null; then
    echo "FAIL[$name]: Rust pane drifted from golden ($WORK/actual/$name.txt)"
    if [ "$VERBOSE" -eq 1 ]; then
      # `|| true`: `diff` exits 1 when the files differ and `set -o pipefail`
      # propagates that through `head`, so without this the shell's `set -e`
      # killed the whole run at the *first* drifting scenario — the survey mode
      # showed one diff and called it a day.
      diff -u "$golden" "$RUST_OUT" | head -60 || true
    fi
    FAIL=$((FAIL+1))
    return
  fi
  echo "OK[$name]: Rust == golden ($(wc -c < "$golden") bytes)"
  PASS=$((PASS+1))
}

# `step` for a screen that is painted from data the mock still has to deliver
# (a panel, a pager, the streamed reply): poll for the panel's own text first,
# then compare. In `--record` a screen that never painted is a hard failure —
# an empty or erroring panel must not be frozen as "passing".
step_when() { # $1 = scenario name, $2 = literal substring that proves it painted
  if ! wait_text "$2"; then
    if [ "$RECORD" -eq 1 ]; then
      die_record "$1" "'$2' never rendered — refusing to record a screen without it"
    fi
    echo "WARN[$1]: '$2' never rendered — comparing the pane anyway"
  fi
  step "$1"
}

# Like `step_when`, but for a screen that is a *stepping stone*: poll for its
# text (its golden is captured elsewhere, or its point is the transition the
# assertions below measure) without capturing anything.
require_text() { # $1 = scenario label, $2 = literal substring
  if ! wait_text "$2"; then
    if [ "$RECORD" -eq 1 ]; then
      die_record "$1" "'$2' never rendered"
    fi
    echo "WARN[$1]: '$2' never rendered"
  fi
}

# ── Assertions ──────────────────────────────────────────────────────────────
# The goldens pin the pixels; they cannot state a *transition* ("PageDown moved
# the viewport", "the first escape kept the panel open", "the filter is gone").
# Those are asserted here, in both modes: verify mode counts a failure and
# carries on; `--record` refuses to freeze a screen whose own assertion failed.
pane_text() { tmux capture-pane -t "$RUST_PANE" -p 2>/dev/null | sed -e $'s/\x1b\\[[0-9;]*m//g'; }
ok()  { echo "  OK[$1]: $2"; }
bad() { # $1 = scenario, $2 = what failed
  echo "ASSERT-FAIL[$1]: $2" >&2
  AFAIL=$((AFAIL+1))
  if [ "$RECORD" -eq 1 ]; then
    die_record "$1" "refusing to record a screen that failed its own assertion"
  fi
}
expect_eq() { # $1 scenario, $2 label, $3 wanted, $4 got
  if [ "$3" = "$4" ]; then ok "$1" "$2 ($4)"; else bad "$1" "$2: want '$3', got '$4'"; fi
}
expect_pane_has() { # $1 scenario, $2 literal, $3 label
  if pane_text | grep -qF "$2"; then ok "$1" "$3"; else bad "$1" "$3: '$2' is not on screen"; fi
}
expect_pane_has_when() { # $1 scenario, $2 literal, $3 label — waits for a repaint
  if wait_text "$2"; then ok "$1" "$3"; else bad "$1" "$3: '$2' is not on screen"; fi
}
expect_pane_lacks() { # $1 scenario, $2 literal, $3 label
  if pane_text | grep -qF "$2"; then bad "$1" "$3: '$2' is still on screen"; else ok "$1" "$3"; fi
}

# The pager's own scroll readout (`… · q close      0%`). Only the pager paints
# that hint row, so the percentage is a readout of the viewport and the honest
# evidence for a page key — the screen alone cannot say whether a key arrived.
pager_percent() {
  pane_text | grep -F 'space/b page' | grep -oE '[0-9]+%' | head -1 || true
}
# The same readout as a bare number, so a scenario can compare positions without
# knowing the page step (one page = viewport − 1 rows, clamped to the last page,
# so the step is a property of the content, not a constant).
pager_percent_num() { local p; p="$(pager_percent)"; printf '%s' "${p%\%}"; }
wait_pager_percent_gt() { # $1 = a lower bound; polls until the readout is above it
  local i cur
  for i in $(seq 1 60); do
    cur="$(pager_percent_num)"
    if [ -n "$cur" ] && [ "$cur" -gt "$1" ]; then return 0; fi
    sleep 0.1
  done
  return 1
}
wait_pager_percent_lt() { # $1 = an upper bound; polls until the readout is below it
  local i cur
  for i in $(seq 1 60); do
    cur="$(pager_percent_num)"
    if [ -n "$cur" ] && [ "$cur" -lt "$1" ]; then return 0; fi
    sleep 0.1
  done
  return 1
}

# A screen's byte fingerprint (trailing whitespace normalised, ANSI included):
# the second half of "did the viewport move?", and the only evidence a key press
# cannot fake — a readout could be repainted while the body stayed put.
pane_fingerprint() {
  pane_text | sed -e 's/[[:space:]]*$//' \
    | python3 -c 'import hashlib,sys; print(hashlib.md5(sys.stdin.buffer.read()).hexdigest()[:16])'
}
wait_fingerprint_changed() { # $1 = the fingerprint the screen had before the key
  local i
  for i in $(seq 1 60); do
    if [ "$(pane_fingerprint)" != "$1" ]; then return 0; fi
    sleep 0.1
  done
  return 1
}

send_lit() { tmux send-keys -t "$RUST_PANE" -l "$1"; }

# Paste `text` the way a terminal emulator pastes: wrapped in the bracketed
# paste markers the TUI asked for (mode 2004). This is the only paste path that
# folds — a key-by-key replay would arrive as ~1600 separate edits — and
# `paste-buffer -p` (tmux >= 3.5) is what adds the markers for real. Sending the
# markers by hand with `send-keys -l` does *not* work: tmux drops the newlines
# out of a literal multi-line string, which changes the very thing under test.
#
# The caller names the literal the box must show afterwards, so a tmux that
# cannot do this fails loudly instead of recording a screen where the paste
# never happened.
paste_text() { # $1 = text to paste, $2 = literal the input box must show
  tmux set-buffer -b tui-paste "$1"
  tmux paste-buffer -p -b tui-paste -t "$RUST_PANE"
  if ! wait_text "$2"; then
    echo "FATAL: bracketed paste never showed '$2' — 'paste-buffer -p' needs tmux >= 3.5" >&2
    exit 1
  fi
}

# Deterministic slash-command submission. Typing "/cmd" triggers the
# autocomplete popup after a 20 ms debounce; a rapid Enter would be consumed
# by the popup (applying the selection) instead of submitting. So: wait for
# the popup, Enter (applies selection, closes popup), Enter (submits).
# If the popup never appeared, the first Enter submits and the second is a
# no-op — either way exactly one submission happens.
submit_cmd() {
  send_lit "$1"
  sleep 0.5
  tmux send-keys -t "$RUST_PANE" Enter
  sleep 0.3
  tmux send-keys -t "$RUST_PANE" Enter
}

# Exactly one Escape, then a pause. Two Escapes sent back to back arrive as a
# single Alt+Escape sequence and close nothing, and a panel with a form on top
# needs one Escape per level (`close_overlay` twice).
close_overlay() {
  tmux send-keys -t "$RUST_PANE" Escape
  sleep 0.5
}

# The help card is 64 rows tall and is clipped by an 80x36 pane, so the
# commands below the fold are only comparable at a taller window. Resize, wait
# for the pane to actually be that size, capture, resize back.
resize_pane() { # $1 = rows (width stays 80)
  local rows="$1" i size
  tmux resize-window -t "$WINDOW" -x 80 -y "$rows"
  for i in $(seq 1 50); do
    size="$(tmux display-message -p -t "$RUST_PANE" '#{pane_width}x#{pane_height}')"
    if [ "$size" = "80x$rows" ]; then
      return 0
    fi
    sleep 0.1
  done
  echo "FATAL: pane did not resize to 80x$rows (got $size)" >&2
  exit 1
}

# ── Scenarios ───────────────────────────────────────────────────────────────
# One TUI session, driven in a fixed order: every screen sits on top of the
# chat the earlier steps produced, so the order is part of the golden.
step welcome

send_lit "hello"
step typed

tmux send-keys -t "$RUST_PANE" Enter
# The sentinel is the reply's own last paragraph: the tool body no longer
# carries an expand hint to wait for (see the assertion below).
step_when reply "deterministic reply"
# A call is one row while collapsed — the call itself, with the diff's `+N -M`
# badge on it. No preview and no hint: the body is what ctrl+g is for. Only a
# *failed* call keeps its body, so the reason is readable without a key.
expect_pane_lacks reply "ctrl+g to expand" "a collapsed tool call shows no output body"

# ctrl+g expands the tool-result body (the unified diff, with its line-number
# gutter), and collapses it again so the screens below are recorded against
# the collapsed body.
tmux send-keys -t "$RUST_PANE" C-g
step_when tool-expanded "Hello, world!"

tmux send-keys -t "$RUST_PANE" C-g
step tool-collapsed

submit_cmd "/status"
# `cost:` is the ledger's last row, so it is the sentinel for "the whole card is
# on screen". The card used to end at `Streaming:`; `/stats` was folded into it
# (the counters have one home now), so the old sentinel would have matched while
# the ledger below it was still missing.
step_when status "cost: 8.42"

# `help-overlay` is the card at 80x36 with its fold row; `help-full` is the whole
# card, and it has to resize *before* the command: `/help` renders a window whose
# height is fixed when the overlay opens (`HelpOverlay::max_rows` = the
# terminal's rows at that moment), so resizing with the card already open keeps
# the 36-row window (measured: the card stayed 36 rows tall, centred, at 80x72).
# `help-scrolled` then covers the third path — scrolling at 80x36, the only way
# the commands below the fold (`/usage`…) are reachable in a short pane.
submit_cmd "/help"
step help-overlay
close_overlay
step help-closed

resize_pane 72
submit_cmd "/help"
step_when help-full "/usage"
close_overlay
resize_pane 36

submit_cmd "/help"
require_text help-scrolled "show all commands"
tmux send-keys -t "$RUST_PANE" PageDown
# "the end of the card" is the *shape* of the hint row, not a row count: the
# count moves whenever a command is added to the card (`/keymap` did), and a
# hard-coded number here then measures the last edit to `help_screen.rs` rather
# than the behaviour this asserts. At the end the row keeps "↑ N above" and
# loses the "↓ N more" half.
expect_pane_has_when help-scrolled "above · ↑↓ scroll" "PageDown reached the end of the card"
expect_pane_lacks help-scrolled "more · ↑↓ scroll" "nothing is left below the fold"
step_when help-scrolled "/usage"
close_overlay

submit_cmd "/model"
step_when model-overlay "Select Model"
close_overlay
step model-closed

submit_cmd "/sessions"
step_when sessions-overlay "Sessions"
close_overlay
step sessions-closed

submit_cmd "/models"
step_when models-overlay "Model Scope"
close_overlay
step models-closed

submit_cmd "/tools"
step_when tools-overlay "enter apply"
close_overlay
step tools-closed

# ── Providers: built-in tab, custom tab, the edit form, closed ──
submit_cmd "/providers"
step_when providers-builtin "900 models"

tmux send-keys -t "$RUST_PANE" Tab
step_when providers-custom "Acme Cloud"

tmux send-keys -t "$RUST_PANE" Enter
step_when providers-form "Edit provider · acme"

close_overlay   # cancel the form, back to the list
close_overlay   # close the panel
step providers-closed

# ── Skills: the merged list, then the same list filtered ──
# `not installed` is the catalogue-only row: waiting for it (rather than for
# the panel itself) is what proves the agent's list *and* the `future skills
# list --json` catalogue have both landed.
submit_cmd "/skills"
step_when skills-overlay "not installed"

send_lit "web"
step_when skills-filtered "Skills (1/"

# `escape` belongs to the panel while its filter is live (80d52a6d): the first
# one clears the query and keeps the panel the second closes it. The two screens
# around it are the same either way, so the assertion — not the golden — is what
# tells the behaviours apart.
close_overlay
expect_pane_lacks skills-escape "/ web" "the first escape cleared the filter"
step_when skills-escape "Skills (4) · All"
close_overlay
expect_pane_lacks skills-closed "Skills (" "the second escape closed the panel"
step skills-closed

# ── Sandbox / permissions: the panel, then a tier applied ──
submit_cmd "/sandbox"
step_when sandbox-overlay "Sandbox & permissions"

tmux send-keys -t "$RUST_PANE" Up
tmux send-keys -t "$RUST_PANE" Enter
step_when sandbox-tier "current: Manual"

close_overlay

# `/permission` with no argument opens the same card as `/sandbox` with the
# focus moved into the permission half (b84fa0a1, D4): the first selectable row
# is `All`, so one `↓` lands on `Workspace`. It used to take three presses —
# they started from the tier rows above — and those three now clamp on the last
# row (`None`), which is why this scenario applies `Workspace` deliberately
# instead of by counting presses that no longer mean the same thing.
submit_cmd "/permission"
step_when permission-overlay "Tool permissions"

tmux send-keys -t "$RUST_PANE" Down
tmux send-keys -t "$RUST_PANE" Enter
step_when permission-applied "current: Workspace"
expect_pane_has permission-applied "Tool permission level: Workspace" \
  "the applied level reached the chat"

close_overlay

# ── Theme: the picker, then light applied and dark restored ──
submit_cmd "/theme"
step_when theme-overlay "High Contrast"

close_overlay

submit_cmd "/theme light"
step_when theme-light "Theme set to light"

submit_cmd "/theme dark"
step_when theme-dark-restored "Theme set to dark"

# ── Readouts: /usage, /transcript (+ search), /agent, /metrics, ──
# ── /snapshot, /tool-output (list + one diff body), /history ──
submit_cmd "/usage"
step_when usage-overlay "Usage · mock-model"
close_overlay

submit_cmd "/transcript"
step_when transcript-pager "y copy"

# `tmux send-keys PageDown` sends \x1b[6~ — the bytes a real terminal sends for
# that key, which `keys::parse_key` names `pageDown` (camelCase). Until 2d488446
# the pager matched only the lowercase `pagedown` its own unit tests handed it,
# so PageDown was dead in a real terminal.
#
# The pager's status row carries its own scroll readout (`… · q close      0%`) —
# the honest evidence for a page key, since the body alone cannot say whether the
# key arrived. The *step* is content-dependent (one page = viewport − 1 rows,
# clamped to the last page), so nothing below assumes a step or an endpoint: the
# assertions are "the readout grew", "the screen's bytes changed" and "PageUp
# walks it back". Measured on a longer transcript (12 warm-up exchanges, the same
# pager): `0 → 67 → 100`, where one PageUp from the bottom lands on `33` — i.e. a
# hard-coded `PageUp → 0%` would report a defect that is not there.
PAGER_TOP_PCT="$(pager_percent_num)"
PAGER_TOP_FP="$(pane_fingerprint)"
expect_eq transcript-page-down "the pager opens at the top" "0" "$PAGER_TOP_PCT"

tmux send-keys -t "$RUST_PANE" PageDown
if wait_pager_percent_gt "$PAGER_TOP_PCT" && wait_fingerprint_changed "$PAGER_TOP_FP"; then
  ok transcript-page-down "PageDown moved the viewport ($PAGER_TOP_PCT% -> $(pager_percent_num)%)"
else
  bad transcript-page-down "PageDown did not move the pager (still $(pager_percent_num)%)"
fi
expect_pane_lacks transcript-page-down "future-tui v0.0.0-mock" "the first screen scrolled off"
step transcript-page-down

# A second page: it either advances again or is already clamped at the bottom.
# The readout must never go backwards, and when the first page reached the end
# the screen must not move at all (the pager's documented clamp).
PAGER_MID_PCT="$(pager_percent_num)"
PAGER_MID_FP="$(pane_fingerprint)"
tmux send-keys -t "$RUST_PANE" PageDown
sleep 0.5
PAGER_END_PCT="$(pager_percent_num)"
if [ -n "$PAGER_END_PCT" ] && [ "$PAGER_END_PCT" -ge "$PAGER_MID_PCT" ]; then
  ok transcript-page-down "a second PageDown never scrolls backwards ($PAGER_MID_PCT% -> $PAGER_END_PCT%)"
else
  bad transcript-page-down "a second PageDown scrolled backwards ($PAGER_MID_PCT% -> ${PAGER_END_PCT:-<none>}%)"
fi
if [ "$PAGER_MID_PCT" = "100" ]; then
  # One page was already the whole content: the second must be a no-op.
  expect_eq transcript-page-down "the last page is clamped" "$PAGER_MID_FP" "$(pane_fingerprint)"
fi

tmux send-keys -t "$RUST_PANE" PageUp
if wait_pager_percent_lt "$PAGER_END_PCT" && wait_fingerprint_changed "$PAGER_MID_FP"; then
  ok transcript-page-up "PageUp walked the viewport back ($PAGER_END_PCT% -> $(pager_percent_num)%)"
else
  bad transcript-page-up "PageUp did not move the pager (at $(pager_percent_num)%)"
fi
if [ "$(pager_percent_num)" = "0" ]; then
  expect_pane_has transcript-page-up "future-tui v0.0.0-mock" "PageUp reached the top, so the first screen is back"
fi
step transcript-page-up

send_lit "/"
send_lit "theme"
step_when transcript-search "] /theme"
# The `/` search is a vim-style editing state: the first escape closes the
# editor and keeps the query (and the pager), the second closes the pager. It
# used to be a single escape; when that changed, the one escape left here
# silently typed `/stats` into an open pager and every scenario after it
# recorded the transcript instead of the panel it claims (found by this
# re-record: `stats` was showing a search for "stats" over the transcript).
close_overlay
expect_pane_has transcript-search "space/b page" "the first escape kept the pager open"
close_overlay
expect_pane_lacks transcript-search "space/b page" "the second escape closed the pager"

# `/stats` used to have a scenario here. The panel is gone — its counters are a
# section of `/status`, which the preamble above already waits for in full (its
# sentinel is the ledger's last row), so a scenario that opened the same numbers
# in a second pager would have nothing left to assert.

submit_cmd "/agent"
step_when agent "Agent information:"
close_overlay

submit_cmd "/metrics"
step_when metrics "Runtime metrics"
close_overlay

submit_cmd "/snapshot"
step_when snapshot "Run snapshot"
close_overlay

submit_cmd "/tool-output"
step_when tool-output-list "Tool calls in this run:"
close_overlay

submit_cmd "/tool-output call_mock_1"
step_when tool-output-diff "Output of call_mock_1"
close_overlay

# `/history` renders one three-row block per match (time/role/tool, the snippet
# with every hit highlighted, then the dimmed ids) and its header is the
# sentinel. The header is the only stable part: the timestamps come from the
# mock's fixed millis through the pane's zone (`TZ=UTC` above), and the
# `[1m…[48;5;237m` mark inside `hello` is the pager's own search-hit colour —
# the pane is where "the match is actually visible" can be byte-checked.
submit_cmd "/history hello"
step_when history "History matches for 'hello': 3"
close_overlay

# ── Pasted text folds, and the whole paste is what goes out ──
#
# The box holds one placeholder instead of 40 wrapped rows, and after Enter the
# transcript shows the *whole* pasted text: the mock echoes the prompt it
# received as `user_message`, so the pane is the wire. `paste-sent` asserts the
# tail marker is on screen and the placeholder is not, which is the difference
# between "the paste was expanded" and "the user sent a placeholder".
# `$(cat …)` drops the fixture's trailing newline, which is why it does not
# have one: the placeholder's character count is then the file's own length.
paste_text "$(cat "$PASTE_FILE")" "[Pasted Content $PASTE_CHARS chars]"
step paste-folded
expect_pane_has paste-folded "[Pasted Content $PASTE_CHARS chars]" "the box holds the placeholder"
expect_pane_lacks paste-folded "PASTE-TAIL-SENTINEL" "the pasted text stays out of the box"

tmux send-keys -t "$RUST_PANE" Enter
step_when paste-sent "PASTE-TAIL-SENTINEL"
expect_pane_has paste-sent "PASTE-TAIL-SENTINEL" "the whole paste reached the agent"
expect_pane_lacks paste-sent "[Pasted Content" "the placeholder never reached the agent"

# ── A pasted image path becomes an attachment ──
#
# The path never appears as text anywhere (the marker stands in for it), and
# the two images are then collapsed to one: deleting the first marker has to
# renumber the survivor, which is a screen-visible rule this is the only place
# to see.
paste_text "$WORK/shot.png" "[Image #1]"
step paste-image-attached
expect_pane_has paste-image-attached "📎 1 image attached" "the box counts the attachment"
expect_pane_lacks paste-image-attached "shot.png" "the path is not inserted as text"

paste_text "$WORK/other.png" "[Image #2]"
step paste-image-two
expect_pane_has paste-image-two "[Image #1][Image #2]" "both markers are in the box"
expect_pane_has paste-image-two "📎 2 images attached" "the box counts both"

# Left x10 puts the caret just after the first marker (the paste is repeated
# verbatim, so both markers are exactly `[Image #1]` long); ten backspaces then
# remove it and nothing else.
tmux send-keys -t "$RUST_PANE" Left Left Left Left Left Left Left Left Left Left
tmux send-keys -t "$RUST_PANE" BSpace BSpace BSpace BSpace BSpace BSpace BSpace BSpace BSpace BSpace
step paste-image-renumbered
expect_pane_has paste-image-renumbered "[Image #1]" "the survivor keeps a marker"
expect_pane_lacks paste-image-renumbered "[Image #2]" "the survivor was renumbered"
expect_pane_has paste-image-renumbered "📎 1 image attached" "one image is left"

tmux send-keys -t "$RUST_PANE" Enter
step_when paste-image-sent "[Image #1]"
expect_pane_has paste-image-sent "[Image #1]" "the marker is what the message carries"
expect_pane_lacks paste-image-sent "shot.png" "neither path is in the message"
expect_pane_lacks paste-image-sent "other.png" "neither path is in the message"

# ── Ctrl+C exit ─────────────────────────────────────────────────────────────
tmux send-keys -t "$RUST_PANE" C-c
sleep 4
RUST_EXIT="$(grep -o '[0-9][0-9]*' "$WORK/rust.exit" 2>/dev/null | head -1 || echo MISSING)"
if [ "$RUST_EXIT" = "0" ]; then
  echo "OK[ctrl-c]: Rust TUI exited 0"
  PASS=$((PASS+1))
else
  echo "FAIL[ctrl-c]: Rust TUI exit code = $RUST_EXIT (want 0)"
  FAIL=$((FAIL+1))
fi

tmux kill-session -t "$SESSION" 2>/dev/null || true

echo
echo "== result: $PASS passed, $FAIL failed, $AFAIL assertion failure(s) =="
if [ "$FAIL" -gt 0 ] || [ "$AFAIL" -gt 0 ]; then
  exit 1
fi
exit 0
