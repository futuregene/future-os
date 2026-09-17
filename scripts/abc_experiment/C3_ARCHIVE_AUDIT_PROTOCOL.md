# Stronger C3 prompt-only archive review

User requested stronger prompt guidance aiming for lookup across all sessions.
Compare the previous enhanced C3 prompt with that SAME prompt plus a stricter
archive-review paragraph. Both conditions are newly sampled on all 18 C3 cases
(36 independent conversations); randomized within pair, seed 19379. Do not
select only prior non-call/low-score cases or repeat failures to reach 18/18.

The new prompt distinguishes confirmed, searched-but-unconfirmed, and not-yet-
checked candidates. When tools/allowance remain, it tells the model to resolve
not-yet-checked historical items through targeted searches before finalizing.
The model identifies those items itself; no external checklist or gold label
is supplied. It recommends literal limit=1 queries, parallel independent calls,
reads only when snippets are insufficient, and no duplicate settled lookups.

This is explicitly a stronger instructional obligation to review unresolved
history, not an unprompted spontaneous behavior claim. The API remains auto;
there is no required choice, hard minimum-call gate, or code-level per-candidate
verification loop. Current question, projection, candidates, tools and generation
settings remain unchanged; neither condition sees a previous answer.

Keep future/deepseek-flash, thinking off, 8192 output cap, 64 logical queries and
256KiB return stop. Retain no-call outcomes. Track calls/triggered cases, hits,
false positives and real-session results; more calls alone is not success.

Root: ~/compact-exp/c3-archive-audit-v1. Opening cumulative CNY53.21163276.
Additional cap CNY3 within global CNY300, reserve before requests, STOP before
another request, no silent partial-case retries. Freeze before execution.
Do not deploy to production or publish a new four-arm ranking from this tuning
run. Existing inspected cases are not a new held-out generalization sample.
