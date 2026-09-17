# Production-shaped closed-book protocol

The run this documents drives every Rust arm through the call shape the runtime uses, and adds
the deployed algorithm as an arm. It does not overwrite the earlier runs' artifacts and does
not mix their scores with these; it has its own ledger root.

## Why the call shape matters

The earlier closed-book runs used a **simulated 128K window**, **no output reservation** and
**no request budget**. That is a reasonable way to compare retention rules at a matched size,
but it is not what production computes, and it never exercises the deployment's own trigger.

This run uses the runtime's limits and call shape:

| | earlier runs | this run |
|---|---|---|
| window | simulated 128K | the registry's declared window (1 000 000) |
| output reservation | none | `effective_max_tokens` (384 000) |
| request budget | not applied | `set_request_budget` with the session's system prompt and the real tool definitions |
| system prompt | Codex's base instructions, shared with the Codex arm | a captured production prompt (`capture_shape.py`) |
| trigger / phase | not modelled | Automatic + PreTurn (production's pre-turn compaction) |
| cadence | fixed 64K chunks | accumulates to the economic trigger |

The request budget is the subtle one: the runtime calls `set_request_budget` at the call site,
and without it `fixed_input_tokens` and `output_reserve_tokens` stay zero, so the effective
limit is higher and compaction fires later than production does. The driver reproduces it along
with both prompts the runtime derives — the budget prompt always reserves the recall guidance,
the outgoing prompt carries it only once a checkpoint exists.

## Arms

Five. Three are this repo's (`summarized`, `deterministic`, and `main`), two are the pinned
external policies (Codex, OpenCode).

`main` is not a reimplementation: [`abc_main_probe.rs`](abc_main_probe.rs) is built inside a
checkout of the released branch and calls that checkout's own entry point and budget rules.
"Is the new strategy better than what is deployed" was previously unanswerable from this
harness.

## Fairness: compaction must be compared against compaction

The arms do not share a trigger. Codex and OpenCode compact at every scored boundary
unconditionally; the Rust arms obey an economic trigger that, on a 1M window with a 384K
output reservation, sits at 613 952 tokens — above every real-session boundary and above two
of the three synthetic ones. A table that mixes them compares "everything retained" against
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
* **compression ratio** — that projection over the history it replaced, which is shared by all
  arms at a boundary;
* **compaction cost** — what *building* the projection costs, once, kept separate from scoring.
  A strategy that calls a model to summarise and one that does not cannot be compared on one
  blended number;
* **per-turn cost** — what *carrying* it costs on every later turn, cache-served, against the
  no-compaction baseline, plus the turns needed to repay the one-off compaction.

Compaction cost and per-turn cost are different quantities and the report keeps them apart: a
model-free strategy has the first at exactly 0 and the second above 0, because its projection
is smaller than the raw history rather than absent.

### Cache-aware cost

The ledger's charge is the **cold** number — these runs do not reproduce the production prefix,
so nothing is cache-served and the figure is an upper bound. Production serves a
cache-friendly summary from the provider's prefix cache, where a cache read costs 0.02 CNY per
1M tokens against 1.0 for fresh input. `report.json` therefore prices every arm at the
`MODELLED_CACHE_HIT` rate (0.98) as well, and at full price for arms that cannot share the
prefix.

Eligibility is a property of the request each strategy builds, decided at token 0, and is
recorded in `CACHE_ELIGIBLE` in `production_shape_rerun.py` with its reason beside it:

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

Same six chains, questions and seeds as the earlier runs. Scored boundaries are moved back to a
pair-complete, message-complete endpoint by `safe_cuts`: `four_arm_rerun.schedule` enforces
that for its chunk boundaries but appends the forced cuts unchecked, which put two real-session
cuts inside a call/result pair.

## Reproducing

```sh
# capture the call shape from a real turn (isolated HOME, fresh port, local stub)
python3 scripts/abc_experiment/capture_shape.py --binary target/debug/future --out ~/compact-exp/shape

# build the drivers
cargo build -p future-agent --example abc_strategy_probe --example abc_probe_bridge
cp scripts/abc_experiment/abc_main_probe.rs <released-checkout>/agent/examples/
(cd <released-checkout> && cargo build -p future-agent --example abc_main_probe)

# the fair run, and the as-deployed run
python3 scripts/abc_experiment/production_shape_rerun.py \
    --output ~/compact-exp/v4-forced --force-compaction \
    --driver target/debug/examples/abc_strategy_probe \
    --main-driver <released-checkout>/target/debug/examples/abc_main_probe \
    --bridge target/debug/examples/abc_probe_bridge --shape ~/compact-exp/shape \
    --budget 300
python3 scripts/abc_experiment/production_shape_rerun.py --output ~/compact-exp/v4-deployed \
    --driver ... --main-driver ... --bridge ... --shape ... --budget 300
```

Closed book only; the open-book phase needs explicit user approval, as before.
