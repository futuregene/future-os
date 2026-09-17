# Full rerun with the new conditional Future recall policy

User authorization: rerun the new strategy on the original frozen open-book
exam, not another synthetic calibration.

## Conditions fixed before execution

- All 72 cases: same v3 projections, original questions, closed scores, model
  future/deepseek-flash, thinking disabled and 8192 output limit.
- Autonomous open book: no closed answer in model context, no generated pending
  list, no required tool choice, no minimum tool use and no no-tool retries.
- Same interface implementations, schemas and per-case 64-query/256KiB stop.
  C/C3 use real Future history CLI/RPC in imported isolated Agent databases;
  Codex uses its dedicated history interface's local reproduction; OpenCode
  uses the same file-observation reproduction. No shell/export substitution.
- C/C3 receive the updated source-derived Rust recall guidance: retrieved
  original records are permitted evidence; visible answers need not be queried;
  missing exact history should be searched before reporting it unknown.
  The tools retain their existing CLI-to-thin-adapter routing mapping.
- Codex and OpenCode source-derived guidance is unchanged from guided-v1. The
  rerun includes all arms, not just an improved or previously weak subset.
- Do not recompile the frozen compaction/bridge/query binaries: the experiment
  extracts the updated Rust prompt source and otherwise reuses the same stored
  projections and query backend. This is a prompt-policy test, not a new
  compaction or full native-client test.

## Freeze and evaluation

Output root: ~/compact-exp/guided-autonomous-open-v2.
Freeze the exact generated guidance, source hashes, binary hashes and every
initial request. Check that user question/projection and generation settings
are unchanged and only the recorded system guidance plus tools/auto differ
from the closed request.

Evaluate open-vs-closed scores after generation. Also compare tool usage and
scores descriptively against guided-autonomous-open-v1 (old recall policy) and
autonomous-open-v1 (no recall guidance). Preserve local-backend and observed-file
coverage limits, lexical/numeric-substring scoring caveats, single-draw limits,
and the distinction between question-level pairing and supplying old answers.
Zero calls are a valid outcome; do not add stronger prompts or rerun low scores
mid-experiment. No claims of production reliability or statistical significance.

## Budget chain

Total authorization remains CNY300. Opening cumulative spend is
CNY48.67444064, including all valid, failed, superseded and diagnostic phases.
The comparison runner's --prior is autonomous-open-v1 to retain its case map.
The --spent-after ledgers, in order, are:

1. guided-autonomous-open-v1
2. history-trigger-calibration-v2
3. history-trigger-stress-v1

Require each opening balance to equal the preceding cumulative balance,
verified completion, settled ledger entries, and matching closing totals.
Reject duplicate/discontinuous ledgers. The calibration-v1 launcher failed
before a model request and has no fee or ledger to add.

Reserve before paid requests, keep unknown charges reserved, use STOP before
another model request when needed, and never silently retry a partial case.
