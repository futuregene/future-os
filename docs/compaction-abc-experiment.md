# Compaction strategy comparison: C, Codex, OpenCode

Three retention strategies, measured on six chains, closed book and open book.

| Strategy | What survives compaction | Source |
|---|---|---|
| **C** | every protected user **and assistant** original + a 2 K deterministic tool-evidence index + recent tail + a **cache-friendly sticky handoff summary** written by the session model | this repo (runtime default) |
| **Codex** | all user messages (≤20 000 tokens) + a whole-history summary; no assistant text, no tool output | `openai/codex` @ `b13164d8`, `compact.rs::build_compacted_history` + `templates/compact/prompt.md` |
| **OpenCode** | a summary + a retained tail (`min(15 000, max(2 000, usable/4))`) | `anomalyco/opencode` @ `e03db9bc`, `session/compaction.ts` |

Codex and OpenCode are reimplementations of the selection rules read from those
commits, not forks. Their prompts were transcribed verbatim and each source file's git
blob SHA is recorded in [abc_external_provenance.json](../scripts/abc_external_provenance.json).

## How C's summary is generated, and why the request shape matters

C's projection is produced by the production Rust path
(`agent/examples/abc_c3_probe.rs` → `prepare_evidence_with_summary`), so its content is
exactly what the runtime commits. The summary is **sticky** — each one receives the
previous one — so facts accumulate across successive compactions instead of being
rewritten from scratch.

The request is deliberately shaped to be **served from the provider's prefix cache**:

* the live conversation is sent as **real messages**, not a flattened string;
* the instruction is **appended last**;
* the agent's own **system prompt and tool definitions** are reused.

Providers cache on the request prefix, so a request that reuses the turns the session
already sent is billed almost nothing, while a request that re-flattens or re-frames the
same content shares no prefix and is billed in full every time. Measured on an isolated
agent running this code path, a session grown to **212 911 tokens** compacted with
`cache_read = 212 548` — **99.8 %** of the request served from cache, `cache_write = 360`
(only the newly appended instruction). Cold, that request costs about ¥0.53; cached,
about ¥0.003.

## Method

* **Chains** — three synthetic (export, analysis, pipeline) and three real sessions,
  **frozen** to immutable JSON before measurement (`real-yt` 756, `real-visual` 685,
  `real-stream` 1582 records). Freezing matters: the real sessions are read live from the
  Agent database, and one of them was still being written to during earlier runs, so its
  record count grew under measurement. No session identifier appears in the repository;
  the frozen copies live in the git-ignored research directory.
* **Boundaries** — three per chain (≈40 %, 70 %, 100 % of the history) = **18 probes**.
* **Exam** — a value-retention probe: values drawn from assistant prose, user turns and
  tool results, plus 8 plausible decoys that appear nowhere. The model marks which
  appeared; a projection that dropped a value cannot distinguish it from a decoy. This is
  recognition, not free recall.
* **Window** — 128 K, the saturation point for C (see limits).
* **Costs** — the provider's own reported `credit_cost`, billed per call.

## Closed book: what each strategy keeps

| Chain | C | Codex | OpenCode |
|---|---:|---:|---:|
| export (synth) | **9/16** | 8/16 | 8/16 |
| analysis (synth) | **9/16** | 8/16 | 8/16 |
| pipeline (synth) | **10/17** | 9/17 | 9/17 |
| real-yt | **35/44** | 31/44 | 24/44 |
| real-visual | **27/36** | **27/36** | 25/36 |
| real-stream | **39/49** | 36/49 | 15/49 |
| **total** | **129/178 (72.5 %)** | 119/178 (66.9 %) | 89/178 (50.0 %) |

| Group | C | Codex | OpenCode |
|---|---:|---:|---:|
| **real sessions** | **101/129 (78.3 %)** | 94/129 (72.9 %) | 64/129 (49.6 %) |
| synthetic chains | **28/49 (57.1 %)** | 25/49 (51.0 %) | 25/49 (51.0 %) |

**Zero false positives for every strategy on every chain.** The differences are recall,
never invention.

| Strategy | median projection | 
|---|---:|
| C | 8 495 tokens |
| Codex | 1 223 tokens |
| OpenCode | 3 249 tokens |

Readings:

* **Closed book, C leads on real sessions and on the synthetic chains.** There it is 28/49
  against 25/49 for both others — those fixtures put their answerable detail in tool
  records, which every rule keeps in some form. The strategies separate only where the
  answering detail lives in the agent's own prose, which is where real questions point.
* **Codex's compact projection is efficient per token but does not compensate.** It keeps
  ~7× fewer tokens than C and answers 5.4 points worse on real sessions; a small
  projection only helps if it still contains the answer.
* **OpenCode is weakest (−28.7 points vs C on real sessions).** Consistent with its
  design: summary plus a short tail, with no verbatim originals and no evidence index.
  Its collapse on `real-stream` (15/49) is the clearest case.

## Open book: adding each strategy's own lookup interface

Every strategy was re-probed with the lookup interface matching its design, over the same
projections:

| Strategy | Interface | What it reads |
|---|---|---|
| C | the archive CLI (`future session history search` / `get`) | the session archive |
| Codex | the window/item interface (`history.search_contents` / `read_item`) | the session archive, grouped into windows |
| OpenCode | the filesystem (`glob` / `grep` / `read`) | a materialised working tree |

| Strategy | Closed | Open | Δ | Mean lookups | Data read |
|---|---:|---:|---:|---:|---:|
| **C** | 129/178 | **168/178 (94.4 %)** | **+39** | 2.67 | 0.20 MB |
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
| C | 48 | ¥0.7113 | **¥0.0148** |
| Codex | 114 | ¥3.3083 | **¥0.0290** |
| OpenCode | 78 | ¥1.2088 | **¥0.0155** |

**Retrieval changes the result completely, and it changes the ranking.** Closed book,
C leads by 10 fields; open book, the two are within 2 of each other (168 against 170),
because both can reach most of what the archive holds. Three findings:

1. **The gain is inversely proportional to what the projection retained.** C starts
   highest and gains least (+39); Codex starts lowest of the two and gains most (+51);
   OpenCode gains +28 and still ends far behind. In other words, **closed book measures
   what a strategy keeps, open book measures what it can find — and only the first
   separates these designs.**
2. **C reaches its result with a fraction of the effort.** It spends 2.67 lookups and
   reads 0.20 MB where Codex spends 6.28 and reads 2.66 MB — 13× less data for a
   comparable score. Its evidence index points at the records worth reading, so its
   lookups are targeted rather than exploratory. On the synthetic chains C is the only
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

## Does C's search need adjusting, and is Codex's search better?

Two questions the open-book results invite, both answerable from the stored runs.

### Codex's search is not better — it is used more often

| Strategy | Fields gained | Lookups | Data read | KB per field | Fields per lookup |
|---|---:|---:|---:|---:|---:|
| **C** | 39 | 48 | 0.20 MB | **4.9** | **0.81** |
| Codex | 51 | 113 | 2.66 MB | 50.9 | 0.45 |

C recovers a field for every 4.9 KB it reads; Codex needs 50.9 KB, and twice as many
lookups per field. By that measure **C's interface is roughly ten times more
byte-efficient**, which is what an evidence index pointing at the records worth reading
should do.

Codex's 2-point lead comes from a single behavioural difference, not a better tool. On
three probes C made **no tool call at all** — it answered straight from its projection —
and lost exactly the values its projection was missing:

| Probe | C closed | C open | C lookups | Codex open | Codex lookups |
|---|---:|---:|---:|---:|---:|
| real-visual s2 | 9/12 | 9/12 | **1 (no call)** | 11/12 | 3 |
| real-stream s1 | 12/15 | 12/15 | **1 (no call)** | 15/15 | 4 |
| real-stream s2 | 14/17 | 14/17 | **1 (no call)** | 17/17 | 5 |

Those three probes are the entire gap. **What is worth borrowing from Codex is therefore
not its interface but its willingness to look things up** — and that willingness comes
from having little choice: a 1.2 K-token projection cannot answer without searching,
where C's 8.5 K-token projection often can.

Where C *does* search, it recovers the gap exactly: on 13 of 18 probes the gain equals
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

## Cost per compaction

Measured on the driver, which issues requests **cold** (it does not first send the
conversation as ordinary turns, so nothing is warm):

| Strategy | Summaries | Mean cost | Mean input tokens |
|---|---:|---:|---:|
| C | 15 | ¥0.1173 | 73 539 |
| Codex | 18 | ¥0.0286 | 7 079 |
| OpenCode | 19 | ¥0.0256 | 7 352 |

The cold figures understate C and overstate nothing: C reads the **whole live
conversation** (67 K tokens on average, up to 128 K) while Codex and OpenCode summarise a
bounded slice (≈7 K). Cold, reading the whole conversation is expensive; **warm, it is
the cheapest of the three**, because the prefix was already paid for by the turns that
produced it — the 99.8 % cache hit above turns a 212 911-token read into ¥0.003, against
¥0.0286 for a 7 K-token *cold* summary.

A real session's summary is always issued warm, since the conversation it summarises has
just been sent. **The production cost of C's compaction is therefore the cached figure,
not the ¥0.1173 in the table above**, which is a driver artefact.

## What is inside a C projection

The ~11 K-token projection is not one block. Measured across all 18 boundaries, split
exactly by reconstructing each part from the committed checkpoint (no model calls):

| Part | Mean tokens | Share |
|---|---:|---:|
| protected originals (user + assistant, verbatim) | 5 155 | **45.8 %** |
| deterministic tool-evidence index | 2 415 | **21.5 %** |
| sticky model summary | 1 592 | **14.1 %** |
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

## Why C is only slightly ahead, despite keeping far more

This is a fair challenge to the result, and the exam's own composition answers it.

### Where the exam's values actually live

| Source | Items | Share | In C | In Codex | In OpenCode |
|---|---:|---:|---:|---:|---:|
| tool result | **120** | **67.4 %** | **72/120** | 62/120 | 43/120 |
| assistant text | 44 | 24.7 % | **44/44** | 43/44 | 32/44 |
| user text | 14 | 7.9 % | 14/14 | 14/14 | 13/14 |
| **total** | 178 | | **130** | 119 | 88 |

Three conclusions, and one of them corrects the premise:

1. **On assistant-sourced items C and Codex are effectively tied: 44/44 against 43/44.**
   C's verbatim retention of assistant prose is perfect, but Codex's whole-history
   summary recovered almost every one of those 44 values anyway. So the exam cannot
   reward C's advantage here — not because the advantage is absent, but because Codex
   does not lose those facts in the first place.
2. **C's actual lead comes from tool results: 72 against 62.** That is the deterministic
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
* **Window sensitivity.** C's protected-original budget scales with the context window,
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

See [scripts/abc_experiment/README.md](../scripts/abc_experiment/README.md). Fixtures,
frozen sessions, ledgers and scoring stay in the local, git-ignored
`.future/research/abc-summary-a47313/`.
