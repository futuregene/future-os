# Held-out long-context history-trigger calibration

This follows the short synthetic calibration; it is not another 72-case product
benchmark and does not change the production Rust prompt yet.

## Fixed design before calls

Three NEW synthetic facts (tag, timeout and artifact path), crossed with:
- short versus long visible context;
- original source-derived recall hint versus that hint plus the previously tested
  conditional lookup-before-unknown policy;
- exact answer hidden versus visible near the beginning of the projection.

This yields 24 cells, plus two long-context absent-history controls (one per
policy), total 26. The same underlying original Agent archive is used throughout.
Policy pairs have identical user messages; length pairs have the same final
natural question and fact availability. Order is randomized with a fixed seed.

Long projections contain at least 60,000 characters of diverse synthetic but
irrelevant engineering notes; report actual provider prompt_tokens rather than
calling this a precise token count. They are not real production conversations.
Expected hidden values never enter the initial messages or tool definitions.
Visible controls intentionally expose the value and measure redundant lookup.

Every case uses real isolated Future history CLI/RPC/database, the same two
query tools, future/deepseek-flash, thinking off, 8192 output cap and AUTO tool
choice. At most eight tool calls per cell. No required, no prior answer, no
candidate checklist, no outcome-conditioned retry. Verify retrieval evidence
AND final answer for hidden-value successes. Absence and no-redundancy behavior
must be reported even when inconvenient.

## Budget and limits

Root: ~/compact-exp/history-trigger-stress-v1.
Opening cumulative spend: CNY48.24603392. This experiment shares the original
calibration's CNY2 ceiling (absolute CNY50.19025064), rather than resetting a new
CNY2 allowance; the global CNY300 cap also remains. Reserve before sending and
retain failures/unknown charges. STOP prevents starting a further request.

Freeze the matrix, fresh facts, archive, messages, code and binaries before
execution. Do not commit private requests/responses publicly. One draw per cell
and three fact fixtures give diagnostic counts, not a production reliability
estimate or statistically independent repeated observations.

Use these results to decide whether the policy deserves a production-shaped
validation. Do not automatically claim that success with thin query adapters
proves the native shell/CLI product behavior or that synthetic long contexts
match realistic C3 summaries.
