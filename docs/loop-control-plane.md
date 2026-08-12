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

The primary way to invoke it is `future loop <command>`; the standalone
`future-loop` binary is equivalent but no longer installed by default —
build it with `cargo build -p future-loop` (or install via
scripts/install-future-loop.sh), and `make run-loop` runs it in dev.

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

`future loop run` asks a pure, injectable-clock kernel whether to run, why,
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
  operator inbox — driven by the `agent` command group (see
  [Multi-agent workflow](#multi-agent-workflow)).

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

Run `future loop` (or `future-loop`) with no arguments for the full grouped help.

## Quick start (skill mode)

Start a conversation with the agent and type:

```
/future-loop Turn this long-running objective into a goal with todos and drive it to completion
```

The skill loads, creates a durable goal, breaks the work into todos (with
dependency chains and a final deliverable-copy todo), and drives it turn by
turn with `future loop run --max-turns 1` — reporting status and cost after
each step.

Or drive it directly from the terminal (`future loop` runs the same code as
the standalone `future-loop` binary):

```bash
future loop goal init --objective "..." --cwd /path/to/project
future loop todo add --goal <id> --text "collect data" --priority P0
future loop todo add --goal <id> --text "write report" --priority P0 \
  --blocks <collect-todo-id> --verify "test -f report.md"
future loop status --goal <id>
future loop run --goal <id> --model future/deepseek-v4-flash --max-turns 1
```

## Multi-agent workflow

The `agent` command group models a goal shared by several agents. Every agent
is identified by an `--agent-id`, scopes its work, and hands over via a
handoff document — so a supervisor (or a human) can reason about who owns
what and what the next agent needs to know.

> The commands below are **flat top-level commands** — `future-loop` dispatches
> `agent`, `scope`, `lane`, `supervisor`, `handoff`, `task-graph`, `attention`
> and `inbox` all at the top level. The `agent` / `todo` / `work-items` groups
> in the help output are only presentation groupings.

### 1. Onboard an agent (register + capabilities)

```bash
# plain registration (prerequisite for quota --agent-id)
future loop agent --goal <id> --agent-id codex

# register AND declare capabilities (input to the capability gate)
future loop agent onboard --goal <id> --agent-id codex --capability shell,github
```

`onboard` records an `AgentOnboarded` event with the declared capabilities.

### 2. Scope & lane

```bash
# identity-scoped frontier: which todos this agent may see/claim, and which
# claims belong to others (outside the frontier)
future loop scope --goal <id> --agent-id codex [--exclude docs,build]

# compact lane recommendation for this agent (classification + action)
future loop lane --goal <id> --agent-id codex
```

The frontier output lists `visible agent todos`, `claimed by self`, `other
agent claims`, `open user gates`, and the `unclaimed advancement` count;
`lane` summarizes the agent's progress scope and a recommended next action.

### 3. Supervisor decisions

```bash
# propose a decision: observe (default) or execute (with capabilities)
future loop supervisor propose --goal <id> --agent-id super --decision-id d1 \
  --target-agent-id codex --kind execute --capabilities shell --summary "run tests"

# record the host's receipt (executed | failed | rejected)
future loop supervisor receipt --goal <id> --decision-id d1 \
  --receipt-id r1 --adapter-id host --outcome executed

# project all supervisor events as JSON
future loop supervisor events --goal <id>
```

### 4. Hand off

```bash
# print the delivery contract (degradation mode + summary) and the handoff doc
future loop handoff --goal <id>

# also write it to .future/loop/goals/<id>/HANDOFF.md
future loop handoff --goal <id> --write
```

The delivery contract is derived from run history (newest first); the handoff
document is rendered as markdown so the next agent can pick up context without
re-reading the whole ledger.

### 5. Coordinate

```bash
# todo dependency graph (topological order; cycles fail closed)
future loop task-graph --goal <id>

# attention queue for one goal, or across all goals
future loop attention --goal <id>
future loop attention --all

# operator inbox urgency projection
future loop inbox --project .
```

## Deployment topology (recommended)

The control plane is deliberately **daemonless**: every `future loop` command
is a short-lived process that loads the ledger, does one bounded thing,
persists, and exits. Availability therefore comes from an **external
scheduler** invoking `future loop run` at your chosen cadence — not from a
long-running loop process you have to keep alive.

```
cron / systemd timer / CI schedule          (the availability source)
   │  one invocation per tick
   ▼
future loop run --goal <id> --agent-id <name> --max-turns 1
   │  bounded slice: decide → execute one turn → writeback → exit
   ▼
<cwd>/.future/loop/                         (event ledger — the only state)
```

Why this is safe to drive externally:

- **Bounded invocations** — each tick is capped by `--max-turns` /
  `--max-turn-secs`; a wedged turn stops gracefully instead of holding the
  goal, and the next tick resumes from the ledger.
- **Restart-safe state** — the event ledger uses content-addressed, idempotent
  appends; a crashed or overlapping tick replays cleanly, and conflicts fail
  closed rather than double-spending quota.
- **Lease coordination** — `run` claims todos under a lease (default 4h,
  `--lease-secs`), so two schedulers cannot silently race the same todo.
  Always pass a stable `--agent-id` (auto-registered on first use);
  `--anonymous` opts out of coordination and can race.
- **Fail-closed kernel** — a cancelled goal never runs; an ambiguous state
  stops instead of spending.

Example drivers:

```cron
# cron — one bounded turn every 15 minutes
*/15 * * * * cd /path/to/project && future loop run --goal <id> --agent-id cron-worker --max-turns 1 >> .future/loop/cron.log 2>&1
```

```ini
# /etc/systemd/system/loop-worker.service
[Service]
Type=oneshot
WorkingDirectory=/path/to/project
ExecStart=/usr/local/bin/future loop run --goal <id> --agent-id systemd-worker --max-turns 1

# /etc/systemd/system/loop-worker.timer
[Timer]
OnCalendar=*:0/15
Persistent=true
```

```yaml
# CI scheduled tick (GitHub Actions). CI runners are ephemeral: persist
# .future/loop/ across runs (e.g. actions/cache) or every tick restarts
# from an empty ledger.
on:
  schedule: [{ cron: "*/30 * * * *" }]
  workflow_dispatch:
jobs:
  tick:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: future loop run --goal <id> --agent-id ci-worker --max-turns 1
```

The scheduler state machine complements the external driver:
`future loop scheduler tick|show` keeps a restart-safe cadence progression
(useful when the driver wants backoff state), and
`future loop scheduler record-host-failure` records missed/late host ticks so
liveness gaps surface in state instead of going silent.

### Optional persistent runner

A daemon is never required, but two long-running conveniences exist:

- **Wrapper loop** (workstations): `while true; do future loop run --goal <id>
  --agent-id local-runner --max-turns 1; sleep 300; done` — the same
  bounded-turn semantics without cron.
- **`future loop serve-status [--port 8791]`** — a zero-dependency, GET-only
  HTTP dashboard (`GET /`, `GET /goals.json`). It is a read-only projection
  and never a second source of truth; run it alongside any topology for
  observability.

For a fully custom runner, `future loop worker-bridge` exposes the reference
stdio contract: the bridge emits one typed turn packet per line on stdout,
your worker executes the bounded turn in its own runtime, and writes one JSON
result line back. Pick **one driver per goal** (or one per
`(goal, agent-id)` in multi-agent setups) — leases make overlaps safe, but a
single driver keeps cadence and quota accounting predictable.

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
make install-skills                      # preferred: links the /future-loop skill (no build —
                                          # `future loop` runs through the unified CLI)
# optional standalone binary (dev use):
bash scripts/install-future-loop.sh      # CLI → ~/.local/bin/future-loop, skill → ~/.future/agent/skills/
# or build in the workspace:
cargo build -p future-loop
```

Prerequisites and full product build/install steps are in
[Build & Install](build-and-install.md).
