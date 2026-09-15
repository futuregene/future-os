# Compaction experiment harness

Drives six compaction strategies over identical frozen fixtures and scores the
same questionnaire against each one. Everything here is synthetic; no private
conversation is read, and every model call is reserved in a ledger before it is
sent and settled to the provider's reported usage afterwards.

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
python3 scripts/abc_experiment/summarize.py --root DIR
python3 scripts/abc_experiment/search_ablation.py --root DIR
```

`--retrieval` starts its own isolated Agent (fresh HOME, fresh port) and gives the
probe a `shell` tool that accepts only `future session history search|get` scoped
to that stage's archive session; anything else is refused by policy. Every arm
uses the same tool, the same 5-call budget and the same 32 KB returned-byte budget,
so the search engine is a controlled variable.

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
