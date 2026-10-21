# Agent Startup Arbitration and Self-Healing

> [中文](agent-supervision.zh-CN.md)
>
> Design date: 2026-10-21. Source baseline: `a91fcf0ec`.
> Status: **P1 has landed** (supervisor, `Busy` yielding, bounded waiting, the breaker, all three entries wired, and
> Agent-triggered continuation). This document separates what landed from the product decisions that still need
> the user's sign-off — see [§2](#2-approved-design-vs-product-decisions); open items are in
> [§12](#12-open-questions) and the measurements are in [§13](#13-measured-behaviour).

Three entry points can bring an Agent up today: `future agent`, `future host`, and `future tui`. All three honour the same instance lock, but **who starts it, who owns it, and what happens when it dies** each has its own answer. That layer is what is missing, and it is what this document adds. It changes neither the instance lock, the storage format, nor the RPC contract.

## 1 Current state

### 1.1 What each entry point owns

| Entry | Runs the Agent in-process | Launches an external Agent | Locks held |
|---|---|---|---|
| `future agent` | Itself | — | `~/.future/agent/agent-instance.lock` (service lock) + the socket's `.owner.lock` (endpoint lease) |
| `future host` | Yes, when it gets the service lock | No | `~/.future/app/desktop.lock` (host lock) + the service lock, conditionally |
| `future tui` | No | Yes: a `future agent --grpc-addr auto` child | Neither |

All three funnel through [`select_agent`](../../packages/rpc/src/connection.rs): handshake an external service first; only on failure plus a confirmed-absent local endpoint (`NotFound`/`ConnectionRefused`) plus a passing [`check_instance_lock`](../../packages/rpc/src/connection.rs) does it issue a `LocalStartPermit`, and the real arbitration is the `try_write()` inside [`AgentService::with_instance`](../../agent/src/service/mod.rs).

### 1.2 Four gaps

1. **Launch happens once.** The TUI calls [`ensure_agent_running`](../../tui/src/agent_supervisor.rs) at the top of `run` and never again; what remains is the [reconnect prober in grpc_client](../../tui/src/rpc/grpc_client.rs) (1s→30s backoff, "Connection to agent lost" in the UI), which **never re-spawns**. Desktop is the same: one call in setup.
2. **headless cannot see an external Agent die.** Its loop calls [`check_running`](../../cli/src/headless/agent.rs) every second, but that only reads **its own host's `Phase::Failed`**. While attached it is `Ready` forever, so a dead external Agent leaves headless serving and every RPC failing.
3. **Losing the lock race is treated as failure.** `InstanceFile::acquire` turns the conflict into a string ([instance.rs](../../agent/src/service/instance.rs)), `AgentHost` records `Phase::Failed` (terminal there: keep the lock when cleanup is uncertain, no in-process restart), and headless exits while the TUI reports `exited with status 1`. **The correct behaviour is to yield and wait.**
4. **The TUI's sidecar dies with the TUI.** `OwnedAgent::drop` kills it ([agent_supervisor.rs](../../tui/src/agent_supervisor.rs)), so headless availability hangs on "some terminal window is still open".

## 2 Approved design vs product decisions

| Id | Content | Nature | Status |
|---|---|---|---|
| D1 | Keep the OS instance lock as the arbiter; add no leader lease | Design | **Approved** |
| D2 | A client that loses the race yields and waits instead of failing | Design | **Approved** |
| D3 | One supervisor per client: probe → confirm absent → take the lock → start or yield | Design | **Approved** |
| D4 | Waiting is bounded and visible (see [§5](#5-waiting-is-bounded)) | Design requirement | **Approved** |
| D5 | A crash-loop breaker (see [§6](#6-the-breaker)) | Design | **Approved** |
| **P1** | Supervisor core + `Busy` classification + wiring the three entries ([§10](#10-implementation-checklist)) | Implementation | **Landed** |
| **D6** | TUI switches to "external first, otherwise in-process" and drops the child sidecar ([§7](#7-tui-starts-the-agent-in-process)) | **Product decision** | **Awaiting the user** |
| **P2** | Let the Agent outlive the TUI: detached sidecar + `--ephemeral` idle exit ([§8](#8-lifetime-and-ownership)) | **Product decision** | **Awaiting the user** |
| **D7** | After the **user** stops the Agent, should other clients restart it? ([§4.3](#43-four-kinds-of-death)) | Product decision | Recommendation: no; **awaiting confirmation** |
| **D8** | Post-restart continuation: who runs it, and when is a run resumable? (§9.1, §9.2) | Design | **Partly landed**: the continuation criterion is implemented; the `cause` distinction is not (see §12) |
| **D9** | The trigger lives in the Agent too (proposed by the user, 2026-10-21) | Product decision | **Landed**: the Agent triggers it after starting; the switch is `resumeInterruptedOnStart` in `~/.future/agent/settings.json` (**on by default**, matching the desktop's `autoResumeInterrupted`); the desktop's one-shot pass is retired |
| **D10** | Continuation sends no user message; it continues inside the loop via a request-only directive (§9.3, proposed by the user, 2026-10-21) | Product decision → design | **Landed** in three steps: the accepting path + directive, the placeholder tool result, then the relaxed assistant tail |

The two entries marked "awaiting the user" are **not settled**. This document does not design code paths for them; it states the options, their costs, and what they affect.

An entry marked "landed" has code and tests behind it; "approved" is still only a plan. D8's cause distinction is not implemented.

## 3 Inputs to a decision

Only three, and no fourth kind of state:

1. **The endpoint**: a readiness probe of the local socket/pipe, yielding `agent_instance_id`.
2. **The instance lock**: `agent-instance.lock` is an OS advisory lock; the kernel releases it when the holder is killed, so **there is no stale-pid problem**. The whole design rests on this.
3. **The stopping signal**: an RPC returning `agent_stopping`, or a shutdown reason. "Shutting down" must never be read as "absent".

Corollary: **lock held but endpoint not ready yet = someone else is starting**. Neither starting (a second instance) nor reporting an error (a false failure) is acceptable.

## 4 The state machine

```
            ┌──────────── readiness probe (with instance_id) ─────────┐
            │                                                         │
   Ready(compatible) ───────────────► Attached / Owned (healthy)
            │  consecutive failures / stream lost / heartbeat timeout / agent_stopping
            ▼
          Lost ──┬── Absent (unreachable + lock free + endpoint lease reclaimable)
                 │        └─► Candidate ─┬─ acquire won ─► Launching ──ready──► Owned
                 │                       └─ InstanceBusy ─► WaitingForOwner
                 ├── Busy (lock held, endpoint not ready) ──────► WaitingForOwner
                 └── Blocked ─────────────────────────────────► NoLaunch (report only, never start)
```

- Jitter before `Candidate`: `min(30s, 1s·2ⁿ) × U(0.8, 1.2)` — the same order as the TUI's existing reconnect backoff, so two clients do not take the lock in the same millisecond.
- `Blocked` covers: version/protocol/storage-contract mismatch, `agent_stopping`, an unreachable explicit `--grpc-addr`, and `FUTURE_DESKTOP_AGENT_MODE=external`. None of these **ever** triggers a launch (existing semantics, unchanged).

### 4.1 Two classifications to add

- `agent/src/service/instance.rs`: promote the lock conflict to a typed `InstanceBusy`, instead of relying on message text.
- `host/src/agent_host.rs`: add `Phase::Busy`, which **must not fall into `Phase::Failed`**. `Failed` means "cleanup is uncertain, the lock stays, no in-process restart"; `Busy` only means "let the other one run" and must be recoverable via `detach()` (`Ready && !owned` → `Stopped`) followed by a new `start()`.

### 4.2 Who may start one

| Entry | May embed | May spawn a child | Self-heals |
|---|---|---|---|
| `future host` | Yes | No (it can embed; a second process buys nothing) | Yes |
| Desktop GUI | Yes | No | Yes |
| `future tui` | See D6 | Today yes; no once D6 is adopted | Yes |
| channels / goal / `future run` / mobile | No | No | Attach only |

For production, keep `future host` under systemd / launchd / NSSM: a process manager does not die with itself, so client self-healing is only the fallback.

### 4.3 Four kinds of death

| Observed | Verdict | Action |
|---|---|---|
| Endpoint gone + lock free (`kill -9`, crash, OOM) | Abnormal exit | Enter `Candidate`, restart with jitter |
| `agent_stopping` / shutdown reason = host exit | Normal exit, but the service is still wanted | Restart |
| Shutdown reason = user stop / upgrade | Deliberate | **Do not restart automatically** (D7); say "stopped — rerun the command or press retry" |
| Version mismatch / explicit endpoint unreachable / external mode | Not a death | `NoLaunch`, report only |

## 5 Waiting is bounded

**Hard rule: no client may wait silently and indefinitely.** Every entry point needs all three of: a cap, a sentence that changes as the wait progresses, and a way out when the cap is reached.

| Entry | Probe cadence | Cap | What the UI/terminal shows meanwhile | Effect on its own runs | After the cap |
|---|---|---|---|---|---|
| `future tui` | 250ms, widening to 1s | **180s** (matches `AgentHost::START_TIMEOUT`) | Status line: `waiting for the Agent started by another client (pid 1234 @ host-a)… 12s`; past 30s, add "a cold data root can take minutes; progress: `<log path>`" | Input stays editable; a send is refused with the same sentence — **never queued, never faked** | The UI stays usable and switches to a background probe every 30s, with the status line permanently visible |
| `future host` | 250ms→1s | **180s** | One stderr line every 30s, same content | Accepts nothing (it never came up) | **Exit with an error** (exit 1), printing the holder's pid/host and the log path, and let the process manager retry |
| Desktop GUI | 250ms→1s | **180s** | The Agent status page shows "waiting for the Agent started by another client", with pid/host | Agent-dependent pages unavailable | Enter `Failed` with a Retry button; never start a second instance |

Three supporting rules:

1. **The cap only bounds "waiting for someone else";** your own launch keeps `AgentHost`'s existing 180s. Do not stack them into 360s.
2. **Each fresh "absent" observation resets the clock**, so "the other launch failed and released the lock" moves straight into `Candidate` instead of burning the full 180s.
3. **The wait must never degrade into silent retries.** Every change (pid obtained, instance id obtained, the other party exiting, the lock released) must show up in that status line.

`pid` / `host` need no new file: `agent-instance.json` already carries `pid`, `host`, `executable` and `futureHome` (plus `startTimeFiletime` on Windows), and `lock_conflict_message` already reads it.

## 6 The breaker

Count only launches **this process started itself** and that **ended before Ready** (failures of someone else's launch do not count, so two clients do not blame each other):

| Parameter | Value |
|---|---|
| Counting window | **300s** (sliding) |
| Threshold | **3** within the window |
| Cooldown once open | **60s**, no automatic launches |
| Half-open | **Exactly one** attempt after the cooldown |
| Half-open fails again | Cooldown doubles: 120s → 240s → 480s → cap **600s** |
| Half-open succeeds | Counter resets to normal |
| Explicit user action | One attempt immediately (rerun the command, press `r` in the TUI, the Desktop Retry button); the counter is not reset |

Opening the breaker **must** print, once: the number of attempts, a summary of each exit reason, the log path, and "this is protection, not surrender". A breaker does not block attaching: if another client brings the Agent up meanwhile, a successful endpoint probe moves it to `Attached`.

A process restart clears the counter (it lives in memory). Two clients therefore have separate budgets — at worst six attempts in five minutes, already spread by the jitter.

**Optional and non-authoritative**: `<home>/agent/agent-launch.lock` lets only one client be "starting and waiting for ready" at a time, with the rest waiting on it. It is not the arbiter (that is always the instance lock); it only avoids doomed initialisation attempts. Add it only if measurement shows churn.

## 7 TUI starts the Agent in-process

> **Awaiting the user (D6).** This section describes the option and its costs.

### 7.1 The proposal

The TUI adopts headless's policy exactly: attach when an external Agent is reachable; when it is absent and the lock is free, run `AgentService` **in this process**; when the lock is held but the endpoint is not ready, wait ([§5](#5-waiting-is-bounded)). The child sidecar and `check_sidecar_program` go away.

### 7.2 Feasibility

- The dependency direction works: `tui → host → agent`, no cycle (`cli → tui` unchanged).
- `AgentHost` is a process-global plus `configure_driver`, i.e. exactly the "each entry configures its driver once" shape the TUI already has; the wiring mirrors [headless's `drive`](../../cli/src/headless/agent.rs).
- No new decision logic: `select_agent` already covers external-first.

### 7.3 Hard constraint

**Only the unified `future` binary can embed.** Agent code re-executes `current_exe()` as its Linux sandbox helper ([runner.rs](../../agent/src/sandbox/linux/runner.rs)), and the `--linux-sandbox-helper` dispatch lives in [cli/src/main.rs](../../cli/src/main.rs); shell tools also depend on the current executable's directory being on PATH. Embedding in the standalone `future-tui` binary would break both the Linux sandbox and the CLI self-invocation.

⇒ The standalone `future-tui` must stay **attach-only**, with the reason stated (the same shape as today's `check_sidecar_program` message).

### 7.4 Costs

| Cost | Detail | Handling |
|---|---|---|
| Lifetime coupling | A sidecar crash leaves the TUI alive; embedded, an `abort`/segfault takes the TUI with it (`catch_unwind` only catches Rust panics) | Accepted; the TUI's crash hook already restores the terminal |
| Exit must be bounded | Embedded means waiting on `stop_until` (10s) at exit, and a failed stop is terminal | Leave raw mode / restore the terminal first, then stop within the bound; exit with the error rather than pretending to be healthy |
| **stderr corrupts the screen** | `environment::report` is `eprintln!` ([environment.rs](../../host/src/environment.rs)) | Add a report sink: the TUI status area plus the log file, with file logging enabled; a prerequisite of this option |
| Cold start must be visible | The TUI currently fails after a 5s probe; embedded uses the 180s budget | Bring the UI up first, say "Agent is starting", add the log hint after 30s |
| Larger footprint | The TUI was the lightest entry point | Provide an escape hatch: `FUTURE_TUI_AGENT=attach-only` (or `--attach-only`) |
| One-shot `-p` mode | Adds a full cold start to every call | See [§12](#12-open-questions) |

### 7.5 Gains

- The three entries converge on one policy, and one implementation stops diverging from `select_agent`.
- The TUI no longer has "the lifetime of a child process" to reason about (P2 becomes unnecessary for the TUI).
- The ownership inversion — "headless's Agent actually lives in some terminal window" — disappears.

## 8 Lifetime and ownership

> **Awaiting the user (P2).** The user's stated attitude to the desktop app is "close it and it's closed", and this item would let the Agent outlive the TUI, so it **must not be presented as settled**.

The problem: once D6 is adopted, an in-process Agent still dies with the TUI. If headless is attached at that moment, its next supervision pass will **restart** the Agent — trading "in-flight runs interrupted" for "the service stays up".

Two options:

| Option | How | Cost |
|---|---|---|
| A (current semantics, natural after D6) | The Agent dies with its starter; other clients restart it per [§4.3](#43-four-kinds-of-death) | In-flight runs are interrupted; but the semantics are simple and no process is left behind |
| B (P2) | Decouple the sidecar from its parent (Unix `process_group(0)`; Windows new process group + `CREATE_NO_WINDOW`), stop killing on `Drop`, and pass an internal `--ephemeral`: when **no active subscription, no running/queued run, no pending approval** holds for `IDLE_GRACE` (120s), it shuts down normally | One process may be left behind; Agent needs an "active subscription count"; the user's "closing it should close it" expectation changes |

Under either option, **the idle decision must be made by the Agent from real state**, never by a client asking "is anyone else there?" on the way out — that window is racy.

## 9 Data safety and client consistency

- A new owner touches the database only after taking the lock in `with_instance`; an old process holds the lock for as long as it lives (through stopping, until the last writer finishes), so **concurrent database access is impossible**. A SIGKILLed process recovers from the WAL and its unfinished runs converge to interrupted.
- A new instance means a new `agent_instance_id`: clients must drop pinned channels, stream cursors and in-memory queue assumptions, and handshake again. The RPC layer already states that a verified channel must never transparently reconnect to a replacement ([transport.rs](../../packages/rpc/src/transport.rs)).
- `agent_stopping` during shutdown must be read as "wait and see", never as `Absent` (the existing `Blocked` branch already does this; the supervisor keeps it).

### 9.1 What happens to the conversation after a restart

[#1277](../../agent/src/rpc/resume.rs) (the implementation of its day, since taken over by the Agent) supplies the right foundation, but its boundaries decide whether "a restart is just jitter" actually holds. This design must depend on it explicitly, or self-healing turns "the conversation continues" into "the conversation stops half way".

#### What it actually does

`resume_interrupted_runs_at_launch` does not resume the stream:

1. It settles the interrupted row as `cancelled` + `error_type=interrupted`.
2. It submits a **new run** whose prompt is `继续上一个任务。` plus "the previous run was interrupted when FutureOS closed (tools may have partly executed)" plus the last six execution-summary lines.
3. The model redoes that stretch with the full transcript as context.

True "uninterrupted streaming" is the other path: while **the Agent is still alive**, a reconnecting client's collector resumes from `after_idx = last_idx`; once the Agent restarts, attach returns `RunGone` and recovery can only come from the journal.

So the accurate statement is: **the conversation does not end and history is not lost, but the current turn is redone**. What the user sees is a visible disconnect → restart → a new run.

#### The four conditions that make it work

| Condition | Today | What this design must do |
|---|---|---|
| Someone runs the continuation pass in the restarted session | Historically only Desktop did; headless ran only `reconcile_interrupted_runs` (settle/reattach, no prompt), and the TUI had nothing | **Landed**: the Agent triggers it itself once Ready, so no host needs to be present; the desktop pass is retired so one conversation cannot receive two continuations (see §9.2) |
| The interruption must leave a resumable state | A graceful stop goes through `cancel_all` → terminal marker → `interruptedRun` null → `plan_resume` says `Terminal` → **no resume**; only an abnormal death (`kill -9` / crash / OOM) leaves `interrupted` | On a graceful exit, mark the run by its cause: `agent_shutdown` → resumable, `user_cancelled` → not (see premise 1 below) |
| The conversation must have a thread + run row in `app.db` | Run rows come from a host-submitted prompt or from import (discovery every 60s) | A box running only a TUI, with no host, has no candidate at all; the first self-healed launch should import once |
| The remaining product gates | Newest run, not `waiting_approval`, within 7 days, `autoResumeInterrupted` (on by default — it spends the user's credits) | Unchanged |

The second row is the easiest thing to break: today's TUI `SIGKILL`s its own sidecar (`OwnedAgent::drop`), which **skips the graceful stop**, so the journal keeps an unterminated run and the conversation happens to be resumable. Once D6 moves the Agent in-process, a normal quit takes the graceful path, the run becomes `cancelled`, and it is **no longer resumable**.

#### The D8 criterion (corrected)

The first draft used **who is still watching** (the active-subscription count): another client subscribed meant interrupted, being alone meant cancelled. **That criterion is wrong**, for two reasons:

1. It fails in exactly the case the user wants (the TUI dies first, then the alone Agent is stopped — so nothing ever resumes);
2. It infers the user's intent from "who is connected", and a connected client does not mean the task should continue.

Use the cause instead ([§9.2](#92-where-it-belongs-the-tui-a-host-or-the-agent), premise 1): **why it ended** decides whether to resume, **not who was watching**. The subscription count is only for P2's idle exit; the two questions must not share one yardstick.

`autoResumeInterrupted` (becoming a key in `~/.future/agent/settings.json`) remains the consent gate for "send prompts on my behalf".

### 9.2 Where it belongs: the TUI, a host, or the Agent

All three locations are technically workable. What separates them is not "can it", but **atomicity of de-duplication** and **where consent comes from**.

#### A. In the TUI only (workable, and it needs no host dependency)

The existing RPC surface already suffices; nothing new is required:

- `list_sessions` reports each session's `updatedAtMs` and `isStreaming` ([session_lifecycle.rs](../../agent/src/rpc/commands/session_lifecycle.rs)) — the age gate and the "is it running" check need no `app.db`;
- `get_state` reports `interruptedRun` and `pendingApprovals`;
- `prompt` submits the continuation.

The TUI only has to read the same consent switch: `future-app-settings` depends on rusqlite + serde alone ([app-settings](../../packages/app-settings/Cargo.toml)) and reads `app.db`, so **`future-host` is not needed**.

The cost: a third copy of the policy appears, and the TUI sees **sessions** rather than "conversations" — so it would also resume sessions created by channels, goal, the CLI and sub-sessions, which the user may not have meant.

#### B. In a host (today)

Good: policy, row settling, the `resume:<run id>` link and the UI all live on the `app.db` side. Bad, as §9.1's first row says: a machine with no host running (TUI only, or bare `future agent`) never resumes anything.

#### C. In the Agent (technically the cleanest)

The Agent already has every input, and needs no `app.db`:

| Input | What the Agent already has |
|---|---|
| This session has an unterminated run | `unterminated_run_id` ([manager/mod.rs](../../agent/src/session/manager/mod.rs)) — reads run markers only, never the whole transcript |
| It is not running now | `runtime.snapshot()` / `is_streaming` |
| It is parked on an approval | `approval_gate.pending_for_session` |
| Age gate | the session's `updated_at` |
| Execution summary | its own journal (more authoritative than `store::list_run_events`) |

It also already owns "a new run closes the prior interrupted one as `interrupted_by_restart`" (`new_run_closes_a_prior_interrupted_run`), so continuation is a natural extension of it.

#### The real difference: whether de-duplication can be atomic

Today `get_state` → `prompt` is **two steps**. Two clients (or a client and a host) both see `interruptedRun`, both pass the check, and then one queues while the other runs — **the same task runs twice**. `new_run_closes_a_prior_interrupted_run` clears the marker only after the first step, so it cannot close that window.

Only "decide + submit" inside one lock removes duplication structurally, and the Agent is the only place that can offer that lock.

#### Decision: the trigger lives in the Agent too (proposed by the user, 2026-10-21)

The user proposed letting the Agent trigger the pass itself: start `future agent`, attach a TUI and run a task, kill the TUI, shut the Agent down, start it again — and the interrupted conversation carries on.

Adopted, because it is the only one of the three locations that **de-duplicates structurally**: one instance holding the lock means one trigger, and the window where "two clients both see `interruptedRun` and each send a prompt" cannot exist. The price is four facts that must be handled with it.

| Responsibility | Owner | Why |
|---|---|---|
| Decide **and** submit the continuation prompt, atomically, triggered by the Agent itself once Ready | **The Agent** | It alone has the journal, the live runs, the approvals and the lock; it is also the only thing running on a bare `future agent` |
| Consent switch | **`~/.future/agent/settings.json`** (new key, e.g. `resumeInterruptedOnStart`) | The Agent must not read `app.db`: a bare `future agent` may have none, and that is app-owned storage (see premise 2 below) |
| The `resume:<run id>`-style link and row display | A host keeps it, as a **consumer** | It already maintains those rows |
| The old one-shot pass (`resume_interrupted_runs_at_launch`) | **Retired** | Keeping both is exactly the double-submit window above |

#### Premise 1: settling must record *why* the run ended

Measured: `request_shutdown` → `drain` → each session's `abort()` → the active run takes the normal terminal path with state `cancelled` ([session_prompt.rs](../../agent/src/rpc/session_prompt.rs)), and `request_abort` has **no cause parameter**. So **"shut it down and start it again" does not resume today**: after a graceful stop `interruptedRun` is null and `plan_resume` says `Terminal`. Only `kill -9` / a crash / OOM (a journal with `run_started` and no `run_terminal`) leaves `interrupted`.

So D8's criterion is not **"who is still watching" (subscription count)** but **"why did it end" (cause)**. The subscription rule fails precisely in the user's scenario: the TUI is already dead at step 3, the Agent is alone at step 4, so it would settle `cancelled`.

Abort therefore needs a cause:

| Cause | Terminal | Resumed on the next start? |
|---|---|---|
| `user_cancelled` (UI stop, cancel queued, a client `abort`) | `cancelled` | **No.** The user told it to stop. |
| `agent_shutdown` (host exit, Ctrl+C, upgrade, window closed) | `interrupted_by_restart` (or `cancelled` plus a resumable marker) | **Yes** |
| No terminal at all (killed / crashed / OOM) | already `interrupted` | **Yes** |

This makes step 4 of the user's scenario (a normal stop) work. The residual risk is "the user stops a runaway task by closing the Agent, and the next start continues it again"; the guardrails below plus cancelling the resumed run bound it.

#### Premise 2: the switch follows the executor (not a cost of D9)

It is tempting to read this as a cost specific to D9; placing the mechanism in headless or the TUI runs into the same question.

There is really one fact: **consent has to live wherever the executor can read it.**

- Trigger in a client → the client executes, so a switch in `app.db` is entirely reasonable (headless already calls `initialize_app_store` at startup).
- Trigger in the Agent (D9) → the Agent executes, and **`future-agent` does not depend on `future-app-settings` at all** ([agent/Cargo.toml](../../agent/Cargo.toml)); a bare `future agent` may have no `app.db`.

So this is not a mark against the choice but a direct consequence of it. And the shape already exists: `set_default_model` is exactly a **sessionless RPC → read `settings.json` → change a field → save** ([providers.rs](../../agent/src/rpc/commands/providers.rs)). A `get/set_resume_policy` pair has the same shape, and the Desktop settings page calls it the way it calls that one today.

Only two small things remain: a field in `~/.future/agent/settings.json` (beside `compaction` / `retry` / `maxTurns`), and a one-time map from the old `app.db` value so an upgrade does not silently reset the user's choice.

#### Premise 3: guardrails (without them it is unbounded spending)

The worst case is a task that reliably kills the Agent (OOM): restart → auto-resume → OOM → … So:

| Guardrail | Value |
|---|---|
| Resumes per session | at most **2** in 24h |
| Continuation chain | once a resumed run is itself interrupted, stop after **2** consecutive |
| Budget per launch | at most **3** sessions / **2** concurrently |
| Age | keep **7 days** |
| Skipped | sessions parked on an approval, `ephemeral` sessions, sub-sessions (unless the root session is also a candidate) |
| Reporting | one line per resume (session, old run, new run); one line stating why it gave up |

Timing requirements: **after** Ready, with the listener already up (so clients can see the run), **without blocking readiness**, registered in `service_work` (so shutdown waits for it), and concurrency-bounded (a cold server must not fire ten model requests at once).

#### Premise 4: clients must recognise "I did not say that"

If the continuation takes the "send a prompt" shape, that prompt is a **user message** and lands in the transcript as one, which all three clients would render as the user's own words; only a marker distinguishes it.

D10 ([§9.3](#93-sending-no-user-message-continue-inside-the-loop)) removes that shape entirely: the continuation appends only a request-only directive, and the transcript never contains the message. `run_started` still carries `auto_resume_of`, because clients must still draw "this stretch continued automatically".

#### Accepted trade-offs

| Trade-off | Detail |
|---|---|
| It spends money with nobody present | Every server restart continues what was interrupted; the user asked for this, but the switch must say so next to itself |
| It covers non-interactive sources | Sessions from channels, goal, the CLI and sub-sessions are resumed too; if that is unacceptable, narrow it to "sessions an interactive client ever joined" (needs a new marker) |
| Relation to P2 | Both need the Agent to know "who is still watching", for different purposes: P2 for idle exit, continuation for the cause. Do not merge them into one criterion. |

#### Jitter bound

What one restart costs the user = detection (probe cadence, ≤1s) + waiting/starting (≤180s, usually seconds) + cold start + one new model call. **The conversation and its history are unaffected**; interrupted tool calls are covered by the continuation prompt's execution summary, but **exactly-once tool execution is not guaranteed** — the prompt and this document must both say so (the existing prompt already does).

### 9.3 Sending no user message: continue inside the loop

Answer: **yes, and the shape already exists in the code**; two things blocked it. **Landed**: the accepting path, the placeholder tool result and the relaxed guard are in place, and the plan below is the order they were built in so each step could be verified on its own.

#### The precedent: a request-only message

When [run_loop.rs](../../agent/src/agent/run_loop.rs) continues an answer after a reconnect, it inserts a message that exists only in that one request, and says exactly why:

> This is request-only recovery context, not a fabricated user submission in the durable conversation or UI.

That is the mechanism for "no user message, just one sentence in the request". Today it is used only for stream retries.

#### Blocker 1: the loop refuses a conversation that ends with an assistant message

`run_streaming_with_messages` rejects it up front:

```rust
if last.role == "assistant" {
    return Err(anyhow!("Internal error: conversation ended with an assistant message. \
         This is a bug — please report it."));
}
```

And that is exactly the tail a mid-run death leaves behind — assistant and tool entries are already on disk while the stream runs.

#### Blocker 2: `prompt_internal` always builds a user message

It builds the message, pushes it into `self.messages`, persists it immediately and broadcasts `user_message`. There is no "run without a prompt" entry point.

#### The tail has three shapes, and one handling does not cover them

| Tail | Is the request valid? | What the continuation must do |
|---|---|---|
| A tool result | Valid | Append the request-only directive |
| assistant (text/reasoning only) | Rejected by blocker 1 | Relax the guard **and** append the directive; or move that assistant stretch out of the request and fold its text into the directive |
| assistant with tool_calls and no results | **Invalid** (HTTP 400: tool_calls must be followed by tool messages) | Add a placeholder tool result, then the directive |

The third is the easiest to miss: the cancel path already knows this (run_loop writes `[Tool execution cancelled …]` for calls it never executed), but **the crash path does not**. And after a crash "did this tool run?" is unknown, so the placeholder must say so rather than copy "not executed".

#### The shape of the change (D10)

1. A resume-run acceptance path inside the Agent: no user message, but still accept-before-mutate, `run_lease`, the `start_run` broadcast and the run row.
2. The guard changes from "reject an assistant tail always" to "allow it only when the journal proves it is an unterminated run's tail". **Do not delete that assistant stretch** — the model already produced it, and deleting it loses history.
3. Append the request-only directive at the tail, carrying what today's prompt carries: continue, why it was interrupted, and the execution summary. It simply never enters the transcript.
4. `run_started` carries `auto_resume_of`, so all three clients render such a run as "continuing automatically".

#### Gains

- **Cache**: appending at the tail leaves the prefix untouched. Putting it in the system prompt changes the prefix, which is the 48× cost order recorded in the workspace memory.
- **Faithful transcript**: it reads as one interrupted turn that continued, not as a user utterance that appeared from nowhere.
- **No fake bubble**, so no client needs a "hide this user message" special case.

#### Costs

- It touches the most delicate path (`run_lease` / journal / broadcaster / run rows), which is riskier than submitting an ordinary prompt.
- `thread-projection` and all three UIs currently assume "a run follows a user message" and must handle a run without one.
- If some provider accepts neither an assistant tail nor an appended directive, the "send a prompt" shape has to remain available. Both paths must coexist.

#### Recommendation

D10 ahead of the original "send a prompt" shape: do the "tail is a tool result" and "assistant with tool_calls, add placeholders" cases first (smallest change, **no need to relax the guard**), then relax it for a plain assistant tail.

As built, the guard needed one intermediate shape before the last step: an assistant turn carrying unanswered tool calls *is* an assistant tail, so step 2 allowed that shape only — while continuing, and only when the turn had calls the placeholder could answer. Step 3 then widened the allowance to any assistant tail on the resuming path. The relaxation is never general: `continue_interrupted_run` re-reads `unterminated_run_id` and refuses anything the journal does not still show as open, and the old "send a prompt" path remains as the fallback for a provider that takes neither.

## 10 Implementation checklist

### P1 (landed)

| Change | Location |
|---|---|
| `Supervisor` / `Decision` / backoff; split `StartupBlocked` into `Busy` and `Blocked` | `packages/rpc/src/connection.rs` |
| `InstanceBusy` typed error | `agent/src/service/instance.rs`, `agent/src/service/mod.rs` |
| `Phase::Busy`, `detach()` | `host/src/agent_host.rs` |
| One-shot launch → resident supervision; the waiting status line ([§5](#5-waiting-is-bounded)) | `tui/src/agent_supervisor.rs` |
| `check_running` → a supervision tick (probe the endpoint, not just our own phase) | `cli/src/headless/agent.rs` |
| Same tick wired in | `desktop/src-tauri/src/agent_supervisor.rs` |
| The breaker ([§6](#6-the-breaker)) | Inside the supervisor; no new file |
| A distinct lock-conflict exit code (e.g. 75) so the parent can read "lost the election" | `agent/src/cli.rs` |
| `CancelCause` threaded from abort to the terminal record (D8's premise) | `agent/src/rpc/session.rs`, `agent/src/runtime/`, `agent/src/session/entry.rs` |
| Agent-triggered continuation (§9.2): decide and submit atomically after Ready, with guardrails | `agent/src/service/startup.rs`, `agent/src/rpc/`, `agent/src/config/mod.rs` |
| The switch in `~/.future/agent/settings.json` plus a one-time map from the old `app.db` value | `agent/src/config/mod.rs`, `packages/app-settings`, the Desktop settings page |
| A user-message-free resume acceptance path + the request-only directive + the assistant-tail guard (D10) | `agent/src/rpc/session_prompt.rs`, `agent/src/agent/run_loop.rs`, `agent/src/session/entry.rs` |
| An "unknown" placeholder tool result when a crashed tail lacks one | `agent/src/agent/run_loop.rs` (beside the existing cancel path) |
| `run_started` carries `auto_resume_of`; clients render "continuing automatically" | `agent/src/session/entry.rs`, `tui/`, `desktop/`, `mobile/`, `packages/thread-projection/` |
| The old one-shot pass is retired (`resume_interrupted_runs_at_launch` and `host/src/agent_bridge/resume.rs` deleted with it) | `desktop/src-tauri/src/lib.rs`; the replacement lives in `agent/src/rpc/resume.rs` |

### Later

- **D6** (TUI embedding): `future-host` added to `tui/Cargo.toml`; an `environment::report` sink; the spawn branch and `check_sidecar_program` deleted.
- **P2** (lifetime): `--ephemeral` + active-subscription counting + idle exit.

## 11 Acceptance

All process-level tests, shaped like the existing `tui/tests/agent_sidecar_launch.rs`:

1. **Concurrent cold start**: headless + tui on one fresh home → exactly one Agent, the other attaches, **neither exits with an error**, identical instance ids.
2. **Owner `kill -9`**: the remaining one or two bring an Agent up within budget, with a new instance id, an intact database, and old runs interrupted.
3. **Two survivors**: headless + tui both attached, kill the owner → exactly one Agent comes up, the other waits/attaches.
4. **Bounded waiting**: a fake owner holds the lock and never becomes ready → each entry prints [§5](#5-waiting-is-bounded)'s sentence and acts per the table at the cap.
5. **Breaker**: a fake Agent exits immediately → after the third attempt it stops and prints why; after the cooldown it makes exactly one half-open attempt.
6. **Slow start**: a fake Agent takes 5s to become ready while another client starts → no lock stealing, no false "absent".
7. **After D6**: quitting the TUI does not take headless down (with P2=A, verify that headless restarts the Agent and in-flight tasks are marked interrupted).
8. **Manual `future agent` Ctrl+C**: clients do not restart it and say so (D7).
9. **The user's five steps** (§9.2's scenario, end to end): `future agent` up → a TUI attaches and starts a long task → kill the TUI → **stop the Agent normally** → start `future agent` again → that conversation must carry on by itself.
10. **The cause distinguishes them**: same home, one deliberate user cancel → a restart does **not** resume; one normal stop → a restart **does**.
11. **Guardrails**: a resumed run that is interrupted again stops after 2 consecutive; three candidate sessions start at most three, at most two at a time, each with a log line.
12. **Shape and marker**: the resumed run has **no** user message in the transcript and all three clients mark it "continuing automatically" (D10); when the tail is an assistant message with `tool_calls`, the request must carry a placeholder tool result or the provider rejects it.
13. Windows: named pipe + `.owner.lock` reclaim + the new process group.

## 12 Open questions

Questions the user must answer, ordered by blast radius:

1. **D6: should the TUI start the Agent in-process?** It affects the TUI's crash isolation, memory and cold start; the gain is one policy for all three entries.
2. **P2: should the Agent outlive the TUI?** Option A (dies with its starter; others restart it, which interrupts the current turn though the conversation is continued — see §9.1) versus B (detached + idle exit).
3. **D7: after the user stops it explicitly, should other clients bring it back automatically?** Recommendation: no (respect "stop means stop"), but it needs confirmation.
4. **Continuation can spend money with no client present**: now that the switch lives in `~/.future/agent/settings.json`, should its default be on or off? (Today's `autoResumeInterrupted` is on)
5. **Should continuation cover non-interactive sources?** (channels / goal / CLI / sub-sessions). Suggested default: resume only sessions identifiable as "the conversation in front of the user"; the rest needs an extra marker.
6. **Go-ahead for P1.** The design is approved, but all three entries' startup paths change; D8/D9/D10's items (the cause, Agent-triggered continuation, the switch following the executor, the user-message-free resume) should ride in P1 — otherwise self-healing leaves conversations half way.

Not user decisions — parameters to tune against measurement: the 180s wait cap, the 3-per-300s breaker threshold, the 60s initial cooldown, `IDLE_GRACE` 120s, and whether `future tui -p` also embeds.

## 13 Measured behaviour

Every row below was **run**, not inferred from reading code: a release binary, one isolated
`FUTURE_HOME` per case, nothing touching the developer's real `~/.future`, and a local stub
model endpoint so no real credential was spent on the verification.

| # | Scenario | Before | Now |
|---|---|---|---|
| 1 | `future agent` owns the Agent, headless attached → `kill -9` the agent | nobody took over within 30s; headless only reconnected | headless brings a new Agent up **within 3s** (`the Agent is gone; starting a new one`, new instance id) |
| 2 | headless owns the Agent, TUI attached → end headless | nobody took over within 40s; the TUI sat on `Connection to agent lost` | the TUI brings a new Agent up **within 1s** |
| 3 | Lock held, default endpoint absent (Agent bound to TCP only) → start the TUI | the TUI **exited 1** with "Cannot start a local Agent" | the TUI **keeps waiting**, printing `waiting for the Agent another process is starting (30s of 180s)` plus the holder's pid/host |
| 4 | Same, with another headless | exit 1 | the same wait through `supervise()` |
| 5 | `resumeInterruptedOnStart=true`, an interrupted run and a model → start `future agent` | no continuation | `1 conversation(s) checked, 1 continued`; the stub received one request whose last user message is the continuation prompt; `interruptedRun` becomes null and `queryCount` goes 1→2 |
| 6 | The same but the switch is **off** (the default) | — | **zero** continuations; the session honestly stays `interrupted_by_restart` |
| 7 | An interrupted session with no usable model | — | reported as `has no model configured; left interrupted`, never a fake success |
| 8 | An interruption older than seven days | — | skipped (counted as checked, not continued) |

Test and environment failures fixed in the same change:

| # | Problem | Root cause (measured) | Fix |
|---|---|---|---|
| 9 | `models::tests::registry_injects_future_models_from_disk_cache` flaked under parallelism (2 of 10 runs) | Several test modules build a `Registry` without a `TestHome`; a refresh thread they spawn publishes a different catalog into the process-global memory cache inside another test's reset→assert window | Test builds serialize reads, publishes and resets through one re-entrant `CACHE_ACCESS` lock, and a refresh thread carries a generation number so one that spans a reset no longer publishes |
| 10 | `grpc::tests::the_public_serve_wrappers…` failed in a long-path worktree | `$HOME/.future/run/agent.sock` exceeded the unix `sun_path` limit (104 bytes on macOS): 113 in the worktree, 78 in the main tree | An endpoint over budget falls back to a per-user short directory under the temp root plus a path hash (still per-instance); short homes keep the documented layout |
| 11 | `cli/tests/bin.rs::config_wizard_reports_the_missing_console…` hung | With piped stdin, the unix path still let `rpassword` open `/dev/tty`; a process with a controlling terminal blocks there forever (Windows already had a guard) | Non-terminal stdin is refused before the platform read, with the same message Windows reports |

Rows 1–3 under "Before" are where this work started: they record the behaviour **before** the fix, so the
table shows that something changed rather than that it always worked.

Not covered: Windows named pipes and process groups; the `cause` distinction with a real credential
(see §12).
