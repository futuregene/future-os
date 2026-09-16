# Four-arm rerun protocol (2026-09-16)

## Question and scope

Compare C3, deterministic C, the documented Codex local-inline rule, and the
OpenCode retained-tail rule on the same frozen histories. This is a **retention
rule replay**, not a benchmark of the complete upstream applications, their
native triggers, hosted retrieval, or production cache prices.

Inputs: the existing three synthetic chains and three frozen private sessions.
The private records lack media, reasoning, some tool arguments and error flags;
we preserve available fields but cannot reconstruct absent ones. Do not publish
raw records, projections, prompts, responses, or session IDs in the repository.

Independent workload units are chains (only three real sessions), not individual
values or repeated boundaries. Report per-chain counts and descriptive paired
comparisons; do not claim population significance from pooled fields.

## Frozen primary phase

- Model: `future/deepseek-flash`; no model substitution.
- Declared retention window: 128,000 tokens.
- Boundary set: synthetic stages 0/3/7; real 40%/70%/100%, rounded backwards to
  complete stored messages. All arms share exactly the same boundaries.
- Common forced compaction schedule: accumulate approximately 64K estimated new
  raw tokens, ending at complete message/tool groups where possible, and compact
  at every probe boundary. All raw records are replayed. This deliberately holds
  trigger timing constant; it does not model native automatic trigger frequency.
- Every arm carries its previous state into the next stage. C/C3 may read their
  raw journal as designed; external arms receive only their previous live state
  plus newly appended records. Summary prompts never see the questionnaire.
- C/C3 use the corrected Rust probe and the production selection functions. The
  probe is not a real agent session: no claim of byte-identical production
  prompt/cache behavior. Request-local output caps and observed error flags
  must be preserved. C3 failures remain C3 intention-to-treat results, with
  deterministic fallback explicitly reported.
- External prompts and selection rules come from `abc_external_strategies.py`
  and the pinned provenance. Whole live Codex history is summarized (not just
  text + six tools). OpenCode uses the selected head and its own bounded tool
  serialization, not an arbitrary 24-record tail. Upstream `select` returning
  no tail means the whole history is the head, not an empty summary request.
- External output caps: Codex 16,384 (explicit benchmark cap), OpenCode 4,096.
  Probe answers: 8,192. Direct bridge requests disable reasoning; C3 follows its
  model client configuration. Record this difference; do not call this a
  pure summary-text-only causal ablation.
- Primary outcome: recognition hits and false positives on a frozen exact-value
  questionnaire, plus mechanical exact containment. Deduplicate values and
  validate every positive/decoy against the complete covered archive.
- One generation per arm/boundary. Randomize arm execution order within each
  chronological chain/boundary block with seed 20260916. No repeated-score
  cherry-picking or automatic resampling of empty/invalid answers; count and
  report invalid answers separately.

## Integrity and accounting

Use a new output directory outside the repository, never overwrite the old
experiment. Persist input, executable, source, prompt, projection, prior-state,
question and response hashes. A changed manifest is an error, not a cache hit.
Every provider call (including Rust-internal retry allowance) is reserved before
sending; one exclusive runner owns the shared ledger. Failed/unsettled calls
retain their reservation and cannot silently retry. Driver retries are bounded
to three. Cache hits do not spend again. Unknown charges retain the reservation.

Initial admission budget: CNY 20. Reservation prices are deliberately conservative
assumptions (5/M input, 20/M output), not a provider-side spending guarantee. Stop
if a reported charge exceeds its reservation or no remaining admission fits.
Record actual provider credit costs separately from reserved/unknown amounts.

## Secondary phases

Only after the primary projections are frozen:

1. Open book: reuse those exact projection hashes. Give **all four arms the same
   archive interface, same covered records, same total UTF-8 byte and tool-call
   budgets**. Separate model turns, actual tool invocations and bytes delivered.
   This isolates retention-plus-common-retrieval, not native product interfaces.
2. Pressure/summary value: define a fixed target set before examining the C3
   output (e.g. values missing from pure C), and score both arms against it.
   A value rescued by the C3 summary must count as a rescue, not disappear from
   C3's denominator. No inference from recovery of values absent from each
   arm's own final projection.

Secondary work requires remaining budget. Incomplete phases stay explicitly
incomplete; a partial table cannot be presented as a completed rerun.

## Commands

```sh
cargo build -p future-agent --example abc_c3_probe --example abc_probe_bridge
python3 -m unittest discover -s scripts/abc_experiment -p 'test_four_arm_rerun.py' -v
python3 scripts/abc_experiment/four_arm_rerun.py \
  --source ~/compact-exp --output ~/compact-exp/rerun-20260916-v1 \
  --driver target/debug/examples/abc_c3_probe \
  --bridge target/debug/examples/abc_probe_bridge --budget 20 --prepare-only
# Remove --prepare-only to execute; use --report-only to aggregate without calls.
```
