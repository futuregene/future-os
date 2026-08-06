# Loop Control Plane (`future-loop`)

> The local control plane for long-running AI agent work — keep objectives,
> gates, todos, evidence, quota, and handoffs stable while an agent executes
> bounded turns.

FutureOS ships a native loop control plane in `orchestration/loop` as the
`future-loop` CLI plus the `/future-loop` agent skill. It turns a conversation
into a durable, reviewable long-running goal: goals, todos, human gates,
monitors, evidence, and completion are persisted outside the chat, and a
deterministic kernel decides what should happen next — one bounded turn at a
time.

> `future-loop` is a Rust rewrite of the
> [loopx](https://github.com/huangruiteng/loopx) control plane, adapted and
> extended for FutureOS (project-local state, gRPC executor bridge, quota
> kernel, extensions & multi-agent).

```
objective / issue / project
   │
   ▼
loop state: objective + gates + todos + scope + evidence + quota
   │
   ├─ human judgment needed? ── yes ─▶ ask a concrete question and wait
   │
   ├─ safe fallback available? ──────▶ run one bounded agent slice
   │
   ▼
agent executes one turn (gRPC) → write evidence + handoff + next todo
   │
   ▼
quota decides the next tick
```

## Highlights

### Durable goals & todo work graph

- **Goals** (`goal init / cancel / delete`): project-local state under
  `<cwd>/.future/loop/`, persisted as an event ledger with replay.
- **Todos** (`todo add / claim / complete / supersede / update / archive`):
  advancement / user-gate / monitor / blocker classes, priorities, dependency
  chains (`--blocks`), claim + lease lifecycle, and the reference-compatible
  completion contract (every completed todo declares a successor or an
  explicit no-follow-up).
- **Human gates** (`gate resolve`): a todo blocks on a concrete question until
  a human decision lands — never a vague "waiting".
- **Monitors** (`--class monitor --cadence ...`): scheduled observation of
  external state (CI, PR, a file), with no-change backoff so a stale target
  never spends quota.

### Deterministic should-run decision kernel

`future-loop run` asks a pure, injectable-clock kernel whether to run, why,
and which todo — identity-scoped frontiers, user-gate precedence, repair
budgets, outcome floors, succession replan obligations, acceptance gaps, and
a scheduler-arbitration layer that classifies every decision into one of nine
dispositions (terminal / monitor-wait / active work / consistency repair /
human gate / quiet wait / …). Enforcement is fail-closed: a cancelled goal
never runs; an ambiguous state stops rather than spends.

### Quota & scheduler

- Slot accounting across `run` / `agent` / `heartbeat` sources, 24h/7d usage
  summaries, and stall repair that detects surface-only progress loops.
- A scheduler state machine with cadence normalization
  (`once / hourly / daily / weekly` or `15m / 1h / 2d`), atomic persistence,
  and host-failure tracking.
- Monitor polls land as replayable events (`MonitorPolled`) with exact
  writeback.

### Event-sourced state & projections

- Content-addressed event ids with idempotent append and fail-closed conflict
  detection; `QuotaSpent` / `EvidenceAttached` events; markdown backfill into
  the ledger.
- Per-goal schema migration bridge (verify / migrate / bridge), privacy-graded
  projections (public-safe / local-private / private-pointer), and a run
  lifecycle (history / compaction / index / retention / stale detection).

### Independent validation

`todo add --verify "cargo test" --max-validation-attempts 5` attaches an
independent validator: the kernel runs it in the goal's cwd after each turn,
only completes the todo when it exits 0, and replans when the retry budget is
exhausted — closure stays validated, not self-judged.

### Extensions & multi-agent

- A capability framework with a provider lifecycle
  (declared → installed → enabled → ready), a queryable catalog, a capability
  gate (run / ask-owner / repair-bridge / skip), and per-capability command
  hooks.
- Extensions with declarative manifests and install / enable / disable /
  rollback (revision-retained), plus a readiness doctor — v1 is declarative
  and never executes extension code.
- Identity-scoped multi-agent: agent scope frontiers, lane recommendations,
  supervisor proposal/receipt events, task leases, handoff documents with a
  delivery contract, a todo dependency graph, and an attention queue /
  operator inbox.

### Evaluation & diagnostics

- A benchmark closed loop (protocol / run / ledger over the same gRPC
  channel), decision replay with a model-behavior corpus, and a canary smoke
  suite (`core-control-plane` / `extension-runtime` / `release-gate`).
- `version` / `doctor` / `history` / `turn` / `todo-event` / `evidence-log`
  diagnostics, plus `backup` / restore.

## CLI overview

```text
goal          goal lifecycle (init / cancel / delete) · status · models · diagnose
todo          add | claim | complete | supersede | update | archive
              gate resolve · replan ack · lease · task-graph
agent         onboard · scope · lane · supervisor
capability    list | propose | commands · catalog · per-capability hooks
extension     install | upgrade | enable | disable | rollback | status | capabilities
ops           version · doctor · history · turn · todo-event · evidence-log
              backup · authority · profile · quota · scheduler · store
              backfill · privacy · runs · heartbeat-prompt · worker-bridge
              serve-status · run
work-items    attention · inbox
handoff       handoff [--write]
benchmark     protocol | run | ledger
replay        record | run · corpus build | run
canary        smoke [--profile ...]
cli           registry [--json] [--include-experimental]
```

Run `future-loop` with no arguments for the full grouped help.

## Quick start (skill mode)

Start a conversation with the agent and type:

```
/future-loop Turn this long-running objective into a goal with todos and drive it to completion
```

The skill loads, creates a durable goal, breaks the work into todos (with
dependency chains and a final deliverable-copy todo), and drives it turn by
turn with `future-loop run --max-turns 1` — reporting status and cost after
each step.

Or drive it directly from the terminal:

```bash
future-loop goal init --objective "..." --cwd /path/to/project
future-loop todo add --goal <id> --text "collect data" --priority P0
future-loop todo add --goal <id> --text "write report" --priority P0 \
  --blocks <collect-todo-id> --verify "test -f report.md"
future-loop status --goal <id>
future-loop run --goal <id> --model future/deepseek-v4-flash --max-turns 1
```

## State layout

```
<cwd>/.future/loop/registry.json                        — registry (source of truth)
<cwd>/.future/loop/goals/<id>/events.jsonl              — per-goal event ledger
<cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md      — reference-compatible projection
<cwd>/.future/loop/runs/                                — run history
```

Runtime state is never written outside the project; add `.future/loop/` to
`.gitignore`.

## Install

```bash
bash scripts/install-future-loop.sh        # CLI → ~/.local/bin/future-loop, skill → ~/.future/agent/skills/
# or build in the workspace:
cargo build -p future-loop
```

Prerequisites and full product build/install steps are in
[Build & Install](build-and-install.md).
