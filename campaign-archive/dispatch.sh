#!/usr/bin/env bash
# dispatch.sh <agent-id> <max-turns> [note]
#
# The ONLY safe way for the supervisor to start a run, because `future goal run` does not check whether
# the agent is already running.
#
# WHY THIS EXISTS (measured): TAU-B reported "another session edited files inside this unit's declared
# write set during the segment", and the ledger showed why - TWO concurrent sessions of
# `worker-tauri-appdata`, one claiming at 21:34:11 and another at 21:37:46, from my own re-dispatch. The
# consequence was real: two coverage scripts ran in parallel, each writing its own lcov
# (/tmp/appdata-tauri-lcov.info at 22:02 and /tmp/appdata-seg4-tauri-lcov.info at 22:03), and the unit
# had to reconcile them before trusting its number.
#
# The workspace guard cannot catch this: `WorkspaceConflict` is defined as "ANOTHER registered agent holds
# a live lease while its declared workspace set overlaps the claimer's" (workspace_guard.rs:128). Two
# sessions of ONE agent are invisible to it, because the holder and the claimer are the same id. So the
# check has to happen here, before the dispatch.
set -uo pipefail
G=goal_160bc247aa63
WT=/Users/geilige/future-os/.worktrees/cov100-r2
AGENT="${1:?usage: dispatch.sh <agent-id> <max-turns> [note]}"
TURNS="${2:?usage: dispatch.sh <agent-id> <max-turns> [note]}"
NOTE="${3:-}"

cd "$WT" || exit 1

state=$(future goal worker list --goal $G 2>&1 | awk -v a="$AGENT" '$1==a {print $2}')
todo=$(future goal worker list --goal $G 2>&1 | awk -v a="$AGENT" '$1==a {print $4}')

if [ "$state" = "running" ]; then
  echo "REFUSING: $AGENT is ALREADY RUNNING (todo $todo)."
  echo "  A second dispatch would create two concurrent sessions of the same agent on the same files -"
  echo "  the guard cannot see it (it compares against OTHER agents only), and the unit then has to"
  echo "  reconcile two parallel measurements (measured: TAU-B, 21:34 vs 21:37, two lcovs a minute apart)."
  echo "  Wait for it, or stop it first: future goal worker stop --goal $G --agent-id $AGENT"
  exit 2
fi

echo "dispatching $AGENT (state was '${state:-ended}') for $TURNS turns${NOTE:+: $NOTE}"
# background subshell + sleep: --detach is not instantaneous, and a short-lived or timeout-bounded parent
# can kill the child before it detaches (INCIDENTS F.8).
( future goal run --goal $G --agent-id "$AGENT" --max-turns "$TURNS" --detach \
    > "/tmp/d-$AGENT.log" 2>&1 & )
sleep 70

state2=$(future goal worker list --goal $G 2>&1 | awk -v a="$AGENT" '$1==a {print $2}')
todo2=$(future goal worker list --goal $G 2>&1 | awk -v a="$AGENT" '$1==a {print $4}')
echo "now: $AGENT state=$state2 todo=$todo2"
tail -2 "/tmp/d-$AGENT.log" 2>/dev/null | sed 's/^/  /'
[ "$state2" = "running" ] || { echo "WARNING: it did not start - read /tmp/d-$AGENT.log"; exit 1; }
