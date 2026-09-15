# A/B/C compaction comparison

**Question:** when S2 already protects original user/assistant text, is the paid
model summary worth keeping?

- **A** — protected originals + recursive model summary + recent tail (the previous default)
- **B** — same originals and tail, summary deleted and not replaced
- **C** — same originals and tail, summary slot replaced by a deterministic tool-evidence index

The current runtime default is C. See [C compaction](compaction.md); this page only
records the experiment that motivated the switch.

## Setup

| Item | Value |
|---|---|
| Data | 2 synthetic chains (ATLAS, MERIDIAN) from the same blueprint, ~2M estimated tokens each at the end |
| Models | DeepSeek Flash, GLM-5.3-Flash |
| Stages | 8 per chain; checked at stages 1, 4 and 8 |
| Workload | 32 A summary operations; 72 answer conditions (3 arms × retrieval on/off × 12 blocks) |
| Answers | 12 fields per questionnaire: first/latest requirements, codes, old/new tool values, a middle-only value, validation, blocker, deployment, unknown device |
| Retrieval | the real `future session history search/get` CLI against a per-stage archive session |
| Budget guards | 5 model requests and 32 KB of retrieved text per retrieval condition; 8192 output cap; ¥10 total |

All three arms share byte-identical protected text and recent tail. B and C never
see A's summary; C learns only its *length* to match the slot budget.

## Accuracy

Closed-book (no history access), 144 fields per arm:

| Arm | Correct fields | Share | Errors claimed with confidence |
|---|---:|---:|---:|
| A | 126/144 | 87.5% | 0 |
| B | 73/144 | 50.7% | 0 |
| C | 130/144 | 90.3% | 0 |

B failed to answer at all rather than inventing values, and it lost exactly the
facts that only ever appeared inside tool output: earliest/latest versions,
validation result and blocker were 0/12 each. Original-text protection still kept
the first question and first assistant code at 12/12, so B is not "context lost" —
it is missing the tool layer that A and C carry.

With the same history-query capability:

| Arm | Delivered | Correct when delivered | Model calls / probe | History queries / probe | Seconds / probe |
|---|---:|---:|---:|---:|---:|
| A | 10/12 | 120/120 | 3.50 | 7.75 | 21.4 |
| B | 10/12 | 117/120 | 4.17 | 12.83 | 27.3 |
| C | 11/12 | 132/132 | 3.25 | 8.08 | 19.1 |

Five conditions ran out of steps or output and are counted as failures, not hidden.
C was cheapest and fastest to answer; B had to query about 60% more to recover what
it never carried.

## Cost

Whole experiment: **200 API requests, ¥3.4899** (2,984,367 input tokens, 1,586,752
of them cached; 213,809 output tokens, 121,992 of them reasoning). Only synthetic
data was sent; no private conversation was used.

A's summary maintenance: 33 requests over 32 stages, **¥1.2331**, about
**¥0.0385 per stage** (~15.9K input / 2.6K output tokens per request).

Per-condition answer cost, and the combined view for "8 stages per chain plus 3
retrieval probes per chain":

| Scenario | A | B | C |
|---|---:|---:|---:|
| 12 closed-book probes | ¥0.3319 | ¥0.3146 | ¥0.2897 |
| 12 retrieval probes | ¥0.3846 | ¥0.5439 | ¥0.3922 |
| **32 summary stages + 12 retrieval probes** | **¥1.6177** | **¥0.5439** | **¥0.3922** |

Training-free B looks cheapest until it has to search: its retrieval bill is the
largest of the three despite a shorter starting context. In this schedule C cost
about 76% less than A — but probe sampling is not a whole natural workload, so that
number is not a product-wide saving.

Break-even, extrapolating from these means: A's summary needs roughly **3 similar
probes** to pay for itself against B, and roughly **61** against C. The C figure is
model-dependent and unstable (about 11 probes for DeepSeek, no positive break-even
for GLM in this sample), so it should not be treated as a threshold.

## What this does and does not show

- It shows that the model summary is not automatically required: a deterministic,
  fixed-budget evidence slot preserved the tool facts C needed at roughly the same
  accuracy as A, with no summary call at all.
- It does not show that deleting the summary outright is safe. B's losses come from
  removing information, not from removing a model call.
- Both datasets come from one blueprint, so they are not two independent natural
  project types. Structured config/status fields favour C.
- C received A's slot length in the experiment. A deployed C must use a fixed or
  otherwise derived budget, which is what the implementation now does (2K, scaled
  down on small windows).
- Request ordering was randomized within blocks but not strictly position-balanced;
  caching affects reported prices.
- One A summary was empty and one was malformed; the retry and both rejected outputs
  are recorded and billed. Five undelivered probes are kept as failures.
- Scoring used deterministic normalization plus one documented manual equivalence
  ("no deployment occurred" = `NOT_DEPLOYED`); it is not independent blinded review.

Raw data (ledger, contexts, answers, scoring, reproduction scripts) stays in the
local, git-ignored `.future/research/abc-summary-a47313/`. This page is the
committed summary.
