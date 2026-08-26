# Loop Control Plane (`future loop`)

> A local control plane that makes long-running AI agent work durable,
> governable, and verifiable — objectives, gates, todos, evidence, and quotas
> persist outside the chat; the agent executes one bounded turn at a time and
> a deterministic kernel decides what happens next.

> **Attribution.** `future loop` contains code derived from
> [LoopX](https://github.com/huangruiteng/loopx) (Apache-2.0) — see
> `orchestration/loop/NOTICE` and `orchestration/loop/UPSTREAM.md`. Future
> Loop is an independent downstream implementation maintained by FutureGene,
> not an official LoopX release, and not certified or endorsed by the LoopX
> project.

## Why it exists

A conversation loses context; a "keep an eye on this for a week" request
should not live in chat history. `future loop` turns the request into a
**durable goal**: a todo graph, human gates, per-step evidence, and a
verifiable definition of done — surviving sessions, restarts, and parallel
workers.

## At a glance

```
objective
   │
   ├─ todo graph (advancement / user-gate / monitor, --blocks dependencies)
   │
   ├─ human judgment needed? ──▶ ask one concrete question and wait (user gate)
   │
   ├─ safe to proceed? ──▶ kernel decision packet: run this todo / wait / replan / terminal
   │
   ▼
agent executes one bounded turn (gRPC) → writes evidence → kernel decides the next turn
```

## Core concepts

| Concept | Command | What it does |
|---|---|---|
| Goal | `goal init` | Project-local state at `<cwd>/.future/loop/`, event-sourced and replayable |
| Todo | `todo add/update/complete/supersede` | Five classes: advancement / user-gate / user-action / monitor (external-state watch) / blocker; `--blocks` dependency chains; `--priority` |
| Evidence | `todo complete --evidence` | **Non-empty, enforced**: closing a todo must state what actually landed (paths, attempt ids, measurements); `--force` is the explicit operator override |
| Acceptance contract | `todo add --acceptance "tok1,tok2"` | Completion evidence must contain every token (case-insensitive) — the hard form of "done ≠ delivered" |
| Verifier | `todo add --verify "cmd"` | The kernel runs the command after each turn; only exit 0 completes the todo (bounded by `--max-validation-attempts`). The physical blocker of empty closures |
| Lease | `lease claim/renew/release/status` | Who holds a todo and until when. **Lease liveness**: the holder's pid is recorded; a dead process's leases are auto-reclaimed — no manual cleanup after killing a worker |
| Gate | `gate resolve` | Any open user gate freezes all work until resolved; user-actions (non-blocking human to-dos) surface to the user without freezing the agent |
| Delivery closure | `delivery status/record` | Completion lands in a pending `delivered` state; an operator resolves it as `verified/failed/rework`; unverified deliveries auto-derive a follow-up after 3 turns |
| Terminal | `frontier show` | Validated closure: todos done/superseded + closure intent + no acceptance gaps + no pending deferred work; `frontier` gives the terminal judgement with gap detail |
| Dashboard | `ui` | Local read-only web dashboard on 127.0.0.1: goal cards, attention queue, kernel decision, todo DAG, workers/cost, run/event ledgers — live over SSE; mutations stay in the CLI |
| Quota | `quota should-run/usage/spend/decisions` | The deterministic should-run kernel: scheduling, refusal reasons, and spend are all auditable |
| Scheduler | `scheduler tick/show/liveness` | Monitor cadence, host-failure records, liveness heartbeats |
| Multi-agent | `agent contract/recipe/succession/collective` | One goal, several workers: contract (backups / handoff rules), named recipes for one-command onboarding, auto back-up promotion on offline timeout, wake roster, collective turn ledger |
| Frontier | `frontier show` | Outcome segments, structured replan rules, bounded semantic history (N=50), terminal judgement |

## Drive loop via the skill (recommended entry)

In most cases you won't type the CLI below — **use the `/future-loop` skill
and let the agent drive**:

```
You say "/future-loop keep an eye on X for a week"
   │
   ▼
Agent loads the future-loop skill (v3.x driving manual)
   ├─ 1. `future loop status` first — continue an existing goal, never duplicate it
   ├─ 2. Confirm the plan with you (steps/model/thinking level) — unless your message already carries the full objective + constraints
   ├─ 3. `goal init` + decompose todos (dependencies via --blocks, hard checks via --verify/--acceptance)
   ├─ 4. Drive turns: `run --max-turns 1 --agent-id <unique>`, relaunching the moment a turn exits
   ├─ 5. Correct a drifting worker via `todo update --text` (picked up at the next turn)
   ├─ 6. Interrupt a stuck worker mid-turn via `supervisor steer --goal G --instruction "..."`
   ├─ 7. Irreversible/expensive/user-only decisions → open a user gate and wait (gates freeze everything)
   └─ 8. Close out: acceptance todo copies artifacts to the project root → validated closure (terminal)
```

**Skill vs CLI**: the skill owns "what to do when, how to decompose, how to
drive" (the orchestration layer); the CLI is the underlying mechanism (state
kernel + hard checks + decisions). The skill is a maintained v3.x manual, kept
in sync with this page; full semantics at
[future-skills/builtin/future-loop](https://github.com/futuregene/future-skills/tree/main/builtin/future-loop).

## User workflow (zero to closure)

```bash
# 1. Create the goal
future loop goal init --objective "..." --cwd DIR

# 2. Decompose — dependencies and hard checks together
future loop todo add --goal G --text "..." --priority P0 --verify "cargo check -p X"
future loop todo add --goal G --text "..." --blocks T1 --acceptance "attempt,scored"
future loop todo add --goal G --role user --class user_gate --text "Release gate" --gate-question "Ship it?"

# 3. Drive turns (one worker per --agent-id; relaunch as soon as a turn exits)
future loop run --goal G --agent-id mac-worker --model M --thinking-level L --max-turns 1

# 4. Human decisions
future loop gate resolve --goal G --todo-id GATE --decision "approve"

# 5. Observe and close
future loop ui                       # live web dashboard (http://127.0.0.1:7717)
future loop status --goal G
future loop frontier show --goal G        # terminal judgement + gap detail
future loop delivery record --goal G ...  # verified / failed / rework
```

## Web dashboard (`future loop ui`)

`future loop ui [--port N] [--root DIR] [--no-open]` serves a local,
**strictly read-only** dashboard on `127.0.0.1` (default port 7717). It
replays the same event ledger as the CLI on every request and pushes
changes over SSE, so the page is always a faithful, live projection of
`.future/loop/` — and nothing else: the server only reads the loop state
root, and only GET endpoints exist (any other method is a 405). Mutations
(gate resolve, goal cancel, …) stay in the CLI — the page shows the exact
`future loop` command to run instead.

- **Overview**: fleet totals (active/terminal/cancelled goals, open gates,
  24h/7d runs/cost/quota slots), the attention queue (severity, waiting-on,
  recommended action), and per-goal cards sorted by triage order.
- **Goal detail** (tabbed): Board — the kernel's should-run decision
  (reason + code + waiting on), next action, spend/throughput (14-day
  sparkline, token/cost/slot buckets, 7-day outcome split), open gates,
  and the todo dependency DAG (layered, click-through inspector with
  verify/acceptance/lease/evidence detail); Todos — full table with
  per-todo runs/token/cost rollup and activity window; Workers — agent
  leases, heartbeats, liveness alerts, per-worker cost/token rollup,
  delivery closure, replan obligations, acceptance gaps; Runs — the run
  ledger (validation receipts, failure kinds, tokens, cost, evidence) +
  semantic history; Events — the raw event ledger.
- All state is projected from `.future/loop/` on every request; the
  dashboard holds no separate state and writes nothing.

## Hard checks first (conventions fail, gates hold)
- Empty-evidence closures are **refused** (fail-closed by default; `--force` opens)
- `--verify` makes "wrote it" mean "it compiles / the artifact exists" — attach one to every delivery todo
- `--acceptance` turns "accepted by an external observable" into a hard check
- Lease liveness self-heals: dead-process leases are reclaimed automatically — relaunching workers needs no manual release
- Workspace guard: multi-agent write conflicts degrade to serial automatically
- Idle turns (15 minutes without writes) are ledgered; correct the worker via `todo update --text` (applies at the next turn)

## Bidirectional messaging (supervisor ↔ worker)

The supervisor (the orchestrator agent running the `/future-loop` skill) and
its workers exchange messages through the goal ledger — no in-process push
channel. Both directions ride the same event-sourced state:

- **Register the supervisor** (once per goal):
  `future loop supervisor register --goal G --session-id <supervisor-agent-session>`
  This binds the supervisor's agent session id to the goal; workers read it on
  `replay` and target their reports at it.

- **Down (supervisor → worker, an interrupt):**
  `future loop supervisor steer --goal G [--agent-id A] --instruction "..."`
  A `WorkerSteered` event lands in the ledger; the running worker's watch task
  sees it and aborts the in-flight run (a real interrupt, `supersede_session`
  semantics — not a system-prompt note). The next turn drains the instruction
  into its envelope and follows it.

- **Up (worker → supervisor, a report):**
  At a turn boundary the worker enqueues a report (`enqueue_if_busy`, so it
  never interrupts the supervisor) to the registered session for exactly three
  state transitions: a user gate opens (①), a todo completes (②), or a todo
  fails on a science/hard error (③). Each report is idempotent-keyed on the
  transition, so re-sends across runs dedup. The durable user gate remains the
  authoritative intervention channel if no supervisor is registered.

## Loop state is CLI-first

The control plane is driven and observed through the **`future loop` CLI** —
goal state is project-local (`<cwd>/.future/loop/`), not attached to any one
client. The TUI, desktop GUI, mobile apps, and IM bots have no built-in loop
views; they drive the same control plane through the **`/future-loop`
skill**, which orchestrates `future loop` commands on the agent's behalf.
Because the state lives in the project and the skill runs through the agent
service, a goal started in one client (e.g. the TUI) can be driven from any
other (e.g. a Feishu chat).

## CLI surface (10 groups, 43 commands)

```bash
future loop registry        # every command
future loop commands        # grouped by operator journey
```

- **goal group**: `goal` `status` `models` `diagnose`
- **todo group**: `todo` `gate` `replan` `frontier` `lease` `task-graph`
- **agent group**: `agent` `list` `scope` `lane` `supervisor`
- **ops group**: `version` `doctor` `history` `turn` `todo-event` `evidence-log` `backup` `authority` `profile` `quota` `scheduler` `store` `backfill` `privacy` `runs` `heartbeat-prompt` `worker-bridge` `serve-status` `run`
- **work-items group**: `attention` `inbox` `delivery`
- **handoff group**: `handoff`
- **cli group**: `registry` `commands`
- **benchmark group**: `benchmark`
- **replay group**: `replay`
- **canary group**: `canary`

## How it fits FutureOS

- **Agent service** (`future agent`, gRPC 127.0.0.1:50051): `run` executes every turn through it
- **Any client through the skill** (TUI, desktop, mobile, Feishu / DingTalk): loop goals are driven by the `/future-loop` skill orchestrating `future loop` commands — there is no native bridge↔loop integration; gates surface as agent messages asking one concrete question
- **The `/future-loop` skill**: the driving manual for agents (v3.x, kept in sync with this doc)
- **State location**: `<cwd>/.future/loop/` (add to the project `.gitignore`)

## Further reading

- Install & build: [build-and-install.md](build-and-install.md)
- Evidence ledger: [long-run-evidence-ledger.md](long-run-evidence-ledger.md)
- TUI usage: [tui.md](tui.md)
- Skill source: [future-skills/builtin/future-loop](https://github.com/futuregene/future-skills/tree/main/builtin/future-loop)
