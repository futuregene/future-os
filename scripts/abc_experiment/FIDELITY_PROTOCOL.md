# Fidelity correction v3 — CLOSED BOOK ONLY

This supersedes the external-arm implementation in the v2 experiment; it does
not overwrite its artifacts or combine old control scores with new external
scores. User approval covers correction and closed-book retesting. Open-book
approval remains withheld. Total admission ceiling is CNY 300 **including**
v2's CNY 21.4803698; v3 accounts that immutable prior ledger as opening spend.

## What is compared

Text/tool retention-policy replay on the same six reduced frozen histories,
not the complete deployed applications, native trigger timing, multimodal
memory or hosted retrieval. The provider declares a 1M context window; 128K is
the simulated **retention-policy window**, not a claim about actual tokenizer
capacity. The common schedule adds about 64K estimated new history and forces
compaction at scored boundaries. Questionnaire is the same seeded exact-value
recognition instrument (including its early-candidate bias), not task execution.
No population significance claim; six chains, only three real conversations.

### What this run cannot say about the cache

The C3 arm is given Codex's base instructions (`--system-prompt-file`) and an
empty tool list, so C3 and Codex share a prefix and differ only in what they
retain. A provider caches on the request prefix — system prompt, then tool
definitions, then messages, compared from token zero — so this run's C3 requests
are **not** the shape production sends, and the two arms prime each other:

* the ~99 % stage-0 hit rate in the v3 ledger is the Codex arm warming the
  prefix C3's request then reuses (the arms run in one block, in randomized
  order); later stages record the ordinary ~10 % that consecutive compactions
  earn by sharing a head;
* on a primed prefix, substituting the system prompt, dropping the tool
  definitions, or adding a single line to the system prompt each measured
  **0 %** in a separate check, so neither the hit rates here nor their absence
  bound the production cost.

Do not quote any cache counter from these ledgers as a property of the C3
strategy. The production measurement (99.8 % on a 212 911-token prefix, on an
isolated agent running the production path) is in
[the comparison report](../../docs/compaction-abc-experiment.md).

## Corrections fixed before paid calls

- Align EVERY arm's compaction and probe boundaries to complete stored messages
  with all tool calls answered. Prior v2 probe boundaries sometimes separated
  a call/result. Move a scored boundary backwards to the nearest safe endpoint.
  Record new cutoffs and questionnaires, and rerun all four arms under them.
- Normalize new raw segments once through the Rust probe's `--dump-messages`.
  Verify every tool call/result count and all text survives. All external arms
  receive those same normalized messages; arbitrary unmatched tool pairing,
  hidden deletion and synthetic reexecution are forbidden. The reduced export
  lacks some original arguments/media/error metadata; no implementation can
  reconstruct information absent from the frozen inputs.
- Codex local-inline path: actual user/assistant/tool message array, pinned
  fallback base instructions, pinned final compaction prompt, recursive live
  state. Protect users using UTF-8 byte/4 ceiling and middle truncation with
  the upstream marker (not a prefix-only slice or our CJK-weighted estimator).
  Its output ceiling follows the frozen provider model maximum, not the prior
  experiment's 16K artificial cap. The current model declares 384K output;
  reserve that full ceiling before sending even though observed outputs are
  much shorter. No changing the cap after seeing a score.
- OpenCode path is consistently `packages/opencode/src/session/compaction.ts`
  at e03db9bc: tail selection by user turns, splitTurn, no-tail fallback;
  serialize selected head with bounded tool text and prior summary; retain
  tail. Convert text/tool messages with the exact upstream AI SDK **6.0.168**
  and compute `Math.round(JSON.stringify(modelMessages).length/4)` using JS
  UTF-16 length. The same SDK output determines actual tail selection. Output
  ceiling is `min(model.limit.output, 32000)` for this path, not core's 4096.
- All summary and answer requests explicitly disable thinking. C3 uses the
  same fixed base instructions as Codex for this controlled replay, supplied
  through its production API; OpenCode retains its dedicated summary-system
  prompt as part of its selected policy. Output allowances differ by policy
  and are recorded, not represented as equal-size ablations. C is model-free.
- A local HTTP regression tests C3's actual outgoing `thinking.type=disabled`
  and `max_tokens=8192` through the observed provider wrapper. C3 returns its
  logical requests for auditing. External requests are recorded in full in
  the private output directory. No original historical tools are executed.

## Evidence and validation

`fetch_fidelity_sources.py` downloads fixed public commit contents with git-blob
SHA verification. Sources, SDK package-lock, model metadata, binaries and all
relevant Python/JS code are hashed in `fidelity-manifest.json`. **Rebuild
`--driver`/`--bridge` from the commit recorded in `SOURCE_REVISION.json`**: the
manifest hashes the executables, so a later build of the same source tree — after
any change to `agent/src` or `agent/examples` — no longer matches, and the
recorded artifacts can only be re-verified against a binary built at that commit.

`test_fidelity.py` covers byte-vs-character counting, middle truncation,
recursive user/summary roles, tool pairing, safe boundaries, cumulative budget,
real SDK lowering and actual JSON tail-token computation. These are offline.

Before running, inspect every normalized segment and dry-run all OpenCode tail
selections without a model. Refuse malformed inputs, model substitution,
changed manifest, unsettled calls, concurrent runner or unapproved budget.
A single owner writes the new ledger. Empty/failed responses stop rather than
silently skip; C3's designed deterministic fallback remains in that arm.

## Running

```sh
# Install ONLY in an isolated research directory (not the repo/global node_modules):
# npm install --prefix ~/compact-exp/fidelity-sdk --ignore-scripts --no-audit --no-fund ai@6.0.168
python3 scripts/abc_experiment/fetch_fidelity_sources.py --output ~/compact-exp/fidelity-sources
CARGO_TARGET_DIR=target/fidelity cargo build -p future-agent --example abc_c3_probe --example abc_probe_bridge
python3 -m unittest discover -s scripts/abc_experiment -p test_fidelity.py -v
CARGO_TARGET_DIR=target/fidelity cargo test -p future-agent --example abc_c3_probe
python3 scripts/abc_experiment/fidelity_rerun.py \
  --previous ~/compact-exp/rerun-20260916-v2 \
  --output ~/compact-exp/rerun-fidelity-v3 \
  --driver target/fidelity/debug/examples/abc_c3_probe \
  --bridge target/fidelity/debug/examples/abc_probe_bridge \
  --sdk-root ~/compact-exp/fidelity-sdk --sources ~/compact-exp/fidelity-sources \
  --prepare-only
# Remove --prepare-only to execute, --detach for monitored background execution.
```

The evaluator and shared forced schedule remain deliberately controlled
benchmark choices; fixing representation fidelity does not make this a native
end-to-end product ranking. Report final scores with per-chain breakdown,
mechanical containment, invalid/truncated responses, C3 fallbacks, costs and
all changed conditions. Keep every older run labelled with its own version.
