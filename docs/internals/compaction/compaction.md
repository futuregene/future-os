# Runtime compaction

Compaction replaces a session's older history — for the next request only — with a bounded
projection. The raw journal is neither deleted nor rewritten; recall reads it.

**Two strategies, and the runtime default is the second:**

| `algorithm_version` | The projection |
|---|---|
| `deterministic-evidence-v1` | protected originals + a recent tail + a deterministic tool-evidence index. **No model call.** |
| `summarized-evidence-v1` | the same projection, plus a model-written handoff summary |

Those are the only two values written, and the only two read. `deterministic-evidence-v1` is
also the fallback: it is committed whenever no provider is reachable or the summary call
fails.

The [experiment](compaction-closed-book-experiment.md) measures what each retains, at what size and
cost; the [developer guide](compaction-development.md) maps the code and the durable state
machine.

## Trigger and admission

The economic trigger is `floor(W × 0.8)` — 80% of the declared window, with no absolute cap.
`effective_trigger` clamps it to `W − O − margin`, where `O` is the output budget this request
actually sends and admission reserves, and `margin = min(2048, W/16)`.

`O` is not the declared ceiling itself but the value `models::effective_max_tokens` narrows it
to: `min(declared, 65536, W/4)` (with an unknown window only the absolute ceiling remains). A
declared ceiling often equals the window — Kimi's `/v1/models` returns
`context_length == max_tokens`, because input and output share one window — and reserving it
verbatim drives `W − O − margin` to zero, so not even a session's first request goes out and no
model call is made. The same function decides both the request's `max_tokens` and the admission
reserve, so the two cannot disagree.

So a 1M-token window declaring 384 000 output reserves 65 536 and triggers at **800 000** (80%
of the window; declaring 16 384 triggers there too); Kimi k3 (1 048 576 window, same declared
value) triggers at **838 860**; a 262 144 window with the same declared value is decided by the
proportional bound and triggers at **194 560** (74% of the window).

The check runs before every model step, including after tool calls and after a model
downshift, and reserves the output budget `O` plus the margin — input must fit
`W − O − margin`. The reserve is subject to the same bound: even a caller that passes an
unnarrowed declared value leaves three quarters of the window for input. The estimate covers
system text, tool definitions, message framing and
conservative image/reasoning costs, and reported usage is taken into account. Selecting a
model does not itself compact; its own next request checks its limits. An input that still
fits is not cut merely for crossing the trigger when no older history is compactable, and
mandatory content that cannot fit fails explicitly rather than being silently truncated.

## The projection

1. Covered user text and selected assistant originals.
2. An evidence slot of at most **2048 estimated tokens**, reduced to `min(2048, W/8)` on
   small windows.
3. Roughly 8K of recent paired tool/conversation history, reduced on small windows.

The history target is about 32K, excluding fixed system/tool costs, and may expand up to
**128K** only within request capacity: `min(128000, (limit − fixed) × 3/4)`, so a 128K window
yields 82K and a 262K window 96K.

User text has priority. When assistant originals exceed the remaining room, newer ones that
fit are kept and the omitted ones are marked **not summarized**; their originals stay
queryable. Protected blocks carry no tool calls, thinking, media bodies or hidden provider
state.

## Evidence selection

Original messages are scanned only through the admitted coverage boundary, in this order:
error results first; then grouped by tool and target path/command, prioritizing error-bearing
and generic config/schema/validation/test targets; then the latest and first record of each
group; then remaining space by recency.

Each selected record becomes one bounded JSON row carrying `entryId`, `blockIndex`,
`sourceOrder`, call ID, tool/target, error flag and head/tail excerpts (up to 380 + 100
characters, with smaller excerpts when space demands). A row is never cut in half to fill the
budget, and metadata has its own length limits.

The index is priority-ordered, not a timeline: `sourceOrder` is original order and older
errors may be superseded. Omitted middles and unselected records stay unknown, and a
successful execution is not proof of complete validation. Excerpts are historical evidence,
not new authorization, and an ambiguous or reused tool-call ID is not attributed to an
arbitrary path. No reasoning, image body or wholesale provider metadata enters the index.

## The summary request

`summarized-evidence-v1` sends the live conversation as **real messages**, with the
instruction appended last, so the prefix is untouched and the request can be served from the
provider's prefix cache. That prefix is the system prompt, then the tool definitions, then
the messages, compared from token zero — so the request carries the **session's own system
prompt** and the **session's own tool definitions**. The system prompt used to grow a
post-checkpoint recall guidance, which meant the budget had to reserve it in advance and the
summary request had to reproduce it; the guidance is gone, so both paths simply send the
session's prompt and a test pins them to one string.

Changing any of the three diverges the prefix and the whole conversation is billed again.
Measured: substituting the system prompt, dropping the tool definitions, or adding one line
each gave **0%** cache on a primed prefix, while the identical-shape request gave 93.7%. On
an isolated agent running this path, a session grown to 212 911 tokens compacted with
`cache_read = 212 548` — **99.8%** served from cache, `cache_write = 360`, about ¥0.003
against about ¥0.53 cold.

The summary is **cumulative**: the instruction carries the previous summary, which is
discarded once the new one is written. Its allowance is `min(W/16, 4096)` plus a 512-token
slot margin, granted only when a summary is wanted, and it widens the admitted budget so the
evidence index keeps its full size and the summary fits inside the target `finalize`
enforces. When the summary fails or no provider is available, the deterministic projection is
committed and the index header states that no summary accompanies it.

## Checkpoints and idempotency

Only the current schema is read. A checkpoint written by a retired algorithm — an older
`schema_version`, or the released string-protocol marker — is not recognised:
`latest_context_checkpoint` skips it, `project_prompt_context` sees no boundary and projects
the journal in full, and the next compaction re-covers that history with the current
algorithm. The retired row remains in the journal as an inert entry that nothing reads, so it
can neither shrink nor expand a prompt. The cost is one request whose input is the full
journal — normally cache-served, since the turn just sent it — and one extra summary call for
that session, once.

`compaction_operations` keys the original user/system/assistant/tool identity and contents,
relevant configuration, budgets, mode/phase, any note, and the policy version; checkpoints,
usage and session-info changes do not themselves change the raw input. A same-key success
reuses the result without another checkpoint, and the ordered writer commits the checkpoint
and the completed receipt atomically. An unresolved `started` operation retains the
concurrency/recovery fence — there is no automatic claim stealing and no fabricated success.
Deletion clears receipts, a fork has its own scope, and ephemeral/in-memory callers have no
cross-restart receipts. Matching upgraded CLI and Agent builds are required: older Agents do
not implement these semantics.

Protected originals are rebuilt by reference; a fork remaps the references and an invalid
range is rejected.

## User CLI

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "Keep current constraints and verification boundaries" --json
future session compact --help
```

This returns an async `accepted`/`operationId` ACK, not completion; agent events report
completion, reuse or failure, and active runs are rejected. There is no wait, force or
model-self-request option.

`--instructions` is a **verbatim user note** for continuation. The deterministic selector
does not interpret natural language or claim to have fulfilled semantic selection
instructions; the note consumes evidence budget, and an excessive note fails explicitly
rather than being silently truncated.

## Cost and recall

Deterministic compaction makes **zero summary-model calls**; the default adds one summary
request per compaction, charged like any other request. Ordinary usage and cost counters are
preserved, and ordinary requests still pay for evidence and retrieved text in their input.
Local scans also cost time and memory.

A [history recall guide](../../guide/session-history.md) exists, but the runtime does not
append it to the model: the post-checkpoint guidance was removed because it did not change
behaviour (the measurement is in
[compaction-open-book-experiment.md](compaction-open-book-experiment.md)). The retained
affordance is `future session history search`/`get` behind the ordinary shell tool, plus the
entry ids the journal already puts in search/read results. Missing exact facts are recovered
by reading the originals, never by replaying old tool side effects.

## Validation

```sh
cargo test -p future-agent
cargo build -p future-cli --bin future
python3 scripts/tests/test_s2_compaction.py --binary target/debug/future --report target/c-smoke.json
```

Use `future.exe` on Windows and respect `CARGO_TARGET_DIR`. The synthetic smoke uses its own
HOME, fresh port and local model stub to verify 80%-of-window triggering, identity,
originals, ordinary usage, byte paging and restart, and that a rejected summary falls back
without a second billed call. Never stop the user's running Agent.

Rule-based evidence is lossy, especially for complex unstructured material; original-history
recall remains essential.
