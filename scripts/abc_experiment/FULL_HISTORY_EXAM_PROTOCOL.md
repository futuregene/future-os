# Matched closed/open experiment with corrected history scope

## What changes and why

The old question referred to "the conversation above" although the grader's
truth set covered the complete original session. Real-context diagnostics
showed that this framing suppresses autonomous lookup. The new question
explicitly targets the COMPLETE original session, labels the supplied text a
compressed projection, and permits original records returned by tools when
those tools are available.

Both closed and open conditions get the exact SAME new question, candidates,
ordering and frozen projection. Generate new answers for BOTH conditions;
do not subtract old-question closed scores from new-question open scores.
Statistical pairing is after generation, never a prior-answer message.

## Matrix and controls

- Same six chains, three boundaries, four projection strategies: 72 pairs.
- Each pair has one new closed and one new open answer: 144 condition outputs.
- No re-running compaction or changing frozen v3 projections, candidate labels
  or candidate order. Keep existing lexical grading, including its documented
  numeric-substring ambiguity; do not selectively relabel items after results.
- Base system is the same in closed/open and explicitly permits available
  evidence. The question conditionally refers to tools, so closed does not
  pretend tools exist.
- Closed: no tools, no recall-tool guide, one model response.
- Open: append the current source-derived functional recall guide, add the
  unchanged arm's tools and tool_choice=auto. No required, no pending list,
  no previous answer. Accept a direct answer without forcing a retry.
- Open C/C3 share the native Future CLI/RPC database backend and routing-aware
  Rust-derived guide. Codex remains the dedicated history-interface local
  reproduction; OpenCode remains the observed-file local reproduction. No
  shell/export substitution or new data is added.
- Model future/deepseek-flash, thinking disabled, 8192 output cap. Open budget
  remains 64 logical queries and stop starting queries after 256KiB returned
  UTF-8 bytes. Native JSON is not cut mid-response.

Within each chain/boundary randomize the eight arm/condition runs using fixed
seed 19331. Open can run before closed; the inference function receives neither
closed output nor gold labels. All conditions have separate conversations,
case state and artifacts. This prevents carryover from a draft answer.

## Data and audit

Native Future cases require actual imported agent.db status and matching
normalized message counts. File contents come solely from the frozen observed
reads, retaining the existing fragment/invalidation policy. Record coverage
ceilings separately: failure to recover unavailable file data is not evidence
of weak file-search implementation. No reading today's project to fill gaps.

Freeze all 144 initial request bodies, new questions, checkpoint gates/guides,
code/source/binary hashes and allocation before sending model requests. Assert
for each pair that the only open differences are the recorded system guide,
tools and auto, while user text and generation settings are identical.

Recompute scores from saved outputs. Pair gained/lost/net and false positives
ONLY after both independently generated answers exist. Report raw closed/open
scores, autonomous tool-use cases, query counts, returned bytes, errors and
coverage limits. Do not overwrite earlier experiment results or make a full
native-product/significance claim from three real-session sources and one draw
per condition.

## Budget and operation

Root: ~/compact-exp/full-history-closed-open-v1.
Opening cumulative spend CNY50.12897400, including all earlier runs and latest
nonuse diagnostics. Global authorization CNY300, reserve-before-send, preserve
failures/unknown costs, no silent partial-case retries. STOP halts before the
next model request. No outcome-conditioned protocol changes or extra draws.
