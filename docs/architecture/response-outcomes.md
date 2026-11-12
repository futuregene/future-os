# Reply endings and anomaly hints

> ([中文](response-outcomes.zh-CN.md)) A model call ending is not a task
> failure; having no body is not a network disconnect either. End semantics are
> recognized by the protocol adapters; the Agent decides the task state and
> writes the anomaly cause into `run_terminal.error` and `agent_end`. Desktop
> uses the same cause for the divider, keeping history and live results
> consistent.

## Protocol boundaries

- Chat Completions: the model `finish_reason` and the SSE `[DONE]` are handled
  separately; after receiving the model end reason, usage reads may continue.
  Only `[DONE]`, or an EOF with no model terminal event, cannot confirm the
  reply completed.
- Responses: the response terminal state is authoritative, merging the final
  output snapshot first; missing deltas alone must not be asserted as no
  output.
- Anthropic: `message_stop` and the stop reason are authoritative; `tool_use`
  continues the tool flow, and `refusal` is not treated as a network failure.
- `pause_turn` keeps its own classification. Current adapters cannot replay the
  opaque payload of server-side tools, so there is no automatic resend; the
  user is told about the model pause and the software's recovery limits.
- Unknown end reasons keep the raw diagnostics, meaning unconfirmed — never
  arbitrarily attributed to model failure or network failure.

## Display

| Situation | Display |
| --- | --- |
| Normal completion with body or tool results | normal message |
| Normal completion with no body and no tool results | neutral divider: the model finished generating without providing a final reply |
| User cancelled | the original manual-stop divider |
| Context compaction | the original compaction progress/completed/failed divider |
| Single-output limit, continuation allowance spent | the output-limit divider; not conflated with context compaction |
| Single-output limit, still continuing | no divider; `output_stream_continuing` is not explanatory copy, the reply keeps growing |
| Single-output limit with `outputContinuation` off | the output-limit divider, behaviour as before the feature |
| Explicit connection read error | model service connection interrupted |
| Request timeout | model service response timeout |
| Missing or unknown model terminal state | reply ended unexpectedly, cause not yet confirmed |
| Upstream content limit, cancellation, pause | each shows the upstream-declared reason |
| Software processing/save failure | software failed to process or save the reply |

Already-generated text, thinking, and tool records are always kept. Hints are
never written into the model body. Turning off thinking display does not affect
dividers. A tool round ending does not error just because there is no text. A
normal empty reply is not rewritten as failure and is not automatically
re-requested.

## Continuing inside the run (output limit)

Hitting the per-response output limit no longer hands control straight back to
the user: the run keeps writing, inside itself. An interactive session does not
need a "继续" button, and an unattended goal worker does not have to redo the
work through a retry.

### Trigger and boundary

Only `detected_by == "finish_length"` continues. Every other upstream stop
(content filter, provider cancel, pause, timeout, stream error) is not a spent
budget — re-sending the identical request only buys the identical refusal — so
those still end immediately as `incomplete`. A user interrupt outranks all of
them.

The allowance is `MAX_OUTPUT_LIMIT_CONTINUATIONS = 2` (at most two continuations
past the initial request), independent of the reconnect budget
(`MAX_STREAM_RETRIES = 5`) so a flaky link and a long answer cannot spend each
other's. It follows the same `max_retries > 0` switch, so `set_auto_retry(false)`
and fusion (which sets it to 0 explicitly) are unaffected: those callers asked
for one request per step.

### Two kinds of cut, two instructions

- **This attempt wrote visible text**: the partial answer is a prefix, so the
  instruction is to keep writing from the break — do not restart, do not repeat
  delivered text, do not redo tool calls that already ran.
- **This attempt wrote nothing visible** (the whole budget went to reasoning):
  there is no prefix to extend, and a reasoning-only assistant message is
  dropped by `convert_to_llm`, so upstream cannot see it either. The
  instruction becomes "stop deliberating and produce the answer", and the run
  **steps the thinking level down one notch** — repeating an identical request
  provably hits the identical wall (measured: three turns at 32,768 tokens, all
  of it reasoning, zero visible characters). Stepping down touches only this
  run's private provider snapshot (`snapshot()`), never the session setting, so
  the user's next message still runs at the level they chose.

Stepping down walks one level on the fixed scale (`high → medium → low → off`)
filtered through the model's own `thinkingLevels`, so a level the model does not
declare never reaches the wire. `default` hides the real depth and cannot be
stepped relatively, so `low` is tried first and the model's shallowest declared
level is the fallback; at `off` there is nothing left to give up.

### Event and recovery semantics

A continuation emits `output_stream_continuing`
(`{type, attempt, maxContinuations, thinkingLevel?}`). Unlike `stream_retry`,
nothing is being reconnected and the reply keeps growing; `thinkingLevel` is
present only when the level was stepped down. Clients should clear a
`reconnecting` banner on it.

The truncated turn's assistant message is **not rebound**: it was already
pushed and persisted under this step's `stream_entry_id`, and writing a second
payload under one identity is rejected by the transcript store as
`conflicting entry identity`. The continuation path only appends placeholder
results for unexecuted tool calls and reuses the existing `retry_text` mechanism
to carry the delivered text into the next request.

When the allowance is spent and the reply is still cut, the original semantics
apply unchanged: `stream_incomplete` is set, `StreamTruncation` is recorded,
`run_terminal.error = [OUTPUT_LIMIT] …`, and the path in "A goal worker's
incomplete retry is not a continuation" takes over. Automatic continuation is a
layer *before* the final failure; it does not replace `incomplete`.

Regression cover (`agent/src/agent/run_loop.rs` tests):
`spent_output_budget_continues_the_same_run`,
`reasoning_only_cut_steps_thinking_down_before_continuing`,
`output_budget_continuation_is_bounded_and_stays_incomplete`,
`auto_retry_disabled_does_not_continue_output_budget`,
`shallower_thinking_level_walks_down_the_scale`.

### The switch and its default

| Setting | Default | When switched off |
| --- | --- | --- |
| `"outputContinuation"` in `~/.future/agent/settings.json` | `true` (on) | behaviour reverts to pre-feature semantics: the cut ends the run as `incomplete`, writes `[OUTPUT_LIMIT]`, and Desktop/Mobile show the "继续" button |

This is **the only path that adds tokens on its own** — users pay per use, so it
must be switchable, and the default belongs here, not only in the code. The
setting is applied at session creation
(`Settings::output_continuation_enabled()` → `Session::set_output_continuation()`)
and is deliberately separate from `auto_retry`: disabling automatic retry (or a
fusion caller setting `max_retries = 0`) is not a statement about continuing, and
conflating them would make one switch silently change two cost-bearing
behaviours.

Two conditions must hold for a continuation to happen: `outputContinuation` is
true **and** `max_retries > 0`. The second is pre-existing, not a new switch.

### The cost ceiling

One LLM round's output ceiling is `max_tokens` (this machine's
`intranet/deepseek-v4.1-flash` declares `32768`; the server hard limit is also
32,768). Continuation turns "one request per round" into "at most
`1 + MAX_OUTPUT_LIMIT_CONTINUATIONS` requests per round", so:

```
worst case output tokens for one LLM round
  = (1 + MAX_OUTPUT_LIMIT_CONTINUATIONS) × max_tokens
  = 3 × 32,768
  = 98,304
```

That 3× is **per LLM round**, not per run. The tool loop starts a fresh round for
each step, and each round gets its own allowance; stepping the thinking level
down lowers actual spend but does not move this declared ceiling.

Two measured points:

- **No step-down (`default` / `high`)**: one round spends `32,768` and is cut,
  with `reasoning_tokens` at 32,761–32,767 and zero visible characters. Three
  consecutive rounds looked exactly like this — i.e. continuation spends the full
  32,768 three times over.
- **Stepped down to `low`**: peak `17,701` (`reasoning_tokens` 16,801) across 18
  LLM calls with no cut (`run_07e4c985f8074ff5baba0fd9421da35a`) — about 54 % of
  the ceiling, so the same allowance buys nearly twice as much.

**Gap (do not read this as measured):** the token delta of one *real*
continuation is not measured yet — since the feature landed, no live run has hit
the limit and taken the continuation branch. Only the arithmetic bound is
available: a continuation re-bills the delivered text as input (roughly
`output_len` more input) and its own output cannot exceed `max_tokens`, so a
round's total output cannot exceed the 98,304 above. The first real continuation
should be recorded here.

## Ordering: who takes over after a cut

Two retry layers live at different depths, and **the inner one wins**. That is
the only reason this section exists — with the order unwritten, the next person
can only guess from logs.

| Step | Layer | Trigger | Action | What the layer above sees |
| --- | --- | --- | --- | --- |
| 1 | agent run loop | `detected_by == "finish_length"`, `outputContinuation` on, `max_retries > 0` | continue **inside the same run**, emit `output_stream_continuing`, at most twice; step thinking down first when the round wrote nothing | nothing — the goal plane's stream consumer ignores that event (`_ => {}`), and the run has not ended |
| 2 | agent run loop | the allowance (2) is spent and the reply is still cut | set `stream_incomplete`, record `truncation`, `agent_end` reports `incomplete` + `[OUTPUT_LIMIT]` | one `incomplete` terminal state |
| 3 | goal worker | `terminal_state == "incomplete"` | count `incomplete_streak`, inject the `CONTINUE:` note, classify `InfraRecoverable` (no repair budget spent) | the next turn's envelope |
| 4 | goal worker | `streak >= DEFAULT_MAX_INCOMPLETE_RETRIES` (3) | judge `incomplete_budget`, stop and `notify_infra_stop` the supervisor | the run ends |

The key corollary — and the reason the note's wording had to change — is that
**every goal worker turn runs with `AgentConfig::default().max_retries = 3`**
(goal sets no `max_retries` / `auto_retry` anywhere), so step 1 always happens
first. A worker can only see `incomplete` once the in-run allowance is spent.
`incomplete_continue_note` therefore must not read like "nobody has tried
continuing yet"; it now says the agent already retried the cut inside that run,
so repeating the same request unchanged will hit the same wall.

Back to the cost formula: one goal turn can spend 98,304 (three rounds) and then
be multiplied by `DEFAULT_MAX_INCOMPLETE_RETRIES` (3) `incomplete` retries — that
is the magnitude worth watching, not a single round's 3×.

Ordering assertions (`orchestration/goal/tests/agent_run_drive.rs`):

- `in_run_continuation_settles_before_the_worker_can_retry` — when an in-run
  continuation reports a single `completed` state, the worker issues **one**
  prompt, its envelope carries **no** `CONTINUE:`, the ledger holds **one**
  record with `truncation` `None`, and the todo closes on that turn. Switching
  the first mock's terminal state to `incomplete` turns it red immediately (three
  prompts).
- `incomplete_continue_note_does_not_replay_completed_work` — steps 3–4 still
  hold once the allowance is spent and the run really reports `incomplete`.
- `incomplete_continue_note_says_the_in_run_continuation_already_ran` (an
  `executor.rs` unit test) — pins the note's wording so it cannot drift back into
  reading like a first attempt.

## A goal worker's incomplete retry is not a continuation

"Continue" in a conversation and "retry" in a goal worker are different
things. The former resumes the text literally; the latter re-sends the same
task. Both share one truncation verdict, but what they carry into the next
attempt is completely different.

### What actually happens

This flow is reached **only after the in-run continuation allowance is spent**;
see the ordering table above.

1. `run_loop.rs` judges the stream truncated (`finish_length` /
   `eof_no_terminal` / …). The run ends `incomplete`, `run_terminal.error`
   records a classification code such as `[OUTPUT_LIMIT] finish_length`, and
   `truncation = {turns_so_far, output_len, tool_calls_so_far, detected_by}`
   is recorded. `output_len` is the body length of the *current* LLM turn at
   the cut, not of the whole run.
2. `console.rs` sees `terminal_state == "incomplete"`, counts with
   `incomplete_streak` (replayed from the ledger, so a restart does not reset
   it) and asks `incomplete_continue_note` for a note. A truncation spends no
   repair budget (classified `InfraRecoverable`); only its own bound applies.
3. The next turn's message is prefixed with:
   `CONTINUE: your previous turn for todo <id> ended INCOMPLETE … Pick up
   exactly where you left off — do not restart work that already completed. The
   agent already retried the cut inside that run before giving up, so repeating
   the same request unchanged will hit the same wall: narrow the step, or work
   from the artifacts already on disk.`
   After `DEFAULT_MAX_INCOMPLETE_RETRIES` (3) consecutive truncations the run
   stops as `incomplete_budget` and reports to the supervisor.

### Why it degrades into redoing the work

The note is *text*; it does not change the shape of the request. Every worker
turn is a fresh prompt carrying:

- the same todo text (`compose_turn_envelope`);
- the previous turn's `RunRecord.evidence` — the **last 4 000 bytes of the
  run's accumulated body** (`EVIDENCE_TAIL_BYTES`), truncated again to 1 200
  characters when it enters the envelope;
- goal memory / signals (failure counts, oscillation, no progress).

The previous assistant message itself — above all the truncated one — is
**not** in there. So:

- The truncated turn left a body → that tail carries it, the next turn can see
  where it got to, and the note means something.
- The truncated turn had `output_len == 0` → there is no body to leave, the
  evidence is empty, and a reasoning-only assistant message is dropped by
  `convert_to_llm`, leaving no trace in history. The next turn receives
  almost the same message as the first, the model re-derives the plan from
  scratch, and hits the same wall. Measured on one goal: three consecutive
  turns all at `completion_tokens 32768 / reasoning_tokens 32761–32767` — the
  whole output allowance eaten by thinking, zero body — with
  `{"output_len": 0, "tool_calls_so_far": 0}` in all three. That is the
  observable shape of "retry ≠ continue".

By contrast, an interactive session's "Continue" sends a **new user message**
and the truncated assistant text is still in the conversation history (so the
continuation is literal), with the host's `build_continue_prompt` adding a
summary of the last six terminal events. A worker has no such history, only
that evidence tail — which is the whole reason the same note behaves
differently on the two sides.

### What is mechanically checked, and the verdict not yet available

Regression coverage (`orchestration/goal/tests/agent_run_drive.rs`):

- `incomplete_continue_note_does_not_replay_completed_work` — after one output
  truncation, the second turn's message must carry the `CONTINUE:` note and
  the truncated turn's partial body (the information carrier for "keep
  going"), the todo must not be re-split into a new entry, and the truncated
  turn's `failure_kind` must be `infra_recoverable` (a truncation is infra, not
  a science failure).
- `incomplete_turn_without_a_body_does_not_surface_its_progress` — pins the
  current gap: with `output_len == 0` the second turn's message contains **no**
  truncation progress (`turns_so_far` / `output_len` / `tool_calls_so_far`), so
  the model cannot know how much of its allowance the interrupted attempt
  burned. This test goes red once that is fixed, and should then be rewritten
  to the new contract.

**Found along the way (not yet fixed): the repair budget is not persisted.**
`writeback` increments `failed_attempts` on the in-memory `Todo`, but no event
in the ledger carries the field — `TodoUpdated` has no `failed_attempts`,
`RunRecorded` carries only the `RunRecord`, and `replay` seeds it from
`TodoAdded`'s initial 0 with nothing to update it afterwards. So **every
replay (status / the next turn / a process restart) reads
`failed_attempts == 0`**, and that is the 0 that `decision::stall`,
`replan_obligation`, `heartbeat`'s "repair attempt N" and the
`max_validation_attempts` gate all see. Verified in the field: in
`goal_d13e9687e665`'s `events.jsonl` the field appears 4 times, always `0`;
and a constructed `HardError` turn replayed back as `0`. That is why the test
above cannot use `failed_attempts` as its witness and asserts the ledger's own
`failure_kind` instead. The fix is to land the count on an event that both
replays and the `--verify` gate can read (a `failed_attempts` field on
`TodoUpdated`, or a new `RepairAttemptConsumed`).

One real verdict is still missing: nothing machine-readable today tells
"continued" apart from "redone". The suggested shape is **overlap** — the
overlap ratio between the last N tokens before the cut and the first N tokens
of the continuation, run at both N = 64 and N = 128, with a low ratio judged a
redo. It needs only text both sides already persist, no extra instrumentation,
and it is unaffected by rewrites of the truncation verdict in `run_loop.rs`.

The automatic continuation described in the ordering section above already
covers "the allowance ran out inside one run": it steps the level down and tries
again, so a worker usually no longer has to bang against the same wall through a
retry, and it never reaches the flow in this section at all. Only after that
allowance is spent too and the run still ends `incomplete` do the measures below
apply — and by then the problem is usually no longer the allowance but the task
outgrowing one response: split it into per-batch todos (each within ~20k output)
and state in the todo text that the continuation must read existing results back
from the artifact directory.

## Diagnostics and verification

Logs record the model end reason, output byte count, thinking block count, tool
count, and the protocol-end / HTTP EOF and packet-received counts; no bodies or
thinking text are logged. Raw history cannot infer the upstream's specific end
marker from `completed`.

Regression coverage: normal stop, refusal, thinking-only, empty response,
length limit, content limit, pause, upstream cancellation, unknown reason,
ending without a terminal state, and Desktop live/history cause mapping. Model
gateways may still have undeclared protocol differences; new compatibility
rules require constructed protocol frames or redacted evidence.
