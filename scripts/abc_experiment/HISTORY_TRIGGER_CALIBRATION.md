# Synthetic history-search trigger calibration

Purpose: find a practical, conditional way to elicit real historical lookup
without tool_choice=required, prior answers, candidate-checking gates, or
another full benchmark. This is behavioral calibration, not a retention score.

Predeclare 12 independent cells:
- Three hidden facts, each under recognition-style question, natural exact
  recall, and natural recall plus a lookup-before-unknown policy (9 cells).
- Visible-answer positive control: answer directly; no redundant lookup needed.
- No-tool control: hidden value should remain unknown rather than guessed.
- Absent-from-archive control: search may return no evidence; do not invent a value.

Use only synthetic, non-sensitive release tags, timeout values and artifact paths.
Hidden natural-recall answers must not appear in system/user input or schemas;
they exist only in an actual isolated Agent database. Recognition cells retain
a public answer-choice list to diagnose the old task framing, not to validate
hidden-value recovery. A success on hidden cells requires BOTH a native query
returning the hidden value as source evidence and the final answer using it.

The additional policy is explicitly stronger than the original conditional hint:
visible context is partial; history results are permitted evidence; if the exact
past decision is missing, consult history before concluding unknown; if visible,
answer directly; if lookup fails or has no evidence, acknowledge uncertainty.
It does not force every question to call a tool, but it does establish a
lookup-before-abstention decision rule. Do not label this identical to the
previous purely optional prompt condition.

Use future/deepseek-flash, thinking off, 8192 output cap and tool_choice=auto.
At most eight tool requests per cell. The control with no tools gets no tool
schemas. One draw per predeclared cell; no outcome-conditioned repetitions.

New root: ~/compact-exp/history-trigger-calibration-v1. Additional spend cap
CNY2 within the total CNY300, opening cumulative spend CNY48.19025064. Reserve
before requests; STOP prevents starting another request. Retain failures/fees.
No full 72-case rerun and no model change without a separate decision.

Freeze fixture, allocation, input messages, source/binary hashes before paid
calls. Verify database import, actual auto-tool requests and actual returned
source content. Report known-answer and absent-answer controls separately and
avoid claiming reliability from one draw per cell.
