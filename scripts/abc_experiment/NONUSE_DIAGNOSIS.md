# Diagnose real-context nonuse without another full score run

Offline findings before this diagnostic: the Rust bridge forwards tools and
tool_choice unchanged; all 72 raw SSE streams in guided-v2 had zero tool-call
or legacy-function-call deltas and normal stop reasons. No parser-loss signature.
Synthetic auto-tool tests already demonstrate callable native Future history.

Use the first frozen boundary of each of the three real sessions, C3 only,
chosen by position rather than score. Keep full projection and actual native
history substrate. Predeclare 18 cells: two system-base wordings x three
question framings x three session sources.

System factor:
- original supplied-historical-record wording;
- explicitly permit visible projection and tool-returned original history.

Question framing factor:
- exact original candidate-list question;
- same candidate list/order and JSON answer contract, but explicitly ask about
  the COMPLETE original session rather than only the visible projection;
- one historical occurrence question, using the first public candidate absent
  literally from the projection. It is not selected using gold labels. This
  condition also changes language/phrasing and granularity, so it is a bundled
  diagnostic and not a pure candidate-count effect.

All conditions remain tool_choice=auto, eight tool calls maximum, same model,
thinking disabled, 8192 output cap, no prior answer or required query count.
A no-tool answer is kept. No outcome-conditioned retries or score replacements.

Root: ~/compact-exp/nonuse-diagnosis-v1. CNY2 added diagnostic cap within global
CNY300, opening cumulative cost CNY49.75429716. Preserve all calls and fees.

Report tool-use counts and inspect actual traces; do not claim a root cause
until a controlled contrast supports it. If the manipulations do not trigger
lookup, report that these candidate explanations were not demonstrated rather
than declaring the original data-source omission or model length to be proven.
The original benchmark scores remain unchanged and these 18 results are not a
new four-arm ranking.
