# Compaction strategy comparison (A/B/C/M/Codex/OpenCode)

**Question:** when S2 already protects original user/assistant text, is a paid
model summary the right way to spend the compaction budget — and how do other
agents' published strategies compare?

The runtime default is C. See [C compaction](compaction.md); this page records the
experiment behind that change.

## Strategies compared

| Arm | What survives compaction | Source |
|---|---|---|
| **A** | protected originals + recursive model summary + recent tail (earlier default) | this repo, pre-C |
| **B** | protected originals + tail, summary deleted and not replaced | ablation of A |
| **C** | protected originals + fixed 2K deterministic tool-evidence index + tail | this repo, current default |
| **M** | summary + recent tail; covered originals dropped | `origin/main` as of this experiment |
| **Codex** | **all user messages (≤20 000 tokens) + summary**; no assistant text, no tool output | `openai/codex` @ `b13164d8`, `compact.rs::build_compacted_history` + `templates/compact/prompt.md` |
| **OpenCode** | **summary + retained tail** (`min(15 000, max(2 000, usable/4))`), tool output truncated to 2 000 chars when summarized | `anomalyco/opencode` @ `e03db9bc`, `session/compaction.ts` + `session/message-v2.ts` |

Codex and OpenCode are reimplementations of the selection rules read from those
commits, not forks; prompts were transcribed verbatim and every file's git blob
SHA is recorded in [abc_external_provenance.json](../scripts/abc_external_provenance.json).
Codex's two other paths were not modelled: remote v2 compaction (needs an
OpenAI-hosted provider) and the `Feature::TokenBudget` path (fresh context window,
no summary). OpenCode's optional `compaction.prune` step was left off because it
is disabled unless configured.

## Setup

| Item | Value |
|---|---|
| Data | 2 synthetic tool-heavy chains (ATLAS, MERIDIAN); ~2 M estimated tokens per chain at stage 8 |
| Models | DeepSeek Flash, GLM-5.3-Flash |
| Compaction points | stages 1, 4, 8 of 8 (A/B/C/M). Codex and OpenCode: **stage 1 only** — see cost limits |
| Questionnaire | 12 fields: first/latest requirements, codes, old/new tool values, a value that only appears mid-record, validation, blocker, deployment, unrecorded device |
| Runs | 4 chains × 3 stages × 6 arms ≈ 268 model requests, **¥5.43** |

## Results

Closed-book, 48 fields per stage (4 chains × 12):

| Arm | stage 1 | stage 4 | stage 8 | all stages | mean context |
|---|---:|---:|---:|---:|---:|
| A | 41/48 | 43/48 | 42/48 | **126/144** | ~7.0 K |
| B | 21/48 | 26/48 | 26/48 | **73/144** | ~6.1 K |
| C | 42/48 | 44/48 | 44/48 | **130/144** | ~7.0 K |
| M (`origin/main`) | 43/48 | 38/48 | 36/48 | **117/144** | ~4.9 K |
| Codex | **46/48** | not run | not run | — | ~1.1 K |
| OpenCode | 43/48 | not run | not run | — | ~4.9 K |

Three findings stand out.

1. **Summary-only retention degrades under repeated compaction; C does not.**
   M scores best of A/B/C/M at the first compaction (43/48) and then falls to
   38 and 36 as later summaries have to carry more. C stays flat (42/44/44) and A
   stays flat (41/43/42), because their covered originals or evidence survive each
   round instead of being re-summarised. This is the same failure mode Codex warns
   about in its own UI text ("long threads and multiple compactions can cause the
   model to be less accurate").
2. **Deleting the summary is the worst option (B, 73/144).** Its losses are tool
   facts that never appeared in user or assistant prose: it is information
   removal, not a cheaper way to do the same thing.
3. **A well-fed summary is strong at first compaction.** Codex scored 46/48 on
   ~1.1 K of context — the smallest projection of all six — because its summarizer
   reads the *entire* history and its replacement keeps every user message. Its
   advantage is the reading, not the retention.

## Cost

| Arm | Cost per compaction | Why |
|---|---:|---|
| C | **¥0** | no model call |
| OpenCode | ~¥0.01 | summarises the truncated head only |
| M | ~¥0.02 | summarises clipped material |
| A | ~¥0.039 | summarises clipped material + protected text |
| Codex | ~¥0.26 | summarises the **entire** history (≈250 K tokens at stage 1) |

Codex's cost grows with the archive: the same call at stage 4 would read ~1 M
tokens and at stage 8 ~2 M tokens. Extending Codex and OpenCode to stages 4 and 8
was estimated at ¥6–17 and **was not run** inside this experiment's ¥10 cap, so
their numbers are single-point. The recorded total is ¥5.43 for 268 requests.

## Limits

- Codex and OpenCode have only the first compaction point. Their later-stage
  behaviour is unmeasured; only their retention rules were verified against source.
- Codex's summary reads the full history while A/C/M summarise clipped material.
  The comparison therefore mixes *what a strategy reads* with *what it keeps*; the
  cost column is where that difference is paid for.
- Both datasets come from one synthetic blueprint with structured tool records,
  which favours strategies that carry tool evidence.
- The questionnaire is fact recall, not task continuation; it does not measure
  whether an agent can resume work.
- Request ordering was randomised within blocks, not strictly position-balanced;
  provider-side caching affects the reported prices.
- Ledger is honest about failures: one M stage was aborted mid-request and retried
  (charge unknown, retry billed), and four early OpenCode calls were **invalidated**
  after the tail budget was found to be estimated on truncated text instead of real
  messages; they were re-run as `v2` and the invalid charges are still counted.

Raw data, fixtures and reproduction scripts: `scripts/abc_experiment/README.md`
(harness) and the local, git-ignored `.future/research/abc-summary-a47313/`
(ledger, contexts, answers, scoring).
