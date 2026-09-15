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

## Files

| File | Purpose |
|---|---|
| `scripts/abc_compaction_experiment.py` | harness: fixtures, ledger, M-arm driver, external arms, probes, reporting |
| `scripts/abc_external_strategies.py` | Codex/OpenCode selection rules and prompts, transcribed from upstream |
| `scripts/abc_external_provenance.json` | upstream repo, commit and git blob SHA for every file that was read |
| `agent/examples/abc_probe_bridge.rs` | direct model transport; reads the local Agent registry, refuses models outside the allowlist |
| `scripts/abc_experiment/abc_compaction_arm.rs` | M-arm driver: dumps any checkout's projection. Copy it into the checkout you want to measure |

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
    --bridge target/debug/examples/abc_probe_bridge --window 32000 --budget 10

# 4. arms X/O (Codex, OpenCode)
python3 scripts/abc_compaction_experiment.py external --root DIR --arm codex \
    --bridge target/debug/examples/abc_probe_bridge --window 1000000 --stages 0 --budget 10
python3 scripts/abc_compaction_experiment.py external --root DIR --arm opencode \
    --bridge target/debug/examples/abc_probe_bridge --window 1000000 --stages 0 --budget 10

# 5. score, then summarise
python3 scripts/abc_compaction_experiment.py probe --root DIR --arm codex \
    --bridge target/debug/examples/abc_probe_bridge --budget 10
python3 scripts/abc_compaction_experiment.py report --root DIR --arm codex
```

`--budget` is a hard CNY ceiling shared by every run in `DIR`; the ledger stops
the experiment rather than overrunning it. `--stages` limits which compaction
points are generated — later stages for the external arms are expensive because
those strategies summarise the entire history.

## Why the external arms are stage-limited

Codex and OpenCode both summarise the *whole* conversation, so their cost grows
with the archive: at stage 1 the history is ~250 K tokens (≈¥0.27 per compaction
for Codex), at stage 4 ~1 M tokens and at stage 8 ~2 M tokens. The recorded run
covers stage 1 for both, which is the first and most common compaction point.
Extending them to stages 4 and 8 was not affordable inside the ¥10 cap.

## Ledger states

`finished` (settled to reported usage), `invalidated` (implementation was wrong;
charge kept), `interrupted` (operator abort; settlement unknown, reservation kept
as spend). An `interrupted` or `started` row is never retried automatically.
