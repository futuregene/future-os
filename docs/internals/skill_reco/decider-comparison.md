# Comparing another decision model against Jev on skill recommendation

The [evaluation](evaluation.md) scores one model — Jev — on the 100-question skill-recommendation
set. When another decision model (for example Perplexity's `pplx-decider-v1.2`) has to be scored
on the same task, the question that comes first is not "how good is it" but "is this harness
still producing the numbers the report published". This page records that check, the roster the
check has to freeze, the command a new model is run with, and — since 2026-11-09 — what the
second model actually measured.

**Result in one paragraph**: on the test deployment, `pplx-decider-v1.2` and Jev both answer
93 of 100 questions correctly, and the questions they get wrong are almost disjoint — decider is
3 questions better at picking the skill and 4 questions worse at refusing. That is not a
difference the set can turn into a decision either way. What does decide it is latency: decider's
floor is 3.48 s per question against a 2.5 s serving-path deadline (§6), so as things stand every
recommendation card would time out. **This is not a model swap until the latency is dealt with**,
and no accuracy number here argues for one.

## 1. Harness validation, 2026-11-02

Jev was re-run end to end — same 100 questions, same `dataset/gold-v1.json`, same serving path
(one Choice over the roster + the `none_of_these` gate, `Suggester.rank`), same 141-skill roster
(§2) — and compared against the numbers in `dataset/score.json`.

| metric | `dataset/score.json` | re-run | delta |
|---|---|---|---|
| decision agreement (n=100) | 93.0% (93) | 93.0% (93) | 0 |
| top-1 correct (n=65) | 90.8% (59) | 90.8% (59) | 0 |
| top-1 in reference set (n=65) | 93.8% (61) | **95.4% (62)** | **+1.6 pp** |
| correct refusal (n=35) | 97.1% (34) | 97.1% (34) | 0 |
| false refusal (n=65) | 1.5% (1) | 1.5% (1) | 0 |
| top-1 = the question's source skill (n=65) | 93.8% | 93.8% | 0 |
| input tokens, median per question | 8,765 | 8,749 | −16 |
| latency, median / p90 | 488 / 1224 ms | 685 / 787 ms | see below |

The instrument reproduces the published decision, top-1, refusal and cost figures exactly. Two
readings need a sentence each.

**"Top-1 in reference set" is the one metric that moved, and it is explained question by
question.** Comparing the re-run with `dataset/score.json`'s `per_question.jev` rows: 95/100
questions got the same first answer and 98/100 the same recommend/refuse decision. Five flipped,
and they cancel out except for that one metric:

| question | published run | re-run | gold | effect |
|---|---|---|---|---|
| p015 | `future-web` | `future-paper` | `[future-web, future-paper]` | top-1 lost, still inside the reference set |
| p028 | `future-explore` | refusal (`none` 0.18 ≥ 0.15) | `[future-explore]` | top-1 lost; the run's one false refusal |
| p035 | `dnanexus-integration` | `latchbio-integration` | `[latchbio-integration]` | fixed |
| p043 | `pyzotero` | `paperzilla` | `[paperzilla]` | fixed |
| n032 | refusal | `future-deep-research` | `[future-web]` | still wrong; its false refusal moved to p028 |

Net: top-1 +2 −2 = 0, in-reference +2 −1 = +1 (61 → 62), correct refusal and false-refusal both
unchanged because the run's single false refusal changed identity (n032 → p028). This is exactly
the run-to-run sampling the evaluation documents in §5.6 — 95/100 and 98/100 sit inside the
91/100 and 97–99/100 it measured across two earlier runs of the identical request — so the 1.6
points are noise, not bias. It should still be read that way whenever these numbers are quoted:
**one run of this set resolves differences of about ±2 questions, no finer.**

Worth recording next to it: p035 and p043 are the two questions §2.2.5 credits to the *removed*
second call (the two wins that leaned on SKILL.md wording). In this run the shipped one-call
pipeline got both of them without any second call. That is further evidence that the "3
questions / 4.6 pp" gap attributed to stage 2 is inside sampling noise rather than a stable
capability difference.

**Latency is not comparable across days.** The re-run's median is higher (685 vs 488 ms) and its
p90 lower (787 vs 1224 ms) than the published single run, on the same endpoint from a different
network. Token counts match, so this is not a payload change. Treat latency as comparable only
between runs measured in the same session on the same gateway — which is how the two models in
§4 are compared.

## 2. The roster is frozen at the 141 skills the gold answers were written against

`dataset/roster.json` holds the 141-skill roster the evaluation used. Two reasons a new run has
to use it rather than the live `skills/` submodule:

- the gold answers name skills that the set was generated from; a roster that gained or renamed
  skills changes both the task and the reference, so the scores would no longer be comparable to
  anything;
- the option table is the model's whole world here. 141 options cost ~8.7k input tokens; each
  extra skill adds a line to every request (evaluation §4.1).

The live submodule has since grown to **242 skills** (17 builtin + 225 third-party, read by the
same `loadRoster`). 242 is still under the 254-option ceiling, so a run on the current catalogue
stays one Choice — but it is a *different* task: it needs a fresh gold pass (`ground-truth.mjs`)
before it can be scored at all, and both models must be run in that same batch, because the
categories of a new gold set cannot be compared with the old scores.

`scripts/skill_reco/bench/decider-run.mjs` therefore materialises `dataset/roster.json` into a
temporary skills tree and feeds it to the shipped `loadRoster`, so the option text, ordering and
220-character truncation are the shipping ones rather than a second implementation of them.

## 3. Running another model

```bash
cd scripts/skill_reco
# 5 questions first: does the endpoint answer with a legal Choice at all?
node bench/decider-run.mjs run decider-smoke --model pplx-decider-v1.2 --limit 5
# then the full set, then the comparison
node bench/decider-run.mjs run decider-test --model pplx-decider-v1.2
node bench/decider-run.mjs compare jev-test decider-test
```

Everything that defines a run, so that a number can be re-obtained rather than trusted:

| input | value |
|---|---|
| endpoint | the account's `base_url` in `~/.future/agent/auth.json` — currently `https://test.future-os.cn/api`; pass `--base` to point elsewhere |
| model id | `--model pplx-decider-v1.2` — the test deployment also accepts `decider` as an alias for it (§5); `--model jev` for the baseline |
| questions | `dataset/questions.json` — 100 (65 answerable, 35 refusable) |
| gold | `dataset/gold-v1.json` |
| roster | `dataset/roster.json` — 141 skills, materialised as a stub tree |
| prompt | the shipped one: `stage1-one-choice-none-v3`, chunk 254, `none_of_these` gate 0.15 |
| decoding | nothing is sent — the payload is `{state, model, questions}`, so the endpoint's default temperature applies (see §4.1 for what it behaves like) |
| concurrency | 1 (the default); a parallel run measures gateway queueing, not the model |
| rounds | a single run is the unit; §4 reports two decider rounds and one Jev round from the same sitting |

- The credential is `~/.future/agent/auth.json`'s `future` entry, the same one the agent uses;
  `FUTURE_API_KEY` overrides it for an endpoint that has its own key.
- `--limit 5` is not a courtesy. The common failure with a new model is not a wrong answer but an
  illegal one — free text, several options, or no `none_of_these` — and `Suggester.rank` reads
  missing probabilities as an empty distribution, i.e. every question becomes a confident
  refusal. The smoke call turns that into an error before 100 questions are spent; it also
  asserts that the checked request is byte-identical to the one the serving path sends, so the
  check cannot drift away from the prompt it is meant to cover.
- Cost and time: ~8.7k input tokens per question for Jev, ~7.2k as reported for decider, ~875k
  and ~719k for the set. At the published prices that is $0.037 (Jev) and $0.014 (decider) per
  100 questions. Sequential runtime follows from latency: ~1 minute for Jev, ~6 minutes for
  decider.
- Records land in `runs/<name>/`, which is gitignored, and carry their config (endpoint, model,
  roster, gate threshold, stage-1 revision). A re-run with a different config re-records instead
  of resuming into stale answers.

## 4. `pplx-decider-v1.2` on the test deployment, 2026-11-09

Jev was re-run first, on the same endpoint and in the same sitting, and reproduced §1 exactly
(93.0 / 90.8 / 97.1, zero false refusals). Then `pplx-decider-v1.2` ran twice.

| metric | Jev (`jev-1.13`) | `pplx-decider-v1.2` |
|---|---|---|
| decision agreement (n=100) | 93.0% | 93.0% |
| top-1 correct (n=65) | 90.8% (59) | **95.4% (62)** |
| top-1 in reference set (n=65) | 95.4% (62) | **96.9% (63)** |
| correct refusal (n=35) | **97.1% (34)** | 88.6% (31) |
| false refusal (n=65) | 0% | 0% |
| top-1 = the question's source skill (n=65) | 93.8% (61) | **98.5% (64)** |
| input tokens, median per question | 8,749 | 7,188 |
| latency, median / p90 | **676 / 774 ms** | 3581 / 3650 ms |

The two systems answer 93 of 100 questions correctly — the same total, reached from opposite
directions. **Six questions differ in total**: decider fixes three (p015 → `future-web`, p019 →
`benchling-integration`, p043 → `paperzilla`, each the gold top-1) and breaks three (n026, n029,
n030 — refusable questions it answers with a skill). p037 is the one question both get wrong
(`matlab`, gold: refuse).

Read on the primary axis, that is a draw: decider is three questions better at "which skill"
(and 3 pp better on the independent `source_skill` label) and four questions worse at "is a skill
warranted at all" — it hands a skill to 4 of the 35 refusable questions (n026, n029, n030, p037)
where Jev does so once (p037, which both get wrong). On 65 positive questions, three questions is
the resolution limit of the set (§1).

The token counts differ by 18% on a byte-identical payload (8,749 vs 7,188): each endpoint
reports usage through its own tokenizer, so per-question cost must be computed from each model's
own count and its own price, not from a shared tokenizer. On published prices decider is ~2.6×
cheaper per question ($0.00014 vs $0.00037) — irrelevant next to §6.

### 4.1 Two rounds, zero differences: this model is deterministic here

`decider-test` and `decider-test2` were run back to back. Across 100 questions they produced
**0 per-question differences**, identical total input tokens (718,603 both rounds) and medians
within 20 ms of each other (3581 vs 3562 ms). Jev, by contrast, moves ±2 questions between runs
of the identical request (§1, evaluation §5.6).

That is why the +3 / −3 split above can be quoted as a difference at all: it is not sampling. A
single Jev run against a single decider run would leave "3 questions" indistinguishable from
noise; with decider stable, the honest statement is *the two models make different mistakes, and
both statements reproduce*. The caveat stays for the sample size (65 positive questions cannot
resolve finer than ±2), not for the model.

### 4.2 The refusal gap is a threshold hypothesis, not a capability gap

The serving path refuses when the `none_of_these` probability reaches `NONE_GATE_THRESHOLD`
(0.15, `scripts/skill_reco/suggest.mjs:94`). That constant is not model-independent: its comment
records that cross-validation on Jev chose 0.11–0.17 fold by fold because those are the edges of
the empty band between Jev's two groups.

Decider's distribution is far more polarised than Jev's:

| | Jev | decider |
|---|---|---|
| answerable questions with `none` ≥ 0.01 | 13/65 | **0/65** (max 0.004) |
| refusable questions with `none` < 0.15 | 1/35 | 4/35 (median 0.864) |
| top-1 probability > 0.5 | 62/65 | 64/65 |

So the same 0.15 asks two different questions of the two models: for Jev it sits in the empty
band; for decider it is a threshold almost nothing lands near. Re-scoring the *same answers* with
a lower gate — no re-run, only the cut — gives:

| gate | decider top-1 | in reference set | false refusal | correct refusal |
|---|---|---|---|---|
| 0.15 (shipped) | 95.4% | 96.9% | 0% | 88.6% |
| 0.01 | 95.4% | 96.9% | 0% | 94.3% |
| 0.005 | 95.4% | 96.9% | 0% | 100% |

This is a **hypothesis, not a result**, and the reason is visible in the same table: at 0.005 the
margin between the smallest refusable `none` (p037, 0.005) and the largest answerable one (0.004)
is 0.001. A threshold that fits one set to four decimal places has been fitted to it. Getting 100%
correct refusal at 0.005 says decider's own distribution is separable *on these 100 questions*;
it does not say where the cut belongs on new ones. Confirming it needs a held-out set — a second
gold batch, or a re-run on the 242-skill catalogue after its own gold pass (§2) — with the gate
chosen on one half and scored on the other.

Until then `NONE_GATE_THRESHOLD` stays 0.15. Moving it is a product-behaviour change for Jev
today, and it must not ride along with a model comparison.

## 5. What the test deployment needed

The previous version of this page ended on an open item: `pplx-decider-v1.2` answered
`model_not_found` on the public gateway, and `https://test.future-os.cn` did not resolve, so the
run was waiting on a base URL and a model id. Both are now answered, and the record of *what
changed* matters more than the fact that it works.

- **The name resolves.** `test.future-os.cn` answers DNS (116.62.200.165, the same address as
  `future-os.cn`) and serves `/api/v1/systemone`.
- **The account already points at it.** `~/.future/agent/auth.json`'s `future` entry carries
  `base_url: https://test.future-os.cn/api`, so the default client — no `--base` flag — is the
  test deployment. That is why `decider-run.mjs run … --model …` is the whole invocation.
- **The model id is `pplx-decider-v1.2`.** Verified against the deployment: `pplx-decider` and
  `decider-v1.2` return `model_not_found`, and so does `jev-latest`, while `jev` resolves. `decider`
  is an accepted alias and is the same model: it returns a byte-identical answer and the response's
  `model` field reads `pplx-decider-v1.2`. Every run recorded here names the full id.
- **The public origin is not a control any more.** With this account's key,
  `https://future-os.cn/api/v1/systemone` answers `auth_required` rather than `model_not_found`,
  so "the public gateway does not serve it" can no longer be demonstrated by a single call. The
  comparison is on the test deployment, as the report intended.
- **First successful call: 2026-11-09.** A 5-question smoke, then one Jev round and two decider
  rounds in the same sitting (records timestamped 10:51–11:02 UTC+8).

## 6. What stands between the two models is latency, not accuracy

Decider's latency is not a tail, it is a floor: across 100 sequential questions its minimum was
**3,481 ms**, median 3,581 ms, p90 3,650 ms, maximum 3,976 ms. Jev's whole distribution fits
between 575 and 877 ms.

The serving path cannot absorb that. `agent/src/skill_reco/mod.rs:50` sets
`TIMEOUT = 2500 ms` deliberately *inside* every client's own deadline
(`RECOMMEND_TIMEOUT_MS = 3000`, `desktop/src/features/agent/useSkillRecommendation.ts:63`) so
that a slow recommender is *decided* as a failure rather than abandoned after the user has waited.
At 3.5 s per call, every call fails that decision. And the failure is not merely a missing card
once: consecutive transient failures put the recommender into a 30 s stand-down
(`agent/src/skill_reco/mod.rs`), during which it does not even open a connection — so switching
models as things stand means cards stop appearing at all, silently, which is precisely the
symptom the diagnostics page exists to explain.

Nothing about the accuracy numbers pays for that. The comparison is a draw on total correctness
(93 vs 93) with disjoint mistakes, and the one metric where decider is clearly ahead — three more
top-1 answers — is at the resolution limit of the set.

So the ending is not "Jev or decider". It is: **any move to decider starts with the deadline** —
either the serving path's timeout moves to ~5 s and the clients' budget to ~6 s (which means the
user waits ~3.6 s before a card appears, for a feature that is currently invisible above ~0.7 s),
or the deployment cuts about a second off the call. Until one of those happens, decider's
accuracy is a number about a latency the product cannot use.

## 7. What these numbers can and cannot show

- With 65 positive and 35 negative questions, a single run distinguishes about two questions
  worth of difference (§1, evaluation §5.6). §4.1 narrows this for decider — its two rounds were
  identical — but it does not widen the set: "decider is 3 top-1 better" is reproducible and
  still inside what 65 questions can certify.
- The gold answers are a `kimi-k3` judgement, not human annotation, and the comparison is
  therefore stronger on "did it pick the skill the question was written from" (an independent
  label, `source_skill`) than on the exact first name.
- Latency and price belong to the endpoint that served the request. A test deployment's numbers
  describe the test deployment; only tokens and decisions travel.
- The gate experiment in §4.2 is fitted on the evaluation set; it is reported as a hypothesis and
  must not be turned into a shipped constant before a held-out check.
