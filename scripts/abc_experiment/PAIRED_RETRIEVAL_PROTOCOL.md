# Paired closed-book + retrieval uplift

## Target estimand

For each frozen chain/boundary/strategy, retain the exact v3 closed answer as
stage 1. Stage 2 revises THAT answer after actual tool retrieval. Primary output:

- newly correct original positive candidates (gained);
- previously correct candidates omitted after revision (lost);
- net uplift = gained - lost = revised hits - frozen closed hits;
- newly introduced and corrected false positives, separately;
- actual queries, returned bytes, model calls and added costs.

This is descriptive **retrieval-plus-revision pipeline uplift**, not an isolated
causal effect of retrieval versus another reasoning opportunity. No fresh
closed draw or extra no-tool revision control is added. Boundaries/items are
not independent replicates; only three real-session sources are available.

## Mandatory second stage without answer leakage

Read the candidate list ONLY from the public question's `Values:` section.
The pending list is every public candidate not literally present in the
projection. Never select by gold labels or by which earlier answers were wrong.
Gold present/decoy sets are used only by the offline scorer and assertions.

Include the prior closed answer as a draft, not an answer key. Already supported
projection facts need not be reloaded. While pending items remain, requests use
`tool_choice=required`. One arbitrary tool invocation is insufficient:

- an exact literal history search is a recorded attempt for that candidate;
- an escaped-literal file grep is an attempt;
- actual original snippets/chunks/observed-file contents containing another
  candidate can establish it; query echoes/error strings are not evidence;
- an attempt, especially within one window or incomplete files, is not proof
  of global absence. The final model still must interpret the returned evidence.

The model chooses order and reads/pagination. Start history presence checks with
limit=1; raise it or read context when needed. All interfaces and lookup data
policies remain those of `INTERFACE_OPEN_PROTOCOL.md` (native Future queries,
local Codex dedicated history replica, local OpenCode observed-file replica).
No shell/rg replacement for Codex history, no OpenCode export enhancement.

Shared allowance: 64 logical requests per case, stop initiating requests after
256KiB returned UTF-8 bytes; responses are not cut mid-native-JSON. This is a new
frozen condition, not a modification of interface-v1's budget. If the allowance
ends with pending candidates, record them explicitly and do not call them
verified. If the provider twice returns a final answer despite required tools,
stop instead of accepting a sham retrieval measurement.

## Preservation and budget

Use the same 72 cases, v3 projection/question hashes, model, disabled thinking,
8192 output cap and original baseline answers. Randomize arm order within each
chain/boundary using a frozen seed. No low-score-only reruns and no replacing
prior results. Each C/C3 case verifies imported agent.db and expected entry count.

New root: `~/compact-exp/paired-retrieval-v1`. Opening spend is ¥44.92906856,
including every earlier run. Total authorization remains ¥300; reserve before
requests and retain failure costs. No additional models or LLM workers.

Freeze source/binary hashes and baselines/pending lists before execution.
Tests cover label-independent selection, echo exclusion, required-tool flow,
paired gain/loss arithmetic and the existing interface behavior. Verify actual
requests include tools/tool_choice, replay tool traces, recompute pending sets
and recompute paired deltas from saved model responses before reporting.
