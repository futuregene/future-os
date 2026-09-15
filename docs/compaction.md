# S2 compaction and historical recall

The production Agent uses a journal-preserving S2 projection. It does not rewrite
or delete the original conversation when compacting. No new model tools are
introduced; [history recall](session-history.md) uses the existing shell tool.

## Trigger and request admission

The economic trigger is `min(floor(context_window * 0.8), 256000)` input tokens.
Reaching it triggers preparation. For example, a 1M model triggers at 256K;
a 128K model at 102400. Model switching only changes configuration; the check
runs at the actual model-request boundary, including mid-run catalog changes.

Admission also accounts for the configured maximum output, system prompt, tool
schemas, message framing and conservative image/reasoning estimates. With window
`W`, maximum output `O`, and safety margin `min(2048, W/16)`, input cannot exceed
`W - O - margin`. This can trigger compaction before the economic threshold.
Provider-reported context usage is used conservatively alongside local estimates.
These are estimates, not a provider-independent exact tokenizer; provider-limit
recovery remains necessary.

A lone unseen user input is not replaced with a lossy summary merely because it
crosses the economic trigger. It may proceed if it still fits the hard request
budget. If system/tools/output alone consume the window, or user text cannot fit,
the Agent fails explicitly rather than silently deleting directives.

## What S2 retains

The projected context has three parts:

1. Original user text and selected assistant text from the covered prefix.
2. A structured state summary, including important tool evidence and next steps.
3. The recent, tool-pair-safe tail.

All user text is protected, including the first question. Assistant text is kept
verbatim while it fits. Reasoning, media bodies, tool calls and provider metadata
are not copied into the protected text blocks. Their stored originals remain in
the journal; existing upstream/tool-level truncation cannot be reversed.

The history target is **32K tokens**, excluding system/tool-definition overhead.
The recent-tail goal is independently capped at 8K (smaller on small windows).
The target may expand to 64K to retain originals, but must still leave headroom in
the actual request budget. If it cannot hold all assistant outputs, the newest
ones that fit stay verbatim; other outputs are summarized, with an explicit
retention notice. If protected user text plus required tail/summary still cannot
fit, preparation fails before calling a summary model. Split large input material
into referenced files or start a new session instead of silently losing it.

Manual compaction bypasses the economic threshold, not budget validation. A
manual checkpoint on a short dialogue can be larger because it retains originals
and adds state. Automatic/provider-limit compaction must make token progress.
Repeated compaction with no uncovered content is a no-op. Manual compaction is
rejected while a run is active; automatic mid-run compaction stays in that run.

## Summary requests

- Use the selected provider/model with tools disabled; do not choose a hidden model.
- Apply a request-local output cap of at most 8192, scaled down for small windows
  and clamped to the model limit. The ordinary request's output configuration is
  unchanged. The summary text budget is at most 4096 estimated tokens.
- Budget instructions, template, prior accumulator, wrappers, output and margin
  before selecting each chunk. Large input is folded in bounded chunks.
- Tool excerpts retain both head and tail; serialized inputs include true history
  entry IDs so useful references can survive summarization. An omitted middle
  remains unknown: absence in an excerpt must not become a claim that the full
  result contains no relevant data/errors or only filler. Successful execution
  is not proof that every requested validation passed.
- Reject incomplete/oversized/structurally invalid summaries. Cancellation is
  checked while connecting and waiting for stream events. Retries are bounded.
- Only provider-context-limit recovery may use the deterministic emergency path;
  it still has to satisfy final budget/progress checks.
- Account the final reported usage once per attempt, including retries and usage
  arriving after a finish frame. Summary accounting does not overwrite the
  conversation's `last_prompt_tokens`. Manual summary totals are persisted too.
  Missing usage must not be interpreted as proof of a free request; existing
  pricing estimates remain estimates when no authoritative cost is supplied.

A checkpoint is committed only after successful preparation and persistence.
The next normal request then receives one history-recall guide, provided the
session is persisted and shell is enabled/permitted. The guide is not stored as
chat text and is not added to summary requests.

## Persistence, restore and upgrades

S2 checkpoints use **schema 3** and store `protected_entry_ids`, not copied message
bodies. Restore reconstructs protected text from original entries. Fork remaps
all range and protected references. Dangling/out-of-range references are rejected;
checkpoint coverage cannot move behind an already represented checkpoint.

Schema 2 and legacy records remain readable. For old semantic checkpoints whose
covered prefix is still stored, projection recovers the original user/assistant
text before the next S2 checkpoint, with normal request-budget checks. Already
discarded legacy history cannot be reconstructed. Older Agents do not understand
schema 3 and should not be used for S2 sessions; update Agent and CLI together.

## Validation

Scoped Rust tests cover threshold boundaries, overhead/output reservation,
protected originals, bounded output demotion, restore/fork, large lines, invalid
references, cancellation, retry accounting, and the actual HTTP output-cap field.

For an isolated executable-level smoke test:

```sh
cargo build -p future-cli --bin future
python3 scripts/test_s2_compaction.py --binary target/debug/future --report target/s2-smoke.json
```

On Windows, pass `target/debug/future.exe`. If `CARGO_TARGET_DIR` is set, use that
binary path. The test starts only its own Agent with a fresh HOME and TCP port,
uses >256K of synthetic tool-heavy history and a local HTTP model stub, exercises
shell search/get, restarts, checks preserved originals and usage, then cleans up.
It makes no external model calls. This validates the execution mechanism, not a
claim that every real model will autonomously retrieve evidence or preserve all
semantics perfectly.
