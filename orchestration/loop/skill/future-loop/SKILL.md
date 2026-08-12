---
version: 2.0.0
name: future-loop
description: FutureOS loop control plane — manage long-running goals, todo lists, human gates, monitors, and validated completion via the loop control plane. Use when the user wants a long-lived/multi-step/cross-session task tracked as a goal, asks to "keep working on X", "track this issue", "run this overnight", needs progress/status of ongoing agent work, or starts a message with "/future-loop" (treat everything after the prefix as the goal).
allowed-tools: Bash(future-loop:*)
category: tools
---

# Loop Control Plane

`future loop` turns a conversation into a durable, reviewable long-running goal:
the goal, todos, human gates, monitors, evidence, and completion are
persisted outside the chat. The agent executes one bounded turn at a time;
the loop control plane decides what should happen next.

## When to use

Load this skill when the user:
- starts a message with **`/future-loop <task>`**: treat the text after the
  prefix as a NEW long-running goal — create the goal and drive it to
  completion (do NOT treat it as a one-shot request);
- wants a **long-lived / multi-step / cross-session** task ("keep working on X",
  "track this issue", "run this overnight", "keep pushing forward");
- asks about the **status or progress** of ongoing agent work;
- needs a decision point surfaced ("waiting for my approval" scenarios);
- wants recurring observation of an external state (CI, PR, file appearing).

For one-shot conversations, just answer normally — no goal needed.

## Prerequisites

- Everything runs through the unified CLI: **`future loop <cmd>`**. Look for an
  existing install (`command -v future`; also `~/.local/bin/future`, `~/bin/future`,
  Homebrew links). If missing, point the user to
  <https://github.com/futuregene/future-os> (README install section) — do NOT
  rebuild from source on the user's behalf. State lives in the PROJECT:
  `<cwd>/.future/loop/` (run from the project dir, or pass --cwd).
- `future loop run` needs the agent server: `future agent` (gRPC
  127.0.0.1:50051). Probe with `future models`.
- **Check binary freshness before relying on newer features** — an old binary
  runs the commands but silently skips new behavior. Probe:
  `strings $(command -v future) | grep -c "max-turn-secs"` — `≥1` = current
  (blocks enforcement, mid-turn steering, turn budget, atomic claims,
  `status --format json`); `0` = stale → rebuild with `cargo build -p future-cli`.

## State layout (project-local, all under `<cwd>`)

```
<cwd>/.future/loop/registry.json                        — registry (source of truth)
<cwd>/.future/loop/goals/<id>/events.jsonl              — per-goal event ledger
<cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md      — reference-compatible projection
<cwd>/.future/loop/runs/                                — run history + <run_id>.live.jsonl
```

Runtime state is NEVER written outside the project. Add `.future/loop/` to the
project `.gitignore`.

## Workflow

### 1. Inspect existing goals first

```bash
future loop status
```
If the user's objective already exists, continue it — never silently create a duplicate.

### 2. Confirm the plan with the user BEFORE creating anything

Present a concrete plan and get confirmation:

1. **Steps** — the todos you will execute, in order. Short and concrete.
2. **Model + thinking level** — default to the CURRENT session's model and
   thinking level (read your own environment). If the user wants a different
   model, `future models [--json]` lists options (prefer `[recommended]` /
   `[default]`); `high` thinking for reasoning-heavy goals, `off`/`low` for
   mechanical work. If the agent server is down and `future models` fails, fall
   back to the current session's model + `high` and say so.
3. Ask: `Plan confirmation (goal: …) Steps: 1)… 2)… 3)… Model: … Thinking: … Confirm?`

Skip this confirmation only when the user's opening message already contains the
full objective AND explicit operating constraints (model, concurrency, deadline) —
that mandate IS the confirmation; start immediately.

### 3. Create a goal (or reuse)

```bash
future loop goal init --objective "..." --cwd <project-dir> [--goal-id <id>]
```
Use the user-named directory/repo root as `--cwd`; `mkdir -p` it if missing.
`goal init` auto-creates an onboarding todo — complete it with `--no-follow-up`
during setup; it is not real work.

### 4. Break the work into todos

```bash
future loop todo add --goal <goal-id> --text "..." --priority P0
```

**Dependency chains.** When "X must finish before Y can start", add `--blocks X`
to Y. A blocked todo stays out of the runnable frontier until every predecessor
reaches Done or Superseded (an unknown predecessor id does not wedge the
frontier; `task-graph` flags it as an error instead). Without `--blocks`, open
todos are independently runnable in priority order.

```bash
future loop todo add --goal G --text "Generate the report" --priority P0 --blocks <data-todo-1>,<data-todo-2>
future loop todo add --goal G --text "Copy deliverables to project root" --priority P0 --blocks <report-todo>
```

**Todo-creation pitfalls.**

1. **Capture todo ids from the `todo add` output** (`todo todo_xxx added ✔`) —
   the only reliable source. There is no `todo list`; look ids up via
   `future loop status --goal G [--format json]` or the event ledger
   (`kind:"todo_added"` events carry the full todo). Field-name gotcha: in the
   ledger the dependency field is `blocked_by_gate` (NOT `blocks`) and the
   validator is `validator` (NOT `verify`).
2. **Verify the wiring after creation** — `future loop status --goal G`; repair
   with `todo update --blocks a,b` (replaces; `--blocks ""` clears; absent leaves
   untouched).
3. **Chain the FINAL validation todo too** — priority alone does not order
   execution; the acceptance todo MUST `--blocks` all implementation todos or it
   can run while they are still stubs.
4. **`goal delete` requires `--force` and is irreversible** — recreate via
   `goal init` afterwards.
5. **Keep cleanup/deletion words out of test-goal objectives** — the run agent
   may execute `goal delete --force` itself if the objective mentions them.
   Delete test goals yourself.
6. **CLI strictness**: unknown flags and unknown todo-ids are rejected; most
   subcommands ignore `--help` (`todo update --help` prints usage). Exact flags:
   `orchestration/loop/src/console.rs` `parse_pairs` call sites.
7. **`todo archive` does NOT leave the frontier** — it only flips
   `archive_state`, which nothing in the decision/frontier code reads: an
   archived todo stays `open` in `status` and remains runnable by `run`. To
   take a todo out of rotation ("暂缓"/"先不做"), use `todo supersede`
   (Superseded unblocks dependents and exits the frontier).
8. **Successor duplicates.** Completing a todo with a successor can create a
   NEW todo that overlaps an existing open one (e.g. the agent rewrites a
   follow-up with better detail). You then hold two todos for the same work —
   supersede the stale one AND repair any `--blocks` that pointed at it:
   `todo update --todo-id <acceptance> --blocks <new-list>` (blocks are
   replaced wholesale).
9. **A turn is not one todo.** A single `run` turn may complete several todos
   in sequence (observed: one turn delivered 24 loop batches + 8 agent batches
   + a cli residual). `status`'s `spent` counter increments only when the run
   process EXITS — a mid-turn `status` still shows the old count and the old
   `next` line.
10. **The run agent can merge PRs itself.** In-turn tooling includes a shell;
    agents have run `gh pr merge` autonomously. When tracking PRs, check
    `gh pr list --state merged`, not just open PRs — and review what landed on
    main at wrap-up time.

**User gates.** When the user asks for approval before a specific action, create
a real gate — describing it in the objective is NOT enough:

```bash
future loop todo add --goal G --role user --class user_gate \
  --gate-question "<exact question>" --text "<exact question>" --blocks <action-todo>
```
Any OPEN gate freezes all work: `run` returns `AskUser`, and `todo complete`
rejects non-gate completions until it is resolved. Gates are completed via
`gate resolve`, never `todo complete`.

**Conditional iteration.** Attach an independent validator; the kernel runs it in
the goal cwd after each turn and only completes the todo on exit 0:

```bash
future loop todo add --goal G --text "Optimize until tests pass" --priority P0 \
  --verify "cargo test" --max-validation-attempts 5
```
Non-zero → repair retry up to the attempts budget, then a replan gate. The agent
can still force-complete via `todo complete --no-follow-up` (agent autonomy).

### 5. End with a deliverable-copy todo

Every goal that produces files MUST end with a final P0 todo that copies
deliverables from the loop state directory to the project root (`cp`, not `mv`),
blocked behind the producing todos:

```bash
future loop todo add --goal G --text "Copy the final deliverables to the project root (cwd)" --priority P0 --blocks <producer-todos>
```

### 6.0 Agent identity — `--agent-id` MANDATORY on every run

`run` without `--agent-id` fails fast; `--anonymous` is the legacy
uncoordinated one-shot. The agent-id is the ONLY mutual-exclusion mechanism:

- `run --agent-id <name>` auto-registers on first use; `agent onboard` declares
  capabilities; `agent list` shows who is registered/running.
- **Unique name per parallel worker** (e.g. `<host>-<task>`). A run sees its OWN
  leased todos as runnable, so two runs sharing one id double-execute.
- With `--agent-id`, `run` claims a lease (default 4h; `--lease-secs N`) BEFORE
  executing and hides other agents' leased todos (`claimed_by_other`) — parallel
  runs grab DIFFERENT todos. Claims are atomic (check+append under one lock; a
  lost race re-decides and takes the next runnable todo).
- Conflict-avoidance checklist before launching: `agent list` → `ps aux | grep
  "future loop run"` → `status --goal G` → `quota should-run --goal G --agent-id
  <me>` → `lease status --goal G --todo-id T --agent-id <me>`.

### 6.1 Multi-agent parallel runs

N concurrent runs on the SAME goal share the ledger safely. On current binaries
the claim race is closed; for extra determinism (or old binaries), assign todos
BEFORE launching:

```bash
future loop agent register --goal G --agent-id worker-1     # … worker-2, worker-3
future loop todo claim --goal G --todo-id <T1> --agent-id worker-1 --lease-secs 14400
nohup future loop run --goal G --agent-id worker-1 --model <M> --thinking-level <L> \
  --max-turns 1 > logs/worker-1.log 2>&1 &
```

- Lease expiry frees the todo for others but does NOT abort a running turn.
  Release a parked claim with `future loop lease release` so the queue's most
  valuable todo is never walled off by a stale lease.
- Relaunch the same command (same agent-id) to continue after each turn — the
  worker keeps its claim and resumes.
- Worker count must fit the machine; agree it with the user (default 3).
- Pre-claim overrides pure-priority ordering: assign highest-value todos first.

### 6.2 Orchestration patterns for parallel workers

1. **Orchestrator-only dangerous actions.** Anything consuming a LIMITED
   external resource (submission quotas, paid calls, irrecoverable deletes)
   must not be callable by workers — a worker's *failed* attempt may still
   consume the resource. Convention: workers finalize artifacts and write a
   signal file (e.g. `logs/NEEDS_ACTION` with the payload); only the
   orchestrator executes the capped action. State it explicitly in todo text.
2. **Shared project memory as broadcast channel.** Instruct workers (in todo
   text) to re-read the project memory file at turn start; use it for
   cross-worker discoveries and infra locations. The turn envelope carries only
   todo text + last evidence.
3. **Steer by updating todo text, anytime.** `todo update --text` on a running
   todo is delivered mid-turn (see §6 mechanics).
4. **Watch artifacts, not just loop status.** Long compute runs via nohup +
   checkpoint files; the orchestrator tracks output mtimes. Killing a worker's
   run process and relaunching with the same `--agent-id` is always safe —
   context replays from the ledger.
5. **Wrap-up belongs to the orchestrator.** A final/validation todo left
   unchained can be picked early and close the goal prematurely — `--blocks` it
   behind everything, or do the wrap-up outside the loop.

### 6. Run the agent — one turn at a time

**Always `--max-turns 1`** and loop manually for visibility:

```bash
future loop run --goal G --agent-id <unique-name> --model <M> --thinking-level <L> --max-turns 1
```

Run mechanics:
- **`--max-turn-secs N`** — per-turn wall-clock budget; timeout stops the run
  gracefully (session cleaned up, context replays on relaunch).
- **Mid-turn steering** — `todo update --text` on the todo a worker is executing
  is injected into the running session (~10s poll; delivered at the agent's next
  step boundary).
- **Session id in the envelope** — the turn message starts with `session: <id>`
  so in-turn tooling can find the real session file deterministically.
- **Live run logs** — streamed events tee to `.future/loop/runs/<run_id>.live.jsonl`.
- **`status --format json`** — machine-readable goal + todos projection.

**Launch pattern — nohup + poll (foreground blocking dies with shell timeouts):**

```bash
nohup future loop run --goal G --agent-id <me> --max-turns 1 > logs/worker-<me>.log 2>&1 &
tail -f .future/loop/runs/<run_id>.live.jsonl
```

**Monitoring a long turn** (multi-hour turns are normal for big tasks):
poll the pid (`kill -0 <pid>`), tail the live jsonl for `tool_start`/
`tool_end` liveness, and watch the worktree's `git log` for commit progress —
`status` alone lags (see pitfall 9). If the turn must be stopped, `kill <pid>`
is safe: the session writes back and the next `run` replays from the ledger.

**PR merge troubleshooting** (when turns produce PRs you must merge):
- `BEHIND` — branch protection requires up-to-date: `git merge origin/main`
  on the PR branch + push, wait for CI, merge.
- A required check stuck at **skipping** — the CI paths-filter probably misses
  the changed directory (e.g. `orchestration/**` was absent from
  `rust_workspace` once, so loop-only PRs could never merge). Fix the filter
  in `.github/workflows/ci.yml` on the same branch; editing the workflow
  re-triggers every suite.
- The `CodeQL` rollup check shows **skipping** while `Analyze (*)` jobs are
  still pending — that is an intermediate state, not a failure; wait.
- GitHub API flakes (SSL/EOF): wait and retry; don't re-push to "fix" it.

**Session lifecycle:** each `run` creates a fresh scratch session and deletes it
on every exit path — nothing durable lives in `~/.future/agent/sessions/`. A
stray `⚠ session cleanup failed` line or accumulated sessions = stale binary
(see Prerequisites) or leftover from other tools; clean with `future session
list` / `future session delete <id>`.

`run` stops when: validated closure (terminal), a human gate opens, a blocker
waits (unresolved `--blocks`), or max-turns/max-turn-secs is reached.

**After each turn:**
1. Report what was done (todo, cost, new status).
2. **Reflect & improve** — what did the turn reveal? Is the objective still
   right? Is the decomposition still optimal (split/merge/reorder)? New risks
   needing a gate/monitor/`--verify`? Every ~3–5 turns, deep-replan the
   remainder as if planning fresh. Apply routine adjustments yourself:
   `todo add` / `todo supersede --reason` / `todo update` / `todo archive`.
3. Check the plan still holds: `future loop status --goal G`.
4. Stop and ask the user ONLY when absolutely necessary (risky/irreversible
   changes, user-only decisions, or you cannot determine the adjustment).
5. Non-zero exit = max-turns hit; rerun only if open todos remain.

### 7. Handle replan gates — resolve routine ones yourself

**PLAN_REVIEW gates** (kernel-injected when validation budget is exhausted,
outcome floor hit, acceptance gaps remain, or the oscillation guard fires —
delivery outcomes flip-flopping accept/reject A→V→A→V across the post-ACK
run history) are the agent's to resolve:
review (`status` / `diagnose --goal G`) → adjust todos via CLI → resolve and
resume:

```bash
future loop gate resolve --goal G --todo-id <gate> --decision "agent replan: <summary>" --note "..."
future loop run --goal G --agent-id <unique-name> ...
```

**Genuine user gates** (approvals, scope changes, risky/irreversible actions):
STOP, quote the exact question and gate id IN THIS CONVERSATION, and wait.
Never guess. After the user decides, `gate resolve` with their decision and
resume.

### 8. Report progress

After each turn: `future loop status --goal G` (+ `quota should-run`). Report
todo state, next action, gates, cost. Close stale open todos the agent did
inline:

```bash
future loop todo complete --goal G --todo-id <stale-id> --no-follow-up --evidence "..."
```

## Key semantics (do not misuse)

- **Terminal ≠ all todos checked.** Completion is validated closure (todos done
  + closure intent + no acceptance gaps). Check `closure_proof`.
- **Never silently complete agent work**: `todo complete` requires
  `--no-follow-up` or `--successor`.
- **Gates freeze ALL work** while any user gate is open (not just linked todos);
  gates complete via `gate resolve`, never `todo complete`.
- **Not every gate is the user's**: `PLAN_REVIEW` checkpoints are agent-resolved;
  genuine human decisions are surfaced and awaited.
- **Monitors**: not-due monitors must NOT be polled (run returns `wait`). Due
  time comes from `--cadence`/`--defer-secs N`; `todo update --resume-when N`
  (numeric) defers N seconds for real, a text value is only a hint with NO
  deadline.
- **Evidence honesty**: report real outputs (paths, test results) — the control
  plane readbacks key results.
- **Deliverables to CWD**: the final todo copies user-facing artifacts to the
  project root.

## Command reference

```bash
future loop status [--goal G] [--format json]
future loop goal init --objective "..." --cwd DIR [--goal-id G] [--goal-doc "..."]
future loop todo add --goal G --text "..." [--priority P0|P1|P2] [--blocks T] [--verify "cmd"] [--role user --class user_gate --gate-question "..."]
future loop todo update --goal G --todo-id T [--text "..."] [--priority ...] [--blocks T]
future loop todo claim --goal G --todo-id T --agent-id A [--lease-secs N]
future loop todo complete --goal G --todo-id T [--no-follow-up | --successor T2] [--evidence "..."]
future loop todo supersede --goal G --todo-id T --reason "..."
future loop gate resolve --goal G --todo-id T --decision "..." [--note "..."]
future loop quota should-run --goal G [--agent-id A]
future loop agent register --goal G --agent-id A        # run auto-registers; onboard declares capabilities
future loop agent list --goal G
future loop lease claim|renew|release|expire|status
future loop run --goal G --agent-id A [--model M] [--thinking-level L] [--max-turns N] [--max-turn-secs N] [--lease-secs N]
future loop backup --goal G [--list | --restore DIR]
future loop serve-status [--port 8791]                  # browser dashboard
future loop models                                      # same catalog as `future models`
```

### Full command surface (`future loop registry`)

Notable extras beyond the workflow commands:

- **todo graph**: `task-graph` (dependency DAG, fails closed on unknown refs);
  `lease claim|renew|release|expire|status`.
- **gates & replan**: `gate resolve`; `replan ack|obligations`.
- **agents**: `agent list|register|onboard`; `scope`; `lane`; `supervisor
  propose|receipt|events`.
- **quota/scheduler**: `quota should-run|usage|spend|tools`; `scheduler
  tick|show|record-host-failure`. `quota tools --goal G` shows per-tool
  quota at the capability boundary (invocations used / limit / trailing
  window): `capability propose` and the per-capability command hooks accept
  `--goal G`, which ledgers every accepted invocation (`capability_invoked`
  event) and refuses calls once a tool reaches 30 accepted invocations in
  the trailing hour (the refusal itself is ledgered); without `--goal` the
  call proceeds uncounted. A saturated tool flips the should-run packet's
  `capability_repair_allowed` predicate to false.
- **ops**: `diagnose [--format json]`; `doctor`; `runs
  history|compact|retention|stale`; `backup`; `evidence-log`; `todo-event`;
  `turn`; `privacy`; `store verify|migrate|bridge`; `authority`; `profile`;
  `version`.
- **work-items & handoff**: `attention`; `inbox`; `handoff --write`.
- **extensions**: `extension install|upgrade|enable|disable|rollback`;
  `capability list|propose|commands|catalog`; `agent-turn-recall`,
  `change-quality`, `content-ops`, `explore`, `integration-branch`,
  `issue-fix`, `periodic-report`, `reward-memory`, `semantic-preference`,
  `value-connectors` (each takes `--input`).
- **benchmark/replay/canary**: `benchmark protocol|run|ledger`; `replay
  record|run`; `canary smoke [--profile core-control-plane|extension-runtime|release-gate]`
  — run `canary smoke` after touching loop code.
