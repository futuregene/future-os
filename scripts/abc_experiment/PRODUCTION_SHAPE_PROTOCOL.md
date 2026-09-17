# Production-shaped closed-book protocol (v4)

Supersedes the v2/v3 closed-book runs **for any statement about the production strategies'
call shape**. v4 does not overwrite their artifacts and does not mix their scores with
these; it is a new root with its own ledger.

## What was wrong with the earlier shape

v3 ran the Rust arms with a **simulated 128K window**, **no output reservation** and **no
request budget**. v3's own protocol recorded that framing as "the simulated retention-policy
window, not a claim about actual tokenizer capacity". It was a reasonable way to compare
retention rules at a matched size, but it meant the numbers were not the ones production
computes, and the deployment's own trigger was never exercised.

v4 drives the same strategies with the limits and the call shape the runtime uses:

| | v3 | v4 |
|---|---|---|
| window | simulated 128K | the registry's declared window (1 000 000) |
| output reservation | none | `effective_max_tokens` (384 000) |
| request budget | not applied | `set_request_budget` with the session's system prompt and the real tool definitions |
| system prompt | Codex's base instructions, shared with the Codex arm | a captured production prompt (`capture_shape.py`) |
| trigger/phase | not modelled | Automatic + PreTurn (production's pre-turn compaction) |
| cadence | fixed 64K chunks | accumulates to the economic trigger |

An earlier version of v4 also omitted `set_request_budget`. That is not a detail: the
runtime calls it at the call site, and without it `fixed_input_tokens` and
`output_reserve_tokens` are zero, so the effective limit is higher and compaction fires
later than production does. The driver now reproduces it, along with both prompts the
runtime derives (`budget_system` always reserves the recall guidance; the outgoing prompt
carries it only once a checkpoint exists).

## Two arms, one new

v4 adds the **deployed algorithm** as an arm: `main` builds
[`abc_main_probe.rs`](abc_main_probe.rs) inside a checkout of the released branch and calls
that checkout's own entry point (`prepare_semantic_with_phase`) with that checkout's own
`context_token_budgets`. It is not a reimplementation and not a port; it is the code that
ships, measured the same way as the rest. "Is the new strategy better than what is
deployed" was previously unanswerable from this harness.

## Fair comparison: post-compaction against post-compaction

The arms do not share a trigger. Codex and OpenCode compact at every scored boundary
unconditionally; the Rust arms obey an economic trigger that, on a 1M window with a 384K
output reservation, sits at 613 952 tokens — above every real-session boundary and above
two of the three synthetic ones. A table that mixes them compares "everything retained"
against "compacted" and reports the shape of the trigger, not the quality of the retention
policy.

v4 therefore runs both ways:

* **`--force-compaction` (the fair comparison).** The Rust arms use Manual, which bypasses
  the economic trigger by design, so every arm compacts at every boundary. All 24 boundaries
  qualify. Only this mode answers "given that each policy compacted, which retains more of
  what the questions ask for, at what size and cost".
* **default (as deployed).** Production's trigger, so the deployment's real behaviour is
  measured — including that it often does not compact at all. Its quality column is *not* a
  retention-policy measurement and must not be quoted as one.

## Metrics

Every arm is reported on the same axes, because none of them alone is the answer:

* **quality** — hits / of present, plus false positives and invalid answers;
* **tokens after** — the projection the next request carries, in one estimator for every arm;
* **compression ratio** — that projection over the history it replaced, which is shared by
  all arms at a boundary;
* **compaction cost** — CNY spent by the compaction calls only, kept separate from scoring.
  A strategy that calls a model to summarise and one that does not cannot be compared on a
  single blended number;
* **per-turn cost** — the same projection re-sent on every later turn, cache-served, plus the
  no-compaction baseline (the raw history) it is read against, and the number of turns it
takes for the one-off compaction to be repaid. Reporting only the one-off compaction cost
would invert the comparison; reporting only per-turn cost would hide that compacting needs
tens of turns to pay for itself.

Everything the report contains must stay distinguishable: **compaction cost** (once, to build
the projection) and **per-turn cost** (recurring, to carry it) are different quantities, and a
model-free strategy has the first at exactly 0 while the second stays above 0, because its
projection is smaller than the raw history rather than absent.

### Cache-aware cost

The ledger's charge is the **cold** number: these runs do not reproduce the production
prefix, so nothing is cache-served and the figure is an upper bound. Production serves a
cache-friendly summary from the provider's prefix cache, where a cache read costs 0.02 CNY
per 1M tokens against 1.0 for fresh input. `report.json` therefore also prices every arm at
the `MODELLED_CACHE_HIT` rate (0.98), and at full price for arms that cannot share the
prefix.

Eligibility is a property of the request each strategy builds, decided at token 0:

* `summarized` — **eligible**: it passes the session's own system prompt and tool
  definitions and the live conversation as real messages. The deployed path measured 99.8 %
  cache read in production, which is what the 0.98 stand-in is anchored to.
* Codex — **eligible**: it reuses its base instructions and appends its instruction last.
* `main` — **not eligible**: it substitutes its own `SUMMARY_SYSTEM_PROMPT` constant.
* OpenCode — **not eligible**: it sends a dedicated compaction system prompt.

The eligibility table lives in `CACHE_ELIGIBLE` in `production_shape_rerun.py`, with the
reason recorded next to each entry so the claim can be checked against the code rather than
taken on trust.

**Do not read the runs' own cache counters as evidence here.** They are contaminated: the
arms run in one block and prime each other, and a later run reuses prefixes an earlier one
left in the provider's cache. One recorded batch showed a 100 % hit on a request whose
prefix the previous (aborted) run had sent. The model exists precisely because the
measurement is not available from this harness.

## Inputs

Same six chains, same questions, same seeds as v3. Boundaries are forced at the three
scored points and moved back to a pair-complete, message-complete endpoint
(`safe_cuts`) — `four_arm_rerun.schedule` enforced that for its chunk boundaries but
appended the forced cuts unchecked, which put two real-session cuts inside a
call/result pair.

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

Closed book only. No open-book approval is implied by this protocol; the open-book phase
remains subject to explicit user approval, as in v3.
