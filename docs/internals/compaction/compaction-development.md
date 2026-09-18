# Compaction developer guide

Runtime compaction projects a bounded view of a session's own journal for the next request.
Two strategies, `deterministic-evidence-v1` and `summarized-evidence-v1`; the
[policy](compaction.md) defines them and the [experiment](compaction-closed-book-experiment.md)
measures them.

## 1. Invariants

- Change the next request's projection, not an in-flight response or a provider's KV cache.
- Never delete or rewrite original journal data. User text has priority, and an omitted
  assistant output must never be described as already summarised.
- Retained tool exchanges stay paired, coverage is monotonic, and references resolve.
- Install only successfully committed checkpoints, with the receipt committed atomically.
- Excerpts are historical data, not instructions. Omission is not absence, and an older
  error may have been superseded.
- Selection never depends on a provider, a gold answer, or entries beyond the coverage
  boundary.

## 2. Source map

| Responsibility | Source |
|---|---|
| Pre-turn, mid-turn and provider-limit execution | [run_loop.rs](../../../agent/src/agent/run_loop.rs) |
| Run configuration and persistence | [session_prompt.rs](../../../agent/src/rpc/session_prompt.rs) |
| Manual execution and terminal results | [session.rs](../../../agent/src/rpc/session.rs) |
| Async compact ACK and busy checks | [settings.rs](../../../agent/src/rpc/commands/settings.rs) |
| User CLI | [session_compact.rs](../../../cli/src/commands/session_compact.rs) |
| Evidence grouping, priority and rendering | [evidence.rs](../../../agent/src/compaction/semantic/evidence.rs) |
| Shared planning and finalize | [semantic.rs](../../../agent/src/compaction/semantic.rs) |
| Request budget | [budget.rs](../../../agent/src/compaction/budget.rs) |
| Durable entry point | [durable.rs](../../../agent/src/compaction/durable.rs) |
| Fingerprint, claim and atomic completion | [compaction_ops.rs](../../../agent/src/session/compaction_ops.rs) |
| Ordered writer and barrier | [persistence.rs](../../../agent/src/session/persistence.rs) |
| Checkpoint and fork references | [checkpoint.rs](../../../agent/src/session/checkpoint.rs), [fork.rs](../../../agent/src/session/fork.rs) |
| Original evidence | [history_query.rs](../../../agent/src/session/history_query.rs) |

## 3. Runtime pipeline

```text
persist original user/assistant/tool entries
 -> estimate complete input and reserve ordinary output/margin
 -> threshold / manual / provider-limit admission
 -> persistence barrier and receipt claim
 -> protection, tail and coverage plan
 -> deterministic evidence from covered originals, at most 2K
 -> budget and progress validation
 -> atomic checkpoint + completed receipt
 -> next model step uses the new projection
```

`prepare_evidence` takes no provider, so the deterministic path cannot make a summary call
by accident; the summarised path is a separate entry point that does. Evidence reserves
`min(2048, W/8)` tokens independently of how long any summary turns out to be, while history
keeps the 32K target, the 128K expansion ceiling and the real request-capacity guard.
The handoff request still targets 4096 text tokens and retains its separate reasoning/output
allowance. Acceptance checks the **complete projected request**, not just whether the text
exceeded that target: a small estimator overrun is accepted when the admitted projection
fits. A result exceeding the projection/request limits falls back without truncating the
summary, shrinking evidence, or adding a model retry. Admission-policy fingerprinting
prevents replaying a receipt computed under the previous text-only rejection rule.

### Evidence rendering

Scan raw messages only through the admitted coverage boundary. Associate a result with its
unambiguous preceding call and never guess a path for a duplicate parallel ID. Prioritize
errors, the first and latest record of each group, important targets, then recency. Bound
both metadata and excerpts, budget the escaped JSON, and never emit half a JSON row.
`entryId`/`blockIndex`/`sourceOrder` distinguish original chronology from priority order, and
no reasoning, image body or provider-private object enters the index.

A user `--instructions` note is stored verbatim with a notice that the selector does not
interpret it, and an oversized note fails. Demoted assistant output is labelled
omitted/not-summarised, never as a generated summary.

## 4. Persistence and idempotency

Schema 3 is the only schema written and the only one read, and the two algorithm names above
are the only values `algorithm_version` takes from this build. A checkpoint carrying anything
else — an older schema, or the released string-protocol marker — is not a checkpoint at all:
the journal is projected in full and the next compaction re-covers it (see
[policy](compaction.md)). The compatibility field `summary` holds the evidence index, plus
the handoff summary when there is one; it is not proof of a model call.

```text
absent -> started -> completed (checkpoint + receipt transaction)
             +-> failed (recorded failure)
started without a result -> indeterminate diagnostic, no automatic takeover
```

Even though deterministic compaction has no summary bill, it keeps the concurrency fence: two
racing preparations must not both activate. `indeterminate` is a diagnostic, not a fourth
table state. A change to original data invalidates the key; checkpoint, run-marker,
session-info and usage changes do not themselves change the original input.

The policy fingerprint covers ordering, excerpts, serialisation and retention semantics, so
review it when any of those change. Replay must not undo a later operation that ran with
different settings. Deleting a session clears its receipts, a fork has its own scope, and
ephemeral callers have no cross-restart promise.

## 5. CLI, and a model-requested interface that does not exist

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "Do not deploy" --json
```

Use fresh correlation IDs and an explicit session, without switching the default session or
touching SQLite directly. The ACK is not completion, and an active run rejects manual
compaction. No force, wait or selector-mode switch is exposed.

**A model self-request interface is not implemented.** Do not simply remove the active-run
guard: mutating context before the triggering tool result is durable would race the live run.
Such an entry point would have to be a nonblocking intent — validate trusted
session/run/epoch origin, ACK immediately, persist the current tool result, then coalesce and
consume at the next model-step boundary through the same machinery. It must never make a shell
wait on its own run, grant a budget or indeterminate-state bypass, or treat a freely supplied
session ID as authorisation. The request and its ACK also add history, so content
idempotency alone cannot prevent self-trigger loops: intent deduplication, pending coalescing,
consumed state and bounded frequency are all required. Do not advertise the interface before
it exists.

## 6. Validation

Cover: zero summary calls on the deterministic paths with accumulated ordinary usage
unchanged; grouped first/latest evidence, errors, Unicode and large results, ambiguous call
IDs, hidden content and instruction-like excerpts; evidence budgets, user protection,
assistant-demotion notices, cancellation and invalid boundaries; retired-schema entries being
ignored, fork remapping, byte reads; duplicate, concurrent and restart receipts, changed
input, corrupt cache and failed durability; and real CLI ACK/busy semantics with bounded
provider-limit recovery.

Use project worktrees, an isolated `HOME` and fresh ports, and never stop the user's Agent.
Raise the macOS file-descriptor limit for the full suite, and place sandbox-test `HOME` under
the project target directory rather than the system temporary allowlist.

### Summary outcome reporting

A committed checkpoint is not proof that a model summary was retained. New summarised-path
checkpoints persist optional `summary_outcome`, also sent on `compaction_committed` and
receipt-reuse `compaction_unchanged` events. The manual result (and durable receipt) exposes
it as `summaryOutcome`, beside `algorithmVersion`:

```json
{"status":"evidence_only","fallback_reason":"summary projection rejected: ...","attempt_usage":[{"prompt_tokens":100000,"completion_tokens":4181,"reasoning_tokens":516,"credit_cost":0.125}]}
```

`status` is `generated` (handoff retained) or `evidence_only` (successful compression without
it). `fallback_reason` is absent on success; `attempt_usage` contains the final reported
usage of each attempt, including discarded summaries and existing transient retries. An
absent price is unknown, not free. This is diagnostic data, **not a second billing event**.
A failure to commit still uses `compaction_failed`, never a success outcome. Older checkpoints
and explicit deterministic-only preparations may lack this optional diagnostic field.
Existing clients can ignore the additive JSON field; no protobuf change is required.

Identical input/policy reuse returns the recorded outcome without another provider call,
including when that outcome was a fallback. This does not add automatic retries on replay.

A receipt hit still costs a fingerprint check and selection. Synthetic results establish
feasibility, not that a complex natural-language task needs no semantic processing.
