# Compaction strategies: closed-book comparison

Five retention strategies, measured on six frozen chains, closed book only.

**Naming.** Arms are named after the code. `summarized` is the runtime default
(`summarized-evidence-v1`), `deterministic` is the same projection with no summary model
called (`deterministic-evidence-v1`), and `main` is the algorithm the released branch
deploys.

| Arm | What it retains | Source |
|---|---|---|
| `summarized` | every protected user **and assistant** original + a 2K deterministic tool-evidence index + a recent tail + a **cache-friendly handoff summary** written by the session model | this repo (runtime default) |
| `deterministic` | the same projection, no summary model called | this repo |
| `main` | recursive model summary + protected originals + a recent tail | this repo, `origin/main` @ **`7e268efe`** |
| `codex` | all user messages (≤20 000 tokens) + a whole-history summary; no assistant text, no tool output | `openai/codex` @ `b13164d8`, `compact.rs::build_compacted_history` + `templates/compact/prompt.md` |
| `opencode` | a summary + a retained tail (`min(15 000, max(2 000, usable/4))`) | `anomalyco/opencode` @ `e03db9bc`, `session/compaction.ts` |

`main` is not a reimplementation: [`abc_main_probe.rs`](../../../scripts/abc_experiment/abc_main_probe.rs)
is built inside a detached worktree of `origin/main` at the commit above and calls that
checkout's own entry point and budget rules, so the column is the shipped algorithm rather
than a port of it. Re-run `git log -1 origin/main` before quoting these numbers; a later
commit invalidates them.

Codex and OpenCode are reimplementations of selection rules read from those commits, not
forks. Their prompts were transcribed verbatim and each source file's git blob SHA is
recorded in [abc_external_provenance.json](../../../scripts/abc_external_provenance.json).

## Call shape

Every Rust arm is driven through the shape the runtime uses, not a simulated one:

| | value |
|---|---|
| window | the registry's declared window, 1 000 000 |
| output reservation | `effective_max_tokens`, 384 000 |
| request budget | `set_request_budget` with a captured session prompt and the real tool definitions |
| system prompt | a real production prompt, captured by [`capture_shape.py`](../../../scripts/abc_experiment/capture_shape.py) |
| trigger / phase | Automatic + PreTurn, production's pre-turn compaction |
| cadence | accumulate to the economic trigger, 613 952 tokens |

That trigger is 80% of the window clamped by `window − output reserve − margin`. It sits
above every real-session boundary in the fixture set, and above two of the three synthetic
ones, so with the production trigger the Rust arms frequently do not compact at all — see
[As deployed](#as-deployed-production-trigger).

## Compaction against compaction

The arms do not share a trigger. Codex and OpenCode compact at every scored boundary
unconditionally, so comparing them against Rust arms that often skipped compaction would
measure the trigger rather than the retention policy. The tables below therefore force every
arm to compact at every boundary: the Rust arms use Manual, which bypasses the economic
trigger by design. All 24 boundaries qualify and all 178 questions are scored under the same
condition.

| Strategy | Recall | Median projection | Compression | Compaction (cold) | Compaction (cached) | Per turn | Break-even |
|---|---:|---:|---:|---:|---:|---:|---:|
| `summarized` | **147/178 (83%)** | 12 113 tok | 5.7% | 7.98 | **0.53** | 0.000480 | 61 turns |
| `deterministic` | 127/178 (71%) | 9 945 tok | 4.7% | **0** | **0** | 0.000394 | immediate |
| `opencode` | 83/178 (47%) | 5 152 tok | 2.1% | 0.99 | 0.99 | 0.000204 | 110 turns |
| `main` | 68/178 (38%) | 3 418 tok | 1.3% | 0.83 | 0.83 | 0.000135 | 91 turns |
| `codex` | 68/178 (38%) | 1 706 tok | 0.6% | 7.64 | 0.42 | 0.000068 | 46 turns |
| *(no compaction)* | — | 232 777 tok | — | — | — | 0.009219 | — |

Cost columns are CNY per compaction, and `Per turn` is the projection re-sent on every later
turn; both are explained under [Cost](#cost-with-the-provider-cache). `Break-even` is how many
turns the recurring saving takes to repay the one-off compaction.

| Group | `summarized` | `deterministic` | `opencode` | `main` | `codex` |
|---|---:|---:|---:|---:|---:|
| real sessions | 98/129 (76%) | **99/129 (77%)** | 50/129 (39%) | 39/129 (30%) | 40/129 (31%) |
| synthetic chains | **49/49 (100%)** | 28/49 (57%) | 33/49 (67%) | 29/49 (59%) | 28/49 (57%) |

| Chain | `summarized` | `deterministic` | `opencode` | `main` | `codex` |
|---|---:|---:|---:|---:|---:|
| export | 16/16 | 9/16 | 12/16 | 10/16 | 12/16 |
| analysis | 16/16 | 9/16 | 10/16 | 8/16 | 13/16 |
| pipeline | 17/17 | 10/17 | 11/17 | 11/17 | 3/17 |
| real-yt | 33/44 | 35/44 | 16/44 | 20/44 | 10/44 |
| real-visual | 27/36 | 27/36 | 24/36 | 14/36 | 21/36 |
| real-stream | 38/49 | 37/49 | 10/49 | 5/49 | 9/49 |

**No strategy produced a false positive on any chain**: every difference above is recall,
not invention.

Readings:

* **The deployed algorithm keeps about one third of what the runtime default keeps.** 38%
  against 83% overall; on real sessions 30% against 76%. It is not a question of size: Codex
  retains 0.6% of the history and scores the same 38%, while `main`'s 1.3% buys nothing
  measurable. Both discard the assistant prose the questions actually point at.
* **Compression is not the objective.** Ranking by projection size inverts the quality order
  exactly: Codex compresses ~10× harder than `summarized` and answers ~45 points worse.
* **The two current strategies separate by workload, not by overall score.** On the synthetic
  chains the handoff summary is decisive — 49/49 against 57% — while on real sessions the two
  are level (76% vs 77%). The synthetic chains are structured and value-dense; real sessions
  spread their detail through prose, where verbatim protection alone already carries it.

### What the handoff summary contributes

Measured without model calls, by counting how many of each question's present values appear in
each arm's projection text:

| | Values present |
|---|---:|
| `deterministic` projection | 130/178 |
| `summarized` projection | **151/178** |

The summary therefore adds values the deterministic projection does not carry, and the scores
follow: 127 against 147. This is worth stating carefully, because it is not a property of the
summary alone but of how much the projection must drop: a projection that already retains
everything verbatim gains nothing from a lossy copy of it, and one that must drop most of a
249K-token history gains whatever the summary can carry forward.

## As deployed (production trigger)

The same six chains under the production trigger instead of a forced one. The Rust arms
compacted at **6 of 24 boundaries**; Codex and OpenCode, which have no economic trigger of
their own here, compacted at all 24.

| Strategy | Recall | Compacted | Median projection |
|---|---:|---:|---:|
| `summarized` | 149/178 (84%) | 6/24 | 34 498 tok |
| `main` | 144/178 (81%) | 6/24 | 4 911 tok |
| `deterministic` | 137/178 (77%) | 6/24 | 32 419 tok |
| `codex` | 85/178 (48%) | 24/24 | 1 644 tok |
| `opencode` | 81/178 (46%) | 24/24 | 5 360 tok |

**These quality numbers are not a retention-policy measurement.** At 18 of the 24 boundaries
the Rust arms sent the un-compacted history, so the table partly measures "did not compact"
rather than "compacts better". It is here to record deployment behaviour:

* the Rust arms skip compaction entirely at all six real-session boundaries, because the
  trigger sits at 80% of a 1 000 000-token window and those histories do not reach it;
* the deployment therefore pays the full history on every turn. Ordering the 90 exam
  questions cost 2.67 CNY against 0.72 CNY for the same questions under the forced run.

## Cost with the provider cache

Two costs are in play and mixing them is the easiest mistake to make. **Compaction cost** is
what building the projection costs, once. **Per-turn cost** is what carrying it costs on every
later turn. The table above gives both, and `deterministic` shows why they are separate: it
compacts with no model call, so its compaction cost is exactly 0, while its per-turn cost is
not — its projection is smaller than the raw history, not absent.

Rates come from the model registry, per 1M tokens: input **1.0**, output **4.0**, cache read
**0.02**, cache write 0. A cache read is therefore 50× cheaper than fresh input, which decides
which strategy is affordable.

Whether a request *can* use the cache is decided at token 0, by its system prompt, because a
prefix is compared from the start:

| Strategy | Shares the session prefix? | Why |
|---|---|---|
| `summarized` | **yes** | sends the session's own system prompt and tool definitions, and the live conversation as real messages. The production path measured **99.8%** cache read (212 548 of 212 911 tokens) |
| Codex | yes | reuses its base instructions and appends its instruction last |
| `main` | **no** | substitutes its own `SUMMARY_SYSTEM_PROMPT` constant, so token 0 matches nothing the session sent |
| OpenCode | no | sends a dedicated compaction system prompt |

The tables price each arm at the registry's rates with a **modelled 98% hit** for the arms that
can share the prefix (the deployed measurement is 99.8%; 98% is the conservative stand-in),
and at full price for the others.

Readings:

* **The cache inverts the cost ranking.** Cold, `summarized` is the most expensive strategy
  here, 7.98 CNY against 0.83 for the deployment. Cache-served it is cheaper — 0.53 against
  0.83 — while answering 45 points more.
* **Cost per point of recall**, over one compaction plus 100 turns, is 0.0070 CNY for
  `summarized`, 0.0112 for Codex and 0.0221 for the deployment: the deployment is about **3×
  worse per unit of recall**, the opposite of what its cold number suggests. `deterministic`
  is cheapest by an order of magnitude at 0.00055, because it spends nothing at all.
* **Compacting repays itself only over tens of turns.** Each arm's per-turn figure is 19–136×
  below the no-compaction baseline, but the one-off cost still needs 46–110 turns to be
  recovered (61 for `summarized`, 91 for the deployment). For a short session, compacting is a
  net cost; what it buys is recall and context headroom, not money.
* **The deployment's saving is real but small, and comes from a different mechanism.** Its
  summary requests carry 11× fewer input tokens (695 775 against 7 762 538), because it does
  not re-read the originals: after its first checkpoint its `covered_from` is *the previous
  checkpoint's entry id*, so each summary folds "previous summary + new range" forward. That
  keeps the requests small, and it is also why its recall is 38% — the originals are never
  consulted again and the summary is summarised recursively.
* **A private system prompt costs about 5× on compaction.** At the same 98% hit, `main`'s own
  requests would come to 0.16 CNY instead of 0.83. What forfeits the cache is choosing a
  bespoke summariser prompt, not the amount of text sent.

These cache figures are **modelled, not measured**: the runs' own cache counters are
contaminated by arm and run ordering — the arms prime each other, and a later run reuses
prefixes an earlier one left in the provider's cache — so the 98% is anchored to the one
production measurement rather than to this harness.

## Method

* **Chains** — three synthetic (`export`, `analysis`, `pipeline`) and three real sessions,
  frozen to immutable JSON before measurement (`real-yt` 756, `real-visual` 685,
  `real-stream` 1582 records). Freezing matters: the real sessions are read live from the
  Agent database, and one was still being written to during an earlier run.
* **Boundaries** — three scored points per chain (≈40%, 70%, 100%) = **18 probes**, plus the
  compaction boundaries the cadence produces (24 in the forced run).
* **Exam** — recognition of exact values: values drawn from assistant prose, user turns and
  tool results, plus 8 plausible decoys that appear nowhere. A projection that dropped a value
  cannot distinguish it from a decoy.
* **Model** — `future/deepseek-flash` for every arm, at matched generation settings.
* **Scoring** — one draw per arm and boundary; there is no significance claim and no
  replicate.
* **Costs** — the provider's reported usage for what was charged; the cache figures are
  modelled from the registry's rates as described above.

## Limits

* **The exam is recognition, not task continuation.** It does not measure whether an agent can
  resume the work, and it asks for exact values — which is what verbatim retention is best at
  and a summary is worst at.
* **Codex and OpenCode are single-point reimplementations.** Their rules and prompts were read
  from specific commits; a later upstream change invalidates the numbers. Codex's first-party
  `history`/`notes` retrieval tools require its hosted backend and feature flags, so a "Codex
  with its own search" arm is not reproducible here; the column is its local fallback.
* **Synthetic fixtures are tool-heavy**, which rewards carrying tool evidence. The real
  sessions are where the strategies separate.
* **One draw per cell.** Differences of a few points are not resolvable at this sample size.
* **The summary's measured benefit is conditional.** It adds values when the projection must
  drop history and adds none when the projection already retains it verbatim; this run varies
  neither factor deliberately.

## Reproducing

The harness and its instructions are in
[scripts/abc_experiment/README.md](../../../scripts/abc_experiment/README.md), and the protocol
this run follows is
[PRODUCTION_SHAPE_PROTOCOL.md](../../../scripts/abc_experiment/PRODUCTION_SHAPE_PROTOCOL.md).

Inputs, ledgers and results live outside any repository, because they include real session
data: the synthetic chains can be regenerated byte-identically from seeded generators, and the
real-session chains cannot be published at all. Scripts stop with an error when a required
input is missing rather than skipping it, so a partial run cannot be mistaken for a complete
one. Third-party reproduction of the real-session half requires substituting your own
sessions; the absolute numbers will differ and the comparisons should hold.
