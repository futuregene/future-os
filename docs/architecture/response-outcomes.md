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
