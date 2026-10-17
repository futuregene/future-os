#!/usr/bin/env bash
# endgame.sh - the frozen-revision pass, with the trigger conditions I set when resolving gate
# todo_605d26305bea made EXECUTABLE rather than remembered.
#
# Trigger (both must hold, else this script refuses and says which):
#   1. no writing unit is running (no `worker list` row in state `running`)
#   2. `staletable.py --verify-stamp` exits 0 for the revision to be published
# Then, in order:
#   0. MERGE origin/main into this branch (added 2026-10-17, see below), resolve, re-run the
#      affected modules' scoped checks
#   a. stamp the revision                       (--stamp)
#   b. run measure-final.sh                     (the four tracks, ~1.5-3 h)
#   c. verify the stamp again                   (a writer starting mid-run voids the pass)
#   d. regenerate the pairing-gate table into the final report
#
# WHY STEP 0 EXISTS (measured 2026-10-17): the campaign branch is based on 8449acf6, while
# origin/main had advanced to 89c9f940 - 68 files, including `refactor(build): unify Rust
# workspace and CLI/Desktop builds` (#1153) - and SIX of those files are files this campaign
# also changed: desktop/src-tauri/src/{lib.rs,agent_bridge/import.rs,agent_bridge/run_control.rs,
# store/runs.rs}, docs/testing/module-tauri.md and mobile/src/remote/__tests__/syncEngine.test.ts.
# A measurement taken on the unmerged branch would therefore describe a revision that NO PR
# CONTAINS, and any line number in the affected modules could be wrong for the revision that
# actually lands. I probed the merge in a scratch worktree (--detach, removed afterwards):
# it applies CLEANLY (no conflicts) and the campaign's three key changes survive it
# (the typed `session_row` fixture in import.rs, the `gui_builder<R>` seam in lib.rs, and the
# mobile syncEngine timer fix). So the merge is cheap - but the numbers must be taken AFTER it.
# The affected modules (src-tauri, mobile) must therefore be RE-MEASURED in step b, not quoted
# from their segment records.
#
# Usage:
#   bash endgame.sh --check          # report whether the trigger holds (safe, no side effects)
#   bash endgame.sh --run            # perform the pass (refuses unless the trigger holds)
#   bash endgame.sh --run --force    # override the quiescence check, and SAY SO in the report
set -uo pipefail
cd /Users/geilige/future-os/.worktrees/cov100-r2 || exit 1
G=goal_160bc247aa63
T=.future/cov100
MODE="${1:---check}"
FORCE=0
for a in "$@"; do [ "$a" = "--force" ] && FORCE=1; done

running=$(future goal worker list --goal $G 2>&1 | awk 'NR>1 && $2=="running"{print $1, $4}')
n_running=$(printf '%s\n' "$running" | grep -c . || true)

echo "=== trigger check ==="
if [ "$n_running" -eq 0 ]; then
  echo "quiescence: OK - no writing unit is running"
else
  echo "quiescence: NOT MET - $n_running unit(s) running:"
  printf '%s\n' "$running" | sed 's/^/    /'
fi

# LOAD PRECONDITION (added 2026-10-17). The campaign has MEASURED that its numbers are load-sensitive:
# units hit load-sensitive budgets, one coverage number moved by ±1 with no source change, and a whole
# class of flakes only appears above load ~15-30. The frozen pass produces the AUTHORITATIVE numbers for
# the 100 % claim, so it must not run on a host that something else is already saturating.
#
# Observed live: load average 82 on an 8-14 core host, caused by 48 detached CPU burners
# (`timeout 2400 sh -c 'while :; do :; done'`, spawned in one batch by load-sensitivity flake work) that
# OUTLIVED the segment that created them, plus a peer session's cargo build in another worktree. That is
# not "quiet", and a pass started then would have measured the load, not the code.
# The check is deliberately taken BEFORE the pass spawns its own compile/test work, and it reports the
# numbers even when it passes, because a bare "OK" is not evidence (F.9).
ncpu=$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 0)
loads=$(sysctl -n vm.loadavg 2>/dev/null | tr -d '{}' || uptime | sed 's/.*load averages*: *//')
load1=$(printf '%s\n' "$loads" | awk '{print $1}')
load5=$(printf '%s\n' "$loads" | awk '{print $2}')
burners=$(ps -eo args 2>/dev/null | grep -c "timeout [0-9]* sh -c while" || true)
# The threshold is NOT the machine's capacity: the campaign measured that its OWN budgets and flakes go
# load-sensitive from roughly load 15-30 (a coverage number moved by 1 with no source change; one flake
# class appears only above load ~15). On this 32-CPU host that is about half the cores, so `load > ncpu`
# would pass a pass that should not run. Half the cores, or more than a handful of pure CPU burners.
thresh=$(( ncpu / 2 ))
[ "$thresh" -lt 2 ] && thresh=2
load_ok=0
if [ "${ncpu:-0}" -gt 0 ] && [ -n "$load1" ]; then
  load_hi=$(awk -v l="$load1" -v n="$thresh" 'BEGIN{print (l > n) ? 1 : 0}')
  burn_hi=$(awk -v b="${burners:-0}" 'BEGIN{print (b > 8) ? 1 : 0}')
  if [ "$load_hi" = 0 ] && [ "$burn_hi" = 0 ]; then
    load_ok=1
    echo "load:       OK - 1-min $load1 / 5-min $load5 on $ncpu CPUs (threshold $thresh; foreign burners $burners)"
  else
    echo "load:       TOO HIGH - 1-min $load1 / 5-min $load5 on $ncpu CPUs (threshold $thresh; foreign burners $burners)"
    echo "            The numbers are load-sensitive (measured: a count moved by 1 with no source change,"
    echo "            and a flake class that appears only above load ~15-30). A pass now would measure the"
    echo "            load. Wait for it to fall, or re-run with --force and SAY SO in the report."
  fi
else
  echo "load:       UNKNOWN - could not read load/$ncpu CPUs; treat as NOT MET"
fi

# BUILD PRECONDITION (added 2026-10-17, INCIDENTS F.75). Every other precondition below assumes the
# host CAN BUILD: the frozen pass compiles four tracks. On this host state a wedged `syspolicyd`
# (measured: pid 217 at 978-1049 % CPU for 8 d 20 h) makes macOS policy-check every newly built binary,
# so builds stall machine-wide - a forced `cargo check -p future-rpc` hit a 120 s timeout with cargo's own
# CPU at 0.06 s user, i.e. blocked rather than computing. Without this check the pass would burn its
# per-track budget (90 min) and emit a report with no numbers.
# The check is the CHEAP signature, not the definitive test: a definitive test costs a forced rebuild.
sp_cpu=$(ps -eo pcpu,comm 2>/dev/null | awk '$2 ~ /syspolicyd$/ {print $1; exit}')
build_ok=0
if [ -z "$sp_cpu" ]; then
  echo "build:      UNKNOWN - syspolicyd not found in ps; treat as NOT MET"
elif awk -v c="$sp_cpu" 'BEGIN{exit !(c > 300)}'; then
  echo "build:      TOO HIGH - syspolicyd is at ${sp_cpu}% CPU (the wedged-policy signature: builds stall"
  echo "            machine-wide while macOS assesses each new binary). The frozen pass BUILDS four tracks,"
  echo "            so this blocks it entirely. Check with: touch packages/rpc/src/lib.rs && cargo check -p future-rpc"
  echo "            (should finish in seconds; if it stalls, this is confirmed). Needs a reboot or a privileged"
  echo "            daemon restart, neither of which this pass can do."
else
  build_ok=1
  echo "build:      OK - syspolicyd at ${sp_cpu}% CPU (below the 300% wedge threshold)"
fi

stamp_ok=0
if python3 $T/staletable.py --verify-stamp $T/gate-snapshot.json >/tmp/stamp-check.txt 2>&1; then
  echo "stamp:      OK - $(tail -1 /tmp/stamp-check.txt)"
  stamp_ok=1
else
  echo "stamp:      NOT MET - $(tail -2 /tmp/stamp-check.txt | head -1)"
  echo "            (this is expected when docs/reports moved since the snapshot; re-stamp first)"
fi

# PAIRING PREFLIGHT: repair must happen BEFORE the stamp, otherwise the stamp captures a red gate and
# the final report carries stale pairings as if they were verdicts. Drift here means a unit re-measured
# after its registry entry was written (INCIDENTS F.29) - it is NOT a disagreement with a ledger.
pair_ok=0
if python3 $T/staletable.py --gate --strict >/tmp/pair-pre.txt 2>&1; then
  echo "pairings:   OK - every pairing justifies its covered rows"
  pair_ok=1
else
  if grep -qi "DRIFT" /tmp/pair-pre.txt; then
    echo "pairings:   STALE - a unit re-measured after its registry entry was written:"
    grep -E "DRIFT" /tmp/pair-pre.txt | head -6 | sed 's/^/              /'
    echo "            Repair with: python3 $T/staletable.py --suggest --labels <label>"
    echo "            ALWAYS pass --labels: the full run MEASURES every /tmp candidate for"
    echo "            all ~22 pairings and takes ~160 s (95 candidates); with --labels it"
    echo "            measures only that label - 0.1 s for an already-clean one, ~8 s for a"
    echo "            drifting one (both measured 2026-10-17, F.74: the filter used to run"
    echo "            after the measurement, so narrowing did not help at all)"
    echo "            (the pairing registry is the tooling unit's artefact; --suggest refuses vacuous"
    echo "             candidates, which is why it is used rather than hand-editing)"
  else
    echo "pairings:   RED for a non-drift reason - read /tmp/pair-pre.txt"
    grep -E "FAILING|PROBLEM" /tmp/pair-pre.txt | head -4 | sed 's/^/              /'
  fi
fi

# The pre-classified repair list (CLOSEOUT 2b). freeze-matrix.py reads ONLY the report each
# pairing names, so it is cheap and it tells the two failure classes apart: a registry/expect
# lag (repair the registry) versus a record that genuinely moved (repair the module). Added
# 2026-10-17 after building it showed 6 of 8 apparent "drifts" were my own reader's bugs -
# without it, the freeze step invites hand-editing a wrong number into the gate.
if [ "$MODE" = "--check" ] && [ "$pair_ok" = 0 ]; then
  if python3 $T/freeze-matrix.py >/tmp/pair-matrix.txt 2>&1; then
    head -1 /tmp/pair-matrix.txt | sed 's/^/            /'
    grep -E "^!!" /tmp/pair-matrix.txt | sed 's/^/            /'
  else
    echo "            (freeze-matrix.py unavailable - see CLOSEOUT 2b for the recorded list)"
  fi
fi

if [ "$MODE" = "--check" ]; then
  echo
  echo "verdict: $([ "$n_running" -eq 0 ] && echo 'QUIET' || echo 'BUSY') (pairings $([ "$pair_ok" = 1 ] && echo ok || echo stale), stamp $([ "$stamp_ok" = 1 ] && echo ok || echo stale), load $([ "$load_ok" = 1 ] && echo ok || echo HIGH), build $([ "$build_ok" = 1 ] && echo ok || echo BLOCKED))"
  echo "next:    bash endgame.sh --run   $([ "$n_running" -eq 0 ] || echo '   # will refuse until the writers stop')$([ "$pair_ok" = 1 ] || echo '   # repair pairings and re-stamp first')$([ "$load_ok" = 1 ] || echo '   # will refuse while the host is saturated')$([ "$build_ok" = 1 ] || echo '   # will refuse while builds stall (F.75)')"
  exit 0
fi

if [ "$build_ok" != 1 ] && [ "$FORCE" != 1 ]; then
  echo
  echo "REFUSING: the host cannot build (syspolicyd at ${sp_cpu}% CPU). The frozen pass compiles four"
  echo "tracks, so starting it now burns the per-track budgets and emits a report with no numbers. This is"
  echo "a MACHINE fault (a wedged /usr/libexec/syspolicyd, measured at ~1000% CPU for days), not a repo one:"
  echo "the usual remedy is a reboot, and killing a system daemon needs root. Re-run with --force only to"
  echo "test the harness itself, and state in the report that no measurement was possible."
  exit 2
fi

if [ "$n_running" -ne 0 ] && [ "$FORCE" != 1 ]; then
  echo
  echo "REFUSING: $n_running writing unit(s) active. A pass now would describe a revision that keeps"
  echo "moving, which is exactly what four earlier attempts measured and could not do. Re-run with"
  echo "--force only if you accept that the report must then state the pass was NOT on a frozen tree."
  exit 2
fi

if [ "$load_ok" != 1 ] && [ "$FORCE" != 1 ]; then
  echo
  echo "REFUSING: the host is saturated (1-min load $load1 / 5-min $load5 on $ncpu CPUs, threshold $(( ncpu / 2 )), foreign burners: $burners)."
  echo "The authoritative numbers for a 100 % claim must not be measured through someone else's load: this"
  echo "campaign has already had a coverage number move by 1 with no source change, and its flake class"
  echo "only appears above load ~15-30. Wait, or use --force and state it in the report."
  exit 2
fi

REV=$(git rev-parse --short HEAD)
DIRTY=$(git status --porcelain | wc -l | tr -d ' ')
echo
echo "=== stamped pass at $REV (dirty paths: $DIRTY) $([ "$FORCE" = 1 ] && echo '[FORCED: tree not quiet]') ==="

echo "--- 0) sync with origin/main BEFORE measuring (the numbers must describe what will land) ---"
git fetch origin main 2>&1 | tail -1
BEHIND=$(git rev-list --count HEAD..origin/main)
if [ "$BEHIND" = 0 ]; then
  echo "up to date with origin/main - nothing to merge"
else
  echo "origin/main is $BEHIND commit(s) ahead; merging (measured clean on 2026-10-17)"
  if git merge origin/main --no-edit >/tmp/merge-out.txt 2>&1; then
    echo "merged cleanly at $(git rev-parse --short HEAD)"
    echo "  affected modules to RE-MEASURE (they overlap upstream): src-tauri, mobile"
  else
    echo "MERGE CONFLICTS - resolve them here, re-run the scoped checks, then re-run this script:"
    git diff --name-only --diff-filter=U | sed 's/^/      /'
    exit 3
  fi
fi

echo "--- 0b) experiment-marker check (INCIDENTS F.59: a snapshot cannot tell an experiment from code) ---"
# A unit that runs a deliberate experiment (a paused clock, an added #[ignore], a
# TEMPORARY diagnostic) must flag it, and I must check before stamping - a stale
# experiment in a "safety" snapshot would make this pass measure the wrong code.
#
# F.67 (2026-10-17): this check EXISTED and still missed a live case, for two reasons,
# both fixed here. (1) The marker list was a hand-written enumeration that did not
# contain F-CLI-5's `FUTURE_CLOCK_DIAG` / `TEMP DIAG` instrumentation - and that
# instrumentation WAS committed into HEAD by a supervisor snapshot, so
# `git status` read clean while the code carried an experiment. (2) It matched only
# '*.rs' files, so a marker in a script or a doc was invisible. The list below is
# still an enumeration (a marker can always be invented), which is why the
# snapshot-side guard (snapshot.sh) checks the SAME list at commit time: catching it
# where it is introduced is the only place with enough context to judge it.
# `-- '*.rs' '*.ts' '*.tsx' '*.js' '*.mjs' '*.sh' ':(exclude).future/**'` - TWO exclusions, each
# learned the hard way. CODE extensions only, because a marker can only corrupt a measurement if it is
# in something that gets built; and .future/ because the tooling and records contain the marker NAMES
# themselves (this list, the incident notes, and the doc sections describing them), so without it the
# check fired on its own prose - twice observed: 2 self-matches in .future/, and 2 on the new
# flaky-cli.md section describing the incident. An alarm that always goes off is one I would ignore.
# The real case still trips it: the commit that introduced F-CLI-5's instrumentation yields 17 hits.
MARKERS='start_paused|TEMPORARY|FIX_STUB_DELAY|TUI_LAUNCH_DIAG|PROBE diag|hookdiag|FUTURE_CLOCK_DIAG|TEMP DIAG|CLOCK diag'
HITS=$(git diff origin/main..HEAD -- '*.rs' '*.ts' '*.tsx' '*.js' '*.mjs' '*.sh' ':(exclude).future/**' | grep -E "^\+.*($MARKERS)" | head -12)
if [ -n "$HITS" ]; then
  echo "MARKERS PRESENT in the diff - confirm each is INTENDED before stamping:"
  echo "$HITS" | sed 's/^/      /'
  echo "      (a deliberate paused-clock test is fine; a temporary flip is not)"
else
  echo "no experiment markers in the diff"
fi

echo "--- a) stamp ---"
python3 $T/staletable.py --stamp $T/gate-snapshot.json 2>&1 | tail -3

echo "--- b) the four tracks (measure-final.sh) ---"
bash $T/measure-final.sh 2>&1 | tail -25

echo "--- c) verify the stamp again (a writer starting mid-run voids the pass) ---"
if python3 $T/staletable.py --verify-stamp $T/gate-snapshot.json >/tmp/stamp-after.txt 2>&1; then
  echo "VERIFIED: every doc and report the snapshot names is unchanged since the stamp"
  echo "unchanged: true" >> coverage/final/FINAL-REPORT.md 2>/dev/null || true
else
  echo "VOID: something moved during the pass. This report describes a MIXED revision."
  tail -6 /tmp/stamp-after.txt
  echo "**VOID — the tree moved during this pass; re-run on a quiet revision.**" \
    >> coverage/final/FINAL-REPORT.md 2>/dev/null || true
fi

echo "--- d) pairing gate into the report (drift is reported as its own state) ---"
if python3 $T/staletable.py --gate --strict >/tmp/gate-strict.txt 2>&1; then
  echo "gate: GREEN (every pairing justifies its covered rows)"
else
  if grep -qi "DRIFT" /tmp/gate-strict.txt; then
    echo "gate: STALE PAIRINGS - drift, which means a unit re-measured after the registry entry was"
    echo "      written. This is NOT a disagreement with a ledger; it is the registry lagging a"
    echo "      re-measurement. Repair pairings.json (--suggest) and re-stamp before publishing."
  else
    echo "gate: RED for a non-drift reason - read /tmp/gate-strict.txt"
  fi
  grep -E "FAILING|DRIFT" /tmp/gate-strict.txt | head -6 | sed "s/^/      /"
fi
python3 $T/staletable.py --md > packaging-gate.md 2>/tmp/gate-md.txt
echo "gate step exit: $?"
cat packaging-gate.md >> coverage/final/FINAL-REPORT.md 2>/dev/null || true

echo "--- e) assemble the self-contained sections (ONE implementation: assemble-report.sh) ---"
if [ -s "$REPORT" ]; then
  bash $T/assemble-report.sh "$REPORT" "$n_running" \
    || echo "WARNING: assembly failed - the report is missing its anchors/gates/governance sections"
else
  echo "WARNING: no report to assemble into ($REPORT)"
fi

echo "=== done: coverage/final/FINAL-REPORT.md ==="
ls -la coverage/final/ 2>/dev/null | tail -4
