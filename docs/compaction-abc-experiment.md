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
| **M** | summary + recent tail; covered originals dropped | `origin/main` at experiment time |
| **Codex** | **all user messages (≤20 000 tokens) + summary**; no assistant text, no tool output | `openai/codex` @ `b13164d8`, `compact.rs::build_compacted_history` + `templates/compact/prompt.md` |
| **OpenCode** | **summary + retained tail** (`min(15 000, max(2 000, usable/4))`), tool output truncated to 2 000 chars when summarized | `anomalyco/opencode` @ `e03db9bc`, `session/compaction.ts` + `session/message-v2.ts` |

Codex and OpenCode are reimplementations of the selection rules read from those
commits, not forks; the prompts were transcribed verbatim and every file's git
blob SHA is recorded in [abc_external_provenance.json](../scripts/abc_external_provenance.json).
Not modelled: Codex remote-v2 compaction (server-side, needs an OpenAI-hosted
provider) and its `Feature::TokenBudget` path (fresh context window, no summary);
OpenCode's `compaction.prune` (disabled unless configured).

## Results

Closed-book, 48 fields per stage (2 chains × 2 models × 12 fields):

| Arm | stage 1 | stage 4 | stage 8 | total | mean context |
|---|---:|---:|---:|---:|---:|
| A | 41/48 | 43/48 | 42/48 | 126/144 | 14.1 K |
| B | 21/48 | 26/48 | 26/48 | 73/144 | 12.7 K |
| C | 42/48 | 44/48 | 44/48 | **130/144** | 14.1 K |
| M (`origin/main`) | 43/48 | 38/48 | 36/48 | 117/144 | 5.4 K |
| **Codex** | **46/48** | **47/48** | **47/48** | **140/144** | **1.6 K** |
| OpenCode | 43/48 | 44/48 | 33/48 | 120/144 | 5.2 K |

Context sizes are harness estimates, not provider tokens (see limits).

Four findings:

1. **Codex wins on both accuracy and context size.** It reads the entire history
   when summarizing and keeps every user message, so its projection is ~1.6 K
   tokens yet it answers 140/144. Nothing else comes close on this fixture.
2. **Summary-only retention degrades across repeated compactions; C does not.**
   M is the best of A/B/C/M at the first compaction (43/48) and falls to 38 and 36
   as each summary must carry more. C stays flat (42/44/44) because its evidence
   is rebuilt from the journal on every round instead of being re-summarized.
   OpenCode shows the same decay, sharply: 43 → 44 → **33**.
3. **Deleting the summary is still the worst option (B, 73/144).** Its losses are
   tool facts that never appeared in user or assistant prose.
4. **C is the cheapest that holds up.** It costs nothing per compaction and stays
   within 10 fields of Codex over 144.

## Cost

Measured on the live provider through the platform's `credit_cost`, current ledger:

| Group | Requests | CNY | Per compaction |
|---|---:|---:|---:|
| A/B/C (compaction only) | 33 | 1.23 | A **¥0.037**; B and C **¥0** |
| A/B/C (all probes) | 690 | 6.17 | — |
| M (`origin/main`) | 260 | 1.88 | ¥0.018 |
| **Codex arm** | 226 | **11.14** | **¥0.50** per compaction (10.2 M input tokens) |
| OpenCode arm | 245 | 1.75 | ¥0.023 |
| **total** | **1454** | **22.18** | |

Cost per compaction is what the retention rule costs to *maintain*; it excludes the
ordinary model requests and any retrieval the agent performs afterwards
(search-enabled probes cost a further ¥0.0036–0.0101 per request). A/B/C's 690
probes cover three retrieval interfaces, so they are not comparable to the other
arms' single-interface counts.

### The Codex arm's price is cache-cold, and that is an artefact

See "Prefix caching" below: a compaction request shaped like Codex's hits the
provider cache ~99.9% and costs **~48× less** than the same 258 K-token read cold.
My driver walks records into a fresh text blob per stage, so consecutive stages
never share a byte-identical prefix and every Codex-stage request was billed as a
miss. **Real Codex appends its summarization prompt to the live message history —
the shape that cached.** Treat ¥0.50/compaction as an upper bound, not Codex's cost.

## Prefix caching

Measured directly on the provider (`scripts/abc_experiment/cache_test.py`,
`cache_probe.py`), one chain, stage 1, DeepSeek Flash:

| Request shape | Input tokens | Cache hit | Billed |
|---|---:|---:|---:|
| conversation as a message array + instruction at the tail | 257,978 | **257,792 (99.9%)** | **¥0.0135** |
| the same conversation, cold | 257,978 | 0 | ¥0.65 (reservation) |
| A-style clipped excerpt, sent twice | 14,846 | 14,592 (98.3%) | ¥0.0145 |

Three consequences:

1. **"Read the whole history" is not intrinsically expensive.** With a shared
   prefix the marginal cost of 258 K tokens is ~¥0.013 — *cheaper than our A
   summary at ¥0.037*, which reads only ~16 K tokens but changes every byte and
   therefore misses the cache every time (A's 33 summaries: 14,592 of 524,196
   tokens cached, **2.8%**).
2. **Compaction design should preserve the prefix.** Appending the summarization
   instruction to the live conversation — rather than flattening and clipping it —
   is what buys the discount, and it works with the summary reading the *full*
   history rather than an excerpt.
3. **This changes the A/B/C cost comparison.** A's advantage over a full-history
   summary was substantially a cache artefact of the experiment, not a property of
   the two designs.

Reproduced under a controlled repeat (`cache_diag.py`): priming the array then
appending an instruction hit **257,792 of 257,943 tokens (99.9%)** and cost
**¥0.0094** versus ¥0.65 for the same prefix cold.

One caveat found late: my first C+summary run primed with `max_tokens=16` and got
0–2% hits, because such a request does not populate the cache. The 99.9% above uses
a normal output allowance. Two earlier identical requests also reported no usage at
all, so their ledger rows are reservations, not settlements.

## Does search help? (same search tool for every arm)

Every arm was then re-probed with access to the **identical** archive CLI —
`future session history search/get` over the same per-stage session, with the same
5-call and 32 KB budget. The search engine is therefore a controlled variable and
only the projection differs.

| Arm | Closed-book | With search (same CLI) | Δ |
|---|---|---|---|
| A | 126/144 · 12/12 delivered | 120/144 · 10/12 delivered | −6 |
| B | 73/144 · 12/12 delivered | 117/144 · 10/12 delivered | **+44** |
| **C** | 130/144 · 12/12 delivered | **132/144 · 11/12 delivered** | **+2** |
| M (`origin/main`) | 117/144 · 12/12 delivered | 107/144 · 9/12 delivered | −10 |
| Codex (hybrid) | 140/144 · 12/12 delivered | 120/144 · 10/12 delivered | −20 |
| OpenCode (hybrid) | 120/144 · 12/12 delivered | 107/144 · 9/12 delivered | −13 |

Undelivered probes count as zero in these totals. The decisive column is how many
probes *finish*: with search, every delivered answer is near-perfect.

| Arm | Closed-book, delivered only | With search, delivered only |
|---|---|---|
| A | 126/144 | 120/120 (100%) |
| B | 73/144 | 117/120 (97.5%) |
| **C** | 130/144 | **132/132 (100%)** |
| M | 117/144 | 107/108 (99.1%) |
| Codex | 140/144 | 120/120 (100%) |
| OpenCode | 120/144 | 107/108 (99.1%) |

**Search equalises accuracy and turns the comparison into one about budget
exhaustion.** Once an arm can look things up it answers essentially perfectly, and
the only remaining difference is how often it burns its five calls without
producing an answer:

* **Adding search helps the information-poor arm most** (B, +44) and adds a small
  amount to C (+2), which already carried the facts it needed.
* **Adding search *hurts* the information-rich arms.** Codex loses 20 fields and
  two deliveries: with a dense projection it searches for detail instead of
  answering, and exhausts the call budget. OpenCode (−13) and `origin/main` (−10)
  behave the same way. This is an artefact of a fixed request budget — with
  unlimited calls the accuracy would likely converge upwards — but a call budget
  is the realistic condition.
* **C is the only arm that delivers 11 of 12 probes with search**, i.e. it spends
  the fewest calls: its evidence index already pins the entries worth reading, so
  lookup is targeted rather than exploratory.

So the honest answer to "does C + search beat Codex" is **it depends which Codex**:

| Comparison | Winner |
|---|---|
| C + search vs Codex **+ search** (same tooling) | **C**, 132/144 vs 120/144 (and 11/12 vs 10/12 delivered) |
| C + search vs Codex **closed-book** | **Codex**, 140/144 vs 132/144 |
| C closed vs Codex closed | Codex, 140/144 vs 130/144 |

C+search beats Codex when the search tool is held constant, because C spends its
lookup budget better. It does not beat Codex's closed-book score, because Codex's
summary is written by a model that read the whole history, and on this fixture
that alone is worth more than a 2 K evidence index plus lookups.

**The Codex-with-search column is a hybrid, not Codex's own capability.** Codex's
built-in `history` tools are gated behind the hosted backend and a paid ChatGPT
plan (see above), so this arm is Codex's projection driven by *our* CLI. It shows
what the projection is worth when a search tool is present; it is not a
measurement of Codex's product.

## Retrieval interfaces compared

Search is not one thing. Every arm was re-probed through three different lookup
interfaces, with the *same* projections:

| | ours | Codex | OpenCode |
|---|---|---|---|
| What is searched | conversation archive | conversation archive, grouped into windows | **the working tree — files, not the transcript** |
| Locator returned | `entryId` + **byte** offset | `window_id` + `item_id` + **`next_offset_chars` cursor** | file path + line number |
| Search semantics | literal, case-insensitive ASCII | literal, **case-sensitive** | **regex** (ripgrep) |
| Reading | fixed-size chunks, `--limit` ≤ 32768 | `offset_chars` / `limit_chars`, caller-chosen | line ranges |
| Snippet returned by search | head+tail chunk at the match offset | `truncated_content` (placement chosen by the server — see below) | matching lines |

> **Correction.** An earlier revision of this page reported that the Codex
> interface collapsed to 11/144 and blamed the absence of a match position. That
> number was an artefact of **two defects in my implementation plus my own
> 5-request cap**: I had not implemented `read_item`'s documented
> `next_offset_chars` cursor, and I had invented a 5-call limit that no agent has.
> Both are fixed; the corrected results are below.

### Uncapped, corrected contract

| Arm | ours | Codex interface | OpenCode interface |
|---|---|---|---|
| A | 121/144 · 1 lost · 13.8 req | — | — |
| B | 107/144 · 1 lost · 8.2 req | — | — |
| **C** | **132/144 · 1 lost · 3.7 req** | **132/144 · 0 lost · 8.8 req** | **132/144 · 0 lost · 4.4 req** |
| Codex projection | — | **143/144 · 0 lost · 8.0 req** | — |
| M (`origin/main`) | — | 119/144 · 1 lost · 9.4 req | — |

Cells are correct/144 · probes lost · mean model requests per probe. Uncapped runs
so far cover the Codex interface for all six arms and all three interfaces for C;
the remaining combinations are still the capped numbers in the table further down.

**C is interface-independent: 132/144 on all three.** Once each interface is
implemented correctly and the model is allowed to finish, the lookup tool stops
being the deciding factor.

**The best result in the whole study is Codex's own projection read through Codex's
own interface: 143/144, zero probes lost.** It costs more rounds (8.0 vs our 3.7)
because its search returns no match position and the model must page with the
cursor, but with the cursor it gets there.

### What the cursor changed

Without `next_offset_chars` the model had to compute its own resume point, and
across the capped run it made **126 `read_item` calls with a guessed non-zero
`offset_chars`** (86000, 86700, 87400, 88007, 88100 — converging by hand on a
marker at ~87 500 characters) before running out of rounds. With the cursor the
same interface reaches 132/144 for C and 143/144 for Codex's projection.

### Snippet placement is unobservable

`search_contents` documents only `max_chars_per_item` → "Maximum characters
returned in each item's `truncated_content`". It does **not** say where the window
is taken from, and `HistoryNotesToolOutput::new` states that "the server applies
the requested output budget before encryption" — so the backend, not the client,
decides. I implemented both `head` and `centered` placement
(`--snippet-mode`-style control in `abc_retrieval.CODEX_SNIPPET_MODE`) rather than
present one guess as the contract. The uncapped numbers above use the default;
placement matters far less once the cursor exists, because the model can page to
any position.

### Capped numbers, for reference

The earlier run, with my 5-request cap and no cursor. Kept because it shows how
strongly an artificial budget can dominate the result:

| Arm | ours | Codex interface | OpenCode interface |
|---|---|---|---|
| A | 120/144 · 2 lost | 46/144 · 8 lost | 75/144 · 5 lost |
| B | 117/144 · 2 lost | 0/144 · 12 lost | 14/144 · 10 lost |
| C | 132/144 · 1 lost | 11/144 · 11 lost | 100/144 · 3 lost |
| Codex projection | 120/144 · 2 lost | 24/144 · 10 lost | 71/144 · 6 lost |
| OpenCode projection | 107/144 · 3 lost | 53/144 · 7 lost | 100/144 · 2 lost |
| M (`origin/main`) | 107/144 · 3 lost | 64/144 · 6 lost | 45/144 · 7 lost |

### OpenCode has no conversation retrieval at all

Its complete tool set — `glob`, `grep`, `read`, `shell`, `edit`, `write`, `task`,
`todo`, `websearch`, `webfetch`, `lsp`, `apply_patch`, `skill`, plus MCP and
plugin tools — contains **no history tool**. Its recovery path is the filesystem:
re-read the project, not the transcript. Implemented faithfully (newest version of
each file only), that mode still reaches 132/144 for C when uncapped, because most
questions are about current state; what it cannot recover is superseded state, and
the `buried` field (3/12 in the capped run) is the direct evidence.

### What this changes

* **"C + search beats Codex" is the wrong frame.** With the interface equalised and
  no cap, Codex's projection scores highest (143/144). C's advantage is not raw
  accuracy but **cost**: it reaches 132/144 — 92% of Codex's score — for **zero
  compaction cost**, and it needs fewer lookup rounds (3.7 vs 8.0) on our interface.
* Search quality is a **property of the interface plus the budget**, not of the
  projection alone. The same projection scored 11/144 and 132/144 under two
  implementations of the same published contract.
* Any claim of the form "strategy X needs N lookups" must state the interface and
  whether the budget was capped.

## Does adding a summary to C help?

Built C+summary: everything C has (protected originals, deterministic evidence,
recent tail) **plus** a model summary, with the summary generated in the
cache-friendly shape — the live material as real chat messages, instruction
appended. 12 probes, uncapped, Codex interface.

| Arm | Score | No usable answer | Requests/probe | Notes |
|---|---:|---:|---:|---|
| **C** | **132/144** | 0 | 8.8 | zero summary cost |
| C + summary | 125/144 | 1 | 9.8 | summary ¥0.02–0.25/stage |
| Codex | **143/144** | 0 | 8.0 | whole-history summary |

**Adding a summary made C worse (132 → 125), and one GLM probe never terminated**
— it burned all 60 rounds of the runaway guard on `analysis` stage 8 and produced
no answer. The summary helped exactly where expected (`buried` misses 8 → 5) but
cost accuracy elsewhere (`latest_version`, and one probe lost entirely).

**This verdict applies to a *naive* summary, not to summaries in general.** See
"Can we copy Codex's "full"?" below: a summary that is *sticky* (each one carries the
previous one forward) and refreshed on an *overflow trigger* reaches 72/72, matching
Codex. What fails here is bolting a one-shot summary onto a projection that already
carries evidence: the summary becomes a second, contradictable account of material the
evidence index already covers, and it pulls the model toward searching instead of
answering. The naive variants rebuilt each summary from scratch, so nothing accumulated
and `buried` improved only modestly while other fields dropped.

### What remains the honest gap

Codex is still 11 fields ahead, and its advantage is **the summary reads the whole
history** (258 K tokens at stage 1) rather than a curated excerpt. Our summaries
read clipped material. That difference — input breadth, not summary-vs-evidence —
is the untested lever. A summary that reads everything costs ~¥0.0094 when the
prefix is warm, which is now the cheapest known way to buy those 11 fields.

## Can we copy Codex's "full"?

Partly — and the useful part is not "read everything".

**Reading the archive does not work.** C keeps the whole journal, so "everything"
means 255 K tokens at stage 1 and 2 080 K at stage 8; the provider rejected the
request from stage 4 on. Codex can summarise "everything" only because its
compaction **replaces** history, so its live context stays under ~690 K.

The working analogue is the **live** context, chained as Codex chains it:

```
live(0) = archive + tail                    -> summary_0
live(N) = [summary_{N-1}] + records since   -> summary_N
projection(N) = C's projection + summary_N
```

Two necessary conditions, both found by failing at them first:

* **the summary must be sticky.** My first version rebuilt every summary from C's
  projection, so its own previous summary was never carried forward; the stage-0
  value was gone by stage 4.
* **compaction must be overflow-triggered.** My second version compacted only at
  probe boundaries, so four stages of records (~1 M tokens) accumulated and the
  stage-8 request was rejected.

With both, on the same six probes (DeepSeek, both chains, stages 1/4/8):

| Arm | Score | No answer | Requests/probe |
|---|---:|---:|---:|
| **C + sticky summary, Codex trigger** | **72/72** | 0 | 7.3 |
| Codex (user messages + summary) | 72/72 | 0 | 7.8 |
| C (evidence only) | 70/72 | 0 | 8.5 |
| C + summary (clipped input) | 70/72 | 0 | 6.2 |
| A (originals + summary) | 69/72 | 0 | 9.3 |
| M (`origin/main`) | 53/72 | 1 | 7.8 |
| C + summary (non-sticky) | 55/72 | 1 | 11.8 |
| B (no summary) | 50/72 | 1 | 8.2 |

The only field that separates them closes: `buried` goes 4/6 → **6/6** for C, so it
matches Codex field for field. **The missing ingredient was never assistant
messages or a bigger summary — it was a cumulative summary carried forward on an
overflow trigger.**

Cost: ¥7.89 for 8 compactions *uncached* (≈¥0.99 each). Primed, the same request
hits 99.9% and costs ¥0.065–0.085, because a real session already sent that prefix.

## Shouldn't keeping assistant text beat Codex?

**It does — but only when the summary is actually compressed.** The tie reported
above is real, and it is not evidence that verbatim retention is worthless.

Codex drops assistant text from what it *retains*, yet its summary **reads** it:
the summarisation input is the whole live history. The information therefore passes
through once, and survives whenever the summary is large relative to its source.
On the assistant-coverage fixture the agent's own messages were **568 characters
(≈142 tokens)**, the summary reproduced its rationale sentence nearly verbatim, and
both rules scored **17/28** — **8/12** on assistant-only fields.

Two things follow.

1. **The headline comparison could not have detected this.** The original 12-field
   questionnaire contains exactly **one** assistant-only value (`first_code`), so
   "Codex 143/144 vs C 132/144" says nothing about assistant handling either way.
2. **Under compression pressure the difference is decisive.** Give the agent *N*
   distinct decision codes, bound the summary to ~200 words, then ask for 10 of
   them exactly:

| assistant content | assistant tokens | summary output | ratio | ours: verbatim retention | Codex rule: summary only |
|---|---:|---:|---:|---:|---:|
| 40 decisions | 2 174 | 4 088 | no pressure | 10/10 | 10/10 |
| 160 decisions | 8 659 | 564 | **15×** | **10/10** | **0/10** |
| 400 decisions | 21 629 | 549 | **39×** | **10/10** | **1/10** |

While the summary is as large as the material it covers, nothing is lost and the
rules tie. The moment it must compress — 15× and beyond — the exact identifiers go
first, and an exact-code question finds **nothing**. Verbatim retention keeps every
one.

This is the property C already has: assistant prose sits in the protected-original
set and is rebuilt from the journal on every round. The earlier comparison simply
never exercised it.

**Production sits further into that regime, not outside it.** C's evidence slot is
2 K tokens; the Codex arm's summaries ran 1.1–1.5 K tokens against a 258 K-token
input — ratios near 200×. The sweep above locates the transition; it does not
overstate the effect.

## A realistic exam: what real follow-up turns actually ask about

The "shouldn't verbatim retention win" section above used a purpose-built fixture.
This reproduces the question on **real sessions**, after measuring what real
follow-up turns actually refer to.

### The measurement

Classified 254 follow-up user turns across 30 real sessions (local, read-only):

| the follow-up refers to | share |
|---|---:|
| **the agent's own output** | **~80%** |
| repeats the user's own earlier ask | ~4.3% |
| a new requirement | 5.9% |

The classifier counted only explicit pronouns, so it under-counted; the 64% it
marked "other" is mostly about the agent's output, named by object rather than by
pronoun (*"我收回「能不能模糊一下像云」这点"*, *"首页的「查看文档」貌似没放在按钮垂直中央"*).

The original 12-field questionnaire put **11 of 12 facts in user turns or tool
results and 1 in assistant prose** — the inverse of where real questions point.

### The exam

Values drawn from assistant prose (12), user turns (2) and tool results (3), with 8
plausible decoys mixed in; the model marks which appeared. A projection that dropped
a value cannot distinguish it from a decoy. This is a **retention probe**
(recognition, not free recall).

### Chains: 3 synthetic + 3 real

`pipeline` is a new synthetic chain that carries its gold partly in assistant turns
(a decision, its rationale code, a retry budget, a self-correction). The three real
sessions are 756/685/591 records; their ids live in a git-ignored config, so **no
session identifier appears in the repository**.

### Result

C3 = assistant *and* user originals verbatim + tool evidence + tail + summary.
Summary-only = user messages + summary (Codex's rule). **Both use the same sticky
summary**, so the only variable is what is retained.

| chain | C3 | summary-only | delta |
|---|---:|---:|---:|
| analysis | 11/18 | 9/18 | +2 |
| export | 11/18 | 9/18 | +2 |
| pipeline | 10/18 | 9/18 | +1 |
| real-stream | 35/45 | 30/45 | +5 |
| real-visual | 29/36 | 27/36 | +2 |
| **real-yt** | **35/44** | **27/44** | **+8** |
| **total** | **131/179** | **111/179** | **+20** |

| group | C3 | summary-only | delta |
|---|---:|---:|---:|
| real sessions | **99/125** | 84/125 | **+15 (+12.0 pts)** |
| synthetic chains | 32/54 | 27/54 | +5 (+9.3 pts) |

**Zero false positives on both sides**: the gap is purely "can the value still be
confirmed from what survived", not hallucination.

C3 is ahead on every chain, the advantage is **larger on real sessions**, and it is
largest on the session with the most assistant prose. The synthetic chains
under-measure — their assistant turns are short, only 6 values per stage were
scorable, and their C3 projection is unrealistically large (~250 K tokens) because
those fixtures bury 99% of their volume in tool records. **The real-session numbers
are the trustworthy ones.**

This closes the loop: measured where real questions point, verbatim retention of the
agent's own output is worth **+12 points** over summary-only — the direction the
earlier fixture hinted at and the original questionnaire could not see.

## Was the earlier assistant-message conclusion a coverage artefact?

**Yes.** I checked which record kind actually contains each of the original 12 gold
values:

| value lives only in | fields |
|---|---|
| user text | `project`, `first_limit`, `latest_limit`, `format` |
| tool results | `old_version`, `latest_version`, `buried`, `validation`, `blocker` |
| **assistant text** | **`first_code` — one single field** |

A fixture built with four assistant-only facts (a decision, its rationale code, a
retry budget, a self-correction) then compared two retention rules on the same
history, closed-book:

| field | source | with assistant text | without |
|---|---|---|---|
| decision | assistant | ✅ STREAMING | ❌ UNKNOWN |
| rationale_code | assistant | ✅ | ❌ UNKNOWN |
| retry_budget | assistant | ✅ 7 → 3 | ❌ UNKNOWN |
| self_correction | assistant | ✅ | ❌ UNKNOWN |
| project / latest_throughput / format | user | ✅ | ✅ |
| latest_version / buried / validation / blocker | tool | ✅ | ✅ |
| **total** | | **19/24** | **14/24** |

Dropping assistant text loses **all four** assistant-only fields and nothing else.
So "Codex keeps no assistant messages and still wins" was an artefact of a
machine-state-heavy questionnaire, not a property of Codex. Assistant prose carries
commitments nothing else restates: decisions, rationale, retractions, and
unresolved questions. Ask "why did you choose this" and the difference is immediate.

## Validated combination (C3)

The three ingredients above were validated together, against criteria fixed before
the run. **C3 = sticky summary + overflow trigger + verbatim assistant originals.**

| # | Criterion | Result | Verdict |
|---|---|---|---|
| P1 | accuracy ≥ Codex on the same probes | C3 **144/144** vs Codex 143/144 | **PASS** |
| P2 | no probe lost, `buried` ≥ 10/12 | 0 lost, **12/12** | **PASS** |
| P3 | exact identifiers ≥ 9/10 at 15× compression | **10/10** (summary-only: 0/10) | **PASS** |
| P4 | no field regresses against plain C | none | **PASS** |

Full sample, 12 probes (2 chains × 2 models × stages 1/4/8), uncapped, same
interface for every arm:

| Arm | Score | Lost | Requests/probe |
|---|---:|---:|---:|
| **C3 (combined)** | **144/144** | 0 | **4.2** |
| Codex | 143/144 | 0 | 8.0 |
| C (evidence only) | 132/144 | 0 | 8.8 |
| B (no summary) | 107/144 | 1 | 8.2 |

| field | C3 | Codex | C alone |
|---|---|---|---|
| `buried` (value mid-record) | **12/12** | 12/12 | 4/12 |
| `latest_version` | **12/12** | 11/12 | 10/12 |
| everything else | 12/12 | 12/12 | 11–12/12 |

**C3 beats Codex on accuracy and uses about half the lookups.** Each ingredient
contributes something the others cannot supply: the sticky summary gives continuity
that a one-shot summary lacks (55/72 without it), the overflow trigger keeps
full-history summarisation bounded (without it the request is rejected from stage
4), and verbatim assistant originals preserve exact self-reported commitments,
which is all that survives a 15×+ compression. None of the three reached 144 alone.

Cost: ¥14.70 for C3's own 12 compactions plus probes, ≈¥0.9 per compaction *cold*.
Primed, the same request hits 99.9% of the prefix cache and costs ¥0.0094 instead
of ¥0.65 — and a real session has already sent that prefix as ordinary turns.

**Implementation follows.** Two things the validation exposed and the code must
handle: reasoning can consume the entire output allowance and return an empty
summary over a large input (retry under a fresh identity; it happened twice), and
the summary prompt must explicitly ask for a carried-forward historical table or
values are dropped as stages accumulate.

## Does Codex have its own retrieval?

**Yes — and this matters for how the comparison should be read.** Codex ships a
first-party history-retrieval tool set in `codex-rs/ext/history-notes`, whose own
description reads: *"Recover prior conversation after a context-window reset by
listing, reading, and searching normalized history."* The `history` namespace
exposes `list_windows`, `list_items`, `read_item` and `search_contents`; a `notes`
namespace persists private notes across context-window transitions; and a thread
hint is injected into the context automatically.

It is not active in the configuration measured here. All of these must hold:

| Requirement | Value in this experiment |
|---|---|
| `Feature::TokenBudget`, `Feature::ContextManagement` | `Stage::UnderDevelopment`, `default_enabled: false` |
| Model `supports_experimental_context` | n/a via third-party provider |
| Provider `supports_codex_backend_routes()`, `requires_openai_auth`, no env key/provider auth | no |
| Auth mode / plan | must be **ChatGPT** with **Plus, Pro or ProLite** |

The history itself lives on the hosted Codex backend (`alpha/history/v2/*`), not in
local files. So the measured Codex arm is its **local fallback compaction without
experimental context management** — the configuration available to anyone using an
API key or a third-party OpenAI-compatible provider, which is what this experiment
ran. A "Codex + its own search" arm is **not reproducible** with API keys.

Its measured cost is in "Retrieval interfaces compared" above: implementing the documented interface faithfully costs C 121 fields and 10 extra lost probes,
because search returns no match position.

Two consequences:

1. The direction "C + search" is the same direction Codex's paid, hosted product
   already takes: compaction **plus** a way back to the original conversation.
   That is a point in favour of the C+search design, not against it.
2. Any comparison between "C with search" and "Codex" must state which Codex is
   meant. Comparing C-with-search against Codex-without-its-history-tools is not
   a like-for-like test, and the numbers in this page are that latter case.

## How the external arms were run

Codex and OpenCode *replace* the conversation, so the Nth compaction reads the
already-compacted history plus what was added since — not the raw archive that
journal-backed arms legitimately rebuild from. Two compactions therefore exist:

* **probe stages force one** (stages 1, 4, 8), so the recorded projection is
  comparable with the other arms at the same boundary — that is the ablation;
* **between probes the agent compacts on its own overflow trigger**, before the
  accumulated history would exceed the provider window. Without this the arm would
  carry a history no real session can hold: the first attempt at a forced
  stage-4 compaction was rejected with
  *"maximum context length is 1048576 tokens. However, you requested 1072531"*.

That rejection also calibrated the harness: the provider counted 1 064 339 tokens
for messages the harness estimated at ~766 K (~1.45×). The overflow threshold uses
that measured factor. Each arm keeps its own output budget: OpenCode hard-codes
4 096, Codex uses the model maximum (capped at 65 536 here).

## Limits

- Codex and OpenCode are single-point-of-source reimplementations. Their retention
  rules and prompts were read from specific commits; a later upstream change would
  invalidate the numbers.
- **The Codex numbers are its local fallback path.** Its first-party history/notes
  retrieval tools require the hosted Codex backend, a paid ChatGPT plan and two
  UnderDevelopment feature flags, so they are absent in any API-key or third-party
  provider setup — including this one. See "Does Codex have its own retrieval?"
  above before comparing a search-enabled arm against these numbers.
- The probe stages are forced for every arm. Between probes the external arms use
  their own trigger, so their boundary state can be the result of an overflow
  compaction rather than a boundary one; the recorded `compactions` field in each
  projection says which happened.
- Harness token estimates are ~1.45× lower than the provider's count on this
  fixture, so the context column understates real prompt sizes.
- OpenCode hit its own 4 096-token summary ceiling on GLM twice (truncated
  summaries are accepted, as its code does) and once returned an empty summary,
  which OpenCode treats as *declining* to compact — recorded as `declined`, not
  rewritten into a fake summary. That is a real fragility of running a 4 K output
  cap against a large history.
- This is fact recall, not task continuation: it does not measure whether an agent
  can resume the work.
- Two synthetic chains from one blueprint with structured tool records; not two
  independent natural task types, and the questionnaire rewards carrying tool
  evidence.
- The search-enabled numbers come from a fixed 5-call / 32 KB budget. That budget
  is what separates the arms, so a larger or unbounded budget would compress the
  differences; the reported ranking should not be read as a property of the
  projections alone.
- **The Codex-interface column measures our approximation, not Codex.** Three
  things bound it: (a) the documented `read_item` contract carries a cursor
  (`next_offset_chars`) that I did not implement; (b) `HistoryNotesToolOutput::new`
  states "the server applies the requested output budget before encryption", so
  snippet placement is server-side and I cannot observe it — I assumed head
  truncation, and a match-centred backend would score higher; (c) namespaced tool
  names, `encrypted` query arguments and server-side execution are Responses-API
  features that cannot be reproduced on Chat Completions. The 5-request cap is also
  mine, not Codex's — production has no such cap, so "exhausts the budget" describes
  my budget interacting with an interface that needs more rounds, not a weak tool.
  The *interface-shape* finding (no match position ⇒ more rounds) is what this
  experiment supports; the 11/144 figure should not be read as Codex's capability.
- Prefix caching was measured only in isolation (see "Prefix caching"). Its effect
  on the full A/B/C/M comparison is inferred, not re-measured end to end.

- Search-enabled probes were run after the closed-book ones and reuse the same
  projections, so they do not re-measure compaction cost. Retrieval cost is
  reported separately (¥0.0036–0.0101 per request).
- The ledger keeps every failure: 4 invalidated early OpenCode calls (wrong tail
  budget), one interrupted M request that was retried, and Codex attempts rejected
  for context length before the trigger was calibrated.

## Reproducing

See [scripts/abc_experiment/README.md](../scripts/abc_experiment/README.md). Raw
data, fixtures, ledger and scoring stay in the local, git-ignored
`.future/research/abc-summary-a47313/`.
