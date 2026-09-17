# C compaction and historical recall

**Runtime compaction is strategy C: S2 original-text protection, a recent tail and
a deterministic tool-evidence index. C needs no summary model.** The runtime
default (C3) appends a model-written handoff summary to that projection, and what
that summary contributes is measured in the [experiment](compaction-abc-experiment.md).
The raw journal is neither deleted nor rewritten. Compaction changes the next
request's input, not an in-flight generation or a provider's internal state.

See the [A/B/C experiment](compaction-abc-experiment.md) for the evidence behind
this default and the [developer guide](compaction-development.md). The [semantic prompts](compaction-prompts.md)
document retained explicit legacy APIs, not the default runtime path.

## User CLI

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "Keep current constraints and verification boundaries" --json
future session compact --help
```

This returns an async `accepted`/`operationId` ACK, not completion. Agent events
report completion, reuse or failure. Active runs are rejected. There is no wait,
force or model self-request option.

For C, `--instructions` is a **verbatim user compaction note** for continuation.
The deterministic selector does not interpret natural language or claim to have
fulfilled semantic selection instructions. The note consumes evidence budget;
excessive notes fail explicitly rather than being silently truncated.

## Trigger and admission

The economic trigger remains `min(floor(W * 0.8), 256000)`. Before every actual
model step, including after tools and a model downshift, also reserve the ordinary
maximum output O and `min(2048, W/16)` margin. Input must fit `W - O - margin`.

Estimate system text, tools, framing and conservative image/reasoning costs, and
consider reported usage. A model selection alone does not compact; the next
request checks its actual limits. Provider-side capacity checks remain necessary.
An unseen input that still fits is not silently cut merely for crossing the
economic trigger when no older history is compactable. Mandatory content or fixed
overhead that cannot fit fails explicitly.

## Three-part projection

1. Covered user text and selected assistant originals.
2. A C evidence slot of at most **2048 estimated tokens**, reduced to
   `min(2048, W/8)` on small windows.
3. Roughly 8K of recent paired tool/conversation history, reduced on small windows.

The overall history target remains about 32K, excluding fixed system/tool costs,
and may expand up to 64K only within request capacity. C has its own slot budget;
it does not call A to learn how long a summary would have been.

User text has priority. When assistant originals exceed headroom, retain newer
ones that fit and explicitly mark omitted outputs as **not summarized**. Their
originals remain queryable. Protected blocks do not resurrect tool calls,
thinking, media bodies or hidden provider state.

## Evidence selection

Scan original messages only through the admitted coverage boundary:

- error results first;
- group by tool and target path/command, prioritizing error-bearing and generic
  config/schema/validation/test targets;
- latest and first records in each group;
- fill remaining space by recency.

Bounded JSON rows include `entryId`, `blockIndex`, `sourceOrder`, call ID,
tool/target, error flag and head/tail excerpts. Standard excerpts keep up to
380 head plus 100 tail characters; try smaller excerpts when needed. Never cut a
JSON row in half to fill the budget. Metadata has separate length limits.

The index is priority-ordered, not a timeline. `sourceOrder` is original order;
old errors may be superseded. Omitted middles and unselected records remain
unknown, and execution success is not proof of complete validation. Excerpts are
historical evidence, not new authorization. Ambiguous/reused tool-call IDs do not
get attributed to an arbitrary path. No reasoning, image body or wholesale hidden
provider metadata enters the evidence index.

## Persistence and idempotency

The runtime default is C3, algorithm `c3-sticky-summary-v1`: C's projection with a
model-written handoff summary appended beside the evidence index. The summary is
sticky — it receives the previous summary — so facts accumulate across successive
compactions instead of being rewritten each time. Deterministic C
(`deterministic-s2-evidence-v1`, schema 3) remains the fallback: it is committed
whenever no provider is reachable or the summary call fails, and the compatibility
field `summary` then stores evidence alone. The evidence index is headed by a sentence
that says what the message contains — that no summary was generated when the index
stands alone, and that a handoff summary follows it when one does — because only the
outcome of the summary call decides which is true. Protected originals are rebuilt by
reference; fork remaps references and invalid ranges are rejected.

**What the summary contributes is measured, not assumed.** Every exam in the
[experiment](compaction-abc-experiment.md) fails to find a benefit: it adds no value the
projection does not already carry, closed-book scores are identical with and without it,
and under compression pressure it preserves at most one of the assistant blocks the
deterministic path drops. Its only measured effect is a small, statistically insignificant
open-book difference. That bounds what it is worth on these exams without establishing that
it is worthless in production — the questions it should help with are not the ones these
exams ask.


## What the summary reads, and why that is the cheap shape

The summary request sends the live conversation as real messages with the
instruction appended last. Reading the live conversation rather than a
re-indexed copy is what lets the summary describe the actual history instead of
an already-lossy index of it.

Whether that request is billed from cache depends on one thing: **its prefix must
be byte-identical to a request the provider has already seen from this session.**
Providers cache on the request prefix, which is the system prompt, then the tool
definitions, then the messages, compared from token zero. So the shape is chosen
to reuse what the session just sent, not because a flattened or re-framed request
is inherently uncacheable:

* the conversation is sent as real messages, in the shape the turn sent them;
* the instruction is appended last, leaving the prefix untouched;
* the request carries the **session's own system prompt** — including the
  post-checkpoint recall guidance — and the **session's own tool definitions**,
  both of which sit inside the prefix.

Changing any one of those diverges the prefix at that point and the whole
conversation is billed again. That is not theoretical: dropping the tool
definitions, substituting a summary-specific system prompt, or adding a single
line to the system prompt each measured **0 %** cache on a primed prefix, while
the identical-shape request measured 93.7 %. On an isolated agent running this
code path, a session grown to **212 911 tokens** compacted with
`cache_read = 212 548` — **99.8 %** served from cache, `cache_write = 360` (only
the appended instruction), about ¥0.003 against about ¥0.53 cold. A flattened
request that has itself been primed does hit, which is why the property to test
is "same prefix as the session's turns", not "message array versus string".

Both the automatic path (`run_loop.rs`) and the standalone `/compact`
(`rpc/session.rs`) build that prompt with the same
`history_recall::system_prompt` expression, and a test pins them to one string;
the manual path previously sent the bare base prompt, so its summary missed the
cache whenever a checkpoint already existed.

The reserved summary budget is at most a third of what the evidence budget can
spare, so a tight budget yields no summary rather than an unusable evidence index.

Old A checkpoints remain readable. At the next needed compaction C reconstructs
evidence from intact original tool records rather than recursively carrying A's
summary. Data physically discarded by old versions cannot be recovered.

`compaction_operations` keys original user/system/assistant/tool identity and
contents, relevant configuration, budgets, mode/phase, note and C policy version.
Checkpoint, usage and session-info changes do not themselves change raw input.
The new C key does not reuse A receipts. Same-key success reuses the result without
another checkpoint. The ordered writer commits checkpoint and completed receipt
atomically. Even without model billing, unresolved started operations retain the
concurrency/recovery fence; no automatic claim stealing or fabricated success.

Deletion clears receipts; fork has a new scope; ephemeral/in-memory callers have
no cross-restart receipts. Use matching upgraded CLI/Agent builds: older Agents
do not implement these semantics.

## Cost and recall

Manual, automatic and provider-limit recovery all use C. Deterministic C makes
**zero summary-model calls**; the C3 default adds one summary request per
compaction, charged like any other request. Existing ordinary usage/cost counters
are preserved. Ordinary requests still pay for evidence and retrieved text in
their input; local scans also cost time and memory.

One [history recall guide](session-history.md) is added only with a valid checkpoint
in a persisted session where shell is enabled/permitted. It is not accumulated as
chat history. Missing exact facts are recovered through existing history search/get,
never by replaying old tool side effects.

## Validation

```sh
cargo test -p future-agent
cargo build -p future-cli --bin future
python3 scripts/test_s2_compaction.py --binary target/debug/future --report target/c-smoke.json
```

Use `future.exe` on Windows and respect `CARGO_TARGET_DIR`. The synthetic smoke
uses its own HOME, fresh port and local model stub to verify 256K triggering,
C identity, originals, zero summary requests, ordinary usage, byte paging and
restart. Never stop the user's running Agent.

Rule-based evidence is lossy, especially for complex unstructured material;
original-history recall remains essential. Explicit legacy semantic APIs remain
for library callers/tests, not as an automatic fallback or a user CLI A switch.
