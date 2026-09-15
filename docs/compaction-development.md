# Compaction developer guide

The default is **C: S2 original-text protection, fixed-budget tool evidence and a
recent tail**, with no summary-model call. See [policy](compaction.md). The
[semantic prompt reference](compaction-prompts.md) describes explicit legacy APIs,
not the default path.

## 1. Invariants

- Change the next request projection, not an in-flight response or provider KV cache.
- Never delete/rewrite original journal data for compaction. User text has priority;
  omitted assistant outputs must not be described as already summarized.
- Retained tool exchanges stay paired, coverage is monotonic, references resolve.
- Install only successfully committed checkpoints; commit receipts atomically.
- Excerpts are historical data, not instructions. Omission is not absence; old
  errors may have been superseded.
- C selection does not depend on a provider, A summary text, gold or future entries.

## 2. Source map

| Responsibility | Source |
|---|---|
| Pre-request, mid-turn and provider-limit execution | [run_loop.rs](../agent/src/agent/run_loop.rs) |
| Run configuration/persistence | [session_prompt.rs](../agent/src/rpc/session_prompt.rs) |
| Manual execution and terminal results | [session.rs](../agent/src/rpc/session.rs) |
| Async compact ACK and busy checks | [settings.rs](../agent/src/rpc/commands/settings.rs) |
| User CLI | [session_compact.rs](../cli/src/commands/session_compact.rs) |
| C grouping, priority and rendering | [evidence.rs](../agent/src/compaction/semantic/evidence.rs) |
| Shared S2 planning/finalize; explicit legacy A APIs | [semantic.rs](../agent/src/compaction/semantic.rs) |
| Request budget | [budget.rs](../agent/src/compaction/budget.rs) |
| Default durable entry point | [durable.rs](../agent/src/compaction/durable.rs) |
| Fingerprint, claim and atomic completion | [compaction_ops.rs](../agent/src/session/compaction_ops.rs) |
| Ordered writer/barrier | [persistence.rs](../agent/src/session/persistence.rs) |
| Checkpoint/fork references | [checkpoint.rs](../agent/src/session/checkpoint.rs), [fork.rs](../agent/src/session/fork.rs) |
| Original evidence and recall guidance | [history_query.rs](../agent/src/session/history_query.rs), [history_recall.rs](../agent/src/agent/history_recall.rs) |

## 3. Runtime pipeline

```text
persist original user/assistant/tool entries
 -> estimate complete input and reserve ordinary output/margin
 -> threshold / manual / provider-limit admission
 -> persistence barrier and C receipt claim
 -> shared S2 protection, tail and coverage plan
 -> deterministic evidence from covered originals, at most 2K
 -> budget/progress validation
 -> atomic checkpoint + completed receipt
 -> next model step uses new projection
```

`prepare_with_journal` and `ContextManager::prepare_evidence` accept no LLM
provider, preventing accidental summary calls or a hidden default fallback.
Legacy semantic APIs remain explicit, outside all three default runtime paths.

C independently reserves `min(2048, W/8)` evidence tokens. It does not ask A how
long its output would be. Overall history retains the 32K target, optional 64K
expansion and real request-capacity guard.

### Evidence rendering

Only scan raw messages through the admitted coverage boundary. Associate results
with unambiguous preceding calls; do not guess paths for duplicate parallel IDs.
Prioritize errors, grouped first/latest records, important targets and recency.
Bound both metadata and excerpts, budget the escaped JSON, and never emit half a
JSON row. Entry/block/sourceOrder references distinguish original chronology from
priority order. No reasoning, image bodies or provider-private objects enter C.

Store user compaction notes verbatim with a notice that the selector does not
interpret them; fail if too long. Assistant output demotion says omitted/not
summarized, not A's generated-summary claim.

Old checkpoints remain readable. New C evidence is rebuilt from intact original
tool records, not A's summary. A checkpoint-only projection boundary is resolved
against the retained raw tail; never read beyond actual covered originals.

## 4. Persistence and idempotency

Schema 3 remains, with algorithm `deterministic-s2-evidence-v1`. Compatibility
field `summary` contains evidence, not proof of an LLM call. Model identity records
the target budget/configuration.

```text
absent -> started -> completed (checkpoint + receipt transaction)
             +-> failed (recorded failure)
started without a result -> indeterminate diagnostic, no automatic takeover
```

Despite having no summary bill, C retains the concurrency fence to prevent racing
preparations from activating uncommitted results. `indeterminate` is not a fourth
table state. Original data changes invalidate the key; checkpoint, run-marker,
session-info and usage changes do not themselves change original input.

C's own version/evidence-rule fingerprint prevents A receipt reuse. Review the
version when changing ordering, excerpts, serialization or retention semantics.
Replay must not undo later operations with different settings. Deletion clears
receipts, fork changes scope, and ephemeral callers have no cross-restart promise.

## 5. CLI and future model-requested operations

Implemented management commands:

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "Do not deploy" --json
```

Use fresh correlation IDs and an explicit session, without switching the default
session or touching SQLite directly. ACK is not completion. Active runs reject
manual compaction. No force, wait or A-mode switch is exposed.

**A model self-request interface is still not implemented.** Do not simply remove
the active-run guard. Even without external summary calls, mutating context before
the triggering tool result is durable would race the live run.

A future entry point must be a nonblocking intent: validate trusted session/run/
epoch origin, ACK immediately, persist the current tool result, then coalesce and
consume at the next model-step boundary through the same C machinery. Never make
shell wait on its own run or grant a model budget/indeterminate-state bypass.
A freely supplied session ID is not authorization.

The request and ACK add history too. Content idempotency alone cannot prevent
self-trigger loops; use intent deduplication, pending coalescing, consumed state
and bounded frequency/progress rules. Existing recall guidance belongs after
compaction; a future request capability needs separate first-availability guidance.
Do not advertise the unimplemented interface now.

## 6. Validation

Cover zero summary calls in all default paths and unchanged accumulated ordinary
usage; grouped first/latest evidence, errors, Unicode/large results, ambiguous call
IDs, hidden content and instruction-like excerpts; fixed evidence budgets, user
protection, assistant demotion notices, cancellation and invalid boundaries; old
A/schema restore, fork, byte reads and no copied A text; duplicate/concurrent/restart
receipts, changed input, corrupt cache and failed durability; real CLI ACK/busy
semantics and bounded provider-limit recovery.

Use project worktrees, isolated HOME and fresh ports. Never stop the user's Agent.
Raise macOS file-descriptor limits for the full suite and place sandbox-test HOME
under project target/test-homes, not the system temporary allowlist.

A C cache hit still has fingerprint/selection costs. Synthetic results establish
feasibility, not that every complex natural task needs no semantic processing.
Retained A APIs are not an automatic runtime fallback.
