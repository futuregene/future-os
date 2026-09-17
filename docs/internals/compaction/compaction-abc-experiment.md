# Compaction strategy comparison

Five retention strategies on six frozen chains, closed book. Four are measured at the
production call shape; the tables further down are an older matched-size run.

**Naming.** The arms are named after the code: `summarized` is the runtime default
(`summarized-evidence-v1`) and `deterministic` is the same projection with no summary model
called (`deterministic-evidence-v1`). The older sections of this document label them **C3**
and **C**; they are the same two strategies.

| Strategy | What survives compaction | Source |
|---|---|---|
| **`summarized`** | every protected user **and assistant** original + a 2 K deterministic tool-evidence index + recent tail + a **cache-friendly handoff summary** written by the session model | this repo (runtime default) |
| **`deterministic`** | the same projection, no summary model called | this repo |
| **`main`** | the algorithm `origin/main` deploys: a recursive model summary, protected originals, a recent tail | this repo, released branch |
| **Codex** | all user messages (≤20 000 tokens) + a whole-history summary; no assistant text, no tool output | `openai/codex` @ `b13164d8`, `compact.rs::build_compacted_history` + `templates/compact/prompt.md` |
| **OpenCode** | a summary + a retained tail (`min(15 000, max(2 000, usable/4))`) | `anomalyco/opencode` @ `e03db9bc`, `session/compaction.ts` |

Codex and OpenCode are reimplementations of the selection rules read from those commits,
not forks. Their prompts were transcribed verbatim and each source file's git blob SHA is
recorded in [abc_external_provenance.json](../../../scripts/abc_external_provenance.json).

**Two shapes are reported, and they answer different questions.** The production-shaped run
below drives every Rust arm through the call shape the runtime uses — registry window, the
model's output reservation, a real request budget, the production trigger and phase — so its
numbers are the ones production computes. The later tables were measured with a **simulated
128K window and no budget**, a matched-size comparison of retention rules. What each shape
can say about the cache, the deployable strategy and the summary's value differs, and every
section below states which shape it used.

## The production-shaped run, and the deployed strategy

The tables further down were measured with a **simulated 128K window, no output
reservation and no request budget** — a matched-size retention comparison, not the numbers
production computes. A later run drives the same strategies at the call shape the runtime
uses (registry window, `effective_max_tokens`, `set_request_budget` with a captured
session prompt and the real tool definitions, production trigger and phase), and adds the
**deployed algorithm** as a fifth arm. Its protocol is
[PRODUCTION_SHAPE_PROTOCOL.md](../../../scripts/abc_experiment/PRODUCTION_SHAPE_PROTOCOL.md);
these are its results.

On this model (declared window 1 000 000, output reservation 384 000) the economic trigger
is **613 952** tokens: 80 % of the window, clamped by `window − output reserve − margin`.
That is above every real-session boundary in the fixture set, so with the production
trigger the Rust strategies frequently do not compact at all. Comparing them against arms
that compact unconditionally would measure the trigger, not the retention policy. Both
views are therefore reported, and only the first is a like-for-like comparison.

**Post-compaction against post-compaction** (every arm forced to compact at every
boundary, 24 boundaries, 178 questions):

| Strategy | Recall | Median projection | Compression | Compaction cost |
|---|---:|---:|---:|---:|
| `summarized` (runtime default) | **147/178 (83 %)** | 12 113 tok | 5.7 % | 7.14 CNY |
| `deterministic` | 127/178 (71 %) | 9 945 tok | 4.7 % | **0** |
| OpenCode | 83/178 (47 %) | 5 152 tok | 2.1 % | 0.25 CNY |
| **deployed (`main`)** | 68/178 (38 %) | 3 418 tok | 1.3 % | 0.81 CNY |
| Codex | 68/178 (38 %) | 1 706 tok | 0.6 % | 6.29 CNY |

| Group | `summarized` | `deterministic` | deployed | Codex | OpenCode |
|---|---:|---:|---:|---:|---:|
| real sessions | 98/129 (76 %) | 99/129 (77 %) | 39/129 (30 %) | 40/129 (31 %) | 50/129 (39 %) |
| synthetic chains | **49/49 (100 %)** | 28/49 (57 %) | 29/49 (59 %) | 28/49 (57 %) | 33/49 (67 %) |

Readings:

* **The deployed algorithm loses three quarters of the real-session answers.** 30 % on real
  sessions, against 76–77 % for the pair that replaces it. The gap is not a matter of size:
  Codex retains 0.6 % of the history and scores the same 38 %, while `main`'s 1.3 % buys
  nothing measurable — both discard the assistant prose the questions actually point at.
* **The handoff summary is worth its cost on this exam, but only there.** Its synthetic
  total is perfect (49/49) where the model-free projection gets 57 %, while on real sessions
  the two are level (76 % vs 77 %). The summary pays for itself through the structured
  chains, and its 7.14 CNY is close to Codex's 6.29 CNY for a far better result; the
  model-free strategy reaches 71 % for nothing.
* **Compression is not the objective.** Ranking by projection size inverts the quality
  order exactly: Codex compresses ~10× harder than `summarized` and answers ~45 points
  worse. A ratio only helps if what remains still contains the answer.
* **Cost tracks the size of the input, not the size of the output.** `summarized` and Codex
  pay almost the same to compact (7.14 vs 6.29 CNY) although their projections differ ~7×,
  because both summarise the same full history. What the smaller projection buys is the
  *later*, per-turn cost.

**As deployed** (production trigger; the Rust arms compacted 6 of 24 boundaries, Codex and
OpenCode all 24). These quality numbers are *not* retention-policy measurements — 18 of the
24 boundaries sent the un-compacted history — and are shown only to record deployment
behaviour:

| Strategy | Recall | Compacted | Median projection when it compacted | Compaction cost |
|---|---:|---:|---:|---:|
| `summarized` | 149/178 (84 %) | 6/24 | 34 498 tok | 0.13 CNY |
| `main` | 144/178 (81 %) | 6/24 | 4 911 tok | 0.03 CNY |
| `deterministic` | 137/178 (77 %) | 6/24 | 32 418 tok | 0 |
| Codex | 85/178 (48 %) | 24/24 | 1 644 tok | 6.50 CNY |
| OpenCode | 81/178 (46 %) | 24/24 | 5 360 tok | 0.44 CNY |

That the deployment passes on compaction at all six real-session boundaries is the direct
consequence of the trigger sitting at 80 % of a 1 000 000-token window. Its scoring cost is
correspondingly higher (2.67 CNY against 0.72 CNY for the same 90 questions), because every
probe carries the full history — the projection size above is a recurring per-turn cost, not
a one-off.

### Cost with the provider cache

The compaction column above is the **cold** price: what the ledger charged when nothing
shared a prefix. Production does not pay that, because a summary request that reuses the
session's own system prompt, tool definitions and messages is served from the provider's
prefix cache. On this model a cache read costs **0.02 CNY per 1M tokens against 1.0 for
fresh input** — 50× cheaper — so the cache decides which strategy is affordable.

Whether a strategy *can* share that prefix is a property of the request it builds, and the
prefix is compared from token 0, so it is decided by the system prompt alone:

| Strategy | Shares the session prefix? | Why |
|---|---|---|
| `summarized` | **yes** | sends the session's own system prompt and tool definitions, and the live conversation as real messages. The deployed path measured **99.8 %** cache read in production (212 548 of 212 911 tokens) |
| Codex | yes | reuses its base instructions and appends its instruction last; in its own deployment those are the session's system prompt |
| `main` | **no** | substitutes its own `SUMMARY_SYSTEM_PROMPT` constant, so token 0 differs from every request the session sent |
| OpenCode | no | sends a dedicated compaction system prompt |

The table below prices every arm's recorded calls at the registry's rates with a **modelled
98 % hit** for the strategies that can share the prefix (the deployed measurement is
99.8 %; 98 % is the conservative stand-in), and at full price for the others. Per-turn cost
is the projection re-sent on every later turn, likewise cache-served, and is what a smaller
projection actually buys down.

Two different costs are in play and mixing them is the easiest mistake to make. **Compaction
cost** is what producing the projection costs, once. **Per-turn cost** is what re-sending
that projection costs on every later turn. A model-free strategy has the first at exactly
zero and still the second above zero — its projection is smaller than the raw history, not
absent:

* the median history at these boundaries is **249 142 tokens**, which costs 0.009866 CNY per
turn cache-served. Every arm's per-turn figure is against that baseline, and that difference
is the ongoing value of compacting at all.
* `deterministic` compacts with **no model call**, so its compaction cost is **0** — not
"small", not "not measured". Its 0.04 CNY is entirely 100 turns × the 0.000406 CNY it costs
to re-send its 10 233-token projection.

| Strategy | Recall | Projection | Compaction (cold) | Shares prefix | Compaction (cached) | Per turn | 100 turns | Breaks even after |
|---|---:|---:|---:|:--:|---:|---:|---:|---:|
| `summarized` | **83 %** | 12 503 tok | 7.98 | yes | **0.53** | 0.000496 | 0.0496 | 56 turns |
| `deterministic` | 71 % | 10 233 tok | **0** | — | **0** | 0.000406 | 0.0406 | immediately |
| Codex | 38 % | 1 821 tok | 7.64 | yes | 0.42 | 0.000073 | 0.0073 | 43 turns |
| OpenCode | 47 % | 5 372 tok | 0.99 | no | 0.99 | 0.000213 | 0.0213 | 103 turns |
| **deployed (`main`)** | 38 % | 4 027 tok | 0.83 | no | 0.83 | 0.000160 | 0.0160 | 85 turns |
| *(no compaction)* | — | 249 142 tok | — | — | — | 0.009866 | 0.9866 | — |

Readings:

* **The cache inverts the cost ranking.** Cold, `summarized` is the most expensive strategy in
the set (7.98 CNY). Cache-served, it is cheaper than the deployed algorithm (0.53 against
0.83) — while answering 45 points more. That is not a coincidence of this fixture: it is
what a 50× price difference on 98 % of the input does.
* **Compacting pays for itself, but only over a session of tens of turns.** Each arm's
per-turn figure is 20–135× below the no-compaction baseline, yet the one-off compaction is
large enough that the saving needs **43–103 turns** to repay it (56 for `summarized`, 85 for
the deployment, 43 for Codex). For a short session, compacting is a net *cost* — the value
being bought is recall, and the size of the context, not money. Do not read the per-turn
column as "cheaper from turn two".
* **Cost per point of recall**, over one compaction plus 100 turns, is 0.0070 CNY for
`summarized`, 0.0112 for Codex and 0.0221 for the deployed algorithm — the deployment is
**3× worse per unit of recall**, the opposite of what its cold-number advantage suggests.
* **The deployed algorithm's saving is real but small, and it is bought with a different
mechanism.** Its summary requests carry 11× fewer input tokens (695 775 against 7 762 538),
because it does not re-read the originals: after the first checkpoint its `covered_from` is
**the previous checkpoint's entry id**, so each summary folds "previous summary + new range"
forward. That is what keeps its requests small, and it is also why its recall is 38 % — the
originals are never consulted again and the summary is summarised recursively.
`summarized` re-covers the whole range from the journal every time, which is why its
requests are large; caching is what makes that affordable.
* **A private system prompt costs about 5× on compaction.** Priced at the same 98 % hit,
`main`'s own requests would come to 0.16 CNY instead of 0.83 — the difference is entirely
that its first token does not match anything the session sent. Choosing a bespoke summariser
prompt is what forfeits the cache, not the amount of text sent.
* **`deterministic` is the cheapest per point by an order of magnitude** (0.0006 against
0.0070) because it spends nothing at all, and it still reaches 71 %. The handoff summary
costs ~12× more per point of recall than not summarising (0.0070 against 0.00057) — it is a
quality purchase, not an efficiency one.

These are modelled figures for the cache, not measurements of it: the runs' own cache
counters are contaminated by arm and run ordering (see PRODUCTION_SHAPE_PROTOCOL.md), and
the 98 % is anchored to the one production measurement rather than to this harness.

## How the summary is generated

`summarized`'s projection comes from the production Rust path, so the *selection* it runs is
the runtime's own. The projection content is that function's output on the frozen records;
it is not byte-identical to a production checkpoint, because the driver rebuilds messages
from a reduced export. The mechanism is in [policy](compaction.md).

**The 128K tables below do not reproduce the production prefix.** They pass Codex's base
instructions through `--system-prompt-file` and an empty tool list, so the `summarized` and
Codex arms send the same prefix and differ only in what they retain. Two consequences follow,
neither of which may be read as a property of the production path:

* the high stage-0 hit rate those runs record (99 %+) is the **Codex arm priming the prefix
  that `summarized` then reuses**, not `summarized` earning it — the arms run in the same
  block; later stages show the ordinary ~10 % that consecutive compactions earn by sharing a
  head;
* substituting a prompt, dropping the tool definitions, or adding one line each measured
  **0 %** on a primed prefix, so those runs cannot bound the production cost either way.

The production-shaped run at the top of this document does pass the session's prompt and tool
definitions, so its summary requests have the production prefix — but a frozen session's own
prompt is rebuilt per turn from its working directory and is not in the journal, so it is a
real captured production prompt standing in for one, not the original. Its cache counters are
likewise not comparable, because the arms run in one block and prime each other. That is why
the cost table above is **modelled** rather than measured.

## Method (the 128K sections below)

* **Chains** — three synthetic (export, analysis, pipeline) and three real sessions,
  **frozen** to immutable JSON before measurement (`real-yt` 756, `real-visual` 685,
  `real-stream` 1582 records). Freezing matters: the real sessions are read live from the
  Agent database, and one was still being written to during earlier runs, so its record
  count grew under measurement. No session identifier appears in the repository; the frozen
  copies live in the git-ignored research directory.
* **Boundaries** — three per chain (≈40 %, 70 %, 100 % of the history) = **18 probes**.
* **Exam** — a value-retention probe: values drawn from assistant prose, user turns and
  tool results, plus 8 plausible decoys that appear nowhere. The model marks which appeared;
  a projection that dropped a value cannot distinguish it from a decoy. This is recognition,
  not free recall.
* **Window** — 128 K, the saturation point for `summarized` (see limits). The
  production-shaped run uses the model registry's 1 000 000 instead.
* **Costs** — the provider's own reported `credit_cost`, billed per call. These are cold
  figures; the production-shaped run prices the cache explicitly.

## Closed book: what each strategy keeps

| Chain | C3 | Codex | OpenCode |
|---|---:|---:|---:|
| export (synth) | **9/16** | 8/16 | 8/16 |
| analysis (synth) | **9/16** | 8/16 | 8/16 |
| pipeline (synth) | **10/17** | 9/17 | 9/17 |
| real-yt | **35/44** | 31/44 | 24/44 |
| real-visual | **27/36** | **27/36** | 25/36 |
| real-stream | **39/49** | 36/49 | 15/49 |
| **total** | **129/178 (72.5 %)** | 119/178 (66.9 %) | 89/178 (50.0 %) |

| Group | C3 | Codex | OpenCode |
|---|---:|---:|---:|
| **real sessions** | **101/129 (78.3 %)** | 94/129 (72.9 %) | 64/129 (49.6 %) |
| synthetic chains | **28/49 (57.1 %)** | 25/49 (51.0 %) | 25/49 (51.0 %) |

**Zero false positives for every strategy on every chain.** The differences are recall,
never invention.

| Strategy | median projection |
|---|---:|
| C3 | 8 495 tokens |
| Codex | 1 223 tokens |
| OpenCode | 3 249 tokens |

Readings:

* **Closed book, C3 leads on real sessions and on the synthetic chains.** There it is 28/49
  against 25/49 for both others — those fixtures put their answerable detail in tool
  records, which every rule keeps in some form. The strategies separate only where the
  answering detail lives in the agent's own prose, which is where real questions point.
* **Codex's compact projection is efficient per token but does not compensate.** It keeps
  ~7× fewer tokens than C3 and answers 5.4 points worse on real sessions; a small
  projection only helps if it still contains the answer.
* **OpenCode is weakest (−28.7 points vs C on real sessions).** Consistent with its
  design: summary plus a short tail, with no verbatim originals and no evidence index.
  Its collapse on `real-stream` (15/49) is the clearest case.

## Open book: adding each strategy's own lookup interface

Every strategy was re-probed with the lookup interface matching its design, over the same
projections:

| Strategy | Interface | What it reads |
|---|---|---|
| C3 | the archive CLI (`future session history search` / `get`) | the session archive |
| Codex | the window/item interface (`history.search_contents` / `read_item`) | the session archive, grouped into windows |
| OpenCode | the filesystem (`glob` / `grep` / `read`) | a materialised working tree |

| Strategy | Closed | Open | Δ | Mean lookups | Data read |
|---|---:|---:|---:|---:|---:|
| **C3** | 129/178 | **168/178 (94.4 %)** | **+39** | 2.67 | 0.20 MB |
| **Codex** | 119/178 | **170/178 (95.5 %)** | **+51** | 6.28 | 2.66 MB |
| **OpenCode** | 89/178 | **117/178 (65.7 %)** | **+28** | 4.33 | 0.54 MB |

Split by workload:

| Workload | C closed → open | Codex closed → open | OpenCode closed → open |
|---|---|---|---|
| real sessions | 101/129 → **119/129** | 94/129 → **126/129** | 64/129 → **85/129** |
| synthetic chains | 28/49 → **49/49** | 25/49 → **44/49** | 25/49 → **32/49** |

Retrieval cost, billed per call:

| Strategy | Retrieval calls | Total | Mean per call |
|---|---:|---:|---:|
| C3 | 48 | ¥0.7113 | **¥0.0148** |
| Codex | 114 | ¥3.3083 | **¥0.0290** |
| OpenCode | 78 | ¥1.2088 | **¥0.0155** |

**Retrieval changes the result completely, and it changes the ranking.** Closed book,
C3 leads by 10 fields; open book, the two are within 2 of each other (168 against 170),
because both can reach most of what the archive holds. Three findings:

1. **The gain is inversely proportional to what the projection retained.** C3 starts
   highest and gains least (+39); Codex starts lowest of the two and gains most (+51);
   OpenCode gains +28 and still ends far behind. In other words, **closed book measures
   what a strategy keeps, open book measures what it can find — and only the first
   separates these designs.**
2. **C3 reaches its result with a fraction of the effort.** It spends 2.67 lookups and
   reads 0.20 MB where Codex spends 6.28 and reads 2.66 MB — 13× less data for a
   comparable score. Its evidence index points at the records worth reading, so its
   lookups are targeted rather than exploratory. On the synthetic chains C3 is the only
   arm that reaches a perfect score (49/49).
3. **The archive bounds the ceiling, not the projection.** Every strategy that can read
   the archive converges toward it; the remaining differences are about how efficiently
   each gets there. The exception is OpenCode (65.7 %): its interface is the filesystem,
   and a file's current state does not contain values that later versions overwrote, so
   part of the archive is structurally unreachable through that interface, however many
   lookups it makes.

> **Correction.** An earlier revision reported that retrieval was worth only +1 to +10
> fields, and concluded it was "neutral". That measurement had three defects: the CLI
> fell back to an agent build that predates the history command, so every lookup returned
> `unknown command` and 0 bytes; the Codex arm was handed an empty window list and the
> OpenCode arm was not dispatched at all; and the synthetic chains passed an empty
> session id. With all three fixed the effect is large and consistent in direction for
> every strategy.

## Does C3's search need adjusting, and is Codex's search better?

Two questions the open-book results invite, both answerable from the stored runs.

### Codex's search is not better — it is used more often

| Strategy | Fields gained | Lookups | Data read | KB per field | Fields per lookup |
|---|---:|---:|---:|---:|---:|
| **C3** | 39 | 48 | 0.20 MB | **4.9** | **0.81** |
| Codex | 51 | 113 | 2.66 MB | 50.9 | 0.45 |

C recovers a field for every 4.9 KB it reads; Codex needs 50.9 KB, and twice as many
lookups per field. By that measure **C3's interface is roughly ten times more
byte-efficient**, which is what an evidence index pointing at the records worth reading
should do.

Codex's 2-point lead comes from a single behavioural difference, not a better tool. On
three probes C3 made **no tool call at all** — it answered straight from its projection —
and lost exactly the values its projection was missing:

| Probe | C closed | C open | C lookups | Codex open | Codex lookups |
|---|---:|---:|---:|---:|---:|
| real-visual s2 | 9/12 | 9/12 | **1 (no call)** | 11/12 | 3 |
| real-stream s1 | 12/15 | 12/15 | **1 (no call)** | 15/15 | 4 |
| real-stream s2 | 14/17 | 14/17 | **1 (no call)** | 17/17 | 5 |

Those three probes are the entire gap. **What is worth borrowing from Codex is therefore
not its interface but its willingness to look things up** — and that willingness comes
from having little choice: a 1.2 K-token projection cannot answer without searching,
where C3's 8.5 K-token projection often can.

Where C3 *does* search, it recovers the gap exactly: on 13 of 18 probes the gain equals
the number of values its projection was missing (`+3` of a gap of 3, and so on).

### Restricting search to tool records: smaller saving than expected

The projection contains **every assistant and user text block** — measured across all 18
projections, 100 % of text blocks are present verbatim. So assistant prose never needs to
be searched. The question is what ignoring it would save.

Search hits by record kind, sampled over the frozen sessions (108 hits, 9 queries):

| Kind | Hits | Snippet bytes | Share of bytes |
|---|---:|---:|---:|
| tool result | 64 | 26 180 | **61.9 %** |
| tool call (arguments) | 34 | 13 781 | **32.6 %** |
| assistant text | 10 | 2 350 | **5.6 %** |

Two things follow, and they point in opposite directions:

* **Assistant text is only 5.6 % of returned bytes**, so excluding it would save almost
  nothing. A special search mode for it is not worth building.
* **Excluding tool calls would be a mistake.** The projection does *not* contain them:
  every assistant text block is present, but only **16–19 %** of tool-call paths survive
  on the real sessions (27/169, 42/216, 29/250) and 40–100 % on the synthetic ones. Those
  calls carry the file paths and commands, and they are 32.6 % of returned bytes —
  dropping them to save 5.6 % would be a bad trade.

So the answer is narrower than the question suggests: **searching only tool records is
safe with respect to assistant prose but not with respect to tool-call arguments, and the
saving is 5.6 %.** The projection's division of labour is already nearly right — prose is
carried verbatim, tool evidence is indexed, and search is the fallback for what neither
covered.

## Cost per compaction at 128K

The driver issues requests **cold** (it does not first send the conversation as ordinary
turns, so nothing is warm):

| Strategy | Summaries | Mean cost | Mean input tokens |
|---|---:|---:|---:|
| C3 | 15 | ¥0.1173 | 73 539 |
| Codex | 18 | ¥0.0286 | 7 079 |
| OpenCode | 19 | ¥0.0256 | 7 352 |

These cold figures are a driver artefact, not what a session pays. C3 reads the **whole live
conversation** (67 K tokens on average, up to 128 K) while the others summarise a bounded
slice (≈7 K), so cold it is the most expensive of the three — and warm it is the cheapest,
because the prefix was already paid for by the turns that produced it. The production-shaped
run above prices that properly, and the isolated-agent measurement turns a 212 911-token read
into ¥0.003 against ¥0.0286 for a 7 K-token cold summary.

## What is inside a C3 projection

The ~11 K-token projection is not one block. Measured across all 18 boundaries, split
exactly by reconstructing each part from the committed checkpoint (no model calls):

| Part | Mean tokens | Share |
|---|---:|---:|
| protected originals (user + assistant, verbatim) | 5 155 | **45.8 %** |
| deterministic tool-evidence index | 2 415 | **21.5 %** |
| model handoff summary | 1 592 | **14.1 %** |
| retained recent tail | 2 095 | **18.6 %** |
| **total** | **11 259** | |

The balance differs by workload, which is worth knowing when reading the scores:

| Workload | Total | Originals | Evidence | Summary | Tail |
|---|---:|---:|---:|---:|---:|
| synthetic (n=9) | 9 067 | 63.1 % | 26.4 % | 10.5 % | 0 % |
| real sessions (n=9) | 13 451 | 34.1 % | 18.1 % | 16.6 % | **31.2 %** |

Two things follow. The evidence index holds a steady ~2.4 K tokens (it has a fixed
2 K budget and fills it), so it costs the same on a small history and a large one. And
real sessions carry a much larger recent tail, because their records are finer-grained:
1582 records for one session means proportionally more of it falls inside the retained
window.

## Ablation at 128K: the summary when the projection keeps nearly everything

This ablation removed the model summary (deterministic projection only, no provider) at the
same 128K shape as the tables around it. **Its result is a property of that shape, not of the
strategy**, and the production-shaped run disagrees with it — deliberately worth reading
together (see [below](#what-the-ablation-establishes)).

| Arm | Closed | Open | Mean lookups (open) | Summary cost |
|---|---:|---:|---:|---:|
| **C3** (runtime default) | 129/178 | **168/178** | 2.67 | ¥0.1173 per compaction cold |
| **C** (deterministic only) | **129/178** | 162/178 | 2.22 | **¥0** |

**Closed book the two are identical — every one of the 18 probes scores the same.** Open
book the summary is worth **+6**.

### Why the summary adds nothing at 128K

The exam asks about exact values, and at this window the deterministic projection already
keeps almost all of them, so there is nothing left for a lossy copy to add. Measuring
containment across all 18 projections (no model calls):

| | Values present |
|---|---:|
| deterministic projection (originals + evidence + tail) | **130/178 (73.0 %)** |
| model summary | 74/178 (41.6 %) |
| **either** | **130/178** |
| **values the summary adds that the rest lacks** | **0** |

Every value in the summary is also in the deterministic projection. The summary is a
subset, so it cannot raise a score that the rest of the projection already sets — it can
only lose information. That is why the two arms tie exactly.

This is the same effect the earlier compression sweep found, from the other direction:
verbatim retention beats summarisation precisely because a summary is a lossy copy. Here
the lossy copy is *added to* the verbatim material rather than replacing it, and a lossy
copy beside an exact one contributes nothing.

### Why it gains +6 open-book: it changes search behaviour

The 6-point gain is not an information gain. On all six probes where the arms differ, the
arm that made **more lookups** scored exactly **3 fields higher**:

| Probe | C3 | C (no summary) | Δ | C3 lookups | C lookups |
|---|---:|---:|---:|---:|---:|
| pipeline s3 | 6/6 | 3/6 | **+3** | **2** | 1 |
| real-stream s1 | 16/17 | 13/17 | **+3** | **4** | 1 |
| real-visual s1 | 15/15 | 12/15 | **+3** | **2** | 1 |
| real-yt s0 | 14/14 | 11/14 | **+3** | **2** | 1 |
| real-stream s0 | 12/15 | 15/15 | −3 | 1 | **2** |
| real-visual s0 | 9/12 | 12/12 | −3 | 1 | **3** |

The correlation is 6 out of 6, in both directions. The summary does not supply the missing
values — it supplies enough context that the model decides to go and look. This is the
same behaviour that separates C from Codex in the open-book comparison above, and it
means **the summary's measured value is a prompt effect, not a retention effect.**

### What this implies for the design

* **The summary is not paying for itself on this exam.** It costs a model call per
  compaction and contributes no value the projection does not already contain. Its only
  measured benefit is indirect.
* **The honest caveat is that this exam is a value-recall probe.** It asks "did this exact
  string appear", which is the task verbatim retention is best at and summarisation is
  worst at. A summary should help on questions the exam does not ask: what the objective
  was, why a decision was made, what remains open. Those are exactly the questions real
  follow-up turns ask (measured earlier at ~80 % "about the agent's own output"), and they
  are not scored here.
* **Do not remove the summary on this evidence.** The measurement bounds what it is worth
  on a recall exam; it does not establish that it is worthless in production. What it does
  establish is that the summary's *retention* contribution is zero and its *behavioural*
  contribution is real, which is a narrower and more useful claim.

### Does the summary earn its cost when the originals are dropped?

The exams above all ran where C3 retained every assistant block, which makes the summary
redundant by construction. This test creates the condition where a summary could matter:
compress hard enough that originals must be dropped, then see whether it preserves them.

**Compression pressure.** Sweeping the window and counting assistant text blocks whose
distinctive phrase survives anywhere in the projection:

| Window | Assistant blocks | Kept verbatim | Carried by summary | Lost |
|---:|---:|---:|---:|---:|
| 128 000 | 446 | **446** | 0 | **0** |
| 32 000 | 446 | 333 | **1** | 112 |
| 8 000 | 277 | 184 | **0** | 93 |
| 4 000 | 277 | 123 | **0** | 154 |

At the production-relevant window nothing is dropped at all. Under real pressure (32 K and
below) between 93 and 154 assistant blocks are dropped, and the summary carries **at most
one** of them verbatim.

**Generation test.** Containment tests exact strings, and a summary paraphrases, so the
test above understates it. This one asks the model to *generate*: from its projection,
list every exact identifier, path, version, size and count it can find. The question names
none of them, so a projection that lost a value cannot recover it by matching. Scored by
exact containment — no judge — across the three real sessions at two windows:

| Arm | Dropped values | Recovered | Rate |
|---|---:|---:|---:|
| C3 (runtime default) | 257 | **1** | **0.4 %** |
| C (deterministic only) | 252 | **1** | **0.4 %** |

**Identical, and both are zero in practice.** The two arms also drop almost the same number
of values (257 against 252), which is the subset result again seen from another direction:
adding the summary changes what is retained by about 2 %.

**One honest caveat on the control.** The same run measured how many *present* values the
model listed, to check the task was possible. That control is too noisy to support any
claim: at 32 K the summary arm listed 20 % against the other arm's 51.5 %, and at 16 K the
order reversed (72.1 % against 25.6 %). The model's listing behaviour varies far more
between runs than any summary effect, so the control is reported here only to show the task
was doable, and **no conclusion is drawn from it about whether the summary helps or harms
precise generation.**

### What the ablation establishes

The three measurements above agree **at this window**: value recall adds 0, compression
pressure preserves 1 of 93–154 dropped blocks, and generation of dropped values recovers
0.4 %, identical with and without the summary.

**The production-shaped run contradicts the first of those.** There, `deterministic`'s
projection contained 130 of 178 examined values and scored 127, while `summarized`'s
contained **151** and scored 147. The handoff summary text alone carried about 47 % of the
examined values, against 41.6 % here — but here every one of them was already in the
deterministic projection, and there they are not.

So the honest statement is conditional rather than absolute: **how much the handoff summary
adds depends on how much of the examined material the projection already retains.** At 128K
the deterministic projection alone carried 130/178 (73 %) and the summary was a subset of it,
so it added nothing; at the production shape the same 130 were carried and the summary added
values on top. The two measurements are endpoints of one variable, not a contradiction, and
neither run varies that variable deliberately — which is the experiment this document still
lacks.

What none of this establishes is that the summary is worthless — or essential — in
production at large. Every exam
used here asks for exact values, which is precisely what a summary is worst at and what
verbatim retention is best at. A summary should help with questions this exam never asks —
what the objective was, why a decision was made, what remains unresolved — and those are
exactly the questions real follow-up turns ask. The honest statement is narrow: **the
summary's retention value is unmeasurable on a value-exact exam, and its measured value is
a prompt effect.**

### Continuation exam: the questions real users ask

Every exam above asks for exact values, which is the one thing a summary cannot do. This one
tests what it is for — supporting continuation — and it takes all three of its inputs from
the real session, so nothing is synthesised:

| Element | Source |
|---|---|
| boundary | the records before a real user turn |
| question | that user turn, verbatim |
| reference answer | the assistant turns that followed it, verbatim |

Real follow-up turns are almost entirely about the agent's own prior output — *"why is prime
agent so fast?"*, *"what did the hand-tuning do?"*, *"why did the loop take so long?"* —
which is exactly the material a summary is supposed to carry and value-recall cannot reach.

Both arms compress the same boundary through the same code path, differing only in whether
the summary is generated, and answer with no tools. Scoring is a blind paired judgement with
the arm labels shuffled.

**The result moved with the sample size, and then stopped meaning anything.**

| Sample | C3 | C (no summary) | Paired |
|---|---:|---:|---|
| 18 questions | **1.61** | 1.39 | summary 9, no-summary 6, tie 3 |
| 27 questions (all eligible) | 1.19 | **1.37** | summary 10, no-summary 11, tie 6 |

On the first 18 questions the summary looked worth +0.22. Adding nine more turned it into
−0.18. Over the full sample the arms are indistinguishable: 48 % of decided pairs favour the
summary, which is a coin flip.

**The judge cannot resolve an effect this small, and that is measurable.** The verdicts were
regenerated on identical answer pairs (the answers are cached), which gives the instrument's
noise floor directly:

| | |
|---|---:|
| mean absolute score spread on identical inputs (0–3 scale) | **0.97** |
| repeated verdicts that were identical | **11/36 (31 %)** |
| repeats differing by the maximum 2 points | 10/36 |
| arm difference being measured | **0.18** |
| 95 % CI, with summary | [0.79, 1.58] |
| 95 % CI, without summary | [1.05, 1.69] |

**The instrument's self-disagreement is five times the effect it is being asked to detect,**
and the two confidence intervals overlap across almost their whole range. The judge also
flips on the paired verdict itself — one question was judged "B is better" and later "A is
better" on identical inputs.

### What this establishes

Three exams now fail to find the summary's retention value, for three different reasons, and
the distinction matters:

| Exam | Outcome | Why |
|---|---|---|
| Value recall (closed book) | summary adds **0** values | it is a strict subset of the projection |
| Compression pressure, generation | **0.4 %** recovery, identical arms | it does not preserve what the originals drop |
| Continuation, real questions | **no measurable difference** | the judge is too noisy to decide |

The first two are negative findings about the summary: it demonstrably adds no content and
preserves nothing under pressure. The third is a **limitation of the measurement**, not
evidence about the summary — it neither supports nor refutes a benefit in the one condition
where a benefit is plausible.

**What would settle it.** A deterministic rubric rather than a free-form judge: mechanically
derived items from the held-out continuation (which files, which identifiers, which stated
blockers the next turns actually referenced), scored by containment. That trades coverage of
"did it capture the gist" for a noise floor near zero, and on this evidence a scorer with no
variance is worth more than a richer question it cannot answer reliably.

**The rubric has since been built and run.** It scores two deterministic quantities against
the reference answer the session actually gave: *content recall* (fraction of the reference's
content terms — ASCII words plus CJK bigrams — that the candidate also contains, i.e.
ROUGE-style coverage) and *referent recall* (fraction of its concrete paths, identifiers,
versions, sizes and counts). Both have exactly zero variance: re-scoring identical inputs
reproduces the numbers bit for bit.

Answers were regenerated for this run with a prompt that forbids tool calls, because the
free-form run's prompt let the model emit `<tool_calls>` markup that nothing executed — those
answers were truncated into markup and would have scored as empty. All 38 questions now
produce a non-empty prose answer and none contains tool markup.

| Arm | Content recall | Content F1 | Referent recall | Chars |
|---|---:|---:|---:|---:|
| C3 | **0.199** | 0.146 | **0.242** | 2 208 |
| C | 0.182 | 0.142 | 0.216 | 2 270 |

| | |
|---|---:|
| mean paired difference (content recall) | **+0.0172** |
| sd of the paired differences | 0.0672 |
| **smallest difference detectable at 80 % power (n=38)** | **0.0305** |
| paired wins | summary 18, no-summary 20 |
| sign test | p = 0.871 |

**The measured difference is smaller than the design can resolve.** With a scorer of zero
variance the binding constraint is no longer the instrument but the effect size relative to
the sample: to detect 0.017 at 80 % power would take roughly 120 questions, not 38. So the
rubric removes the judge's noise and thereby exposes the real situation — **the summary's
benefit on continuation questions, if it exists, is small enough that this exam cannot see
it at any sample size it was run at.**

That is where this line of investigation stops honestly. Three exams fail to find the
summary's retention value and one of them was rebuilt specifically to remove the instrument
that was masking the answer; what remains is an effect size below the resolution of the
measurement, not a demonstration that the effect is absent. Confirming or refuting a benefit
of about two percentage points would require a much larger question set than the three real
sessions can supply.

One incidental finding, relevant to production rather than to the arms: at a 2 048-token
output cap the answers came back **empty** on several probes, because reasoning consumed the
entire allowance before any text was emitted. The same failure mode was observed earlier in
the handoff-summary path. It is a real hazard for any call whose output budget is set without
regard to reasoning overhead.

## Why C is only slightly ahead, despite keeping far more

This is a fair challenge to the result, and the exam's own composition answers it.

### Where the exam's values actually live

| Source | Items | Share | In C3 | In Codex | In OpenCode |
|---|---:|---:|---:|---:|---:|
| tool result | **120** | **67.4 %** | **72/120** | 62/120 | 43/120 |
| assistant text | 44 | 24.7 % | **44/44** | 43/44 | 32/44 |
| user text | 14 | 7.9 % | 14/14 | 14/14 | 13/14 |
| **total** | 178 | | **130** | 119 | 88 |

Three conclusions, and one of them corrects the premise:

1. **On assistant-sourced items C and Codex are effectively tied: 44/44 against 43/44.**
   C3's verbatim retention of assistant prose is perfect, but Codex's whole-history
   summary recovered almost every one of those 44 values anyway. So the exam cannot
   reward C3's advantage here — not because the advantage is absent, but because Codex
   does not lose those facts in the first place.
2. **C3's actual lead comes from tool results: 72 against 62.** That is the deterministic
   evidence index doing the work, and it is where the +10 over Codex originates.
3. **The exam is 67 % tool-sourced.** Only a quarter of its items can distinguish the
   two strategies on assistant content, and on those they tie.

### The intuition is right, but it needs a compression ratio

The premise — "if questions target assistant content, C should win by a lot" — is
correct in principle and was measured earlier under conditions this exam does not
create:

| Assistant content | Compression | Verbatim retention | Summary-only |
|---|---:|---:|---:|
| 160 exact decision codes | 15× | **10/10** | 0/10 |
| 400 exact decision codes | 39× | **10/10** | 1/10 |

Verbatim retention wins decisively **once the summary is forced to compress hard**. On
these real sessions the summary is not under that pressure: the model writes 1.6 K tokens
of summary for a 73 K-token input, and at that ratio it keeps the assistant facts it
needs. So the two strategies differ mainly on tool results, where C has an index and
Codex has nothing.

**What would separate them** is a session whose assistant turns carry many exact,
similar-looking identifiers — the case the fixture above models — or a tighter summary
budget. This exam contains neither, and its 67 % tool weighting is a property of the two
real workloads measured, not a universal one.

## Limits

* **Codex and OpenCode are single-point reimplementations.** Their retention rules and
  prompts were read from specific commits; a later upstream change invalidates the
  numbers. Both are configured as available to anyone using an API key or a third-party
  OpenAI-compatible provider. Codex's first-party `history`/`notes` retrieval tools
  require the hosted Codex backend, a paid ChatGPT plan and two undeveloped-stage feature
  flags, so a "Codex with its own search" arm is **not reproducible** here; the Codex
  column is its local fallback compaction.
* **The exam is recognition, not task continuation.** It does not measure whether an
  agent can resume the work.
* **Synthetic fixtures are tool-heavy.** They reward carrying tool evidence; the real
  sessions are where the strategies separate.
* **Window sensitivity.** C3's protected-original budget scales with the context window,
  and containment saturates at 128 K (measured: 65 % at 32 K, 79 % at 128 K, unchanged at
  256 K and 1 M). The production window comes from the model registry — the `128000`
  constant in `models/future.rs` is only the fallback for a model that declares none.
* **The open-book numbers use a bounded budget** (32 KB returned per probe, 60-round
  guard). A different budget would move the lookup counts.
* **Synthetic sessions had to be materialised for the open-book pass.** The archive CLI
  reads the Agent's session database, which contains only real sessions, so the synthetic
  fixtures were written into an isolated copy of that database as real sessions — registry
  row, entries and message blocks — before that arm could read them. The production
  database is never written; the copy is made with SQLite's backup API.
* **Retrieval cannot recover what no interface stores.** OpenCode's filesystem path sees
  only the newest version of each file, which is a property of that interface rather than
  a tuning issue.

## Reproducing

The harness and its instructions are in
[scripts/abc_experiment/README.md](../../../scripts/abc_experiment/README.md). Inputs, ledgers and
results live in `~/compact-exp` (override with `ABC_ROOT`), outside any repository because
they include real session data.

Reproducibility is therefore **partial, and split along this page's own structure**:

| Derived from | Reproducible from a checkout? |
|---|---|
| The three synthetic chains (`export`, `analysis`, `pipeline`) | **Yes** — seeded generators, byte-identical on regeneration |
| Everything involving `real-yt`, `real-visual`, `real-stream` | **No** — these are private conversations, read from the operator's Agent database and snapshotted by `freeze_sessions.py`; the ids and the snapshots are not published |

Concretely: the synthetic half of every table here can be rebuilt from the repository alone,
and the real-session half cannot. That is **129 of the 178 exam items (72 %)** — every
real-session row in the closed-book table, the entire open-book table (the synthetic chains
have no archive to read), the ablation, the continuation exam and the composition
breakdowns.

Scripts stop with an error when a required input is missing rather than skipping it, so a
partial run cannot be mistaken for a complete one. Third-party reproduction of the
real-session results requires substituting **your own** sessions through the steps in the
README; the absolute numbers will differ, and the comparisons should hold.
