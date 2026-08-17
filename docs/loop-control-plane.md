# Loop Control Plane (`future loop`)

> A local control plane that makes long-running AI agent work durable,
> governable, and verifiable — objectives, gates, todos, evidence, and quotas
> persist outside the chat; the agent executes one bounded turn at a time and
> a deterministic kernel decides what happens next.

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
| Todo | `todo add/update/complete/supersede` | Three classes: advancement / user-gate / monitor (external-state watch); `--blocks` dependency chains; `--priority` |
| Evidence | `todo complete --evidence` | **Non-empty, enforced**: closing a todo must state what actually landed (paths, attempt ids, measurements); `--force` is the explicit operator override |
| Acceptance contract | `todo add --acceptance "tok1,tok2"` | Completion evidence must contain every token (case-insensitive) — the hard form of "done ≠ delivered" |
| Verifier | `todo add --verify "cmd"` | The kernel runs the command after each turn; only exit 0 completes the todo (bounded by `--max-validation-attempts`). The physical blocker of empty closures |
| Lease | `lease claim/renew/release/status` | Who holds a todo and until when. **Lease liveness**: the holder's pid is recorded; a dead process's leases are auto-reclaimed — no manual cleanup after killing a worker |
| Gate | `gate resolve` | Any open user gate freezes all work; PLAN_REVIEW checkpoints are agent-resolved |
| Delivery closure | `delivery status/record` | Completion lands in a pending `delivered` state; an operator resolves it as `verified/failed/rework`; unverified deliveries auto-derive a follow-up after 3 turns |
| Terminal | `frontier show` | Validated closure: todos done/superseded + closure intent + no acceptance gaps + no pending deferred work; `frontier` gives the terminal judgement with gap detail |
| Quota | `quota should-run/usage/spend/decisions` | The deterministic should-run kernel: scheduling, refusal reasons, and spend are all auditable |
| Scheduler | `scheduler tick/show/liveness` | Monitor cadence, host-failure records, liveness heartbeats |
| Multi-agent | `agent contract/recipe/succession/collective` | One goal, several workers: contract (backups / handoff rules), named recipes for one-command onboarding, auto back-up promotion on offline timeout, wake roster, collective turn ledger |
| Frontier | `frontier show` | Outcome segments, structured replan rules, bounded semantic history (N=50), terminal judgement |

## User workflow (zero to closure)

```bash
# 1. Create the goal
future loop goal init --objective "..." --cwd DIR

# 2. Decompose — dependencies and hard checks together
future loop todo add --goal G --text "..." --priority P0 --verify "cargo check -p X"
future loop todo add --goal G --text "..." --blocks T1 --acceptance "attempt,scored"
future loop todo add --goal G --role user --class user_gate --gate-question "Ship it?"

# 3. Drive turns (one worker per --agent-id; relaunch as soon as a turn exits)
future loop run --goal G --agent-id mac-worker --model M --thinking-level L --max-turns 1

# 4. Human decisions
future loop gate resolve --goal G --todo-id GATE --decision "approve"

# 5. Observe and close
future loop status --goal G
future loop frontier show --goal G        # terminal judgement + gap detail
future loop delivery record --goal G ...  # verified / failed / rework
```

## Hard checks first (conventions fail, gates hold)

- Empty-evidence closures are **refused** (fail-closed by default; `--force` opens)
- `--verify` makes "wrote it" mean "it compiles / the artifact exists" — attach one to every delivery todo
- `--acceptance` turns "accepted by an external observable" into a hard check
- Lease liveness self-heals: dead-process leases are reclaimed automatically — relaunching workers needs no manual release
- Workspace guard: multi-agent write conflicts degrade to serial automatically
- Idle turns (15 minutes without writes) are ledgered; steer mid-turn via `todo update --text`

## Three-client experience

Loop state is visible through every FutureOS frontend: **TUI in the terminal,
the desktop GUI, and the mobile apps (Android · iOS)** — one gRPC agent
service, seamless across clients. Mobile is a FutureOS differentiator: most
agent runtimes are desktop-only, while `future` runs natively on your phone,
completing the all-platform story with desktop and TUI.

## CLI surface (10 groups, 43 commands)

```bash
future loop registry        # every command
future loop commands        # grouped by operator journey
```

- **goal group**: `goal` `status` `models` `diagnose`
- **todo group**: `todo` `gate` `replan` `frontier` `lease` `task-graph`
- **agent group**: `agent` `scope` `lane` `supervisor`
- **ops group**: `version` `doctor` `history` `turn` `todo-event` `evidence-log` `backup` `authority` `profile` `quota` `scheduler` `store` `backfill` `privacy` `runs` `heartbeat-prompt` `worker-bridge` `serve-status` `run`
- **work-items group**: `attention` `inbox` `delivery`
- **handoff group**: `handoff`
- **quality group**: `benchmark` `replay` `canary`

## How it fits FutureOS

- **Agent service** (`future agent`, gRPC 127.0.0.1:50051): `run` executes every turn through it
- **Channel bridges** (Feishu / DingTalk): messages can trigger loop actions; loop gates and alerts flow back into chat
- **The `/future-loop` skill**: the driving manual for agents (v3.x, kept in sync with this doc)
- **State location**: `<cwd>/.future/loop/` (add to the project `.gitignore`)

## Further reading

- Install & build: [build-and-install.md](build-and-install.md)
- Evidence ledger: [long-run-evidence-ledger.md](long-run-evidence-ledger.md)
- TUI usage: [tui.md](tui.md)
- Skill source: [future-skills/builtin/future-loop](https://github.com/futuregene/future-skills/tree/main/builtin/future-loop)
