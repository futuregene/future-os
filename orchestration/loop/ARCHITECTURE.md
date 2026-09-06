# Loop architecture: durable kanban, reliable controls, evidence-driven agents

Operational commands: [control plane guide](../../docs/loop-control-plane.md).
Agent driving policy: `skills/builtin/future-loop/SKILL.md`; research methodology:
`skills/builtin/future-research/SKILL.md` (the skills submodule).

## Boundaries

The model chooses strategies, interprets scientific evidence, and asks people for
judgment. The kernel supplies deterministic state, dependencies, leases, evidence,
validation, and advisory signals. It does not force a replan because a heuristic
says a worker is stuck. More rules are not a substitute for giving the agent the
right evidence.

The event ledger is authoritative. Sessions are useful caches, not durable task
memory. All control capabilities are exposed through the CLI; the dashboard remains
strictly read-only. The human supervises the orchestrator's judgment. A non-LLM
watchdog supervises process liveness and notification transport, not reasoning.

## Floors, signals, and budgets

- **Floors:** legal task transitions, explicit completion intent, acceptance gaps,
  dependency constraints, leases, and validated terminal closure.
- **Signals:** unsuccessful attempts, outcome streaks, oscillation, missing artifact
  activity, monitor state, reported milestones. These describe observations, not
  mandatory strategy changes.
- **Bounds:** outer run iteration limits, validation attempt limits and independent
  validator wall time. Do not confuse `--max-turns` with a token/currency cap or
  a timeout for the agent's entire inner reasoning/tool loop.

Manual completion intentionally differs from machine verification: it records a
manual review (or explicit operator override), never a fabricated validator pass.
A delivery is initially pending. `delivery record` records the reviewer's judgment;
the kernel cannot establish the truth of a scientific claim by checking tokens or
file existence. Changes to acceptance standards need explicit justification.

## Task graph and ownership

Advancement tasks declare incoming prerequisites using `--blocks`; gates/blockers
can declare outgoing dependents. Both edge directions are honored by scheduling
and manual completion. An open scoped gate blocks its dependents, not unrelated
work. `--global-gate` is the explicit global freeze. Resolve decisions with
`gate resolve`, not `todo complete`.

`--owner` is a durable assignment; lease expiry does not remove it. Unowned todos
form an intentionally shared pool. Unique worker IDs are required for parallel
runs; sharing an ID defeats own-lease exclusion. Coordination todos belong to the
orchestrator and never enter a worker's advancement frontier. Workspace guards
remain mandatory unless a caller has established non-conflicting write sets.

Dependencies determine eligibility; priority only orders eligible offers. Workers
exchange knowledge through evidence and artifacts, not an additional negotiation
protocol. Fan-out, synthesis and a subsequent fan-out are expressed as ordinary
todos and dependency edges.

## Durable steering

New steering uses `ControlIssued` with a UUID, target (worker or broadcast), text
and interrupt flag. Routine guidance waits for a turn boundary; `--interrupt`
additionally aborts the in-flight session. Latest-wins is restricted to the SAME
scope. A target-specific instruction cannot erase another worker's instruction;
broadcast and target-specific guidance can both appear, in ledger order.

`ControlAcknowledged` is recipient-specific and written only after a completed
turn's durable run record. One recipient does not consume a broadcast for everyone.
A failed transport or interrupted turn leaves instructions pending. This is
**at-least-once guidance**, not exactly-once external side effects: a crash after
execution but before acknowledgment can replay an instruction. Guidance must be
idempotent; irreversible operations still need their own approval/idempotency.

The interrupt watcher does not advance over incomplete JSONL lines or failed
interrupt delivery. Legacy `WorkerSteered` / `SteerConsumed` events remain readable
for old ledgers, but new CLI instructions never use their single-slot projection.
Model/thinking configuration changes require an appropriately configured new
session, not a steering message.

## Detached execution and independent liveness

Production `run` re-executes into a detached child and checks for immediate startup
failure. `--detach` marks the internal foreground child;
`FUTURE_LOOP_NO_DETACH=1` is the foreground embedding/test escape hatch. The unified
`future` binary retains its `loop` dispatch prefix when re-executing.

A successful detached dispatch or supervisor registration also ensures an
independent `supervisor watch --goal G` process exists. An OS file lock allows
only one watcher per goal. Every two seconds it observes dead lease holders,
flags deliveries awaiting verification for five minutes, and services the outbox.
It remains alive when the last worker dies and does not require another paid run
or an LLM polling session to discover the failure. `scheduler tick` and post-run
sweeps remain supplemental reconciliation triggers.

The watcher exits when the goal is deleted/cancelled or terminal with no pending
notification. It is not an OS boot service: after host restart, launch the same
CLI under a service manager or re-register/resume the goal. No local process can
guarantee progress through host power loss without a host restart mechanism.

Lifecycle commands stop invalidated workers before removing their state. Late
completion cannot resurrect superseded tasks. This is distinct from interrupting
for new guidance, where the task remains resumable.

## Durable, batched supervisor outbox

1. Persist `SupervisorNote` before connecting to the agent, even with no supervisor.
   Deduplicate by episode key.
2. Prepare an immutable `SupervisorBatchPrepared`: target session, note keys,
   bounded message, UUID. A batch contains at most 32 notes and points to full
   ledger evidence. The watchdog coalesces notes arriving between ticks.
3. Push with `enqueue_if_busy`, never interrupting the supervisor. A retry uses
   exactly the same request key AND message, including after an ambiguous reply.
4. Record `SupervisorBatchDelivered` only after remote acceptance. This receipt
   means accepted by the agent queue, not that the supervisor acted on it.

The ledger, prepared batches, receipts and OS lock prevent concurrent flushers
from independently delivering the same pending notes. If transport fails, retain
and retry. A new registered supervisor can recover notes from the ledger. Treat
old notifications as prompts to reconcile CURRENT status, not unconditional orders
to restart a worker. Foreground embedders without a watchdog may flush immediately;
the same persistence and replay rules apply.

## Bounded validators

Validators run asynchronously with independent wall time (120 seconds default;
positive `FUTURE_LOOP_VALIDATOR_TIMEOUT_SECS` override). Both output pipes are
continuously drained; only bounded diagnostic tails are retained. Hung validators
or inherited pipes time out. Cancellation/timeout cleans up the subprocess tree
(Unix process groups; Windows process-tree termination).

Commands use `sh -c` on Unix and `cmd.exe /D /S /C` on Windows. Shell language is
therefore platform-specific; portable tasks should invoke a portable checker.
A failed command returns diagnostics for repair; a spawn/timeout failure is
inconclusive rather than a passed check. Receipt metadata must not hide manual
review behind a machine-pass label.

## Turn envelope and evidence handoff

Every turn includes the objective, todo, explicit acceptance/validator contract,
resolved decisions, upstream evidence, prior evidence, relevant failures, a small
recent-history window and advisory signals. Recent reported milestones are also
fed back as claims to verify, not proven results.

For fan-in, every resolved predecessor gets an index entry. Summary bytes are
shared fairly; an early verbose source cannot consume the entire budget. The
index is O(fan-in), while summaries remain bounded. Full evidence is available via
`status --format json`. Superseded sources are labeled, not treated as verified.
The orchestrator must name artifact paths in downstream task text; summaries are
not a substitute for reading reports and reproducing decisive measurements.

Separate **liveness**, **activity** and **progress**. `write/edit` execution starts
are artifact-activity proxies; shell calls are not automatically writes. Provider
input/execution phases are not double-counted. Actual progress requires evidence:
new artifacts, validation, measured improvements or falsified hypotheses. Neither
file churn nor uninterrupted thinking proves progress or lack of it.

## Proportional orchestration and honest stopping

Use light, standard or heavy workflows according to uncertainty/risk. A code
review does not require a fixed citation count or a worker swarm. Parallelize
meaningfully different methods only when the expected benefit exceeds overhead.
Confirm worker configuration and resource limits; do not silently override them.

At checkpoints, compare new evidence and remaining gap with expected next-step
cost. Stop as achieved, bounded infeasible, budget-limited, low-return, or blocked.
Only achieved work satisfies successful closure. Preserve partial artifacts and
acceptance gaps; ask before expanding budget. Proving all conceivable methods
impossible is not required to stop an open-ended investigation honestly.
