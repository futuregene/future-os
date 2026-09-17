# C3 query-priority optimization

Goal: increase meaningful historical lookup when the complete-history task has
unresolved facts, not inflate counts with duplicates. Preserve autonomous tool
choice and inspect answer quality alongside usage.

All 18 C3 cases from full-history-closed-open-v1 are included, not only low
scores or no-tool cases. Each gets a freshly generated control and enhanced
policy answer, randomly ordered within pair (seed 19357), for 36 independent
conversations. The control is the current open-book system/guide. The variant
only appends Complete-history evidence discipline to the system message.
Question, projection, candidates, output format, tool schemas and native
Future CLI/RPC/data are identical. Both are open book; this is not a new
four-arm ranking or a closed-answer revision experiment.

The new condition raises completeness and evidence-provenance priority:
summary/index silence is not negative evidence; do not turn a whole-history
question into a visible-only list; check unresolved values before omitting them
merely for absence from the projection. Keep already supported facts, use
snippets before full reads, stop when evidence suffices, and do not repeat
settled queries. This is an explicitly stronger conditional prompt policy,
NOT tool_choice=required and NOT a hard minimum-call or per-candidate gate.
The model chooses its queries; no generated pending list or old answer is fed.

Use future/deepseek-flash, thinking disabled, 8192 output cap, existing 64-query
and 256KiB return-stop allowance. One draw for every predeclared cell; preserve
failures/no-tool answers and do not silently retry partial cases.

Measure tool-use cases (especially the nine real-session boundaries), actual
queries, positive hits, false positives, bytes and cost. Success requires more
appropriate lookup without sacrificing accuracy/precision; a larger raw count
alone is insufficient. Old main scores remain unchanged, and any result on
these already-inspected cases is a tuning result rather than held-out evidence.

Root: ~/compact-exp/c3-query-priority-v1. Opening cumulative CNY52.48767224,
additional optimization cap CNY3 within the global CNY300. Source/code/input/
binary hashes frozen before calls. No production Rust prompt change until
results are audited. The prior interrupted file write executed nothing and
started no model calls.
