# Closed-book protocol

The design behind `run_closed_book.py`. For the commands, see
[README.md](README.md#closed-book); for the results, see
[compaction-closed-book-experiment.md](../../docs/internals/compaction/compaction-closed-book-experiment.md).

## Why the call shape matters

An earlier generation of closed-book runs used a **simulated 128K window**, **no output
reservation** and **no request budget**. That compares retention rules at a matched size, but
it is not what production computes and it never exercises the deployment's own trigger.

| | earlier runs | this run |
|---|---|---|
| window | simulated 128K | the registry's declared window (1 000 000) |
| output reservation | none | `effective_max_tokens` (384 000) |
| request budget | not applied | `set_request_budget` with the session's system prompt and the real tool definitions |
| system prompt | a shared base prompt | a captured production prompt (`capture_shape.py`) |
| trigger / phase | not modelled | Automatic + PreTurn, production's pre-turn compaction |
| cadence | fixed 64K chunks | accumulates to the economic trigger (613 952) |

The request budget is the subtle one: the runtime calls `set_request_budget` at its call site,
and without it `fixed_input_tokens` and `output_reserve_tokens` stay zero, so the effective
limit is higher and compaction fires later than production does. The driver applies the same
budget, from the same function, with the session's prompt and the real tool definitions.

## Arms

Five. Three are this repo's — `summarized`, `deterministic`, and `main` — and two are pinned
external policies, Codex and OpenCode, transcribed in `strategies.py`.

`main` is not a reimplementation. `released_probe.rs` is copied into a detached worktree of the
released branch and built there, so it calls that checkout's own entry point and budget rules.
"Is the new strategy better than what is deployed" was previously unanswerable from this
harness, and answering it silently is worse than not answering it: `--main-commit` is required
and recorded in the manifest, because a binary path does not identify the code it was built
from and an unlabelled `main` column rots as the released branch moves.

## Fairness: compaction must be compared against compaction

The arms do not share a trigger. Codex and OpenCode compact at every scored boundary
unconditionally; the Rust arms obey an economic trigger that, on a 1M window with a 384K output
reservation, sits at 613 952 tokens — above every real-session boundary and above two of the
three synthetic ones. A table that mixes them compares "everything retained" against
"compacted" and reports the shape of the trigger, not the quality of the retention policy.

So the run has two modes:

* **`--force-compaction` — the fair comparison.** The Rust arms use Manual, which bypasses the
  economic trigger by design, so every arm compacts at every boundary and all 24 qualify. Only
  this mode answers "given that each policy compacted, which retains more of what the questions
  ask for, at what size and cost".
* **default — as deployed.** Production's trigger, so the deployment's real behaviour is
  measured, including that it often does not compact at all. Its quality column is not a
  retention-policy measurement and must not be quoted as one.

## Metrics

Every arm is reported on the same axes, because no single one is the answer:

* **quality** — hits / of present, plus false positives and invalid answers;
* **tokens after** — the projection the next request carries, in one estimator for every arm;
* **compression ratio** — that projection over the history it replaced, shared by all arms at a
  boundary;
* **compaction cost** — what *building* the projection costs, once, kept separate from scoring.
  A strategy that calls a model to summarise and one that does not cannot be compared on one
  blended number;
* **per-turn cost** — what *carrying* it costs on every later turn, with the provider's cache
  modelled, against the no-compaction baseline, plus the turns needed to repay the one-off
  compaction.

Compaction cost and per-turn cost are different quantities and the report keeps them apart: a
model-free strategy has the first at exactly 0 and the second above 0, because its projection
is smaller than the raw history rather than absent.

### Cache-aware cost

The ledger's charge is the **cold** number — these runs do not reproduce the production prefix,
so nothing is cache-served and the figure is an upper bound. Production serves a cache-friendly
summary from the provider's prefix cache, where a cache read costs 0.02 CNY per 1M tokens
against 1.0 for fresh input. `report.json` therefore prices every arm at the
`MODELLED_CACHE_HIT` rate (0.98) as well, and at full price for arms that cannot share the
prefix.

Eligibility is a property of the request each strategy builds, decided at token 0, and is
recorded in `CACHE_ELIGIBLE` with its reason beside it:

* `summarized` — **eligible**: it passes the session's own system prompt and tool definitions
  and the live conversation as real messages. The production path measured 99.8% cache read,
  which is what the 0.98 stand-in is anchored to.
* Codex — **eligible**: it reuses its base instructions and appends its instruction last.
* `main` — **not eligible**: it substitutes its own `SUMMARY_SYSTEM_PROMPT` constant.
* OpenCode — **not eligible**: it sends a dedicated compaction system prompt.

**The runs' own cache counters are not evidence here.** They are contaminated: the arms run in
one block and prime each other, and a later run can reuse a prefix an earlier one left in the
provider's cache — one recorded batch showed a 100% hit on a prefix an earlier aborted run had
sent. The model exists because this harness cannot measure it.

## Inputs

Six chains, the same questions and seeds as the other experiment. Scored boundaries are moved
to a pair-complete, message-complete endpoint by `safe_cuts`: `harness.schedule` enforces that
for its chunk boundaries but appends the forced cuts unchecked, which put two real-session cuts
inside a call/result pair.

Two synthetic-chain generators and the real-session freeze are described in
[README.md](README.md#laying-out-the-inputs).

## What this protocol does not cover

Open-book retrieval is a separate experiment with its own protocol
([OPEN_BOOK_PROTOCOL.md](OPEN_BOOK_PROTOCOL.md)), because it changes the request in a way this
comparison deliberately holds fixed: the tool definitions are present in both, but only the
open-book run lets the model call them.
