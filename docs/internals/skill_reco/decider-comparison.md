# Comparing another decision model against Jev on skill recommendation

The [evaluation](evaluation.md) scores one model — Jev — on the 100-question skill-recommendation
set. When another decision model (for example Perplexity's `pplx-decider-v1.2`) has to be scored
on the same task, the question that comes first is not "how good is it" but "is this harness
still producing the numbers the report published". This page records that check, the roster the
check has to freeze, and the command a new model is run with.

Status: the harness check below is done; **no second model has been run yet**, because its test
endpoint is on a network this machine cannot reach (§4).

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
§3 are compared.

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

`bench/decider-run.mjs` therefore materialises `dataset/roster.json` into a temporary skills tree
and feeds it to the shipped `loadRoster`, so the option text, ordering and 220-character
truncation are the shipping ones rather than a second implementation of them.

## 3. Running another model

```bash
cd scripts/skill_reco
# 5 questions first: does the endpoint answer with a legal Choice at all?
node bench/decider-run.mjs run pplx-test --base https://<host>/api --model pplx-decider-v1.2 --limit 5
# then the full set, then the comparison
node bench/decider-run.mjs run pplx-test --base https://<host>/api --model pplx-decider-v1.2
node bench/decider-run.mjs compare jev-prod pplx-test
```

- The credential is `~/.future/agent/auth.json`'s `future` entry, the same one the agent uses;
  `FUTURE_API_KEY` overrides it for an endpoint that has its own key.
- `--limit 5` is not a courtesy. The common failure with a new model is not a wrong answer but an
  illegal one — free text, several options, or no `none_of_these` — and `Suggester.rank` reads
  missing probabilities as an empty distribution, i.e. every question becomes a confident
  refusal. The smoke call turns that into an error before 100 questions are spent; it also
  asserts that the checked request is byte-identical to the one the serving path sends, so the
  check cannot drift away from the prompt it is meant to cover.
- Cost and time: ~8.7k input tokens per question, ~875k for the set; at pplx-decider-v1.2's
  published $0.02/Mtok that is about $0.018, and about two minutes sequentially (the runner is
  sequential by default because concurrent requests measure gateway queueing, not the model).
- Records land in `runs/<name>/`, which is gitignored, and carry their config (endpoint, model,
  roster, gate threshold, stage-1 revision). A re-run with a different config re-records instead
  of resuming into stale answers.

## 4. Open item

`pplx-decider-v1.2` is not on the public gateway: `POST https://future-os.cn/api/v1/systemone`
answers `model_not_found` for it (and `/v1/decisions` is not routed there), while `jev` works.
The test environment named in the report (`https://test.future-os.cn`) does not resolve from a
normal workstation — it is an internal name. So the run waits on three values only the deployer
has: the base URL, the exact model id, and a key if it is not the Future account's.

## 5. What these numbers can and cannot show

- With 65 positive and 35 negative questions, a single run distinguishes about two questions
  worth of difference (§1 above, evaluation §5.6). A model that lands within ±2 questions of Jev
  is "no measurable difference on this set", not "equal".
- The gold answers are a `kimi-k3` judgement, not human annotation, and the comparison is
  therefore stronger on "did it pick the skill the question was written from" (an independent
  label, `source_skill`) than on the exact first name.
- Latency and price belong to the endpoint that served the request. A test deployment's numbers
  describe the test deployment; only tokens and decisions travel.
