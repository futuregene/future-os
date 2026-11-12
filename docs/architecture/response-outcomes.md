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
| Single-output limit | the output-limit divider; not conflated with context compaction |
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

## A goal worker's incomplete retry is not a continuation

"Continue" in a conversation and "retry" in a goal worker are different
things. The former resumes the text literally; the latter re-sends the same
task. Both share one truncation verdict, but what they carry into the next
attempt is completely different.

### What actually happens

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
   exactly where you left off — do not restart work that already completed.`
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

Until that verdict exists, a truncation with `output_len == 0` should be
handled as follows rather than by re-running `future goal run` repeatedly:
lower the thinking level (`--thinking-level low` / `off`); split the large task
into per-batch todos (each within ~20k output); and state in the todo text that
the continuation must read existing results back from the artifact directory.

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
