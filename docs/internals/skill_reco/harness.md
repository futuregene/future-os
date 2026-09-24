# Jev skill recommendation demo

Suggests a skill as you type: given a request, one TypeSafe Jev call (the System One model,
`POST /v1/systemone`) picks the best fit out of **all 141 skills** in the `skills/` submodule
(15 builtin + 126 third-party) — or decides that none of them fit. A 100-question evaluation:
**[evaluation.md](evaluation.md)**.

## Running it

**Prerequisites (two steps; neither is optional)**

```bash
# 1) Get the code, with the submodule — the 141 skill directories come from it
git clone --recurse-submodules https://github.com/futuregene/future-os.git
#    In an existing checkout, fetch main and initialise the submodule instead:
#    git fetch origin main && git checkout main && git submodule update --init skills

# 2) Start it — the credential comes from the Future account
#    (~/.future/agent/auth.json); no arguments needed
cd scripts/skill_reco
node server.mjs                                   # defaults to http://127.0.0.1:8791
```

This line must appear in the startup log. If it does not, step 2 did not take effect and the
UI will be empty:

```
roster: 141 skills (builtin 15 + third-party 126) from .../skills
```

**The credential is the Future account's own key; there is no separate Jev key.** Recommendation
goes through the platform gateway `{future_base_url}/v1/systemone` (currently
`https://future-os.cn/api`) and authenticates as the **signed-in Future account** — so if you have
signed in, you have it, and there is nothing to configure.

About the credential:

- **Read from the `future` entry of `~/.future/agent/auth.json` by default** (the same file the
  agent, desktop app and TUI use), overridable with `FUTURE_API_KEY` for a throwaway key. The
  repository has **no** `.env` and no example key value.
- **The browser never sees it.** The page talks only to the local server; the credential stays in
  the `server.mjs` process and the server forwards the request to the gateway.
- **Do not paste the key into an issue, a log or a screenshot.** `probe.mjs` prints only a mask
  (first four characters + length + origin) so you can tell which key is in use.
- **`.gitignore` already covers the run artifacts**: `runs/` holds the per-question cache from the
  evaluation (including raw API responses) and does not enter the repository.

The UI also starts without a credential: with no `future` entry in `auth.json` and no
`FUTURE_API_KEY`, the server falls back to **local BM25 heuristic ranking** (the page labels it
prominently as "local fallback" — that is not a Jev result), which is enough to look at the UI and
the wire shape.

Other parameters (each with an identically named environment variable): `--port` / `PORT`,
`--skills` / `SKILLS_ROOT`, `FUTURE_BASE_URL` / `FUTURE_MODEL` (the account's `base_url` wins, and
the model is `jev`), `LOCAL_ONLY=1` (force local fallback), `NONE_GATE_THRESHOLD=0.15`,
`CHUNK_SIZE=254`, `SHORTLIST=3`, `MIN_QUERY_CHARS=6`.

`SHORTLIST` **does not participate in the decision** (that reads the top-1 name plus the none
probability): it only sets how many candidates the API hands the UI, i.e. how many candidate cards
are drawn. Setting it to 1 just hides two rows; only 0 breaks the decision (`verdict` reads the
first name). While the removed "re-check the top 3" call still existed, `SHORTLIST` decided **who
got re-checked**; that call is gone.

Command-line self-check (also taking the key from the environment):

```bash
FUTURE_API_KEY=your-key node probe.mjs                         # validate the key + run the whole path
FUTURE_API_KEY=your-key node probe.mjs "extract the tables from this PDF into markdown"
```

In the browser: six or more characters start a recommendation, and the call fires after a pause
(⌘/Ctrl+Enter fires it immediately). The badge in the top-right shows the backend; `本地回退`
("local fallback") means Jev was not used. Three tabs at the bottom: candidates / full ranking /
call details (the real request JSON and the usage).

## How it works

```
the user's request
  │
  └─ one call (routing): a single Choice offering all 141 skills plus none_of_these
        question: "which skill in `criteria` does `request` need, or does none of them help?"
        option text: each option is the skill name → its one-line description
                     (the table is written once, ≈8.8k tokens)
        → answers two things at once: who fits best (probability over real candidates)
          and whether nothing fits (probability on none)
     refusal: none probability ≥ 0.15 → no suitable skill; otherwise recommend the highest
```

**Why a single call.** There used to be a second one: the top-3 candidates, this time with the full
description and the opening of their SKILL.md, re-asked as one noul per candidate (behind its own
fit threshold). On 97% of the questions it passed it merely confirmed the first call's answer, at
the price of one more network round-trip per request. Three shapes, measured as a paired comparison
over the same records (`bench/drop-stage2.mjs`):

| shape | first-answer correct | first answer ∈ reference set | second call | latency p50 | p90 | cost/question |
|---|---|---|---|---|---|---|
| **① one call only (this version)** | 59/65 = 90.8% | 93.8% | **0/65** | **488 ms** | **1224 ms** | **¥0.00265** |
| ② only when unsure (previous version) | 62/65 = 95.4% | 96.9% | 20/65 | 526 ms | 1338 ms | ¥0.00274 |
| ③ always two calls | 62/65 = 95.4% | 96.9% | 65/65 | 772 ms | 1410 ms | ¥0.00295 |

So dropping it costs **3 questions (4.6 percentage points)** and buys p50 −284 ms, p90 −186 ms and
−10.1% cost.

**Note that ② strictly beats ① on this data**: same quality, only 38 ms more median latency —
because the three questions it recovers had a routing top-1 probability of 0.77 / 0.36 / 0.39, all
far below 0.95, so ② caught every one of them. This version ships as "one call only"; restoring ②
is a small change, and the full evidence is in [evaluation.md](evaluation.md) §2.2.5 and §4.5.

**A caveat that belongs next to those numbers**: two of the three (p019, p035) have question wording
that overlaps the winning skill's SKILL.md *body* rather than its description (p019: cloud /
platform / sciences). The reference answers were written from the SKILL.md, so the questions borrow
the body's vocabulary — and only the second call can see the body. That makes the three an **upper
bound** on the real advantage, not an expected value.

**The single-call prompt is as good as its wording gets.** That was measured separately: 14
variants, four batches, compared **inside one request** so run-to-run noise cancels (see below).
None beat the current wording — a shorter question loses 4 refusals (the "choosing it is a normal
answer, not a fallback" permission is load-bearing), a neutral phrasing loses 2, describing the
none option in more detail pushes 2 extra suggestions, halving the description loses 4 answers, and
sweeping the description length across 110/150/180/190/220/240/no-truncation shows **220 is exactly
the knee** (shorter loses answers one at a time, longer buys nothing). See
[evaluation.md](evaluation.md) §2.2.6.

So **what still separates this from the two-call version is missing information, not wording**: those
questions need the SKILL.md body, while the option table only carries descriptions. The only way to
recover it is to put the second call back.

**Why a Choice rather than 141 parallel nouls**: the two have the same quality (5-fold held-out
validation: 63/65 for both) but the Choice costs **about half the tokens** (8.8k vs 21.4k per
question) — a noul has to repeat the question template and criteria 141 times, a Choice writes them
once.

**Why the Choice must offer `none_of_these`**: this is the crux. Jev will not say "none of these",
and a Choice without a none option **must** name a winner. Without one, the only way to decide
whether to suggest at all was a separate "is a skill needed?" question, which produced 8–9 false
refusals (first-answer correct down to 84.6%). Making "none of them" an **option** that competes
against real candidates on the same question made refusal clean (0 false refusals) and put quality
back above 95%.

**Why that is the only escape option**: adding "more than one fits" and "the skill I need is not in
this list" was tried (§2.2.7). The first was **never selected** in 100 questions — the flow needs
*which one to call*, and "several fit" does not answer that question. The second was used accurately
(all 3 selections were questions naming a tool outside the catalogue) but those questions were
already refused by none, while it took probability away from none and ate the gate's margin,
pushing one extra suggestion. **The root cause**: the gate reads none's *share* of the distribution,
the probabilities sum to 1, so **adding an option silently recalibrates the only refusal signal** —
adding options is not a neutral operation.

**Why the default is "everything in one block"**: 141 skills fit in one Choice, and measurement shows
it is **exactly as good as chunking** (18/30/47 per block — both 63/65 held out), while a single
block saves the "re-rank across blocks" call and 9% of the cost. So the default is **254, the largest
number of skills that fits**, meaning "do not split until the API forces it". The one case that needs
chunking is **a catalogue above 254**: a Choice accepts at most **255 options** (measured; see below)
and the none option occupies one. Above that, set `CHUNK_SIZE` (e.g. 18) to enable the chunked path
plus a final Choice — the code is in place and covered by held-out validation; `CHUNK_SIZE=255`
fails at startup with an explanation instead of returning 400 on every request.

**The 255-option ceiling is measured, and none counts against it**: `bench/option-limit.mjs` sends
253/254/255/256 options — 255 passes, 256 is rejected
(`400 Too many choices. Must have at most 255 choices.`). So the cap is **254 skills per block**, not
255.

**Why refusal is a threshold**: Jev only emits probabilities, so "should I suggest anything" has to be
decided by the caller. Making it an explicit threshold keeps it explainable (the UI shows "none
probability 0.71 ≥ 0.15, so refused") and adjustable. The threshold is a wide plateau: 0.1–0.9 give
nearly the same result, because questions that should be refused sit above 0.9 and ones that should
be answered sit near 0, with nothing in between.

| threshold | default | effect |
|---|---|---|
| `NONE_GATE_THRESHOLD` | 0.15 | the Choice's probability on `none_of_these`, at or above which nothing is suggested |
| `CHUNK_SIZE` | 254 (one block) | skills per Choice; capped at 254 (255 options − none), so a catalogue above it must lower this |
| `FITS_THRESHOLD` | 0.75 | **not part of serving**: the removed call's threshold, now defined in `bench/second-call.mjs` |

## Score on the 100 questions

Evaluation details, how the question set was built, the cost basis and the limitations all live in
[evaluation.md](evaluation.md) (100 questions: 65 generated from skill documentation, 35 hand-written
"no skill applies"; reference answers from `future/kimi-k3`; top-1 only).

| system | decision agreement | first-answer correct | correct refusal | false refusal | cost/question | latency/question |
|---|---|---|---|---|---|---|
| **Jev (this demo, one call)** | 93.0% | 90.8% | 97.1% | 1.5% | **¥0.0027** | **0.49 s** |
| Jev (two calls, removed) | 96.0% | 95.4% | 97.1% | 1.5% | ¥0.0030 | 0.77 s |
| deepseek-flash | 95.0% | 93.8% | 97.1% | 0% | ¥0.0249 | 1.75 s |
| embedding retrieval (local omlx) | 67.0% | 64.6% | 71.4% | 16.9% | local compute | 0.015 s |

Counted as "the prediction falls inside the reference set" (12/65 questions have more than one
acceptable answer) the first answer is **93.8%**; against the independent criterion of "the skill the
question was written from", the first choice hits **93.8%**.

**⚠️ The precision ceiling of all these numbers**: three independent runs of the same code gave
93.8% / 93.8% / **90.8%** — about ±2 questions of noise. So "Jev vs deepseek-flash" cannot be
resolved at this question count, and every "X buys N more correct answers" conclusion in the report
is a **paired** comparison inside one batch of records, which the noise does not affect. See
[evaluation.md](evaluation.md) §5.6.

Every wrong answer was traced to its cause ([evaluation.md](evaluation.md) §3.3): only 3 of 7 are
real recommendation errors — two where two sibling libraries fit and the question cannot tell them
apart, one ambiguous "which tool should I use" labelling; two more were actually inside the reference
set (apparent errors caused by scoring only the first answer), and two were recovered by the removed
second call.

Cost basis (1 credit = ¥1; Jev at the published $0.042/Mtok with free output, converted at ¥7.2/$).
How it got here: 21,357 tokens per question with the original 141 nouls → **8,765 (−59%)** after
switching to a single Choice with none, and the serving path is that one call.

## Code layout

```
scripts/skill_reco/
├── server.mjs        local HTTP server: static page + /api/suggest (the key stays in this process)
├── roster.mjs        reads SKILL.md from skills/builtin + skills/third-party into a catalogue
├── suggest.mjs       one Choice + one gate threshold (Jev and the local fallback share the decision)
│                     only the serving path lives here: the removed call moved to bench/second-call.mjs
├── jev.mjs           System One HTTP client (credential from the Future account; 429/529 backoff)
├── localRank.mjs     BM25 literal fallback when no credential is available (labelled; not Jev)
├── probe.mjs         command line: validate the credential + run the whole path
├── public/           front end (no build, no dependencies)
├── bench/second-call.mjs  the removed second call (kept so the evaluation reproduces; unused in serving)
└── bench/            100-question evaluation (question set / reference answers / systems / scoring / report)
                      read stage1-stability.mjs before trusting any comparison: ±2 questions of run noise
                     stage1-prompt.mjs is the prompt-variant experiment (paired inside one request, §2.2.6)
                     check-parity.mjs asserts this directory matches the agent's mechanism item by item
```

Every step under `bench/` caches by question id, so a re-run only fills in what is missing; the cache
carries a configuration fingerprint, so changing a threshold or the request shape re-runs instead of
mistaking an old answer for a new result.

**Two guards; run them after touching this directory:**

```bash
node check-doc-refs.mjs                                     # every script the docs name must exist
AGENT_SKILL_RECO=.../agent/src/skill_reco/mod.rs \
  node bench/check-parity.mjs                               # this directory matches the agent's mechanism
```

**Plus a manual end-to-end pass over the gRPC command** (not in CI: it needs grpcurl, a
real credential and a live gateway, so it costs money):

```bash
future agent --home /tmp/reco-test --verbose --log-file    # isolated; never the agent you use
python3 suggest-skill-tests.py              # 43 checks, 5 suites; ~33 calls ≈¥0.04
python3 suggest-skill-tests.py --only A,B,D # the three free suites (no network)
python3 order-sensitivity.py --runs 8       # order sensitivity vs run-to-run noise
```

The unit tests cover the decision logic; this covers the wire — the typed payload the
clients parse, every failure collapsing to "no recommendation", and the boundaries the
constants imply (the 254-candidate cap, CJK, very long input).

Both exit non-zero on a problem, and — just as important — **fail when they measured nothing** (a doc
they cannot read, an agent source they cannot read) rather than printing a pass. `check-doc-refs.mjs`
found two references to deleted scripts the day it was written, and `check-parity.mjs`'s default path
was itself off by one directory level, working only when `AGENT_SKILL_RECO` was passed explicitly.

## Relationship to the production code

The product implements recommendation in the agent, in Rust (`agent/src/skill_reco/mod.rs`), and all
three clients (desktop / TUI / mobile) call it. This demo is a **second implementation** (JS), so it
is worth being explicit about who owns what.

**The experiment scripts have to stay in JS — that is the design, not laziness.** The 37 scripts under
`bench/` exist to **change the very things production fixes**: prompt wording (loose vs strict),
payload shape (description 220 vs 256, with or without the Chinese line), chunk size, gate threshold,
option sets. If they ran the production code they would have no knobs at all — and then today's
constants (0.15 / 254 / 220) **could not have been derived**. They are the factory that produced
production, not the product.

**The serving path (`suggest.mjs` + `jev.mjs` + `roster.mjs`) is aligned with production, and a guard
watches it.** The two agree today:

| item | demo | production | |
|---|---|---|---|
| endpoint | `{account base_url}/v1/systemone` | same | agree |
| credential | the `future` entry of `auth.json` | same | agree |
| model | `jev` | same | agree |
| gate threshold | 0.15 | 0.15 | agree |
| candidate cap / description length | 254 / 220 | 254 / 220 | agree |
| none option text | identical | identical | agree |
| `instructions` | an identical string | same | agree |
| pricing | `$0.042/Mtok`, `¥7.2/$` | same | agree |

The `instructions` row was once **not** aligned: the demo sent an object `{question, how_to_judge}`
while production sent a single string. The gateway accepts both, so nothing errored — which also
meant the demo was not a faithful stand-in for production and the two could drift silently.

Those values are now compared **item by item, read out of both sources**, by
`bench/check-parity.mjs` (rather than each side asserting its own copy in its own test — two green
tests that disagree are exactly the failure this catches):

```bash
AGENT_SKILL_RECO=/path/to/agent/src/skill_reco/mod.rs node bench/check-parity.mjs
# prints 11 items plus one cap invariant; exits 1 if anything drifts
# the default path is <repo>/agent/src/skill_reco/mod.rs (where it sits once both branches are in main)
```

The guard also checks one **internal invariant**: `CHUNK_SIZE + 1 == MAX_CHOICE_OPTIONS` (254
candidates + 1 none = 255, exactly the API ceiling) — the two numbers live in different files, and
this stops "changed one, forgot the other".

The demo's prices were originally **inlined across nine scripts** (15 occurrences of `0.042` and
`7.2`). They now come from `USD_PER_MTOK_INPUT` / `USD_TO_CNY` exported by `jev.mjs`: money in a
report computed from a stale rate is worse than no money at all, and the rate now has one home,
watched by the guard.

**On "can it just use the production code"**: the serving path could, but the demo would lose its most
useful part — `suggest_skill` returns **only the final skill**, not the top-3, the probabilities or
the gate value (those exist only in the agent's log). The demo UI is built on exactly those numbers to
explain *why* a skill was recommended. Switching the demo to call the agent would mean adding those
diagnostic fields to the RPC (additive, so compatible); without them the UI would be a single line of
conclusion. **The current trade-off: keep the JS serving path (so the ranking and the gate stay
visible) and use the guard above to hold the mechanism to production item by item.**
