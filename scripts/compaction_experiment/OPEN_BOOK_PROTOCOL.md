# Open-book protocol

The design behind `run_open_book.py`. For the commands, see
[README.md](README.md#open-book); for the results, see
[compaction-open-book-experiment.md](../../docs/internals/compaction/compaction-open-book-experiment.md).

## The question

Closed book asks what a projection retains. Open book asks whether the model, given the same
projection *and* the ability to search the original history, recovers what the projection
dropped. Those are different claims and the second is the one production rests on: after
compaction the archive is still there, reachable through the ordinary shell tool.

## What is held identical to closed book

* the projection under test — the same frozen file the closed arm was scored on, asserted by
  SHA before the case runs;
* the questionnaire and its scoring (`exam.py`), so a score here is comparable with one there;
* the model and generation settings;
* the archive contents — the same records, reconstructed into an isolated Agent database.

## What changes

| | closed book | open book |
|---|---|---|
| tool definitions | sent, never invoked | sent, and the model may call them |
| tool execution | none | production handlers, via `production_tool_executor` |
| follow-up turns | none | the tool loop, to a budget |
| system prompt | the session's own | the same — production appends nothing |

The system prompt is deliberately identical in both. An earlier version of this exam appended
a recall guidance describing the history CLI; the runtime no longer does, and the exam asserts
the prompt is unmodified so it cannot drift back without the run failing.

The session id is likewise **not** in the prompt. A model that wants the archive has to name
the session itself, or read it out of the tool definitions' usage; that is what a session
faces too.

## The retrieval budget

Two limits bound a case, both generous relative to what a real answer needs:

* **`TOOL_LIMIT` = 24 executions.** When the allowance is spent the harness withdraws the tool
  definitions and appends a notice to answer from what is available, rather than letting the
  model call into a wall until the turn cap.
* **`BYTE_STOP` = 256 KiB of returned output.** Native outputs are never re-truncated, so a
  single large result can exhaust the budget.

Every call is recorded with its arguments, status, output size, native exit code and duration.
`status` distinguishes `native` (reached a production handler), `denied` (refused by the
budget or by the harness's path boundary) and `error` (the handler or the sandbox failed).
Per-case attribution of recovered values relies on these traces, because a value the model
reports must be traceable to a call that produced it.

## Flags that change the question

These are not interchangeable, and a report that omits which was used is not interpretable:

* **`--require-retrieval`** adds a verification instruction to the question and forces the
  first tool call. **This is not a production behaviour.** It measures the upper bound of what
  is retrievable, not what a session does. Every number that depends on it is labelled.
* **`--exam-wording v2`** removes three things from the stored question that presuppose the
  projection is the whole story: "the conversation above **is the record**" (against the
  runtime's own "partial projection"), asking only about values that "appeared **in that
  conversation**", and "if you cannot check a value, leave it out". The candidate list and its
  order are preserved verbatim, so the two wordings differ only in framing.
* **`--closed-reprobe`** re-scores the frozen closed projections under the current wording with
  no tools. Run it whenever the wording changes: a wording that made the instrument easier
  would move the closed arm too, and without this control a dropped open score cannot be
  distinguished from a harder question.
* **`--retry-unsettled`** explicitly retries a call that an operator abort left in `started`,
  keeping the original reservation as spend and logging it. The ledger refuses to retry
  silently, and that stays the default: settlement of an interrupted call is unknown.

## Isolation

This is the part to read before trusting a number.

**The harness bounds the file tools.** Production's `read`/`write`/`edit` resolve an absolute
path as-is and never consult the rule-based denials, so a replay that offers production's whole
tool set would be able to read sibling fixtures and the operator's files.
`production_tool_executor` therefore takes `allowed_roots` and refuses any path outside them,
deny-by-default. That boundary is harness code layered on production handlers, not a production
guarantee, and the protocol's claim is exactly that.

**Nothing bounds `shell` to the case.** The OS sandbox limits what a shell command may do, but a
command can still name a path outside the workspace. In the recorded run, 18 shell calls named
a path outside the case and 15 returned non-trivial output from the operator's own filesystem.
None of it reached an answer — every recovered value was attributed to a `future session
history` query — but a replay that reads the host can be contaminated by it, and a study
holding private session data cannot rest on the tool layer alone. Check `traces/` before
quoting a result, and prefer an isolated HOME and a fresh port.

## Comparing arms

`--arm` selects one arm; the run writes per-case results under `<output>/results/` and leaves
finished cases in place, so an interrupted run resumes without repeating paid calls. To compare
two arms, run each into its own output directory against the same `--closed` root; the closed
scores come from that root, so both are measured against the identical baseline.

## Limits to state alongside any result

* **One draw per case.** 18 cases and 178 values: a few points is one flip.
* **Recognition, not continuation.** The questions ask for exact values, which favours verbatim
  retention over summarisation.
* **Retrieval proven, not optimised.** The recorded run issued 100 calls and hit the output
  budget; no attempt was made to find a better query strategy.
* **The required variant is an upper bound.** It is not reachable in production.
