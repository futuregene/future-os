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
| export (synth) | **8/16** | **8/16** | **8/16** |
| analysis (synth) | **8/16** | **8/16** | **8/16** |
| pipeline (synth) | **9/17** | **9/17** | **9/17** |
| real-yt | **35/44** | 31/44 | 24/44 |
| real-visual | **27/36** | **27/36** | 25/36 |
| real-stream | **39/49** | 36/49 | 15/49 |
| **total** | **126/178 (70.8 %)** | 119/178 (66.9 %) | 89/178 (50.0 %) |

| Group | C | Codex | OpenCode |
|---|---:|---:|---:|
| **real sessions** | **101/129 (78.3 %)** | 94/129 (72.9 %) | 64/129 (49.6 %) |
| synthetic chains | 25/49 (51.0 %) | 25/49 (51.0 %) | 25/49 (51.0 %) |

**Zero false positives for every strategy on every chain.** The differences are recall,
never invention.

| Strategy | median projection | 
|---|---:|
| C | 8 495 tokens |
| Codex | 1 223 tokens |
| OpenCode | 3 249 tokens |

Readings:

* **C leads on real sessions and ties on synthetic ones.** On the synthetic chains all
  three are identical (25/49) — those fixtures put their answerable detail in tool
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

| Strategy | Interface |
|---|---|
| C | the archive CLI (`future session history search` / `get`) |
| Codex | the window/item interface (`history.search_contents` / `read_item`) |
| OpenCode | the filesystem (`glob` / `grep` / `read` over a materialised tree) |

| Strategy | Closed | Open | Δ | Mean lookups per probe |
|---|---:|---:|---:|---:|
| **C** | 126/178 | **126/178** | **0** | 2.72 |
| **Codex** | 119/178 | **120/178** | **+1** | 3.28 |
| **OpenCode** | 89/178 | 88/178 | **−1** | 1.78 |

Retrieval cost, billed per call:

| Strategy | Retrieval calls | Total | Mean per call |
|---|---:|---:|---:|
| C | 58 | ¥0.9232 | **¥0.0159** |
| Codex | 59 | ¥0.3079 | **¥0.0052** |
| OpenCode | 32 | ¥0.3493 | **¥0.0109** |

**Retrieval does not move the scores here, in either direction.** All three strategies
land within one field of their closed-book result. Two reasons, both worth stating:

* **The strategies that retain a lot have little left to look up.** C already carries the
  protected originals, the evidence index and the summary, so its lookups mostly confirm
  what it holds. Note its cost per lookup is the highest (¥0.0159) because each request
  carries the largest projection.
* **OpenCode's recovery path cannot see superseded state.** Its interface is the
  filesystem, and only the newest version of each file is on disk — so a value an earlier
  version overwrote is unreachable no matter how many lookups it makes. It spends the
  fewest lookups (1.78) and gains nothing.

This corrects an earlier version of this page, which reported that retrieval *helped* C
by 10 points and *cost* Codex 20. Those numbers came from a Python approximation of C's
evidence selection and from a 5-call cap that I had invented; with the real code path and
no cap, the effect is neutral. A lookup interface is not free accuracy — it is a way to
recover specifics that neither the summary nor the evidence index preserved, and none of
these three projections lost enough for it to matter on this exam.

## Cost per compaction

Measured on the driver, which issues requests **cold** (it does not first send the
conversation as ordinary turns, so nothing is warm):

| Strategy | Summaries | Mean cost | Mean input tokens |
|---|---:|---:|---:|
| C | 15 | ¥0.1014 | 67 491 |
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
not the ¥0.1014 in the table above**, which is a driver artefact.

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
  guard). A different budget would move the lookup counts, though not the conclusion that
  retrieval is neutral on this exam.
* **Retrieval cannot recover what no interface stores.** OpenCode's filesystem path sees
  only the newest version of each file, which is a property of that interface rather than
  a tuning issue.

## Reproducing

See [scripts/abc_experiment/README.md](../scripts/abc_experiment/README.md). Fixtures,
frozen sessions, ledgers and scoring stay in the local, git-ignored
`.future/research/abc-summary-a47313/`.
