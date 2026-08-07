---
version: 1.0.1
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

- The loop control plane runs through the unified CLI: **`future loop
  <cmd>`**. Before anything else, look for an existing install on the
  system (`command -v future`; also check common locations like
  `~/.local/bin/future`, `~/bin/future`, Homebrew cask/brew links). If
  `future loop` is NOT available, do NOT rebuild from source — point the
  user to the official install instructions at
  <https://github.com/futuregene/future-os> (README install section) and
  wait for them to install. (This skill installs to
  `~/.future/agent/skills/future-loop/`.) State lives in the PROJECT:
  `<cwd>/.future/loop/` (run from the project dir, or pass --cwd).
- The agent must be running for `future loop run` — start it with
  `future agent` (gRPC 127.0.0.1:50051). Probe it with `future models`
  — a plain curl to the gRPC port reports nothing useful.
- **Verify the CLI binary is not stale before relying on recent loop
  features.** The installed `future` binary can be older than the loop
  source (observed 2026-08: session auto-cleanup, `todo update --blocks`,
  and gate enforcement silently absent from an out-of-date binary — the
  commands still "work" but skip the new behavior with no warning). Quick
  freshness probe: `strings $(command -v future) | grep -c "session cleanup failed"`
  — `1` = fresh (session auto-clean present), `0` = stale → rebuild with
  `cargo build -p future-cli` (or reinstall) before relying on loop features.
  If you see `⚠ session cleanup failed` lines or stray per-run sessions in
  `future session list`, treat it as the same stale-binary symptom.

## State layout (project-local, all under `<cwd>`)

```
<cwd>/.future/loop/registry.json                        — registry (source of truth)
<cwd>/.future/loop/goals/<id>/events.jsonl              — per-goal event ledger
<cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md      — reference-compatible projection
<cwd>/.future/loop/runs/                                — run history
```

Runtime state is NEVER written outside the project. Add `.future/loop/` to the
project `.gitignore` (runtime state, never commit).

## Workflow

### 1. Inspect existing goals first

```bash
future loop status
```
If the user's objective already exists, continue it — never silently create a duplicate.

### 2. Confirm the plan with the user BEFORE creating anything

Before `goal init`, present a concrete plan and get the user's confirmation:

1. **Steps** — the todos you will execute, in order (e.g. collect data →
   analyze → write report → copy deliverables). Keep it short and concrete.
2. **Model + thinking level** — default to the CURRENT session's model and
   thinking level (read them from your own environment: `Current model:` /
   `Thinking level:`), and propose those as the default. Keep the user's
   adjustment cost low:
   - if the user wants a different model, run `future models` to list
     alternatives (`--json` for machine-readable output); prefer a
     model flagged `[recommended]`, else `[default]`;
   - thinking level: `high` for reasoning/analysis-heavy goals, `off`/`low`
     for mechanical or fast-turn work — `future models` shows each
     model's suggested level;
   - if the agent is not running, `future models` will fail — fall
     back to the current session's model + `high` and tell the user the model
     list was unavailable.
3. Ask for confirmation, e.g.:
   ```
   Plan confirmation (goal: <objective text>)
   Steps: 1) <...>  2) <...>  3) <...>
   Model: <current session model, e.g. future/deepseek-v4-flash> (default = current session; can switch)
   thinking: <current session thinking level, e.g. high> (default = current session; adjustable)
   Confirm start? (model/thinking or steps adjustable)
   ```

Do NOT create the goal or todos until the user confirms (or adjusts).

### 3. Create a goal (or reuse)

```bash
future loop goal init --objective "..." --cwd <project-dir> [--goal-id <id>]
```
When the user names a directory (e.g. "in /tmp/foo"), pass it as `--cwd`;
when they name a repo, use its root. If the directory does not exist,
create it first (`mkdir -p`).

### 4. Break the work into todos (optional but recommended)

```bash
future loop todo add --goal <goal-id> --text "..." --priority P0
```

**Dependency chains.** When a todo logically depends on the output of another
todo, encode that with `--blocks`. Without `--blocks`, the run loop sees all
open todos as independently runnable and may reorder or skip them. Example:

```bash
# Report generation depends on first fetching data:
future loop todo add --goal <goal-id> --text "Generate the change-analysis summary report" --priority P0 \
  --blocks <data-todo-id-1>,<data-todo-id-2>
# Copy-to-CWD must happen after the report exists:
future loop todo add --goal <goal-id> --text "Copy the final deliverables to the project root (cwd)" --priority P0 \
  --blocks <report-todo-id>
```

Rule of thumb: if "X must finish before Y can start", add `--blocks X` to Y.

**Todo-creation pitfalls (field-tested 2026-08, cli-rust-port + tui-rust-port goals).**

1. **Capture todo ids from the `todo add` output itself.** The command prints
   the new id (`todo todo_xxx added ✔`) — that is the only reliable source.
   There is NO `future loop todo list` subcommand, and `future loop status`
   has NO `--format json` flag (only `future models` does) — piping
   `--blocks $(future loop status --format json …)` fails SILENTLY (the
   substitution yields an empty string, `--blocks` is accepted with no value,
   and the dependency is quietly dropped). To look ids up afterwards, parse
   `future loop status --goal G` (`todos:` line lists `id=status`), or read
   the event ledger `<cwd>/.future/loop/goals/<id>/events.jsonl`
   (`kind: "todo_added"` events carry the full `todo` object). **Field-name
   gotcha:** in the ledger the dependency is `blocked_by_gate` (NOT `blocks`)
   and the validator is `validator` (NOT `verify`) — inspecting a todo with
   guessed field names will falsely report flags as dropped; print the whole
   object (or the goal's `schema.json`) before concluding.
2. **Verify the wiring after creation.** Run `future loop status --goal G` and
   confirm each dependent todo's `blocks` is set. An empty `--blocks` never
   errors — only a status check catches it. **Repair in place with
   `todo update --blocks`** (supported since the 2026-08 loop update):
   `future loop todo update --goal G --todo-id T --blocks a,b` REPLACES the
   blocking set, and `--blocks ""` clears it; an update without `--blocks`
   leaves the set untouched. Older loop builds silently ignored `--blocks`
   — for those, the only repair was to
   recreate the goal: `future loop goal delete --goal G --force` (delete is
   IRREVERSIBLE and refuses without `--force`) → `goal init` → re-add all
   todos in dependency order, capturing each id from the add output.
3. **Chain the FINAL validation todo too.** The run loop schedules any open,
   unblocked todo — priority alone does NOT order execution. The last
   acceptance/validation todo MUST `--blocks` all implementation todos;
   otherwise it can be picked while they are still stubs (observed: the
   differential-test todo ran mid-port and had to be re-planned via
   `complete --successor` back to the implementation todos).
4. **`goal init` auto-creates an onboarding todo.** A fresh goal starts with
   one extra open todo ("Run `future loop status` … record the goal count as
   evidence"). Complete it with `--no-follow-up` during setup (or at the end,
   with the goal count as `--evidence`); don't mistake it for a real work
   item — it does not block the chain but stays open until completed.
5. **`goal delete` requires `--force` and is irreversible.** A mis-created
   goal cannot be repaired in place — `future loop goal delete --goal G`
   refuses with "irreversible — pass --force". Passing `--force` removes the
   registry entry + state, so recreate via `goal init` afterwards.
6. **`registry.json` holds only goal summaries** (objective/cwd/status); todo
   ids and their `blocks`/priority live in the event ledger
   `<cwd>/.future/loop/goals/<id>/events.jsonl` — the reliable place to
   re-derive ids after a botched creation.
7. **Subcommand `--help` is NOT supported.** `future loop todo update --help`
   silently ignores the flag (and "updates" even a nonexistent `--todo-id`).
   Check the exact flags in `orchestration/loop/src/main.rs`
   (`todo_add`/`todo_update` `parse_pairs`) or in this skill instead.
8. **The agent treats the objective as an instruction list — keep
   cleanup/deletion words OUT of test goals.** The run-loop agent may execute
   `future loop goal delete --force` itself if the objective mentions
   "cleanup/delete" (observed 2026-08: a smoke-test goal whose objective ended
   with "清理" got self-deleted by the agent mid-run). Since 2026-08 the loop
   no longer panics on this (`goal X not found (deleted while running?)` is a
   clean error), but the goal state is gone either way — delete test goals
   yourself, don't ask the agent to.

If the user asks for approval before a specific action, create a real gate:
```bash
future loop todo add --goal <goal-id> --role user --class user_gate \
  --gate-question "<the exact question>" --text "<the exact question>" \
  --blocks <todo-id-of-the-action>
```
Describing approval in the objective text is NOT enough — a gate todo is
what actually blocks the action. Semantics (verified): the `--blocks` value
is a declarative link; what actually freezes work is the gate being OPEN —
the run loop returns `AskUser` for the whole goal until the gate is resolved
(any open gate freezes everything, not just the linked todo), and since the
2026-08 hardening pass `todo complete` also rejects completing non-gate
todos while a gate is pending.

**Conditional iteration (validate until it passes).** When a todo must
iterate until a condition is met (e.g. optimize until tests pass), attach an
independent validator — the kernel runs it in the goal's cwd after each turn
and only completes the todo when it exits 0:

```bash
future loop todo add --goal <goal-id> --text "Optimize until tests pass" --priority P0 \
  --verify "cargo test" --max-validation-attempts 5
```

- `--verify "<command>"`: exit 0 → todo completes (validated); non-zero →
  todo stays open and the loop iterates (repair), bounded by
  `--max-validation-attempts` (default 3);
- after the budget is exhausted the kernel replans and surfaces to the user
  (final judgment stays human);
- a todo without `--verify` keeps the old behavior (agent self-judged).

### 5. Add a deliverable-copy todo as the final step

Every goal that produces files (reports, CSVs, docs, etc.) MUST end with
a final P0 todo that copies the deliverables from the loop state directory
to the CWD:

```bash
future loop todo add --goal <goal-id> --text "Copy the final deliverables to the project root (cwd)" --priority P0
```

This todo should be the last in the chain — add it as the successor of the
report-generation todo. When executing it, copy files with `cp` (not mv)
from `<cwd>/.future/loop/goals/<id>/` to `<cwd>/`, so the user can find
results directly in the project root without digging into `.future/loop/`.

### 6. Run the agent — one turn at a time

Use the confirmed model and thinking level from step 2. **Always use
`--max-turns 1`** and loop manually — running all turns in one shot makes the
user wait with no visibility; a turn-by-turn loop lets them see each step's
progress, cost, and any issues as they happen:

```bash
future loop run --goal <goal-id> --model <confirmed-model> \
  --thinking-level <confirmed-thinking> --max-turns 1
```

> **Session lifecycle (no manual cleanup needed):** each `run` creates a
> fresh scratch agent session, executes the bounded turn loop, then **deletes
> the session automatically on every exit path** (terminal, gate, monitor
> wait, max-turns, or error) — context is replayed into each run via the
> turn envelope from the goal events.jsonl, so nothing durable lives in the
> agent session. `~/.future/agent/sessions/` should therefore NOT accumulate
> per-run files. If you ever see a stray `⚠ session cleanup failed` line, or
> want to tidy leftover sessions from other tools, clean them manually with
> the top-level CLI: `future session list` to find ids, then
> `future session delete <id>` (same agent `delete_session` RPC the loop
> uses).

> **Long turns vs. shell timeouts**: a single `run --max-turns 1` turn often
> takes several minutes — the ~120s default timeout of typical shell tools is
> usually NOT enough. Run **blocking** with an explicit longer timeout on the
> shell call (e.g. `timeout: 1800`), and poll `future loop status` between
> turns. Avoid backgrounding the run (nohup/&): it hides progress, risks
> overlapping turns, and the kernel expects one run at a time. If a blocking
> run is interrupted, check `status` before re-running to see what completed.

After each `run`, before starting the next step:
1. Report what was done (which todo, cost, new status).
2. **Reflect & improve — run a deliberate reflection pass (not just a status
   check)**: before deciding what's next, step back and ask:
   - **What did this turn reveal?** New facts, surprises, or assumptions that
     turned out wrong (write them down — they are plan inputs, not noise).
   - **Is the objective still right?** Does the goal text match what the work
     has shown? Are the acceptance criteria still valid and testable?
   - **Is the todo decomposition still optimal?** Too coarse → split into
     smaller todos; too fine → merge; wrong order → reorder with `--blocks`;
     missing a step → `todo add`; obsolete step → `todo supersede --reason`.
   - **Is there a better path?** Would a different approach, tool, or
     ordering reach the objective faster or with higher quality? Don't keep
     a plan just because it was approved — the plan serves the objective.
   - **New risks / dependencies?** Anything that now needs a human gate
     (`--role user --class user_gate`), a monitor (`--class monitor`), or a
     validation hook (`--verify`) to keep the goal on track?
   - **Periodic deep re-plan** (every ~3–5 turns or at milestones): re-read
     the objective and the full todo list, and rewrite the remaining plan as
     if you were planning it fresh with everything you now know.
   Apply the answers via the CLI immediately — you are allowed to adjust the
   plan yourself, no user confirmation for routine replans:
   - `todo add` — new steps discovered by the completed step;
   - `todo supersede --reason "..."` — steps that are now obsolete;
   - `todo update` — fix a step's text/priority/blocks (`--blocks a,b` replaces
  the blocking set, `--blocks ""` clears it);
   - `todo archive` — tidy completed work.
3. **Check whether the plan still holds** — `future loop status --goal <goal-id>`:
   does the remaining todo list still make sense given what this step revealed?
4. Stop and ask the user ONLY when absolutely necessary (see step 7):
   risky/irreversible changes, decisions only the user can make, or you
   cannot determine the right adjustment.
5. If the exit code is non-zero, it means `--max-turns 1` was hit — check
   `future loop status --goal <goal-id>` and run again ONLY if open todos
   remain.
6. If validated closure is reached (terminal), stop.

`run` stops when: validated closure (terminal), a human gate needs the user,
a blocker waits (unresolved `--blocks`), or max-turns is reached.

### 7. Handle replan gates — resolve them yourself; escalate only when necessary

**Replan gates (plan needs adjustment).** When the kernel decides the plan
cannot continue as-is (validation/repair budget exhausted, outcome floor,
acceptance gaps, succession obligation), it injects a `PLAN_REVIEW` user gate.
**Handle it yourself first** — the agent owns routine replans:

1. review the plan: `future loop status --goal G` (and `future loop diagnose --goal G`);
2. adjust the todo list with the CLI: `todo add` / `todo update` /
   `todo supersede --reason "..."` / `todo archive`;
3. resolve the gate and re-run:
   ```bash
   future loop gate resolve --goal G --todo-id <gate> --decision "agent replan: <summary>" --note "..."
   future loop run --goal G ...
   ```

**Escalate to the user ONLY when absolutely necessary** — never guess a
human decision:
- the adjustment is risky or hard to reverse (deleting real work, changing
  the goal/acceptance, production actions, approvals);
- a decision is required that only the user can make;
- you tried but cannot determine the right adjustment.

Then stop and report the gate IN THIS CONVERSATION, quoting the exact
question and the gate todo id, and wait — do not resolve it yourself. After
the user decides, resolve with their decision and resume `future loop run`.

After each `run`, if the output contains `USER GATE` / `ask_user`:
1. if it is a `PLAN_REVIEW` gate and the adjustment is routine → self-resolve
   (steps 1–3 above);
2. otherwise (a genuine user decision) → stop, report the exact question and
   gate id to the user IN THIS CONVERSATION, and wait;
3. after the user decides:
   ```bash
   future loop gate resolve --goal <goal-id> --todo-id <gate-id> --decision "<user's decision>" --note "..."
   ```
4. resume with `future loop run` again.

### 8. Report progress

After each turn, always run status and report the result to the user:

```bash
future loop status --goal <goal-id>
future loop quota should-run --goal <goal-id> [--agent-id <id>]
```
Report: current todo state, next action, any gates, cost if available.

Also check for stale open todos — if the agent did the work inline (e.g.
fetched issues data while generating the report without marking the issues
todo done), close them manually:

```bash
future loop todo complete --goal <goal-id> --todo-id <stale-id> --no-follow-up \
  --evidence "data already collected and included in report"
```

## Key semantics (do not misuse)

- **Terminal ≠ all todos checked.** Completion is validated closure
  (todos done + closure intent + no acceptance gaps). Check `closure_proof`.
- **Never silently complete agent work**: `todo complete` requires
  `--no-follow-up` or `--successor`; the CLI rejects silent completion.
- **Gates freeze ALL work until resolved — enforced at the CLI too.**
  The run loop returns `AskUser` while any user gate is open (it does NOT
  gate by reverse `--blocks` wiring — any open gate freezes everything), and
  `todo complete` now rejects completing a non-gate todo while a gate is
  pending. To complete gated work: `gate resolve` the gate first, then
  complete the todo. Gates themselves are completed via `gate resolve`, never
  `todo complete`.
- **Never guess a gate decision — but not every gate is the user's.**
  Gates created by the kernel as `PLAN_REVIEW` replan checkpoints are the
  agent's to resolve (review plan → adjust todos via CLI → resolve gate).
  Gates that pose a genuine human decision (approvals, scope/goal changes,
  risky/irreversible actions) MUST be surfaced to the user IN the
  conversation — stop and wait, never guess.
- **Monitors**: not-due monitors must NOT be polled; `run` handles cadence
  (a run with only not-due monitors returns `wait / wait_monitor`). A
  monitor's due time is driven by its cadence (`--cadence 15m|1h|2d` or a
  class) or `--defer-secs N`; numeric `todo update --resume-when N` also
  defers N seconds from now (real deadline), while a non-numeric value is a
  text-only hint with NO deadline (the todo stays deferred forever — don't
  use text hints when you need the todo to become due again).
- **`--verify` semantics**: exit 0 → todo completes validated; exit non-zero
  → validation fails, the kernel marks repair-required and retries up to
  `--max-validation-attempts` (default 3), then replans. NOTE: the agent
  itself can still `todo complete --no-follow-up` a failing validator via the
  CLI (agent autonomy) — the validator only gates the kernel's automatic
  completion path.
- **Evidence honesty**: report real outputs (paths, test results); the
  control plane readbacks key results itself.
- **Deliverables to CWD**: agent turns write artifacts into the loop state
  directory. The final todo in every goal MUST copy user-facing deliverables
  to the CWD so the user finds them immediately at the project root.

## Command reference

All commands below use the unified `future loop` form.

```bash
future loop status [--goal G]
future loop goal init --objective "..." --cwd DIR [--goal-id G] [--goal-doc "..."]
future loop todo add --goal G --text "..." [--priority P0|P1|P2] [--role user --class user_gate --gate-question "..." --blocks T]
future loop todo update --goal G --todo-id T [--text "..."] [--priority ...] [--blocks T]   # fix wiring after add
future loop todo claim --goal G --todo-id T --agent-id A [--lease-secs N]
future loop todo complete --goal G --todo-id T [--no-follow-up | --successor T2] [--evidence "..."]
future loop todo supersede --goal G --todo-id T --reason "..."
future loop gate resolve --goal G --todo-id T --decision "..." [--note "..."]
future loop quota should-run --goal G [--agent-id A]
future loop heartbeat-prompt --goal G          # re-entry packet for the next turn
future loop run --goal G [--model M] [--thinking-level L] [--max-turns N]
future loop backup --goal G [--list | --restore DIR]
future loop serve-status [--port 8791]         # browser dashboard

> Model listing is available BOTH on the top-level CLI (`future models
> [--json]`) AND as `future loop models` (same catalog). Session cleanup is
> automatic per `run`; for manual cleanup of leftover sessions use the
> top-level `future session list` / `future session delete <id>`.

### Full command surface (52 commands, 12 groups — `future loop registry`)

Beyond the workflow commands above, the control plane ships a wider surface
(verified 2026-08, v0.0.1574). Use `future loop --help` / `future loop
registry` for the authoritative list; the notable extras:

- **todo graph**: `todo update` (text/priority/blocks/evidence/note —
  `--blocks a,b` replaces, `--blocks ""` clears, absent leaves untouched);
  `task-graph` (dependency DAG — **fails closed** on unknown block refs);
  `lease claim|renew|release|expire|status` (task leases — claim requires the
  agent to be `agent register`ed for the goal).
- **gates & replan**: `gate resolve`; `replan ack` / `replan obligations`
  (kernel-injected plan-review checkpoints).
- **agents**: `agent register|onboard` (onboard declares capabilities);
  `scope` (identity-scoped frontier); `lane` (lane recommendation);
  `supervisor propose|receipt|events`.
- **quota/scheduler**: `quota should-run|usage|spend`;
  `scheduler tick|show|record-host-failure`.
- **ops**: `diagnose` (per-goal decision surface, supports `--format json`);
  `doctor` (ledger integrity + canary self-check); `runs
  history|compact|retention|stale`; `backup --list|--restore`;
  `evidence-log`; `todo-event`; `turn` (per-todo turn envelope);
  `privacy` (privacy-graded projection); `store verify|migrate|bridge`;
  `authority`; `profile`; `version`.
- **work-items & handoff**: `attention` (attention queue); `inbox` (operator
  inbox urgency); `handoff --write` (handoff doc + delivery contract).
- **extensions**: `extension install|upgrade|enable|disable|rollback`;
  `capability list|propose|commands|catalog`; `agent-turn-recall`,
  `change-quality`, `content-ops`, `explore`, `integration-branch`,
  `issue-fix`, `periodic-report`, `reward-memory`, `semantic-preference`,
  `value-connectors` (each takes `--input` for one private turn context).
- **benchmark/replay/canary**: `benchmark protocol|run|ledger`;
  `replay record|run` (+ `replay corpus build|run`); `canary smoke
  [--profile core-control-plane|extension-runtime|release-gate]` (release
  gate default) — run `canary smoke` after touching loop code.
```
