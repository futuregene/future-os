#!/usr/bin/env bash
# snapshot.sh <commit-message-file> [--apply]
#
# The ONE way the supervisor commits in-flight work, because a snapshot can commit an
# EXPERIMENT as if it were deliverable code.
#
# WHY THIS EXISTS (F.67, measured): F-CLI-5 instrumented cli/src/browser/{backend.rs,
# chromium_navigation.rs} with `TEMP DIAG (F-CLI-5)` / FUTURE_CLOCK_DIAG eprintln sites,
# its run ended before it reverted them, and my snapshot (`git add -u; git commit`)
# committed the experiment into HEAD. From then on `git status` read CLEAN and the only
# trace was in history - i.e. the memory that a snapshot cannot tell an experiment from
# code (F.59) was realised, and the marker check in endgame.sh did not catch it because
# its marker list did not contain that marker (also fixed there).
#
# So: check the STAGED diff for experiment markers BEFORE committing, and list them with
# the file and the line. This is a warning, not a refusal, because a deliberate
# paused-clock test is legitimate - but it prints where it is introduced, which is the
# only place that has the context to judge it.
set -uo pipefail

MSG=$1
APPLY=0
[ "${2:-}" = "--apply" ] && APPLY=1

MARKERS='start_paused|TEMPORARY|FIX_STUB_DELAY|TUI_LAUNCH_DIAG|PROBE diag|hookdiag|FUTURE_CLOCK_DIAG|TEMP DIAG|CLOCK diag'

git add -f .future/cov100/ 2>/dev/null
git add -u 2>/dev/null

# F.84: `git add -u` stages EVERYTHING modified - including files a unit is still writing. Twice today
# that swept a live unit's record into my commit (app-data's module-tauri-appdata.md, and F-TAURI-4's
# flaky-tauri.md before that), which is the F.80 hazard at the staging step rather than at the commit
# step. Worse, `git add -f .future/` also force-adds the per-run journals: one commit came out at
# 3.2 MILLION lines. So: unstage every file whose owner still holds a live lease, and never force-add
# the journal trees.
SKIP=$(python3 .future/cov100/inflight-staged.py --skip-paths 2>/dev/null)
if [ -n "$SKIP" ]; then
  echo "=== unstaging files owned by a RUNNING unit (F.84) ==="
  echo "$SKIP" | sed 's/^/      /'
  echo "$SKIP" | tr '\n' '\0' | xargs -0 -r git reset -q -- 2>/dev/null
fi
git reset -q -- '.future/goal/goals/*/runs/' '.future/goal/runs/' 2>/dev/null

echo "=== staged experiment-marker check (F.67) ==="
# CODE extensions only: a marker can only corrupt a measurement if it is in something that gets built,
# and restricting to code keeps the check from firing on the prose in the records that DESCRIBE an
# incident (observed twice - the .future/ incident notes and the flaky-cli.md section, both of which
# name the markers). A warning that always fires is a warning I would stop reading.
HITS=$(git diff --cached -- '*.rs' '*.ts' '*.tsx' '*.js' '*.mjs' '*.sh' ':(exclude).future/**' | grep -nE "^\+.*($MARKERS)" | head -20)
if [ -n "$HITS" ]; then
  echo "MARKERS in the STAGED diff - each must be INTENDED:"
  echo "$HITS" | sed 's/^/      /'
  echo "      A deliberate paused-clock TEST is fine. An added eprintln! diag, a flipped"
  echo "      flag, or an #[ignore] left behind by an experiment is NOT - revert it first."
else
  echo "none"
fi

# F.80: a marker check cannot see a WITHDRAWAL - there is no marker in a budget a unit implemented,
# measured and rejected. What is knowable at commit time is that the file's owner is STILL RUNNING,
# which is what predicts the risk. Advisory, never a refusal.
echo "=== are these files still owned by a running unit? (F.80) ==="
python3 .future/cov100/inflight-staged.py 2>/dev/null || echo "      (inflight-staged.py unavailable)"

if [ "$APPLY" != 1 ]; then
  echo "DRY RUN: staged only. Re-run with --apply to commit."
  git diff --cached --stat | tail -3
  exit 0
fi

git commit -q -F "$MSG" && echo "committed: $(git log --oneline -1)"
