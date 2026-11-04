# Fusion evaluation and reporting protocol

## Two purposes

1. Improve the shipping Fusion protocol by inspecting errors at each stage.
2. Produce reproducible comparisons that can be shared publicly.

Development evidence and public evaluation evidence have different roles. A sample
used to tune prompts or change Fusion is development data thereafter. Record the
change, then evaluate on a previously frozen holdout before making a broad claim.

## Frozen comparison

The first comparison is the local saved Fusion configuration against
`future/deepseek-flash` at `max`, confirmed by the user on 2026-11-04.
The local catalogue does not support `xhigh` for this model. Never silently replace
an unavailable binding with a default effort, another model, or another participant.

Keep question wording, supplied evidence, output schema and scorer fixed. Use a
fresh session and empty workspace. Disable ordinary tools; Fusion itself is the
only permitted outer tool. Empty `read_scope` removes inner file tools. Inspect
the actual transcripts, rather than trusting a prompt that says tools are disabled.

The same DeepSeek Flash max model handles the ordinary Fusion-enabled request
and the single-model request. Both use the product's default system prompt with
project-context files disabled for this closed-text task. Enable Fusion with the
normal `/fusion` prefix. Keep automatic tool selection; do not force a tool call,
require an exact consultation count, or add benchmark-only routing instructions.

The daily profile's primary endpoint is the ordinary final answer. Report Fusion
invocation count, completed consultations and correctness with a completed
consultation separately. A correct answer without a consultation is not evidence
of Fusion-assisted reasoning. Preserve all consultations and failures; do not
resubmit a question or abort a natural response merely to make the model consult.

Keep kernel conclusions and stage answers as diagnostics. Only score a stage
against the original reference when one consultation carries the complete
original question verbatim; a subquestion's answer is not the whole-task answer.
This conservative coverage rule is not a test of semantic faithfulness.

Earlier legacy runs used a kernel-primary, exactly-one-consultation protocol.
Their original grades and manifests remain unchanged. Changes to entry prompts
and time policy are a new configured-system experiment, not a controlled ablation
that isolates one causal improvement.

This is a configured-system comparison, not a compute-matched experiment. Capture
all billed usage and distinguish kernel cost from outer routing cost. A token
limit is an upper bound, not evidence that equal computation was used.

## Data selection

The current user decision on 2026-11-04 supersedes the earlier 20-case and
7-BBEH-plus-7-Omni plans. Pause the previous baseline sweeps and freeze:

- **Fusion 10**: the seven locked BBEH mistakes plus `omni:2617`, and one
  previously correct control from each dataset. The two controls use a fixed
  hash ordering over completed, parsed correct responses. Preserve these exact
  ten historical baseline records; no new generation is used to assemble them.
- **Max 1000**: all 460 BBEH mini cases plus 540 Omni-MATH integer-answer cases
  sampled without replacement using seed `future-deepseek-max-1000-20261104-v1`.
  Before sampling, exclude `omni:383` (disputed premise/reference) and `omni:2865`
  (missing required figure); record these exclusions in the frozen manifest.

Use the ten-case suite for Fusion optimization first. Start the new thousand-case
single-model baseline only when requested; preparing its inputs does not start
model generation. Its run must not rewrite the Fusion suite or cached baseline.
ZebraLogic is retired from both workflows. Old question bundles and outputs are
historical evidence, not active selection defaults. The full BBEH split overlaps
the development suite, so the thousand-case baseline is not an untouched holdout.

Freeze upstream revisions and content hashes, eligible subsets, sampling seed,
question IDs, final-answer requirements, scorer version, repeats and timeout
before generating any model outputs for an unselected capability evaluation.
Synthetic generated tasks require independent solution validation.

The historical first development run deliberately selected by **baseline** outcome:
freeze a uniform random sample of 100 ZebraLogic, 100 BBEH mini and 200 Omni-MATH
integer-answer cases first, then run DeepSeek Flash max once on all 400. Keep its
full scores, outputs, failures, usage and provenance. Once all records exist,
select 14 parsed wrong answers and 6 parsed correct answers using a fixed hash
ordering within each outcome. Exclude service and parse failures from these
selection pools, but keep them in the original baseline denominator. If either
pool is too small, report that and stop; do not retry or replace cases.

Freeze those 20 cases and hashes of their original baseline results **before any
Fusion generation**. Subsequent runs generate only Fusion and reuse the exact
historical baseline. Label historical usage and latency separately. Verify source,
question, reference, prompt, scorer, effective model/effort and result hashes before
spending on the comparison. Never query historical sessions on a different Host.
The 6/20 baseline score is fixed by selection, not an estimate of model ability.
Report recovered mistakes and newly corrupted successes; suppress population
confidence intervals for this selected challenge. It is development evidence,
not a representative holdout or a broad capability claim. The 400-case baseline
report remains the evidence of unselected single-model performance.

Outside this explicitly labelled outcome-selected development challenge, do not
select examples after observing either model's outputs.

The smoke suite has one task from each of ZebraLogic, BBEH mini and Omni-MATH.
Only the integer-answer subset of Omni-MATH is eligible in this version. BBEH mini
has no task labels in the downloaded file, so its pilot selection is over the
whole mini split. Do not describe this as a full or task-balanced BBEH evaluation.

These retained datasets are public. Revision pinning supports reproducibility;
it does not prove exclusion from a model's training data. Do not describe this
evaluation as contamination-free.

For a later formal evaluation, use an untouched 600–1,000-question holdout, report
each dataset separately, and fix any aggregate weights before looking at results.
Choose the actual sample size with reference to the minimum improvement worth
detecting; 600 questions are not a universal significance guarantee. Repeating
the same 30 questions does not produce 90 independent questions.

## Scoring and failures

Score the submitted final answer offline, with no reference-answer feedback to
participants or moderator. Integer answers use integer equality. ZebraLogic
requires a complete correct grid; ordering rows/columns is not a reasoning error.
BBEH uses its pinned official scorer; choice questions use the final option.

Missing results, unparseable outputs, timeouts and model/service errors remain in
the scheduled denominator and are also reported separately. A diagnostic rerun
is a new run; keep the original. If an infrastructure bug invalidates an exercise,
write the reason and rerun the entire frozen suite, not just the losing examples.
Do not retry a wrong answer until it becomes correct.

The new single-model baseline explicitly permits additional attempts only after
an accepted run's terminal ledger confirms HTTP 502. Default: three additional
attempts, with 2/4/8-second backoff (configurable, capped at 30 seconds per wait).
Keep each attempt's original response, ledger, IDs and usage. A failed attempt is
not another independent question. Successful retries use the final attempt as the
case result; keep all earlier failures and mark potentially incomplete usage.
After the retry budget is exhausted, retain the failure in the denominator and
continue the remaining questions. Non-502 execution errors still stop new
submissions for inspection. Do not reinterpret answer errors or unknown submission
outcomes as 502s. Resuming an acknowledged run means waiting on that same run;
resuming a terminal case never generates another answer. This new retry policy
does not rewrite or regenerate the ten-case historical baseline.

Report accuracy, parse success, error rate, paired wins/losses, full/partial
participation, mean and preferably median/tail latency, tokens and available
provider cost. Wilson intervals describe one-pass per-arm accuracy; a paired
bootstrap resamples questions, keeping repeats together. Small-sample confidence
intervals and especially a bootstrap from three cases are diagnostic only.

## Diagnosing Fusion

Preserve first answers, issue map, participant reviews and synthesis. Compare the
latest accepted successful attempt of each role; never count a retry as another
independent participant. Record missing/unparseable stage answers separately.

- Wrong to correct: does review fix a first-round mistake?
- Correct to wrong: does the discussion spread an error?
- Correct minority preserved: when a strict minority initially answers correctly,
  does the final conclusion retain that answer?
- First round entirely wrong: can later reasoning produce a correct result?
- Transfer loss: does the outer answer change a correct Fusion conclusion?
- Mapping loss: was a decisive argument omitted or distorted by issue mapping?

The last question requires inspecting supplied public arguments; agreement is not
evidence of factual verification. Do not score verbosity or consensus as ability.
Do not infer hidden reasoning quality solely from the final-answer score.

## Public report contents

For a public comparison include:

- Date, model/provider identifiers, thinking levels, moderator/panel bindings,
  FutureOS binary version and generation-policy version.
- Exact data revision, subset, sample size, IDs or sample-selection procedure,
  scorer version and repeat count.
- Correct/scheduled counts, per-dataset scores, paired outcomes and uncertainty;
  all exclusions, errors and failed attempts.
- Latency and actual available cost. Missing cost is unknown; platform credits
  are not automatically USD. State whether usage includes outer routing,
  preflight, failed attempts and report repairs.
- The frozen harness hash and commands needed to reproduce the evaluation.
- Whether the questions were used in development, and what is held out.

Prefer claims of the form “On this frozen X-question, text-only subset, under
these settings, Fusion answered A correctly and DeepSeek Flash max answered B.”
Three pilot cases do not support “reasoning improved by X%” or a general model
ranking. Report percentage points and relative changes distinctly when a later
adequately sized experiment supports them. A correct illustrative case can be
shared as a case study, with its selection context disclosed.

Do not publish benchmark question/solution text where upstream terms restrict it.
Do not publish credentials, local absolute paths, raw private session identifiers
or home databases. Raw data and transcripts stay in ignored local directories;
reviewed aggregate exports belong in `reports/`. All results, including sanitized
exports, remain ignored local artifacts. GitHub receives evaluation code,
protocols, source locks and frozen question bundles, including their question
text, reference answers, source attribution and hashes. The full upstream
datasets stay in the ignored download cache. Never commit generated model answers,
scores, timing/cost data, transcripts, run snapshots or home databases.

## Sources

- [ZebraLogic](https://arxiv.org/abs/2502.01100)
- [BBEH](https://github.com/google-deepmind/bbeh)
- [Omni-MATH](https://omni-math.github.io/)
- [MuSR](https://github.com/Zayne-sprague/MuSR)
- [Debate or Vote](https://arxiv.org/abs/2508.17536): motivation for adding
  compute/ensemble controls in later experiments; not an inference about Fusion.
