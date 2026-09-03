# Loop Architecture: a kanban tool, not a rule engine

This document states the design principles of the loop control plane. For the
operational model (concepts, commands, multi-agent), see
`docs/loop-control-plane.md`; for the agent-facing driving manual, see the
`future-loop` SKILL.md.

## Design principles

1. **Kanban, not a rule engine.** The kernel offers deterministic tools —
   todo state, verify gates, acceptance contracts, evidence, leases — and
   never decides "you're stuck, stop and replan" on the agent's behalf. It
   computes signals (outcome floor, oscillation, repair count, monitor stall,
   no-progress turns) and surfaces them as **advisories** in the turn
   envelope; what to do about them is a decision, and decisions do not live
   in the kernel.

2. **The agent is the orchestrator — with observability and control
   levers.** The decision-maker is the model, not the kernel. The kernel
   enforces only the **correctness floor** — the hard constraints that keep
   goal state legal (verify gates, acceptance contracts, terminal closure,
   leases) — and never judges whether an *exploratory* result is
   *right*. But "the agent decides" is only meaningful if the agent can
   *see* and *act*. So the loop hands the orchestrator levers, not just
   state:

   - **observe** — four faces, not just `tail`:
     - **behavior (the process)** — `worker tail` streams a worker's live
       turn log (a condensed tool/usage view): watch the worker's *hands*;
     - **artifacts (the results)** — read the files the evidence points at
       (the report, the data): this is how the orchestrator judges an
       *exploratory* result — not by tailing the process but by reading the
       *work*;
     - **the ledger (state & history)** — `status` / `diagnose` expose todo
       state, signals, gates, and leases on demand: the authoritative board;
     - **pushes (events)** — supervisor notifications deliver state
       transitions (see "Run lifecycle & orchestrator notification");
   - **steer** — `supervisor steer` interrupts/corrects a running worker,
     `todo update` reshapes the kanban mid-flight;
   - **stop** — `worker stop` halts a worker on the orchestrator's judgement;
   - **close** — manual `todo complete` deliberately does **not** re-run the
     machine `--verify` gate: for exploratory deliverables the orchestrator's
     reading of the artifact is the judgement, and the kernel does not
     second-guess it.

   Observability and steerability are therefore part of the architecture,
   not conveniences: they are what makes the orchestrator real.

   **Workers escalate; they never decide whether a human is needed.** A
   worker that hits something it can't resolve does not open a "user gate"
   or judge "this one is for a human" — it **escalates to the orchestrator**
   (a signal/message, not a freeze) and keeps its lane otherwise. *Whether*
   the question needs a human at all is the orchestrator's call: it may
   answer directly, reshape the todo, change strategy, or — only then —
   take it to the person. How the orchestrator reaches the person (and
   whether that freezes anything) is **not constrained by the loop** — it is
   orchestration-layer behavior, outside the kernel. So the kernel has no
   `blocked_by: human` concept and no worker-opened gate: only dependency
   edges (`--blocks`, work must wait) and a reliable worker→orchestrator
   escalation channel. The human sits at the top of the supervision stack
   (see "Supervision layers"), reached *through* the orchestrator, never
   addressed by a worker directly.

3. **The user drives through the skill.** Users do not script the kernel
   directly; they state a goal (`/future-loop <task>`) and the agent — guided
   by the `future-loop` SKILL.md — decomposes it, drives runs, reads signals,
   and escalates decision points. SKILL.md owns "what to do when, how to
   decompose, how to steer" (the orchestration layer); the CLI/kernel is the
   underlying mechanism (state + hard checks). Decision guidance lives in
   SKILL.md precisely because that is where the decision-maker reads.

4. **All capabilities are exposed through the CLI.** `future loop <cmd>` is
   the single machine interface to the control plane: every state change —
   goal/todo mutation, gate resolution, lease, steer, stop, completion — is
   a CLI invocation, deterministic and auditable in the event ledger. The
   skill drives the CLI on the agent's behalf; a human operator types the
   same commands; the dashboard (`ui`) is deliberately **read-only**
   (mutations stay in the CLI). One interface, no side doors: anything the
   loop can do, the CLI can express.

5. **State is durable; context is replayable.** Goals, todos, evidence, and
   signals persist outside the chat (event-sourced, replayable). The ledger
   (with evidence) is the **authority** on state and history; session
   continuity is a valuable **cache**, refreshed from the ledger when the
   world moved while the session was parked (see "Worker session lifecycle"
   below). Whether to resume a session at all is the caller's choice, not
   the kernel's.

## The two categories of kernel behavior

The kernel does exactly two kinds of things, distinguished by a single
question: **if it is violated, is the goal's state now illegal?**

- **Floors** — enforced; a violation puts the goal into an illegal state.
  The kernel's only coercive power.
- **Gauges** — computed and handed to the agent; *what to do about them is
  never the kernel's business*. Some inform strategy, some cut off budget,
  but all are information, not coercion.

### A. Floors (enforced — deterministic kanban semantics)

These are state-consistency hard constraints. Weakening them would put the
goal into an illegal state:

| Floor | Why it must stay |
|---|---|
| succession closure missing | completion must declare a successor / no-follow-up, or the goal can never close |
| acceptance gap | hard contract: the acceptance token must be satisfied |
| terminal judgement | deterministic kanban state: all todos done + gaps satisfied |
| blocker | a blocker |
| work leased to others | concurrency correctness |
| verify gate | correctness: exit 0 before complete |
| lease | concurrency mutual exclusion |

Machine verification has a deliberate complement: **orchestrator judgement,
recorded through the delivery closure**. A completion lands as `delivered`;
the orchestrator (or operator) resolves it as `verified / failed / rework`
by reading the artifact — the kernel records the judgement, never
second-guesses it (a manual `todo complete` does not re-run `--verify`).

### B. Gauges (information — never a forced replan)

Everything the kernel computes and surfaces, the agent reads and acts on —
or ignores. Both strategy hints and spend caps are kernel-computed
quantities handed over, differing only in how the agent uses them, not in
kind. None of them decides "you are stuck → replan".

*Soft gauges (strategy hints)* — surfaced as advisories in the turn
envelope, queryable via `status` / `diagnose`:

| Gauge | Detection | What the agent sees |
|---|---|---|
| outcome floor | `surface_streak >= threshold` | N consecutive turns without a material outcome |
| oscillation | A→V→A→V alternation | deliveries flip-flop accept/reject |
| repair budget | `failed_attempts > MAX` | failed todos stay runnable (no filtering); "failed N times" |
| monitor stall | `consecutive_no_change >= 3` | quiet wait + "consider expiring the watch lane" |
| no-progress | `no_progress_turns >= 2` | "consider restarting with a fresh session" |

*Hard gauges (spend caps)* — they bound consumption; when one trips, the
goal waits and the orchestrator (or user) decides what next — still never a
kernel-forced replan:

| Gauge | What it caps |
|---|---|
| validation budget | a `--verify` gate that keeps failing still bounds the run loop |
| quota | should-run decisions, scheduling refusals, spend |
| turn timeout | one bounded turn at a time |

The single distinction that matters: a floor says *this state is illegal*;
a gauge says *here is a number* — and the response to a number is always a
decision, which lives with the agent, not the kernel.

## Run lifecycle & orchestrator notification

How a run is launched, and how a worker reaches the orchestrator session,
are architectural decisions (mechanisms of the principles above), fixed here:

1. **Runs are detached (async).** The orchestrator never blocks on a run:
   it dispatches the run as an independent process, immediately regains
   control, and keeps watching other workers, reading signals, and answering
   gates. A synchronous run would demote the orchestrator to "just another
   worker" for the run's whole lifetime.

   **This is a caller-side contract, not a kernel mechanism.** `run` is a
   foreground CLI invocation; the kernel offers no server-side spawn or job
   handle. Detachment is achieved by how the orchestrator launches it
   (shell background / nohup / setsid / scheduler) and is enforced by
   discipline: **the orchestrating agent must never run `future loop run`
   synchronously and wait for a todo to complete** — while it blocks, no
   worker is watched, no gate is answered, no signal is read, and the goal's
   dead time is the orchestrator's fault (see the skill's drive playbook).
   The liveness path below (lease + pid + scheduler tick) exists precisely
   because detached runs cannot rely on a blocked caller noticing anything.

2. **The ledger is the authoritative state.** Every worker writeback lands
   in the event ledger — replayable, auditable, crash-safe. The ledger never
   loses "what happened", even when every other channel fails.

3. **Orchestrator awareness = push triggers + ledger pulls.** Because the
   orchestrator is an LLM session (it has polling, not interrupts), state
   *transitions* — completion, failure, a gate opening, a dead worker, a
   signal escalating after N unacknowledged turns — are pushed to the
   supervisor session as messages. Pushes are **volatile
   triggers, not the record**: they are idempotent (dedup-keyed on the
   transition, so re-sends are no-ops) and droppable (no supervisor
   registered or agent unreachable → dropped, the ledger remains
   authoritative). The orchestrator's ledger reads (`status`, `worker tail`,
   the next turn envelope) always converge on the truth, so a lost message
   costs latency, never correctness. Two push paths exist: the worker's own
   transition reports, and the scheduler's dead-holder sweep for workers
   that died too abruptly to report.

4. **Detached runs are supervised by lease + pid liveness, not by a parent
   process.** A synchronous run gets crash-supervision for free (the blocked
   caller notices when the run dies). Detaching removes that implicit
   supervision, so it is taken over formally: the scheduler's dead-holder
   check (`notify_dead_holders`) notices a lease whose holder pid is gone
   and pushes a relaunch prompt to the supervisor. Detach as default
   therefore depends on this liveness path being reliable — it is the
   primary supervision, not a fallback.

### Supervision layers: the human supervises the orchestrator

Supervision is a stack, and it tops out at a person:

- **the orchestrator (supervisor) watches workers** — via the lease + pid
  liveness above, plus `worker tail` for live inspection;
- **the human watches the orchestrator.** The orchestrator is the top of the
  automated supervision chain; nothing in the loop supervises it. When it
  stalls, goes wrong, or decides a question needs a person, the human is
  the escalation target. Workers reach the person only *through* the
  orchestrator (they escalate, never address the human directly); how the
  orchestrator then involves the person — and whether that freezes any work
  — is orchestration-layer behavior the kernel does not constrain. The
  person may also step in directly at any time (`todo update`, `worker
  stop`, manual `todo complete`): automation below, a person at the top.

## The kanban's structure: todos, dependencies, workers

Three relations define how multi-worker work is laid out on the board, and
— crucially — how information flows without any worker-to-worker messaging.

**Todo↔todo: the dependency DAG is the board's skeleton.** `--blocks` edges
are the *only* ordering mechanism — there is no global priority queue and
no worker-level sequencing, only edges between todos. A fan-out is one todo
blocking several downstream todos; a fan-in (synthesis) is one todo blocked
by several upstream ones. The graph says *what must precede what*, nothing
more.

**Todo↔worker: weak, runtime-brokered binding.** A todo is not owned by any
worker — it sits on the board for anyone to claim. Who actually does which
todo is brokered at runtime in two steps: the orchestrator's intent when it
spawns a worker (it knows which model should probe which direction), and
the worker's claim when it matches (specialization, free lease). The
relation is many-to-many and resolved dynamically; a **lease** is the
"currently held" snapshot of it, giving mutual exclusion and liveness, not
assignment. The kernel does not assign todos to workers — the orchestrator
shapes the board, workers claim from it.

**Worker↔worker: no direct messages — the board is the shared state.**
Workers never talk to each other. Information moves only through three
carriers, all mediated by the ledger: **evidence** (the durable declaration
of what landed), the **artifact files** it points at (the report, the
data), and the **turn envelope's context layer** (recomputed from the
ledger into the next turn). A worker that "looks at upstream results and
summarizes" is not receiving messages — that *is* its todo: it is ordered
after the upstream todos by `--blocks`, its envelope injects their evidence
and artifact paths, and it reads those artifacts.

**Fan-out → synthesize → fan-out, in these terms.** Spawn several workers
on different models along different directions (parallel todos, no mutual
edges); a synthesis todo `--blocks` them all, so a downstream worker reads
their artifacts and summarizes; a second wave of todos `--blocks` the
synthesis. Grouping, model choice, direction assignment, and round
progression are all **orchestration-layer** decisions (the orchestrator
shapes the todo texts, `--blocks` wiring, and spawn configurations); the
board only guarantees order (edges) and mutual exclusion (leases), and does
not model *which artifact flows into which todo* — that wiring the
orchestrator writes into the todo text and acceptance contract.

## Steering vs. reconfiguring a running worker

The orchestrator can change two different things about a worker mid-goal,
and they take different mechanisms because they differ in whether the
session survives:

- **Change *what* it does (instruction / objective) → steer.** A
  `supervisor steer` records a `WorkerSteered` event (latest wins) and the
  worker's steer-watch aborts the current turn so the next turn drains the
  instruction into its envelope. This is the *interrupt* form — not a silent
  note-append: the in-flight reasoning is abandoned and the worker resumes
  under the new instruction. The session (its accumulated context) survives.

- **Change *what it runs on* (model / thinking level) → retire + respawn.**
  Model and thinking level are properties of a session, fixed at spawn;
  they are not hot-updatable through steer. Changing them means the
  worker's *configuration* changed, and configuration is identity: retire
  the session and spawn a fresh one on the new configuration, cold-starting
  its context from the ledger (this is exactly what "context limit /
  direction change" retirement already does). So reconfiguration is not a
  third channel — it is the ordinary retire-and-respawn transition applied
  to a config change.

The rule of thumb: **steer changes the task; respawn changes the worker.**
Both are orchestrator decisions; the kernel only records the events.

## Worker session lifecycle

A worker session is a **first-class lifecycle object**, not an appendage of
a run: the orchestrator spawns it, attaches work to it, parks it, resumes
it, and retires it. Resume-vs-fresh is one transition inside this
lifecycle, not the whole story.

### States and transitions

```
   spawn ──► ACTIVE (on duty, holds a lease, executing turns)
                │   ▲
     interrupt  │   │ resume (InfraRecoverable → back to the
                ▼   │   pre-interruption state)
           INTERRUPTED (FailureKind recorded)
                │
                ├── InfraRecoverable → resume
                ├── ContextCorrupted → RETIRE + spawn fresh
                └── HardError        → RETIRE + spawn fresh

   ACTIVE ──park──► PARKED (no matching work / cost / quota; context sealed)
   ACTIVE ◄─resume + delta── PARKED

   any state ──► RETIRE: goal done / direction changed (mass supersede) /
                context limit / explicit fresh. Retired ≠ deleted — the
                ledger keeps everything.
```

A session in **either** ACTIVE or PARKED can be interrupted (a parked
session cannot hit a 429, but its host can die) — INTERRUPTED records the
interruption, and an `InfraRecoverable` resume returns to the state the
session was in.

**When to park**:

- no runnable todo matches this worker's specialization (model, thinking
  level, accumulated todo context) — don't let it spin polling;
- cost control: keeping a session alive while waiting on a monitor/gate is
  not worth it;
- quota pressure: yield session capacity to a higher-priority goal.

**What a resumed session needs** — a parked session's *reasoning chain* (why
it chose an approach, what it tried that failed) is its real value and is
kept as-is; but the *world* changed while it slept, and resuming on a stale
world model is the biggest resume pitfall. The refresh is just the ordinary
turn envelope, recomputed from the ledger on the resume turn (see "The turn
envelope" below): because the envelope's context layer is always derived
live from the ledger, a resumed session automatically sees the world as it
is now — new todos, fresh evidence, current gauges, gate verdicts.

**When fresh is mandatory (RETIRE + spawn, never resume)**:

- `ContextCorrupted`: the verify gate rejected the output — the reasoning
  chain is polluted and would carry a false premise forward;
- direction change: after a mass supersede / replan, the old context is
  residue of dead routes;
- context limit: retire *before* hitting the token ceiling, with a handoff —
  the outgoing worker writes "what I learned, where the pits are" into the
  ledger (this is exactly why evidence is enforced non-empty), and the fresh
  session cold-starts from the ledger.

### FailureKind: classifying the interruption

`FailureKind` classifies an interruption to decide the INTERRUPTED →
(resume | RETIRE) branch:

- `InfraRecoverable` — the accident was *outside* (429 / rate-limit /
  connection reset / agent crash / stream gap); the reasoning state is
  intact: **resume**.
- `ContextCorrupted` — the accident was *in the reasoning*: the verify gate
  rejected the output, so the reasoning state is polluted: **fresh**.
- `HardError` — the turn errored without a recoverable infra cause: **fresh**.

The kernel only provides this classification (observation data); the caller
decides resume-vs-fresh explicitly. **Fresh is the default — there is no
`--session-policy` flag.** The only resume path is an explicit pin
(`--resume-session <id>`), because the goal-level retention holds a single id
and is ambiguous with parallel workers. The kernel stays a pure tool: it
provides state and signals, but never makes the decision.

Two scoping rules keep the lifecycle simple: parking happens at **turn
boundaries** (no preemptive mid-turn suspension or checkpoints), and a
session is bound to **one goal** (no cross-goal reuse — context
contamination outweighs the savings).

**How the states map to the kernel's actual surface.** The lifecycle above is
the model; the kernel implements it with fewer moving parts, and the mapping
matters when reasoning about behavior:

- **ACTIVE / INTERRUPTED** are explicit: a run is a live process holding
  leases; the writeback stamps a `FailureKind` per turn and `cmd_run` records
  a `SessionRetention` (id + classification + resumable advisory) when the
  run exits.
- **PARKED has no dedicated state.** A quiet-wait exit (monitor not due,
  blocker with no fallback, work leased to others, gated with no fallback)
  retains the session and stops — the *effect* of parking, derivable from
  the goal's frontier. Resuming "with delta" is just the ordinary turn
  envelope recomputed from the ledger on the next `run`.
- **RETIRE has no dedicated state either.** `worker stop --delete` and a
  non-resumable retention (`HardError` / `ScienceVerifyFailed`) are the
  retire transition: the next run cold-starts fresh from the ledger. A
  context-limit hit classifies as `HardError`, so it retires by the same
  rule — there is no separate pre-emptive "retire before the ceiling with a
  handoff todo" mechanism; write the handoff into evidence before the
  context runs out.
- **spawn** is `run`'s session creation; there is no orchestrator-side
  session registry beyond the ledger's `worker_session_bound` events.

## The turn envelope: what the orchestrator injects into a worker

The turn envelope is the single information interface between the
orchestrator/kernel and a worker — the per-turn prompt the worker executes.
It carries **two layers**, and deliberately not a third:

- **Instruction layer (every turn)** — the TODO text and the completion
  contract ("report what you did; declare the successor or
  `--no-follow-up`"). Without these the worker has neither a task nor a
  definition of done.
- **Context layer (recomputed from the ledger every turn)** — the goal and
  objective, the previous turn's evidence, this todo's failure history
  (classified), recent semantic history, and resolved gate verdicts. This
  is what stops the worker re-doing work and re-stepping into known pits —
  it is where "durable artifacts, not session memory" actually lands.

**Not in the envelope: the kernel's scheduling internals.** The should-run
verdict, mode, and arbitration disposition are the kernel's *own* decision
state, addressed to the orchestrator and the operator — not to the worker.
Putting them in the worker's prompt would leak the scheduler's hesitation into
the executor (a worker's job is how to do the work, not whether the kernel
thought it should run) and blur the observe/decide split. The envelope tells
the worker *what to do and the context to do it well*; it does not tell the
worker what the kernel was thinking.

**In the envelope: the observable signals.** Signals (outcome floor,
oscillation, failure count, no-progress) are a different kind of quantity:
observations about the *work*, recomputed from the ledger — not the kernel's
hesitation. Principle 1 promises them as advisories "in the turn envelope",
and that is where they live: the envelope's context layer carries a `signals`
block recomputed by the same kernel detectors that render the delivery
reason's advisories (one detector set, two consumers — the orchestrator reads
them in the packet reason, the worker reads them in the envelope). What to do
about a signal is, as everywhere, a decision — never a kernel directive.

**One envelope, no special cases.** A first turn, a resumed turn, and an
ordinary turn all use the same envelope; the differences fall out of
whatever the ledger currently holds. A first turn's envelope is naturally
short (no failure history, no previous evidence); a resumed turn's envelope
naturally reads as "the world since you parked" — because the context layer
is always computed live from the ledger.

## Trust & authorization boundary

A worker runs **with the user's full trust domain**: it executes arbitrary
shell (a `--verify` gate is a command), writes its workspace, and a steer
message injects instructions into it. There is no sandboxing layer in the
loop itself; containment is the **workspace boundary** (a worker stays in
its workspace unless `--force-workspace` says otherwise). When a worker
reaches something at or beyond that edge — irreversible, expensive,
credential-bound — it does not decide "this needs a human"; it escalates to
the orchestrator, which decides whether to proceed, reroute, or involve the
person. Autonomy inside the trust domain, the orchestrator as the gate at
its edge.
