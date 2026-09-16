# Legacy A semantic calls and prompts

**Strategy C needs no summary model, and deterministic C makes no summary calls.
The runtime default is C3 — C plus a model-written handoff summary — so the default
path does issue one summary request per compaction.** This reference covers that
shared prompt construction plus the retained explicit legacy A APIs, and is not a
description of what the default sends. See [C compaction](compaction.md) and the
[measurement of what the summary contributes](compaction-abc-experiment.md).

`summary_prompt` and `call_summary_model_with_messages` are **shared with the C3
default path**; `summarize_fold`, `serialize_message` and
`call_summary_model_bounded` belong to the retained legacy A path alone. The
authoritative implementations are
[semantic.rs](../agent/src/compaction/semantic.rs) and
[semantic/evidence.rs](../agent/src/compaction/semantic/evidence.rs). The [Chinese companion](compaction-prompts.zh-CN.md)
also reproduces the complete output-template constant verbatim.

## How many calls?

- A cache hit, no work, or an untriggered automatic check needs **zero** summary calls.
- A one-chunk successful summary needs **one** API request.
- A successful K-chunk fold needs **K** requests, with each output becoming the
  next chunk's prior-summary accumulator.
- A chunk can have up to two additional transient/incomplete-response retries.
- Context/length errors can trigger one stricter folding pass, whose chunk count
  may differ. There is no universal one-call or three-call total per operation.

SSE chunks are stream fragments, not distinct model calls. The user CLI returns
an ACK, not a final call count. Ordinary answering and history-QA requests must
not be counted as summary requests.

Explicit legacy semantic calls use the supplied provider/model with tools disabled.
The C3 default instead sends the live conversation as real messages with the agent's own
system prompt and tool definitions, which is what makes its request servable from the
provider's prefix cache — see [C compaction](compaction.md). Summary text is limited
to about 4096 estimated tokens; the request-local generation cap is at most 8192,
scaled for small windows and clamped to the model limit. Ordinary chat output
settings are not changed.

## Exact system prompt

```text
You are a context summarization agent. Produce a structured handoff summary so another coding agent can continue the work. Do not continue the conversation or answer its questions. Output only the requested structure, using the conversation's primary language.
Evidence completeness: tool results may be partial excerpts. Describe only what the visible excerpt establishes; omitted content remains unknown. Never infer that the full result contains no relevant data, no errors, or only filler because its middle is omitted. Preserve this qualification and the history entry reference. A successful tool execution is not proof that all requested validation passed.
```

This is not the ordinary chat system prompt. The entire project/system context,
tool definitions and post-compaction recall guide are not copied into this request.

## User-message assembly

Each summary `ModelRequest` has the dedicated system prompt, **one user text
message**, and an empty tools list. Protocol adapters map this logical request to
the provider's native fields. The user text contains, in order:

1. Optional carry-forward instructions and `<prior-summary>...</prior-summary>`:
   the old checkpoint summary or the previous fold output; newer evidence wins
   conflicts and completed/resolved work must be updated.
2. `<conversation>...</conversation>`: this chunk of the covered history,
   serialized with role labels and `[History entry <entry_id>]` references.
3. Optional `Additional user instructions for this compaction:` text from the
   user's `--instructions` option.
4. The fixed Markdown output template and its Rules.
5. The S2 retention/budget suffix below.

The recent retained tail is not itself the covered input for this operation.
Protected originals inside the covered range may still be supplied as background.

### Serialized content

- User, assistant and system text: role label plus text.
- Tool calls: call ID, name and JSON arguments.
- Tool results: result/error label, associated call ID and bounded head/tail text
  (2000 characters normally, 512 in strict mode, plus omission markers).
- Reasoning text: labeled, limited to 2000 characters normally or 512 in strict mode.
- Images: textual placeholder or non-data URL reference, not embedded image bytes.
- Hidden provider metadata is not serialized wholesale.

The protected S2 region excluding thinking does **not** imply that summary input
contains no reasoning text. The serializer includes a limited reasoning excerpt.
Ordinary text/tool arguments are split by chunk budgets rather than by the above
tool-result character cap. Characters and tokens are different units.

## Output template and suffix

Required headings are `Objective`, `Important Details`, `Work State` (with
`Completed`, `Active`, `Blocked`), `Next Move`, and `Relevant Files`. Empty sections
remain. Rules request terse bullets, exact known symbols/paths/errors/identifiers,
current rather than superseded state, and no discussion of the summarization
process. See `SUMMARY_TEMPLATE` for the exact placeholders and Rules.

The final suffix is (N is the calculated summary text budget):

```text
S2 retention: original user directives and selected assistant text are preserved separately. Older assistant outputs may be summarized to fit; carry forward their important facts. Prioritize tool evidence, exact symbols/values, corrections and verification boundaries. Keep canonical headings exactly as shown; write the body in the conversation language. Keep the summary within N estimated tokens.
```

When changing prompts, excerpts or budgets, review the idempotency fingerprint,
structure checks, chunk admission and this reference together. Count actual API
requests/usage, including retries and failures; do not infer charges from stream
fragment counts or checkpoint counts.
