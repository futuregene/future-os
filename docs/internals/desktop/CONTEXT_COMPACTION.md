# Context compaction architecture and next-phase semantic compaction plan

> ([中文](CONTEXT_COMPACTION.zh-CN.md)) Status: **v2 data foundation landed; the
> semantic compaction core (S1) is implemented and wired into the runtime** —
> automatic (PreTurn/MidTurn), provider-limit recovery, and manual `/compact`
> all call `prepare_semantic_with_lifecycle` (`agent/src/agent/run_loop.rs`,
> `agent/src/rpc/session.rs`), producing `semantic-v1` summaries with a
> `deterministic-emergency-v1` fallback on provider-limit failure. The
> model-switch "old model first" fallback chain (S3) is implemented but not
> yet connected at runtime call sites (all pass `None`; tests only); S4
> compatibility/release close-out is 【待核实】. (2026-08-24; runtime wiring
> re-checked 2026-09-16)

Baseline commit: `8fd6804e Implement durable context compaction checkpoints`

This document is the current authoritative design for FutureOS context
compaction. It has two parts:

1. the landed v2 data and compatibility foundation, which later work must not
   break;
2. the next phase's local semantic compaction, model-switch detection, and
   compaction-request fault tolerance.

This document no longer lists the completed Journal, Prompt Projection,
ContextCheckpoint, RPC/UI markers, and fork compatibility work as future
migration items.

## 1. Non-relaxable delivery constraints

Every phase must guarantee: **no user-perceivable data break**.

After upgrades, existing sessions, messages, reasoning, tool calls/results,
attachments, run states, compaction markers, forks/clones, and complete exports
must remain readable. Compaction algorithms or schema evolution must not:

- delete, overwrite, re-attribute, or reorder existing messages;
- hide reasoning, tool errors, attachments, or provider metadata;
- disguise internal summaries as real user messages;
- make old sessions unable to continue running;
- make old compaction markers disappear or duplicate;
- make the original entries covered by a checkpoint disappear from UI history
  or complete export;
- switch to an in-memory context that cannot be recovered after restart
  because a checkpoint write failed.

agent and desktop release with the same version; online mixing of old and new
processes is not required. But new versions must be permanently read-only
compatible with all published session JSONL and run-event JSONL on disk.
Reading old logs must not trigger implicit batch migration or rewriting.

## 2. Confirmed product decisions

### 2.1 Not doing this phase

The following capabilities are explicitly excluded from the next phase:

1. **Independent compaction model/agent**: summaries use the current session
   model; model-switch scenarios may only retry between the old model and the
   user-selected new model — no third dedicated model is introduced.
2. **TokenBudget opening a new window without a summary**: clearing the
   model-visible history and starting a new window without a semantic summary
   is not allowed.
3. **Provider-native remote compaction**: no `/responses/compact`,
   `compaction_trigger`, or provider-private compaction objects.
4. **Remote → local fallback**: since there is no remote compaction path, no
   such fallback chain is designed.
5. **Writing the whole `replacement_history` into the checkpoint**: checkpoints
   keep only the summary, covered range, and metadata; the model context is
   derived from the append-only journal, avoiding duplicating the complete
   history in storage.

### 2.2 Doing this phase

The next phase adopts "OpenCode-style local structured summarization + the
existing FutureOS append-only checkpoint foundation", absorbing Codex's
lifecycle and fault-tolerance designs that do not depend on a remote provider:

- explicit merging of the previous summary;
- retained tail selected by complete user turns;
- atomic boundaries between assistant tool calls and tool results;
- bounded serialization of tool output/media in the summary input;
- three compaction phases: `PreTurn`, `MidTurn`, `Standalone`;
- model-switch and context-window downshift detection;
- the compaction request's own token budget, context-limit retry, and transient
  error retry;
- a deterministic emergency summary after semantic summary failure;
- switching the prompt projection only after the checkpoint durable commit
  succeeds;
- complete telemetry, failure diagnostics, and compatibility fixtures.

## 3. The landed v2 baseline

### 3.1 Immutable Session Journal

`SessionEntry` is the only persistent fact layer:

- user, assistant, reasoning, tool calls/results, and run markers stay
  append-only;
- compaction only appends a `type: "compaction"` v2 checkpoint entry;
- original messages are never deleted or rewritten for model-context
  optimization;
- UI, audit, fork, and complete export can still access all original data
  before the checkpoint coverage.

### 3.2 AgentMessage-native Prompt Projection

The run loop no longer overwrites the session truth through the lossy
`ConvertToLLM -> ConvertFromLLM` round-trip. Each model request derives one
`PromptContext` from the full journal and the latest valid checkpoint:

```rust
struct PromptContext {
    messages: Vec<ProjectedMessage>,
    usage: ContextUsage,
}

struct ProjectedMessage {
    message: AgentMessage,
    source_entry_ids: Vec<String>,
}
```

Messages before the checkpoint are only replaced by the summary inside the
model prompt; they remain fully in the journal, UI, and export. The recent tail
after the cutoff must keep provider metadata, tool errors, attachments, and
unknown content blocks field for field.

### 3.3 Explicit ContextPreparation

Whether compaction happened is expressed by a strongly typed result, not
inferred from array lengths, shared atomics, or string prefixes:

```rust
enum ContextPreparation {
    Unchanged { prompt: PromptContext },
    Compacted {
        prompt: PromptContext,
        checkpoint: Box<ContextCheckpoint>,
    },
}
```

Automatic compaction, provider context-limit recovery, and manual compaction
already go through the same `ContextManager`.

### 3.4 v2 ContextCheckpoint

The new writer reuses the existing `SessionEntry` envelope and writes a
structured v2 checkpoint only inside `content`:

```json
{
  "id": "entry_...",
  "type": "compaction",
  "role": "system",
  "content": {
    "schema_version": 2,
    "checkpoint_id": "cp_...",
    "covered_from_entry_id": "entry_...",
    "cutoff_entry_id": "entry_...",
    "summary": [],
    "tokens_before": 120000,
    "tokens_after": 18000,
    "trigger": "automatic",
    "algorithm_version": "v2",
    "model": "...",
    "context_window": 200000
  },
  "timestamp": "2026-08-24T12:00:00+08:00"
}
```

The loader already accepts:

1. the historical `[Context compaction: ...]` pseudo user message;
2. the old `type: "compaction"`, `content: {summary,tokens_in,tokens_out}`;
3. the v2 checkpoint.

A corrupted new checkpoint, or one with missing references or an invalid range,
falls back to the previous most recent valid checkpoint; with no valid
checkpoint, it safely rebuilds from the full journal.

### 3.5 Durable commit and UI markers

Each time a compaction flow actually starts, a unique `operation_id` is
generated, and the full lifecycle is emitted through the run-event journal:

1. the client calls the `compact` RPC, which returns `operationId` immediately
   — one unary RPC no longer synchronously waits for the model summary;
2. `compaction_started`: emitted before the summary request starts; the UI
   shows "Compacting";
3. `compaction_committed`: emitted after the checkpoint durable commit
   succeeds; the UI updates the same marker in place to success;
4. `compaction_failed`: emitted when any of summarization, boundary planning,
   or persistence fails; the UI updates the same marker in place to failure and
   keeps error details;
5. `compaction_unchanged`: ends the async operation when there is no new
   content to compact; the UI restores input submission and shows a
   no-compaction-needed hint.

All four events carry the same `operation_id`; `trigger` and `phase` are
explicitly carried in started/failed/unchanged, and committed carries them from
the checkpoint. The `Unchanged` path needing no compaction emits no started and
ends the accepted manual operation directly with unchanged, avoiding a fake
state.

The checkpoint shares the same FIFO persistence queue as message appends. Only
after all previous appends, the checkpoint append, flush, and `fsync` succeed
is it allowed to:

1. activate the new in-memory checkpoint;
2. emit `compaction_committed`;
3. update the UI marker to success.

Restart, reconnect, and history loading recover state from the checkpoint and
run-event journal; live events only carry low-latency notifications. The old
`compaction_end` remains compatibly readable and deduplicates against the
checkpoint markers. When the process is force-killed before committed, the
checkpoint is not activated and the prompt projection is unchanged; on recovery
an unclosed running marker must be handled as interrupted — never permanently
showing "Compacting".

### 3.6 Fork, paging, and storage boundaries

- fork rebuilds the old-to-new entry ID map and remaps the checkpoint's covered
  range; checkpoints with incomplete ranges are not copied;
- the session entry RPC supports compatible offset/limit ordered paging; old
  calls without paging parameters keep the original behavior;
- checkpoints and messages share one log sequence; paging does not change
  history content;
- the desktop SQLite schema is unchanged, and later phases must not add SQLite
  tables for context compaction either.

## 4. Remaining problems of the current baseline

The v2 data foundation fixed misreporting, history rewriting, and
compatibility, but the current summarizer is still a deterministic file-
operation summary:

```text
Previous conversation summarized.
Files read: ...
Modified: ...
```

It cannot stably preserve:

- the user's final goal and explicit constraints;
- decisions made and why;
- completed, in-progress, and blocked work;
- key commands, errors, verification results, and identifiers;
- non-file tool results;
- semantics from the previous checkpoint that are still valid.

Therefore the current implementation satisfies "raw data is not lost" but not
"the model can still reliably continue the task after multiple compactions".
The next phase only changes summary generation and trigger policies — it does
not overturn the v2 Journal/Checkpoint/RPC/UI foundation.

## 5. Next-phase target architecture

```text
Append-only Session Journal
        |
        v
Prompt Projection
  latest valid checkpoint summary + replayable tail after cutoff
        |
        v
Compaction Trigger Policy
  automatic / provider-limit / manual / model-context-downshift
        |
        v
Turn-aware Tail Selector
  complete user turns + atomic tool calls/results
        |
        +-------------------------------+
        |                               |
        v                               v
Semantic Summary Input              Recent Tail
  previous summary                   AgentMessage kept as-is
  covered head
  bounded tool/media serialization
        |
        v
Semantic Summarizer (current session model)
        |
        +-- request fault tolerance and chunked fold
        |
        +-- failure -> Deterministic Emergency Summarizer
        |
        v
ContextCheckpoint v2
        |
        v
append + flush + fsync
        |
        v
activate projection + emit committed event
```

## 6. Compaction lifecycle

A new explicit phase concept is added:

```rust
enum CompactionPhase {
    PreTurn,
    MidTurn,
    Standalone,
}
```

`phase` is an orthogonal attribute beyond `trigger`; it is proposed as an
optional additive field in the v2 checkpoint `content` and event payloads.
Missing on old checkpoints is allowed when reading and does not affect
compatibility.

| Phase | Where it happens | Typical trigger | How it continues |
| --- | --- | --- | --- |
| `PreTurn` | before a new round's normal model request | `Automatic`, `ModelContextDownshift` | the new turn starts normally after the checkpoint commits |
| `MidTurn` | when the assistant/tool loop still needs follow-ups | `ProviderContextLimit`, threshold overflow | keep the current tool boundary and retry directly within the same run |
| `Standalone` | user-initiated compact | `Manual` | generate and commit the checkpoint; no fake continue user message |

Execution must not resume via a synthetic `Continue...` user message. After
MidTurn compaction the run loop continues directly, avoiding changes to
transcript semantics or duplicate tool execution.

## 7. Trigger Policy

### 7.1 Automatic

Keep the current basic criterion:

```text
tokens_before = max(provider reported input_tokens, local estimate)
needs_compaction = tokens_before > context_window - reserve_tokens
```

Only the next prompt's input occupancy is counted; completion, reasoning
output, and cache statistics must not be double-mixed into the prompt size. The
default reserve keeps using the current model window policy; the concrete value
comes from the model registry/config.

### 7.2 ProviderContextLimit

When the provider returns a context-length/body-size error, force entry into
the same `ContextManager` — no fake token numbers. One checkpoint durable
commit consumes one model-request retry; the same projection with the same
error must not form an infinite compaction loop.

### 7.3 Manual

User instructions become additional constraints on the summary prompt, not
text concatenated directly into the summary. For example:

```text
Specifically keep JSONL compatibility, SQLite boundaries, and unfinished tests.
```

Manual compaction uses the same structured summarization, turn/tool safety
boundaries, persistence, and compatibility paths as automatic compaction, but
with a bounded recent-tail budget: keep the smaller value from the
configuration while capping the limit at 15K tokens, avoiding an oversized
context window turning explicit compaction into a near no-op.

When every real turn fits in that budget, manual compaction must not force a
cutoff after the first message. In that case the entire committed history
(including the previous checkpoint summary and the complete tail after it)
enters the new summary, `cutoff_entry_id` points at the latest persistable
message, and the compacted prompt keeps only the new summary. Only when history
exceeds the recent-tail budget is the most recent verbatim tail kept.

Desktop offers the "Compact" context tool at the top of the `/` menu in the
input box of conversations that already have an Agent session; the phone
offers the same tool at the top of its own `/` menu. Selecting it directly
calls the standalone `compact` RPC, generating no `/compact` user
message, ordinary Agent reply, or new Run; the summary request is the
operation's only LLM communication. `Manual` explicitly skips the automatic
context-window threshold but must still find valid turn/tool boundaries. A
successful checkpoint appends to JSONL with `trigger: "manual"`, and
Desktop/Mobile show the user's choice with the "you manually compacted this
conversation's context" divider; failures keep the manual marker too. The
phone reaches the same operation through the Desktop bridge: the
`compact_context` command (advertised as `compaction_v1`) forwards to the
session-scoped RPC and relays its operation id, which the phone correlates
against the terminal `compaction_*` event before reporting the outcome. When the
menu matches both context tools and Skills, tools are on top and Skills below,
with the "技能 / Skills" divider text shown only before Skills in mixed
results; single-category results show no divider. Chinese and English names and
descriptions both participate in search.

### 7.4 ModelContextDownshift

Proposed new trigger:

```rust
CompactionTrigger::ModelContextDownshift
```

Before a model settings change commits, re-evaluate the current
`PromptContext` with the new model's context window, reserve, and input
capabilities:

```text
current_prompt_tokens > new_context_window - new_reserve_tokens
```

Switching models alone, when the new model can fit the current prompt, does not
force compaction. FutureOS uses plain-text summaries and does not need Codex's
provider compaction compatibility hash.

## 8. Turn-aware Tail Selection

### 8.1 Turn definition

A retained turn starts at a real user message and ends before the next real
user message. Internal checkpoint summaries, UI-only entries, and run markers
must not be misjudged as new user turns.

Selection accumulates backward from the latest turn until reaching
`keep_recent_tokens`. Automatic compaction uses the model-window-derived
budget; manual compaction caps the effective budget at 15K tokens. Complete
turns are preferred; only when a single turn alone exceeds the budget is a safe
boundary search inside the turn allowed.

When manual compaction finds that the whole history from the first real turn
fits in the recent-tail budget, choose "summarize everything" instead of
"keep everything verbatim": the cutoff moves forward to the latest complete
journal boundary and the retained tail is empty. This rule guarantees that a
short conversation's `/Compact` truly shrinks into one summary covering the
whole conversation, not just the first message.

### 8.2 Tool atomic boundaries

The following combinations must not be split by a checkpoint cutoff:

- an assistant tool call and its corresponding tool result;
- parallel tool calls within one assistant message and their completed
  results;
- reasoning/tool metadata the provider requires to replay in pairs.

When the budget boundary lands on a tool result, roll back to the assistant
message that initiated the call. Synthetic dangling-tool repair items have no
independent journal ID and cannot become checkpoint cutoffs.

### 8.3 Recent tail fidelity

The recent tail gets no text summarization and no old-style round-trips; it
keeps the complete `AgentMessage`:

- provider item IDs, encrypted reasoning;
- Anthropic thinking signature/redacted thinking;
- `ToolResult.is_error`;
- attachments, unknown content blocks, and message metadata.

Summary input may serialize lossily; the recent tail may not.

## 9. Semantic Summary

### 9.1 Fixed output structure

Summaries use the current session model and must output fixed Markdown:

```markdown
## Objective
- what the user wants to accomplish

## Important Details
- user constraints, preferences, key facts, design decisions and reasons

## Work State
### Completed
- completed and verified items

### Active
- in-progress and partially completed items

### Blocked
- blocked, failed commands, and unknowns

## Next Move
1. concrete next actions

## Relevant Files
- exact paths, symbols, and their roles
```

The summary must match the session's primary language, preserve exact paths,
commands, error strings, URLs, IDs, and the user's explicit wording, not answer
questions from the old conversation, and not continue executing the task.

### 9.2 Previous summary merging

The second and later compactions must explicitly provide the model:

```text
<prior-summary>...</prior-summary>
<conversation>the head produced after the checkpoint that this compaction will cover...</conversation>
```

The prompt must explain:

- the new summary completely replaces the prior summary; information not
  carried over is lost from the model-visible context;
- goals, constraints, decisions, and parallel work still valid in the prior
  summary must be preserved;
- the conversation is newer than the prior summary; on conflict the
  conversation wins;
- resolved blockers and completed active work move to the correct state;
- `Objective` and `Next Move` must reflect the latest state.

The previous summary itself must not be silently dropped by ordinary token
trimming.

### 9.3 Summary input serialization

What goes to the summarizing model is an independently derived view, not a
rewrite of the raw JSONL. First-round serialization rules:

- user text kept verbatim; attachments become mime, filename, and a stable
  reference description — no raw binary embedded;
- assistant text kept; reasoning keeps only bounded text that helps explain
  decisions;
- tool calls keep the tool name and normalized arguments;
- tool results keep at most 2,000 characters by default, with truncation
  marked;
- tool errors keep the error type and bounded error text;
- original messages already covered by a previous checkpoint are not
  re-serialized; only the prior summary is used.

All truncations only affect the summary-request input and the model prompt;
they are never written back to the journal and never affect the UI or complete
export.

### 9.4 Model selection

No independent compaction model/agent is added:

- ordinary Automatic, ProviderContextLimit, and Manual use the current session
  model;
- ModelContextDownshift prefers the old model to summarize old history before
  the model switch really takes effect;
- when the old model is unavailable or a retryable model-related error occurs,
  retry with the user's already-selected new model;
- no third model is searched or called, and summary work is never routed to a
  hidden agent.

## 10. Compaction request fault tolerance

The compaction request itself must establish an independent budget before
sending:

```text
summary_input_budget
  = summarizer_context_window
  - system_and_summary_prompt_tokens
  - summary_output_reserve
  - safety_margin
```

It must not be assumed that "a normal session can enter compaction" implies
"the complete old history + summary prompt" is guaranteed acceptable to the
same model.

### 10.1 Pre-send bounding

Reduce the summary input in the following order until it fits the budget:

1. tool output truncated to 2,000 characters;
2. further keep only the tool name, argument summary, success/error status, and
   the key tail;
3. reasoning reduced to bounded text;
4. large attachments keep only descriptions;
5. the covered head is split into multiple chunks by complete turns.

The recent tail must not be trimmed to make the summary request pass; the
recent tail does not participate in the summary model's conversation input at
all.

### 10.2 Chunked fold

When the covered head cannot be processed in one request, chunk it oldest to
newest:

```text
summary_0 = previous_summary or empty
summary_1 = summarize(summary_0 + chunk_1)
summary_2 = summarize(summary_1 + chunk_2)
...
final_summary = summarize(summary_n + last chunk)
```

Each chunk must preferentially land on complete turn/tool boundaries. Every
step uses the same structured template and the same "the old summary is
replaced" merging rules. Only the final summary is written into the checkpoint;
intermediate summaries write no journal and emit no UI markers.

The summary's `Objective` only describes user goals not yet completed. The
latest request already answered, verified, or delivered must go into
`Completed`; when all requests are complete, `Objective` reads "waiting for the
user's next instruction" — a request must not be wrongly kept as an
in-progress goal just because it is the last user message.

### 10.3 Context-limit retry

Even when local estimates say the request fits, the provider may still return a
context limit. When that happens, one stricter re-planning is allowed:

- lower the single-chunk budget and the tool/reasoning caps;
- re-chunk by complete turns;
- re-send the current fold step;
- never repeat ordinary agent tool calls that already produced external side
  effects, because summary requests expose no tools.

The same fold step performs at most an agreed number of context-limit retries;
beyond that, enter the deterministic emergency summary — no infinitely deleting
oldest messages in a loop.

### 10.4 Transient error retry

Use bounded exponential backoff for timeouts, connection interruptions, server
overload, explicitly retryable 5xx, and rate limits; authentication failures,
invalid requests, cancellations, and non-retryable errors are not retried. A
user interrupt must terminate the summary request immediately — no checkpoint
submission continuing in the background.

### 10.5 Deterministic emergency summary

After semantic summary retries are exhausted, a local deterministic summary may
guarantee provider-limit recovery and manual operations a definite result, but
it must be more complete than the current file-list implementation:

- carries the previous summary verbatim;
- keeps the recent user goals and explicit instructions;
- extracts the assistant's recently completed content;
- extracts tool names, argument summaries, success/failure, and key errors;
- extracts read/modified files;
- records whether the current run/turn still needs tool follow-ups;
- carries manual compaction instructions;
- gives the retained tail start and unknown next items.

Use `algorithm_version` to distinguish quality:

```text
semantic-v1
deterministic-emergency-v1
```

Emergency summaries must equally pass checkpoint range validation and durable
commit. Never write an empty summary, and never misreport a semantic summary
failure as a `semantic-v1` success.

## 11. Checkpoint commit flow

All policies still produce the existing `ContextCheckpoint`; no
`replacement_history` is written:

```text
1. compute the contiguous covered range from ProjectedMessage.source_entry_ids
2. validate covered_from, cutoff, and tool boundaries
3. generate operation_id and emit compaction_started
4. generate the final summary
5. build the candidate PromptContext of summary + recent tail
6. compute tokens_before/tokens_after
7. append the checkpoint to the session JSONL
8. flush + fsync
9. activate active_checkpoint
10. emit compaction_committed carrying the same operation_id
11. call or retry the model with the new PromptContext
```

When any of steps 4 to 8 fails:

- the active checkpoint is not modified;
- no committed event is sent;
- `compaction_failed` is sent carrying the same operation_id, trigger, phase,
  and error details;
- the UI updates the running marker in place to failure;
- the memory-only summary is never used to call the ordinary model;
- a structured persistence error is returned.

## 12. JSONL, RPC, and SQLite compatibility

### 12.1 JSONL

The next phase keeps writing the v2 `SessionEntry` envelope — no new top-level
row types. Additive optional fields in the v2 `content` are allowed, e.g.:

```json
{
  "phase": "pre_turn"
}
```

Old v2 checkpoints missing new fields must have stable defaults. When
`trigger` adds `model_context_downshift`, desktop/mobile/RPC must update in the
same commit as the agent; historical readers keep accepting the existing three
triggers.

Summary algorithm upgrades must never rewrite old checkpoints. New writers
express the algorithm via `algorithm_version`, not batch migrations replacing
historical summaries.

### 12.2 Run-event and RPC

The manual `compact` RPC returns the async acceptance result
`accepted + operationId`; `compaction_started`, `compaction_committed`,
`compaction_failed`, and `compaction_unchanged` go into the existing run-event
journal and the RPC `StreamEvent` — no new session JSONL top-level row types.
The post-success prompt projection still uses only the checkpoint journal entry
as the fact source; started/failed/unchanged only describe operation status and
must not change the historical context.

### 12.3 SQLite

The next phase does not modify the desktop SQLite schema and adds no message,
run-event, summary, or checkpoint tables. Model-switch state, summary
intermediate results, and retry states belong to a single run's memory; only
the final checkpoint enters the agent JSONL.

## 13. Next-phase development plan

### Phase S1: semantic summary core

Goal: replace the current file-list summary with a structured semantic summary
without changing triggers yet.

- define the fixed summary template and the previous-summary merge prompt;
- build a summary-only `AgentMessage` serializer;
- implement the turn-aware head/tail selector;
- implement tool call/result atomic boundaries;
- the current session model executes a tools-free summary request;
- final checkpoints write `algorithm_version = "semantic-v1"`;
- manual instructions enter the summary prompt;
- keep the existing v2 checkpoint, RPC, and UI schema compatible.

Suggested main change areas:

- `agent/src/compaction/`: selection, serialization, prompt, summary
  orchestration;
- `agent/src/agent/run_loop.rs`: async `prepare` integration;
- `agent/src/rpc/session.rs`: manual compaction async conversion and result
  mapping;
- `agent/src/session/checkpoint.rs`: only supplementary compatible
  algorithm/optional-field reading tests.

### Phase S2: compaction request budget and fault tolerance

Goal: guarantee the summary request itself is bounded and recoverable under
long sessions and provider errors.

- compute the independent summary input/output budget;
- implement the 2K tool-output first-round truncation and strict mode;
- implement chunked summary fold by turn/tool boundaries;
- implement one context-limit re-planning;
- wire bounded retryable transport backoff;
- user cancellation threads through all fold steps;
- implement `deterministic-emergency-v1`;
- intermediate summaries are not persisted and emit no events.

### Phase S3: lifecycle and model switching

Goal: unify PreTurn/MidTurn/Standalone and compact ahead of time when the model
window shrinks.

- add `CompactionPhase`;
- add the `ModelContextDownshift` trigger;
- evaluate the new context window before a model settings commit;
- on downshift prefer the old model to summarize, allowing the user-selected
  new model to retry;
- after MidTurn compaction continue the same run with no synthetic user
  message;
- each provider-limit retry binds a unique checkpoint, preventing loop
  compaction;
- phase/trigger additions thread through agent, RPC, desktop, mobile, and
  thread projection.
- started/committed/failed correlate via operation_id; Desktop/Mobile show the
  running/completed/failed divider in place;
- agent/process interruption must not leave a permanent running state, and the
  active checkpoint must not change before committed.

### Phase S4: compatibility, observability, and release close-out

Goal: prove the semantic upgrade causes no data or behavior breaks.

- add summary input/output tokens, chunk counts, retry reasons, compression
  ratio, and algorithm version metrics;
- record structured failure phases, not sensitive summary bodies;
- run upgrade tests with published old session/run-event fixtures;
- verify mixed chains of semantic and emergency checkpoints;
- verify fork, paging, reconnect, export, and marker dedup;
- verify the current run, model field, and context window consistency around
  model switches;
- complete the full agent, RPC, desktop, mobile, and projection tests.

S1–S4 can be delivered as independent commits, but semantic compaction is
enabled by default only after S1 and S2 both complete; otherwise an oversized
summary request could degrade automatic compaction from "low quality but
usable" to "direct failure".

## 14. Verification matrix

### 14.1 Summary correctness

- first compaction produces the complete fixed Markdown structure;
- second compaction preserves goals, constraints, and decisions still valid in
  the prior summary;
- new conversation conflicting with the prior summary uses the new facts;
- completed/active/blocked migrate correctly with progress;
- exact paths, commands, error strings, URLs, and IDs are not rewritten without
  reason;
- semantic checkpoints are rejected when the summary is empty, template-only,
  or missing required structure.

### 14.2 Tail and provider fidelity

- the tail starts at a complete user turn by default;
- the cutoff does not split assistant tool calls/results;
- a single oversized turn can find a safe internal boundary; when it cannot, an
  explicit error or re-planning is returned;
- the recent tail's Responses item IDs, encrypted reasoning, thinking
  signature, tool errors, attachments, and metadata are field-for-field
  identical;
- summary-input truncation does not modify the original tool output in the
  journal/UI/export.

### 14.3 Request fault tolerance

- strict-mode retry executes when estimates fit but the provider returns a
  context limit;
- an overlong covered head folds through multiple complete-turn chunks;
- every fold step merges the previous accumulator summary;
- no half-written checkpoint on intermediate step failure;
- retryable network errors back off within caps;
- authentication, invalid requests, and user cancellation are not wrongly
  retried;
- after retry exhaustion a `deterministic-emergency-v1` is written, not a fake
  semantic success;
- the emergency summary also cannot be empty and must carry the prior summary.

### 14.4 Lifecycle and model switching

- a new turn starts normally after PreTurn compaction;
- completed tools are not re-executed after MidTurn compaction;
- Standalone manual compaction generates no synthetic user message;
- switching from a large-window to a small-window model with a prompt over the
  threshold compacts first, then switches;
- no meaningless checkpoint when the new model can fit the current prompt;
- when the old model fails and the user-selected new model succeeds, only one
  checkpoint is committed;
- when both models fail, enter the deterministic emergency summary or return an
  explicit error — no partial state left.

### 14.5 Persistence and zero data break

- the journal prefix before and after compaction is byte-identical, only the
  checkpoint appended;
- the active projection does not change when checkpoint commit/fsync fails;
- after restart the PromptContext matches the post-commit state;
- with multiple alternations of semantic and emergency checkpoints, only the
  latest valid checkpoint applies;
- old string markers, old compaction entries, and old v2 checkpoints can be
  read mixed together;
- old sessions continuing to run do not rewrite existing JSONL;
- checkpoint ranges stay valid after fork;
- UI/reconnect/event replay does not duplicate markers;
- started → committed and started → failed both show exactly one in-place
  updated marker;
- after a forced exit, uncommitted operations do not change the checkpoint and
  the recovery UI does not stay permanently in running;
- the complete export keeps including all original messages and tool outputs;
- the desktop SQLite schema snapshot is completely unchanged.

## 15. Completion criteria

The next phase is complete only when all of the following hold:

1. the default compaction summary is the structured `semantic-v1`, not a file
   list;
2. multiple compactions explicitly merge the previous summary;
3. the retained tail is selected by turn and tool atomic boundaries;
4. summary requests have an independent token budget, chunked fold, and bounded
   retry;
5. semantic summary failure has an identifiable `deterministic-emergency-v1`;
   no empty checkpoints written;
6. PreTurn, MidTurn, and Standalone behaviors have regression tests;
7. context-window downshift completes necessary compaction before switching
   models;
8. no independent compaction model/agent, provider-native remote compaction, or
   summary-less new window exists;
9. checkpoints do not save whole `replacement_history`;
10. the journal stays append-only; the checkpoint activates only after durable
    commit;
11. published old JSONL/run-event fixtures, fork, paging, UI markers, and
    complete export pass zero-data-break tests;
12. the desktop SQLite schema is unchanged.

## 16. Final decision

The next phase of FutureOS adopts:

> **local structured semantic summarization + previous-summary fold +
> turn-aware recent tail + bounded request fault tolerance + append-only
> ContextCheckpoint**

OpenCode's summary protocol and tail selection are the main algorithmic
references; Codex contributes only lifecycle, model-downshift detection, and
request fault tolerance unrelated to provider-native compaction. FutureOS keeps
its own completed immutable journal, stable entry provenance, durable
checkpoint, fork reference remapping, and UI/RPC compatibility foundation.

Any implementation requiring deleting original messages, rewriting historical
JSONL, writing whole replacement history, clearing the model context without a
summary, or switching the prompt before the checkpoint durably commits does not
conform to this design.
