# Compaction developer guide

Implementation baseline: `6b30cce6` (S2, history recall and durable idempotency).
This document separates **implemented behavior** from **proposed interfaces**.
See [S2 policy](compaction.md) and [history recall](session-history.md) for usage.

## 1. Mental model and invariants

Compaction changes the input to the **next** model request. It does not edit an
in-flight response or a provider's internal KV cache.

```text
immutable conversation journal
        + checkpoint coverage / protected references
        -> protected original text + state summary + recent tail
        -> next ModelRequest
```

Preserve these invariants:

- Compaction does not delete or rewrite original user/system/assistant/tool records.
- Protected blocks contain text, not replayed tool calls, thinking, media bodies or
  provider-owned hidden state.
- Retained tool exchanges remain paired; coverage never moves behind an existing
  checkpoint and protected references resolve to valid original entries.
- Incomplete, over-budget or unsuccessfully persisted preparation does not install
  an uncommitted checkpoint.
- Historical commands and instructions are evidence, not fresh authorization.
  Neither compaction nor recall re-executes historical side effects.

## 2. Implementation map

| Responsibility | Source |
|---|---|
| Pre-request / mid-turn / provider-limit execution | [agent/run_loop.rs](../agent/src/agent/run_loop.rs) |
| Run-scoped configuration, persistence and journal wiring | [rpc/session_prompt.rs](../agent/src/rpc/session_prompt.rs) |
| Manual execution, usage persistence and results | [rpc/session.rs](../agent/src/rpc/session.rs) |
| Async compact ACK, busy checks and terminal events | [rpc/commands/settings.rs](../agent/src/rpc/commands/settings.rs) |
| Request budgets | [compaction/budget.rs](../agent/src/compaction/budget.rs) |
| Planning, folding, retries and final validation | [compaction/semantic.rs](../agent/src/compaction/semantic.rs) |
| `prepare_with_journal` | [compaction/durable.rs](../agent/src/compaction/durable.rs) |
| Content fingerprints, claims and atomic completion | [session/compaction_ops.rs](../agent/src/session/compaction_ops.rs) |
| Ordered writer, barrier and `commit_compaction` | [session/persistence.rs](../agent/src/session/persistence.rs) |
| Checkpoint encoding / validation and fork remapping | [checkpoint.rs](../agent/src/session/checkpoint.rs), [fork.rs](../agent/src/session/fork.rs) |
| Read-only history queries | [history_query.rs](../agent/src/session/history_query.rs), [CLI](../cli/src/commands/session_history.rs) |
| Conditional model guidance | [agent/history_recall.rs](../agent/src/agent/history_recall.rs) |

## 3. Implemented execution path

### Admission and trigger

Accept the run and persist its user message with a stable entry ID. Before the
actual model step, resolve model limits and project the original journal through
the valid checkpoint. Estimate system text, tool definitions, message framing,
images/reasoning conservatively and consider provider-reported usage.

The economic trigger is `min(floor(W * 0.8), 256000)`. Input must also fit
`W - O - min(2048, W/16)`, where `O` is the configured maximum output. The capacity
bound may be reached earlier. An unseen user input is not lossily summarized just
because it crosses the economic trigger if there is no older history to compact
and the request still fits. Disabling auto-compaction does not disable admission.

### S2 preparation

Keep original user text and as much assistant text as fits, plus a recent paired
tail of roughly 8K or less on small windows. Target about 32K history tokens,
excluding fixed system/tool overhead; expand up to 64K only within available
headroom. Prefer newer assistant texts that fit, summarize other outputs with an
explicit retention notice, and fail rather than silently remove user directives
when mandatory content cannot fit.

Use the selected model with tools disabled. Summary text is budgeted at up to
4096 estimated tokens; the request-local output cap is at most 8192, scaled down
and bounded by the model. Do not mutate the ordinary request's output settings.
Include the template, accumulator, custom instructions, wrappers, output reserve
and margin in each fold's budget. Tool excerpts carry original entry references.
Omitted content remains unknown: successful execution or a partial excerpt is not
proof of complete validation, absence of errors, or absence of relevant content.

Connection and stream waits are cancellable; retries are bounded. Only
provider-context-limit recovery may use a deterministic emergency summary, still
subject to final budget and progress checks.

### Commit and the next request

1. Drain accepted journal appends through `SessionPersistence.barrier()`.
2. Persist the idempotency claim before model work; do not hold a database
   transaction across an external model request.
3. Validate preparation, then atomically commit the checkpoint and completed
   receipt through the ordered writer.
4. Install successful context for subsequent model requests.
5. Add one recall guide only for a valid checkpoint in a persisted session with
   shell enabled/permitted. Do not append it to chat history or summary requests.

The manual RPC returns an asynchronous ACK. **An ACK is not completion.** Clients
must correlate terminal events with the operation; neither the ACK nor a process
exit code alone proves that compaction completed.

## 4. Persistence and idempotency contract

Schema-3 checkpoints store coverage, summary, `protected_entry_ids`, model/policy
identity and token estimates. Restore, import and fork validate/remap references.
Schema 2 remains readable; data physically discarded by older versions cannot be
recreated.

`compaction_operations` is keyed by `(session_id, input_key)`. The input digest
scans canonical original user/system/assistant/tool records in order, including
identity and contents, not merely the last entry ID. Object keys are canonicalized;
array and message ordering remain significant. Checkpoints, run markers, usage and
session-info changes do not themselves change original history.

The policy key includes model/protocol/thinking/cwd/tools, budgets, trigger/phase,
normalized custom instructions and summary policy.

```text
absent --atomic claim--> started --checkpoint + receipt transaction--> completed
                          |                                           |
                          +--recorded known failure--> failed          +--reuse

started without a durable result -> compaction_indeterminate, no blind re-send
```

`indeterminate` is a diagnostic, not a fourth stored state. The original work may
still be running or may have been interrupted; the state does not prove whether
an external provider charged anything.

- A successful same-key duplicate adds no checkpoint, model call or usage charge,
  even when the first result retained a recent tail.
- New data or relevant parameters form a new key. Different compaction modes are
  not interchangeable operations.
- Replaying an old result must not undo a later operation with different settings.
- Removed, modified or invalid cached checkpoints cause an error, not silent
  regeneration. Known failures replay an error; unresolved starts do not expire
  into automatic retries.
- Session deletion clears receipts; fork has a new scope. Ephemeral/direct
  in-memory callers cannot promise cross-restart receipts.

RPC `operationId` identifies an attempt; `sourceOperationId` on reuse identifies
the original compaction. In-memory request-ID caching is only supplementary.
Changes to semantics, fingerprint inputs or serialization require an idempotency
policy-version review so old results are not reused incorrectly.

## 5. Should a model-callable compact CLI be added?

**The user management CLI is implemented; the model-requested entry point remains
a proposal.** Do not synchronously expose manual compact inside its own active run.

### User / automation management command (implemented)

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "Keep current constraints and validation boundaries" --json
```

[session_compact.rs](../cli/src/commands/session_compact.rs) uses
`RunClient::compact_session` to wrap the existing compact RPC. It retains busy,
budget, idempotency and writer checks, uses an explicit session and fresh request
ID, and does not switch the default session or access SQLite directly.

The command returns an async ACK and operation ID, not a completed summary. There
is no `--wait`, `--force`, or model-facing `compact request` entry point. A future
wait option must subscribe/replay terminal events rather than treating the ACK as
completion.

### Model command: request, do not execute immediately

The model's shell tool executes inside an active run, so the current manual RPC
returns `session_busy`. Removing that guard and waiting synchronously can create:

```text
run waits for shell -> shell waits for compact -> compact waits for run boundary
```

It also risks covering unpersisted tool results or competing with auto-compaction.
A model-facing command should submit a nonblocking **intent** instead.

Proposed spelling only:

```text
future session compact request --session SESSION_ID --request-id INTENT_ID --json
```

Proposed lifecycle:

1. Validate the target against a trusted tool-execution session/run/epoch binding
   and record one pending intent.
2. Return acceptance immediately. Neither the CLI nor model polls or waits for
   its own run to finish.
3. Persist the current tool call and result normally.
4. Consume the intent at the next model-request boundary, coordinated with the
   automatic/provider-limit path rather than starting a second worker.
5. Re-evaluate necessity, stable input and budget, then use the existing durable
   preparation/commit mechanism.
6. Return a no-op when unnecessary, install a successful projection, or fail/
   continue under existing admission policy.

The request is advisory: no model-controlled force bypass for capacity, budget,
permissions or indeterminate receipts, and automatic safety checks remain the
backstop. If trustworthy execution-origin binding is unavailable, ship only the
user management CLI first. A freely supplied session ID is not authorization.

### Content idempotency alone cannot prevent self-compaction loops

The request tool call and its ACK become new history. A new history digest does
not prove useful progress. Add run/epoch/intent deduplication, pending-request
coalescing, consumed state and bounded frequency/progress rules. A fresh model
request ID must not bypass a pending or indeterminate operation. Summary requests
must remain tool-free.

### Keep the two prompt instructions separate

The existing **history recall** guide belongs after compaction. A future
**request-compaction capability** needs a short explanation when it first becomes
available; otherwise the model cannot discover its first request from a guide
that only appears afterwards. Advertise it only after implementation and explicit
enablement. Say that execution happens at a boundary, not immediately, and prohibit
waiting/polling/repeated requests. Do not advertise the unimplemented model-request
entry point; the management CLI still rejects active runs.

See [model calls and prompts](compaction-prompts.md) for call-count semantics,
the exact system prompt and user-message assembly.

## 6. Development and validation checklist

Cover duplicate/concurrent keys, retained-tail retries, restart, same-ID content
changes, configuration changes, transaction failures, unresolved starts, fork/
delete, corrupted cached checkpoints, trailing usage and ordinary output settings.

Before shipping the proposed model entry point, also verify:

- Acceptance during an active run returns immediately; execution starts only
  after its triggering tool result is durable, with no wait cycle.
- Auto-trigger and model intent produce one coordinated decision.
- Stale epochs, cancelled/finished runs and untrusted cross-session targets fail.
- Repeated intent IDs, changing IDs and growth caused by the request itself do
  not generate unbounded summaries or charges.
- No-progress, unresolved receipts, persistence failure and provider limits have
  explicit outcomes.
- The summarizer has no tools; unsupported CLI versions, unavailable shell and
  ephemeral modes do not advertise unavailable capabilities.

Work in an isolated project worktree. Test Agents need separate HOME and ports;
never restart the user's Agent. Raise macOS file-descriptor limits for the full
suite and keep sandbox-test HOME under project `target/test-homes`, outside the
system temporary allowlist. Do not log credentials or send private sessions to
external models.

A receipt hit still hashes/checks original history locally; it is not O(1) or free.
Measure I/O, RSS and latency as history grows. Synthetic stubs verify mechanics,
not real-model summary or recall quality.
