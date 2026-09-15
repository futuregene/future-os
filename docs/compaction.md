# C compaction and historical recall

**Runtime compaction defaults to C: S2 original-text protection, a recent tail and
a deterministic tool-evidence index. No summary LLM is called.** The raw journal
is neither deleted nor rewritten. Compaction changes the next request's input,
not an in-flight generation or a provider's internal state.

See the [developer guide](compaction-development.md). The [semantic prompts](compaction-prompts.md)
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

C uses schema 3 with algorithm `deterministic-s2-evidence-v1`. The compatibility
field `summary` stores evidence, not a generated summary. `model` describes the
target window, not a model that performed summarization. Protected originals are
rebuilt by reference; fork remaps references and invalid ranges are rejected.

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

Manual, automatic and provider-limit recovery all use C: **zero summary-model
calls**. Existing ordinary usage/cost counters are preserved and no auxiliary
summary tokens are charged. Ordinary requests still pay for evidence and retrieved
text in their input; local scans also cost time and memory.

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
