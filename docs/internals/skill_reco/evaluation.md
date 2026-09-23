# Skill recommendation evaluation: Jev vs retrieval and general chat

**Task**: given a catalogue of 141 skills and one user request, decide which skill to use — or that
none applies.
**Conclusion**: Jev, on **one call**, reaches 90.8% first-answer correct, refuses 97.1% of the
questions that should be refused, with 1.5% false refusals, at 0.49 s and ¥0.0027 per question. That
is **on par with, or slightly below, the general chat model it is compared against** — but clearly
cheaper and faster, and its refusal is an explainable probability rather than prompt behaviour. Local
embeddings cannot be used on their own (they have no concept of "should I recommend anything").

> **This version collapsed the flow from two calls into one**: the former second call (re-asking the
> top-3 candidates together with their SKILL.md body) has been removed from the serving path. It used
> to get 3 more questions right (4.6 percentage points) at the price of one more network round-trip
> per request. The cost and the reasoning, plus the measured comparison against the intermediate
> "only when unsure" variant, are in §2.2.5 and §4.5.

| system | decision agreement | first-answer correct | first answer ∈ reference set | correct refusal | false refusal | cost/question | latency/question |
|---|---|---|---|---|---|---|---|
| **Jev (one call, this version)** | 93.0% | 90.8% | 93.8% | 97.1% | **1.5%** | **¥0.0027** | **0.49 s** |
| Jev (two calls, removed) | 96.0% | 95.4% | 96.9% | 97.1% | 1.5% | ¥0.0030 | 0.77 s |
| deepseek-flash (same questions, same prompt) | 95.0% | 93.8% | 98.5% | 97.1% | 0% | ¥0.0249 | 1.75 s |
| embedding retrieval (local omlx) | 67.0% | 64.6% | 72.3% | 71.4% | 16.9% | local compute | 0.015 s |

**Read §5.6 before reading that table**: the same stage-1 code produced 93.8% and 90.8% first-answer
correct in two independent runs, i.e. the 65 positive questions here can only resolve a difference of
about ±2 questions. The "on par with, or slightly below" wording above is written to that precision.

## 1. Background: what Jev is

Jev is the brand name of TypeSafe's System One model. It is not the same kind of thing as a chat
model: **it does not generate text, it judges the content it is given.**

### 1.1 Call shape

```
POST https://api.typesafe.ai/v1/systemone
Authorization: Bearer <key>

{
  "state":     <the content to be judged: string / object / array>,
  "model":     "jev-latest",
  "questions": { "<a name you choose>": <question object>, ... }
}
```

One call may carry **any number of questions**, all evaluated **in parallel** against the same
`state`, returned under the same names:

```json
{ "model": "jev-1.13.0",
  "answers": { "<a name you choose>": <answer object>, ... },
  "usage":   { "input_tokens": 272, "output_tokens": 20 } }
```

### 1.2 Only three question types

| type | asks | answers |
|---|---|---|
| **`noul`** | a yes/no question | `noul`: a number in 0–1 = "the probability that the answer is yes" |
| `choice` | pick one of the options you supply | `choice` (the selected option) + `probabilities` (**always summing to 1**) + `confidence` |
| `score` | grade the content on your scale | `score` (the probability-weighted grade) + `legend` + `probabilities` |

`criteria` is the question's supporting text: for a noul it says what yes/no mean, for a choice
**one key per option** (i.e. the option list), for a score the ordered grade descriptions.
`instructions` may be a string, or an object (the question in one field, the data it refers to in
others, addressed with backticks as `` `field` ``).

### 1.3 Three things you must know to use Jev

1. **`noul`s are independent of each other; a `choice` is a distribution that must allocate all
   100%.** The former is naturally comparable (141 of them side by side can be ranked), while the
   latter always has a winner (even when every option is unsuitable). Measured on the same questions
   here: three parallel nouls sum to 1.78 on average (range 0.20–2.89) — they are **not** a normalised
   distribution; a choice is forced to sum to 1.
2. **It will not say "none of these fit".** That is by design — it only emits probabilities. So
   refusal has to be decided by the caller, and this project used both routes: give the choice a
   `none_of_these` option so that "none fits" competes with real candidates **on the same question**
   (rather than asking a separate "is a skill needed?" question, which produced 8 extra false
   refusals), then threshold that probability; and, for the removed second call, threshold the fit
   probability of the 3 candidates (see §2.2, §2.3).
3. **Only input tokens are billed** ($0.042 per million tokens, **output tokens free**,
   docs.typesafe.ai/models). So "putting skill documentation into the question" is the main source of
   cost.

---

## 2. Method

### 2.1 Skill catalogue

The evaluation covers **all** skills in the `skills/` submodule: **141** of them — 15 builtin plus
126 third-party (covering bioinformatics, chemistry, clinical work, data science and more). The
fields available per skill:

| field | source | instance for `scanpy` |
|---|---|---|
| `name` | SKILL.md frontmatter `name:` | `scanpy` |
| `description` | SKILL.md frontmatter `description:` (full, untruncated) | "Standard single-cell RNA-seq analysis pipeline. Use for QC, normalization, …" (434 chars) |
| `chinese_description` | `description_zh` in the catalogue's `skills.json` | "标准单细胞 RNA-seq 分析：QC、标准化、降维、聚类、差异表达" (34 chars) |
| `instructions_excerpt` | first 900 chars of the body **after** the frontmatter | "# Scanpy: Single-Cell Analysis ## Overview …" |

### 2.2 Recommender structure: one call, one threshold

```
the user's request
  │
  └─ call 1 (routing): one choice, options = the 141 skills + `none_of_these`
        option text: skill name → description (truncated to 220 chars), the whole table
        written once, ≈8.8k tokens
        → one probability distribution (always summing to 1) + one none probability
     gate: none probability ≥ 0.15 → refuse; otherwise recommend the highest
```

The decision reads exactly one number: the probability of `none_of_these`
(`NONE_GATE_THRESHOLD`). That is the model's own answer — the Choice is asked to select it when no
skill fits — rather than a threshold we cut on some other score. Why refusal is done this way is in
§1.3 item 2.

**The former second call** (re-asking the top-3 together with the full description and the SKILL.md
body, one noul per candidate, behind its own fit threshold) no longer serves. Its trade-off is in
§2.2.5 and the full evidence in §4.5. The code keeps it because the evaluation scripts use it to
produce the recorded comparison; `server.mjs` never calls it.

### 2.2.1 Routing: one Choice with a none option is enough, and halves the cost

Four routing shapes were tried, same 100 questions, only the routing changed. **Note the basis**:
these comparisons were made under the then-current two-call setup (the downstream re-check fixed),
because what is being compared is "how good are the candidates routing produces"; the re-check was
later removed (§2.2.5), but **the routing choice itself is unaffected** — it depends only on candidate
quality, which is what this section measures.

| routing shape | first-answer correct (held out) | false suggestions | tokens/question | cost/question |
|---|---|---|---|---|
| 141 parallel nouls + a separate needs_skill gate | 62/65 = 95.4% | 2/35 | 21,357 | $0.00090 |
| **one Choice (141 options + `none_of_these`)** | **63/65 = 96.9%** | **1/35** | **10,372** | **$0.00044** |
| chunked Choice (18 per block, each with none) + a final Choice | 63/65 = 96.9% | 1/35 | 11,371 | $0.00048 |
| one Choice (141 options) **without** none, plus a separate gate | 55/65 = 84.6% | 0/35 | 10,410 | $0.00044 |

**Two findings, one of which overturns an earlier judgement of mine.**

**Finding 1: the `none` option is required, and it is the only way to refuse.** Compare the last two
rows: with a single Choice and no none option, the only way to decide whether to suggest at all was a
separate "is a skill needed?" question, and 8 questions that should have been answered were falsely
refused (84.6%). Making "none of them fit" an **option** that competes with real candidates on the
same question drove false refusals to zero and put first-answer correct back to 96.9%. The reason is
the phrasing: with concrete candidates in front of it, "none of these work" is a judgement that can be
made item by item; without candidates it becomes an abstract question, and is answered less reliably.

**Finding 2 (overturning an earlier judgement): probability saturation does not affect the result, so
chunking is unnecessary.** I had rejected the single Choice on the grounds of saturation — only a
median of **2** options receive non-zero probability (on 48/100 questions the 2nd and 3rd are exactly
0), so "the top three" looked like a tie at zero. Measuring it showed:

```
the correct skill reaching the top 3 handed to the re-check:
  single Choice (141 options)   65/65 = 100%
  chunked 18 (8 blocks)         65/65 = 100%
```

Because **the correct skill is usually in the winner position** (median 0.89 on a real candidate), and
the zeros at positions 2 and 3 are just padding. Saturation damages candidate **diversity**, not
whether the list **contains the right answer** — though that is also the price to acknowledge once the
re-check is gone: if positions 2 and 3 are mostly padding at zero, there is no reliable "second
opinion" left to correct with (§2.2.5). Since chunking brings no benefit it should not be paid for:
chunking costs an extra "re-rank across blocks" call (+9%), so **the default does not chunk**.

**The only reason to keep chunking**: a Choice accepts at most **255 options**, so a catalogue above
254 must be split. The code keeps that path (setting `CHUNK_SIZE` enables it), covered by this
section's experiments and by held-out validation.

**The ceiling is measured, and none occupies one of the slots** (`bench/option-limit.mjs`): sending
253 / 254 / 255 / 256 options, 255 passes and 256 returns
`400 Too many choices. Must have at most 255 choices.` So the cap is **254 skills per block** (255 −
none), which is exactly `CHUNK_SIZE`'s default, meaning "do not split until the API forces it".
`CHUNK_SIZE=255` throws **at module load** with the reason (a guard in `suggest.mjs`) instead of
letting every request hit a 400 — this off-by-one is a real trap, because the API documentation only
says "at most 255 choices" without saying whether none counts.

**The threshold.** The gate reads the final Choice's none probability, thresholded at 0.15. It is a
wide plateau: 0.1–0.9 give nearly the same result, because questions that should be refused sit above
0.9 and ones that should be answered sit near 0, with almost no samples in between
(`stage1-chunked-sweep.mjs`). 0.15 sits at the bottom of that empty band; it is not a tuned spike.

**Why the cost halves.** The skill body (≈8.6k tokens) is paid either way; the difference is the
template: the noul version repeats "question + criteria" 141 times (≈12k tokens), the Choice version
writes it once.

### 2.2.2 Held-out validation

If the thresholds above were chosen on the same 100 questions, all they can show is "no difference was
observed". Five-fold held out (`chunked-cv.mjs`, `onechunk-compare.mjs`: each fold picks thresholds on
the other four and is then applied to the unseen fifth, with the false-suggestion budget matched
between the two sides):

| false-suggestion budget | 141 nouls | single Choice + none | chunked 18 | chunked 30 | chunked 47 |
|---|---|---|---|---|---|
| 0 (no false suggestions) | 40/65 = 61.5% | 55/65 = 84.6% | **57/65 = 87.7%** | 55/65 = 84.6% | 55/65 = 84.6% |
| **1 (= the shipped operating point)** | 62/65 = 95.4% | **63/65 = 96.9%** | **63/65 = 96.9%** | 60/65 = 92.3% | 60/65 = 92.3% |

**Conclusion: the single Choice ties the best chunked variant on the held-out set (63/65) at 9% lower
cost; both beat the original 141 nouls and cost half as much.** Under a zero-false-suggestion budget
chunking is slightly better, but that budget is not the shipped operating point (the shipped version
accepts 1 false suggestion), so it is no reason to choose it. (Both rows are measured under the
two-call basis, per §2.2.1.)

**Limitation**: each fold holds only 13 positive questions, and the 95% confidence interval on 65
positives is [89.5%, 99.2%] — nearly 10 percentage points wide. The correct reading is therefore
"**no quality difference was observed**", not "equivalence was proven". Pinning a difference down
needs 300–500 questions.

### 2.2.3 Looking at routing alone: what it can rank

This section is the full capability of the shipped version (`stage1-ranking.mjs`). The two sets of
figures below come from two independent runs, and the difference between them is exactly the
run-to-run noise of §5.6:

| what is measured | single Choice (shipped) | 141 nouls (earlier shape, control) |
|---|---|---|
| correct skill ranked **first** | 61/65 = 93.8% (another run: 60/65 = 92.3%) | 58/65 = 89.2% |
| correct skill in the **top 3** | 65/65 = 100% | 65/65 = 100% |
| correct skill in the **top 5** | 65/65 = 100% | 65/65 = 100% |
| refusal (none ≥ 0.15 / top-1 noul < 0.70) | 34/35 = 97.1%, false refusals 0–1/65 | 29/35 = 82.9%, false refusals 1/65 |

The same table from an earlier run, for comparison (it shows the size of the noise):

| what is measured | single Choice (shipped) | 141 nouls (earlier shape, control) |
|---|---|---|
| correct skill ranked **first** | 61/65 = 93.8% | 58/65 = 89.2% |
| correct skill in the **top 3** | 65/65 = 100% | 65/65 = 100% |
| correct skill in the **top 5** | 65/65 = 100% | 65/65 = 100% |
| refusal (none ≥ 0.15 / top-1 noul < 0.70) | 34/35 = 97.1%, false refusals 0/65 | 29/35 = 82.9%, false refusals 1/65 |

**But the Choice row's "top 3 / top 5" must not be read as recall.** It ranks the 141 options by
probability and takes the head, and that distribution saturates: a median of only **2** options
receive non-zero probability, 42/100 questions have **only 1**, and on **46** of the 65 answerable
questions the 3rd-ranked probability is exactly 0. In other words, past the first name, the last two
slots of "top 3" are **padding tied at zero**, not "two more candidates". The noul row is a genuine
ranking (one independent probability per skill, median 10 non-zero), so its top 3 / top 5 do mean
something.

Comparing the two says three things:

- the shipped Choice is **more accurate at rank 1** (93.8% vs 89.2%) and its gate is far stronger
  (97.1% vs 82.9%);
- but candidate **diversity** really is lost: the noul version offers several genuine candidates per
  question, while the Choice has real candidates at positions 2/3 on only 4/65 questions (the rest is
  padding);
- that **did not affect the final result** — in both designs the correct skill is inside the top 3, and
  all the re-check required of a candidate list was "contains the correct skill" (§2.2.1, last
  paragraph).

In other words, halving the cost by dropping the noul version paid its price in a metric this
evaluation does not use. The cost breakdown is in §4.4.

### 2.2.4 What the removed call had to carry (recorded for reference)

The second call was "the same question, better evidence": each candidate carried its full description
plus the first 900 characters of its SKILL.md. This section keeps the field decisions, because **if
that path is ever restored the conclusions still apply** — do not add the deleted fields back:

| field | source | length | decision |
|---|---|---|---|
| `name` | SKILL.md frontmatter | 6 | kept |
| `description` | SKILL.md frontmatter `description:` (full) | 434 | kept (the "signature") |
| `instructions_excerpt` | first 900 chars of the body after the frontmatter | 900 | **must not be dropped**: sending name + description alone collapses the zero-false-refusal threshold from 0.79 to 0.29 and destroys refusal entirely |
| ~~`chinese_description`~~ | `description_zh` in the catalogue's `skills.json` | 34 | **removed**: a paired A/B (198 question×candidate pairs) showed a median difference of 0.000 and 0 questions changed end to end; putting it in the routing option table even added 1 false refusal |
| ~~`tagline`~~ | — | — | **removed**: for 141/141 skills it is a byte-prefix of `description`, i.e. pure duplication |

(`bench/stage2-zh.mjs`, `bench/stage1-zh.mjs`. The two payload-shape experiments that measured the
`instructions_excerpt` and `tagline` rows were removed together with the call they studied — their
conclusions are recorded in the table above, and no script here reproduces them.)

### 2.2.5 Why only one call is left

The second call merely confirmed the first call's answer on 97% of the questions it was reached on,
while costing one more network round-trip per request. Three shapes measured on the same records
(`bench/drop-stage2.mjs`, a paired comparison: all three read the same stage-1 answers):

| shape | first-answer correct | first answer ∈ reference set | second-call count | latency p50 | p90 | p95 | cost/question |
|---|---|---|---|---|---|---|---|
| **① one call only (this release)** | 59/65 = 90.8% | 61/65 = 93.8% | **0/65** | **488 ms** | **1224 ms** | **1401 ms** | **¥0.00265** |
| ② only when unsure (previous version) | 62/65 = 95.4% | 63/65 = 96.9% | 20/65 | 526 ms | 1338 ms | 1542 ms | ¥0.00274 |
| ③ always two calls | 62/65 = 95.4% | 63/65 = 96.9% | 65/65 | 772 ms | 1410 ms | 1672 ms | ¥0.00295 |

Three conclusions:

1. **① is cheaper and faster than ③ and only 3 questions behind** (3 more correct = 4.6 percentage
   points); that is the direct reason for removing it.
2. **But ② strictly beats ①**: the same quality (62/65) for 38 ms more median latency and 3.4% more
   cost, while paying the second call on only 20/65 questions. The three questions ① loses had a
   routing top-1 probability of just 0.77 / 0.36 / 0.39 — **all far below 0.95, so ② caught every one
   of them**.
3. So "should there be a second call" comes down to whether **+3 questions (4.6pp)** is worth
   **+38 ms median latency / +114 ms p90**. This version ships with the "one call only" decision, on
   the grounds that a shorter path is better (one fewer round-trip = one fewer failure mode).
   Restoring ② takes three changes: wire `verifySecondCall` from `bench/second-call.mjs` back into
   `server.mjs`, add the "only when unsure" test back into `suggest.mjs`, and its threshold constant.
   The code is still there (the removed call was merely moved to `bench/`), and the evidence is
   `bench/drop-stage2.mjs`.

**A caveat that must be read alongside those numbers**: two of the three questions it recovers (p019,
p035) have question wording that overlaps the winning skill's SKILL.md *body* and not its description
(p019: cloud / platform / sciences; p035: python / cloud / automatic / no-code). The reference answers
were written from the SKILL.md, so the questions borrow the body's vocabulary — and only the second
call sees the body. **So its advantage is an upper bound on the real advantage, not an expected
value** (§4.5).

### 2.2.6 Can the prompt be tuned further: 14 variants, 4 batches, answer no

This section is a **negative result**, but a useful one: taking the one-call design further can only
mean changing the prompt or the option text, both of which were measured, and **neither beat the
current wording**. It also pins two sentences that look deletable into load-bearing structure, so
nobody removes them by accident later.

**The method matters more than the variants.** The same code changes its top-1 on about 9% of
questions between runs (§5.6), which is the same size as any prompt change could produce. So variants
are **not** sent as separate requests and compared; they are **put inside one request** as several
Choice questions (same `.state`, each with its own option table), which makes the comparison paired
and cancels run-to-run noise. The risk is cross-contamination, so every batch **includes the shipped
wording as a control** and compares it against the same record from an independent request:

| batch | variants | control vs independent request, decision agreement |
|---|---|---|
| A (how to ask) | 4 | **100/100** |
| B (what goes in the options) | 5 | **99/100** |
| C (isolating A's confound) | 4 | **100/100** |
| D (description-length sweep) | 4 | **100/100** |

Contamination is negligible, so within-batch comparisons are trustworthy.
(`bench/stage1-prompt.mjs`, `--batch A|B|C|D|E`. **Batch E serves a different purpose**: it sends only
two option tables at a time, and exists to re-check the variant that sat *last* in batch D — five
tables in one request (median 30,526 tokens) squeezed the final question, see the batch D note below.)

**A: how to ask.** The shipped wording is the best row.

| variant | first-answer correct | first answer ∈ reference set | correct refusal | false refusal | false suggestion |
|---|---|---|---|---|---|
| **shipped (control)** | **61/65 = 93.8%** | **63/65** | **34/35 = 97.1%** | 0.0% | 1/35 |
| a shorter question | 61/65 | 63/65 | **30/35 = 85.7%** ✗ | 0.0% | 5/35 |
| an explicit bar for "pick a skill" | 59/65 ✗ | 61/65 | 34/35 | 3.1% | 1/35 |
| stressing "if a general assistant can answer, no skill" | 56/65 ✗✗ | 58/65 | 34/35 | 6.2% | 1/35 |

Two points:

- **"Choosing it is a normal answer here, not a fallback." is load-bearing**: deleting it ("a shorter
  question") leaves the first answer unchanged but **drops refusal from 97.1% to 85.7%** (4 more false
  suggestions). Jev needs explicit permission that choosing none is not a cop-out.
- **"Stressing that a general assistant suffices" looks like addressing the cause but is clearly
  worse** (−5 first answers, 6.2% false refusals): it over-refuses questions that should be
  recommended. That is the opposite direction from the "matrix inversion" bad case in §5.3 — trying to
  fix that one with prompt wording breaks other questions first.

**C: splitting A's confound.** "A shorter question" changed two things at once (terser wording *and*
deleting the permission), so the presupposition of "needs a skill" was tested on its own:

| variant | first-answer correct | correct refusal | false refusal |
|---|---|---|---|
| **shipped (control)** | **61/65** | **34/35** | 0.0% |
| dropping the "needs a skill" presupposition | 60/65 | **32/35 = 91.4%** ✗ | 0.0% |
| asking "whether" first, then "which" | 60/65 | 34/35 | 1.5% |
| option description at 240 chars | 61/65 | 34/35 | 1.5% |

**The currently "biased" wording ("The request needs a skill from `criteria`") is in fact better**:
removing its presupposition loses 2 refusals. So the intuition "write the question more neutrally" is
wrong here.

**B: what goes in the options.** The shipped option text (brand description truncated to 220 chars) is
also the best or tied-best.

| variant | first-answer correct | first answer ∈ reference set | correct refusal | option-table size |
|---|---|---|---|---|
| **shipped (control)** | **61/65** | **63/65** | 33/35 | 7,521 tokens |
| option description halved (110 chars) | 57/65 ✗✗ | 59/65 | 34/35 | **3,890 tokens** |
| option description untruncated | 60/65 | 63/65 | 34/35 | 13,279 tokens |
| spelling out the none option | 59/65 ✗ | 61/65 | 34/35 | 7,551 tokens |
| category labels in the options | 61/65 | 63/65 | 34/35 | 7,960 tokens |

**"Spelling out the none option" backfires** (−2 first answers): writing "a general assistant can
answer this, so none applies" into the option description pushes the model to over-refuse — the same
cause as the "stress the general assistant" row in batch A. "Category labels" ties but costs more.

**D: sweeping the description length to find the knee.** The option table is 86% of the whole request,
so "shorter" is the only meaningful cost lever:

```
110 chars → 57/65      180 chars → 59/65      240 chars → 61/65
150 chars → 58/65      190 chars → 60/65      untruncated → 60/65
                        220 chars → 61/65  ← shipped
```

"256 chars" once appeared to refuse **all 100 questions**, and that was an **artefact of the
apparatus**; the real cause is worth recording separately:

`stage1-prompt.mjs` caches by **question id** (`runs/stage1-prompt-D/<id>.json`), and one file holds
**every variant in that batch**. 256 was added to `VARIANTS` after the cache already held all 100
files, so `todo` was empty, **nothing re-ran**, and no record had a `desc_256` key. In the scorer
`!r -> null -> refusal`, i.e. "no data" and "refused" are the same value, so it was counted as "all
100 refused".

Two fixes: the scorer now validates that every question × variant is present **before** scoring, and
**fails loudly** naming the cache directory to delete (instead of quietly producing a set of fake
numbers); and `bench/ctx-limit.mjs` tests whether packing several tables into one request is a problem
at all — the same shipped option table repeated k times in one request, each position compared against
its own separate-request record:

```
k=2  17,210 tok   position 1:6/6  2:6/6
k=3  25,657 tok   position 1:6/6  2:6/6  3:6/6
k=4  34,104 tok   position 1:6/6  2:6/6  3:6/6  4:6/6
k=5  42,551 tok   position 1:6/6  2:6/6  3:6/6  4:6/6  5:6/6
k=6  50,998 tok   position 1:6/6  2:6/6  3:6/6  4:6/6  5:6/6  6:6/6
```

**Every position is identical, including the sixth table**, so "paired inside one request" (the method
of batches A–E) holds up to five tables (about 34k tokens). This section's earlier explanation —
"the context is full and the last question degrades" — was **wrong**.

Batch D re-run (real data, control 60/65):

| variant | first-answer correct | first answer ∈ reference set | correct refusal | false refusal | questions differing from control |
|---|---|---|---|---|---|
| 220 chars (control) | 60/65 = 92.3% | 62/65 | 34/35 | 1.5% | — |
| 150 chars | 58/65 = 89.2% | 60/65 | 34/35 | 1.5% | 4 |
| 180 chars | 57/65 = 87.7% | 59/65 | 34/35 | 1.5% | 5 |
| 190 chars | 60/65 = 92.3% | 62/65 | 34/35 | 0% | 2 |
| **256 chars** | **61/65 = 93.8%** | **63/65** | 34/35 | **0%** | 3 |
| (batch E independent re-check) 256 chars | 62/65 = 95.4% | 64/65 | 34/35 | 0% | — |

So 256 chars **costs no quality** (two independent paired comparisons each +1, inside the noise), and
its price is purely **money**. Measured per step with single-variant requests (`bench/desc-cost.mjs`,
same questions, one table per request, so that request's `input_tokens` belong entirely to that step):

| description truncation | option-table chars | tokens/question | vs 220 | cost/question | 100k calls/day |
|---|---|---|---|---|---|
| 110 | 15,510 | 5,705 | −3,077 | ¥0.0017 | ¥173 |
| 150 | 21,130 | 6,945 | −1,837 | ¥0.0021 | ¥210 |
| **220 (shipped)** | 30,036 | **8,782** | — | **¥0.0027** | **¥266** |
| 256 | 34,126 | 9,600 | **+818** | ¥0.0029 | ¥290 |
| 300 | 38,575 | 10,500 | +1,718 | ¥0.0032 | ¥318 |

**Conclusion: 256 chars does not earn back that 9.3%** (¥266 → ¥290/day), so `descChars` stays at 220.
Note that the cost here is **linear**: one more character of description is one more character for each
of 141 skills.

**A methodological lesson worth recording separately, because it is the same trap biting for the third
time**: the cache is keyed by question id while a record holds the whole batch of variants, so **adding
a variant to a batch that has already run does not run it — it merely goes missing**, and missing looks
exactly like refused in the scorer. The first two were "the cache fingerprint did not include the
stage-1 shape" (rewriting the criteria appeared to have no effect) and "predict-jev masks the gate".
The common factor: **the tooling silently reported "not measured" as "measured"**. Every script now
either carries a configuration fingerprint or validates completeness before scoring — without that,
"a variant refuses everything" and "it never ran" are indistinguishable.

**Monotone up to 220, then flat**: shorter loses answers one at a time (3 at 150 chars, 4 at 110), and
longer saves nothing and gains nothing. **220 is the knee of that curve**, so the option length has no
room left either.

**Conclusion: the single-call prompt is at a local optimum; the 90.8% in this evaluation is not bad
prompt writing.**

Read together with §2.2.5 this leads to a clear judgement: **what still separates this from the
two-call version is missing information, not wording.** The removed call got those extra questions
because it could read the SKILL.md body, and the single-call option table contains no body (§4.5 item
d: relaxing the description length cannot recover it either, because the wins turn on words from the
body). So there are only two routes: accept the gap on those questions, or put the second call back —
**there is no third "tune the prompt some more" route.**

### 2.2.7 Offering more "escape options" (more than one fits / the skill I need is not listed)

A natural idea: `none_of_these` only covers "none of them fit", but two other cases exist — **several
fit** (12 of the 65 answerable questions have a reference set of multiple skills), and **the needed
skill is not in the table** (the user names a tool outside the catalogue). So add two more options and
give the model somewhere to put those cases.

Measured (`bench/escape-options.mjs`, 100 questions, 4 option sets paired inside one request,
contamination check 100/100):

| option set | first-answer correct | first answer ∈ reference set | correct refusal | false suggestion | questions differing from control |
|---|---|---|---|---|---|
| shipped (none only) | 61/65 = 93.8% | 63/65 | 34/35 | 1 | — |
| + "more than one fits" | 61/65 = 93.8% | 63/65 | 34/35 | 1 | **0** |
| + "the skill I need is not in this list" | 61/65 = 93.8% | 63/65 | **33/35** | **2** | 1 (worse) |
| + both | 61/65 = 93.8% | 63/65 | **33/35** | **2** | 1 (worse) |

**Conclusion: adding them gains nothing, and "not in this list" is mildly harmful.**

**Why "more than one fits" was never selected (0/100)**: the question asks "**which one**, or does none
help", and "several fit" **is not an answer to that question** — the flow needs **one** skill to call,
so the model never makes it the winner (it only uses it as a hedge: non-zero probability on 35/100
questions, at most 0.27). Nor is there anything for it to fix:

| | hit the reference set | matched reference's first answer | refused |
|---|---|---|---|
| shipped | 12/12 | 10/12 | 0/12 |
| + "more than one fits" | 12/12 | 10/12 | 0/12 |

**"Several fit" was never broken** — the shipped version hit the reference set on all 12 of those
questions.

**Why "not in this list" is worse**: the model does use it, and uses it accurately — it was selected 3
times (n013 / n014 / n016), all questions of the "user names a tool outside the catalogue" kind.
**But those three had none probabilities of 0.38 / 0.34 / 0.40, so the shipped gate already refused
them**, leaving the new option nothing to earn. Its cost appears elsewhere:

```
n026  "what biological functions does the BRCA1 protein generally have"   reference: refuse
  shipped      gget=0.35  future-database-lookup=0.22  none=0.16  → refused (gate stops it)
  + not-listed gget=0.38  future-database-lookup=0.18  none=0.14  → suggests gget (a false suggestion)
```

**The same model judgement and the same wrong answer, with only the normalisation changed**: the new
option took 0.02 of probability away from none, none fell below the threshold, and the gate let through
the suggestion it had been stopping.

**This is the easiest structural constraint in the design to trip over**: the gate reads none's *share*
of the distribution, and a Choice's probabilities always sum to 1 — so **adding anything to the option
table is not a neutral operation; it silently recalibrates the only refusal signal.** The dilution is
visible across the batch: among questions that should be answered, the maximum none fell **0.14 → 0.09**,
and among questions that should be refused, the minimum none fell **0.16 → 0.14** — the empty band moved
down as a whole. The model's sense of direction did not change, but the gate's margin was eaten.

Both ways of compensating were tried, and neither has value:

* **Treating "the model chose an escape option" as a refusal**: `+ more than one fits` falls from 61/65
  to **56/65** (7.7% false refusals), because a higher probability on "several fit" does not mean "no
  skill should be used".
* **Re-tuning the gate for each option set** (`escape-options.mjs` §7 swept 0.05–0.30): `+ not in this
  list` returns to 61/65, refusal 34/35, 1 false suggestion at a gate of 0.10–0.12 — **exactly the
  shipped result**. In other words, adding options forces a **recalibration**, and after recalibrating
  the **gain is zero**. It also carries the extra risk that the threshold was fitted on these 100
  questions.

(Requests that genuinely name something outside the catalogue are handled by none today, well enough:
n013 / n014 / n016 were all refused. Making that case more explicit should not be done by adding an
option but by writing it into the description of none inside `criteria` — and batch B already showed
that "spelling out none" pushes over-refusal, losing 2 first answers.)

**How far these conclusions reach**: 100 questions can resolve about 2 questions of difference, so "tied"
here means "no effect larger than 2 questions was observed", not "equivalence was proven"; and every
"worse" is measured against the control **inside the same request**, so the direction is trustworthy.
These variants also apply only to **this catalogue and these question types**; a different catalogue
(especially one where skills resemble each other more) is worth re-running
`bench/stage1-prompt.mjs`.

### 2.3 Why refusal is a threshold rather than left to the model

Because of §1.3 item 2: Jev will not say "none of these fit". What it can do is **select
`none_of_these` when that option is offered**, and that is still only a probability, so a threshold is
needed in the end. The benefit is that it is **explainable** (the UI shows "none probability 0.71 ≥
0.15, so refused") and **adjustable** (a new catalogue can be recalibrated without touching the prompt).

It was set once: `NONE_GATE_THRESHOLD = 0.15`. And it is a wide plateau — questions that should be
refused sit above 0.9 and ones that should be answered sit near 0, with almost no samples in between,
so 0.1–0.9 give nearly the same result (`bench/stage1-chunked-sweep.mjs`). 0.15 sits at the bottom of
that empty band.

(The removed second call carried its own fit threshold, `FITS_THRESHOLD = 0.75`. Across these 100
questions it **never fired once**: 34 of the 35 questions that should be refused were already stopped by
the gate. So that threshold was never necessary for refusal — see §4.5.)

### 2.4 Building the question set (100 questions)

| class | count | how |
|---|---|---|
| **a skill applies** | 65 | 15 builtin and 50 third-party skills sampled evenly; `future/kimi-k3` read each skill's SKILL.md and wrote one sentence **a real user would say**, explicitly forbidden from naming the skill or any tool/package/API from the document; each was then machine-checked and rewritten on failure (up to 5 rounds, feeding the offending words back as a blacklist). All 65 passed. |
| **no skill applies** | 35 | hand-written. Small talk, general knowledge, engineering work with no matching skill (fix CSS, write a git hook, split a Python file), requests naming services that do not exist (book a flight, add a Trello card, post to Mastodon, file a Concur expense), and 7 near-miss traps that touch a scientific topic but only want an explanation. |

**Independent re-check** (`check-dataset.mjs`, distrusting the generation-time check): **0** of the 65
questions name a skill; one "word that only appears in this skill's documentation" hit (p061's
`variant`, a false positive on a generic word). Sample questions (naming no tool):

```
p004  [future-deep-research] I keep seeing claims online like "intermittent fasting can reverse
                            type 2 diabetes" — can you properly trace where these claims come from…
p053  [scanpy]              I have single-cell RNA-seq data and want a full downstream analysis:
                            QC, normalisation, dimensionality reduction, Leiden clustering, marker
                            genes and visualisation — can you set up a pipeline for me?
p005  [future-document]     I have a product manual as a PDF and want to extract the text for notes…
n011  [none]                my CSS grid layout collapses in Safari, can you take a look
n019  [none]                book me a flight to Berlin next Tuesday
```

### 2.5 Reference answers ("gold")

Produced by `future/kimi-k3`, seeing only the 141-line skill catalogue plus the question, returning at
most 3 skill names or an explicit "no skill needed". **It cannot see which skill the question was
generated from**, so this is an independent judgement. It is an LLM judgement, not human labelling —
listed as a limitation in §5.

**How reliable the reference is itself**: running the same 100 questions twice independently gives the
**same first answer on 99/100** (the only flip is p037: refused once, `matlab` once); positions 2–3
drift (the full set matches on 84/100). So "should this be refused" and "who is first" are trustworthy,
while "cover every name it lists" is limited by its own wobble.

Both generations are kept in the repository for checking: `runs/gold-v1` (= the version all scores in
this report use, and the content of `dataset/gold-v1.json`) and `runs/gold-v1-b` (the other, for
comparison only). Because the reference comes from an LLM, **a fresh generation yields scores 1–2
questions different**, so `dataset/gold-v1.json` must match the version that produced
`dataset/score.json` — otherwise the reference read from the repository and the scores do not agree.

### 2.6 Systems compared

| system | note |
|---|---|
| **embedding retrieval** | `Qwen3-Embedding-0.6B-4bit-DWQ` on a local omlx. Cosine between the question vector and 141 "name + one-line description" vectors, nearest neighbour. **It has no refusal mechanism** — cosine always has a nearest neighbour, so refusal needs a threshold; that threshold is fitted on the even-numbered questions and evaluated on the odd-numbered ones (otherwise the same data both tunes and scores). |
| **deepseek-flash** | `future/deepseek-flash`, with **exactly the same prompt and the same list** as the reference answers, only a different model — answering "could any chat model do this?". |

Both LLM calls ran in an empty directory with `--no-tools`, so the agent's own skill catalogue or
project memory could not leak into the prompt.

### 2.7 Scoring: top-1 only

Each system may name up to 3 skills, but **only the first is scored**:

- **decision agreement**: refuse when refusal is right, and when a skill is right have the first choice
  match the reference;
- **first-answer correct**: on the 65 answerable questions, the first choice matches the reference;
- **correct refusal**: on the 35 unanswerable questions, it actually refused;
- **false refusal**: it refused on one of the 65 answerable questions.

Scoring by set equality is not used, because the reference usually names only one skill and set
comparison would treat "a longer list" as an error. The quality of the gold itself (whether it covers
positions 2 and 3) is not scored.

There is also a criterion **independent of any LLM judgement**: for those 65 generated questions, the
skill the question was written from (`source_skill`), which passes through no model at all.

---

## 3. Results

### 3.1 Decision quality

**All 100 questions** (65 answerable / 35 unanswerable)

| system | decision agreement | first-answer correct | first answer ∈ reference set | correct refusal | false refusal |
|---|---|---|---|---|---|
| **Jev (one call, shipped)** | 93.0% | 90.8% | 93.8% | 97.1% | 1.5% |
| Jev (two calls, removed) | 96.0% | 95.4% | 96.9% | 97.1% | 1.5% |
| deepseek-flash | 95.0% | 93.8% | 98.5% | 97.1% | 0.0% |
| embedding (cosine ≥ 0.56) | 67.0% | 64.6% | 72.3% | 71.4% | 16.9% |
| embedding (nearest neighbour, never refuses) | 50.0% | 76.9% | 84.6% | 0.0% | — |

False suggestions (suggesting a skill where refusal was right): Jev 1/35 (`p037`), deepseek-flash
1/35, embedding 10/35.

**The held-out odd half** (50 questions; the embedding threshold was not fitted on this half)

| system | decision agreement | first-answer correct | correct refusal | false refusal |
|---|---|---|---|---|
| Jev (one call) | **96.0%** | 93.8% | **100.0%** | 0.0% |
| deepseek-flash | 94.0% | 90.6% | 100.0% | 0.0% |
| embedding (cosine ≥ 0.56) | 62.0% | 65.6% | 55.6% | 18.8% |

**Independent criterion: the skill the question was written from** (65 questions, through no model)

| system | first choice = generating skill |
|---|---|
| Jev (two calls, removed) | 98.5% |
| Jev (one call, shipped) | 93.8% |
| deepseek-flash | 93.8% |
| embedding (nearest neighbour) | 80.0% |

**The "first answer ∈ reference set" column**: 12 of the 65 questions have reference answers listing
several acceptable skills, while "first-answer correct" only compares against the reference's first.
The gap between the two (93.8% vs 90.8%) comes from `p019` (predicted `biopython`, reference
`["benchling-integration", "biopython"]`) and `p052` (predicted `rdkit`, reference
`["datamol", "rdkit"]`). Both are listed so that a scoring difference is not mistaken for a quality
difference; the rest of the report uses the stricter one (top-1).

**⚠️ The precision ceiling of these numbers**: the same one-call code gave 93.8% / 93.8% / 90.8%
first-answer correct in three independent runs. So "Jev vs deepseek-flash" **cannot be resolved** at
this question count; §5.6 has the full measurement.

### 3.2 The one-call version's 7 wrong answers

| question | reference | Jev | nature |
|---|---|---|---|
| which tool for 3x3 matrix inversion + numerical integration | refuse | `matlab` | **false suggestion** (the only one) |
| a scalable cloud pipeline for RNA-seq steps locally | `latchbio-integration` | `dnanexus-integration` | sibling platforms confused (**the removed second call fixed it**) |
| see recent papers on a tracked project | `paperzilla` | `pyzotero` | sibling tools confused (**the removed second call fixed it**) |
| a lab robot script for a 96-well gradient dilution | `opentrons-integration` | `pylabrobot` | sibling tools confused |
| candidate drug compounds from a batch of SMILES | `datamol` | `rdkit` | the reference lists two (`datamol, rdkit`); the prediction is inside the set |
| export GenBank from a cloud R&D platform | `benchling-integration` | `biopython` | the reference lists two; the prediction is inside the set |
| where protein folding stands today | `future-web` | refused | **false refusal** (the only one; the gate stopped it) |

### 3.3 Are those wrong answers the recommender's fault, or the evaluation's?

Each was traced, and **only 3 of the 7 are real recommendation errors**:

**Two are artefacts of the scoring convention.** `p019` and `p052` both predicted inside the reference
set (see §3.1), just not the reference's first. `p052`'s own `datamol` description reads "Preferred for
standard drug discovery including SMILES parsing", so both answers stand up.

**Two were fixable by the removed second call** (`p035`, `p043`) — the price this decision paid.
`p035`'s question wording shares vocabulary with the winning skill's SKILL.md body (python / cloud /
automatic / no-code) — see the caveat at the end of §2.2.5.

**One is labelling ambiguity.** `p037` "which tool for matrix inversion + numerical integration": the
user asks *which tool to use*, and the reference judged "no skill needed". The Choice called `matlab` a
skill both times, which is not absurd; refusing it would need the gate at 0.96, which would falsely
refuse a dozen questions that should be answered (§5.3).

**Two are sibling libraries that the question itself cannot separate.** `p042` (two lab-robotics
libraries; the re-check put the winner only 0.01 ahead of the runner-up, and the question **does not
mention the hardware brand**), `p035`/`p043` (two cloud platforms, two reference managers). One more
structural weakness: the 65 questions were written from a particular skill's documentation, so the
reference is naturally biased towards that skill; when a question cannot separate sibling tools, which
one is "wrong" is undecidable. **The improvement is not in the recommender but in giving the questions
discriminating information** (p042 should have named the hardware brand).

### 3.4 What the removed call did (same 65 questions)

Counting only the questions the gate passed, how often the two calls agreed on rank 1, and the net
effect of its corrections (`bench/stage2-diff.mjs`):

| | count |
|---|---|
| both calls picked the **same** first place | 62/65 = 95.4% |
| corrected | 3 (p019, p035, p043) |
| broke | 0 |
| turned an answerable question into a refusal | 0 |
| refused on its own (fit threshold) | 0 |

So what it bought was **+3 questions (+4.6 percentage points first-answer correct)**, at the price of one
more network round-trip per request (p50 +284 ms, p90 +186 ms) and +10.1% cost; and 2 of those wins
depend on wording from the SKILL.md body (§2.2.5).

## 4. Cost and latency

### 4.0 Pricing basis

| system | how it is priced | conversion |
|---|---|---|
| Jev | TypeSafe's published rate **$0.042 per million input tokens, output tokens free** | converted to CNY at **¥7.2/$** (an assumed rate; a different rate scales the whole column) |
| deepseek-flash / kimi-k3 | the Future platform bills in **credits**, **1 credit = ¥1** | already CNY |
| embedding | local omlx, no API fee | token counts only; electricity and hardware depreciation are not counted |

Token counts come from the `usage.input_tokens` returned by each call, and cost is computed from the
table above.

### 4.1 Cost per question (median)

| system | input tokens | cost/question | basis |
|---|---|---|---|
| **Jev (one call)** | **8,765** | **¥0.0027** ($0.00037) | published $0.042/Mtok, output free |
| Jev (two calls, removed) | 10,221 | ¥0.0031 ($0.00043) | same (plus the other call's fit nouls) |
| deepseek-flash | 12,531 | **¥0.0249** | platform credits (1 credit = ¥1); ¥2.49 for 100 questions |
| embedding (local) | 41 | local compute | omlx `usage.prompt_tokens` |
| reference answers, kimi-k3 (build-time, not serving) | 25,189 | ¥0.5163 | platform credits (1 credit = ¥1) |

Three points must be made:

- **Jev uses 30% fewer input tokens than deepseek-flash**, because the whole skill table is written once
  in the option table (≈8.8k tokens) instead of one question per skill. But its cost still **grows
  linearly with the catalogue** (a longer option table costs more), and a Choice accepts at most **255
  options** (measured; none occupies one), so a catalogue above 254 must be split (the code keeps that
  path — see §2.2.1).
- **Converted to the same unit, Jev is about 9× cheaper than deepseek-flash** (¥0.0027 vs ¥0.0249). That
  conclusion depends on the assumed ¥7.2/$ rate.
- **Almost all the cost is in that one call**: the only part that could be saved (the removed second
  call) is 10.1%, and §4.5 explains why it is not worth saving.

### 4.2 Total cost and time for 100 questions

| system | total input tokens | total cost | total time |
|---|---|---|---|
| **Jev (one call)** | 876,252 | **¥0.265** ($0.0368) | 63.9 s |
| Jev (two calls, removed) | 975,160 | ¥0.295 ($0.0410) | 87.1 s |
| deepseek-flash | 1,253,100 | **¥2.49** | 182.2 s |
| embedding (local) | 4,100 | local compute | 1.4 s |
| reference answers, kimi-k3 | 2,518,900 | ¥51.63 | — |

Scaled to 100,000 recommendations per day (median cost per question × 100k): Jev about **¥265/day**
(¥947/day for the original 141-noul version); deepseek-flash about **¥2,490/day**.

### 4.3 Latency (one full decision per question)

| system | p50 | p90 | p95 | mean | vs Jev |
|---|---|---|---|---|---|
| **Jev (one call)** | **488 ms** | **1224 ms** | **1401 ms** | 639 ms | — |
| Jev (two calls, removed) | 772 ms | 1410 ms | 1672 ms | 871 ms | 1.6× |
| deepseek-flash | 1,747 ms | — | — | 1,822 ms | 3.6× |
| embedding (local) | 15 ms | — | — | 14 ms | 0.031× |

Jev is the fastest of the three; embedding is about 30× faster and suits instant recall while typing.

**One counter-intuitive point**: the single remaining call asks about 141 skills (8.8k tokens) while the
second call asked about 3 candidates (1.5k tokens) — **a 6× difference in tokens, yet the same order of
magnitude in time** (p50 488 vs 272 ms). That suggests this endpoint's time is dominated by **the fixed
cost of one round-trip**, not by tokens. It also explains why removing a call buys far more latency
(p50 −284 ms) than cost (−10.1%).

(These are single-run readings and drift between runs; read the ratios, not the absolute values.)

### 4.4 What the one call's cost consists of

There is no longer a "how much does each of the two calls cost" question — the serving path is one call,
and the cost is its cost:

| stage | median/question | mean/question | 100 questions | share | cost (100 questions) |
|---|---|---|---|---|---|
| routing (1 Choice, 141 options + none) | 8,765 | 8,763 | 876,252 | **100%** | $0.0368 |
| ~~the second call~~ | — | — | ~~98,948~~ | — | ~~$0.0042~~ |

The 34 questions the gate refused pay this one call as well — there is no second call to save.

**Basis note**: the evaluation script `predict-jev.mjs` still probes the removed call on every question
so that §4.5's comparison can be reproduced; that is **evaluation overhead** (median 10,250
tokens/question), not serving cost. The shipped row in `dataset/score.json` is priced on the serving
basis.

### 4.5 The full evidence for removing the second call

The three-way comparison is in §2.2.5; here are the other measurements behind that decision (all
recomputed offline from the cache by `bench/drop-stage2.mjs` and `bench/stage2-budget.mjs`, with no
further API calls):

**a) That path recovers 3 questions, and not by luck.** The two calls agreed on rank 1 for 62/65, and
all 3 corrections were right, none was wrong, and none turned into a refusal — its record on these
questions is 3:0. That is a genuine paired gain (the same stage-1 answers, not a comparison across
runs).

**b) Its fit threshold never fired, so refusal never depended on it.** 34 of the 35 questions that
should be refused were stopped by the gate, and it did not refuse the remaining one. → **Removing it
cannot make refusal worse**, confirmed in §3.1: both versions' "correct refusal 97.1% / false refusal
1.5%" are identical.

**c) Its advantage is an evaluation bias.** Two of the three wins (p019, p035) have question wording
overlapping the winning skill's SKILL.md body and not its description:

```
p019 question: "...pulls all registered DNA sequences ... from our cloud-based life sciences R&D platform"
     benchling description: "Benchling Python SDK and REST API integration for registry entities, ..." (no cloud platform)
     benchling SKILL.md:    "Benchling is a cloud platform for life sciences R&D."
p035 question: "...scalable cloud pipeline with automatic ... no-code UI ..."
     content words in latchbio's SKILL.md but not its description: python, cloud, automatic, no-code
```

The question set was generated from the SKILL.md, so the questions borrow the body's words — and only
the second call can see the body. **That is an upper bound on the real advantage, not an expected
value** — but it is not purely an artefact either: those words (cloud / python / no-code) are what real
users would say. The honest statement is "1 of the 3 (p043) is clean, 2 are discounted".

**d) Relaxing the routing description length cannot recover it.** 83% of the 141 descriptions are
truncated at 220 chars (median length 329), and removing the truncation adds only about 5,758 tokens
(+66%). But the two questions above win on **words from the body that the description does not contain
at all** (benchling's description never says "cloud platform for life sciences R&D"), so a longer
description cannot recover them. The only way is to put the call that carries the body back — that is,
②.

**e) The candidate count cannot be reduced.** If the second call is kept, it needs at least 3
candidates: re-checking only rank 1 or only ranks 1–2 each lose 2 more questions, because the questions
it corrects are ones where routing put the right answer at rank 2 or 3 (`bench/stage2-budget.mjs`).

### 4.6 Would "the gap between rank 1 and rank 2" be better than the top-1 probability?

Intuitively yes: a distribution where **rank 2 is right behind** looks more suspect than "rank 1 is very
high", and "two candidates are alike" is exactly the shape of the two corrected questions. So replacing
the skip rule from `top-1 probability ≥ T` with `gap between 1 and 2 ≥ T` is a reasonable guess. Measured
result: **the two are exactly equivalent, with no gain.**

**First, the two corrected questions on all three signals** (`stage1-confidence.mjs`):

| question | gold | routing rank 1 | top-1 | gap 1–2 | confidence |
|---|---|---|---|---|---|
| p019 | `benchling-integration` | `biopython` | 0.81 | **0.72** | 0.79 |
| p043 | `paperzilla` | `pyzotero` | 0.40 | **0.04** | 0.38 |

Note p019's gap is 0.72 — **under a gap signal it looks highly confident**, yet it is precisely the one
that needed re-checking.

**To catch both corrections, how few questions must each signal re-check:**

| signal | re-checks needed | threshold |
|---|---|---|
| top-1 probability | **10/66** | 0.82 |
| gap 1–2 | **10/66** | 0.73 |
| `confidence` | **10/66** | 0.80 |
| `none` probability | 59/66 (useless) | 0.03 |

**And all three signals select the same 10 questions** (identical question by question): the differences
in both directions are **0** — "high top-1 but a close rank 2" happens **0** times, and "a large gap but
a low top-1" happens **0** times. A combined rule (`top-1 < T1 or gap < T2`) is worse: 17/66.

**Why they are equivalent, for three reasons:**

1. **The option distribution saturates**: on **31** of the 66 questions the gate passed, rank 2's
   probability is exactly 0, making the gap identically equal to top-1 — the two signals cannot differ
   by construction (and saturation happens because the model puts all its probability on one option, see
   §2.2.1).
2. Across the other 35, the two signals still order the questions almost identically, so they select
   the same set at the operating point.
3. **`confidence`, which was meant to be a third signal, is not an independent number**: a Choice's
   response returns both `probabilities` and `confidence`, and measured **|confidence − the selected
   option's probability| averages only 0.006** — it is essentially the selected option's probability,
   carrying no extra information.

Strictly this is not a mathematical identity: a question like `p1=0.90, p2=0.30`, where rank 1 is high
and rank 2 is also high, would make the two disagree (the gap would re-check, top-1 would skip) — but
**not one of these 100 questions is like that**. So the conclusion is "equivalent on this data", not
"always equivalent"; **`top-1` is kept**, being simpler (it is already the quantity the gate uses, and is
already shown in the UI).

## 5. Limitations

### 5.1 The reference answers are LLM judgements, and one of the compared systems is another LLM

The reference answers come from `kimi-k3` and one compared system is `deepseek-flash` — both doing the
same classification task, where **agreeing with each other is their strength**, while Jev is a small
decision model for which "agrees with another large model" is an unfavourable yardstick. A real verdict
needs human labelling.

**A stronger limitation: the reference itself carries 1–2 questions of noise.** §2.5 measured its own
wobble (first answer 99/100 identical, **the full set only 84/100**), and this report's error analysis
uses the set. Two concrete consequences:

- **`p037` ("which tool for matrix inversion") flipped between "refuse" and `['matlab']` across the two
  generations** — that single question decides whether it is a "false suggestion" or "correct". §3.3 says
  its "whether it should be refused is itself doubtful", and now there is direct evidence: **the
  reference answers do not agree with each other.**
- **`p019` came out as `['benchling-integration']` in one generation and `['benchling-integration',
  'biopython']` in the other** — the former scores Jev's `biopython` as wrong, the latter as "inside the
  set". §2.2.5's "the removed call recovers 3 questions" is 3 or 2 depending on which reference version
  you use.

So any conclusion at the scale of 1–2 questions, whether it comes from a system or from the reference,
is inside the noise; that is why the report always states whether a difference is a paired comparison
within one batch of records.

### 5.2 The question set is still on the easy side, and one-sided

All 65 generated questions are positives ("this skill really should be used"); none is a borderline case
where the skill exists but general help is better. Against the criterion of the skill the question was
written from, the one-call version's first choice hits **93.8%** (61/65; the two-call version 98.5%),
which says the questions are moderate to easy.

### 5.3 Refusal has a known ceiling

The one question that should have been refused but was not is "which tool for 3x3 matrix inversion +
numerical integration": the gate gave it a very low none probability (0.02), so `matlab` was recommended.
Its fit in the re-check was 0.95, while questions that should genuinely be recommended start at 0.76 with
a 10th percentile of 0.94 — **the two classes overlap in that range**, and refusing it would falsely
refuse a dozen questions that should be answered (offline threshold sweep: at a refusal threshold of 0.96,
14 false refusals, trading first-answer correct from 95.4% down to 75.4%). And whether that question
should be refused at all is itself doubtful — see §3.3.

### 5.4 Statistical precision

With 65 positives and 35 negatives, the 95% confidence intervals (Wilson) are roughly:

| metric | point estimate | 95% CI |
|---|---|---|
| decision agreement (n=100) | 93.0% | [86.1%, 96.6%] |
| first-answer correct (n=65) | 90.8% | [81.2%, 95.8%] |
| first answer ∈ reference set (n=65) | 93.8% | [85.2%, 97.6%] |
| correct refusal (n=35) | 97.1% | [85.5%, 99.5%] |

(The two-call version's first answer is 95.4% = 62/65, CI [87.3%, 98.4%]; embedding's nearest neighbour
is 76.9% = 50/65, CI [65.4%, 85.5%].)

### 5.5 Cost basis

- Jev is priced from the published rate; an actual bill may differ by account tier or gateway.
- The two chat models expose only platform credits (1 credit = ¥1), which cannot be converted to USD.
- Embedding's "¥0" means **zero marginal API cost**, excluding hardware and electricity; a hosted
  embedding API billed per token would still be under a hundredth of Jev.

### 5.6 Run-to-run noise in the same code (read before trusting any comparison)

This is the limitation in this report that is **easiest to overlook and most consequential**. `runs/`
holds three independent records of **the same request** (`bench/stage1-stability.mjs`); their only
difference is Jev's sampling:

| comparison | same first answer | same decision (recommend/refuse) | each run's first-answer correct |
|---|---|---|---|
| earlier vs another | 91/100 = 91.0% | 99/100 = 99.0% | 93.8% vs 93.8% |
| earlier vs this run | 91/100 = 91.0% | 97/100 = 97.0% | 93.8% vs 90.8% |
| another vs this run | 90/100 = 90.0% | 98/100 = 98.0% | 93.8% vs 90.8% |

Three consequences:

1. **The same code gives a different first place on about 9% of questions between runs**, and flips
   "recommend/refuse" on 1–3%.
2. So **first-answer correct itself wobbles by about ±2 questions (±3 percentage points)** — which is why
   §3.1 writes "Jev vs deepseek-flash" as "cannot be resolved".
3. **But it does not affect paired comparisons**: every "adding X gained N questions" comparison in this
   report is made on **the same batch of recorded stage-1 answers** (e.g. §3.4, §4.5), so those
   differences are immune. Comparing two independent runs ("90.8% this time vs 96.9% last time") is what
   would be invalid, and the report never does it.

The wobble above is on the **recommender** side; the reference side wobbles by a similar amount (the full
set matches on 84/100, §5.1). Together they are the precision ceiling for every "1–2 questions"
conclusion here.

One concrete example: in this run `n032` ("where protein folding stands") had a gate none probability of
0.15, **exactly on the threshold**, flipping from "recommend future-web" to "refused"; the previous run
had 0.12 and let it through. Only a handful of questions sit near the threshold, but they decide the
1–2 percentage points.

(`p035` also changed first place: `dnanexus-integration` once and `latchbio-integration` the other, with
the same correctness both times — both are outside the reference set.)

## 6. Conclusions

1. **Jev is usable for this task, and fast, cheap and explainable**: one call gives 90.8% first-answer
   correct (93.8% counting "predicted inside the reference set"), 97.1% correct refusal, 0.49 s and
   ¥0.0027 per question, with refusal being a probability you can show the user.
2. **It did not beat the general chat model**: 93.0% vs 95.0% decision agreement. But the two pay
   differently — Jev's refusal is **an explicit threshold** (explainable, adjustable, auditable), a chat
   model's refusal is prompt behaviour (not adjustable, only re-askable as a whole); and Jev is 3.6×
   faster and about 9× cheaper.
3. **"One fewer network call" has a price, and the price is quantifiable**: removing the second call
   buys p50 −284 ms, p90 −186 ms and −10.1% cost, at the price of 3 questions (4.6 percentage points).
   The full three-way comparison is in §2.2.5, where "only when unsure" (20/65) **strictly beats** full
   removal on this data — this version ships with the "one call only" decision, and restoring that
   middle path is a small change.
4. **Refusal is entirely done by that one call**: the gate's none probability refused 34/35 of the
   questions that should be refused, while the removed call's own fit threshold never fired across 100
   questions. Hence both versions' refusal metrics are identical (97.1% / 1.5% false refusals).
5. **Embedding cannot be used on its own**: it ranks acceptably (nearest-neighbour first-answer correct
   76.9%) but has no "should I recommend anything" mechanism, and any threshold brings a large number of
   false refusals (at ≥0.56: 16.9% false refusals, only 71.4% correct refusal). Its right place is as a
   **recall layer**.
6. **Saving money comes from changing the request shape, not from adding components**: the original 141
   nouls cost 21,357 tokens per question, and "one Choice + a none option" brought it to 8,765 (−59%)
   without adding a single dependency. The payload should carry only fields that truly inform: `tagline`
   (a prefix of `description` for 141/141) and `chinese_description` (zero questions changed end to end
   when removed) have both been dropped.
7. **Differences in this report must be read as paired comparisons**: the same code changes its first
   place on about 9% of questions and flips a decision on 1–3% between runs (§5.6). Every "X buys N more
   correct answers" conclusion is a paired comparison within one batch of records and is unaffected;
   comparing absolute scores across runs is invalid.
8. **The prompt and option text have no room left** (§2.2.6): 14 variants, 4 batches, paired inside one
   request — none beat the current wording. A shorter question loses 4 refusals (the "choosing none is
   not a cop-out" permission is load-bearing), a neutral phrasing loses 2, describing the none option in
   more detail pushes 2 extra, halving the description loses 4 answers, and sweeping the description
   length across 110/150/180/190/220/240/untruncated shows **220 is exactly the knee**. In other words:
   **what still separates this from the two-call version is missing information (the SKILL.md body), not
   wording** — only "accept the gap" or "put the second call back", with no third prompt-tuning route.
9. **The option table should contain only legal answers to the question being asked** (§2.2.7): adding
   "more than one fits" was never selected (0/100), because the flow needs "which one to call" and it is
   not an answer to that; "the skill I need is not listed" was used accurately (3 for 3 on that kind of
   question), but those three were already refused by the gate, and its only net effect was to **take
   probability away from none and eat the gate's margin**, pushing 1 extra suggestion. The general
   lesson: the gate reads none's **share**, and probabilities sum to 1, so **adding anything to the
   option table silently recalibrates the only refusal signal** — adding options is not neutral.

## 7. Reproducing

```bash
cd scripts/skill_reco/bench
node build-dataset.mjs                      # generate 65 questions with no skill names + 35 hand-written
node check-dataset.mjs                      # independent re-check: no skill name may appear
node ground-truth.mjs                       # kimi-k3 produces the reference answers
node gold-stability.mjs gold-v1 gold-v1-b   # the reference's own wobble (run ground-truth twice)
node predict-jev.mjs                        # Jev's answers + a probe of the removed call (needs FUTURE_API_KEY)
node predict-embed.mjs                      # omlx embeddings
node predict-llm.mjs                        # deepseek-flash
node stage1-ranking.mjs                     # routing alone: top-1/3/5, refusal quality, cost
node stage1-stability.mjs                   # run-to-run noise (§5.6 — read this before any comparison)
node stage1-prompt.mjs --batch A|B|C|D|E|all # prompt/option-text variants (§2.2.6, paired in one request; E sends two tables)
node option-limit.mjs                       # how many options a Choice really accepts (255; none occupies one)
node option-limit.mjs 253 254 255 256       # sweep the boundary
node desc-cost.mjs 110 220 256 300          # the price of each description length (§2.2.6 batch D)
node drop-stage2.mjs                        # one/two/intermediate shapes, paired (§2.2.5, §4.5)
node escape-options.mjs                     # escape options (more than one fits / not listed) (§2.2.7)
node ctx-limit.mjs                          # how many option tables fit in one request (the paired method's premise)
node stage2-diff.mjs                        # per-question diff for two calls + anatomy of each wrong answer (offline)
node stage2-budget.mjs                      # whether the candidate count can be reduced (offline)
node stage1-onechunk.mjs                    # routing shape: raw single-Choice responses (for offline threshold sweeps)
node onechunk-compare.mjs                   # single Choice vs 141 nouls and the chunked variants
node chunked-cv.mjs                         # 5-fold held-out validation for chunk sizes and both designs
node stage2-zh.mjs --ab                     # paired A/B for the Chinese description (inside one request)
node stage1-zh.mjs                          # the negative control: Chinese description in the routing option table
node stage1-compact.mjs --ladder            # historical: the compaction ladder for the retired noul routing
node compare-runs.mjs <dirA> <dirB>         # per-question diff between two runs
```

Each step caches by question id under `runs/<step>/`, so a re-run only fills in what is missing; the
cache carries a configuration fingerprint (the two thresholds plus the request-shape version), so a
configuration change re-runs instead of mistaking an old answer for a new result. After editing question
text, clear the affected caches first with `node drop-changed.mjs <steps...>`.


