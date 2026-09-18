# The compaction experiments

Two experiments, one directory. Each has a report under
`docs/internals/compaction/`, and this directory holds everything needed to rerun it.

| Report | Question | Entry point |
|---|---|---|
| [compaction-closed-book-experiment.md](../../docs/internals/compaction/compaction-closed-book-experiment.md) | which retention strategy carries the most history, and what does each cost? | `run_closed_book.py` |
| [compaction-open-book-experiment.md](../../docs/internals/compaction/compaction-open-book-experiment.md) | once the model can search the original history, does it recover what compaction dropped? | `run_open_book.py` |

Both send production's own request: the session's system prompt, unchanged, and the tool
definitions from `coding_tools()`. Both score with the same questionnaire (`exam.py`), so a
number from one report is comparable with a number from the other.

**Costs real money.** Every model call is reserved and settled in the run directory's ledger
before it is sent, and a run stops at its `--budget`. Inputs, ledgers and results live in
`~/compact-exp` (override with `ABC_ROOT`), outside any repository, because the real-session
chains are private conversations.

## Building the pieces

```sh
# probes and the tool executor
cargo build -p future-agent --example compaction_probe \
                          --example production_tool_executor \
                          --example model_bridge
# the CLI, for the archive the open-book exam searches
cargo build -p future-cli --bin future
```

`capture_shape.py` needs a session system prompt and the real tool definitions, because a
frozen session's prompt is rebuilt from its working directory and is not in the journal.
Capture them once from a live turn:

```sh
python3 scripts/compaction_experiment/capture_shape.py --binary target/debug/future \
    --project <a real project dir> --out ~/compact-exp/shape
```

## Closed book

```sh
MAIN=$(git -C <released-checkout> rev-parse HEAD)
cargo build -p future-agent --example compaction_probe          # in this worktree

python3 scripts/compaction_experiment/run_closed_book.py \
    --output ~/compact-exp/v4-forced --force-compaction \
    --driver target/debug/examples/compaction_probe \
    --main-driver <released-checkout>/target/debug/examples/released_probe \
    --main-commit "$MAIN" \
    --bridge target/debug/examples/model_bridge \
    --shape ~/compact-exp/shape --budget 300

# the same chains under the production trigger instead of a forced one
python3 scripts/compaction_experiment/run_closed_book.py \
    --output ~/compact-exp/v4-deployed --driver ... --main-driver ... \
    --main-commit "$MAIN" --bridge ... --shape ... --budget 300
```

`--report-only` re-derives `report.json` from an existing run without calling a model. The
`main` arm needs `released_probe.rs` copied into that checkout and built there:

```sh
cp scripts/compaction_experiment/released_probe.rs <released-checkout>/agent/examples/
(cd <released-checkout> && cargo build -p future-agent --example released_probe)
```

`--main-commit` is required: the harness cannot infer which code a binary was built from, and
an unlabelled `main` column rots as the released branch moves.

`--force-compaction` is what makes the comparison fair. Codex and OpenCode compact at every
boundary unconditionally; the Rust arms skip compaction when the economic trigger has not
fired. Without it the table measures the trigger rather than the retention policy. See
[CLOSED_BOOK_PROTOCOL.md](CLOSED_BOOK_PROTOCOL.md).

## Open book

One arm at a time. `--smoke` validates the whole path with no model call, which is worth doing
first because it costs nothing:

```sh
python3 scripts/compaction_experiment/run_open_book.py \
    --closed ~/compact-exp/v4-forced --arm summarized --output ~/compact-exp/open-summarized \
    --future target/debug/future \
    --executor target/debug/examples/production_tool_executor \
    --driver   target/debug/examples/compaction_probe \
    --shape-probe target/debug/examples/compaction_probe \
    --base-prompt ~/compact-exp/shape/system-prompt.txt \
    --bridge target/debug/examples/model_bridge \
    --smoke --only export          # drop --smoke for the real run
```

Two flags change what is measured, and they are not interchangeable:

* `--require-retrieval` adds a verification instruction to the question and forces the first
  tool call. This is **not a production behaviour**; it measures the upper bound of what is
  retrievable, not what a session does. Every result in the report that depends on it says so.
* `--exam-wording v2` drops the question's claim that the projection is the complete record.
* `--closed-reprobe` re-scores the frozen closed projections under the current wording, so a
  wording change is checked against a matched baseline rather than the stored one. No tools.
* `--retry-unsettled` explicitly retries a call an operator abort left in `started`, keeping
  its reservation as spend. The ledger refuses to retry silently, and that is the default.

Interrupting a run is safe: finished cases are kept and skipped on restart.

## Laying out the inputs

`freeze_sessions.py` snapshots real sessions out of the Agent's database, once, to immutable
JSON. Nothing is generated into the repository.

```sh
python3 scripts/compaction_experiment/freeze_sessions.py
```

The synthetic chains (`export`, `analysis`, `pipeline`) come from seeded generators and
regenerate byte-identically; the real chains (`real-yt`, `real-visual`, `real-stream`) cannot
be published at all. Third-party reproduction of the real half means substituting your own
sessions — the absolute numbers will differ, the comparisons should hold.

## Files

| File | What it is |
|---|---|
| `run_closed_book.py` | closed-book entry point: five arms, four metric families, `--report-only` |
| `run_open_book.py` | open-book entry point: one arm, tool loop, per-case traces |
| `harness.py` | shared base: inputs, questionnaire wiring, cadence, the paid-call ledger |
| `arms.py` | the two branch arms and their normalisation |
| `strategies.py` | Codex and OpenCode selection rules, transcribed from upstream |
| `exam.py` | the questionnaire and its scoring |
| `request_shape.py` | production's prompt and tool definitions, fetched from the Rust code |
| `transport.py` | SSE parsing, size estimate, atomic JSON writes |
| `native_stores.py` | an isolated Agent database holding a copy of one chain's records |
| `native_future_shell.py` | executes model tool calls through production's handlers |
| `native_codex.py` | isolated environment shared by the replay adapters |
| `capture_shape.py` | captures a real turn's system prompt and tool definitions |
| `freeze_sessions.py` | snapshots real sessions to immutable JSON |
| `provenance.json` | upstream repo, commit and git blob SHA for every transcribed rule |
| `released_probe.rs` | driver for whatever a checkout deploys; build it in that checkout |
| `opencode_fidelity.mjs` | the pinned AI SDK that computes OpenCode's serialisation |
| `verify_request_shape.py` | asserts the exam sends production's prompt and tools |

## Verifying without spending

```sh
python3 scripts/compaction_experiment/verify_request_shape.py   # prompt, tools, CLI, isolation
python3 scripts/compaction_experiment/run_open_book.py ... --smoke
python3 scripts/compaction_experiment/run_closed_book.py --output <dir> --report-only
```

`verify_request_shape.py` checks that the system prompt is the session's own byte for byte and
does not change across a checkpoint, that the tool definitions from the probe and from the
executor agree, that `future session history` is still present and usable, and that the
executor refuses a path outside the case and a tool production does not install.

## Known limits

* **The real-session half is not reproducible by a third party.** It needs your own sessions.
* **One draw per case.** Small differences are not resolvable; nothing here claims significance.
* **The exam is recognition.** It asks for exact values, which favours verbatim retention over
  summarisation.
* **`shell` is not confined to the case.** Production's sandbox bounds it, but a replay's tool
  calls can still name paths outside the workspace; the harness bounds the file tools with
  `allowed_roots` and cannot bound `shell`. Treat host reads as possible and check traces.
