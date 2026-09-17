# Standard autonomous open-book comparison

## Primary question

How does performance change when the same model receives the same frozen
historical projection and original question, with retrieval tools available?
The model decides whether, what and how much to query. No minimum tool use is
required, and a no-tool answer is a valid outcome, not an instrumentation failure.

This supersedes the use of **closed-answer-assisted revision** as the primary
open-book comparison. Earlier runs remain intact and separately labelled:
- interface-open-v1: optional retrieval with additional guide text;
- paired-retrieval-v1: stopped instrumentation attempt, fees retained;
- paired-retrieval-v2: supplied-closed-answer retrieval/revision diagnostic,
  not standard open book.

## Exact initial-request control

Load each actual v3 closed-book API request from its saved ledger/request file.
Deep-copy that body and add ONLY:

```json
{"tools": "the assigned arm's frozen schemas", "tool_choice": "auto"}
```

Assert that removing these two fields gives the original body exactly. In
particular, messages, model, generation controls, streaming settings and output
limit are unchanged. Initial messages must be exactly system + user.

Do NOT append the prior answer, a pending-candidate list, a revision instruction,
an arm-specific system guide, a mandatory-lookup instruction or gold labels.
The tool definitions describe their functions; their schema text is part of the
intended tool-availability intervention. The model can answer immediately.

After an actual model-selected tool call, append the original tool call and
its result. At budget exhaustion only, a short exhaustion notice is permitted.
Never force `tool_choice=required`, never retry merely because no tool was used,
and never select lower-scoring cases for additional draws.

Closed outputs and gold labels are read for scoring only after generation.
Statistical pairing is by frozen question/projection identity; it is not a
continuation from the closed answer in the model's conversation.

## Arms and unchanged data limits

- C/C3: same thin history_search/history_get tools, actual Future CLI/RPC and
  isolated imported agent.db. Check imported status and expected message count
  before every paid case. No shell is exposed.
- Codex: local reproduction of its dedicated history interface. No shell/rg
  substitution and no claim of executing its inaccessible hosted service.
- OpenCode: local glob/grep/read file-tool reproduction over last known observed
  file fragments. No transcript-file invention, current-project reads or export
  enhancement. Preserve directory structure. Its limited information coverage
  remains a separate reporting issue.

Use the unchanged interface_open_tools.py implementations and schemas. This
keeps the previous local backend assumptions (including head excerpts/Unicode
character counts and file-fragment limitations) visible rather than silently
changing the experiment again.

## Budget, allocation and stopping

One new open draw for all 72 cases, all four arms. Reuse the verified v3 closed
scores; do not re-run compaction or closed answers. Arm order is randomized
within each chain/boundary using the frozen seed in the runner.

- Model: future/deepseek-flash; thinking disabled; output cap 8192.
- Same allowance across open arms: 64 logical queries and stop starting queries
  after 256KiB of returned UTF-8 bytes. Native JSON is not cut mid-response.
- Report queries, model turns, output bytes, tool errors and no-lookup cases
  separately. Do not manufacture gain by making tools mandatory.
- Aggregate authorization: CNY300 including every earlier valid, failed or
  superseded attempt. Opening spend CNY46.16260308, anchored to paired-v2's
  verified ledger/report. Reserve before sending, preserve failures/unknown
  charges, refuse silent partial-case retries. STOP halts before the next paid
  model request.

Output root: ~/compact-exp/autonomous-open-v1. Freeze code, binary, request,
projection/question hashes, schemas and request-parity receipt before execution.
No private inputs/requests/responses/projections are committed publicly.

## Evaluation and interpretation

Primary: open score minus frozen closed score, plus newly correct/lost positive
items and changes in false positives. These are comparisons of independently
answered conditions, not edits to the closed draft. Also report which cases
actually queried and the mechanically reachable substrate coverage.

Retain the existing frozen lexical scoring; do not silently remove ambiguous
items such as a quantity substring after seeing results. Document this scoring
limitation. There is one generation per condition and only three independent
real-session sources, so do not make population-significance or full-product
ranking claims. A zero-tool outcome demonstrates lack of autonomous tool use
under this prompt, not that the API cannot retrieve the missing information.

## Required checks

1. 72 initial open bodies equal their actual closed requests after removing only
   tools/tool_choice; no assistant draft or generated checklist is injected.
2. Unit tests accept no-tool answers without a forced second attempt, verify
   selected tool results enter the next request, and keep evaluation separate.
3. Final verification checks all saved requests/outputs, no required tool choice,
   closed-score pairing, native database proofs, replayable local tool traces,
   scorer arithmetic and cumulative fees. Invalid/failed calls remain visible.
