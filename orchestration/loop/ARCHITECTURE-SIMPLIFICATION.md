# Loop Architecture Simplification: a kanban tool, not a rule engine

> Principle: **the decision-maker is the model (the agent), not the loop
> kernel.** The loop kernel should be a **kanban tool** — offering the
> deterministic tools (todo state, verify gates, acceptance contracts,
> evidence, leases) — not a **rule engine** that decides "you're stuck, stop
> and replan" on the agent's behalf. Agent decision guidance lives in the
> `future-loop` SKILL.md, which the agent reads and acts on itself.

## I. Feature classification

### A. Rule-engine features (de-emphasized: "forced replan" → "signal + keep delivering")

These were hard-coded kernel rules that decided "you're stuck" and forced a
replan — the kernel making the agent's decisions for it, against the
"the model decides" principle. Each is now an observation the agent reads,
never a replan the kernel forces:

| Rule | Former trigger | Now |
|---|---|---|
| outcome floor | `surface_streak >= threshold` | record a signal; surface as an advisory in the delivery reason |
| oscillation | A→V→A→V alternation | record a signal; surface as an advisory in the delivery reason |
| repair budget | `failed_attempts > MAX` | failed todos stay runnable (no filtering); advisory notes "failed N times" |
| monitor stall | `consecutive_no_change >= 3` | quiet wait + advisory ("consider watch-lane expiry") |
| LLM zombie | `no_progress_turns >= 2` | advisory ("consider restarting with a fresh session") |

### B. Correctness floors (kept — deterministic kanban semantics, not "deciding for the agent")

These are state-consistency hard constraints. Weakening them would put the
goal into an illegal state:

| Floor | Why it must stay |
|---|---|
| succession closure missing | completion must declare a successor / no-follow-up, or the goal can never close |
| acceptance gap | hard contract: the acceptance token must be satisfied |
| terminal judgement | deterministic kanban state: all todos done + gaps satisfied |
| user gate | a user gate freezes work |
| blocker | a blocker |
| work leased to others | concurrency correctness |
| verify gate | correctness: exit 0 before complete |
| lease | concurrency mutual exclusion |
| validation budget | a `--verify` gate that keeps failing still bounds the run loop — a correctness floor, not a policy rule |

### C. Decision guidance moved into SKILL.md

The kernel no longer decides for the agent, but SKILL.md teaches the agent
to **read the signals and decide for itself**:

- see `surface_streak >= N` → consider changing strategy or superseding
- see the oscillation signal → consider a different validator or splitting the todo
- see `failed_attempts > 1` → consider superseding or asking the operator
- see a monitor with no change → consider watch-lane expiry or writing a blocker

## II. What changed

1. `decision/mod.rs` — removed the rule-engine replan branches (outcome floor /
   oscillation / repair budget / LLM zombie / monitor stall); the delivery
   reason now carries the signals as **advisories**.
2. `decision/stall.rs` — detectors kept as observation data (signal sources),
   no longer used to force a replan.
3. `decision/oscillation.rs` — same.
4. `console.rs` — the run loop no longer breaks on repair-budget exhaustion
   (only the validation-budget break remains).
5. `state.rs` + `console.rs` — **session retention**, the same principle applied
   to resume-vs-fresh: the kernel records *why* a session was interrupted
   (`SessionRetention` with a `FailureKind`) and keeps the session id on disk;
   the caller decides resume-vs-fresh via
   `run --session-policy auto|fresh|resume` / `--resume-session ID`.
6. SKILL.md — new "agent decision guidance" + "session retention" sections.
7. Signal exposure — the signals stay visible in the delivery reason (the
   agent sees them directly in the turn envelope) and are queryable.

## III. Signals are retained (not deleted — repurposed)

`outcome_floor_breach` / `oscillation_replan_reason` / `repair_exhausted` /
`is_monitor_stalled` remain as detection functions. They went from
"forced-replan triggers" to "agent-readable observations", exposed two ways:

1. as advisories in the delivery reason (the agent sees them directly in the
   turn envelope);
2. as queryable state (e.g. `status` / `diagnose`) the agent reads on demand.

## IV. Session retention (the same principle extended)

Resume-vs-fresh is also "the caller decides": the kernel only provides
**observation data** — *why* the session was interrupted — and never decides.
`FailureKind` classifies the interruption:

- `InfraRecoverable` — LLM state intact (429 / rate-limit / connection reset /
  agent crash / stream gap): **resumable**.
- `ScienceVerifyFailed` — the verify gate rejected the output; the reasoning
  state is broken: **fresh**.
- `HardError` — the turn errored without a recoverable infra cause: **fresh**.

The caller decides explicitly via `--session-policy` / `--resume-session`;
the default `auto` resumes only sessions the kernel judged resumable
(`InfraRecoverable`). The kernel stays a pure tool: it provides state and
signals, but never makes the decision.
