#!/usr/bin/env bash
# assemble-report.sh <report-path> [n_running]
#
# Appends the sections that make FINAL-REPORT.md self-contained: content anchors, the gate sweep on this
# revision, ACCEPTANCE.md + CLOSEOUT.md verbatim, and an obligations checklist.
#
# Separate from endgame.sh on purpose: the assembly is testable on a throwaway file, and if it fails at
# the freeze it can be re-run WITHOUT repeating a 1.5-3 h measurement.
set -uo pipefail
ROOT=/Users/geilige/future-os/.worktrees/cov100-r2
T="$ROOT/.future/cov100"

# ENV HYGIENE, and this is not optional: run standalone, this script inherits the caller's PATH, where
# Homebrew's rust shadows rustup - and clippy then reports errors for code the pinned toolchain accepts.
# Measured: without this, the sweep below printed clippy=101 for four crates that are clean under
# 1.97.0 (the version rust-toolchain.toml pins). Same failure family as the repo memory's
# "Homebrew 遮蔽 rustup: 修法是 PATH 前缀".
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo
export RUSTUP_HOME=/Users/geilige/.rustup
export HOME="$ROOT/target/test-home"
mkdir -p "$HOME"
REPORT="${1:?usage: assemble-report.sh <report-path> [n_running]}"
n_running="${2:-?}"

say() { printf '%s\n' "$*" >> "$REPORT"; }

echo "assembling into $REPORT"

# 1. content anchors -----------------------------------------------------------------------------
say ""
say "## Content anchors (a number describes one artifact)"
say ""
say "| report | bytes | sha256 |"
say "|---|---|---|"
for f in /tmp/ws-final-lcov.info /tmp/tauri-final-lcov.info "$ROOT/coverage/final/web-coverage-final.json" \
         /tmp/mobile-final-lcov.info; do
  [ -s "$f" ] || continue
  say "| ${f#$ROOT/} | $(wc -c < "$f" | tr -d ' ') | $(shasum -a 256 "$f" | cut -d' ' -f1) |"
done
say ""
say "_Reports not present above were not produced by this run; a number without an anchor is not verified._"

# 2. gate sweep ---------------------------------------------------------------------------------
say ""
say "## Gate sweep on this revision"
say ""
say "| target | fmt | clippy --all-targets -D warnings |"
say "|---|---|---|"
for p in future-cli future-agent future-channel future-goal future-rpc future-tui; do
  cargo fmt -p "$p" --check >/tmp/asm-f.txt 2>&1; frc=$?
  cargo clippy -p "$p" --all-targets -- -D warnings >/tmp/asm-c.txt 2>&1; crc=$?
  say "| $p | $frc | $crc |"
done
if [ -d "$ROOT/desktop/src-tauri" ]; then
  ( cd "$ROOT/desktop/src-tauri" && cargo fmt --check >/tmp/asm-f2.txt 2>&1 ); frc=$?
  ( cd "$ROOT/desktop/src-tauri" && cargo clippy --all-targets -- -D warnings >/tmp/asm-c2.txt 2>&1 ); crc=$?
  say "| desktop/src-tauri | $frc | $crc |"
fi
# JS gates: use the REPO'S OWN scripts. `eslint .` is not the repo's lint - package.json scopes it to
# `src/**/*.{ts,tsx} shot/**/*.{ts,tsx} vite.shot.config.ts`, and linting the rest reports ~106 errors in
# scripts/ and config files the repo deliberately excludes. A gate sweep that invents its own invocation
# publishes false reds (this one did, until checked).
for d in desktop mobile; do
  if [ -f "$ROOT/$d/package.json" ]; then
    if grep -q '"typecheck"' "$ROOT/$d/package.json"; then
      ( cd "$ROOT/$d" && npm run typecheck >/tmp/asm-t.txt 2>&1 ); trc=$?
    else
      ( cd "$ROOT/$d" && npx tsc --noEmit >/tmp/asm-t.txt 2>&1 ); trc=$?
    fi
    if grep -q '"lint"' "$ROOT/$d/package.json"; then
      ( cd "$ROOT/$d" && npm run lint >/tmp/asm-e.txt 2>&1 ); erc=$?
    else
      erc="n/a"
    fi
    say "| $d (JS) | typecheck: $trc | npm run lint: $erc |"
  fi
done

# 3. verbatim governance documents ---------------------------------------------------------------
say ""
say "---"
say ""
for f in "$T/ACCEPTANCE.md" "$T/CLOSEOUT.md"; do
  [ -s "$f" ] || continue
  say "# (appended from $(basename "$f"))"
  say ""
  cat "$f" >> "$REPORT"
  say ""
done

# 4. obligations checklist -----------------------------------------------------------------------
say ""
say "## Obligations checklist (what THIS run does and does not establish)"
say ""
say "| # | obligation | status |"
say "|---|---|---|"
say "| 1 | metric: per-line LCOV DA:0 (JS: all-statements-zero) | yes - stated in the header; INVALID rows are rejections, not numbers |"
say "| 2 | run identity: revision, command, tool exit code | revision in the header; exit codes in the harness log |"
say "| 3 | content anchors (sha256 of every measured report) | yes - section above |"
say "| 4 | waived-vs-open arithmetic per unit | in each module record (see ACCEPTANCE.md and docs/testing/*.md); not re-derived here |"
say "| 5 | tripwire 0 MISSING / 0 STALE | yes - the pairing gate section |"
say "| 6 | independent review verdict per waiver set | CLOSEOUT section 3 (REV-FE / CH / TUI / DESK; REV-TAURI-SHELL when landed) |"
say "| 7 | forbidden inverse (no guard deleted, no assertion weakened, nothing ignored/skipped) | checked per unit at acceptance; NOT re-derived by this run |"
say "| 8 | gates (fmt/clippy/tsc/eslint) | yes - gate-sweep section above |"
if [ "$n_running" = "0" ]; then
  say "| 9 | tree quiescence (no writer active during the run) | **yes** - no writer was running at start |"
else
  say "| 9 | tree quiescence (no writer active during the run) | **NO - $n_running unit(s) were running; this pass is NOT a frozen measurement** |"
fi
say "| 10 | stamp verified after the run | see the VOID line (if present) in the stamp section |"
