# Compaction experiment harness

## Current fidelity-corrected closed-book run (v3)

Use [FIDELITY_PROTOCOL.md](FIDELITY_PROTOCOL.md), `fidelity_rerun.py`, and
`opencode_fidelity.mjs` with the isolated, locked AI SDK 6.0.168. V3 implements
Codex real-message requests, its byte-based user budget/middle truncation, and
OpenCode's SDK-based tail-token selection and correct output cap. All arms use
matched generation settings and tool-pair-safe boundaries. It is closed-book
only; open-book remains subject to explicit user approval.

**Neither run measures the cache.** Both give the C3 arm a system prompt and tool
list that no session sends, so its cache counters describe the arm that primed
the prefix (Codex in v3, an earlier `C3proj` run in v2), not the strategy. The
protocol documents say so; the production measurement is in
[docs/compaction-abc-experiment.md](../../docs/compaction-abc-experiment.md).

[FOUR_ARM_PROTOCOL.md](FOUR_ARM_PROTOCOL.md), `four_arm_rerun.py` and
`four_arm_open.py` are retained as **historical v2**, not an upstream-fidelity
reference. Likewise, the older six-chain/external score scripts below are
historical experiments. In particular `external_score.py` used simplified
external prompts and clipped history. `abc_external_strategies.py` retains old
helpers for reproducing that history; v3 uses its own corrected Codex budgeting
and actual SDK lowering instead. Do not mix scores across versions. Private
artifacts and upstream fixtures belong outside this repository.


Drives six compaction strategies over identical frozen inputs and scores the same
questionnaire against each one. Every model call is reserved in a ledger before it is sent
and settled to the provider's reported usage afterwards.

## Inputs and where they live

The experiment root defaults to **`~/compact-exp`**; `ABC_ROOT` overrides it. It is
deliberately outside any repository, because parts of it are real session data and large
ledgers that must never be committed. Scripts **stop with an error** when an input they need
is absent — they do not skip it, because a partial run otherwise looks like a complete one.

| Input | Reproducible from the repo? | How to supply it |
|---|---|---|
| `data/*.json` (3 synthetic chains × 8 stages) | **Yes** | `python3 scripts/abc_compaction_experiment.py prepare --root DIR` and `python3 scripts/abc_experiment/build_pipeline_chain.py`. Both are seeded, and regenerating produces byte-identical fixtures. |
| `frozen-sessions/` (3 real sessions) | **No** | Private. Create `real-sessions.json` with your own session ids, then run `freeze_sessions.py`. See below. |
| `synthetic-session-db/` | Yes | `make_synthetic_sessions.py`; it copies the local Agent database and writes the synthetic sessions into the copy. |
| ledgers, projections, results | Derived | Produced by the runs; see "Running it". |

**Scripts that only use the synthetic chains** run on a fresh checkout with nothing but
`prepare`. **Scripts that use real sessions cannot** — they need your own conversations,
because the report's real-session numbers come from private sessions that are not and will
not be published.

### Supplying real sessions

1. Create `~/compact-exp/real-sessions.json`:

   ```json
   {"chains": {"my-session-a": "<session-id>", "my-session-b": "<session-id>"}}
   ```

   Session ids are the ones in `future session list` / the Agent database. `excluded` is
   optional and records ids that were deliberately not measured.

2. Run `python3 scripts/abc_experiment/freeze_sessions.py`. It snapshots each session once
   to `frozen-sessions/<name>.json`; every later step reads the snapshot, so results stay
   reproducible even while the live session keeps changing.

Chain names are free-form, but a few scripts default to `real-yt`, `real-visual`,
`real-stream` (the names used in the report) and take `--chains` to override.

| Arm | Strategy |
|---|---|
| `A` | protected user/assistant originals + recursive model summary + recent tail (earlier default) |
| `B` | same originals and tail, summary removed and not replaced |
| `C` | same originals and tail, summary slot replaced by deterministic tool evidence (**current default**) |
| `M` | whatever the checked-out branch implements, run through its own code |
| `codex` | `openai/codex` local inline compaction: all user messages (≤20 000 tokens) + summary |
| `opencode` | `anomalyco/opencode`: summary + retained tail (`min(15 000, max(2 000, usable/4))`) |

Results and limits: [docs/compaction-abc-experiment.md](../../docs/compaction-abc-experiment.md).

The harness runs two kinds of probe over each arm's projection: **closed-book**,
and **search-enabled** with the identical archive CLI, so the search engine can be
held constant while the projection varies.

## Files

| File | Purpose |
|---|---|
| `scripts/abc_compaction_experiment.py` | harness: fixtures, ledger, M-arm driver, external arms, probes, reporting |
| `scripts/abc_external_strategies.py` | Codex/OpenCode selection rules and prompts, transcribed from upstream |
| `scripts/abc_external_provenance.json` | upstream repo, commit and git blob SHA for every file that was read |
| `agent/examples/abc_probe_bridge.rs` | direct model transport; reads the local Agent registry, refuses models outside the allowlist |
| `scripts/abc_experiment/abc_compaction_arm.rs` | M-arm driver: dumps any checkout's projection. Copy it into the checkout you want to measure |
| `scripts/abc_experiment/summarize.py` | prints the six-arm table and cost breakdown from a results root |
| `scripts/abc_experiment/search_ablation.py` | prints the closed-book vs search-enabled table and delivered-only accuracy |
| `scripts/abc_experiment/interface_ablation.py` | compares the three retrieval interfaces across every arm |
| `scripts/abc_retrieval.py` | the three lookup interfaces (ours / Codex-shaped / OpenCode filesystem) |

## Running it

```sh
# 1. freeze the fixtures (idempotent; refuses to overwrite a frozen set)
python3 scripts/abc_compaction_experiment.py prepare --root DIR

# 2. build the transport
cargo build -p future-agent --example abc_probe_bridge

# 3. arm M: build the driver inside the branch you want to measure
cp scripts/abc_experiment/abc_compaction_arm.rs <that-checkout>/agent/examples/
(cd <that-checkout> && CARGO_TARGET_DIR=... cargo build -p future-agent --example abc_compaction_arm)
python3 scripts/abc_compaction_experiment.py main --root DIR \
    --arm-binary <that-checkout>/target/debug/examples/abc_compaction_arm \
    --bridge target/debug/examples/abc_probe_bridge --window 32000 --budget 25

# 4. external arms (Codex, OpenCode); --stages lists the probe stages to record
python3 scripts/abc_compaction_experiment.py external --root DIR --arm codex \
    --bridge target/debug/examples/abc_probe_bridge --window 1000000 --stages 0 3 7 --budget 25
python3 scripts/abc_compaction_experiment.py external --root DIR --arm opencode \
    --bridge target/debug/examples/abc_probe_bridge --window 1000000 --stages 0 3 7 --budget 25

# 5. score each recorded projection, then summarise
python3 scripts/abc_compaction_experiment.py probe --root DIR --arm codex --bridge target/debug/examples/abc_probe_bridge
python3 scripts/abc_compaction_experiment.py probe --root DIR --arm codex --bridge target/debug/examples/abc_probe_bridge \
    --binary target/debug/future --retrieval        # same questionnaire, archive CLI available
python3 scripts/abc_compaction_experiment.py probe --root DIR --arm codex --bridge target/debug/examples/abc_probe_bridge \
    --retrieval --retrieval-mode codex              # Codex-shaped window/item interface
python3 scripts/abc_compaction_experiment.py probe --root DIR --arm codex --bridge target/debug/examples/abc_probe_bridge \
    --retrieval --retrieval-mode opencode           # filesystem glob/grep/read
python3 scripts/abc_experiment/summarize.py --root DIR
python3 scripts/abc_experiment/search_ablation.py --root DIR
python3 scripts/abc_experiment/interface_ablation.py --root DIR
```

`--retrieval-mode` picks the lookup interface: `ours` (the `future session history`
CLI, and the only mode that needs `--binary`), `codex` (windows, short item IDs,
character offsets, case-sensitive search) or `opencode` (glob/grep/read over a
materialised working tree). Each starts an isolated HOME; every arm uses the same
interface, the same 5-call budget and the same 32 KB returned-byte budget, so the
lookup engine is a controlled variable.

`--budget` is a hard CNY ceiling shared by every run in `DIR`; the ledger stops the
experiment rather than overrunning it. `--id-suffix` re-runs a step under a new
identity (the ledger refuses to silently overwrite a recorded one).

## External arms: history replay

Codex and OpenCode replace the conversation, so the Nth compaction reads the
already-compacted history plus what was added since — not the raw archive the
journal-backed arms rebuild from. The driver therefore:

* forces one compaction at each probe stage, so the recorded projection is
  comparable with the other arms at the same boundary;
* compacts between probes on the strategy's own overflow trigger, before the
  accumulated history would exceed the provider window.

`PROVIDER_WINDOW`, `PROVIDER_MARGIN` and `CALIBRATION` in the harness are
*measured*, not assumed: a forced stage-4 Codex request was rejected with
"maximum context length is 1048576 tokens. However, you requested 1072531", while
the harness estimated those messages at ~766 K — hence the 1.45× factor.

Behaviour transcribed from source and worth knowing when reading results:

* OpenCode declines to compact when the summary comes back empty (`!summary.trim()`)
  and keeps the history; the driver records that as `declined`.
* Neither agent inspects the completion reason, so a summary truncated by the
  output cap is accepted and flagged `summary_truncated` here.
* OpenCode's summary output is hard-capped at 4 096; Codex uses the model maximum.

## Ledger states

`finished` (settled to reported usage), `invalidated` (implementation was wrong;
charge kept), `interrupted` (operator abort; settlement unknown, reservation kept
as spend), `#failedN` (a recorded attempt that was retried; both charges kept).
An `interrupted` or `started` row is never retried automatically.
