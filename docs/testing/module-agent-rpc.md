# Module `agent/rpc` + `agent/grpc` (worker group `ag-rpc`) — coverage handoff

> **Round 2 (cov100-r2) supersedes §1–§9 below for scope, figures and ledger.**
> §1–§9 are the round-1 record (2026-08-27, revision `c173e64e`) and are kept
> verbatim; their `verify.py`/`ag-rpc` gate no longer exists in round 2 (the
> round-2 unit is `todo_b6e2e6ae5236`, scope = the seven subtrees listed in §0.1,
> metric = per-line LCOV `DA:0`). Round-2 state: **§0**.

## 0. Round 2 — `todo_b6e2e6ae5236` (unit `AG-A`)

Round 2 replaces round 1's `verify.py` gate and its statement-level figures. Here
the metric is **per-line LCOV `DA:0`** (`.future/cov100/covlines.py`), the scope
is the seven subtrees in §0.1, and every residual line is classified in §0.5
(either covered by a named test or waiver-tagged with a category + mechanism
from `docs/testing/waiver-ledger.md`).

### 0.1 Scope and run identity

**Which number lives where (read this before quoting a figure).** This section is
the *original* run identity plus the reproducible command, kept as written; the
command block below names the **current** report. **§0.2 is the authoritative
per-segment table** — it lists every segment with the report file its figures came
from — and **§0.5** is the current per-line ledger, whose heading and totals match
the report named in the block below. A figure quoted from anywhere else in this
document is historical; §0.2 says which segment it belongs to.

Subtrees owned by this unit: `agent/src/rpc`, `agent/src/grpc`,
`agent/src/service`, `agent/src/session`, `agent/src/skills`,
`agent/src/approval_review`, `agent/src/cli.rs` (+ tests under `agent/tests`).
Peer workers own the rest of `agent/src`; this record claims nothing outside the
seven subtrees.

* Worktree `/Users/geilige/future-os/.worktrees/cov100-r2`, branch
  `test/cov100-r2`, **revision `8449acf6`** (= `origin/main` at campaign start)
  plus this unit's test-only edits. Other workers' in-flight edits were present
  in the tree during the measurements (§0.7).
* Host macOS arm64; toolchain pinned by `rust-toolchain.toml`
  (`1.97.0-aarch64-apple-darwin`); `cargo-llvm-cov 0.8.7`.
* Environment per `.future/cov100/PLAN.md` §2: private
  `CARGO_TARGET_DIR=target/cov-agent-a`, `HOME=$PWD/target/test-home`,
  `ulimit -n 10240`, `-j 4`, `CARGO_HOME`/`RUSTUP_HOME` pinned to rustup.

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home CARGO_TARGET_DIR=target/cov-agent-a
mkdir -p "$HOME"

  # THE measurement: build + run every test target + report, one process.
  # NOTE: the report name below is THIS unit's CURRENT measurement — the rework's
  # report, on the tree that carries the `cli.rs` 590 test. Every segment's report is
  # named in §0.2; this block used to name segment 6's `/tmp/agent-a-s7*` and then
  # segment 19's `/tmp/agent-a-s26.info`, neither of which described the record (F.48).
cargo llvm-cov -p future-agent --no-fail-fast --lcov \
    --output-path /tmp/agent-svc-rework.info -j 4

  # cargo-llvm-cov writes the worktree's absolute paths; covlines.py --under matches
  # /Users/geilige/future-os/agent/src/... — normalise, then measure:
sed 's|/Users/geilige/future-os/.worktrees/cov100-r2/|/Users/geilige/future-os/|' \
    /tmp/agent-svc-rework.info > /tmp/agent-svc-rework-norm.info
python3 .future/cov100/covlines.py total  /tmp/agent-svc-rework-norm.info \
    --under agent/src/rpc --under agent/src/grpc --under agent/src/service \
    --under agent/src/session --under agent/src/skills \
    --under agent/src/approval_review --under agent/src/cli.rs
python3 .future/cov100/covlines.py files  /tmp/agent-svc-rework-norm.info --under agent/src/rpc ...
python3 .future/cov100/covlines.py detail /tmp/agent-svc-rework-norm.info --under agent/src/rpc ...
```

If one test target fails, `cargo-llvm-cov` emits no report; the profiles are
still on disk, so render them separately (same private target dir):

```bash
cargo llvm-cov report --lcov --output-path /tmp/agent-a-final.info
```

That is the round-1 record's own warning (§1) about `report` re-merging stale
profiles. It is safe here only because the profiles come from a single run in
this worker's private target dir, and the render was sanity-checked (every win
lands in a file this unit edited, no file goes to `0/0`, and the unit total
moved in one direction). The run that produced the final figures had one
load-flaked test fail (`session::persistence::tests::timeout_drain_handoff_executes_queued_command`,
§0.7) — it passes in isolation and left 7 assertion lines uncovered:

```bash
cargo test -p future-agent --lib -- session::persistence::tests::timeout_drain_handoff_executes_queued_command
```

### 0.2 Before / after

| measurement | covered | measured | **uncovered (DA:0)** | lines% |
|---|---|---|---|---|
| supervisor baseline, rev `8449acf6` (`.future/cov100/baseline/ws-dirty-lcov.info`) | 26949 | 27530 | 581 | 97.8896 |
| segment 1 (`/tmp/agent-a-after-norm.info`) | 27078 | 27557 | 479 | 98.2618 |
| segment 2 (`/tmp/agent-a-after2-norm.info`) | 27220 | 27653 | 433 | 98.4342 |
| segment 2 extras (`/tmp/agent-a-final-norm.info`) | 27270 | 27690 | 420 | 98.4832 |
| segment 3 (`/tmp/agent-a-s3-norm.info`) | 27355 | 27740 | 385 | 98.6121 |
| segment 4 (`/tmp/agent-a-s5-norm.info`) | 27472 | 27832 | 360 | 98.7065 |
| segment 5 (`/tmp/agent-a-s6-norm.info`) | 27484 | 27831 | 347 | 98.7532 |
| segment 6 (`/tmp/agent-a-s7-norm.info`) | 27581 | 27905 | 324 | 98.8389 |
| segment 7 (`/tmp/agent-a-s9-norm.info`) | 27682 | 27990 | 308 | 98.8996 |
| segment 8 (`/tmp/agent-a-s12-norm.info`) | 27877 | 28160 | 283 | 98.9950 |
| segment 9 (`/tmp/agent-a-s13-norm.info`) | 27942 | 28214 | 272 | 99.0359 |
| segment 10 (`/tmp/agent-a-s15-norm.info`) | 28001 | 28264 | 263 | 99.0695 |
| segment 11 (`/tmp/agent-a-s17-norm.info`) | 28055 | 28316 | 261 | 99.0783 |
| segment 12 (`/tmp/agent-a-s19-norm.info`) | 28055 | 28316 | 261 | 99.0783 |
| segment 13 (`/tmp/agent-a-s20-norm.info`) | 28056 | 28316 | 260 | 99.0818 |
| segment 14 (`/tmp/agent-a-s21-norm.info`) | 28103 | 28359 | 256 | 99.0973 |
| segment 15 (`/tmp/agent-a-s22-norm.info`) | 28105 | 28359 | 254 | 99.1043 |
| segment 16 (`/tmp/agent-a-s23-norm.info`) | 28112 | 28359 | 247 | 99.1290 |
| segment 17 (`/tmp/agent-a-s24-norm.info`) | 28115 | 28359 | 244 | 99.1396 |
| **the narrative so far, as measured then (`/tmp/agent-a-s25-norm.info`)** | **28118** | **28359** | **241** | **99.1502** |

**−340 uncovered lines** in the unit so far (581 → 241). That 241 is where *this
narrative* had reached, not where the unit ended: the **maintained table further down
this section continues from segment 19 to the rework's 202**, and 202 is the figure
§0.5's ledger and §0.8 describe. Segment 18 closed **three** lines and killed **two
more by arithmetic**:

| work | what changed |
|---|---|
| the folded-projection budget arm | **covered** (−1) — `journal_compaction.rs` 259, with the multi-event fixture below |
| the serialized-snapshot budget arm | **covered** (−1) — `journal_compaction.rs` 289, a different quantity from 259 |
| the pager's page-boundary break | **covered** (−1) — `sqlite_store/events.rs` 369, crossed as a side effect of the same fixtures |
| the pricing-array budget arm | `coverable` → `unreachable-by-construction` (2 lines) — `journal_compaction.rs` 292/293, **proved dead by arithmetic** |

Segment 17 closed **three** lines and took `rpc/broadcaster/replay.rs` to **zero**:

| work | what changed |
|---|---|
| the current-run ring-overflow path | **covered** (−2) — `replay.rs` 172/173: it prefers the stored snapshot over the raw replay |
| the historical run snapshot | **covered** (−1) — `replay.rs` 211: `run_snapshot` returns the published snapshot |
| the multi-line `json!` interior line | `coverable` → `attribution-artifact` — `entry_records.rs` 311, with counts |
| `session/fork.rs` (line 202) | stays `coverable`, now with the six-`?` recipe instead of one sentence — **covered in segment 24** |

Segment 16 closed **seven** lines — the largest single-file win of round 2, and it
took `session/history/paging.rs` to **zero**:

| work | what changed |
|---|---|
| `session/history/paging.rs` 128/129/131/133/134 | **covered** (−5) — the cut at an entry too large for the page, in both directions, once the fixture supplies the shape that makes it reachable (a row with no entry body) |
| `session/history/paging.rs` 145/146 | **covered** (−2) — the UTF-8 boundary walk before slicing a resumed fragment |

Segment 15 closed **two** lines (the memory-only journal lifecycle), measured the
**three lines segment 14 explicitly left unmeasured**, and moved a further **11**
to verified waivers:

| work | what changed |
|---|---|
| `rpc/broadcaster/journal.rs` 93/106 | **covered** (−2) — a memory-only broadcaster's `flush_journal`/`shutdown_journal` are documented no-ops, and both are calls a host makes unconditionally |
| `session/fork.rs` 194/205/384, `journal_writer.rs` 322 | the row segment 14 marked "not yet measured" → `attribution-artifact`, with each line's neighbours recorded |
| `rpc/session.rs` 918/969/1223 | `attribution-artifact` with counts (three closing braces) |
| `rpc/session.rs` 1371 | `unreachable-by-construction` — the `_` arm of the pricing replay, dead by **two** independent `event_type` filters |
| `skills/manager.rs` 1934/1939 | `unreachable-by-construction` — **F-29 again**: they are `assert!` failure-message arguments |
| `rpc/session.rs` 979 | `coverable` → `unreachable-by-construction`: the summarizer builds every checkpoint summary out of text, so no non-text block can reach the arm — and segment 22 added the assertion that measures it (F-51) |
| `service/startup.rs` 27/144/154, `session_lifecycle.rs` 644 | `attribution-artifact` with counts (four more, from the artifact-vocabulary audit) |
| `session_lifecycle.rs` 361 | `unreachable-by-construction` — every write path normalizes string content, and **the test that names this branch takes the other one** (F-34) |

Segment 14 closed **four** lines, moved **21** more to verified waivers, and
**corrected** an over-strong finding from segment 13:

| work | what changed |
|---|---|
| `rpc/commands/session_title.rs` 210/213 | **covered** (−2) — the title handler's success arm, end to end against a local SSE server |
| journal writer's idle shutdown | **covered** (−1) — `journal_writer.rs` 180: shutdown of an already-retired writer is idempotent |
| the listener stage exit | **covered** (−1 in this run) — `shutdown.rs` 303, whose pair with 273 is load-dependent (F-30 corrected below) |
| `skills/manager.rs` 177/265/404/430/434/497/499 | `coverable` → `attribution-artifact` (7), each with its neighbours' counts |
| `skills/registry.rs` 176/220/240, `session/summary.rs` 56/100, `session/manager/mod.rs` 92/215, `session/fork.rs` 118/142, `rpc/broadcaster/journal.rs` 37, `journal_writer.rs` 194 | `coverable` → `attribution-artifact` (12 more), measured line by line |
| `session/legacy_import.rs` 290 | `coverable` → `attribution-artifact` — it is a `tracing` **field line** (F-14), and the row's old mechanism was simply wrong |
| `rpc/broadcaster/journal.rs` 43/45 | `coverable` → `unreachable-in-this-environment` — the *second* ownership re-check, inside the lock-free window |

Segment 13 closed **two** lines for real, moved **eight** more to verified
waivers, and reported a **flip** rather than claiming a third win:

| work | what changed |
|---|---|
| `rpc/commands/skills.rs` 14/15 | **covered** (−2) by `install_skill_publishes_the_installed_skill_after_a_platform_download` — the install command's success arm, which no test had ever taken: line 13 ran **9** times and 14/15 read 0, i.e. all nine installs failed |
| `rpc/session.rs` 3497/3713/4974/5011/5147/5426 | `coverable` → `unreachable-by-construction` (6 lines): all six are `assert!` **failure-message arguments**, which a green suite cannot evaluate (F-29). Renumbered +99 in segment 22 |
| `grpc/mod.rs` 100/101 | mechanism corrected with counts: the macro at 98 runs **31** times, the fields read 0, and 99/102 have no `DA` record (F-14 again) |
| `service/shutdown.rs` 273/303 | **a flip, stated plainly:** segment 12's run had 273 = 0 / 303 = 1; this run has **273 = 1 / 303 = 0**, all three tests green in both. So this segment's net is −1, not −2 (F-30) |

**Segment 12 closed zero lines and moved seven from `coverable` to a verified
waiver** — its whole product was classification, so its row here is the category
move rather than a delta:

| work | what changed |
|---|---|
| audit (b) on `session/database/mod.rs` | `coverable` → `unreachable-in-this-environment` (5 lines; **corrected in segment 23 — see F-54, the arm is reachable but racy**) | with the probe that shows *why*: the success arm runs 235 times and the failure arm needs the worker's own connection to fail, which file damage cannot do because SQLite serves the cached page 1 |
| F-27 on `rpc/session.rs` 885/886 | category kept (`attribution-artifact`), **mechanism corrected**: the row claimed "a disabled DEBUG callsite", but the enclosing macro is an `info!` that runs 6 times — the fields are simply outside the instrumented region |

Segment 11 closed **2**, both by measurement, and its main product was
**verification** rather than lines:

| file | before | now | Δ | newly covered |
|---|---|---|---|---|
| `service/shutdown.rs` | 14 | 13 | **−1** | 273 — the scheduler/monitor stage exit, the last one left (§0.5 has the recipe) |
| `rpc/mod.rs` | 13 | 12 | **−1** | 247 — the failed-rebind log of `create_session`'s foreign-journal path |

The verification half is worth as much as the lines: **all eight `settings.rs`
`platform-unmeasured` rows were checked against their `cfg` sites and hold** (the
only `Err` producer is the Windows branch at `sandbox/mod.rs` 1073-1075; the
macOS and Linux branches build `Ok` unconditionally), and **two `coverable` rows
were mislabelled** — `instance.rs` 34 says in its own text that no fixture can
force it, and `cli.rs` 310 is unreachable-by-construction because
`create_dir_all` fails first. §0.5 carries both, and F-24 records the category
convention this sweep had to settle.

Segment 10 closed **9**, in three files, and it found one **misclassification**:

| file | before | now | Δ | newly covered |
|---|---|---|---|---|
| `service/shutdown.rs` | 18 | 14 | **−4** | 232 (the run/native-work deadline exit), 293 (the journal's shutdown flush fails), 303 (the listener stage after an unpolled `serve`), 307 (the storage close after a panicked worker) |
| `rpc/mod.rs` | 16 | 13 | **−3** | 374/375/380 (the *non-busy* `set_model` failure and its `break`, reached with a model whose protocol cannot be built) |
| `cli.rs` | 12 | 10 | **−2** | 195/197 — the `agent --linux-sandbox-helper` argv rewrite, which this ledger had carried as `platform-unmeasured` and is **not** platform-gated at all |

Segment 9's rows, for reference:

| file | Δ | newly covered |
|---|---|---|
| `service/mod.rs` | **−6** | 79-81 (the local-start gate accepts `" AUTO "`), 105 (the `?` on an unopenable instance lock), 130 (host error **and** failed cleanup, both preserved), 132 (a panicking host is resumed) |
| `rpc/session.rs` | **−4** | 381 (no dequeue once the work boundary closed), 668 (an event with no wire projection is dropped), 673/674 (the run-scope broadcast arm) |
| `service/shutdown.rs` | **−1** | 237 (an ephemeral session's cancelled queue is not written as a terminal row) |

Segment 8's rows, for reference:

| file | before | now | Δ | newly covered (from a `covlines.py detail` diff of s9 vs s12) |
|---|---|---|---|---|
| `rpc/mod.rs` | 28 | 16 | **−12** | 362-373 — the whole "session configuration is busy" retry arm, by holding the session's `agent_loop` write lock so the first attempt reports busy and then releasing it so the retry applies the model |
| `session/legacy_import.rs` | 9 | 4 | **−5** | 20, 72, 86, 111, 322 — the generic read failure, the two context propagations, the uncancelled `Ok(())` arm, and the mid-import cancellation return |
| `rpc/run_snapshot.rs` | 5 | 1 | **−4** | 36, 39, 68, 95 — an unreadable `tool_start`, a start with no tool identity, and an unreadable fragment in both the folding and the reconstruction |
| `skills/manager.rs` | 18 | 14 | **−4** | 411, 412, 416, 417 — the publish failure, the "no backup to restore" decision, and the drop of the pending operation row |

Segment 8 also **withdrew two earlier classifications** rather than bank them:
`legacy_import.rs` 20 was carried as an attribution artifact and is now covered
(it needed an error kind `source_unreadable` rejects), and `rpc/mod.rs` 353/354
was carried as coverable work and is now measured as an artifact (§0.4 F-14).

Per-file movement against the baseline (uncovered lines; every file with any
delta, so the column sums to the −273 exactly):

| file | baseline | now | Δ |
|---|---|---|---|
| `agent/src/rpc/approval.rs` | 45 | 1 | **−44** |
| `agent/src/rpc/commands/session_reset.rs` | 37 | 11 | **−26** |
| `agent/src/session/reset.rs` | 26 | 1 | **−25** |
| `agent/src/rpc/mod.rs` | 36 | 12 | **−24** |
| `agent/src/approval_review/evidence.rs` | 25 | 4 | **−21** |
| `agent/src/service/shutdown.rs` | 34 | 13 | **−21** |
| `agent/src/grpc/mod.rs` | 22 | 2 | **−20** |
| `agent/src/approval_review/provider.rs` | 16 | 2 | **−14** |
| `agent/src/rpc/commands/settings.rs` | 55 | 41 | **−14** |
| `agent/src/rpc/broadcaster/journal_writer.rs` | 19 | 8 | **−11** |
| `agent/src/rpc/session_prompt.rs` | 14 | 3 | **−11** |
| `agent/src/session/history/display_index.rs` | 11 | 2 | **−9** |
| `agent/src/rpc/session.rs` | 24 | 16 | **−8** |
| `agent/src/service/mod.rs` | 14 | 8 | **−6** |
| `agent/src/approval_review/fingerprint.rs` | 5 | 0 | **−5** |
| `agent/src/approval_review/tool_evidence.rs` | 5 | 0 | **−5** |
| `agent/src/session/legacy_import.rs` | 9 | 4 | **−5** |
| `agent/src/rpc/commands/mod.rs` | 5 | 1 | **−4** |
| `agent/src/rpc/run_snapshot.rs` | 5 | 1 | **−4** |
| `agent/src/skills/manager.rs` | 18 | 14 | **−4** |
| `agent/src/approval_review/policy.rs` | 3 | 0 | **−3** |
| `agent/src/approval_review/types.rs` | 3 | 0 | **−3** |
| `agent/src/service/config.rs` | 3 | 0 | **−3** |
| `agent/src/service/instance.rs` | 7 | 4 | **−3** |
| `agent/src/session/journal_compactor.rs` | 4 | 1 | **−3** |
| `agent/src/session/sqlite_store/events.rs` | 7 | 4 | **−3** |
| `agent/src/session/tools.rs` | 3 | 0 | **−3** |
| `agent/src/approval_review/action.rs` | 2 | 0 | **−2** |
| `agent/src/cli.rs` | 12 | 10 | **−2** |
| `agent/src/rpc/approval/escalation.rs` | 3 | 1 | **−2** |
| `agent/src/session/manager/mod.rs` | 7 | 5 | **−2** |
| `agent/src/session/sqlite_store/transcript.rs` | 6 | 4 | **−2** |
| `agent/src/approval_review/context.rs` | 1 | 0 | **−1** |
| `agent/src/service/activity.rs` | 2 | 1 | **−1** |
| `agent/src/service/work.rs` | 1 | 0 | **−1** |
| `agent/src/session/checkpoint.rs` | 1 | 0 | **−1** |
| `agent/src/session/database/mod.rs` | 8 | 7 | **−1** |
| `agent/src/session/history/paging.rs` | 8 | 7 | **−1** |
| `agent/src/session/history/recall.rs` | 5 | 4 | **−1** |
| `agent/src/session/manager/metadata.rs` | 1 | 0 | **−1** |

| segment 19 (`/tmp/agent-a-s26-norm.info`) | 28158 | 28397 | 239 | 99.1584 |
| segment 20, intermediate (`/tmp/agent-a-s27-norm.info`) | 28165 | 28397 | 232 | 99.1830 |
| segment 20 (`/tmp/agent-a-s29-norm.info`) | 28171 | 28397 | 226 | 99.2041 |
| segment 21 (`/tmp/agent-a-s31-norm.info`) | 28215 | 28436 | 221 | 99.2228 |
| segment 22 (`/tmp/agent-a-s32-norm.info`) | 28302 | 28521 | 219 | 99.2321 |
| segment 23 (`/tmp/agent-a-s33-norm.info`) | 28321 | 28530 | 209 | 99.2674 |
| segment 24 (`/tmp/agent-a-s34-norm.info`) | 28398 | 28602 | 204 | 99.2868 |
| segment 25 (`/tmp/agent-a-s35-norm.info`) | 28399 | 28602 | 203 | 99.2903 |
| segment 26, the closure segment (`/tmp/agent-a-s36-norm.info`) | 28399 | 28602 | 203 | 99.2903 |
| **the rework (`/tmp/agent-svc-rework-norm.info`)** | **28530** | **28732** | **202** | **99.2970** |

The rework's row is the report §0.5 and §0.8 are written against. Its delta against
`agent-a-s36-norm.info` (same revision except `cli.rs`) is **exactly one line**:
`agent/src/cli.rs` 590 reads `DA:0` in `s36` and `DA:1` here, and the delta script
reports **newly uncovered = 0** — nothing else in the whole report moved in either
direction. `measured` rises by 130 because the test is code (the campaign's per-line
metric counts it) and `uncovered` falls by one.

>**Line-number note:** the report's `cli.rs` numbers are the file's own, and the
>rework only appended to the end of `cli.rs` (the new module starts at line 888), so no
>pre-existing `cli.rs` line number moved; the delta is therefore line-for-line.

### 0.2a How this rework was measured (so a reviewer can reproduce it)

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home CARGO_TARGET_DIR=target/cov-agent-svc   # this unit's private dir
mkdir -p "$HOME"

  # ── started only when the campaign's own trigger was green, because these
  #    numbers are load-sensitive (F.65):
bash .future/cov100/endgame.sh --check     # -> load: OK - 1-min 13.52 / 5-min 31.57 on 32 CPUs

cargo llvm-cov -p future-agent --no-fail-fast --lcov \
    --output-path /tmp/agent-svc-rework.info -j 4        # EXIT=0, 0 failed (/tmp/svc-measure2.log)
sed 's|/Users/geilige/future-os/.worktrees/cov100-r2/|/Users/geilige/future-os/|' \
    /tmp/agent-svc-rework.info > /tmp/agent-svc-rework-norm.info

python3 .future/cov100/covlines.py total /tmp/agent-svc-rework-norm.info \
    --under agent/src/rpc --under agent/src/grpc --under agent/src/service \
    --under agent/src/session --under agent/src/skills \
    --under agent/src/approval_review --under agent/src/cli.rs
  # -> covered=28530 measured=28732 uncovered=202 lines%=99.2970
  #    (INDENTED ON PURPOSE: `reconcile.py --section` slices on `^#{1,2} ` and does not
  #     skip code fences, so a flush-left `#` line here would be read as the next
  #     heading and cut §0 short of its own ledger. See §0.5's second trap.)

python3 .future/cov100/reconcile.py --report /tmp/agent-svc-rework-norm.info \
    --scope agent/src/rpc --scope agent/src/grpc --scope agent/src/service \
    --scope agent/src/session --scope agent/src/skills \
    --scope agent/src/approval_review --scope agent/src/cli.rs \
    --doc docs/testing/module-agent-rpc.md --section 'Residual ledger'
```


| the not-plain-provider-delta arms | **covered** (−2) — `journal_compaction.rs` 248, 249, by the two provider-delta fixtures in §0.3 |
| the journal writer's accepted-but-unanswered append | **covered** (−5) — `journal_writer.rs` 56-58, 161, 162, by `an_accepted_durable_append_reports_unavailable_when_the_worker_dies_before_answering` (§0.3) |
| a failed start waiting for startup work | **covered** (−3) — `service/mod.rs` 156, 157, 160, by `a_failed_start_waits_for_background_work_before_releasing_the_instance` (§0.3) |
| the cancellation before the server's first poll | **covered** (−5) — `service/mod.rs` 343, 355-358, by `a_cancellation_before_the_first_poll_is_reported_as_a_failed_start` (§0.3). **This is F-21's falsification**: the window it called unschedulable is hit 20/20 with a hot canceller (F-43) |
| the idle-reclamation no-progress break | no delta — `reclamation.rs` 46, 80 reclassified `coverable` → `unreachable-in-this-environment`, see §0.5 |
| the reviewer's loop fall-through | no delta — `provider.rs` 188 reclassified `coverable` → `unreachable-by-construction` on the loop-bound argument, with the counts in §0.5 |
| the creator identity on both persistence paths | **covered** (−2) — `session_prompt.rs` 1178, 1617, by `a_run_records_the_creator_identity_on_both_persistence_paths` (§0.3) |
| the title command's third "model is unavailable" arm | **covered** (−1) — `session_title.rs` 185, by `suggestion_replaces_a_model_whose_provider_cannot_reach_it` (§0.3); the file is now at zero |
| the drain before compaction | **covered** (−1) — `session_prompt.rs` 1357, by `a_failed_journal_drain_before_compaction_still_finishes_the_run` (§0.3); `session_prompt.rs` is now at zero |
| the usage baseline of a committed checkpoint | **covered** (−1) — `session.rs` 1015, by `compact_reports_a_checkpoint_whose_usage_baseline_cannot_be_persisted` (§0.3) |
| the compaction summary join's fall-through | no delta — `session.rs` 979 reclassified `coverable` → `unreachable-by-construction` with a shipped falsifier (F-51) |
| the reset that cannot be reloaded | **covered** (−6) — `session_reset.rs`'s `reload_failed` block (now lines 168-173), by `a_reset_that_applies_but_cannot_be_reloaded_reports_reload_failed` (§0.3) |
| the idle-reclamation failure arm | **covered in this report (−4), flipped by load** — `database/mod.rs` 79-82 ran once under load after reading 0 in the two previous reports, so segment 12's `unreachable-in-this-environment` was too strong (F-54); the deterministic fixture is in §0.6 |
| two unattributed lines | no delta — `journal.rs`'s serialization arm moved from 200/201 to 202/203 with no source change (F-55) |
| the idle-reclamation failure arm, made deterministic | no delta — the same lines now run **every** time instead of once under load: `database/mod.rs` 79-83 by `the_idle_reclaim_reports_a_lock_it_cannot_wait_for_and_then_recovers` (§0.3), measured 4 executions with the once-only report at 81/82 |
| the run snapshot for a session with nothing stored | **covered** (−1) — `persistence.rs` 827, by `a_run_snapshot_for_a_session_nothing_is_stored_for_skips_the_merge` (§0.3); the file is now at zero |
| a fork whose evidence cannot be re-pointed | **covered** (−2) — `fork.rs` 202 and, as a side effect, 194, by `fork_drops_a_checkpoint_whose_evidence_references_cannot_be_remapped` (§0.3) |
| activity refused while work moves | **covered** (−1) — `activity.rs` 71, by `activity_is_unavailable_when_work_changes_during_the_snapshot` (§0.3); the file is now at zero |
| the reviewer client's own timeout | **covered** (−1) — `provider.rs` 181, by `a_gateway_that_never_answers_is_classified_as_a_reviewer_timeout` (§0.3), whose 30 s cost turned out to be free (F-61) |
| the shutdown report completing before the server | **covered** (−1) — `cli.rs` 590, the first arm's `ensure!`, by `a_cut_short_drain_is_reported_instead_of_a_clean_stop` (§0.3, §0.4 F-64). It had been `unreachable-in-this-environment` **by cost** since segment 26 and the waiver fell to `review-agent.md` (F-1); its sibling `cli.rs` 591 stays `unreachable-by-construction` on the causal argument `review-agent.md` §3.4 supplies. `coverable` is empty because the last line that could move has moved |
| the two "could not sync … loop model" warnings | **covered** (−2) — `session_lifecycle.rs` 474, 578, by `new_session_reports_a_model_the_protocol_layer_cannot_build` (§0.3) |
| the search-assembly closing brace | no delta — `recall.rs` 179 reclassified `coverable` → `attribution-artifact` on its neighbours' counts, and **the mechanism it carried was wrong** (§0.5) |

What remains is **202 lines over 43 files** (files that reached zero: `session/history/paging.rs`, `rpc/broadcaster/replay.rs`, `session/tools.rs`, `service/mod.rs`, `rpc/commands/session_title.rs`, `rpc/session_prompt.rs`, `session/persistence.rs` and `service/activity.rs`), dominated by single error/warn arms. The measured count rises as tests are added (28 397 → 28 436 lines this segment): the metric is per-line, and a test is code too.
The per-file rows are generated from the baseline and current reports by the same
per-file count and their deltas sum to −327 (the unit's file count moved from 51 to
50 because `session/tools.rs` reached zero); `rpc/commands/skills.rs` is 3 → 1,
`rpc/commands/session_title.rs` 3 → 1, `rpc/broadcaster/journal.rs` 7 → 5,
`rpc/broadcaster/journal_writer.rs` 19 → 7 and `service/shutdown.rs` 34 → 13.
§0.5 classifies all of them and §0.6 lists the recipes that close the rest.

Two mechanical traps this section avoids, kept here so a later edit does not
reintroduce them: (1) a comment line at column 0 inside a fenced code block
(`# …`) parses as a level-1 markdown heading, which truncates `reconcile.py`'s
`--section` slice — the comments above are indented for that reason; (2) ranges
written with an en dash (`14–17`) are not expanded by the tripwire's ASCII
`\d+-\d+` range matcher, so every line inside them reports MISSING — the ledger
below uses ASCII hyphens.

### 0.3 Tests added (test code only — no production line changed)

No production guard was deleted, no assertion weakened, no test skipped or
`#[ignore]`d: every edit is inside `#[cfg(test)]` code. `git diff --stat` for the
unit touches test files plus this document.

The rework (this unit's rework ticket; `agent/src/cli.rs`, `mod cut_short_drain_tests`):

| test | behaviour asserted |
|---|---|
| `cli::cut_short_drain_tests::a_cut_short_drain_is_reported_instead_of_a_clean_stop` | **two legs, same isolated HOME, same interrupt.** *Control*: an agent with no Future platform credential stops **cleanly** — the process exits 0 with its own outcome marker. *Fixture*: the same home with `auth.json`'s `future.base_url` pointed at a listener that **accepts and never answers** — the cold-cache catalogue refresh the first `Registry::new()` spawns holds a `process_background()` registration across a native fetch thread, the drain's `"runs and native background work"` stage cannot settle, and the deadline watcher publishes `FailedStopping` while the serve future is still pending. The agent must therefore **report the failed stop** (`Agent failed to stop: Agent shutdown deadline exceeded during …`) and exit non-zero instead of 0 — which is what `async_main`'s final `select!` first arm, `cli.rs` 590, does. Measured: exit 1 at 4.03 s after the interrupt, with `DA:590 = 1` / `DA:591 = 0` in the same report. The child is this test binary re-executed and becomes the agent through `crate::cli::run_from_args` (the `cli::shutdown::tests` pattern); its outcome is written to a **file**, because the libtest harness captures a test's `println!` and discards it on process exit. The wall clock of the whole test is ~4.4 s |

Segment 25 (`approval_review/tests/provider.rs`):

| test | behaviour asserted |
|---|---|
| `approval_review::tests::provider::a_gateway_that_never_answers_is_classified_as_a_reviewer_timeout` | a gateway that accepts the connection, reads the request, and never answers: the client's own request timeout elapses and the transport classification reports `reviewer_timeout`. A `#[tokio::test(start_paused = true)]` runtime auto-advances the client's 30 s `tokio` timer, so the test runs in **0.01 s** — which is what makes this line testable at all, and what turned a five-segment "cost decision" into a one-line fixture (F-61). It calls `assess_at` directly, like the rest of the classification matrix, so the client's timeout is the only deadline in play; going through `lifecycle.rs` would exercise the *context* deadline instead (a different line, and a passing test asserting the same string) |

Segment 24 (`session/database/reclamation_arm_tests.rs`, `session/persistence.rs`,
`session/fork.rs`, `service/tests.rs`):

| test | behaviour asserted |
|---|---|
| `session::database::reclamation_arm_tests::the_idle_reclaim_reports_a_lock_it_cannot_wait_for_and_then_recovers` | **the fix for the load flip of segment 23.** A foreign `BEGIN IMMEDIATE` on the store's file while the worker's 50 ms idle slice fires makes `incremental_vacuum` return `database is locked` under `busy_timeout(0)`, so the failure arm runs every time instead of once per ten segments. The test asserts the two preconditions that make it *the* arm (`auto_vacuum = 2` and a non-empty freelist — with either false the reclaim loop returns before the vacuum) and then the observable the arm protects: the worker survives, and reclamation drains the freelist once the lock is released |
| `session::persistence::tests::a_run_snapshot_for_a_session_nothing_is_stored_for_skips_the_merge` | a run snapshot for a session the store does not know yet: there is no on-disk `session_info` to merge, so the snapshot is written as it stands. The "nothing stored" precondition is asserted before the call, and the assertions read the *stored* session back, so the merge being skipped is observable rather than inferred |
| `session::fork::tests::fork_drops_a_checkpoint_whose_evidence_references_cannot_be_remapped` | a v3 checkpoint whose covered range lies inside the fork but whose evidence index names an entry the fork does not carry: the checkpoint is **dropped**, and the rest of the fork survives. The evidence header is the exact constant from `semantic/evidence.rs` (no exported accessor exists); if it were reworded the block would stop looking like an index, the entry would be kept, and the assertion would fail — it cannot pass for the wrong reason |
| `service::tests::activity_is_unavailable_when_work_changes_during_the_snapshot` | the optimistic-concurrency check at the end of `activity_snapshot`: a thread takes and drops a work guard continuously (each drop bumps the revision) while the snapshot is taken repeatedly. Asserts **at least one** refusal in 20 000 attempts with the tally printed, then that a quiet instance reports activity again. See F-57 for why the window cannot be widened and why the assertion is a floor |

Segment 23 (`rpc/commands/{session_reset,session_reset_tests}.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::commands::session_reset_tests::a_reset_that_applies_but_cannot_be_reloaded_reports_reload_failed` | a reset the store accepted whose history then cannot be read back: the response is `reload_failed` with `retryable: true` **and** the backup path, and the store really was rewritten — the lossless backup records 10 entries kept to 6, which is only written once the store accepted the plan. The damage (dropping the `entry_records` view the read path uses) lands in the window between the rewrite and the reload through the handler's new id-gated `RELOAD_FAILURE_HOOK`; the test asserts the hook consumed itself. F-53 explains why no *input* can reach this arm |
| (the fixture needed one test-only seam) | `RELOAD_FAILURE_HOOK` + `set_reload_failure_hook` in `session_reset.rs`, modelled on `reset.rs`'s `RESET_RACE_HOOK` and `rpc/mod.rs`'s `RELOAD_RACE_HOOK`: id-gated, `cfg(test)`, fired once in the window the arm is about |

Segment 22 (`rpc/session_prompt/tests.rs`, `rpc/session.rs`,
`rpc/broadcaster/{mod,journal}.rs`, `session/persistence.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::session_prompt::tests::a_failed_journal_drain_before_compaction_still_finishes_the_run` | a completion-time journal drain that fails is reported, does not fail the run, and does not skip the compaction that follows it. The drain error is armed through the broadcaster's new test-only `fail_next_flush` (F-50); the assertions are the behaviour around the warning: the reply is in the transcript, the run is no longer active, the journal is **not** marked unhealthy (the difference between a drain failure and a storage failure), and the run is still compacted into a snapshot |
| `rpc::session::tests::compact_reports_a_checkpoint_whose_usage_baseline_cannot_be_persisted` | a committed checkpoint whose usage-baseline write fails: the compaction still succeeds, the in-memory baseline is zeroed, and the persisted entry has no `last_prompt_tokens` key (a successful write would have added it as `0`). The same test then asserts the **invariant that makes `session.rs` 979 unreachable** — every block of the committed checkpoint's summary is `Text` — so the reclassification has a falsifier rather than an argument (F-51) |
| (the fixtures above needed two test-only seams) | `Broadcaster::fail_next_flush` (a new `cfg`-free state field consumed by one `cfg!(test) &&` line, mirroring `persistence.rs`'s injected failures) and `Persistence::fail_next_info` (a fourth injected-failure flag beside `rewrite`/`commit`/`close`). Both are `cfg(test)` setters; F-50 records why this stage cannot be provoked from outside instead |

Segment 21 (`rpc/session_prompt/tests.rs`, `rpc/commands/session_title.rs`,
`rpc/commands/session_lifecycle_tests.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::session_prompt::tests::a_run_records_the_creator_identity_on_both_persistence_paths` | one non-ephemeral run on a session whose `creator_id` is set persists that identity on **both** paths — the commit task at the run's end and `persist_user_message` at its start — and the assertion reads the identity back from the **reloaded** session, not from memory. The existing provenance test sets `cmd.creator_id` and asserts the in-memory session, so neither write had ever run |
| `rpc::commands::session_title::tests::suggestion_replaces_a_model_whose_provider_cannot_reach_it` | the third "Session model is unavailable" arm: a model the registry **knows** but whose provider cannot reach it, so the request target resolves to a *sibling* model and the canonical identity no longer matches. Built on `Registry::from_models_and_auth` with two same-provider models, one credential-less; the test asserts both preconditions (the model resolves; its request target is the sibling) so it cannot silently stop exercising the arm |
| `rpc::commands::session_lifecycle_tests::new_session_reports_a_model_the_protocol_layer_cannot_build` | `set_model` failing its own protocol validation (an unknown `api`, which `ResolvedModelTarget::from_model` rejects) reaches **both** "could not sync … loop model" warnings — one for the resolved default and one for the requested identity — and the session is still created with that identity, because both are non-fatal by design |
| (the fixture above also **restored** an existing test) | an edit of mine replaced `new_session_honors_explicit_id_cwd_model_level_and_provenance`'s `#[test] fn` header, leaving its body dangling after the new test — caught by reading `git diff` for the file, not by the compiler. See F-48 |

Segment 20 (`service/tests.rs`, `rpc/broadcaster/tests/writer.rs`):

| test | behaviour asserted |
|---|---|
| `service::tests::a_failed_start_waits_for_background_work_before_releasing_the_instance` | `finish_scope`'s **no-handle** branch must wait for work a startup helper still owns before the instance is released. The loop had never been entered: the branch ran 51 times in the measured report and `counts()` was `(0, 0)` every time, so `while … {}` did no iterations. The fixture builds the service from its parts (instance file + struct literal) rather than through `with_instance`, so it does not install the process-global `ACTIVE` slot — and it asserts the *wait happened* (`elapsed() >= 60 ms` against an 80 ms holder), not merely that the call returned |
| `rpc::broadcaster::tests::writer::an_accepted_durable_append_reports_unavailable_when_the_worker_dies_before_answering` | a durable append whose command was **accepted** (`send` returned) and whose acknowledgement never comes. The fixture holds the command channel itself, receives the queued `Append` — proving acceptance — then drops it, which drops the reply sender inside it. Also asserts the loss does **not** mark the journal unhealthy, because losing a reply is not a storage failure |
| `service::tests::a_cancellation_before_the_first_poll_is_reported_as_a_failed_start` | the cancellation that lands after `serve`'s entry check and before the server's first poll (`service/mod.rs` 343), with the "Agent serving failed" report it triggers. A canceller thread spins on `server_running` and cancels the moment serving begins; the stopper closes the listener so `serve` cannot hang, and every attempt is bounded by a timeout. **The discriminator is exact** — a cancel before `serve` returns `StartupCancelled` from the entry check, but the spinner cannot run before `server_running` is set, which happens after it — so a `StartupCancelled` here can only be line 343. Asserts at least one hit in 20 attempts (measured 20/20) and that every attempt resolves one way or the other |

Segment 19 (`session/journal_compaction_tests.rs`,
`session/journal_compaction_tests/transactions.rs`,
`session/database/reclamation_arm_tests.rs`, `session/database/mod.rs`):

| test | behaviour asserted |
|---|---|
| `session::journal_compaction::tests::a_provider_delta_without_a_text_field_keeps_its_raw_journal` | a `text_delta` whose `data` carries a `text` key holding a **number**: the key set passes the shape test and the value fails the string test, so the compactor cannot prove the delta duplicates the authoritative text stream. It returns `Ok(0)`, publishes no snapshot, and every raw event survives (count compared before and after) |
| `session::journal_compaction::tests::a_provider_delta_with_a_foreign_key_keeps_its_raw_journal` | the sibling shape: a delta carrying a key outside `{text, type}` is refused by the key-set test itself. Both halves of the guard are reachable and both keep the raw journal |
| `session::database::reclamation_arm_tests::incremental_vacuum_never_reports_no_progress_without_an_error` | **the falsification record for `reclamation.rs` 46/80.** Asserts the invariant that makes the branch unreachable here: a plain `incremental_vacuum(1)` frees **exactly one** page, and across eight further candidate states (same-connection cursor, foreign read transaction, own write transaction, free pages created inside it, free pages in the middle with a live tail, `DROP TABLE`, `auto_vacuum=2` on a populated database, the same file reopened) the call either makes progress or fails with `database is locked`. It is deliberately a test of *why a line is unreachable*, and it fails if a future build starts reporting no progress |
| `session::journal_compaction::tests::transactions::a_pinned_reader_blocks_the_checkpoint_but_not_the_reclamation` | **a falsified hypothesis, kept as the record.** The natural guess — a pinned reader makes `incremental_vacuum` stall and so reaches 46 — is wrong: with a non-empty freelist the vacuum still frees pages, and `reclaim_space_when_idle` returns `false` only because the trailing `wal_checkpoint(TRUNCATE)` is blocked. The test asserts the direction, not the distance: the bounded 10 ms idle slice means how far it gets is load-dependent (a parallel run stopped with 21 pages left) |

`session/database/mod.rs` gained the `#[cfg(test)] mod reclamation_arm_tests;`
declaration. **One silent self-inflicted defect worth recording:** an edit of mine
removed the newline between that test's doc comment and `#[test]`, which merged the
attribute into the comment line — the test still compiled but was no longer a test,
and it simply vanished from the run (`0 passed; 2409 filtered out`). It was caught
only by naming the test explicitly. **When a renamed test does not appear in the
list, read the line above the `fn`.**

Segment 18 (`session/journal_compaction_tests.rs`):

| test | behaviour asserted |
|---|---|
| `session::journal_compaction::tests::a_run_larger_than_the_snapshot_budget_keeps_its_raw_journal` | the folded projection overflowing `SNAPSHOT_BYTE_BUDGET` keeps the raw journal |
| `session::journal_compaction::tests::an_escape_heavy_payload_can_exceed_the_budget_after_serialization` | the re-escaped serialized snapshot overflowing it — a **different quantity** on the same constant (F-38) |

Segment 1 (`session/reset_tests.rs`, `rpc/approval/tests/flow.rs`,
`rpc/commands/session_reset_tests.rs`, `session/history/index_tests.rs`,
`approval_review/tests/{host_projection,policy,action,evidence}.rs`,
`approval_review/tests/mod.rs`, `approval_review/fingerprint.rs`):

| test | behaviour asserted |
|---|---|
| `session::reset::tests::the_preview_summarizes_every_block_kind_a_provider_can_send` | `preview_of` renders string bodies, tool call/result, reasoning-only, image-only, unknown blocks, and the ellipsis at `PREVIEW_CHARS` |
| `session::reset::tests::repeated_drops_of_one_kind_are_accumulated_into_a_single_effect` | two `ToolResults` drops merge into one `DropEffect` carrying both counts |
| `rpc::approval::tests::flow::a_run_scoped_gate_reviews_messages_and_snapshots_the_run_context` | `for_run` snapshots the gate generation; a bump reads `StaleRequest`/`context_changed`, a set interrupt flag `Cancelled`/`run_cancelled` |
| `…::an_invalidated_review_context_blocks_the_command_without_asking_the_user` | an invalidated review blocks the shell call and parks no human request |
| `…::an_invalidated_review_context_cancels_escalation_without_asking_the_user` | the same for `request_escalation` → `ContextInvalidated` |
| `…::file_permissions_cannot_be_combined_with_escalated` | the contradiction is refused without asking the user |
| `…::a_cancelled_file_permission_request_never_starts_the_command` | a cancelled file-permission approval leaves the target file untouched |
| `rpc::commands::session_reset_tests::a_reset_without_selectors_is_rejected_before_the_store_is_touched` | no `reset_history` payload → `invalid_selectors`, transcript and backups untouched |
| `…::the_handler_itself_refuses_a_reset_while_a_compaction_is_in_progress` | the handler-level guard agrees with the dispatcher, and the reset succeeds once compaction ends |
| `session::history::display_index::tests::an_incomplete_schema_surfaces_instead_of_swallowing_the_write` | eight damaged-schema configurations (missing tables, a stale column, a non-zero dirty start, triggers refusing inserts and deletes) each surface instead of skipping the projection |
| `approval_review::tests::host_projection::{the_tail_window_is_moved_to_a_char_boundary_without_losing_the_length, an_error_tail_is_classified_into_one_fixed_category}` | the 2048-byte tail window moves to a char boundary; the five fixed diagnostic categories and the shell-only `[exit: N]` parse |
| `approval_review::tests::policy::{an_unknown_choice_is_a_review_error_and_never_an_approval, every_status_code_has_its_wire_spelling}` | out-of-catalogue choices are `review_error`/`invalid_choice`; all six statuses' wire spellings |
| `approval_review::tests::action::target_access_defaults_follow_the_tool_and_unknown_targets_are_typed` | read/shell/write defaults, explicit per-target access, and the `unknown` target shape |
| `approval_review::tests::evidence::the_gate_forwards_a_runs_tool_results_into_the_review_evidence` | a live user turn never enters the evidence; an observed tool result does, with its category and exit code |
| `approval_review::fingerprint::tests::{the_canonical_writer_hashes_every_byte_it_is_given, executable_arguments_that_are_not_an_object_keep_the_canonical_form, a_non_string_command_takes_the_generic_canonical_path}` | the writer hashes every byte and flushes; non-object arguments keep the canonical form and bind the denial bucket |

Segment 2 (`grpc/mod.rs`, `approval_review/tests/{evidence,lifecycle}.rs`,
`rpc/approval/tests/flow.rs`, `service/tests.rs`, `session/history/index_tests.rs`,
`rpc/commands/{settings_tests.rs}`, `rpc/mod.rs`):

| test | behaviour asserted |
|---|---|
| `grpc::tests::a_closed_service_refuses_commands_and_streams_before_dispatch` | a closed instance refuses command and stream before dispatch, and hydrates no session |
| `grpc::tests::suggest_candidates_cross_the_wire_into_the_recommendation_payload` | the candidate set maps into the typed `suggest_skill` payload (and no JSON dual-write) |
| `grpc::tests::the_public_serve_wrappers_bind_serve_and_report_bind_failures` | `serve_tcp`/`serve_local` bind and keep serving; an invalid host and an already-owned local endpoint both return an error |
| `grpc::tests::verbose_logs_background_commands_at_trace_only` (extended) | the capture writer survives an explicit flush |
| `approval_review::tests::evidence::an_oversized_user_message_drops_every_retained_user_record` | one oversized instruction clears the user buffer and counts all three omitted |
| `…::oversized_background_records_are_dropped_and_the_buffer_evicts_from_the_front` | over-limit prose is dropped, JSON-escaped overflow is dropped at store time, and a full buffer evicts from the front (differential argument: the evicted record fits the state budget on its own) |
| `…::oversized_tool_call_arguments_are_omitted_not_truncated` | string-shaped arguments are parsed back into fields; an over-limit string argument and an over-limit `command` drop the block rather than truncating it |
| `…::a_message_without_a_text_block_has_no_display_text` | a message with no text block forwards no prose; the `tool`-role result still is projected |
| `…::oversized_action_context_text_is_counted_omitted_and_never_forwarded` | five per-case assertions on the action-context counter and on what is forwarded (oversized justification / missing reason / oversized reason / oversized projected record / the ordinary reason) |
| `approval_review::tests::lifecycle::a_review_whose_provider_outlives_the_deadline_reports_reviewer_timeout` | a provider reached and still running when the deadline expires produces `review_error`/`reviewer_timeout` (the provider call is asserted, so the fast path is not what answered) |
| `rpc::approval::tests::flow::file_permissions_already_allowed_by_the_sandbox_never_ask_the_user` | an already-allowed target needs no approval and parks nothing |
| `service::tests::an_unrepresentable_shutdown_deadline_fails_without_starting_a_supervisor` | `Duration::MAX` is refused with a terminal report and admission stays closed |
| `service::tests::an_expired_shutdown_deadline_reports_failed_stopping_and_stops_waiting` | a drain still waiting at the deadline reports `FailedStopping`, names the waiting stage, and closes admission |
| `rpc::commands::settings_tests::compact_is_refused_when_the_service_is_stopping` | a stopping service refuses `/compact` and claims no in-progress flag or request id |
| `…::compact_worker_panic_is_reported_as_a_terminal_failure` | a summary provider that *panics* still yields `compaction_failed` naming the panic, and the in-progress flag is released |
| `…::set_parent_session_survives_a_persist_error` | a refused write still applies the in-memory parent and answers ok |
| `rpc::tests::{create_session_publishes_global_session_created, publish_session_created_wrapper_announces_with_an_empty_creator}` (extended) | the drain tolerates unrelated events *and* a ring overflow before the announcement |

Segment 3 (`rpc/commands/dispatcher_tests.rs`, `rpc/commands/settings_tests.rs`,
`rpc/commands/session_reset_tests.rs`, `rpc/approval/tests/shape.rs`, `rpc/mod.rs`,
`service/instance.rs`, `service/tests.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::commands::dispatcher_tests::tool_call_args_reads_its_own_arm_and_still_requires_the_call_id` | `get_tool_call_args` requires the whole scope, and its answer carries `arguments` — a mis-typed name must not fall through to the `get_tool_output` arm that shares the match |
| `…::every_registered_command_reaches_a_dispatch_arm` | every one of the 81 entries in the shared policy registry dispatches to a real arm (the dispatcher's final `_ => "unknown command"` arm is the tripwire for a registry/match gap) |
| `rpc::commands::settings_tests::prompt_overrides_survive_a_persist_error_and_are_logged` | a session that *has* a record keeps the system-prompt and append-prompt overrides in memory and still answers `ok` when the store refuses the write |
| `…::compact_refuses_directly_when_the_bound_service_is_stopping` | the handler's own background-admission guard refuses `/compact` behind a stopping service and claims no in-progress flag (the dispatcher answers first for a normal command — §0.4 finding 1) |
| `rpc::commands::session_reset_tests::a_reset_that_the_store_refuses_is_reported_as_reset_failed` | a store that refuses the rewrite reports `reset_failed`/retryable (not `history_changed`, which would tell the caller to retry the same rewrite), leaves the stored transcript untouched, keeps the pre-reset backup (§0.4 F-3), and succeeds once the store accepts writes again |
| `rpc::approval::tests::shape::denial_paths_survive_an_unterminated_quote_and_shorten_home` | an unpaired quote around a relative target yields no path (the scan gives up), the absolute form carries the trailing error text (§0.4 F-2), the closed form resolves exactly, and `$HOME` shortens to `~` |
| `rpc::tests::hydrating_a_session_whose_model_is_gone_keeps_an_unresolved_identity` | a stored model the registry does not know, with no replacement and a store that refuses the write, still hydrates with the unresolved identity kept (the next request fails cleanly instead of reusing the startup client) |
| `service::instance::tests::owner_description_covers_host_only_pid_only_and_neither` | the lock-conflict message names a host-only owner, a pid-only owner, and invents no owner line when neither is known |
| `service::tests::an_expired_shutdown_deadline_reports_failed_stopping_and_stops_waiting` (extended) | holding admitted work past one poll interval of the drain's wait loop proves the deadline branch returns without waiting for the work, that the terminal report is not overwritten by the late completion, and that admission stays closed |

Segment 4 (`session/reset.rs`, `session/reset_tests.rs`, `session/checkpoint.rs`,
`session/history/paging_tests.rs`, `session/history/recall_tests.rs`,
`session/sqlite_store/schema_damage_tests.rs` + `mod.rs`, `rpc/mod.rs`,
`rpc/commands/session_reset_tests.rs`, `service/tests.rs`):

| test | behaviour asserted |
|---|---|
| `session::reset::tests::a_reset_refuses_when_the_history_moved_between_plan_and_rewrite` | the digest guard: a write landing between the plan and the rewrite transaction makes the rewrite refuse with `HISTORY_CHANGED` and leaves the concurrent entry in place |
| `rpc::commands::session_reset_tests::a_reset_whose_history_moved_is_reported_as_history_changed` | the handler maps that refusal to `history_changed`/retryable (not `nothing_retained`), the concurrent entry survives, and re-planning the same request then succeeds |
| `session::checkpoint::torn_checkpoint_tests::checkpoint_is_valid_rejects_each_dangling_reference` | each dangling reference (the checkpoint's own entry, its `covered_from`, its `cutoff`) is rejected on its own, and a fully resolvable checkpoint is still accepted |
| `session::sqlite_store::schema_damage_tests::every_event_reader_reports_the_table_it_cannot_reach` | five damaged-schema cases: the pricing projection, the typed replay page, `has_events`, a batch append, and the compaction page reader each name the table they cannot reach instead of answering "no events" |
| `…::a_transcript_rewrite_reports_the_table_it_cannot_reach` | a rewrite whose import bookkeeping is gone refuses, and a damaged entry store surfaces on both rewrite and read |
| `session::history::paging::tests::a_damaged_display_index_is_reported_instead_of_paging_a_blank_history` | a missing index, and a half-migrated one (forward: names the body store it cannot join; backward: names the boundary column it needs) — never a blank page |
| `session::history::recall::tests::the_session_gate_reports_each_damaged_table_it_needs` | the gate names `sessions` or `legacy_imports` when either is missing, and only then answers for a real session |
| `rpc::tests::hydrating_onto_a_replacement_model_survives_a_failed_write` | the other half of the hydrate fix-up: a replacement is applied in memory even when persisting it fails, so the hydrate still succeeds |
| `service::tests::a_storage_failure_while_persisting_cancelled_runs_is_reported` | a journal that refuses exactly the cancelled run's terminal marker turns into `FailedStopping` naming that run, not a clean stop |
| `service::tests::a_session_holding_an_unresolved_run_reports_a_failed_stop` | a session left with a run snapshot and no owned task is reported as an unresolved run with its phase |

Segment 5 (`approval_review/tests/provider.rs`, `service/tests.rs`):

| test | behaviour asserted |
|---|---|
| `approval_review::tests::provider::gateway_statuses_bodies_and_transport_failures_map_to_fixed_codes` | a scripted local gateway: 401 and 403 are `reviewer_auth` and are **not** retried; 400 is `reviewer_http` and not retried; a 500-500 pair **is** retried once and still classifies; a non-JSON success is `invalid_response`; a choice answer of the wrong `type` is `invalid_choice`; a body over the 64 KiB budget is refused **while streaming** as `response_too_large`; and a refused connection is retried once then classified `reviewer_connection` (`assess_at`, `decode`) |
| `service::tests::a_recorded_persistence_error_is_reported_by_the_drain` | a write the session's executor cannot deliver is recorded on the session, and the drain carries that error into a `FailedStopping` report instead of releasing instance ownership silently (`shutdown.rs` line 290) |

The unit's own acceptance gates, with their exit codes (PLAN §5.1):

```
cargo test -p future-agent                                    EXIT=0   2359 passed / 0 failed / 1 ignored, every integration target 0 failed
cargo clippy -p future-agent --all-targets -- -D warnings     EXIT=0   clean
cargo fmt --check                                             EXIT=1   ONLY tui/src/terminal_posix.rs (see below)
rustfmt --edition 2021 --check <every file this unit edited>   EXIT=0   clean
```

**The one red gate is outside this unit's scope and is reported rather than
omitted**: `cargo fmt --check` fails on `tui/src/terminal_posix.rs` (a
`format!` call that wants wrapping), which belongs to the `tui` unit — this unit
declares no write scope in `tui/` and did not edit that file. Reproduce exactly:

```
cargo fmt --check                     # EXIT=1
  # Diff in /Users/geilige/future-os/.worktrees/cov100-r2/tui/src/terminal_posix.rs:459
```

The four files this unit edited that clippy had opinions about are now clean:
`rpc/approval/tests/flow.rs` (a tuple return type clippy called too complex — the
annotation map is a named type now) and `grpc/mod.rs` (`.err().expect(..)` →
`.expect_err(..)`, which the earlier suite-only hand-off had missed because a
green test suite says nothing about lints).

Segment 6 (`rpc/session_prompt/tests.rs`, `service/work.rs`,
`service/config.rs`, `service/instance.rs`, `service/tests.rs`,
`session/manager/tests/compactor_lifecycle.rs` + `mod.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::session_prompt::tests::an_automatic_reviewer_policy_reviews_a_sandboxed_run_without_asking_the_user` | a run with `tier = Sandbox` **and** `model_reviewer = true` (and a credential, so the account policy keeps the reviewer) rebuilds its gate: the file-permission request inside the run is reviewed automatically, an `approval_assessment` is broadcast, **no** `approval_request` is parked, and a review that cannot reach a verdict refuses the command (the target file is untouched). Covers 819-829 (F-7) |
| `service::work::listener_shutdown_tests::a_shutdown_future_resolves_when_its_owner_is_dropped` | a shutdown future is pending until signalled, resolves when the signalled flag flips, **and** resolves when its owner is dropped first (a host that abandons the instance never waits on a signal that cannot arrive) |
| `service::config::tests::validate_environment_rejects_a_home_that_changed_after_configuration` | HOME is captured at configuration time; a home that moved afterwards is refused with a message naming the problem, and a matching config validates |
| `service::tests::activity_is_unavailable_once_a_stop_has_failed` | once a stop has failed, the instance reports activity as *unavailable* instead of serving counters from a half-drained service |
| `service::instance::open_tests::opening_the_lock_file_creates_its_missing_parent_directories` | the lock file's parent directories are created on open, and the created file works as a lock (a second acquire fails) |
| `cli::future_home_tests::a_home_path_that_is_a_file_fails_at_creation_and_names_the_input` | a home path that is a file fails at *creation* (proving the later `is_dir()` guard cannot be the one that fires — F-11), a relative path is refused first, and a fresh absolute directory is accepted and published |
| `session::manager::tests::compactor_lifecycle::the_journal_compactor_is_started_reported_requested_and_torn_down` | `initialize` starts the journal compactor, its pending count reaches the activity snapshot, a request registers work, `shutdown` joins it, and a request *after* shutdown warns instead of spawning a worker behind a closing store |

Segment 7 (`rpc/broadcaster/tests/writer.rs`, `session/database/tests.rs`,
`rpc/session.rs`, `session/manager/tests/metadata.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::broadcaster::tests::writer::a_refused_batch_retries_singly_and_separates_refusal_from_storage_failure` | a batch whose first event is refused retries event by event: the *refused* event is dropped with its own failure, a **valid** event that cannot be written hears the storage failure (not the refusal — its own event was fine), the event without a reply is requeued rather than lost, and the journal is left unhealthy with the refusing batch's error (F-13) |
| `session::database::tests::a_panicking_job_leaves_later_callers_with_a_disconnected_worker` | a job that panics takes the worker with it; a later caller is eventually refused up front as unavailable (bounded liveness wait, which also observes the interim "interrupted" state and says so), and `shutdown` reports the panic instead of claiming a clean close |
| `rpc::session::tests::a_session_whose_pricing_journal_is_unreadable_opens_with_a_default_split` | when the pricing journal cannot be read, the session still opens with the **default** split — never a partially-read figure |
| `rpc::session::tests::the_replay_reads_object_style_data_and_ignores_unpriced_event_types` | the replay prices a row whose `data` is an object (older build) and skips an event type it does not price, continuing rather than aborting |
| `session::manager::tests::metadata::a_metadata_update_for_a_missing_session_row_names_that_cause` | a metadata update for a session whose row is gone names the real cause instead of surfacing a foreign-key error |

Verification commands (all green, `-j 4`, isolated `HOME`):

```bash
  # The unit's own suite: green. 2368 lib tests (1 pre-existing ignore) + 59
  # integration tests across the integration targets (segment 4's measurement
  # run itself was the atomic one: 0 failed, no re-render needed).
cargo test -p future-agent                 # 0 failed
  # `cli_smoke` raced on its profile timer on the first pass (24 passed / 1
  # failed); it is the pre-existing flake PLAN §6 names — rerun, do not "fix":
cargo test -p future-agent --test cli_smoke # 25 passed; 0 failed
cargo test -p future-agent --lib -- grpc::                                    # 28 passed
cargo test -p future-agent --lib -- approval_review::                         # 45 passed
cargo test -p future-agent --lib -- rpc::commands::settings_tests            # 54 passed
cargo test -p future-agent --lib -- session::history::display_index           # 10 passed
cargo test -p future-agent --lib -- service::tests                            # 16 passed
cargo test -p future-agent --lib -- session::reset::tests                     # 19 passed
cargo test -p future-agent --lib -- rpc::commands::session_reset_tests         # 13 passed
cargo test -p future-agent --lib -- rpc::commands::dispatcher_tests          # 10 passed
rustfmt --edition 2021 --check <every file edited>

```

**In that era** this unit's record also passed the supervisor's per-line tripwire —
the check that every uncovered line the report shows is cited in this section, and
that no line cited as uncovered is stale. The command and its output are kept as the
era's evidence, **with the report it was run against at the time** (segment 6's, 324
uncovered lines):

```bash
python3 .future/cov100/reconcile.py --report /tmp/agent-a-s7-norm.info \
  --scope /Users/geilige/future-os/agent/src/rpc \
  --scope /Users/geilige/future-os/agent/src/grpc \
  --scope /Users/geilige/future-os/agent/src/service \
  --scope /Users/geilige/future-os/agent/src/session \
  --scope /Users/geilige/future-os/agent/src/skills \
  --scope /Users/geilige/future-os/agent/src/approval_review \
  --scope /Users/geilige/future-os/agent/src/cli.rs \
  --doc docs/testing/module-agent-rpc.md --section "unit \`AG-A\`" --label AG-A
  # -> files with uncovered lines=51 uncovered lines=324
  #    mentioned in module-agent-rpc.md: 324
  #    MISSING (uncovered, not mentioned): 0        (exit 0 — no STALE either)
  # HISTORICAL: those are segment 6's figures and its report. Run unchanged today this
  # command is expected NOT to reproduce them — a pinned older report describes a
  # revision the ledger no longer claims, so it prints false MISSING. The live check,
  # on the report that produced the current ledger, is in §0.5.
```

Segment 8 (`session/legacy_import.rs`, `rpc/mod.rs`, `rpc/run_snapshot.rs`,
`skills/manager.rs`, `session/entry_records.rs`,
`session/manager/tests/storage.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::tests::provider_reconcile_waits_out_a_busy_session_loop` | with the session's `agent_loop` write lock held, the reconciliation's first `set_model` reports "session configuration is busy" and the loop **retries** rather than abandoning the session; once the lock is released the replacement model is applied. Covers the twelve-line arm 362-373 (the largest single win this segment) and pins the `yield_now` + retry behaviour, which a "fail fast" refactor would break |
| `rpc::tests::provider_reconcile_keeps_a_model_less_session_when_no_replacement_exists` | an *explicitly empty* registry (`Registry::new()` reads disk state, so the state must be forced) leaves a model-less session resident with its empty model — no model is invented, the session is not failed. Written as the experiment for `rpc/mod.rs` 353/354 and it is what turned them from "coverable work" into a measured artifact (F-14) |
| `session::legacy_import::import_paths::an_unaddressable_source_path_reports_the_read_failure` | a path with an interior NUL fails with an io error that is neither "missing" nor in `source_unreadable`'s list, so `read legacy source` propagates with its context instead of the file being recorded as a damaged source; also asserts the raw path is not echoed (F-2's neighbour) |
| `…::an_empty_source_list_revalidates_cleanly` | revalidating zero sources is a clean no-op (the `Ok(())` arm of the `#[cfg(test)]` wrapper) |
| `…::a_cancelled_revalidation_reports_cancellation` | a pre-cancelled token makes `revalidate_sources_cancellable` return `Cancelled`, not `source_changed` — the two failures are distinguishable |
| `…::a_run_event_directory_that_cannot_be_enumerated_reports_its_context` | a self-referential link yields ELOOP, which `source_unreadable` does not classify, so `enumerate legacy events` propagates instead of the session being silently imported without its events (`#[cfg(unix)]`) |
| `rpc::run_snapshot::tests::unreadable_events_survive_and_never_count_as_duplicates` | an unreadable `tool_start`, a start with no tool identity, and an unreadable fragment: all survive verbatim, and the unreadable fragment neither contributes to reconstructing a tool's arguments nor counts as duplicate evidence |
| `skills::manager::tests::a_failed_publish_drops_the_pending_operation` | a read-only install root makes the publish rename fail: the error surfaces as `publish staged skill`, nothing is left pending in `skill_operations`, and no half-published directory appears (`#[cfg(unix)]`) |
| `session::entry_records::tests::session_info_entries_persist_their_metadata_content` | a `session_info` entry appended to a session round-trips its content as *content* (the session's metadata snapshot), not as a block list — the behaviour contract, and the control for F-15 |
| `session::manager::tests::storage::opening_storage_reports_the_database_path` | the first storage read opens `agent.db` and publishes the progress line under a subscriber — the control for F-14 |

Segment 9 (`agent/tests/service_startup.rs`, `rpc/session.rs`,
`service/tests.rs`):

| test | behaviour asserted |
|---|---|
| `a_panicking_host_is_resumed_and_leaves_the_instance_reusable` (fixture case `host-panic`) | a host that panics is **resumed** to the caller rather than reported as an ordinary startup failure, and the scope is released before the unwind completes — a second `with_instance` on the same data root succeeds in the same process |
| `an_unopenable_instance_lock_stops_startup_before_any_host_code` (case `unopenable-instance-lock`) | a regular file where `<home>/agent` belongs makes the lock unopenable: the host never runs, and the foreign file is left exactly as it was found instead of being replaced by a directory |
| `a_host_error_and_a_failed_cleanup_are_reported_together` (case `host-error-and-dropped-server`) | a host error and a failed cleanup are different failures and both survive: the returned error carries the host's cause *and* the cleanup report, rather than one replacing the other. Reached by returning an error while dropping an undrained serving future |
| `connection_gate_blocks_held_locks_and_startup_races_before_storage` (case `gated-auto-addr`) | `grpc_addr = Some(" AUTO ")` is recognized as the automatic endpoint, so the local-start permit is admitted and the host runs (an explicit `IP:port` is still refused) |
| `rpc::session::tests::on_scheduled_run_finished_stops_dequeuing_once_work_is_closed` | once the work boundary has closed, a finished scheduled run does **not** dequeue the next one: the queued run is left for the shutdown drain, which is the only owner allowed to start or cancel it |
| `rpc::session::tests::compaction_events_are_published_to_the_scope_their_phase_belongs_to` | a `Standalone` compaction signal goes to the **session** scope (no run stamp, `idx = -1`) while an in-run phase keeps the run stamp (`idx >= 0`) — the distinction clients rely on to tell a session signal from a run event — and an event with no wire projection (`text_start`) is dropped rather than published as an empty frame |
| `service::tests::an_ephemeral_sessions_cancelled_queue_is_not_written_as_a_terminal_row` | an ephemeral session holding a queued run still shuts down `Stopped`: the terminal row for its cancelled queue is skipped because that write would fail against a session with no persisted transcript (no `terminal persistence` error is reported) |

Segment 10 (`service/tests.rs`, `rpc/mod.rs`, `cli.rs`,
`agent/tests/service_startup.rs`):

| test | behaviour asserted |
|---|---|
| `service::tests::a_dead_storage_worker_is_reported_by_the_journal_and_storage_stages` | a storage worker that panicked *after* startup is reported by two different drain stages: the event journal cannot flush its queued buffered delta (`event journal: …`) and the storage layer cannot join its dead writer (`storage shutdown: …`), so the shutdown is `FailedStopping` rather than a silent cleanup. The recipe — a buffered delta (no reply, so it stays queued) plus a panicking job of its own to kill the worker — is in §0.5 |
| `service::tests::an_expired_deadline_reports_the_stage_that_never_settled` | a registration that is never released keeps `counts()` non-zero, so the drain's run/native-work stage can never settle and a 50 ms deadline ends the shutdown there; the report names *that stage*, and a later clean completion cannot re-open a terminal report |
| `service_startup::an_unpolled_serve_future_times_out_the_listener_stage` (fixture case `shutdown-without-polling-server`) | a host that starts serving and then neither polls nor drops `serve` leaves the listener stage unsettled: the deadline ends the shutdown, the report names `RPC listener and streams`, and the scope **retains instance ownership** (the lock metadata survives) instead of releasing a root whose connection drain never finished |
| `rpc::tests::provider_reconcile_reports_a_replacement_whose_protocol_cannot_be_built` | the reconciliation surfaces a replacement it cannot use: a single registered model with an unknown `api` makes `ResolvedModelTarget::from_model` fail, so `set_model` reports a real error (not "busy"), the retry loop `break`s, and the session keeps its old model rather than adopting one that cannot be built |
| `cli::tests::the_agent_prefixed_helper_is_rewritten_then_refused_off_linux` | the `agent`-prefixed argv rewrite runs (the branch is not platform-gated), and the non-Linux refusal of the hidden helper proves it — a renamed standalone binary still reaches the helper deterministically |

Segment 18 (`session/journal_compaction_tests.rs`):

| test | behaviour asserted |
|---|---|
| `session::journal_compaction::tests::a_run_larger_than_the_snapshot_budget_keeps_its_raw_journal` | the **folded estimate**'s budget: a run whose folded events exceed `SNAPSHOT_BYTE_BUDGET` is left as a raw journal — `compact_run` returns 0, no `run_snapshots` row appears, and every raw event survives |
| `session::journal_compaction::tests::an_escape_heavy_payload_can_exceed_the_budget_after_serialization` | the **serialized snapshot**'s budget, a *different quantity*: the estimate counts an event's `data` once while serializing re-escapes it, so two escape-heavy events sit inside the estimate's budget and cross the serializer's. It asserts both halves of that (the estimate stays in, the serialized pair goes over) before driving the arm |
| helper `run_with_bodies` (with `escape_heavy_data`) | builds a run from `(event_type, data)` pairs closed with a real `agent_end` at a contiguous idx, which is what makes the compaction guards reachable at all |

Segment 17 (`rpc/broadcaster/tests/journal_replay.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::broadcaster::tests::journal_replay::a_compacted_run_is_served_from_its_published_snapshot` | a **historical** run that was compacted is served from its published snapshot by both readers: `events_since` returns the folded projection with no raw events (and its tuple's third value is the *minimum index of the empty event list*, a 0 that is not the cursor — the first version of this test asserted the cursor and failed, which is how that was established), and `run_snapshot` returns the same events and cursor |
| `rpc::broadcaster::tests::journal_replay::a_truncated_current_run_prefers_its_published_snapshot` | the **current** run, once its in-memory ring has overflowed (2 100 events against `MAX_RUN_EVENTS` = 2000), answers a below-the-horizon cursor from the store rather than from memory, and prefers the published snapshot over replaying raw rows |

Segment 16 (`session/history/paging_tests.rs`):

| test | behaviour asserted |
|---|---|
| `session::history::paging::tests::a_page_of_bodyless_rows_can_be_cut_at_the_materializer_budget` | a page whose rows carry **no entry body** (an index that outlived or never had its source — `history_display.source_position` is nullable) is admitted by the page reader's raw budget and then cut by the materializer's, in **both** directions: forward reports `nextOffset` at the row that did not fit, backward past it. It pins the relationship between the two budgets, which is what makes that arm reachable at all |
| `session::history::paging::tests::a_resumed_fragment_stops_on_a_character_boundary` | a client resuming a fragmented read passes its byte offset and gets the **next** fragment: the slice stops on a real UTF-8 boundary even though the budget cut lands inside a three-byte character, the reported `totalBytes` is the whole entry, and two fragments are **adjacent slices** of it (so a client can concatenate). The fixture derives the offset from the entry's own serialized form instead of guessing, which makes it deterministic |
| `session::history::paging::tests::a_multi_row_page_never_takes_the_oversized_entry_cut` | a multi-row page is transported whole: its serialized total never crosses the materializer's budget, because each row with a body contributes `overlay + body` to the raw count but only its projection to the materializer |

Segment 15 (`rpc/broadcaster/tests/journal_lifecycle.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::broadcaster::tests::journal_lifecycle::journal_lifecycle_calls_are_no_ops_without_a_configured_journal` | a broadcaster that never bound a journal is a **supported** state, so the two lifecycle calls a host makes unconditionally — the per-run completion `flush_journal` and the drain's `shutdown_journal` — both succeed instead of reporting a failure the caller cannot act on; shutdown stays terminal and idempotent (a second shutdown and a later flush also succeed) and nothing is committed |

Segment 14 (`rpc/commands/session_title.rs`, `rpc/broadcaster/tests/writer.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::commands::session_title::tests::a_successful_generation_returns_the_title_from_the_configured_provider` | the title handler's **success** path: the on-disk provider config points at a local SSE server serving a chat-completions delta followed by a `stop` finish; the response carries the streamed title as `data.title` and the session's model reference as `data.model`. Every earlier handler test stopped at model selection (a closed port), so the `Ok` arm had never executed |
| `rpc::broadcaster::tests::writer::event_batch_writer_refuses_appends_when_unhealthy_or_when_its_worker_is_gone` (extended) | an extra assertion at the end of the existing test: after `stop_worker_for_test()`, `shutdown()` **succeeds** rather than reporting a failure the caller cannot act on — retirement is idempotent, which is what a drain meeting an already-retired writer depends on |

Segment 13 (`rpc/commands/skills_tests.rs`):

| test | behaviour asserted |
|---|---|
| `rpc::commands::skills_tests::install_skill_publishes_the_installed_skill_after_a_platform_download` | the install command's **success** path, end to end: a local HTTP server serves a real package zip, the handler installs it, the skill lands in the installed directory, `list_installed_skills` reports it as managed at the requested version, and `welcome_skills` is republished for the client's next `get_state`. The two pre-existing install tests were both failure paths (a rejected id, an unreachable platform), and line 13's own count proved it: `manager.install(..)` ran 9 times and the success arm read 0 |

Segment 12 (`session/database/connection_tests.rs`):

| test | behaviour asserted |
|---|---|
| `session::database::connection_tests::a_replaced_database_file_is_invisible_to_an_already_open_connection` | the falsification record for the idle-reclamation error arm: after the database file is truncated and the WAL holding a good page 1 is removed, an **already-open** connection still answers — `freelist_count` returns from the cache and the worker still closes cleanly. It is deliberately a test of *why a line is unreachable* rather than of production behaviour, and it is what lets `database/mod.rs` 79-83 carry a real category instead of an argument |

Segment 11 (`service/tests.rs`, `rpc/mod.rs`):

| test | behaviour asserted |
|---|---|
| `service::tests::an_undriven_scheduler_worker_times_out_the_monitor_stage` | a session whose scheduler worker is registered but can never be observed to finish keeps the drain's *scheduler and cancellation monitors* stage unsettled, so a 150 ms deadline ends the shutdown there and the report names that stage. The worker is registered on a second current-thread runtime that the test never drives, which is what makes "never finishes" deterministic instead of a race |
| `rpc::tests::create_session_replaces_a_foreign_broadcaster_and_reports_a_failed_rebind` | registration of a session whose broadcaster journals **another** session: a private broadcaster is installed (the foreign one is never adopted), and the rebind itself can also fail without failing the registration — a session whose legacy import was skipped cannot bind an event journal, so `configure_journal` is reported as unconfigured while `create_session` still returns the id |

Two mechanical traps that this section had to avoid for the tripwire to pass,
and that later segments must keep avoiding: the two in §0.2 (heading-like
comments inside fences, and en-dash ranges).

### 0.4 Findings (reusable recipes)

**F-1..F-3** are behavioural findings from reading and probing the code, **1..6**
are reusable techniques.

* **F-2** `diagnostic_path` runs an unpaired quote to the end of the line, so the
  "path" it reports for a line like `cp: cannot create '/outside/note: Permission denied`
  is `/outside/note: Permission denied` (`rpc/approval/escalation.rs`). Cosmetic:
  the string feeds card text and the persistent-rule suggestion only, and a
  path with trailing prose does not resolve, so nothing is widened. Left as
  behaviour (asserted in `denial_paths_survive_an_unterminated_quote_and_shorten_home`),
  not "fixed", because changing it moves user-visible diagnostics.
* **F-3** A history reset that fails *inside the store* has already written its
  backup: the pre-reset copy stays on disk next to the untouched transcript. The
  refusal path for a plan that selects nothing (before any write) leaves no
  backup. Both asserted in `a_reset_that_the_store_refuses_is_reported_as_reset_failed`.
* **F-5** `reset_history` had a real safety invariant with **no test**: the
  digest it captures before writing the backup is re-checked inside the rewrite
  transaction, and a mismatch means "the history moved under you, re-read".
  Earlier segments classified that arm as an unreachable race. It is reachable —
  a `#[cfg(test)]` hook in that window (the pattern the repo already uses for
  `RELOAD_RACE_HOOK`) makes the concurrent write deterministic, and both the
  store refusal and the handler's `history_changed` mapping are now tested.
  **Lesson: before waiving a "race" arm, check whether a test-only seam can make
  it deterministic.**
* **F-6** A `?`-tail (`)?;` or a `}` at the end of a multi-line statement) reads
  `DA:0` while its own statement executes: measured on the current report,
  `sqlite_store/events.rs` 63 = 50 hits but 64 = 0 and 65 = 50;
  `transcript.rs` 174 = 463 but 177 = 0 and 178 = 463; same for `events.rs`
  277/350/358 and `transcript.rs` 221/323. The error paths *are* reached — this
  unit's damaged-schema tests drive them and assert the error text — so these are
  **attribution-artifact** rows with their neighbouring counts as evidence, not
  uncovered behaviour.
* **F-7 [FALSIFIED, then resolved.]** The previous segment recorded
  `session_prompt.rs` 819-829 (the run-scoped review gate) as "unreachable / the
  harness hangs". **Both halves of that were wrong**, and the lines are now
  covered (measured: `DA:count 1` on 819-829, single-target llvm-cov).
  What actually happened, so nobody repeats it:
  1. `crate::test_support::TestHome::new()` takes the process-global
     `HOME_ENV_LOCK` itself and holds it for the value's lifetime, and
     `std::sync::Mutex` is **not reentrant**. The test took `home_env_lock()`
     first and then constructed a `TestHome` → self-deadlock. The ">240 s hang"
     was that, not the product.
  2. A sandbox-wrapped plain shell command is auto-allowed
     (`if sandbox.wraps_shell() { return None; }`), so a run gate with a reviewer
     is only *consulted* for a **file-permission request** — which needs approval
     even when the shell is wrapped. The first version of the test used a plain
     `shell` call, so no review ever ran.
  **Lesson: when a fixture hangs, check the fixture's own locks before calling
  the code path unreachable — and prefer a fixture that measures the claim
  (`DA` counts) over one that argues it.**
* **F-9** The main loop's `.err().expect(..)` → `expect_err(..)` fix (F-8) has a
  sibling trap: `expect_err` needs `Debug` on the *ok* type, so a tonic stream
  response (`Pin<Box<dyn Stream>>`) cannot use it — use `.await.err().expect(…)`.
  Both spellings appear in this unit's tests for exactly that reason.
* **F-10** `session_prompt.rs` 819-829 build the run gate from
  `run_sandbox_policy.model_reviewer && sandbox.wraps_shell()`, and the account
  policy strips `model_reviewer` when no Future credential resolves
  (`account_sandbox_policy`). A test for those lines therefore needs *both* a
  credential and a sandbox-tier policy; with only one of them the block is
  skipped silently.
* **F-11** `cli.rs`'s `set_future_home` documents `source` as "names the input the
  user gave, so the error points at the right one", but only two of its three
  refusals interpolate `source`: the `create_dir_all` failure names the *path*
  ("could not create FutureOS home <path>"). Asserted as-is in
  `cli::future_home_tests…` rather than changed, because a user-visible message
  is a deliberate decision, not a coverage edit. The same test is the evidence
  that line 310 (`is not a directory`) is **unreachable-by-construction**:
  `create_dir_all` cannot succeed on an existing file, so reaching 310 requires
  swapping the path between the two calls.
* **F-12 [measured, upgrading F-6's artefact claim.]** A `?`-tail line (`)?;`) can
  read `DA:0` **even when the call's error path is provably taken**. Measured this
  segment: `sqlite_store/transcript.rs` line 177 and `sqlite_store/events.rs` line
  64 stay at 0 in a run of `sqlite_store::schema_damage_tests`, whose whole point
  is to make exactly those statements *fail* and assert the error text — while the
  neighbouring lines read 476/51. So the 0 is the line's region attribution, not an
  untaken branch, and the artefact label (with neighbour counts) is the correct
  classification rather than "needs a failing fixture". The same check is why
  `connection.rs` 19/42/52, `database/mod.rs` 65/133, `reclamation.rs` 17,
  `manager/mod.rs` 92/124/150/158/215, `manager/loading.rs` 41/65 and
  `legacy_import.rs` 20/397 are labelled as artifacts rather than as coverable work.
* **F-13** A journal event the store refuses is decided from the event's **own
  shape** (missing epoch/run id/idx) *before* any storage is touched, which is what
  lets a single batch separate "this event is refused" from "the store cannot write
  right now": drop `storage_meta` and queue `[refused, valid]`, and the batch fails
  as a refusal while the single retry of the valid event fails as storage. That
  split is the reason the two arms exist, and the test
  `a_refused_batch_retries_singly_and_separates_refusal_from_storage_failure` now
  exercises both.
* **F-14 [measured.] A `tracing` macro's *field-expression* lines read `DA:0` no
  matter how often the event fires.** Two independent instances, both measured in
  the same run: `session/manager/mod.rs` 150 is the `database = %self.database_path().display(),`
  field of the storage-open `info!` — the test enables an INFO sink subscriber so
  `149` reads **782** and `150` reads **0**, while `151`/`152` have no `DA` record
  at all; and `rpc/mod.rs` 353/354 are the `session_id`/`model` fields of the
  "no configured replacement model" `warn!`, whose arm ran **11** times (`351` = 17,
  `352` = 11, `357` = 11) and whose fields still read 0. Enabling a subscriber and
  reaching the arm are therefore *both* insufficient, which is what makes this an
  artifact rather than unexercised work — and it moves the same class out of
  "coverable": `run_control.rs` 250 is labelled with it for the same reason.
* **F-15 [measured.] An `if` *header* and a block's closing `}` can read `DA:0`
  while the whole body is covered.** `session/entry_records.rs` 133 = **3354** with
  134 (the `if kind == "session_info"` header) = 0, and the body 136-138 = **472**
  each with 139 (its `}`) = 0; likewise 213-215 = **2572** each with 216 (`)?;`) = 0,
  and 147-148 = 3 with 149 (`}`) = 0. The experiment was a new test
  (`session_info_entries_persist_their_metadata_content`) that drives the
  `session_info` body on purpose: it confirms the body runs and the header line
  does not move. This is the single largest artefact class in the unit, and it is
  why §0.5's `attribution-artifact` rows carry neighbour counts rather than a
  story.
* **F-16** Covering the *publish* failure of a skill install is possible; covering
  its *restore* arm is not, and the proof is a two-line argument worth keeping:
  line 412's `backup.exists()` can only be true if line 409's `rename(dest, backup)`
  already succeeded, which needs a writable install root on the same device — and
  that leaves only `ENOENT` on the staged candidate the function itself creates and
  holds. A read-only install root (`a_failed_publish_drops_the_pending_operation`,
  mode `0o555`, `#[cfg(unix)]`) therefore covers 411/412/416/417 and nothing can
  cover 413-415 without a delete racing the function. **Before adding a fault
  hook, check whether the arithmetic of the failure already excludes the arm.**
* **F-17 [process, not code.] `cargo fmt` is workspace-wide; this worktree is
  shared.** Using it to fix one of this unit's files also reformatted
  `agent/src/sandbox/mod.rs` and `cli/tests/bin.rs`, which other units were editing
  in the same worktree at the same time (their semantic edits are intact and the
  changes are rustfmt-only, and this handoff reports it rather than hiding it). Use
  `rustfmt --edition 2021 <your files>` when other units may be mid-edit; reserve
  `cargo fmt` for a worktree you own.
* **F-18** The clippy-clean way out of a failing `Result<T, E>` whose `T` has no
  `Debug` is *not* `let Err(error) = … else { panic!(…) }`: that is clean but adds
  one **uncovered** line per use — the `panic!` no test can reach. This unit paid
  that in two places before noticing the count go 283 → 285, and switched to
  `.map_err(|error| format!("{error:#}")).err().unwrap()`, which needs no `Debug`
  on `T` and introduces no unreachable line (the count returned to 283). Covering
  code with code that cannot be covered is a net loss under this metric.
* **F-19 [measured, and it corrects a standing assumption.] `wait_for_shutdown`'s
  "shutdown reporter unavailable" arm (`shutdown.rs` 131-134) is unreachable
  through the public API, and the reason is structural rather than incidental:
  `AgentHandle` owns the `Arc<ShutdownShared>` that owns the `watch::Sender`, and
  `wait_for_shutdown` borrows that same handle — so the sender cannot be dropped
  while a receiver of it is waiting. Only a future refactor that decouples the
  reporter from the handle makes it reachable; today it is a defensive arm, not
  untested work. Recorded so the next segment does not spend a fixture on it.
* **F-20 [measured.] The per-stage `return errors` lines in `drain`
  (`shutdown.rs` 232/273/303) are the *deadline-exceeded* exits, not the
  "nothing to do" fast path** — the `if !self.wait(..) { return errors }` shape
  reads like a fast path but `wait` only returns false once the deadline watcher
  has already published a failure. That distinction decides the fixture: it needs
  work that stays pending in *that specific stage* (`has_owned_task`,
  `compaction_in_progress`, `auxiliary_settled`) while a short deadline expires,
  not a clean shutdown. An expired deadline with a still-admitted command takes
  the earlier exit at 187 instead, which is why 232 stayed uncovered through
  segments 5-8.
* **F-21 [code reading, flagged for empirical falsification.] A branch can be
  *plausibly* reachable and still need a test seam, and the difference matters for
  classification.** The arm at `service/mod.rs` 343 ("cancellation detected after
  the server started polling but before readiness") sits between the entry
  `check()` at 301 and the first poll. Reading the body, every statement in
  between is synchronous — `enter()`, `listener_shutdown()`, constructing the
  serve future, `drop(admission)` — so there is no yield point for the *task's
  own* thread to cancel in, and only another OS thread can hit the window. That
  would put it in the same class as `shutdown.rs` 153/187 (F-4) — where the
  *fixture* was wrong — except that here the window is real rather than a test
  defect. Two consequences for the next segment: (a) it must not be waived as if
  the line were dead, because it is not; (b) it must not be classified on this
  reading alone — F-7 was falsified by exactly that kind of argument. The way to
  settle it is a fixture (or a probe that tries the window many times and whose
  DA counts are read), or the repo's existing seam pattern (`RELOAD_RACE_HOOK`,
  `RESET_RACE_HOOK`, `ACTIVATE_PERSISTED_SESSION_FAIL_HOOK`), which is what
  segment 7 used for a different race.
* **F-22 [segment 10 — a *misclassification*, worth grepping for elsewhere.] A row
  labelled `platform-unmeasured` whose code is not `cfg`-gated.** `cli.rs` 195/197
  were carried as "the `--linux-sandbox-helper` argv handoff (Linux)", but
  `run_from_args`'s rewrite lives in ordinary code that compiles and runs on every
  platform — the existing test simply passed the flag *without* the `agent` prefix,
  so the branch was never entered. The tell is in the category definition: waiver
  categories are per *line*, and `platform-unmeasured` means "compiles only on
  another OS", which is a `cfg` fact a reviewer can check with a grep. Two rows
  were reclassified the other way in the same file (228 and 234-236: they *are*
  platform-independent code that cannot be reached *here*, which is
  `unreachable-in-this-environment`, not `platform-unmeasured`). **The check is
  mechanical: for each `platform-unmeasured` row, confirm the line sits behind a
  `cfg` for the other OS.** A probe of `cli.rs`'s own test module also cost a
  process: `run_from_args` with an unknown flag makes `clap` call `exit(2)`, which
  kills the test binary rather than returning an error.
* **F-23 [segment 10 — the watcher publishes before the drain observes it.]** The
  drain's per-stage exits are only executed *after* the shutdown report exists, so
  a test that returns as soon as `shutdown()` resolves leaves the supervisor thread
  mid-wait and the `return errors` line unexecuted — the line then depends on the
  supervisor winning a race against the end of the test rather than on the
  fixture. A bounded sleep (200 ms) after the deadline made 232 deterministic; the
  same shape applies to every deadline-exit row this unit cites.
* **F-20 (corrected scope, segment 10.)** Only the exits whose stage can be made
  to *never settle* are cheap; the rest need the specific stage:
  `service_work.counts()` must be `(0, 0)` to reach the scheduler/monitor stage,
  so a second `background()`/`auxiliary()` guard cannot be used to reach 273 — the
  unsettled thing has to be a session's own runtime, and since
  `stop_cancellation_timer` aborts the armed watchdog, the auxiliary route needs a
  task an abort cannot drop (a blocking finalizer). That is the recipe the next
  segment should start from; 273 is the only stage exit left.
* **F-24 [segment 11 — what `platform-unmeasured` actually means here.] A
  mechanical sweep of every `platform-unmeasured` row against its `cfg` sites
  settles the question F-22 raised, and it cuts the *other* way.** The waiver
  ledger's one-line gloss ("compiles only on another OS") reads as though the line
  must be absent from the report, but this repo's working convention is broader and
  is visible in other modules' records: `module-cli.md` carries
  `commands/doctor.rs:173,183,186-189` as `platform-unmeasured` with "On this host
  the probe reports `available=true` … Authority: the Linux CI run", and
  `module-agent-core.md` carries a Chromium row whose arm "needs a host without a
  browser — a bare container, i.e. the Linux CI image". Those are `DA:0`-on-this-host
  lines whose *reachability belongs to another platform's run* — exactly the shape
  of this unit's `settings.rs` rows. So the category covers two situations, and the
  question that distinguishes a real defect is not "is it `cfg`-gated?" but **"is it
  reachable here with a better fixture?"** F-22's row failed that question (it was
  coverable, and it is now covered); `settings.rs`'s eight rows pass it (verified
  below). The unit keeps `platform-unmeasured` for the latter and states the
  missing host in each row.
* **F-25 [segment 11 — the mechanical sweep found two rows whose mechanism
  contradicted their category.]** Both were labelled `coverable`, i.e. "a fixture
  exists for this", and both said otherwise in their own text or their own
  neighbours: `instance.rs` 34 needs `try_write` to fail with a non-`WouldBlock`
  error ("no fixture can force that on a healthy filesystem" — the row's own
  words), and `cli.rs` 310 sits after a `create_dir_all(dir)?` that already fails
  when the path is a file, which the unit's own segment-9 test asserts. Reading a
  category against the mechanism it describes is a cheap audit: a `coverable` row
  whose mechanism says "needs a broken filesystem" or "needs another host" is
  mislabelled, and the three-line fix keeps the count honest.
* **F-26 [segment 12 — why the idle-reclamation error arm cannot be faked from
  outside.] An already-open SQLite connection keeps serving cached page 1, so
  damaging the file underneath it does not make its pragmas fail.** Two routes were
  tried: truncating the file, and truncating it after removing the WAL that still
  holds a valid page 1. Both leave the live connection answering (`freelist_count`
  returns from the cache; `SELECT 1` never touches the file at all). The arm needs a
  *live* connection whose own statements fail, which no external damage produces —
  that is what `session/database/mod.rs` 79-83 now records, backed by  `a_replaced_database_file_is_invisible_to_an_already_open_connection` and by the
  counts (success arm 235, failure arm 0, the reclaim gate 85169).
* **F-27 [segment 12 — audit (b) also catches *artifact* rows with a plausible but
  wrong mechanism.]** `rpc/session.rs` 885/886 carried the right category
  (`attribution-artifact`) with the wrong reason: "a disabled DEBUG callsite does
  not evaluate its fields". The enclosing macro is a `tracing::info!` at 881 that
  runs **6** times, and 885 = 0, 886 = 0 while 882-884 and 887-888 have no `DA`
  record at all. The category survives; the *reason* would not have survived the
  reviewer's falsification, and a waiver whose stated mechanism is false is worse
  than one with no mechanism because it looks checked. The general lesson: an
  artifact row's mechanism must name the macro and its neighbour counts, not a
  level or a subscriber setting (F-14 already said the level does not matter).
* **F-29 [segment 13 — an `assert!` failure message is unreachable by
  construction, and the ledger had six of them as coverable work.]** Rust expands
  `assert!(cond, "fmt", args…)` to `if !cond { panic!("fmt", args…) }`, so the
  expressions after the first comma are evaluated **only when the assertion fails**.
  In a green suite they therefore read `DA:0` forever, and "covering" them would
  mean making the test fail. `rpc/session.rs` 3497/3713/4974/5011/5147/5426 are
  exactly that (a persistence-error probe and four `*session.cumulative_cost{,_split}.lock()`
  dumps), and the row had carried them as `coverable` with the recipe "each needs one
  more end-to-end run fixture" — a recipe no fixture can satisfy. **The general
  check:** for any `coverable` row, ask whether the line sits inside a *message*
  position of an assertion or a `panic!`; if it does, it belongs in
  `unreachable-by-construction` (which is how the fixture-body `panic!` rows in
  `session/fork.rs` 485/500 were already classified). Worth noting the *detector* is
  easy to get wrong: my first bracket-walking script found one line out of six, so
  this was settled by reading each line rather than by trusting a tool.
* **F-30 [segment 13 — two mutually exclusive deadline arms, one of which must be 0
  every run.]** `service/shutdown.rs` 273 and 303 are the drain's deadline exits for
  the scheduler/monitor stage and the RPC-listener stage. Each has a fixture that
  makes *its* stage unsettleable, and each fixture passes; but the pair flips:
  segment 12's report had 273 = 0 / 303 = 1, segment 13's has 273 = 1 / 303 = 0,
  with all three tests green in both runs. The stage a shutdown finishes in depends
  on how the deadline interleaves with the stages, so no fixture can make both read
  > 0 in the same run. That makes them `unreachable-in-this-environment` in the
  specific sense that matters here — not "needs another host" but "the measurement
  cannot see both, whatever the fixture" — and it is the honest classification
  rather than calling one of them covered and the other coverable, which would be
  true only of the run that produced the report. Practical consequence for the
  ledger: a row naming *both* lines in its claim cell is flagged `STALE` by the
  tripwire on every run, because one of the two is always covered — which is how
  this was detected.
* **F-31 [segment 14 — the audit needs artifact vocabulary as well as "no
  fixture" vocabulary.]** Extending the row audit to words that describe
  *artifacts* (`closing brace`, `?-tail`, `closing argument`, `brace`) found **ten**
  rows claiming `coverable` while their mechanism already said "a closing brace and a
  `?`-tail" — i.e. the row was internally inconsistent and only the category was
  wrong. Measuring each line's neighbours (§0.5 now carries the counts) moved **20
  lines** from `coverable` to `attribution-artifact` and 2 more to
  `unreachable-in-this-environment`, all without touching the uncovered total. Two
  lessons: (a) a row can be *self-contradicting* — the mechanism is the honest half
  and the category is the one a reviewer reads, so check them against each other;
  (b) **one of those rows was not an artifact at all**: `journal_writer.rs` 180 was a
  reachable `return Ok(())` hiding under "early returns in the batch loop", and
  extending an existing test with one `shutdown()` call covered it. An artifact
  claim must therefore carry the neighbour counts *and* the argument that the
  enclosing statement cannot fail — otherwise it may be coverable work in disguise.
* **F-30 CORRECTED [segment 14.]** Segment 13 concluded that `shutdown.rs` 273/303
  were mutually exclusive ("exactly one reads 0 every run"). Segment 14's run has
  **both covered**: the three runs are 0/1, 1/0 and 1/1. So the pair is
  *load-dependent*, not mutually exclusive, and any single run may show either, both
  or — presumably — neither. The row is kept out of the claim cell and the three
  measurements are recorded instead. **The general lesson for this goal:** a
  two-run observation of a "flip" is not enough to establish exclusivity, and the
  correction cost nothing because the row's text carried the raw counts rather than
  only the conclusion.
* **F-32 [segment 15 — a "not yet measured" annotation is itself a deliverable.]**
  Segment 14 left one row explicitly marked *"these three are not yet measured — the
  row may move too"* rather than assuming, and segment 15 measured it: all three
  `session/fork.rs` lines and `journal_writer.rs` 322 are artifacts (each brace's
  block body runs 4-5 times while the brace reads 0). Two things are worth keeping.
  First, the annotation paid for itself immediately: it turned an assumption into a
  five-minute measurement. Second, **the same row also shows the opposite risk** —
  in segment 14 the identical wording ("early returns / closing braces") hid
  `journal_writer.rs` 180, a *reachable* line that one added `shutdown()` call
  covered. So a brace row is not automatically an artifact, and an unmeasured brace
  row is not automatically coverable: it is an unmeasured row, and the only honest
  state until someone runs the neighbour check. The annotation is now the ledger's
  convention for that state, and `audit_artifact.py` flags any *claimed* artifact
  that cannot show its neighbours.
* **F-33 [segment 15 — two independent filters can prove a match arm dead.]** The
  `_ => {}` arm of the pricing replay (`rpc/session.rs` 1371) is unreachable, and the
  proof is a conjunction rather than a single guard: the read side selects only
  `event_type IN ('usage','model_changed')` (`sqlite_store/events.rs` 56-58), and the
  **snapshot** side only stores payloads whose decoded event matches
  `"usage" | "model_changed"` (`journal_compaction.rs` 212-221). Either filter alone
  would leave the other source open — the `UNION ALL` in that query is why the row
  had been left as coverable for several segments. **Look for every writer of a
  value before declaring an arm dead, not just the one in the same statement.**
* **F-34 [segment 15 — a test can carry the name and comment of a branch it never
  takes.]** `get_fork_messages_handles_legacy_bare_string_content`
  (`commands/session_lifecycle_tests.rs` 723) says in its comment *"this legacy shape
  is written directly to disk to exercise the fallback branch"*, and it passes — but
  it writes through `SessionManager::storage().replace(..)`, which runs
  `normalize_entry`, so the stored row is a **block array** and the test asserts
  through the *array* branch (line 352-360). Line 361 therefore reads 0 with the
  test running and green. The assertion (`blocks[0].text == "plain legacy text"`) is
  satisfied by both branches, which is exactly why nobody noticed. Consequences:
  (a) the row is `unreachable-by-construction`, not coverable — no write this build
  performs can produce the input; (b) the test keeps its value (it proves the legacy
  *text* survives the round trip, which is the user-visible contract) but its
  comment should not be read as coverage proof. **The general check: when a test
  asserts a value both branches would produce, its name is not evidence — only the
  line count is.**
* **F-35 [segment 16 — the STALE check stops covering a file the moment it reaches
  zero, which is when a stale row is most likely.]** `reconcile.py` resolves each
  ledger row's path against the report's **uncovered** line map
  (`claim_targets(path, ln, per_file)`); a file with no uncovered lines is not in
  that map, so `claim_targets` returns empty and the row is skipped. This segment
  took `session/history/paging.rs` from 7 to 0 and the run printed **no STALE block
  while its seven-line row still claimed lines that were now covered** — the row had
  to be removed by hand. The direction matters: it is a *false clean*, the same
  family as the earlier "0 claims parsed" vacuity, and it fires precisely at the
  transition a coverage campaign spends its time producing. Until the tool is fixed,
  the rule is: **when a file's count reaches zero, re-read its rows and delete or
  convert them; do not take the absence of a STALE line as evidence.**
* **F-36 [segment 16 — two budgets over different quantities decide one arm, and the
  arm is reachable only where they coincide.]** `read_display_page` counts a row's
  **raw** size (indexed overlay + stored entry body) and caps a page at
  `HISTORY_PAGE_BYTE_BUDGET`; `materialize_display_page` counts each row's
  **re-serialized projection** against the same constant. For a row *with* a body the
  second count is about half the first (the overlay is itself a projection of that
  body), so a multi-row page can never reach the materializer's budget — which is
  why the cut arm had read 0 for ten segments while the two adjacent lines had run
  471 and 23 times. The arm becomes reachable exactly where the counts coincide: a
  display row with **no** entry body, where the payload *is* the indexed overlay.
  `history_display.source_position` is nullable for that case, so the shape is real
  rather than manufactured. **The general recipe for a "budget/limit" arm that never
  fires: find the two places the same budget is applied to different quantities, and
  look for the input where the two quantities are equal.**
* **F-37 [segment 17 — the compaction fixtures' two gates, which cost this
  segment three test iterations.]** A `run_snapshots` row can only be produced by
  `compact_run`, and `compact_run` refuses — returning `Ok(0)` without writing
  anything — unless **the run's last raw event is `agent_end`**
  (`journal_compaction.rs` 276-277). A fixture that broadcasts 2 100 `text_chunk`s
  and stops therefore compacts nothing, and the snapshot arm stays uncovered with
  no error to explain it. The second gate is the one that makes the *current*-run
  path reachable at all: `events_page` only consults the store when its ring has
  dropped events the caller still asks for (`truncated`), so the fixture must
  overflow `MAX_RUN_EVENTS` = 2000 **and** close the run properly. Recipe in one
  line: `start_run` → >2000 events → one `agent_end` → seed the `runs` row with a
  terminal status → `compact_run`. **Generalisation:** when a helper returns a
  silent `Ok(0)`/`None` instead of an error, grep it for early `return`s before
  building the fixture — six of this function's guards are invisible from the call
  site, and only one of them is about the run being settled.
* **F-38 [segment 18 — three budget checks in a row, over three different
  quantities, and only the last is dead.]** Compaction decides whether a run is
  representable as a snapshot by comparing **three** sizes against the same
  constant, and they are not the same thing:
  1. the reader caps each **stored** payload at `PAGE_BYTES` = 4 MiB and returns
     `NULL` above it, which turns into an earlier `Ok(0)` (`events.rs` 364-365 →
     `journal_compaction.rs` 186) — so the first fixture, one huge event, never
     reached the arm it was written for;
  2. `DeltaFolder::estimated_bytes` counts a non-coalescable event as
     `data.len() + 512`, once;
  3. `serde_json::to_string(&snapshot)` re-escapes every `data` string, so a
     payload with *n* escapes grows by `7/6` (each `\u0001` gains a backslash:
     6 bytes → 7).
  The reachable window for each arm follows from the arithmetic, and writing it
  down before the fixture is what makes the fixture one iteration instead of
  three: to reach (2) safely, use **many** events of ~2 MiB rather than one large
  one; to reach (3), keep the estimate within the budget while the re-escaped
  total crosses it. **General form (extends F-36): when several guards share one
  constant, list what each one *measures* before choosing the input — a fixture
  that satisfies one guard may be rejected by an earlier one over a different
  quantity, with a silent `Ok(0)` as the only feedback.**
* **F-39 [segment 18 — a "budget" row may be dead while its *condition* runs.]**
  The pricing-array arm is the cleanest arithmetic kill in this unit:
  `pricing_bytes = 2 + Σlen + n` while the serialized array is
  `pricing_bytes − 1`, so the `ensure!` one line earlier already guarantees the
  guard can never fire. The measurement agrees exactly: the condition (292) runs
  **88** times and is never true, and the body (293) reads 0. Note the ledger
  consequence, which the tripwire caught: **only the body is a claim** — naming
  292 in the claim cell is a `STALE` error, because 292 *does* run. A dead guard
  therefore needs two different treatments inside one row.
* **F-8** `cargo clippy -p future-agent --all-targets -- -D warnings` found two
  real defects that the (green) test suite could not: `clippy::err_expect` on a
  `Result<String, Status>` in the gRPC test and `clippy::type_complexity` on a
  four-element test helper's return type. Both are fixed. Worth noting for every
  unit: a green suite is not a green PR — this unit shipped those two until the
  new gate requirement surfaced them.
* **F-4** `shutdown.rs` lines 153/187 were reached only after the test held
  admitted work past one poll interval of `wait()`: the loop checks `settled()`
  *before* the report, so dropping the last piece of work in the same tick as the
  deadline report exits the loop without taking the "deadline already reported"
  branch. Measured, not guessed (single-target llvm-cov on `service::tests`).

1. **Handler admission guards are shadowed by the dispatcher.** `commands/mod.rs`
   refuses a command when `state.service_work.enter()` fails (line 45) *and*
   rejects `COMPACTION_CONFLICTS` before dispatch, so the equivalent guard inside
   a handler (`settings.rs` line 191 and `session_reset.rs` lines 89-96) never runs through
   `handle_command_internal`. Two existing handler tests passed while their guard
   lines stayed at `DA:0`. **Check any residual admission/guard arm in
   `commands/*.rs` against the dispatcher first**; such arms are covered by
   calling the handler directly (`commands::settings::handle_compact`,
   `commands::session_reset::handle_reset_session_history`).
2. **A `?`-tail at the end of a statement is the error-propagation region.** Where
   the called function can be made to fail, forcing the failure covers it — that
   is how the six `display_index.rs` tails fell (damaged schema, refusing
   triggers). Where the same trick does *not* move the counter (see 3) the line
   is an attribution artifact.
3. **A statement with no branch between two covered neighbours can still read
   `DA:0`.** `evidence.rs:316` is a plain `fields.insert(...)` directly after the
   covered `fields.insert(...)` on 314 inside one block; no input can skip 316
   while running 314, so its counter is a region-splitting artifact. Same class:
   `grpc/mod.rs:100-101` (`address.port()` is skipped although `address.ip()` on
   the previous line of the same `tracing::info!` runs and the event is
   emitted), and the closing `}` lines listed in §0.5.
4. **`watch` channels cannot error while the borrow is alive.**
   `shutdown.rs:131-134` is unreachable by construction: `wait_for_shutdown(&self)`
   holds the `Arc<ShutdownShared>` that owns the `Sender` for the whole call, so
   `changed()` can never report a dropped sender.
5. **A key with no producer is dead code, provably.**
   `evidence.rs:573-574` builds `truncated_sources` from records whose
   `"truncated"` field is `true`; `grep -rn '"truncated"' agent/src` finds only
   that read site, so no record can ever match.
6. **`expect_err` needs `Debug` on the ok type.** For a tonic stream response use
   `.await.err().expect(...)` — it drops `T` and needs no bound (a `match` with an
   `Ok(_) => panic!()` arm would add a never-executed line to the ledger).

* **F-40 [segment 19 — a `break` that no input can reach, proved by probing
  ten states rather than by arguing].** `reclaim_space_when_idle` breaks when
  `PRAGMA incremental_vacuum` leaves the freelist unchanged while returning `Ok`
  (`reclamation.rs` 45-46); its test-only twin does the same (79-80). The
  *plausible* mechanism — a pinned reader, or free pages that cannot be relocated —
  is wrong on both counts. In this build (rusqlite's bundled SQLite, WAL,
  `busy_timeout(0)`) the call frees **exactly one page** whenever it can run, and
  returns `database is locked` when it cannot write; the two rows are therefore
  `unreachable-in-this-environment`, and `database/reclamation_arm_tests.rs` ships
  the eleven-check invariant as the falsifier. Note the shape this shares with
  F-38: one constant, several guards, and a fixture that satisfies one guard while
  an earlier one rejects it — here the earlier guard is "a write lock exists" and
  the rejection is an `Err`, not a silent `Ok(0)`.
* **F-41 [segment 19 — an assertion can pin a load-dependent outcome].** The
  pinned-reader test passed alone and failed in the parallel run
  (`left: 21, right: 0`): the reclamation loop is bounded by the 10 ms idle slice
  as well as by the freelist, so under load it stops early with pages left. The
  honest assertion is the **direction** (`free_after < free_before`), not the
  distance. This is F-30's lesson one level down: there a two-run "flip" was read
  as exclusivity; here a single isolation run read as determinism.
* **F-42 [segment 19 — a probe that proves a category and returns zero lines is a
  legitimate deliverable].** `reclamation_arm_tests.rs` covers **no** previously
  uncovered production line, yet it is the entire evidence for moving two lines to
  a waiver category. Recording it matters for a second reason: the file must stay
  in the suite, because it is what will fail if the category ever stops being true.

* **F-43 [segment 20 — `service/mod.rs` 343: F-21's "unschedulable window" is
  schedulable, and the way it was proved is the reusable part].** F-21 reasoned from
  the source that every statement between `serve`'s entry check (301) and the first
  poll (338) is synchronous, so "only another OS thread can hit the window", and
  left the line *flagged for empirical falsification* rather than classified. The
  measurement: a thread spinning on `server_running` — the last observable before
  the poll — and cancelling the instant it sees it hits the window **40 of 40** times
  (and 20 of 20 in the shipped fixture). The gap is not empty: between setting
  `server_running` (324) and polling the server (338), `serve` subscribes to two
  `watch` channels and builds the guard and the pinned future, which is *hundreds of
  nanoseconds* of work, while a thread already spinning sees the store in tens. The
  general lesson is F-38/F-41 in a new place: **"no schedule can reach this" is a
  quantitative claim about relative durations, and it must be measured, not
  reasoned** — and the measurement must be *biased towards a hit* (here, by giving
  the canceller a head start), or a zero proves nothing.
* **F-44 [segment 20 — a fall-through `Err` with every arm returning is dead, and the
  counts are the only way to see it].** `approval_review/provider.rs` 188 is dead by
  construction: `for attempt in 0..=1` with both `continue`s guarded by
  `attempt == 0`, so the last iteration always returns. The trap is that the line's
  *value* is indistinguishable from a live one: the existing double-5xx fixture
  (`500` then `502`) asserts `reviewer_http` and passes, and that string comes from
  `179` (`176 = 2`, `179 = 1`, **188 = 0**). So a test asserting the message looks
  like it covers the line; only the per-line counts separate them. Generalised: when
  two lines produce the same observable, the ledger row must cite the counts, and a
  "recipe" that predicts a reachable arm must be checked against them.
* **F-45 [segment 20 — an acknowledgement lives inside the accepted command].** The
  journal writer's `AppendFailure::unavailable` arm fires when a caller's reply
  channel is dropped *after* its command was accepted. The first version of the
  fixture held the queue's receiver and dropped **it**, assuming that would abandon
  the reply — it hung instead: `recv` on a channel whose *sender* is gone still
  yields whatever was already queued, and the reply sender had travelled *inside* the
  `Append` command, which the test was still holding. The working model is to receive
  the command and drop **that**. It is also why no live worker can be used: both ways
  the worker stops (`Shutdown` and `Drop`) flush pending events and answer them
  first, so a hand-held channel is the only honest fixture for "the worker died with
  the command in hand".

* **F-46 [segment 20 — clippy found two defects the green suite could not].** The
  first version of both `service::tests` fixtures built their config as
  `let mut config = AgentConfig::default(); config.future_home = …;`, which
  `clippy::field_reassign_with_default` rejects under `-D warnings`. The tests were
  green, the coverage was right, and only the lint gate saw it — the PLAN §5.1 rule
  in this unit's own history for the second time (F-8 was the first). Fixed with
  struct-update syntax, then the whole measurement was re-run on the fixed tree
  rather than assuming a test-only edit could not move a number.

* **F-47 [segment 21 — F-28's cleanup is too blunt, and the literal rule costs a
  full rebuild].** F-28 says to `rm -rf target/cov-agent-a/llvm-cov-target` before
  measuring, so a failed run cannot leave stale profiles for `cargo llvm-cov report`
  to merge. That directory also holds the **build**, so the rule as written forces a
  from-scratch rebuild of every dependency: under this machine's peer load (13-16
  load average) that took **8 m 53 s**, and two measurement attempts died on the
  timeout with *no* target completed. The profiles are `*.profraw` files **directly in
  that directory**, so the equivalent and sufficient form is
  `find target/cov-agent-a/llvm-cov-target -maxdepth 1 -name '*.profraw' -delete`,
  after which the same measurement finished green in **103 s** with the build intact.
  The invariant F-28 protects is "no stale profile data", not "no build".
* **F-48 [segment 21 — an edit that removes structure leaves valid-looking code].
  ** Adding a test by replacing an anchor that included the *existing* test's
  `#[test]`/`fn` header left that test's body attached to the new function, so the
  original test silently ceased to exist while the file still parsed. It was caught
  by reading `git diff` for the file. This is segment 19's lesson (an edit that ate a
  newline and merged an attribute into a comment) in a second shape, and the cheap
  check is the same: after inserting a test, grep for the **neighbouring** test names
  and read the diff's `-`/`+` lines, not just the test result.
* **F-49 [segment 21 — two rows carried a mechanism that did not match their line].**
  `recall.rs` 179 was labelled `coverable` as "a match arm in snippet assembly (a
  match at an offset that yields an empty fragment)", but the line is the `}` closing
  the `if let Value::Object(..)` in the *search-result assembly*, with `174-178` all
  executed (10/10/10/2/8). `session_lifecycle.rs` 474/578 were labelled "a loop that
  refuses the model sync", but `set_model` fails **before touching any loop lock** —
  at its protocol validation. Both were found by reading the line rather than the
  label, and both then got the right fixture (474/578) or the right category (179).
  The unit now has four such corrections (F-25/27/31/49); the pattern is that a row
  written from a *guess* about where a line sits is the row most likely to be wrong.

* **F-50 [segment 22 — a guard that reads health cannot be reached by breaking
  health].** `prompt_internal` drains the event journal before asking for compaction,
  and its warning fires only when the drain fails **while the journal is healthy**
  (`!is_ephemeral && broadcaster.persistence_error().is_none()`). That state cannot be
  provoked from the outside: every way a live drain fails — the worker gone, or a store
  that can no longer commit the batch — makes an *append* fail first, and an append
  failure marks the journal unhealthy, which takes the guard's other branch. Two
  external routes were tried and both were refuted by measurement: dropping
  `storage_meta` mid-run (the buffered deltas are invisible to it, but `agent_end` is a
  *durable* append, so it fails first and the run then never finishes — the first
  version of this fixture timed out at 30 s, which is the evidence), and retiring the
  worker mid-run (same reason). So the error is *injected* through a test-only fail
  point, exactly as this repo already does for append failures and for four
  persistence-command failures; what the fail point does not establish is reachability,
  and that is established by two existing tests that make a real drain fail (a dead
  worker; a store that cannot commit the batch). The generalisable rule: **when a guard
  tests a *precondition* and the failure path you want to reach is the one that
  invalidates that precondition, the two are mutually exclusive from outside — inject
  the error and cite the tests that prove it is reachable.**
* **F-51 [segment 22 — a dead arm whose sibling shape is legitimate still needs a
  measured invariant].** `session.rs` 979 is the `_ => None` arm of the compaction
  summary join. The non-text shape it handles is real — `compaction/mod.rs` 715 builds
  exactly `[Text, Reasoning]` for a *different* consumer of the same field — which is
  why this row read "the seed shape is legitimate" and stayed `coverable` for several
  segments. What settles it is that the *summarizer* ends in
  `vec![ContentBlock::text(summary)]` (`compaction/semantic.rs`), so nothing the
  preparation produces can be non-text; the loop had run 14 times and never matched.
  Rather than argue it, segment 22 asserts it: the new compaction test reads the
  committed checkpoint back off the loop and requires every summary block to be `Text`.
  That assertion is the row's falsifier — the F-26/F-42 pattern applied to a
  classification rather than to a probe.
* **F-52 [segment 22 — the injection idiom has a clippy-visible trap].** The natural
  form for a test-only fail point is `#[cfg(test)] if take(&mut journal.flag) { … }`
  inside a function that then needs `let mut journal`. In a **production** build the
  whole block disappears, so `mut` becomes unnecessary and
  `cargo clippy --all-targets -- -D warnings` fails with `unused_mut` (it did). The
  shape this repo already uses — a *non*-`cfg`-gated field consumed by
  `if cfg!(test) && take(&mut journal.flag)` — keeps the statement single-line in both
  builds, so there is no uncoverable `cfg(not(test))` branch and no build where `mut`
  is unused. Copy that shape for any new injected failure.

* **F-53 [segment 23 — the `reload_failed` arm is unreachable by *input*, and that
  is a fact about the read path, not about the guard].** The handler writes the
  history, then re-reads it into memory; the arm fires when the re-read fails. Every
  *input* that could make the re-read fail makes the **planner** fail first, because
  both go through the same `entry_records` view (`reset_history` → `storage().entries()`
  → `typed_entries` → `read_entry_rows`): dropping the view before the command lands on
  `reset_failed` — which is exactly the dead end §0.6 had recorded for this row two
  segments before, so the dead end was real and the row's recipe was the fantasy. What
  makes the arm reachable is a failure that arrives **between** the two reads — and the
  reset's own `digest`/`rewrite` pair exists precisely because something else can touch
  the store in that window. So the fixture drives that window through a test-only hook,
  the documented idiom in this repo for exactly this class (`RESET_RACE_HOOK`,
  `RELOAD_RACE_HOOK`), and asserts the *behaviour* the arm is for: the caller is told
  the disk was reset while memory was not, with the backup path to recover from.
  **Generalised:** when a read-before and a read-after go through the same path, no
  input can separate them — the window between them can only be driven by a
  concurrency seam. Check whether the pre-read is the same object before writing the
  fixture; if it is, save the budget and go straight to the hook.
* **F-54 [segment 23 — a category can be falsified by load, and the counter-evidence
  was already inside this unit's own record].** `database/mod.rs` 79-82 (the idle
  reclamation failure arm) read **0** in every report from segment 12 to segment 22
  and was carried as `unreachable-in-this-environment`, with a probe asserting the
  invariant "`incremental_vacuum` never reports no-progress silently". In this report
  they read **1** each, with no source change: one of the worker's ~269 idle reclaims
  hit a lock timeout under load 13-16. The mistake was readable from the unit's **own**
  segment-12 probe: it measured that `PRAGMA incremental_vacuum(1)` returns
  `database is locked` under a foreign write lock — i.e. the `Err` the arm needs is
  produced by a condition that *does* occur. The probe answered a different question
  ("can file damage do it? no") and was written up as if it had answered this one.
  **Rule: a probe's negative result only covers the mechanism it actually tried** —
  the categories need "no mechanism can", and one refuted mechanism is not that.
  A deterministic fixture is now written out in §0.6.
* **F-55 [segment 23 — the *arm* is the claim, not its line numbers].** `journal.rs`'s
  `event serialization failed` closure is dead, and llvm-cov attributes its unentered
  region to **200/201** in one report and **202/203** in the next with no source change
  between them. Line numbers that move under the same code mean a row pinned to them
  will drift in and out of `STALE` on its own; the mechanism and the arm are stable, the
  numbers are not. Write the arm into the mechanism cell and re-read the pair from the
  report the ledger cites — and when a `STALE` fires on a pair like this, check the
  *source* before renumbering. **Two corrections (finding F-5b of `review-agent.md`,
  accepted here):** (a) the closure is dead because **`serde_json::to_value(event)`
  cannot fail** on `&SseEvent` — five `String`s and two `i64`s, no non-string map key,
  no custom `Serialize` (`rpc/protocol.rs`); the earlier wording named `to_string`, a
  function this code does not call. (b) "expect the numbers to drift" overstated it: the
  pair slid **once** (`s32` → `s33`) and then agreed across four consecutive reports
  (`s33`-`s36`), so the honest instruction is "re-read the pair from the report the
  ledger cites", not "expect it to move".
* **F-56 [segment 23 — the ledger's own delta tables are claim rows].** Three times in
  two segments a `STALE` block has been caused not by a ledger row but by a *delta-table
  row* of mine (`| audit (b) on \`session/database/mod.rs\` 79-83 | … |`), because a row
  whose first cell carries `path + numbers` is parsed as a claim wherever it sits. The
  fix is always the same one the ledger rows already use: prose in cell 1, numbers
  later. When a `STALE` block names lines that no ledger row claims, grep the **whole
  section** for the path, not just the ledger.

* **F-57 [segment 24 — a window that cannot be widened, and why the assertion is a
  floor].** `activity_snapshot` reads the work revision, walks the sessions, and
  re-reads the revision before publishing; if work moved in between, the snapshot is
  refused. That window is real but microseconds wide, and — unlike every other race
  fixture in this unit — nothing can be held to widen it: every lock the function
  takes (`sessions.try_read`, each `session.try_read`, `try_pending_count`,
  `journal_compaction_activity`) is a **`try_`** lock, so acquiring one from the test
  produces a *different* error rather than a delay. The only honest fixture is the
  biased one (F-43): a thread keeping the revision in motion while the snapshot is
  tried repeatedly, asserting "at least one hit in N" and printing the tally. And
  because the assertion is a floor, the **evidence** that the right arm ran is the
  DA count (measured 71 = 1 with 70 = 2) checked by probe — not the assertion.
  Generalised: check whether a window's locks are `try_` before planning to widen it;
  if they are, plan the biased fixture and the probe from the start.
* **F-58 [segment 24 — polling a worker prevents the idling you are waiting for].**
  The first version of the idle-reclaim fixture polled `freelist_count` every 25 ms
  while waiting for the reclaim to happen. Every probe is a job, and the worker's
  reclaim runs **only** on a `recv_timeout` expiry (50 ms), so a probe every 25 ms
  meant the timeout never expired: the check was the reason the thing it checked for
  never happened, and the test failed with "reclamation must resume". Sleeping 150 ms
  between single probes fixed it. The unit already carried the inverse of this lesson
  from segment 16 (a budget arm needs the right *input*); this one is about the
  *observer* changing the observed. Any fixture that waits on an idle-triggered
  behaviour must leave the worker idle to be triggered.
* **F-59 [segment 24 — a brace line can move from artifact to covered when its block
  starts running repeatedly].** `database/mod.rs` 83 is the `}` of
  `if !reclaim_error_reported {`; it read 0 in every report while the arm ran at most
  once, and segment 23's row labelled it an attribution artifact with the neighbours'
  counts. Once the deterministic fixture made the enclosing loop run the arm **four**
  times in a slice, the same brace reads **3** and is a coverage claim. So "artifact"
  is a statement about *this run's* region attribution, not a permanent property of a
  line — and a brace row should be re-read whenever a fixture changes how often its
  block executes.

* **F-60 [segment 25 — a gate line cited a log that was never written].** Segment
  24's §0.7 block printed `cargo fmt --check EXIT=0 clean (/tmp/g28-fmt.log)`. The
  command was never run: `/tmp/g28-clippy.log` exists, `/tmp/g28-fmt.log` does not.
  The claim was an *assumption* about a gate, formatted to look like its output — and
  the supervisor's own run caught a real red in this unit's file around that time,
  which is how it surfaced. The substance happened to be right (the tree is clean
  now, checked for real in segment 25), but provenance was wrong, and "the figure
  agrees with reality" is not the standard the campaign set: it asks for the tool's
  actual output. **The mechanical guard is trivial and is now the rule here: quote a
  log path only if the file exists.** `ls` the log before citing it; when a gate was
  not run, say "not run" rather than leaving a plausible number.
* **F-61 [segment 25 — a fixture whose cost is a wall-clock wait may cost nothing at
  all].** `provider.rs` 181 carried the same sentence for five segments: the client is
  a `static` with `.timeout(REVIEW_TIMEOUT)` = 30 s and nothing injects a shorter
  deadline, "so the only route is a 30 s wait — a test the suite would pay for on every
  run". That reasoning stopped one step early. The timeout is a **`tokio` timer**, and
  a `start_paused = true` runtime auto-advances timers when every task is idle: the
  fixture runs in 0.01 s and the 30 s is free. The generalisable part is the *check*:
  before classifying a line as "coverable but too expensive", ask **what the wait is
  made of** — if it is a `tokio`/`std` timer on the test's own runtime, paused time
  costs nothing; if it is a real syscall, a sleep in a thread pool, or a foreign clock,
  it is a genuine cost. This is the second finding from this unit that came from
  refusing a "too expensive" label (F-47 was the first, about deleting a build).
* **F-62 [segment 25 — an ordered drain makes one select arm unreachable, and the
  ordering is the proof].** `cli.rs` 590/591 are the two statements of `async_main`'s
  final `select!` first arm (the shutdown report completing before the serve future).
  They had been `coverable` with a guess-shaped recipe since segment 12. Reading the
  order settles their categories *differently*: `drain` waits for `!server_running`
  before its storage stages, `server_running` is cleared only in `ServerGuard::drop`,
  and the guard lives inside the serve future that `async_main` pins in its own scope —
  so a **`Stopped`** report cannot exist while the serve future is pending, and the
  drain still has stages *after* the listener wait, so the report lands strictly later.
  **591 is therefore unreachable-by-construction**, and only 590 remains (reached by
  the *other* publication path: the deadline watcher failing the listener stage while a
  long RPC keeps tonic from settling). Two lessons: a pair of lines can end up in
  *different* categories, and "which future can publish first?" is answered by the
  stage order, not by trying fixtures.

* **F-63 [segment 26 — the closure decision, **OVERTURNED**. Kept in place because the
  wrong reasoning is the reusable part].** `cli.rs` 590 was the unit's last `coverable`
  line, and the decision was to **waive it by cost**. `review-agent.md` (F-1) falsified
  the waiver by building the fixture in ~5 s, and §0.4's F-64 records the closure; what
  is worth keeping from F-63 is *which* of its steps were wrong, because they are the
  campaign's most repeatable mistakes:
  * **The route analysis was right and still led to the wrong conclusion.** The arm
    does need a drain stage that fails **before** the RPC-listener wait, and every
    pre-listener exit is indeed deadline-driven — but the conclusion drawn from that
    ("so the fixture must keep an unresolved run or admitted work alive for 4 s") was
    too narrow: an admitted `ExecuteCommand` is not the only thing that keeps a later
    stage open. A **native background registration** (`service::work::process_background`)
    held by a thread parked on a platform fetch does it with no RPC at all, and the
    stage that then fails is the later `"runs and native background work"` wait
    (`shutdown.rs` 215), not the `"admitted commands"` wait (184).
  * **"Every unary command returns promptly without a real provider" is FALSE**
    (corrected here, finding F-5a of `review-agent.md`): `shell` blocks for up to
    `SHELL_EXECUTION_TIMEOUT = 120 s` (`packages/rpc/src/command_policy.rs:18`), and the
    review measured the grpcurl call still running after 2.5 s. That route fails for a
    different reason — the drain **cancels** it (`cancel_all` in `drain`, the reply is
    `"shell command cancelled"`), so no stall survives the cancellation; not because the
    command returns quickly.
  * **Cost was asserted, never measured.** "The fixture would race a wall-clock deadline
    inside an already-flaky suite" is the shape F-47 and F-61 had already refuted twice
    in this unit (a "too expensive" label that dissolved on inspection). The measured
    cost here is **~4.3 s of wall time in one test, with a deterministic outcome** — the
    error path lands 4.03 s after the interrupt and does **not** race the Ctrl-C grace
    watchdog, because the pending platform fetch holds no blocking task for
    `runtime.shutdown_timeout(1 s)` to wait on.
  **The generalisable part, restated: "coverable" and "worth covering" are different
  questions, and the second one may not be answered by reading what a blocker "would
  have to be" — read what the *drain* waits on, stage by stage, and then ask which of
  those waits the agent's own startup code can already hold open.**

* **F-64 [the rework — `cli.rs` 590 is covered by a test, and the fixture is smaller
  than both the waiver and the review assumed].** `a_cut_short_drain_is_reported_instead_of_a_clean_stop`
  (`agent/src/cli.rs:923`, `mod cut_short_drain_tests`) closes the first arm of
  `async_main`'s final `select!`. **Reusable recipe (no model, no provider, no RPC,
  ~4.3 s, deterministic):** an isolated HOME carrying `auth.json` whose
  `future.base_url` is a local listener that **accepts and never answers**, one real
  agent subprocess re-executed from the test binary (`crate::cli::run_from_args`, the
  `cli::shutdown::tests` pattern), wait for the handshake, then **SIGINT**. The
  cold-cache Future catalogue refresh that the first `Registry::new()` spawns
  (`models/future.rs`'s `spawn_future_models_refresh_unlocked`: it takes
  `crate::service::work::process_background()` and then parks a native thread inside
  `fetch_future_models`) holds that registration while the fetch waits, so the drain's
  `"runs and native background work"` stage cannot settle, the deadline watcher
  publishes `FailedStopping` at the 4 s `async_main` itself requested, the listener
  stage is never reached (the serve future is still pending), and the **first** arm runs
  its `ensure!`. Measured: the child exits **1** with `Agent failed to stop: Agent
  shutdown deadline exceeded during runs and native background work; instance ownership
  retained` 4.03 s after the interrupt, and in the same report `DA:590 = 1` /
  `DA:591 = 0`.
  **Two reusable lessons beyond the line:**
  1. **The un-cancellable class is bigger than "an admitted RPC".** `drain` cancels
     session runtime state (§0.5's `cli.rs` 590 row names the calls); anything that
     registers **native background work** instead — a platform fetch, a monitor, a
     scheduler — survives cancellation, and the agent's **own startup** creates such
     work on a cold cache with no client involved. When a drain-stage line needs a
     fixture, the first thing to ask is what the agent starts by itself that talks to
     the network.
  2. **The control leg is what made the mechanism honest — and the first version of it
     was wrong.** That version's control kept the stalling platform credential and only
     skipped the gRPC `sync_future_models` call, on the assumption that the *admitted
     RPC* was what held the drain. The control failed the same way the fixture leg did,
     which is what proved the RPC was never the blocker: the startup background refresh
     alone pins `"runs and native background work"` open. Had the test shipped without a
     control that removes the **credential**, its name would have credited the wrong
     mechanism — and a fixture that "works" on the first try is exactly the case that
     deserves a control leg before it is cited as evidence for a specific line.

### 0.5 Residual ledger — **202 lines**, every line classified

`python3 .future/cov100/covlines.py detail /tmp/agent-a-s35-norm.info --under agent/src/rpc …`
(full command in §0.1). Categories are `docs/testing/waiver-ledger.md`'s, plus
`attribution-artifact` for a line llvm-cov does not instrument.
`✔` marks a mechanism this unit established by reading the code and running its
check; a row whose count column is `—` is a line this unit's tests moved to
covered, kept in place so a reviewer can see it was classified rather than
dropped. Every line in the unit appears in exactly one row, and the table sums
to the measured headline.

| file | lines | n | category | mechanism |
|---|---|---|---|---|
| `rpc/commands/settings.rs` 14, 15, 16, 17, 19, 39 | 6 | unreachable-by-construction ✔ | `serde_json::to_value(..).unwrap_or_else(..)` fallbacks whose value can only contain `bool`/`String`/`Option<String>` and a `PathBuf` built from the literal `/usr/bin/sandbox-exec`; `to_value` cannot fail (macOS: seatbelt; Windows: `path: None`) |
| `rpc/commands/settings.rs` 21, 22, 23, 24, 43, 44, 45, 46 | 8 | platform-unmeasured ✔ | the `Err` arms of `platform_sandbox_probe_product` / `probe_windows_sandbox_product`. **Verified TRUE by segment 11's mechanical sweep, with the cfg sites named:** the only branch that can return `Err` is `sandbox/mod.rs` **1073-1075** (`#[cfg(target_os = "windows")]` → `probe_windows_sandbox_product()?`); the macOS branch (**1055-1072**) and the Linux branch (**1085-1099**) both build `Ok` unconditionally. For the Windows helper the reachable-here body is `sandbox/mod.rs` **1202-1207** (`unavailable_without_error`), so only the `#[cfg(target_os = "windows")]` body at **1214-1217** can produce `Err` |
| `rpc/commands/settings.rs` 33 | 1 | platform-unmeasured ✔ | `handle_probe_windows_sandbox`'s warn arm: needs a probe result carrying a diagnostic, which only the Windows body (`sandbox/mod.rs` **1198-1201**) can return — the non-Windows body is **1202-1207** (`unavailable_without_error`, no diagnostic). Verified segment 11 |
| `rpc/commands/settings.rs` 53, 54, 55, 56 | 4 | platform-unmeasured ✔ | `reset_windows_sandbox_capabilities` returns `Err(Unsupported)` from its non-Windows body (`sandbox/mod.rs` **1227-1233**), so the `?` propagates first; only the Windows body (**1223-1226**) can produce the `Ok` this arm renders. Verified segment 11 |
| `rpc/commands/settings.rs` 513, 514, 515, 516, 517, 520, 521, 522, 523, 524, 525, 526, 527 | 13 | platform-unmeasured ✔ | the probe-`Err` arms (the `tier == "sandbox"` refusal at 513-518 and the graceful fallback at 520-527) of `handle_set_sandbox_policy`: same dependency as the 21-24 row above — `platform_sandbox_probe_product()` cannot fail outside its Windows branch (`sandbox/mod.rs` 1073-1075). Verified segment 11 |
| `rpc/commands/settings.rs` 535, 536 | 2 | unreachable-in-this-environment ✔ | needs a macOS host where `Path::new("/usr/bin/sandbox-exec").exists()` is false — the probe result is hard-coded to the system path (`sandbox/mod.rs` **1057-1058**), so no fixture can produce `available = false` here. **Verified segment 11:** `/usr/bin/sandbox-exec` exists on this host (`-rwxr-xr-x root wheel`), so the fallback arm is genuinely unreachable here |
| `rpc/commands/settings.rs` 299, 300, 301, 302, 303, 304, 305 | 7 | unreachable-in-this-environment ✔ | the `thread::Builder::spawn` failure arm of `/compact` (the OS must refuse a new thread); 191 (the handler's own background-admission guard) is now covered by a direct call, §0.3 segment 3 |
| `rpc/mod.rs` | — | covered by test ✔ | lines 179-180 — `hydrating_onto_a_replacement_model_survives_a_failed_write` (segment 4) |
| `rpc/mod.rs` | — | covered by test ✔ | lines 168, 171 — `hydrating_a_session_whose_model_is_gone_keeps_an_unresolved_identity` (§0.3 segment 3): an unavailable model with no replacement keeps its identity and the failed write is logged instead of failing the hydrate |
| `rpc/mod.rs` 241 | 1 | attribution-artifact ✔ | the `broadcaster_owner` **field line** of `create_session`'s foreign-journal `tracing::error!`, and the cleanest F-14 evidence in the unit: the macro ran **2** times (line 239 = 2) while 241 reads **0** and 240, 242, 243 have no `DA` record at all. Segment 11's `create_session_replaces_a_foreign_broadcaster_and_reports_a_failed_rebind` drives the arm on purpose and the field line still does not move |
| `rpc/mod.rs` | — | covered by test ✔ | line 247 — the *failed rebind* log of the same path, by the same test's second half: a session whose legacy import was skipped cannot bind an event journal (`bind_events` bails on the present-but-skipped row), so `configure_journal` fails and the replacement broadcaster is reported as unconfigured without failing the registration. Measured: 247 = 1 |
| `rpc/mod.rs` 353, 354 | 2 | attribution-artifact ✔ | the two *field-expression* lines of the `tracing::warn!` that reports "no configured replacement model". The arm demonstrably ran: `351` = 17, `352` (the macro's opening line) = 11 and `357` (the `continue` after it) = 11, while 353/354 read 0 and 355/356 have no `DA` record at all. Segment 8's `provider_reconcile_keeps_a_model_less_session_when_no_replacement_exists` was written to try to flip them — it forces an explicitly empty registry (`Registry::new()` is never empty, it reads disk state) and the arm fires 11 times, and the field lines still read 0. Same class as `manager/mod.rs` 150 and `run_control.rs` 250 (F-14) |
| `rpc/mod.rs` | — | covered by test ✔ | lines 374, 375, 380 — the *non-busy* `set_model` failure arm and the `break` that stops the retry loop, by `provider_reconcile_reports_a_replacement_whose_protocol_cannot_be_built` (segment 10): a single registered model whose `api` is unknown makes `ResolvedModelTarget::from_model` fail, so `set_model` reports a real error rather than "busy" and the session keeps its old model. Measured with a single-target run: 374 = 1, 375 = 1, 380 = 1 |
| `rpc/mod.rs` 376, 388, 390 | 3 | attribution-artifact ✔ | 376 is the second *field-expression* line of that same `tracing::error!`: the segment-10 probe shows 375 = **1** while 376 = **0** in the same arm, which is the F-14 shape measured *within one macro*. 388/390 are the closing braces of the retry loop whose body is covered |
| `rpc/mod.rs` 1514, 1519, 1523, 1560, 1565, 1569 | 6 | unreachable-by-construction ✔ | `panic!` arms inside the two drain tests: they fire only if `create_session`/`publish_session_created` stops announcing, which the same test asserts cannot happen; any input that reaches them fails the test in the *other* arm first. Renumbered three times as tests were added to this module: 1280/… → 1381/… (+101, segment 8) → 1436/… (+55, segment 10) → 1514/… (+78, segment 11) |
| `service/shutdown.rs` 93, 94, 95, 96, 112, 113 | 6 | unreachable-in-this-environment ✔ | the two `thread::Builder::spawn` failure arms (deadline watcher, supervisor): need the OS to refuse a thread |
| `service/shutdown.rs` 106, 107, 108 | 3 | unreachable-in-this-environment ✔ | `catch_unwind` returning `Err` — the drain itself panicking; a bug-injection fixture, not an input |
| `service/shutdown.rs` 131, 132, 133, 134 | 4 | unreachable-by-construction ✔ | finding 4: the `watch::Sender` lives inside the `Arc` the method borrows for the whole call, so `changed()` cannot report a dropped sender |
| `service/shutdown.rs` | — | covered by test ✔ | line 232 — the run/native-work stage exit, by `an_expired_deadline_reports_the_stage_that_never_settled` (segment 10): a background registration that is never released keeps `counts()` non-zero, so that stage cannot settle and a 50 ms deadline ends the shutdown there. The report is terminal and names the stage. **The watcher publishes before the drain observes it**, so the test sleeps a bounded 200 ms for the supervisor's own turn — without that, the line is a race with the test rather than part of it |
| `service/shutdown.rs` | — | covered by test ✔ | line 303 — the RPC-listener stage exit, by the `shutdown-without-polling-server` fixture (segment 10): a host that starts serving and then neither polls nor drops `serve` leaves `server_running` true, so that stage cannot settle and the deadline is what ends the shutdown; the error names the stage and the scope retains instance ownership |
| `service/shutdown.rs` | — | covered by test ✔, **load-dependent** (lines **273** and **303**) | the drain's deadline exits for the scheduler/monitor stage (273) and the RPC-listener stage (303), each with a fixture that makes its own stage unsettleable (`an_undriven_scheduler_worker_times_out_the_monitor_stage`, `shutdown-without-polling-server`). **Corrected in segment 14: this pair is load-dependent, not mutually exclusive.** The three runs are 273 = 0 / 303 = 1 (segment 12), 273 = 1 / 303 = 0 (segment 13) and **both = 1** (segment 14) — so segment 13's finding that "exactly one reads 0 every run" was too strong: what is true is that *which* of the two the deadline interrupts depends on scheduling, and any single run may show either, both or neither. Both are covered in this revision's report. A future run showing one at 0 is the expected consequence of that, not a regression, and the row is kept out of the claim cell for exactly that reason |
| `service/shutdown.rs` | — | covered by test ✔ | line 237 — the ephemeral skip in the queued-run terminal pass, by `an_ephemeral_sessions_cancelled_queue_is_not_written_as_a_terminal_row` (segment 9): an ephemeral session holding a queued run shuts down to `Stopped`, because writing that terminal row would fail against a session with no persisted transcript |
| `service/shutdown.rs` | — | covered by test ✔ | lines 253-257 — `a_storage_failure_while_persisting_cancelled_runs_is_reported` (segment 4): a trigger that refuses only the cancelled run's terminal marker |
| `service/shutdown.rs` | — | covered by test ✔ | lines 281-284 — `a_session_holding_an_unresolved_run_reports_a_failed_stop` (segment 4) |
| `service/shutdown.rs` | — | covered by test ✔ | lines 290 — `a_recorded_persistence_error_is_reported_by_the_drain` (segment 5) |
| `service/shutdown.rs` | — | covered by test ✔ | line 293 — `broadcaster.shutdown_journal()` failing, by `a_dead_storage_worker_is_reported_by_the_journal_and_storage_stages` (segment 10). The recipe is worth keeping: append a **buffered delta** (`tool_delta`/`text_chunk`/… — `durable = !is_buffered_delta(..)`, so it carries no reply and stays queued), then make `store.append_events` fail by killing the SQLite worker with a panicking job of its own (`db.call(|_| panic!(..))` inside `catch_unwind`). The journal's shutdown flush then has a non-empty pending batch it cannot write, so `shutdown_journal` returns `Err` instead of `Ok` — an empty batch short-circuits to `Ok` and would not reach the arm |
| `service/shutdown.rs` | — | covered by test ✔ | line 307 — `session_manager.shutdown()` failing, by the same test: `Database::shutdown` joins the SQLite worker, so a worker that panicked turns the close into `Err("SQLite worker panicked during shutdown")`. This is the storage-level sibling of the writer-failure test and it depends on segment 7's finding that a panicked job leaves the worker dead |
| `service/shutdown.rs` | — | covered by test ✔ | lines 153, 187 — `wait()`'s "deadline already reported" return and the drain's early exit: reached by the expired-deadline test once it holds the admitted work past one poll interval of the wait loop. Confirmed with a single-target instrumented run (`cargo llvm-cov -p future-agent --no-report --lib -- service::tests` then `cargo llvm-cov report --lcov`) showing counts 2 and 1 on those lines — the first version of the test dropped the last piece of work too early, which is why they read 0 before |
| `rpc/commands/session_reset.rs` | — | covered by test ✔ | lines 104-108 — `a_reset_whose_history_moved_is_reported_as_history_changed` (segment 4), via the deterministic `RESET_RACE_HOOK` — the race is not waived, it is forced (F-5) |
| `rpc/commands/session_reset.rs` 133, 134, 135, 136, 137 | 5 | unreachable-by-construction ✔ | `nothing_retained`: the handler only runs on a hydrated session, and hydration always writes the `session_info` record entry the message is about — the repo's own test says so (`a_session_with_no_record_entry_refuses_to_be_emptied`), and the storage-level refusal is the one this unit's tests drive |
| `rpc/commands/session_reset.rs` | — | covered by test ✔ | the `reload_failed` block (now lines 168-173) — the post-apply `ensure_history_loaded` failing after the store accepted the reset — by `a_reset_that_applies_but_cannot_be_reloaded_reports_reload_failed` (segment 23). The damage lands in the window **between** the store accepting the rewrite and the in-memory history being reloaded from it, through the handler's new id-gated `RELOAD_FAILURE_HOOK`; the test asserts the reset *did* apply (the backup records 10 entries kept to 6) while the response is `reload_failed`/retryable with the backup path. See F-53 for why no input can reach this arm |
| `rpc/commands/session_reset.rs` | (removed from this row) 101-103, 110-111, 118-123 | — | covered by test ✔ | `a_reset_that_the_store_refuses_is_reported_as_reset_failed` (§0.3 segment 3): a store that refuses the rewrite reports `reset_failed`/retryable, leaves the transcript untouched and keeps the pre-reset backup |
| `rpc/session.rs` | — | covered by test ✔ | lines 1332 and 1343 — the default-split fallback and the object-style `data` row — by `a_session_whose_pricing_journal_is_unreadable_opens_with_a_default_split` and `the_replay_reads_object_style_data_and_ignores_unpriced_event_types` (segment 7) |
| `rpc/session.rs` | — | covered by test ✔ | lines 381, 668, 673, 674 — `on_scheduled_run_finished_stops_dequeuing_once_work_is_closed` (segment 9) leaves a queued run in place once the work boundary closes, and `compaction_events_are_published_to_the_scope_their_phase_belongs_to` covers the run-scope broadcast arm (673) and the no-wire-projection drop (668) by driving `broadcast_compaction_event` directly with each phase |
| `rpc/session.rs` 918, 969, 1223 | 3 | attribution-artifact ✔ | closing braces of covered blocks, measured one by one in segment 15: 918's block reads **1/1/1/5** on 914-917 with 919 at **23**; 969's reads **1/1/abs/1** on 965-968 with 970 at **4**; 1223's reads **8/8** on 1221-1222 with 1224 at **15** |
| `rpc/session.rs` | — | attribution-artifact ✔, **flipped** (line 1217) | the `}` closing `if status.is_none() {` in the shell-timeout loop. The header at `1215` and the assigned statement at `1216` read **25** each in this report, and the brace read **2** in the previous two runs (a flip between reports, not a change in behaviour). Nothing can make the brace's own region newly executed, so the artefact label is the accurate one (F-15) |
| `rpc/session.rs` 1371 | 1 | unreachable-by-construction ✔ | the `_ => {}` arm of the pricing-replay match. **Proven dead by two independent filters:** the read side selects `json_extract(payload,'$.event_type') IN ('usage','model_changed')` (`sqlite_store/events.rs` 56-58), and the snapshot side only stores payloads whose decoded event matches `"usage" | "model_changed"` (`journal_compaction.rs` 212-221) — so no payload reaching this loop can carry another event type. Measured: the enclosing match ran, and 1373-1375 read **43/44** |
| `rpc/session.rs` 979 | 1 | unreachable-by-construction ✔ | **reclassified from `coverable` in segment 22, on a measurement rather than an argument.** It is the `_ => None` arm of the manual-compaction summary join (`checkpoint.summary.iter().filter_map(..)`): `ContextCheckpoint::summary` is a `Vec<ContentBlock>`, and a non-Text block in it is a **legitimate, already-tested shape** — `compaction::tests::checkpoint_projection_ignores_non_text_summary_blocks` (`compaction/mod.rs` 715) builds exactly `[Text, Reasoning]` for a *different* consumer of the same field. Reaching **this** line would need such a checkpoint to flow through `ServerSession::compact`. **It cannot**: the summarizer ends in `vec![ContentBlock::text(summary)]` (`compaction/semantic.rs`), so every summary the preparation hands over is text-only, and the closure's `Text` arm is the only one it can take. Measured two ways in segment 22: the loop had run **14** times in the previous report and never matched `_`, and the new compaction test now asserts the invariant directly on the committed checkpoint (`session.agent_loop.active_checkpoint`) — all blocks are `Text`. That assertion is the falsifier: if the engine ever produces a non-text summary block, it fails and this row must go back to `coverable` |
| `rpc/session.rs` 1338 | 1 | unreachable-by-construction ✔ | the `continue` for a pricing payload that will not parse: the row's `payload` column carries `CHECK(json_valid(payload))`, so a stored row always parses into `serde_json::Value`. Malformed *event data* is caught later (line 1346, covered by `a_journal_with_rows_it_cannot_read_still_prices_the_rows_it_can`) |
| `rpc/session.rs` 885, 886 | 2 | attribution-artifact ✔ | the `cache_read`/`cache_write` **field-expression** lines of the *manual-compaction* `tracing::info!` at 881. Measured segment 12: that macro runs **6** times while its fields are 885 = 0, 886 = 0 and 882-884, 887-888 have **no `DA` record at all** — the fields sit outside the instrumented region, so no level or subscriber can move them (F-14). **This row's earlier mechanism was wrong** (it read "a disabled DEBUG callsite does not evaluate its fields"): the macro is an `info!` that demonstrably runs, so the old reason would not survive the reviewer's falsification even though the category does |
| `rpc/session.rs` | — | covered by test ✔ | line 1015 — the warn for a checkpoint whose usage-baseline write failed — by `compact_reports_a_checkpoint_whose_usage_baseline_cannot_be_persisted` (segment 22): a manual compaction whose baseline persist is injected to fail. The pair of assertions is what makes it the *baseline* write and not an earlier one: the checkpoint still commits, the in-memory baseline is zeroed, and the stored entry has no `last_prompt_tokens` key at all (a successful write would have added it as `0`) |
| `rpc/session.rs` 2567 | 1 | platform-unmeasured ✔ | the PowerShell `Start-Sleep` branch of the shell timeout helper (Windows). Renumbered from 2484 (+83: segment 9's two tests lengthened this module) |
| `rpc/session.rs` 3497, 3713, 4974, 5011, 5147, 5426 | 6 | unreachable-by-construction ✔ | **all six are `assert!` failure-message arguments**, i.e. the expressions *after* the first comma of an assertion: 3497 `session.broadcaster.persistence_error()`, 3713 `result["summaryOutcome"]`, and 4974/5011/5147/5426 `*session.cumulative_cost{,_split}.lock()`. Rust expands `assert!(cond, fmt, args…)` to `if !cond { panic!(fmt, args…) }`, so those arguments are evaluated **only when the assertion fails** — a green suite cannot reach them, and "reaching" them would mean the test failed. **Reclassified from `coverable` in segment 13, correcting a wrong mechanism** (the old row said "each needs one more end-to-end run fixture", which is not achievable and would have cost a future segment its budget). Finding F-29, and the same shape as the fixture-body `panic!` rows already carried as unreachable-by-construction (`session/fork.rs` 485/500). Renumbered twice: from 3414/3531/4792/4829/4965/5244 (+83, segment 9), then +99 below 3590 when segment 22 added a compaction test above them |
| `rpc/session_prompt.rs` | — | covered by test ✔ | lines 819-829 — `an_automatic_reviewer_policy_reviews_a_sandboxed_run_without_asking_the_user` (segment 6), measured `DA:count 1` on all eleven lines. **F-7 was falsified**: the earlier "unreachable / hangs" was the fixture's non-reentrant HOME lock plus a plain (auto-allowed) shell call |
| `rpc/session_prompt.rs` | — | covered by test ✔ | lines 1178 and 1617 — `creator_id` copied into the session-info payload on **both** persistence paths (the end-of-turn commit task inside `prompt_internal`, and `persist_user_message` at the run's start) — by `a_run_records_the_creator_identity_on_both_persistence_paths` (segment 21): one run on a session whose `creator_id` is set. Each site is gated on the identity being non-empty, so neither had ever executed; the existing provenance test sets `cmd.creator_id` and asserts the in-memory session, and nothing ran a prompt on one. Measured **1** each |
| `rpc/session_prompt.rs` | — | covered by test ✔ (file now at zero) | line 1357 — the "could not drain the event journal before compaction" warning — by `a_failed_journal_drain_before_compaction_still_finishes_the_run` (segment 22). The guard above it reads the journal's **health**, so the warning needs a drain that fails while the journal is still healthy; every way a live drain fails (a dead worker, a store that cannot commit the batch) makes an *append* fail first and marks health, which takes the guard's other branch. See F-50 for why the error is injected and what proves the arm is reachable in production |
| `rpc/broadcaster/journal_writer.rs` | — | covered by test ✔ | lines 470-477 and 484-486 — the per-event retry loop's storage arms, the requeue of a reply-less event, and the "batch stays failed" exit — by `a_refused_batch_retries_singly_and_separates_refusal_from_storage_failure` (segment 7) |
| `rpc/broadcaster/journal_writer.rs` | — | covered by test ✔ | lines 56-58 — the `AppendFailure::unavailable` constructor — by `an_accepted_durable_append_reports_unavailable_when_the_worker_dies_before_answering` (segment 20). The **reply sender lives inside the queued command**, so an acknowledgement is abandoned by dropping the accepted command, which is what the fixture does with a hand-held channel: it receives the `Append`, drops it, and the appender's `recv` fails. A live worker cannot be used, because both ways it stops (the `Shutdown` handshake and `Drop`) flush pending events and answer them first |
| `rpc/broadcaster/journal_writer.rs` | — | covered by test ✔ | lines 161, 162 — the `recv` arm that reports an accepted append as unavailable — by the same fixture: measured **1** execution each, reached only after the `send` at 153 had succeeded (the test asserts the command is in the queue before dropping it) |
| `rpc/broadcaster/journal_writer.rs` | — | covered by test ✔ | line 180 — `return Ok(())` for a writer whose worker is already gone, by extending `event_batch_writer_refuses_appends_when_unhealthy_or_when_its_worker_is_gone` (segment 14) with a `shutdown()` after `stop_worker_for_test()`: retirement is idempotent, so a drain that meets an already-retired writer must not report a failure. The row had carried it as "an early return", which undersold a reachable line — the condition at 179 runs 19 times and 180 read 0 only because no test shut a retired writer down |
| `rpc/broadcaster/journal_writer.rs` 194 | 1 | attribution-artifact ✔ | the closing brace of `if let Some(thread) = self.thread.take()`: 190-193 read **19** each and the brace reads 0 |
| `rpc/broadcaster/journal_writer.rs` 322 | 1 | attribution-artifact ✔ | **measured in segment 15:** the `}` closing the `if let Some(reply)` block inside the "refused because the journal is failed" arm — 318-320 read **1** each, 323 reads **1** and 324 reads **9814**. F-15's shape; the enclosing arm demonstrably ran |
| `skills/manager.rs` 413, 414 | 2 | unreachable-in-this-environment ✔ | the *restore* arm of the install rollback, with the proof: reaching 413 needs `backup.exists()` at 412, which is only true if line 409's `rename(dest, backup)` already succeeded — that requires the install root to be writable and on the same device as the staging directory, which leaves only `ENOENT` on the staged candidate the function itself creates and holds. The sibling decision is covered: `a_failed_publish_drops_the_pending_operation` (segment 8) makes the publish rename fail with a read-only install root, taking 411, 412, 416 and 417 and asserting nothing is left pending (§0.4 F-16) |
| `skills/manager.rs` 177, 265, 404, 430, 434, 497, 499 | 7 | attribution-artifact ✔ | **all seven are closing lines of covered multi-line statements**, measured line by line in segment 14: 177 `)?;` closes a `prepare` whose 174-176 read **36** each; 404 `)?;` closes an `execute` whose 400-403 read **12** each; 265, 430, 434, 497, 499 are `}` closing `if` blocks whose bodies read **3**, **1**, **1**, **1**, **1**. The row's own mechanism ("`?`-tails and closing braces … forcing the underlying call to fail covers them") described an artifact while claiming `coverable` — the same defect F-25 found, caught this time by extending the audit to artifact vocabulary. Renumbered from 1736/1737/1738-era numbering (+40) |
| `skills/manager.rs` 1776, 1777, 1778 | 3 | **unreachable-by-construction** ✔ — **corrected from `attribution-artifact`** | the `flush` of the test-local `FailingConnection` double. **The invariant:** that double is private to `#[cfg(test)] mod tests`, its only consumer is `serve_connection` (1682), and `serve_connection` calls `read` and `write_all` — **it never calls `flush`** (neither does any other caller, in test or production code: the production connection at 1670 is a `TcpStream`, whose `flush` is a no-op that nothing asks for either). So no input reaches the body, and the `impl Write` must supply `flush` for the bound, so the lines cannot be deleted either. Measured in this rework's own report: 1776 = 0, 1777 = 0, 1778 = 0 while the `write` half carries the counts (1763-1774 = 1 each, 1775 = 2 — the `write_all` that the streaming arm calls). **Why the artifact label was wrong (finding F-3 of `review-agent.md`, accepted here):** `attribution-artifact` means "llvm-cov attributes the line to a macro or a closing brace" and is admissible only *with the enclosing statement's counts*; here the neighbours are covered because the statement is covered, and the cited region is exactly where the unexecuted code is — a never-called method, not an attribution offset. **The same shape and the same reasoning already carry the same category elsewhere in this campaign:** `module-agent-core.md`'s `sandbox/linux/violation.rs:285-287` row (a fixture `Closed` writer's `flush`, "production `write_marker` writes and returns without ever flushing, so nothing can call it"). Named invalidator: **any** call to `flush` on that double — one added fixture line, or a new statement in `serve_connection` — makes all three lines coverable, and this row must then be re-derived. Renumbered from 1736/1737/1738 (+40, segment 8's test) |
| `skills/manager.rs` 1934, 1939 | 2 | unreachable-by-construction ✔ | **F-29, second instance — and the ledger's own mechanism gave the game away:** these are `listed.err()` and `catalogue.err()` **inside the failure-message arguments** of `assert!(listed.is_ok(), "…", listed.err())` and its `catalogue` twin. Rust expands `assert!(cond, fmt, args…)` to `if !cond { panic!(fmt, args…) }`, so those expressions are evaluated only when the assertion **fails**; the suite is green, so they read 0 forever. The old text called them "the error branch of a catalogue listing", which reads like reachable work. Reclassified in segment 15 by reading the two lines above them. Renumbered from 1894/1899 (+40) |
| `approval_review/provider.rs` | — | covered by test ✔ | lines 133, 136 — `gateway_statuses_bodies_and_transport_failures_map_to_fixed_codes` (segment 5). They were previously labelled attribution artifacts — a claim the segment-5 run then falsified, which is exactly the kind of stale waiver this ledger is meant to surface |
| `approval_review/provider.rs` | — | covered by test ✔ (the file's last *coverable* line; 188 stays unreachable-by-construction) | line 181 — the `is_timeout()` arm of the transport classification — by `a_gateway_that_never_answers_is_classified_as_a_reviewer_timeout` (segment 25): a gateway that accepts the connection, reads the request, and never answers. **The 30 s cost this row carried for five segments turned out not to be a cost at all** (F-61): the client timeout is a `tokio` timer, so a `#[tokio::test(start_paused = true)]` runtime auto-advances it to firing the moment every task is idle — the test runs in **0.01 s**, not 30. Two things made this hard to see earlier and are worth keeping: (a) `lifecycle.rs` already has a passing test named for `reviewer_timeout`, but that exercises the **context** deadline (`context.rs`'s `remaining = REVIEW_TIMEOUT - elapsed`), which is a different line — an F-44 trap that made the row look covered from a distance; (b) the direct `assess_at` call used here is what leaves the client's own timeout as the only deadline. Measured 179/180/181 = 1 with 183 (the `reviewer_connection` sibling) = 0, so the timeout branch ran and the connection branch did not. The row's earlier text follows for the record: the `is_timeout()` needs the request/connect deadline to elapse, and a non-routable address fails with `is_connect()` instead. Recipe: point the endpoint at a blackhole address and accept 30 s, or add a seam that takes the client timeout as a parameter |
| `approval_review/provider.rs` 188 | 1 | unreachable-by-construction ✔ | the loop's fall-through `Err("reviewer_http")`, dead because **every arm of the match returns on the last attempt** (`for attempt in 0..=1`, and the two `continue`s are both guarded by `attempt == 0`). Measured support: the existing double-5xx fixture (`500` then `502`, `tests/provider.rs`) asserts exactly this string and passes, while the counts show `176` = **2**, the `"reviewer_http"` literal at `179` = **1**, and **188 = 0** — the identical observable comes from the other line, so only the counts separate them (F-34/F-39 shape). A test asserting the message alone would look like it covered this line |
| `approval_review/provider.rs` | — | covered by test ✔ | lines 63, 157, 169-180, 183 — `gateway_statuses_bodies_and_transport_failures_map_to_fixed_codes` (segment 5): the whole classification matrix against a scripted local gateway |
| `session/history/paging.rs` | — | covered by test ✔ | lines 36 — `a_damaged_display_index_is_reported_instead_of_paging_a_blank_history` (segment 4) |
| `session/history/paging.rs` | — | covered by test ✔ | **all seven in segment 16 — this file is now at zero uncovered lines.** 128/129/131/133/134 (the cut at an entry too large for the page) by `a_page_of_bodyless_rows_can_be_cut_at_the_materializer_budget`: eight 1_048_570-byte index rows are admitted by the page reader (its raw budget, 8·1_048_570 = 8_388_560 ≤ 8_388_608) and then cross the *materializer's* check on the eighth row, which also counts separators and the page envelope — forward takes `next = ordinal` and backward `next = ordinal + 1`. **The two budgets are the finding:** the cut is only reachable when a row's indexed payload is its whole contribution, i.e. when the display row has **no entry body** — and `history_display.source_position` is nullable precisely for that case, while a row *with* a body contributes `overlay + body` to the raw count but only its projection to the materializer, so its materialized size is about half. 145/146 (the UTF-8 boundary walk before slicing a fragment) by `a_resumed_fragment_stops_on_a_character_boundary`: the text is one repeated three-byte character, so `offset + ENTRY_FRAGMENT_BYTE_BUDGET` lands mid-character for any character-start `offset` (1048576 mod 3 = 1), the walk steps back, and the fixture asserts the two fragments are adjacent slices so a client can concatenate them |
| `session/history/recall.rs` | — | covered by test ✔ | lines 41 — `the_session_gate_reports_each_damaged_table_it_needs` (segment 4) |
| `session/history/recall.rs` 92, 227, 247 | 3 | attribution-artifact ✔ | `?`-tails of prepared statements that execute (F-6 shape); the damaged-schema test drives the failing statement in the sibling query |
| `session/history/recall.rs` 179 | 1 | attribution-artifact ✔ | the `}` closing `if let Value::Object(fields) = entry` in the search-result assembly. **The mechanism recorded here was wrong** — it described snippet assembly elsewhere — and the neighbours settle it: `174` = **10**, `175`/`176` = **10**, `177` = **2**, `178` = **8**, `181` = **14**, while the brace reads 0. Nothing can make a brace's own region newly executed, so the artifact label is the accurate one and this line is not work (F-15) |
| `session/history/display_index.rs` 64 | 1 | unreachable-by-construction ✔ | the `_ => None` arm of the `tool_result` pairing: a block that passed the `b["type"] == "tool_result"` filter always deserializes into `ContentBlock::ToolResult`, so the other arm is dead for it |
| `session/history/display_index.rs` 160 | 1 | attribution-artifact ✔ | the closing brace of the `refresh_session_metadata` update; 156-159 are covered by the damaged-schema test |
| `session/legacy_import.rs` | — | covered by test ✔ | line 20, the `matches!` body of `source_unreadable`, read 4 before and reads 0 now: reaching the generic read failure requires evaluating it on an error kind it rejects, which `an_unaddressable_source_path_reports_the_read_failure` (segment 8) does — so the earlier attribution-artifact reading is withdrawn |
| `session/legacy_import.rs` 397 | 1 | attribution-artifact ✔ | `)?;` tail whose statement executes: 395-396 read 775 each |
| `session/legacy_import.rs` | — | covered by test ✔ | lines 72, 322 — `an_unaddressable_source_path_reports_the_read_failure` (a NUL path cannot be addressed, and that io error is neither missing nor unreadable, so `read legacy source` propagates) and `a_run_event_directory_that_cannot_be_enumerated_reports_its_context` (a self-referential link yields ELOOP, which `source_unreadable` does not classify, so `enumerate legacy events` propagates); both segment 8 |
| `session/legacy_import.rs` 88 | 1 | unreachable-by-construction ✔ | `Err(ParseFailure::Cancelled) => unreachable!()` in the `#[cfg(test)]` wrapper `revalidate_sources`, which always passes a fresh, uncancelled token — so the arm is unreachable by construction (the author says so) |
| `session/legacy_import.rs` 351 | 1 | unreachable-in-this-environment ✔ | `return Err(StartupCancelled)` when the parse reports cancellation. Every deterministic cancellation point precedes it: the fingerprint loop checks at 326 and 337 first, so with a non-empty source list reaching 351 means the token flipped in the nanoseconds between the fingerprint (341) and `parse_sources`' first check (422). There is no seam, and a spinning test would be hope rather than evidence |
| `session/legacy_import.rs` 290 | 1 | attribution-artifact ✔ | `index = index + 1` is a **field-expression line of the `tracing::info!` at 288**, which ran **54** times while 290 reads 0 and 289/291-293 have no `DA` record at all — the F-14 shape, and the row's earlier reading ("the arm needs an event that advances the index across a page boundary") was wrong because the macro demonstrably runs. Measured segment 14 |
| `session/fork.rs` 118, 142 | 2 | attribution-artifact ✔ | the closing braces of two `if` blocks in the run-id rewrite: 114-117 read **25/25/25/39** and 138-141 read **4** each. Measured segment 14 |
| `session/fork.rs` 205, 384 | 2 | attribution-artifact ✔ | **measured in segment 15** (they were the row left "not yet measured" in segment 14): 194 (the `}` of the run-id rewrite block) **is covered by segment 24's fork fixture**, which walks that block; 205 is the `}` closing the checkpoint-summary block and 384 is the `}` of the run-id rewrite block whose 192 reads **5** and whose 195 reads **5**; 205 is the `}` of the summary-rewrite block whose 201-204 read **4** each with 206 at **4**; 384 is the `}` of the fork-point check whose 381-383 read **4/1/3** with 385 at **3**. All three are F-15's shape (the block body runs, the brace does not) |
| `session/fork.rs` | — | covered by test ✔ | line 202 — `return false` when a checkpoint summary's evidence references cannot be remapped — by `fork_drops_a_checkpoint_whose_evidence_references_cannot_be_remapped` (segment 24): a v3 checkpoint whose range lies inside the fork but whose evidence index names an entry the fork does not carry. The test asserts the *decision* (the checkpoint is dropped while the rest of the fork survives), and the probe shows 201 = 1 with 204 = 0 — the summary branch ran, and it was the remap that refused, not an earlier `return false` (180, 186 and 199 all read 0). **The recipe segment 17 left here is kept below because it is what the fixture was built from.** Recipe:  the call is `crate::compaction::remap_evidence_references(&mut blocks, &id_map)` and it returns `None` — making the caller refuse the fork — on any of six `?`s in `semantic/evidence.rs` 62-79: a text block that is not a `[Context compaction` header, a missing `Coverage cutoff:` line, a cutoff that is not JSON, **a cutoff id absent from `id_map`**, a row that is not JSON, or a row without `entryId`. The cheapest is the bolded one: build a checkpoint entry whose `summary` text carries a valid header and a `Coverage cutoff: "<id>"` naming an entry the fork does not carry. `SessionEntry::compaction` plus the v3 `protected_entry_ids` shape (see `compaction/mod.rs` 715 and `legacy_import.rs`'s `checkpoint_line`) is the fixture skeleton |
| `session/fork.rs` 485, 500 | 2 | unreachable-by-construction ✔ | `panic!("checkpoint expected")` / `panic!("text expected")` inside a test fixture builder: they fail the test if the builder's own shape changes |
| `session/database/mod.rs` 65, 133 | 2 | attribution-artifact ✔ | the closing braces of the worker's `match receiver.recv_timeout(..)` and of `call`: neighbours 63-64 read 1285 each and 135 reads 44, 134 reads 43, while the brace reads 0 (F-6/F-12 shape) |
| `session/database/mod.rs` | — | covered by test ✔ | line 119 — a dead worker is refused up front — by `a_panicking_job_leaves_later_callers_with_a_disconnected_worker` (segment 7) |
| `session/database/mod.rs` | — | covered by test ✔ (lines 79-82; **was a load flip, now deterministic**) | the idle-reclamation failure arm's body, by `the_idle_reclaim_reports_a_lock_it_cannot_wait_for_and_then_recovers` (segment 24): a foreign `BEGIN IMMEDIATE` on the store's file while the worker's 50 ms idle slice fires. `reclaim_space_when_idle` sets `busy_timeout(0)`, so `incremental_vacuum` returns `database is locked` and the worker takes the `Err` arm — measured **4** executions with the once-only report at 81/82, so it no longer depends on load (F-54 was the flip; this is its fix). The fixture asserts what the arm protects: the worker survives and reclaims once the lock is released |
| `session/database/mod.rs` | — | covered by test ✔ | line 83 — the closing brace of `if !reclaim_error_reported {` — by the same fixture (measured **3**): with the arm running four times in one slice, llvm-cov attributes the brace's region too, which is what makes it a coverage claim rather than an attribution artifact. **This is the one place in this ledger where a brace line moved from artifact to covered because the surrounding block began to execute more than once**, and it is worth watching: the same could be true of the other brace rows if their blocks are ever entered repeatedly |
| `session/database/mod.rs` | — | (superseded — the old claim was lines 79-83; see the two rows above) | the idle-reclamation failure arm (the first failure reports at 79-81, later ones stay quiet via `reclaim_error_reported`). **Reclassified from `coverable` in segment 12, and the reason is measured rather than argued.** The arm needs the worker's *own* connection to fail: 73 (`if worker_reclaim.load`) runs **85169** times and the success arm runs **235** times (74, and 75-78 at 235 each), while 79-83 read 0 — so the reclaim path is exercised constantly and never fails. Damage from outside cannot reach it, because SQLite serves the cached page 1 to an already-open connection: the probe `a_replaced_database_file_is_invisible_to_an_already_open_connection` (§0.3 segment 12) truncates the file and removes the WAL that still holds a good page 1, and the live connection still answers `freelist_count` (returning 0) — `SELECT 1` never touches the file at all. Two damage routes tried; both fail for that documented reason |
| `session/database/connection.rs` 19, 42, 52 | 3 | attribution-artifact ✔ | `)?;` tails whose statements execute: 17-18 read 892 with 20-21 at 880-887 (line 19); 40-41/43 read 837 (line 42); 50-51/53 read 2 (line 52). **Failure-driven evidence**: the damaged-schema tests make exactly these calls *fail* and assert the error, and the tails still read 0 (F-12) |
| `session/database/reclamation.rs` 17 | 1 | attribution-artifact ✔ | the closing brace of the reclaim call: 15-16 read 53 each |
| `session/database/reclamation.rs` 46, 80 | 2 | unreachable-in-this-environment ✔ | the two `break`s that stop incremental reclamation when a slice frees nothing. **Ten candidate states were probed and none produced it** (`database/reclamation_arm_tests.rs`, segment 19): a deleted filler table, free pages in the middle with a live tail, an open cursor on the same connection, a foreign read transaction, the connection's own write transaction, free pages created inside it, a `DROP TABLE`, `auto_vacuum=2` requested on a populated database (and after reopening it), and `auto_vacuum=1`. This build frees **exactly one page** per `incremental_vacuum(1)` or fails with `database is locked`; it never reports no progress while returning Ok. Neighbours: 43/44/45 read **113** each (the slice ran 113 times) while 46 reads 0, and the test-helper twin at 80 has 77/78/79 at **6441**. The shipped test asserts that invariant, so a future SQLite build which starts reporting no progress fails it — which is exactly when this row must become coverable |
| `session/sqlite_store/events.rs` 277, 350, 358 | 3 | attribution-artifact ✔ | the `?`-tail of a statement whose own call executes: 275-276 and 278 read 135809 (line 277), 348-349/351 read 215 (line 350), 356/359-360 read 215 (line 358). The damaged-schema tests make exactly these calls fail and assert the error text while the tails stay 0 (F-12) |
| `session/sqlite_store/events.rs` | — | covered by test ✔ | line 64 — the pricing projection's `?` tail — is covered in the current report (the segment-7 tests make that statement fail as well) |
| `session/sqlite_store/events.rs` | — | covered by test ✔ | line 369 — the pager's `break` when the next event would push a page past `PAGE_BYTES` (4 MiB). Segment 18's compaction fixtures cross it: each event's *stored* payload is ~2.33 MiB, so a page holds one and the next one breaks. Measured: 369 = 3 |
| `session/sqlite_store/events.rs` | — | covered by test ✔ | lines 144, 204 — `every_event_reader_reports_the_table_it_cannot_reach` (segment 4) |
| `session/sqlite_store/transcript.rs` 177, 221, 323 | 3 | attribution-artifact ✔ | same `?`-tail shape: 174 = 463 with 177 = 0 and 178 = 463 (same at 221 and 323); the damaged-schema test drives the failing statement in each case |
| `session/sqlite_store/transcript.rs` 259 | 1 | attribution-artifact ✔ | the closing brace of the run-id backfill block whose statements (250-258) are covered |
| `session/sqlite_store/transcript.rs` | — | covered by test ✔ | lines 93, 166 — `a_reset_refuses_when_the_history_moved_between_plan_and_rewrite` and the schema test's skipped-import guard (segment 4) |
| `session/entry_records.rs` 15 | 1 | unreachable-by-construction ✔ | the second early `return value` (when `as_object_mut` yields nothing): serde_json's `Value::index(&str)` returns a non-Null value only for `Value::Object`, so any value that passes the type guard on lines 8-13 is already an object and line 15 cannot be reached. The first return (line 12) is covered 1993 times by `a_non_entry_value_is_passed_through_unchanged` |
| `session/entry_records.rs` 134, 149, 216 | 3 | attribution-artifact ✔ | header / closing-brace / `?`-tail lines whose statements are demonstrably covered: 133 = 3354 with 134 = 0; 147-148 = 3 with 149 = 0; 213-215 = 2572 with 216 = 0. Segment 8 added `session_info_entries_persist_their_metadata_content` as the experiment — a session_info entry does drive the body (136-138 = 472 each) and the header line still reads 0 |
| `session/entry_records.rs` 139 | 1 | attribution-artifact ✔ | the closing brace of the session_info branch, whose body (136-138) runs 472 times |
| `session/entry_records.rs` 311 | 1 | attribution-artifact ✔ | the `"role":"system"` line **inside a multi-line `json!` literal** in a test fixture: 308-310 and 312-314 read **1** each while 311 reads **0**. The macro demonstrably evaluates (every other line it spans has a record), so this is the macro-interior variant of F-14/F-15 — a line llvm-cov attributes elsewhere. Measured segment 17 (was 288 before segment 8's test shifted the module) |
| `session/manager/mod.rs` 92, 215 | 2 | attribution-artifact ✔ | 92 is `)?);` closing the compactor builder whose 89-91 read **115/102/102**; 215 is `)?;` closing a transcript insert whose 211-214 read **4** each. Measured segment 14 |
| `session/manager/mod.rs` | — | covered by test ✔ | lines 99-100 — the same segment-6 compactor test: a request after `shutdown` warns instead of starting a worker |
| `session/manager/mod.rs` 124, 150, 158 | 3 | attribution-artifact ✔ | 150 is the `database = %…` *field line* of the storage-open `tracing::info!` (F-14): `opening_storage_reports_the_database_path` (segment 8) enables an INFO sink subscriber so the macro is evaluated, and 149 reads 782 while 150 reads 0 and 151-152 have no `DA` record at all — the fields sit outside the instrumented region, so no subscriber setting can cover them. 124 and 158 are closing braces of covered blocks |
| `session/manager/loading.rs` 41, 65 | 2 | attribution-artifact ✔ | `?` tails whose statements execute: 39-40/42 read 70 (line 41) and 63-64/66-67 read 509 (line 65) |
| `session/manager/metadata.rs` | — | covered by test ✔ | line 59 — the missing-row cause — by `a_metadata_update_for_a_missing_session_row_names_that_cause` (segment 7) |
| `session/journal_compaction.rs` 171 | 1 | attribution-artifact ❌ **REFUTED by measurement — needs a ruling (see the mechanism cell; the claim stays in this cell, deliberately)** | the `return Ok(0)` taken when the stop flag is already set at the top of the compaction walk's loop. **The “load-sensitive flip” this row used to claim is WITHDRAWN, and the withdrawal is a measurement rather than a re-argument:** this source file is **byte-identical to the baseline revision** (`git show 8449acf6:agent/src/session/journal_compaction.rs` — 351 lines, same statement positions), and `DA:171` reads **0 in all 39 reports on this host** — including both reports the old text named, the supervisor's baseline (`.future/cov100/baseline/ws-dirty-lcov.info`: 171 = 0) and this unit's first run (`/tmp/agent-a-after-norm.info`: 171 = 0) — and it is still 0 in the report this ledger pins (`/tmp/agent-svc-rework-norm.info`). **Not one of the 39 reports shows it non-zero**, so “covered in the baseline and first run” is false on the artifacts it cites. Neighbours in the pinned report: **170 = 240, 172 = 240** — the guard runs 240 times and never sees the flag set. **The category is refuted as well:** `attribution-artifact` means llvm-cov does not instrument the region, but 171 **has its own `DA:` record** (count 0) in every report, while its statement-shape siblings read non-zero (152-157 = 3, 186 = 1) — this is a genuinely unexecuted line, not an attribution offset, so the label suppresses work that is not an artifact question. **What it actually needs:** `stop` is set only by `CompletedRunCompactor::shutdown()` (`session/journal_compactor.rs:72`), on a thread other than the walk's, so the flag must flip while the walk is between pages. That window contains a **blocking `db.call`** (`journal_compaction.rs:175`), i.e. it is a real race rather than a synchronous window, and the campaign's own “hold the SQLite worker” recipe makes it reachable: keep the worker busy with a slow job so the compactor blocks inside `db.call`, call `shutdown()`, then release the worker — the next loop-top check observes the flag and 171 executes. **Recommended ruling: `coverable`** (not a waiver), with that fixture; under the ledger's own discriminator, “is it reachable here with a better fixture?” is answered *yes*. The request is recorded here rather than applied because writing the fixture means editing `agent/src/session/journal_compaction.rs`'s test module, which is **outside this rework's declared write set**. **Why the number is NOT moved into this cell (the convention this row was routed for):** that convention is for a line that **flips between runs**, so that the claim cell cannot assert a run-dependent fact. This line does not flip — 39/39 reports agree at 0 — and for a non-flipping line the ledger's own form (every `unreachable-*` row, §0.6's coverable rows) puts the number in the claim cell, where the tripwire checks it. Moving it here would hide a genuinely uncovered line from the tripwire's STALE half, which is the failure mode the convention exists to prevent, not one it licenses |
| `session/journal_compaction.rs` | — | covered by test ✔ | lines 248, 249 — `provider_plain = false` when a `text_delta` is not plain provider text, by the two provider-delta tests (segment 19): one delta whose `text` is a number (the key set passes, the value fails the string test) and one carrying a key outside `{text, type}` (the key test itself refuses). Both keep the raw journal and publish no snapshot. Measured 248 = **1** and 249 = **1** in a single-target probe |
| `session/journal_compaction.rs` | — | covered by test ✔ | lines 259 and 289, by `a_run_larger_than_the_snapshot_budget_keeps_its_raw_journal` and `an_escape_heavy_payload_can_exceed_the_budget_after_serialization` (segment 18). **259 is the folded estimate** (`DeltaFolder::estimated_bytes` counts a non-coalescable event as `data.len() + 512`), **289 is the serialized snapshot** — two different quantities, which is why one fixture could not reach both. Measured: 259 = 1, 289 = 1 |
| `session/journal_compaction.rs` 293 | 1 | unreachable-by-construction ✔ | the body of `if pricing.len() > SNAPSHOT_BYTE_BUDGET { return Ok(0) }`. **The condition at 292 runs and is never true** (`DA:count` 88), which is why only the body is claimed here. **Dead by arithmetic, in the F-36 style:** `pricing_bytes` starts at 2 and adds `len(item) + 1` per item, so after *n* items `pricing_bytes = 2 + Σlen + n`; the serialized array is `2 + Σlen + (n − 1) = pricing_bytes − 1`. Line 216's `ensure!(pricing_bytes <= SNAPSHOT_BYTE_BUDGET)` therefore guarantees `pricing.len() ≤ BUDGET − 1 < BUDGET`, so the condition can never hold. Measured: 292 runs **88** times and is never true, and 293 reads 0 — the two lines agree with the proof |
| `session/journal_compactor.rs` | — | covered by test ✔ | lines 91-93 — `the_journal_compactor_is_started_reported_requested_and_torn_down` (segment 6) |
| `session/journal_compactor.rs` 78 | 1 | attribution-artifact ✔ | the closing brace of the `if let Some(thread)` in `shutdown`: 71-77 have 35 hits each and 79-80 have 35, while the brace itself reads 0 (F-6 shape) |
| `session/summary.rs` 56, 100 | 2 | attribution-artifact ✔ | 56 is the `}` closing a nested block whose 52-55 read **10/10/10/3** (57 reads 30); 100 is `}))?);` closing a multi-line `json!` whose 98-99 read **28** each. Measured segment 14 (the row's mechanism already said "a closing brace and a `?`-tail", i.e. it described an artifact while claiming coverable) |
| `session/tools.rs` | — | covered by test ✔ | lines 20-22 — `Session::tool_call_args` is exercised by `tool_call_args_reads_its_own_arm_and_still_requires_the_call_id` (§0.3 segment 3) |
| `session/persistence.rs` | — | covered by test ✔ (file now at zero) | line 827 — the `return` when a run snapshot is asked for a session whose info is not on disk — by `a_run_snapshot_for_a_session_nothing_is_stored_for_skips_the_merge` (segment 24): a session whose first snapshot is also its first write, so `latest_session_info_content` finds nothing to merge and the snapshot is written as it stands. The row had recorded this as "the timeout-drain handoff"; **that mechanism was wrong** — the line is the *merge* early return, and the fixture is a store with no entries for the id (a deliberately explicit precondition, asserted before the call). Called from the timeout-drain handoff's sibling path, which is why the old note named it. Renumbered +18 in segment 22, which added the `fail_next_info` seam above it |
| `session/persistence.rs` | — | covered in the current report ✔ | lines 1606-1612 were the assertion lines of `timeout_drain_handoff_executes_queued_command`, uncovered only in the run where that liveness test flaked; the current report shows them covered (§0.7). Kept as a row (not a claim) so a reviewer can see why an earlier report showed 8 residuals here |
| `session/repair.rs` 90 | 1 | attribution-artifact ✔ | the `drop_idx.len()` argument of the dedupe warning: the callsite lines 88-89 have 26 hits (the dedupe test at `repair.rs` 275 asserts the drop) while its own argument expression reads 0 — a disabled callsite does not evaluate its arguments |
| `session/checkpoint.rs` | — | covered by test ✔ | lines 91 — `checkpoint_is_valid_rejects_each_dangling_reference` (segment 4): a checkpoint whose own entry is not in the history is rejected |
| `session/reset.rs` 473 | 1 | attribution-artifact ✔ | the closing brace of the `Attachments` drop block; 469-472 (the `meta.remove` + `removed.max(1)`) are covered by `dropping_attachments_removes_the_files_the_images_and_the_manifest` |
| `session/journal_compaction_tests/transactions.rs` (88, 117, 141) and `.../validation.rs` (48, 77) | 88, 117, 141, 48, 77 | 5 | **coverable (§0.6)** | control flow of the compaction test harnesses (documented as fixtures, not production) |
| `skills/registry.rs` 176, 220, 240 | 3 | attribution-artifact ✔ | 176 is the `}` of an `if let` whose body (174-175) read **115**; 220 and 240 are `)?;` closing multi-line queries whose 216-219 and 237-239 read **95** and **170**. Measured segment 14 |
| `service/mod.rs` | — | covered by test ✔ | lines 79, 80, 81 — the `addr.trim().eq_ignore_ascii_case(AUTO_ENDPOINT)` arm of the local-start endpoint parser, by the `gated-auto-addr` fixture case (segment 9): with `grpc_addr = Some(" AUTO ")` the permit is accepted and the host runs, so both the non-empty and the case-insensitive comparison are evaluated |
| `service/mod.rs` | — | covered by test ✔ | line 105 — the `?` on the instance-lock acquisition, by `an_unopenable_instance_lock_stops_startup_before_any_host_code` (segment 9): a regular file at `<home>/agent` makes `InstanceFile::open` fail, the host never runs, and the foreign file is left as found. This is a `?`-tail that **does** take its line (contrast F-12) |
| `service/mod.rs` | — | covered by test ✔ | lines 130, 132 — the both-`Err` arm (host error **and** failed cleanup, each preserved: `a_host_error_and_a_failed_cleanup_are_reported_together`, which returns an error while dropping an undrained serving future) and the `resume_unwind` arm (a panicking host is resumed rather than swallowed: `a_panicking_host_is_resumed_and_leaves_the_instance_reusable`, which also shows the scope is released so a later service can take the same root) — both segment 9 |
| `service/mod.rs` | — | covered by test ✔ | lines 156, 157, 160 — the wait loop's body in `finish_scope`'s **no-handle** branch — by `a_failed_start_waits_for_background_work_before_releasing_the_instance` (segment 20). The loop has to be entered **once** with work still owned, and this is the only reachable way: the branch runs 51 times in the measured report with `counts()` always `(0, 0)`. The fixture builds the service directly (instance file + literal) rather than through `with_instance`, so it does **not** install the process-global `ACTIVE` slot that would poison every other test in the binary; measured 7 iterations, so the body is genuinely exercised |
| `service/mod.rs` | — | covered by test ✔ | lines 343, 355, 356, 357, 358 — the cancellation that lands **after** `serve`'s entry check and **before** the server's first poll, and the "Agent serving failed" report it triggers — by `a_cancellation_before_the_first_poll_is_reported_as_a_failed_start` (segment 20), measured **20** executions each. **F-21 called this window unschedulable and was wrong** (F-43): a thread already spinning on `server_running` sees the store in tens of nanoseconds while `serve` still has to subscribe to two `watch` channels and build the guard and the pinned future — hundreds of nanoseconds. The fixture gives the canceller a head start, the result is unambiguous (a `StartupCancelled` seen here cannot come from the entry check, which runs before `server_running` is even set), and the assertion is "at least one hit in 20 attempts", so it is not a coin flip |
| `service/instance.rs` 16 | 1 | attribution-artifact ✔ | the closing brace of the parent-directory creation in `InstanceFile::open`: 13-15 read 82 hits each and 17 reads 82, while the brace reads 0 (F-6 shape). The behaviour is covered by `opening_the_lock_file_creates_its_missing_parent_directories` (segment 6) |
| `service/instance.rs` 34 | 1 | unreachable-in-this-environment ✔ | the `anyhow::Error::from(error)` arm of the lock probe: needs `try_write` to fail with an error that is *not* `WouldBlock`, i.e. a lock backend or filesystem that cannot take the lock at all. **Reclassified from `coverable` in segment 11 — the row's own mechanism already said "no fixture can force this on a healthy filesystem", which is not a description of coverable work** |
| `service/instance.rs` 131 | 1 | unreachable-in-this-environment ✔ | `hostname_raw()` returning `None` when `gethostname` fails |
| `service/instance.rs` 264 | 1 | platform-unmeasured ✔ | `%COMPUTERNAME%` (Windows host identity) |
| `service/instance.rs` | — | covered by test ✔ | lines 216-218 — `owner_description_covers_host_only_pid_only_and_neither` (§0.3 segment 3): the host-only, pid-only and neither cases of the lock-conflict message |
| `service/startup.rs` 27 | 1 | attribution-artifact ✔ | the `settings_path.display()` **field-expression** line of the "Failed to load settings … Falling back to defaults" `tracing::warn!`: the macro at 25 ran **1** time, 27 reads **0**, and 26/28/29 have no `DA` record at all (F-14). Measured segment 15 |
| `service/startup.rs` 144, 154 | 2 | attribution-artifact ✔ | `?`-tails closing multi-line constructor calls: 144 closes `ResolvedModelTarget::from_model` whose 140-143 read **3** each, and 154 closes `Engine::new` whose 147-153 read **40** each. Measured segment 15 |
| `service/activity.rs` | — | covered by test ✔ | lines 40 — `activity_is_unavailable_once_a_stop_has_failed` (segment 6) |
| `service/activity.rs` | — | covered by test ✔ (file now at zero) | line 71 — the optimistic-concurrency check `revision != service_work.revision()` — by `activity_is_unavailable_when_work_changes_during_the_snapshot` (segment 24): a thread that takes and drops a work guard continuously while the snapshot is taken repeatedly, asserting "at least one refusal in N attempts" with the tally printed. The window cannot be widened from outside — every lock in the function is a `try_` lock, so holding one produces a *different* error rather than a delay (F-57) — which is why this is a biased race fixture rather than a deterministic one, and why the probe count (71 = 1 with 70 = 2) is part of the evidence. The old note read "needs a concurrent `close_streams`/`register`. A deterministic test needs a seam (there is none today); a spinning test would be hope, not evidence |
| `service/config.rs` | — | covered by test ✔ | lines 36, 38-39 — `validate_environment_rejects_a_home_that_changed_after_configuration` (segment 6) |
| `service/work.rs` | — | covered by test ✔ | lines 112 — `a_shutdown_future_resolves_when_its_owner_is_dropped` (segment 6): the `break` when the watch sender is gone |
| `cli.rs` | — | covered by test ✔ | lines 195, 197 — the `agent --linux-sandbox-helper …` argv rewrite. **This row was misclassified as `platform-unmeasured`: the rewrite itself is not `cfg`-gated** (`cli.rs` 191-200 runs on every platform), and the earlier test passed the flag *without* an `agent` prefix, so it never took the branch. `the_agent_prefixed_helper_is_rewritten_then_refused_off_linux` (segment 10) supplies the prefix and the non-Linux refusal proves the rewrite ran |
| `cli.rs` 228, 234, 235, 236 | 4 | unreachable-in-this-environment ✔ | **reclassified from `platform-unmeasured`:** these lines *compile* on macOS (they are not `cfg`-gated), they are simply not reachable here. 228 needs a Windows probe result carrying a diagnostic, and on macOS the helper returns `unavailable_without_error`; 234-236 need `reset_windows_sandbox_capabilities()` to succeed, and it returns `Err(Unsupported)` off Windows, so the `?` propagates first. `platform-unmeasured` means "compiles only on another OS", which is true of neither |
| `cli.rs` 310 | 1 | unreachable-by-construction ✔ | "FutureOS home is not a directory". **Reclassified from `coverable` in segment 11** by reading the two lines above it: `create_dir_all(dir)?` (307-308) runs first and returns `Err` when the path exists as a *file* (`AlreadyExists`/`NotADirectory`), so the `?` propagates before the guard; and when it succeeds the path is a directory by construction. The unit's own `a_home_path_that_is_a_file_fails_at_creation_and_names_the_input` (segment 9) asserts exactly that ordering, which is the evidence |
| `cli.rs` 449, 505, 506 | 3 | attribution-artifact ✔ | argument lines of display/size formatting macros whose statement line is covered (finding 3) |
| `cli.rs` | — | covered by test ✔ | **line 590** — the `anyhow::ensure!(report.status == Stopped, "Agent failed to stop: …")` in the **first** arm of `async_main`'s final `select!` — by **`a_cut_short_drain_is_reported_instead_of_a_clean_stop`** (`agent/src/cli.rs:923`, `mod cut_short_drain_tests`). **This line was waived (segment 26, option B) and the waiver fell to review: F-1 of `review-agent.md` falsified it, and the reclassification is `coverable` → covered by a test.** The fixture is smaller than the review's probe and reaches the arm without any RPC: an isolated HOME whose `auth.json` gives the Future provider a `base_url` that **accepts TCP and never answers**, one SIGINT, ~4.3 s of wall time, no model and no provider. **Measured mechanism (this unit, re-measured, and tighter than segment 26's guess):** the cold-cache Future catalogue refresh that the first `Registry::new()` spawns (`models/future.rs`'s `spawn_future_models_refresh` → `spawn_future_models_refresh_unlocked`, which takes `crate::service::work::process_background()` and then blocks a **native** thread in `fetch_future_models`) holds that registration for as long as the fetch waits. The drain cancels *session runtime state* (`session.abort()`, `cancel_session`, approval/shell cancellation — `service/shutdown.rs` 162-182) and **cannot** cancel a platform fetch, so its `wait("runs and native background work")` stage (shutdown.rs **215**) never settles; at the 4 s deadline `async_main` itself requested the deadline watcher publishes `FailedStopping`, the report is ready while the serve future is still pending (the listener stage, which would stop the listener, is never reached), and the **first** arm runs the `ensure!` — which fails, because a `FailedStopping` report is not `Stopped`. **Measured, not asserted:** the child agent exits **1** with `Agent failed to stop: Agent shutdown deadline exceeded during runs and native background work; instance ownership retained` at **4.03 s** after the interrupt, in the same report where `DA:590 = 1` and `DA:591 = 0`. The test carries its own **control leg** — the identical home, agent and interrupt **without** the Future credential — which stops cleanly (exit 0), so the non-zero exit is attributable to the un-cancellable platform work and not to the isolated home or the interrupt. The test re-execs the test binary and becomes the agent through `crate::cli::run_from_args`, the same entry point `main` uses (the `cli::shutdown::tests` pattern), because the startup path installs process-global state. **Segment 26's stated reason for the waiver is corrected here as well** (F-5a): its route (b) said "every unary command returns promptly without a real provider" — **that is false**, `shell` blocks for up to `SHELL_EXECUTION_TIMEOUT` = 120 s (`packages/rpc/src/command_policy.rs:18`), and the review measured it blocked. What actually defuses that route is not promptness but the drain's **cancellation**: the admitted shell is aborted and the call returns `"shell command cancelled"`, so no stall remains (review P1, exit 0 in 0.1 s) |
| `cli.rs` 591 | 1 | unreachable-by-construction ✔ | `server.await`, the statement after that `ensure!`, needs the report to be **`Stopped`** while the serve future is still pending. **The mechanism is upgraded to the causal enumeration `review-agent.md` §3.4 supplies** (the earlier version stopped one step short and left a non-`biased` `select!`'s tie-break as the decider, which would have made this a flake rather than dead code): (i) a report can only be published by the drain, by the deadline watcher (which publishes **`FailedStopping`**), or by a failed spawn — and `finish()` is idempotent and never rewrites an existing report (shutdown.rs 40-57), so every early `return errors` in `drain` fires only *after* a report already exists; (ii) the drain publishes a `Stopped` report only after its `wait("RPC listener and streams", || !self.shutdown.server_running.load(..))` stage (shutdown.rs 300-303); (iii) `server_running` has exactly **two** writers — `store(true)` at `service/mod.rs:325-326` and the `false` in `ServerGuard::drop` (`service/mod.rs` 302-319) — and that guard is a local of `RunningAgent::serve`, so it drops **in the same poll that returns the serve future's `Ready`** (`service/mod.rs` 353). At the instant the report *can* become `Stopped`, the second arm is therefore already ready and the select takes it; a random tie-break cannot bite, because the report branch is always pending at the start of that poll and the select returns within it. **Covering 590 is what makes this waiver durable:** the first arm's mechanism is now measured (see the row above), and in the same report 591 reads 0 — so the pair is "covered + unreachable-by-construction" rather than two waivers with one guess. Named invalidator: **a `Stopped` report that does not observe `!server_running`** — the listener wait moved after `finish`, a watcher that publishes `finish(vec![])`, or a third writer of `server_running = false` outside the serve future's own poll. Any of those makes 591 live, and the row must be re-derived rather than patched |
| `rpc/commands/session_title.rs` | — | covered by test ✔ (file now at zero) | line 185 — the **third** "Session model is unavailable" arm, the one taken when the model is *known* but its provider cannot reach it — by `suggestion_replaces_a_model_whose_provider_cannot_reach_it` (segment 21). The two earlier arms fail on an *unknown* model (`registry.resolve(..)` / `resolve_request_target(..)` returning `None`) and were already covered; this one needs the resolve/`resolve_request_for` split, i.e. a model with no credential whose sibling in the same provider has one. The test swaps in a two-model registry through `Registry::from_models_and_auth` (the established test seam) and asserts both preconditions — the model resolves, and its request target is the sibling — so it cannot silently stop exercising the arm |
| `rpc/commands/session_title.rs` | — | covered by test ✔ | lines 210, 213 — the title handler's **success** arm (the payload build and the `Ok(data)` response), by `a_successful_generation_returns_the_title_from_the_configured_provider` (segment 14): the on-disk provider config is pointed at a local SSE server serving a chat-completions delta plus a `stop` finish, and the response carries the streamed title and the session's model reference. The other handler tests stopped at model selection (a closed port), so the `Ok` arm had never run — the same gap class as `skills.rs` 14/15 in segment 13. Measured: 210 = 1, 213 = 1 |
| `rpc/commands/providers.rs` 593, 594, 595, 600 | 4 | attribution-artifact ✔ | the `SkillCandidatePayload` field lines and the serialization error arm of a two-`String` payload: the sibling `Ok` path is covered by `suggest_candidates_cross_the_wire…`, and `to_value` cannot fail for two strings |
| `rpc/commands/mod.rs` 404 | 1 | unreachable-by-construction ✔ | the final `_ => "unknown command"` arm: `command_policy` rejects an unknown type 340 lines earlier, so the arm is reachable only if a command is registered yet missing from every dispatch list — and `every_registered_command_reaches_a_dispatch_arm` (§0.3 segment 3) sweeps all 81 registry entries and fails on exactly that error, so the invariant is now test-backed rather than asserted |
| `rpc/commands/skills.rs` | — | covered by test ✔ | lines 14, 15 — the install command's **success** arm (the discovery republish and the empty-payload success), by `install_skill_publishes_the_installed_skill_after_a_platform_download` (segment 13). Both earlier install tests were failure paths (a rejected id, an unreachable platform), and line 13's own count told the story: `manager.install(..)` ran **9** times and 14/15 read 0, i.e. every one of those nine calls returned `Err`. Measured after the fix: 13 = 1, **14 = 1, 15 = 1** |
| `rpc/commands/skills.rs` 27 | 1 | unreachable-by-construction ✔ | `_ => unreachable!()` after the dispatcher's own exhaustive match |
| `rpc/commands/session_lifecycle.rs` 361 | 1 | unreachable-by-construction ✔ | the `else` branch of `cmd_get_fork_messages`' content projection (a bare-string content, vs the block array). **Every write path normalizes a string into a block array first:** `replace` → `rewrite_transcript_rows` → `normalize_entry` (`sqlite_store/transcript.rs` 225) and the manager's `serialize_entry` (`manager/loading.rs` 11-13), whose line 19 is literally `Some(Value::String(text)) => vec![json!({"type":"text","text":text})]`. So no row *this build* writes can hold a bare string; the arm is retained for a database written by an older build. **The existing test that names this branch is a false positive — see F-34.** Measured: the array branch (352-360) ran, 361 read 0 |
| `rpc/commands/session_lifecycle.rs` 644 | 1 | attribution-artifact ✔ | the `}` closing the `if *before == after` block, whose 641-643 read **9** each while 645 reads **9**. Measured segment 15 |
| `rpc/commands/session_lifecycle.rs` | — | covered by test ✔ | lines 474 and 578 — the two `tracing::warn!` arms that report a model the fresh loop could not take — by `new_session_reports_a_model_the_protocol_layer_cannot_build` (segment 21). **The mechanism recorded here was wrong**: these are not "a loop that refuses the sync" but `set_model` failing its own protocol validation (`ResolvedModelTarget::from_model` rejects an unknown `api`), which happens *before* any loop lock is touched. Both arms are non-fatal by design and the test asserts the session is still created with the requested identity. Measured **1** each |
| `rpc/commands/run_control.rs` 250 | 1 | attribution-artifact ✔ | the `session_id = %sess.session_id` **field-expression** line of the "session abort requested by RPC client" `tracing::info!`: the macro at 249 ran **5** times, 250 reads **0**, and 251-253 have no `DA` record at all (F-14). Measured segment 16 — the row had carried it as coverable, with the field line's own mechanism already saying "a `tracing` field line" |
| `rpc/run_snapshot.rs` | — | covered by test ✔ | lines 36, 39, 68, 95 — `unreadable_events_survive_and_never_count_as_duplicates` (segment 8): an unreadable `tool_start`, a start with no tool identity, and an unreadable fragment that neither contributes to `streamed_arguments` nor counts as duplicate evidence |
| `rpc/run_snapshot.rs` 202 | 1 | unreachable-by-construction ✔ | `unreachable!()` on a variant the match above already excluded |
| `rpc/prompt_helpers.rs` 733 | 1 | platform-unmeasured ✔ | `PathBuf::from(r"C:\workspace")` — the Windows path shape |
| `rpc/approval.rs` 611 | 1 | unreachable-by-construction ✔ | `else if status == Cancelled` in `resolve_approval`: the only producers of a `Cancelled` verdict are `RunApprovalReviewer::invalidated()`'s two arms, the function re-reads `invalidated()` three lines earlier and returns before this comparison, and `invalidated()` is monotone (the flag is only set, the generation only grows) |
| `rpc/approval/escalation.rs` 93 | 1 | attribution-artifact ✔ | the closing brace of `shorten_home`, whose body is covered by `denial_paths_survive_an_unterminated_quote_and_shorten_home` (the `~` arm is line 88) |
| `rpc/broadcaster/journal.rs` 37 | 1 | attribution-artifact ✔ | `)?;` closing the `EventBatchWriter::new` call whose 33-36 read **680** each. Measured segment 14 |
| `rpc/broadcaster/journal.rs` 43, 45 | 2 | unreachable-in-this-environment ✔ | the **second** copy of the "event journal is permanently owned" guard (line 43's `bail!` and its field line 45). The first copy at 21-26 is covered by `configured_journal_cannot_be_rebound_to_another_session`, which returns before reaching this one; 43 read 0 while the enclosing condition at 42 read **690**. This second check exists only to close the lock-free window between the first check (line 27) and the re-acquired lock at 41, so it needs a *concurrent* rebind to hit — a deterministic fixture would need a seam. Measured segment 14 |
| `rpc/broadcaster/journal.rs` | — | covered by test ✔ | lines 93, 106 — the two "no journal configured" arms (`flush_journal`'s `None => Ok(())` and `shutdown_journal`'s `else { Ok(()) }`), by `journal_lifecycle_calls_are_no_ops_without_a_configured_journal` (segment 15). A memory-only broadcaster is a **documented supported state** (`configure_journal`'s own comment: "tests and short-lived utility broadcasters may intentionally remain memory-only"), and both calls are ones a host makes unconditionally — the per-run completion flush and the drain's shutdown — so reporting a failure there would be a false alarm. Measured before the fix: `flush_journal`'s match ran **71** times and `shutdown_journal`'s **19**, never with `writer = None`. The row had carried them as "the idempotent close path" under `coverable`, which is F-31's self-contradiction pattern again — and this one was real work, not an artifact |
| `rpc/broadcaster/journal.rs` 202, 203 | 2 | unreachable-by-construction ✔ | `AppendFailure::Unavailable("event serialization failed")`: the append is `serde_json::to_value(event)` on the `&SseEvent` being appended, and that value cannot fail to serialize — `SseEvent` is five `String`s and two `i64`s (`rpc/protocol.rs`), with no non-string map key and no custom `Serialize`. **The mechanism cell used to name `to_string`; corrected (F-5b of `review-agent.md`): the code calls `serde_json::to_value` (201), and the reason has to name the function the code actually calls.** **The pair's stability, corrected too:** "these two numbers move between runs with no source change, expect the numbers to drift" overstates it. The zero-count pair slid **once**, from 200/201 (`s32`) to 202/203 (`s33`), on a segment whose only in-scope deltas were `session_reset.rs` and `session/database/mod.rs` — and four consecutive reports (`s33`-`s36`) then agreed at 202/203. **Re-measured in this rework's own report** (`/tmp/agent-svc-rework-norm.info`, same revision): 202 = 0, 203 = 0, with 200 = 11 922, 201 = 11 922, 204 = 11 922 and 205 = 44 774 — the arm still holds and the pair is still where the ledger says it is. What is still true, and is the reason the row names the *arm* rather than the pair: entries in a `.map_err` closure's unentered region are attributed to lines *inside* it, so a future recompile may re-attribute the region to 200/201 again. A reviewer should read the arm **and** re-read the pair from the report the rest of the ledger cites (F-55). Named invalidator: a second `SseEvent` shape whose serialization can fail — a map with non-string keys, or a fallible custom `Serialize` — makes the closure enterable and the pair coverable |
| `rpc/broadcaster/replay.rs` | — | covered by test ✔ | **all three, segment 17 — this file is now at zero uncovered lines.** 172/173 (a *current* run whose ring dropped the events the client asks for prefers the stored snapshot) by `a_truncated_current_run_prefers_its_published_snapshot`; 211 (`run_snapshot` for a *historical* run returns the published snapshot) by `a_compacted_run_is_served_from_its_published_snapshot`. Both need a real `run_snapshots` row, which only compaction writes, so both fixtures settle the run in `runs` and call `compact_run`. **Two constraints make the fixture work and are worth reusing (F-37):** compaction returns `Ok(0)` unless the run's **last raw event is `agent_end`** (`journal_compaction.rs` 276-277), and the *current*-run path additionally needs the in-memory ring to have overflowed (`MAX_RUN_EVENTS` = 2000) so `truncated` is true — which is why that fixture broadcasts 2 100 `text_chunk`s **and then one `agent_end`** |
| `grpc/mod.rs` 100, 101 | 2 | attribution-artifact ✔ | the `address.ip(), address.port()` **field-expression** lines of the TCP-listening `tracing::info!`. Measured segment 13: the macro at 98 runs **31** times while 100 and 101 read **0** and 99/102 have **no `DA` record at all** — the fields sit outside the instrumented region, so neither an INFO subscriber nor a wider test can move them (F-14). The row's earlier mechanism ("`address.port()` is skipped although `address.ip()` runs") described the same asymmetry but as a *behaviour*, which would invite someone to write a fixture for it; the counts show no fixture can help |
| `approval_review/evidence.rs` 143 | 1 | attribution-artifact ✔ | the closing brace of the user-buffer `else` block whose statements (131-142) are covered |
| `approval_review/evidence.rs` 316 | 1 | attribution-artifact ✔ | finding 3: a branch-free statement directly after a covered sibling in the same block |
| `approval_review/evidence.rs` 573, 574 | 2 | unreachable-by-construction ✔ | finding 5: no producer of `"truncated"` exists in the workspace |

**How this ledger is checked.** The authority is the supervisor's per-line
tripwire over the report and this section, which inspects every uncovered line
rather than a count. **The rule this record follows (F.48): name the report that
produced the ledger above, and re-derive the command when the ledger moves.** A block
that pins a superseded report keeps the verdict of a revision it no longer describes,
and then reads as *"the ledger is broken"* rather than *"the anchor is stale"*. Here
that report is the rework's own — `/tmp/agent-svc-rework-norm.info`, whose measurement
is §0.2a and whose `covlines.py total` line is the 202 above:

```
python3 .future/cov100/reconcile.py --report /tmp/agent-svc-rework-norm.info \
  --scope /Users/geilige/future-os/agent/src/rpc \
  --scope /Users/geilige/future-os/agent/src/grpc \
  --scope /Users/geilige/future-os/agent/src/service \
  --scope /Users/geilige/future-os/agent/src/session \
  --scope /Users/geilige/future-os/agent/src/skills \
  --scope /Users/geilige/future-os/agent/src/approval_review \
  --scope /Users/geilige/future-os/agent/src/cli.rs \
  --doc docs/testing/module-agent-rpc.md --section "Residual ledger" --label AG-A
  # -> files with uncovered lines=43 uncovered lines=202
  #    mentioned in module-agent-rpc.md: 202
  #    MISSING (uncovered, not mentioned): 0        (exit 0 — no STALE either)
```

`--section "unit \`AG-A\`"` (§0's own heading) reaches the **same verdict** and is the
wider slice — it checks the ledger *and* the citations in §0.1-§0.4. Both labels are
printed because the section must actually *contain* the ledger: a slice that stops
short parses no rows, and the tool then reports 0 STALE for a region it never read
(F-35's class, from the opposite direction).

**Two ways this block has silently stopped reproducing, both measured here — check
for them before believing a MISSING count:**

1. **A superseded report.** This block used to name segment 19's
   `/tmp/agent-a-s25-norm.info` while the ledger said 241 — the anchor the ticket that
   produced this paragraph found. Run verbatim against **this** revision it prints a
   **`STALE` block over 10 lines (8 `COVERED`, 2 `ABSENT`) and no MISSING at all, exit
   0** (`/tmp/svc-s25-frozen.txt`). Neither half is a real gap, and one reason explains
   both: **the numbers moved**, by exactly the shifts this record already documents.
   Segment 22 added a compaction test above the `assert!` pair, so the five
   `assert!`-argument lines below 3590 are **99 lower** in that report's revision than in
   this ledger — the report has `session.rs` **3614, 4875, 4912, 5048, 5327** where this
   ledger cites 3713, 4974, 5011, 5147, 5426 — and `persistence.rs` has **809** where
   this ledger cites 827 (that file grew by 18, separately: segment 24's
   fork/persistence rows). So the report's numbers look *missing* to the tripwire (this
   ledger never cites them) while this ledger's numbers look *stale*: 3713, 4974, 5011 and
   5426 read `COVERED`, because ~99 lines of unrelated code now sit there, and 5147 reads
   `ABSENT`, with no `DA:` record at all; the tenth line, `journal.rs` 203, is the
   serialization pair §0.2 documents moving from 200/201 to 202/203. A superseded report
   therefore cannot check a renumbered ledger: it disagrees with it on *line identity*, not
   on coverage — and from the output alone that disagreement is indistinguishable from a
   genuinely broken ledger. That is why the anchor matters.
   **And the MISSING count is not merely stale — it is self-erasing**, which is the
   sharper lesson. The tripwire's *"mentioned"* half matches numbers appearing **anywhere
   in the text**, so a paragraph that quotes a missing line number *repairs the MISSING it
   quotes*. Measured on this same stale report across three revisions of this document:
   **MISSING 7** (the ticket's run, before this unit's rework), **6** (this unit's earlier
   text, `/tmp/svc-run-C.txt`), **0** (now, because the sentences above name all six
   numbers). The **row-derived `STALE` half read 10 in both runs re-measured here**
   (`/tmp/svc-run-C.txt`, `/tmp/svc-s25-frozen.txt`), whatever the MISSING count did, and it
   is the half to quote. Quote the **rule** and the row-derived output; treat a pasted MISSING count as a
   snapshot of one text revision, never as a constant — and note that `reconcile.py` exits
   on `MISSING` alone (`sys.exit(1 if missing else 0)`), so a clean exit 0 with a `STALE`
   block below it is a real state this record can be in.
2. **A column-0 `#` comment inside a code fence counts as a heading** to the tool's
   slicer: `reconcile.py`'s `slice_section` matches `^#{1,2} ` and does not skip fences.
   A pasted output written flush-left therefore **truncates the slice**, and the check
   under-reports *"mentioned"*. Measured: with §0.2a's `# -> covered=…` line at column 0
   — which is how this unit wrote it last revision — `--section "unit \`AG-A\`"` stopped at
   that line and printed **MISSING 104** where the ledger was complete. §0.2 already
   recorded this trap for an earlier revision ("a comment line at column 0 inside a
   fenced code block (`# …`) parses as a level-1 markdown heading, which truncates
   `reconcile.py`'s `--section` slice"); **§0.2a reintroduced it anyway**, which is the
   argument for putting the warning *at* the block, as §0.2a and this one now both do.
   **This one is a defect in the tool, not in this document** (`reconcile.py` is outside
   this unit's write set), so it is reported rather than fixed; the consequence for the
   campaign is that any record with a flush-left `#` line inside the section it passes to
   `--section` under-reports *"mentioned"* — and the tool offers no protection: the word
   `fence` does not occur in `reconcile.py` at all, and `selftest.py` never mentions
   `slice_section` (0 occurrences), so this behaviour is untested (checked 2026-10-17).
   A sweep of `docs/testing/*.md` finds **278** such lines in 30 documents, and **12** of
   them are records that both document a `--section` invocation and carry a fence-level
   `#` line *above* their ledger rows — so the same false MISSING is available to every
   one of those records until either the tool learns about fences or their pasted outputs
   are indented.

**A blind spot in that tool, found the moment this unit took a file to zero
(F-35).** `reconcile.py`'s STALE half looks each row's path up in the report's
*uncovered* map (`claim_targets(path, ln, per_file)`), so a file with no uncovered
lines is absent from it, `claim_targets` returns nothing and the loop `continue`s.
This run therefore printed **no STALE block while `session/history/paging.rs`'s
seven-line row was still claiming lines that had just been covered** — I removed the
row by hand. The consequence for a reader: **the STALE check silently stops covering
a file at exactly the moment its last line is covered**, which is when a leftover
row is most likely. Until the tool is fixed, the manual step is to delete or convert
every row for a file whose count reaches zero, and to re-read the ledger for the file
rather than trusting the absence of a STALE line.

**The STALE half earned its keep again this segment.** While writing the 273/303
row the natural shape was a claim cell naming both lines, and the tripwire
immediately answered `STALE → shutdown.rs:273 (DA>0)`, because a claim cell asserts
"these lines are uncovered" and 273 is covered in this run. The row now keeps its
numbers in the *mechanism* cell — the documented rule for lines a test covers —
since "one of the pair is 0 each run" is a property a claim cell cannot express.

**The flipping-row rule, stated once so the rows below can be judged against it
(`review-agent.md` §9.1 is the source, and the supervisor confirmed it is campaign-wide
rather than AG-A-only on 2026-10-17):** a row whose line **flips between runs** must
carry its number in the **mechanism** cell and its text must say that it flips. The
claim cell is what the tripwire reads as an assertion about one specific report, so a
run-dependent fact there makes the record assert something the next run contradicts —
the F.48/F.9 class, and exactly what the STALE half is for. The rows that follow that
rule today are the `service/shutdown.rs` 273/303 pair (above) and `rpc/session.rs`
1217 below. **The rule is conditional on the flip being real**, and this section now
also records a row where it was not: `session/journal_compaction.rs` 171, whose "flip"
is refuted by 39 reports and which therefore keeps its claim — a marker for the other
case is `❌`, used on that row's category cell, meaning "this row's own text is refuted
by measurement and the replacement is the supervisor's ruling". `✔` keeps meaning
"mechanism established by this unit and re-checked".

Row shape matters for that check, and this section uses the tool's
`LEDGER_ROW_SPACED` shape — the file in backticks, then its numbers, in the
*first cell*: ``| `path.rs` 12, 14 | n | category | mechanism |``. Rows that
record lines a test now **covers** deliberately keep their numbers out of the
first cell (``| `path.rs` | — | covered by test ✔ | lines 12 … |``), because the
checker reads any `path` + numbers first cell as an assertion that those lines are
still uncovered — and it should: that is how a stale row gets caught. Segment 9
caught a different half of the same hazard: adding two tests to `rpc/session.rs`
moved every line number below them by **+83**, so the six rows that cite lines in
that module had to be renumbered even though nothing about them changed. A ledger
that cites line numbers inside the file it is measuring has to be re-derived
whenever that file grows — that is a property of the metric, not an oversight.

`python3 .future/cov100/rowscan.py docs/testing/module-agent-rpc.md --section
"unit \`AG-A\`"` reports `shipped 92 / claims 198` for this section (`rows 521`,
`no-colon 92`), so the STALE half genuinely runs (earlier revisions parsed as ZERO
claims — a vacuously green check). **Re-measured 2026-10-17:** the text here used to
read `shipped 110 / claims 237`, which neither this revision nor the committed one
reproduces — the same stale-pasted-count class as the tripwire block above, found while
applying the flipping-row rule. The whole-document figure is `shipped 108 / claims 367`
over `rows 624`.

The `n` column is a per-row count, not a partition of the report: claim rows
account for **198** of the 203 uncovered lines; the remaining 5 are the compaction
*test harness* rows, whose numbers sit in the mechanism cell of a combined row
(`transactions.rs` 88/117/141 and `validation.rs` 48/77). The category split over
the claim rows (this table, the totals and the claim count are `sumcheck3.py`'s
output against the report, not hand arithmetic):

| category | rows | lines |
|---|---|---|
| `attribution-artifact` | 46 | 87 |
| `unreachable-by-construction` | 24 | 50 |
| `unreachable-in-this-environment` | 12 | 32 |
| `coverable` — not waived, fixture named | **0** | **0** |
| `platform-unmeasured` | 7 | 29 |
| **total claimed** | **89** | **198** |

**The ledger has been reshaped more than it shrank: `coverable` has gone 96 → 72 →
60 → 55 → 39 → 35 → 21 → 15 → 12 → 6 → 3 → 1 while `attribution-artifact` went
74 → 81 → 85 → 87 → 88 → 89 → 87, and the uncovered total moved from 261 to 203
over that whole span.** This segment one line left `coverable` by being covered
(`provider.rs` 181, whose stated cost proved illusory — F-61) and one by moving to
`unreachable-by-construction` on the drain's ordering (F-62). Both row
audits (`/tmp/audit_rows.py` and `/tmp/audit_artifact.py`) report **zero**
candidates as of this revision, which is the state to keep. That is the
measurement work being done properly — most of what remained was mislabelled, not
untested — but it is worth stating plainly for the reader of the count alone: the
number that matters at the end is "zero, or waived with a category", and this ledger
is converging on classifying correctly rather than on deleting lines. See F-31 and
F-32.

`coverable` fell by **five lines** in segment 12, and **none of them was covered** —
all five moved to `unreachable-in-this-environment` on measured evidence
(`database/mod.rs` 79-83). Combined with segment 11's move (four lines, two of
them covered) and segments 19-25 (twenty-six covered, six reclassified),
`coverable` has fallen from 114 to **1** while the total fell by 58: a category is a claim about *why* a line is uncovered, so a row whose
mechanism contradicts its category is a defect even when the line count does not
move. The two audits that find them are F-24 ("is this line reachable here with a
better fixture?") and F-25/F-27 ("does the row's mechanism match its category?"),
and both are cheap enough to run over the whole ledger — `/tmp/audit_rows.py`
scans for a `coverable` row whose mechanism says *no fixture*, *no seam* or
*exhaustion*, and for an `unreachable` row that mentions a fixture.

Two notes on the categories that moved. `attribution-artifact` grew by 8 lines
because segment 8 *measured* the class rather than asserting it (§0.4 F-14/F-15) —
including `rpc/mod.rs` 353/354, which this ledger had been carrying as coverable
work. On this unit's evidence that label means "a line llvm-cov does not
instrument", not "code nobody exercised", so a row may only carry it together with
the counts showing the enclosing statement runs. `unreachable-*` grew because two
rows were split and justified with an argument that a fixture cannot reach them.

Rows whose `n` column is `—` are not claims: they record lines this unit's tests
moved to covered (kept so a reviewer can see they were classified rather than
dropped). There is no `?` row left in this revision, and **as registered there is no
`coverable` row either**: every line in this unit is either covered by a named test or
carries one of the four waiver categories. **One row is an open question rather than a
settled waiver** — `session/journal_compaction.rs` 171, whose `attribution-artifact`
category is refuted by measurement and whose recommended replacement is `coverable`
(the row's mechanism cell carries the evidence, the fixture and the request). It is
counted in this table as registered, so the totals below stay arithmetic; if the
supervisor rules it `coverable`, this ledger has **1** coverable line and §0.6's list
reopens for exactly that entry.

**Segment 26 asked the supervisor to add a fifth, "waived for fixture cost" shape to
`docs/testing/waiver-ledger.md`, for `cli.rs` 590 — and that request is withdrawn here,
because the waiver it was meant to protect fell.** `review-agent.md` (F-1) built the
fixture in ~5 s and measured `DA:590 = 1`; this unit then wrote the test (§0.4, F-64)
and now measures the line as covered. The shape was never added, so nothing has to be
removed: the four categories in `docs/testing/waiver-ledger.md` stand, and the lesson
is the ledger's own discriminator applied **without** the escape hatch — "is the line
reachable here with a better fixture?" answered *yes* means `coverable`, and
`coverable` means a test, not a new category. The same answer was given to the Windows
recorder PE (`module-goal.md`'s `webui/server.rs` 105-110, now
`unreachable-in-this-environment` shape (a)).

### 0.6 Cover-first list for the next segment

**Nothing on this list is unwaived work any more — with one question pending, named
below.** The last `coverable` row — `cli.rs` 590, the single line this list carried into
segment 26 — is **covered** by the test in §0.4 F-64, and §0.5's remaining rows are
all argued waivers with named mechanisms. **The exception is
`session/journal_compaction.rs` 171**, which the 2026-10-17 hygiene pass found in the
wrong category (`attribution-artifact` on a line with its own `DA:` record) with a false
flip claim; its fixture is described in §0.5's row and **the recipe below that fits it is
the failing-worker one** (hold the SQLite worker so the compactor blocks inside
`db.call`, set the stop flag through `shutdown()`, release) — so this list reopens for
that one entry the moment the supervisor rules the row `coverable`. What follows is therefore kept as the record of the *recipes* that closed the
segments before it (each is reusable for a new line, but this unit has none), and the
targets it names are all closed: the failing-store / damaged-schema recipe is the
highest-yield one this unit ever had, and the two shapes that closed the last lines were
the **stalling platform endpoint** (F-64) and the **deterministic lock** (§0.6's
`database/mod.rs` fixture).

1. **The failing-store / damaged-schema recipe** — still the highest-yield one
   left, and segment 10's two biggest wins came from it (a panicking
   `db.call(..)` job kills the worker; a **buffered delta** keeps the journal's
   pending batch non-empty so its shutdown flush has something to fail on — see
   §0.5's `shutdown.rs` 293 row for the full recipe). Two other flavours:
   `Manager::test_execute("CREATE TRIGGER …")` or `DROP TABLE|VIEW` for a
   *targeted* refusal (note `entry_records` and `history_display` are views), and a
   second `rusqlite::Connection` on the store's file for a damaged schema.
   Targets: `rpc/session.rs` 1015 and its accessor group,
   `session_reset.rs` 136-141 (see the dead ends recorded below),
   `session/manager/mod.rs` 92/124/150/158/215, `session/database/*`,
   `skills/manager.rs`, `session/legacy_import.rs`, `session/fork.rs`.
**The rework closed this list by covering its last entry.** `cli.rs` 590 — which segment
26 waived "by cost" — is now **covered by a test** (§0.4 F-64; the row and its measured
mechanism are in §0.5). **There are no `coverable` rows left in this ledger**, so there
is no unwaived work for a next segment to do: the unit's remaining 202 lines are all
covered-by-test or classified, and the one line that could still move has moved.

**Segment 25 emptied it to a single line, and the honest end-state is worth stating
plainly here.** The only `coverable` line left is `cli.rs` 590, with a decided
recipe and a stated cost (§0.5): a client holding a long unary RPC while a shutdown
deadline expires in the drain's listener stage, so the watcher publishes
`FailedStopping` while the serve future is still pending. That needs an
integration-level fixture which **I did not build**, so the line stays coverable and
unclaimed. Everything else in the ledger carries a category with a mechanism: 29
lines are `platform-unmeasured` (Windows `cfg` arms whose `cfg` sites are cited),
31 `unreachable-in-this-environment`, 50 `unreachable-by-construction` and 87
`attribution-artifact` (lines llvm-cov does not instrument, each with its
neighbours' counts). **So this unit is not converging on zero uncovered lines by
writing more tests; it is converging on a complete, argued waiver ledger** — which
is what the campaign's 口径 A accepts, and the one line that could still move is 590.

**Segment 24 all but emptied this list.** Four of the five rows it carried are now
covered — the run snapshot with nothing to merge (`persistence.rs` 827), the fork
whose evidence cannot be re-pointed (`fork.rs` 202, which also covered 194), the
activity refusal (`activity.rs` 71) — and the deterministic fixture §0.6 described
converted the idle-reclamation flip into a stable claim (`database/mod.rs` 79-83).
What is left is **three lines over two rows**: `cli.rs` 590/591 and
`approval_review/provider.rs` 181. Both have their cost stated below, and neither
is a puzzle.

**Segment 23 closed the largest entry this list has carried for eight segments.** The
`session_reset.rs` `reload_failed` block (6 lines) is covered by the fixture in §0.3,
which drives the window between the store accepting the reset and the memory re-read
— see F-53 for why no input could ever reach it and why the previously recorded dead
ends were correct. What is left here is six lines over five rows.

**A deterministic fixture for `database/mod.rs` 79-82, ready to write** (F-54 — the
arm ran once under load, so it is reachable and this row can become a permanent
coverage claim instead of a flip): open the store, take a **foreign
`BEGIN IMMEDIATE`** on its file with a second `rusqlite::Connection`, and let the
worker's 50 ms idle slice fire (`reclaim_requested` starts `true`, and
`request_space_reclamation()` re-arms it). `reclaim_space_when_idle` sets
`busy_timeout(0)`, so `PRAGMA incremental_vacuum(1)` returns `database is locked` —
the `Err` arm at 79-82 — and the unit's own `reclamation_arm_tests.rs` already
measures that this SQL errors under exactly that lock. Assert the observable the arm
protects: the worker survives and reclaims once the lock is released.

**Segment 22 closed two more and disproved a third.** The completion-time drain
(`session_prompt.rs` 1357) and the checkpoint's usage baseline (`session.rs` 1015)
are covered by the two fixtures in §0.3; `session.rs` 979 left the list as
`unreachable-by-construction`, with the invariant assertion that measures it
(F-51). What remains here — 12 lines over 7 rows — is the tail this unit has not
been able to reach with a fixture that is honest about what it proves.

**Segment 21 closed four of the entries this list carried.** The creator identity on
both persistence paths (`session_prompt.rs` 1178/1617) fell to one run with a
creator set; the title command's third unavailable-model arm (`session_title.rs`
185, taking that file to zero) needed a two-model registry through
`Registry::from_models_and_auth`; and both `session_lifecycle.rs` warnings fell to
one fixture whose model the protocol layer cannot build. `recall.rs` 179 left the
list un-covered: it is a closing brace, and the recipe it carried described
different code (F-49). What is left here is the genuinely expensive tail.

**Segment 20 closed three more entries, and falsified a classification.** The
journal writer's accepted-but-unanswered append (56-58, 161-162) fell to a
hand-held command channel (§0.3); `service/mod.rs` 156-160 fell to a directly
built service; and 343 with 355-358 fell to a hot canceller thread that **F-21 had
declared unschedulable** (F-43). `provider.rs` 188 left this list the other way: it
is dead by construction (F-44), not coverable work. The only entry here whose cost
is a deliberate question is `provider.rs` 181, which needs the client's 30 s
`REVIEW_TIMEOUT` to elapse.

**Segment 19 closed two of the entries this list used to carry.** The
`journal_compaction.rs` `provider_plain` pair (248/249) fell to two plain fixtures
(`run_with_bodies` with a `text_delta` whose `text` is a number, and one with a
key outside `{text, type}`). The `database/reclamation.rs` pair (46/80) did **not**
fall and is no longer work: ten probed states left it `unreachable-in-this-
environment`, with `reclamation_arm_tests.rs` as the falsifier (F-40). Read the two
closed entries below as history, not as a work list.

2. **The exact-marker trigger** — `WHEN NEW.content_json LIKE '%…%'` refuses one
   specific write without disturbing the rest of a drain or a run; that is how
   `shutdown.rs` 253-257 fell.
3. **Direct calls below the dispatcher** (the §0.4 F-1 shadow): `commands/skills.rs`
   lines 14-15, `commands/mod.rs` guards, `session_title.rs` 185.
4. **Small accessor/lifecycle fixtures** — the pattern the earlier segments used
   repeatedly: a public accessor or lifecycle step called directly
   (`ServiceWork::activity_counts`, `CompletedRunCompactor::pending_count`,
   `Manager::initialize/shutdown`, `AgentConfig::validate_environment`,
   `InstanceFile::open`, `wait_closed`). Each was a single test for 1-3 lines and
   needed no new seam. Two remain in that family: `service/activity.rs` 71 (the
   revision check — needs a concurrent `close_streams`, i.e. a seam that does not
   exist) and `approval_review/provider.rs` 188 (the two-phase gateway).
5. **`session_reset.rs` 136-141: two routes ruled out, so do not re-derive them.**
   The arm needs `ensure_history_loaded` to fail *after* the store accepted the
   reset. (a) An **empty** `entries` table would produce exactly the
   "session has no entries or was not imported" error, but the reset refuses with
   "nothing would be left" first (`nothing_retained`, lines 111-116, itself
   unreachable-by-construction). (b) Dropping a table the read uses also breaks the
   write (`sessions`, `entries`), which lands on `reset_failed` at 101-103 instead.
   What is still unexplored: a selector combination that leaves a row set the
   *loader* rejects while the *planner* accepted it — e.g. a run terminal whose
   `run_started` the selection dropped, if that is expressible through
   `DropEffect`. Check `session_from_entries`'s validation before building a
   fixture.
6. **`service/shutdown.rs` 273 is the one stage exit left** — and F-20 says why it
   is not reachable with the fixture that took 232: `service_work.counts()` must be
   `(0, 0)` to get past the earlier stage, so the unsettled thing has to be a
   session's own runtime (`scheduler_settled()` false, or `auxiliary_settled()`
   false), and `stop_cancellation_timer` aborts the armed watchdog, so the
   auxiliary route needs a task an abort cannot drop — a **blocking** finalizer,
   which is the case `work.rs` lines 41-42 describe.
5. **Attribution re-check before waiving** — the exercise that paid off twice this
   segment: a `path` + `?`-tail or brace that reads `DA:0` may be an artifact
   (compare neighbouring counts, as in F-6) or may be genuinely reachable with a
   better fixture (F-7). Both outcomes are results; guessing is not.

### 0.7 Coordination notes and reproducibility

* **All four PLAN §5.1 gates are green on the current tree.**

```
cargo test -p future-agent                                EXIT=0  2486 passed / 0 failed / 10 ignored over all targets (lib 2423, log /tmp/agent-a-s36.log — 218 KB of it)
cargo clippy -p future-agent --all-targets -- -D warnings  EXIT=0  clean (/tmp/g30-clippy.log)
cargo fmt --check                                         EXIT=0  clean, **0 lines of diff tree-wide** (/tmp/g30-fmt.log, 0 bytes)
rustfmt --edition 2021 --check <the files this unit edited>  EXIT=0
```

**The rework re-ran all four gates on its own tree — the tree the report in §0.8 was
measured on — and they are green:**

```
cargo test -p future-agent -j 4                             EXIT=0  2487 passed / 0 failed / 10 ignored over all targets
                                                                    (lib 2424 — segment 26's 2423 plus the new test; /tmp/svc-tests.log)
cargo test -p future-agent -j 4 (instrumented, the measurement run)  EXIT=0  same counts, 0 failed (/tmp/svc-measure2.log)
cargo clippy -p future-agent --all-targets -- -D warnings    EXIT=0  clean (/tmp/svc-clippy.log, 0 warning/error lines)
rustfmt --edition 2021 --check agent/src/cli.rs              EXIT=0  clean (/tmp/svc-fmt.log, 0 bytes)
python3 .future/cov100/reconcile.py <7 AG-A scopes, --section 'Residual ledger'>
                                                             EXIT=0  MISSING 0, no STALE (/tmp/svc-reconcile.txt)
```
No production line was touched by the rework: the diff is `agent/src/cli.rs`'s new
`mod cut_short_drain_tests`, and the `crate::cli::run_from_args` call inside it is the
public entry point `main` already uses.

* **One pre-existing environment race in this crate fired once in the rework's gate run
  and is reported, not fixed** (it is in this unit's scope, but it is not this
  ticket's work): `utils::util_tests::default_config_dir_exists`
  (`agent/src/utils/mod.rs:835`) reads `default_config_dir()` — `FUTURE_HOME`, else
  `$HOME/.future` — **without** `crate::test_support::home_env_lock()`, while several
  tests in the same binary (`cli.rs`'s home tests, `tools/mod.rs`,
  `sandbox/rules.rs`, `skills/mod.rs`) set `FUTURE_HOME` to a temp dir for the body of
  their own test *and do take that lock*. Observed once: the assertion
  `dir.to_string_lossy().contains(".future")` failed because another test's temporary
  override was visible; the same test passes in isolation and in the re-run that
  produced `/tmp/svc-tests.log` and in the measurement run. The fix is the one line the
  other readers already use (`let _guard = crate::test_support::home_env_lock();`), and
  it is **not** written here because changing another segment's test is a supervision
  decision, not a rework step. The coverage consequence is the class this record
  already documents: a load-flaked test leaves its own assertion lines uncovered, which
  is why the reported total is always quoted together with the run that produced it.

**The peer-owned red that segments 14 and 15 reported is gone, and that is worth a
line because both segments' text blamed it.** `tui/src/terminal_posix.rs` was
mid-edit (unparseable) in the shared worktree then; the `tui` unit has since
finished, so `cargo fmt --check` is clean here. Nothing in this unit changed for
it — the gate simply stopped having an out-of-scope failure, which is what a shared
worktree does between runs. The earlier segments' reports were accurate about the
cause and are left in place as history rather than rewritten.

All four gates are green, in and out of this unit's scope.

**The shared *tool* is no longer mid-edit, and the fix it received is the one this
unit reported.** `.future/cov100/reconcile.py` now runs from the working tree, and
against this record it returns the same verdict as before
(`files with uncovered lines=48, uncovered lines=239, MISSING 0, no STALE`, exit 0).
The supervisor confirmed the STALE blind spot this unit found in F-35 — the half
that catches a ledger *overstating* its residual skipped any file whose uncovered
set had become empty, exactly the state a coverage campaign produces — and wired
`staletable.py` to the same implementation. `selftest.py` reports `SELFTEST OK`
(all ten sections) here. **The history below is kept because it explains why this
check was run from `HEAD` for two segments; for a reviewer reproducing any of it,
`git show HEAD:.future/cov100/reconcile.py` still works as the fallback.**

**Earlier, for two segments, one shared *tool* was mid-edit and the result rested on
its committed version.** `reconcile.py` raises `NameError: name 'claims' is not
defined` at its vacuity check — an in-progress edit by another process. The same
invocation against the committed copy reproduced the verdict quoted here
(`files with uncovered lines=48, uncovered lines=244, MISSING 0, no STALE`), which
is why the figure stood: `reconcile.py` is not in this unit's write set, so it was
read from `HEAD` rather than touched, exactly like the peer-owned `cargo fmt` reds
of segments 5/9/10/11 and the unparseable `tui` file of 14/15. **For a reviewer
reproducing this: if the tool crashes, run `git show HEAD:.future/cov100/reconcile.py`
first.**

**One measurement hazard, recorded because it produced a wrong number first
(F-28).** The instrumented run failed once on the **peer-owned**
`models::tests::registry_injects_future_models_from_disk_cache` (it passes in the
plain run and in the clean re-run; the file is `agent/src/models/`, outside this
unit's write set). Because that run exited 101, `cargo llvm-cov` wrote **no** lcov
file, and `cargo llvm-cov report` then merged whatever profiles were still in the
private target dir — it reported 261 uncovered, i.e. *the previous segment's*
figure, which is not evidence of anything. The fix is in the record: remove
`target/<dir>/llvm-cov-target` and re-run, which produced the `s19` report above
(2458 passed / 0 failed, exit 0). This is §1's `report` re-merge warning with a
concrete reproduction.

**The gates caught two defects in this segment's own test code** that the green
suite could not: `clippy::err_expect` on `.err().expect(...)` and a
`cargo fmt --check` diff. Both are fixed. The clippy fix has a coverage cost worth
recording (F-18): the obvious replacement `let Err(error) = … else { panic!(…) }`
*adds* two uncovered lines (the unreachable `panic!`), which is why the tests use
`.map_err(|error| format!("{error:#}")).err().unwrap()` — no `Debug` bound on the
success type, and no unreachable line.

**One process hazard, reported rather than hidden (F-17):** `cargo fmt` is
workspace-wide and other units edit this same worktree. Running it to fix one of
this unit's files also reformatted two files outside this unit's declared write set
(`agent/src/sandbox/mod.rs`, `cli/tests/bin.rs`) that a peer had just changed. The
changes are rustfmt-only and the peers' semantic edits are intact, but this unit's
snapshot therefore carries a formatting touch on files it does not own. From here
the unit uses `rustfmt --edition 2021 <its own files>`; treat workspace-wide
`cargo fmt` as out of scope in a shared worktree.

* **Re-run live on the final tree** (after the last doc edit, so the record cites
  a tree that still exists): `cargo fmt --check` → `EXIT=0`; `cargo clippy -p
  future-agent --all-targets -- -D warnings` → `EXIT=0` (`/tmp/g9-clippy.log`);
  `cargo test -p future-agent` → `EXIT=0`, **2434 passed / 0 failed / 10 ignored**
  summed over every target (lib 2376, all integration targets green), zero
  `FAILED`/`error[`/`panicked` lines (`/tmp/g9-test.log`). This segment changed no
  Rust after the measurement; the doc edit is the only diff, and `git status`
  confirms it.
* **One peer-owned failure appeared in this segment's first measurement run and
  did not survive a rerun**: `models::tests::registry_injects_future_models_from_disk_cache`
  (`agent/src/models/mod.rs`, owned by another unit — not in this unit's scope,
  and this unit did not edit it) failed inside the instrumented parallel run and
  passes in isolation and in the clean rerun that produced the report below. It is
  reported here rather than omitted, and it is the reason the report was rendered
  once with `cargo llvm-cov report` before the clean run replaced it.
* **Neither known flake fired in the final run**: the `cli_smoke` profile-timer
  race (PLAN §6) and `session::persistence::tests::timeout_drain_handoff_executes_queued_command`
  both passed.
* **Reviewer traps confirmed this segment** (§0.4 F-12/F-13): a `?`-tail reads
  `DA:0` even when its call's error path is *provably* taken, so "needs a failing
  fixture" is the wrong classification for those lines; and a refused journal
  event is decided from the event's own shape before any storage is touched, which
  is what makes the refusal/storage split testable.
* Disk hygiene: `target/cov-agent-a` is this worker's private target dir and is
  removed when the unit closes (PLAN §2).

### 0.8 Handoff status — the rework that closed the last line

**`cli.rs` 590 is covered by a test; the segment-26 waiver has fallen and is withdrawn.**
`review-agent.md` (F-1) falsified the "by cost" judgement with a measurement, and §0.4's
F-64 records the test that closes it: `a_cut_short_drain_is_reported_instead_of_a_clean_
stop` (`agent/src/cli.rs:923`, `mod cut_short_drain_tests`). **`coverable` stays empty —
and now nothing in this ledger rests on that kind of judgement: the one line that could
still move has moved.**

**Why the waiver was wrong, kept because the reasoning is the reusable part** (F-63,
above): the route analysis was right — the arm does need a drain stage to fail before
the RPC-listener wait — but (i) its search space was too narrow, because it looked only
for *admitted work* or an unresolved run and never at **native background work**, which
the agent's own startup creates on a cold cache with no client involved; (ii) its premise
about unary commands was **false** — `shell` blocks for up to 120 s
(`packages/rpc/src/command_policy.rs:18`) and what defuses that route is the drain's own
**cancellation**, not promptness (review P1: `"shell command cancelled"`, exit 0 in
0.1 s); and (iii) its cost was **asserted, never measured**. The review's fixture takes
~5 s; the one shipped here takes ~4.3 s and needs no RPC at all.

**FINAL MEASUREMENT — the rework (frozen tree, gates run, log names kept):**

* `cargo test -p future-agent -j 4` → **EXIT=0**, **2487 passed / 0 failed /
  10 ignored** over all targets (lib **2424** = segment 26's 2423 + the new test), zero
  `FAILED` (`/tmp/svc-tests.log`).
* The instrumented measurement run itself → **EXIT=0**, 0 failed, same counts
  (`/tmp/svc-measure2.log`).
* `cargo clippy -p future-agent --all-targets -- -D warnings` → **EXIT=0**.
* `rustfmt --edition 2021 --check agent/src/cli.rs` → **EXIT=0** (the unit formats
  its own file; no workspace-wide `cargo fmt`, F-17).
* `cargo llvm-cov -p future-agent --no-fail-fast --lcov --output-path
  /tmp/agent-svc-rework.info -j 4` **started only on a green `endgame.sh --check` load
  line** (`load: OK - 1-min 13.52 / 5-min 31.57 on 32 CPUs`, F.65) → **EXIT=0** →
  normalised to `/tmp/agent-svc-rework-norm.info` → `covlines.py total …`: covered
  **28530** / measured **28732**, uncovered **202**, **99.2970 %**
  (segment 26: 28399 / 28602, 203, 99.2903 %; baseline `8449acf6`: 26949 / 27530, 581,
  97.8896 %).
* **The delta against segment 26's report is one line and nothing else**: over the whole
  report, `DA:0 → >0` holds for `agent/src/cli.rs:590` and for no other pre-existing
  line, and `DA:>0 → 0` holds for **none** — the flips an earlier attempt of this
  measurement showed (`llm/mod.rs` 422, `sandbox/mod.rs` 906, `service/shutdown.rs` 303,
  all three documented as load-sensitive in this campaign) did not recur, so this
  report's numbers are quoted without a flip caveat.
* **The tripwire, on this report and the seven AG-A scopes**:
  `reconcile.py … --section 'Residual ledger'` → `files with uncovered lines=43
  uncovered lines=202 / mentioned in module-agent-rpc.md: 202 / MISSING (uncovered, not
  mentioned): 0`, exit 0, **no STALE block** (`/tmp/svc-reconcile.txt`).

**FINAL CLASSIFICATION — every line accounted for, nothing unwaived:**

| category | rows | lines | what it means here |
|---|---|---|---|
| `attribution-artifact` | 45 | 84 | llvm-cov does not instrument that region (macro field lines, headers, braces, lines inside `json!`); every row carries its neighbours' counts — **except one: `session/journal_compaction.rs` 171, whose `❌` marker says the category is refuted by its own `DA:` record and needs a ruling (§0.5)**. It is counted here as registered so this table still sums to the ledger. Three lines left the category in this rework: `skills/manager.rs` 1776-1778 were a never-called `flush`, not an attribution offset (F-3 of `review-agent.md`) |
| `unreachable-by-construction` | 25 | 53 | an invariant makes it unreachable; the invariant is named per row, and one (`session.rs` 979) carries a shipped falsifier test. This rework moved `skills/manager.rs` 1776-1778 in and removed `cli.rs` 590 from the ledger altogether |
| `unreachable-in-this-environment` | 11 | 31 | the mechanism this host cannot produce is named — all 11 are Windows `cfg` arms or OS-state arms (the twelfth was `cli.rs` 590 **by cost**, and it is covered now) |
| `platform-unmeasured` | 7 | 29 | Windows-only `cfg` arms; the `cfg` sites are cited in §0.5 |
| `coverable` | **0** | **0** | — |
| **total claimed** | **88** | **197** | the other 5 lines are the compaction harness rows |

**Every count in this paragraph was re-derived 2026-10-17 on the rework's report and
this revision's text** (F.58: a summary number is a claim, and the two below were already
stale by one): the tripwire returns **MISSING 0 / no STALE** (exit 0) over **43 files**;
`sumcheck3.py` sums the `n` columns to **197** and the four categories to **88 rows** —
segment 26's text read 198 / 89, before `cli.rs` 590 left the ledger; `rowscan.py` reports
**108 shipped rows / 367 claims** over this document; `audit_artifact.py` reports **0**
candidates, while `audit_rows.py` reports **3** — all three the known heuristic noise (a
row whose *mechanism* prose contains "reachable here" under an unreachable category:
`service/instance.rs` 34 and `cli.rs` 228/234-236), not new defects; `selftest.py`
reports `SELFTEST OK`.

**What this unit did across its 26 segments plus the rework**, for the record: 581 →
**202** uncovered lines (**−379** — the rework's one line closed the last `coverable`
row), with the work visible in §0.2's **29-row** measurement table (the baseline through the
rework) and §0.4's findings, whose tags now run to **F-64**. Eleven files reached zero entirely, listed with their before-counts in §0.2's per-file table, and the segment notes name the others as they fell (e.g. `session/history/paging.rs`, `rpc/broadcaster/replay.rs`, `service/mod.rs`, `rpc/commands/session_title.rs`, `rpc/session_prompt.rs`, `session/persistence.rs`, `service/activity.rs`). The most reusable results are the
falsification findings — F-7 (a fixture deadlock masquerading as unreachable code), F-43
(an "unschedulable" window hit 40/40 by a hot spinner), F-54 (a category falsified by
load, with the counter-evidence already in this unit's own probe), F-61 (a "30 s cost"
that costs 0.01 s under paused time) and F-63 above — plus F-60, which is a correction of
this unit's own reporting rather than of anyone else's.

**A second self-correction, in the same class as F-60:** the first version of this
section asserted three summary numbers from memory — a "36-row" measurement table,
"four" rows carrying shipped falsifiers, and "fourteen" files at zero. Checked
against the document they are **26**, **one** and **eleven**, and they are corrected
above. The check is now part of this unit's closing routine: every count in a
handoff is recomputed from the artifact rather than recalled.

**One thing this unit did not do, deliberately, and one it no longer asks for:**
`docs/testing/waiver-ledger.md` is outside its nine declared write paths, so the
"waived for fixture cost" convention segment 26 raised for `cli.rs` 590 was recorded in
§0.5 with a request that the supervisor add the shape rather than reaching outside the
scope it was given. **The rework withdraws that request**: `review-agent.md` (F-1)
falsified the waiver, the line is covered by a test (§0.4 F-64), the shape was never
added, and §0.5 now carries the closure instead. `docs/testing/waiver-ledger.md` is
still untouched by this unit, and it no longer needs to change because of anything here.

**Peer-owned, reported not omitted:** nothing outside the nine declared paths was edited
in this segment; no instrumented-run flake fired.

> **Round-1 record follows (revision `c173e64e`, 2026-08-27).** Figures, gate
> and ledger below are historical; §0 supersedes them.

> Source paths and coverage figures in this record refer to the measured
> revision. The subsequent storage refactor groups current implementations under
> `agent/src/session/{database,sqlite_store,history,manager}/` and
> `agent/src/rpc/broadcaster/`; the tables below retain their original evidence.

Subtree scope (frozen by `verify.py` `RUST_GROUPS["ag-rpc"]`): `agent/src/rpc/` and `agent/src/grpc/`.
Gate: `python .future/cov100/verify.py rust-module ag-rpc 99.99 docs/testing/module-agent-rpc.md coverage/agent-ag-rpc-report.json`.
Evidence token for this run: **`gate-green`** (see §8 for the exit status of that exact command).

## 1. Run identity and the exact measurement command

* Task `goal_6d61125ba837` / `todo_a6074c81c7c9` (this revision), subtree **ag-rpc**, crate `future-agent`.
* Private build dir `target/cov-w-ag-rpc3` via `CARGO_TARGET_DIR` (`cargo-llvm-cov 0.9.1` has no `--target-dir`).
  Named after the agent id, not the crate: the previous revision used `target/cov-ag-rpc2`, and a crate-named dir
  is how two workers collided earlier in this goal.
* Report consumed by the gate: `coverage/agent-ag-rpc-report.json` (gitignored, not committed).
* Upstream revision when this was measured: `c173e64e` plus this branch's edits; the measurement ran green
  (**2012 passed / 0 failed / 1 ignored**).

```powershell
$env:CARGO_TARGET_DIR = "target/cov-w-ag-rpc3"
  # THE measurement: atomic (test run + report in one invocation), every test included,
  # no exclusion list and no name filter. --test-threads=4 is a run setting, not a filter:
  # all 2012 lib tests of the crate run (1 ignored, pre-existing). Wall time ~15 min.
cargo llvm-cov -p future-agent --lib -j 3 --json `
    --output-path coverage/agent-ag-rpc-report.json -- --test-threads=4

python .future/cov100/verify.py rust-module ag-rpc 99.99 `
    docs/testing/module-agent-rpc.md coverage/agent-ag-rpc-report.json
```

The per-line view quoted in §6 comes from a **separate, non-measuring** export of the same profile data:
`cargo llvm-cov report --lcov --output-path coverage/ag-rpc6.lcov` (same `CARGO_TARGET_DIR`). It is used only to
locate code — the gate's per-file `uncovered` counts are the authoritative numbers (§2), and the two views
disagree by design (F11).

### Why this exact invocation

1. **Atomic only.** `cargo llvm-cov report` as a separate step is unsafe in this checkout: after a run whose
   test target fails, the raw profiles are consumed and the standalone `report` re-merges stale/foreign
   `.profraw`, producing an all-zero report (`0/60903` was observed and caught by the gate's own
   "broken measurement" check). One command, or no number. (The non-measuring `--lcov` export above is a
   *read* of the profile data the measuring run already produced in this same private target dir; it plays no
   part in §2 or §8.)
2. **No exclusion list.** An earlier attempt produced a report with three pre-existing load-sensitive tests
   skipped; that report read **96.59%** — the skipped tests account for ~123 covered lines in this subtree,
   so a skip list materially *understates* it. The final measurement runs every test.
3. **`--test-threads=4`.** The whole crate's tests are instrumented and run in one process; at full
   parallelism several pre-existing liveness deadlines expired (see §7 F6–F8). Lower parallelism keeps every
   test running while making the deadline-based tests reliable here. Coverage is unaffected by the setting.
4. **No name filter.** The previous revision measured `-- rpc:: grpc::` because the shared checkout did not
   compile (F2). This revision's whole-`--lib` run was green (2012 passed / 0 failed / 1 pre-existing
   ignored), so the unfiltered report the task asked for is the one consumed here.
5. **`-j 3`.** Several workers share this box; the dispatched budget is small on purpose (plan §Concurrency).

## 2. Before / after (real llvm-cov line coverage)

| measurement | lines | % | uncovered | files |
|---|---|---|---|---|
| before (dispatch baseline, `coverage/agent-uncovered-final.txt`) | 13777/14294 | 96.3831% | 517 | 16 |
| previous revision of this doc (§2 of the filtered run, `-- rpc:: grpc::`) | 14263/14598 | 97.7052% | 335 | 16 |
| previous revision, final report, all tests (`target/cov-ag-rpc2`) | 15026/15302 | 98.1963% | 276 | 16 |
| this revision, intermediate pass (same tree minus a token-only clippy fix) | 15094/15336 | 98.4220% | 242 | 16 |
| **after — this revision, final tree, all tests, `coverage/agent-ag-rpc-report.json`** | **15092/15338** | **98.3961%** | **246** | **16** |

**−30 uncovered lines this revision** (276 → 246), concentrated exactly where the task pointed — and both passes
agree on that: the intermediate pass read 242 for the same reason the final one reads 246, i.e. the known
±3-line per-file movement of `settings.rs`/`session.rs` under load (F10). Every other file's count is identical
between the two passes and to the previous revision's table.

| file | before | after | delta |
|---|---|---|---|
| `agent/src/rpc/session_prompt.rs` | 27 | **5** | **−22** |
| `agent/src/rpc/commands/session_lifecycle.rs` | 12 | **4** | **−8** |
| `agent/src/rpc/session.rs` | 20 | 20 | ±0 (read 19 in the intermediate pass — F10; a token-only clippy fix in one of its tests landed between the two passes) |
| `agent/src/rpc/commands/settings.rs` | 55 | 55 | ±0 (read 52 in the intermediate pass — F10; the file is untouched here) |
| `agent/src/rpc/mod.rs` | 34 | **34** | **±0 — see F15**: the lines this file's row listed as the
  activation-failure arms were *branch/region* artifacts and were already line-covered; the real gap was in the
  two callers above. The 9 instrumented lines the new seam adds are all covered, so the file's total grows
  1216 → 1225 and its uncovered count stays flat. |
| (other 11 files of the subtree) | — | — | unchanged |

The denominator grows because the new tests and the two `#[cfg(test)]` seams are instrumented lines inside the
same subtree; the honest figure is the uncovered count.

Per-file remainder, subtree only (`coverage/ag-rpc-json-gaps.py` re-derives it, sorted by uncovered):

| uncovered | covered/total | file |
|---|---|---|
| 55 | 532/587 | `agent/src/rpc/commands/settings.rs` |
| 54 | 1962/2016 | `agent/src/rpc/approval.rs` |
| 34 | 1191/1225 | `agent/src/rpc/mod.rs` |
| 24 | 966/990 | `agent/src/grpc/mod.rs` |
| 22 | 1857/1879 | `agent/src/rpc/protocol.rs` |
| 20 | 4001/4021 | `agent/src/rpc/session.rs` |
| 11 | 436/447 | `agent/src/rpc/commands/providers.rs` |
| 5 | 1269/1274 | `agent/src/rpc/session_prompt.rs` |
| 5 | 350/355 | `agent/src/rpc/run_snapshot.rs` |
| 4 | 521/525 | `agent/src/rpc/commands/session_title.rs` |
| 4 | 495/499 | `agent/src/rpc/commands/session_lifecycle.rs` |
| 3 | 21/24 | `agent/src/rpc/commands/skills.rs` |
| 2 | 679/681 | `agent/src/rpc/prompt_helpers.rs` |
| 1 | 227/228 | `agent/src/rpc/commands/mod.rs` |
| 1 | 237/238 | `agent/src/rpc/commands/observability.rs` |
| 1 | 246/247 | `agent/src/rpc/commands/run_control.rs` |
| 0 | 102/102 | `agent/src/rpc/commands/test_support.rs` |

## 3. Files changed — test code only

**No production line was touched**: no guard removed, no `#[cfg(test)]` wrapped around production code, no
type widened, no assertion weakened. Every change is inside a `#[cfg(test)] mod tests` block (or a test-only
helper), plus one new test module registration.

**This revision (`todo_a6074c81c7c9`)** — the two places the task named, plus one clippy fix inside the same subtree:

| file | change |
|---|---|
| `agent/src/rpc/mod.rs` | +2 `#[cfg(test)]` items and one `#[cfg(test)]` block inside `activate_persisted_session` (the activation-failure hook, §3a.3); +4 tests live in `commands/session_lifecycle_tests.rs` |
| `agent/src/rpc/commands/session_lifecycle_tests.rs` | +4 tests: the post-commit activation failure through `fork` and through `clone`, the failure injected directly on `activate_persisted_session`, and the cold (first) activation announcement |
| `agent/src/rpc/session_prompt.rs` | +1 `#[cfg(test)]` registry and its capture/`struct` items, plus the `#[cfg(test)]` capture call in `prompt_internal` (§3a.4) |
| `agent/src/rpc/session_prompt/tests.rs` | +3 tests invoking the captured production callbacks (checkpoint commit, escalation, `tool_sandboxed`), + `run_fixture_with_id` / `take_run_callbacks` / `test_checkpoint` helpers (the pre-existing `run_fixture` is now a one-line delegate) |
| `agent/src/rpc/session.rs` | clippy fix in the previous revision's test `manual_compaction_with_nothing_left_to_compact_fails_its_receipt`: `vec![…]` → `[…]` (`clippy::useless_vec`), which `-D warnings` would have failed on. Semantics unchanged (the binding is only read through `iter`/`iter_mut`/indexing), line count unchanged, and the test was re-run green. No coverage-bearing change. |

> Archived (previous revision's changed-file table (unchanged by this revision)): moved to [docs/archives/testing/module-agent-rpc.md](../archives/testing/module-agent-rpc.md) — superseded/historical text, kept verbatim with its original dates.

### 3a. The test-only seams (previous revision's two, then this revision's two)

All are `#[cfg(test)]`-only and change **no** production behaviour, default path, error message or public
API: in a non-test build the added items do not exist, and every injection point is unreachable because its
only setter/hook is `#[cfg(test)]`.

1. **`agent/src/rpc/protocol.rs` — event-persistence-worker handle.** The writer already owned its worker
   thread and command channel; the seam exposes them to tests instead of making a test race wall-clock:
   * `EventWriteCommand::Hold(Barrier)` (`#[cfg(test)]` variant, `#[cfg(test)]` match arm) parks the worker
     inside its command loop, so a test can act on the channel at a known point in the worker's lifecycle.
   * `EventBatchWriter::hold_worker_for_test()` — sends the gate and returns it.
   * `EventBatchWriter::disconnect_worker_for_test()` — drops the only command sender while the worker is
     parked, and installs a replacement whose receiver is already gone, so the writer's own `Drop` handshake
     is skipped instead of blocking on a worker that has exited.
   * `EventBatchWriter::stop_worker_for_test()` — retires the worker through its real `Shutdown` handshake and
     **joins** the thread, replacing the previous test's `sleep(200ms)` with an observed fact.
2. **No new seam was needed for `session.rs`.** `SessionPersistence` already has test-only failure injection
   (`fail_next_commit`, `kill_worker_for_test`, `reset_error` from the pre-existing test-support pattern), so
   the compaction-ticket failures are injected through the existing `fail_next_commit` hook and the real
   journal/store. Nothing in `agent/src/session/` was modified (and `agent/src/session/` is outside this
   task's declared write set — see F12 for the one arm that would need a new seam there).
3. **`agent/src/rpc/mod.rs` — post-commit activation failure (this revision).** Same shape as the two hooks
   already in this file (`GET_SESSION_PRE_INSERT_HOOK`, `RELOAD_RACE_HOOK`):
   * `ACTIVATE_PERSISTED_SESSION_FAIL_HOOK: parking_lot::Mutex<Option<String>>` (`#[cfg(test)]` static) and
     `fail_next_session_activation_for_test(&str)` (`#[cfg(test)]` private fn) arm the failure for exactly one
     session id.
   * The check sits at the **top** of `activate_persisted_session`, before `try_get_session`, so the injected
     failure has the same observable footprint as the real hydrate failure it stands in for: nothing is
     hydrated, registered, or announced, and the durable journal is untouched. The hook is consumed on use, so
     the next activation of the same id succeeds. In a non-test build the block does not exist and the
     function body is byte-identical to before.
4. **`agent/src/rpc/session_prompt.rs` — run callbacks (this revision).** The three callbacks the run task
   receives are the run's own `Arc`s; the seam only keeps a test-only handle on them:
   * `RunCallbacksForTest { on_checkpoint, escalation, on_sandboxed }` + `RUN_CALLBACKS_FOR_TEST`
     (`LazyLock<Mutex<HashMap<String, RunCallbacksForTest>>>`, keyed by session id, mirroring the file's
     existing `SCHEDULED_DEQUEUE_HOOK`) + `capture_run_callbacks_for_test(...)`.
   * One `#[cfg(test)] { capture_run_callbacks_for_test(&session_id, checkpoint_callback.clone(),
     escalation.clone(), on_sandboxed.clone()); }` block after the closures are built (the two `erased`
     clones are themselves `#[cfg(test)]` bindings). Nothing is captured in a non-test build; the clones
     therefore cost production nothing.
   * Why a capture instead of a real run: `escalation`/`on_sandboxed` are only invoked by the tools layer on a
     host with a working sandbox backend (this host has none, which is why the pre-existing
     `run_sandbox_denial_escalates_through_session_wiring` returns early), and `on_checkpoint` is **shadowed by
     the compaction ticket** in this wiring (see F16) — a compaction journal and the checkpoint callback are
     installed under the same `!is_ephemeral` condition, and whenever a journal is present
     `prepare_with_journal_and_summary` returns `Some(ticket)`, so `run_loop`'s `ticket.finish(...)` branch
     always wins and `ctx.on_checkpoint` is unreachable *at runtime*. The tests invoke the captured closures
     with production inputs and assert the effects (durable checkpoint entry, published approval exchange,
     published `tool_sandboxed` event).

### New tests this revision (7)

| test | file | what it pins down |
|---|---|---|
| `activate_persisted_session_reports_a_post_commit_failure_and_keeps_the_session_readable` | `commands/session_lifecycle_tests.rs` | the committed session's activation fails: the error is **returned** (not swallowed), the live registry gains nothing, the durable entry count **and** journal revision are unchanged, and a later activation loads the full committed transcript |
| `fork_reports_a_post_commit_activation_failure_and_keeps_the_committed_child` | same | through the RPC surface: `success:false` with `fork committed but could not be activated: …`, the child stays on disk with its history, the retry replays the same commit (`created:false`, same id, no duplicate entries) |
| `clone_reports_a_post_commit_activation_failure_and_keeps_the_committed_child` | same | the same contract for `clone`, incl. the parent's in-memory-message guard and the clone's own provenance path |
| `activating_a_cold_persisted_session_announces_the_first_activation` | same | a committed-but-not-resident session activates, is registered, and is announced on the global stream with its own id; the second activation returns the **same** `Arc` and announces nothing |
| `checkpoint_callback_commits_the_checkpoint_durably_and_reports_a_failed_commit` | `session_prompt/tests.rs` | the run's own checkpoint callback: its entry reaches the journal with `EntryType::Compaction` and the checkpoint identity in the body; a closed persistence boundary makes the callback **return** the error, and the failed commit leaves exactly one checkpoint entry behind |
| `escalation_callback_publishes_a_decidable_sandbox_request_on_the_session_stream` | same | the run's own escalation requester: the request is published to the session's subscribers (`approval_request`, `kind:"sandbox_escalation"`, the command, this session's id), the decision arrives from the session's gate and is returned as `Approved`, and `approval_decision:approved` is published for the same request id |
| `sandboxed_notifier_broadcasts_tool_sandboxed_for_the_run` | same | the run's own notifier: one `tool_sandboxed` event on the session's stream carrying `type` and the exact command |

### New tests (11) — previous revision

> Archived (previous revision's 11 new tests): moved to [docs/archives/testing/module-agent-rpc.md](../archives/testing/module-agent-rpc.md) — superseded/historical text, kept verbatim with its original dates.

### New tests (28) — previous revision

> Archived (previous revision's 28 new tests): moved to [docs/archives/testing/module-agent-rpc.md](../archives/testing/module-agent-rpc.md) — superseded/historical text, kept verbatim with its original dates.

## 4. Dimensions — evidence per dimension

| dimension | evidence in this subtree |
|---|---|
| `boundary` | full thinking vocabulary incl. unknown value; missing/empty/`None` tool scope; empty session id; 9-shape argument-payload table; 8192-byte title overflow; >128-byte skill id; odd `shell_timeout_ms` (`1`, `u64::MAX`, `0`); CJK bodies (`怎么导出？`, `缓存我`) round-tripped through projection and HTML export; a skill dir without `SKILL.md` not counted |
| `error-path` | unopenable store ⇒ typed retryable failures from 3 entry points + hydration gate; damaged store for `switch_session`; compaction-busy refusal for `delete_session`; path-traversal skill ids with zero state change; platform unreachable ⇒ fail-closed install/catalogue; `suggest` arms (oversized, empty, tool call, no completion); unsupported title locale; title request reaching the provider then failing closed; nonzero shell exit (pre-existing); previous revision: the compaction receipt commit failing (ticket `finish` → `fail`, error propagated, receipt left failed, no retry, no state advanced); the same for a no-op ticket and for the ephemeral `commit_checkpoint` path; a summarizer outage degrading to the evidence index instead of losing the compaction; a summary that cannot be charged persisted (skipped-or-priced); a cost-cache write that cannot land; a setting write that cannot land; a store that cannot accept the event batch (durable append, flush, and a later append after the shared health flag was cleared); and a worker that dies with an empty batch and with a pending one. **This revision:** activation failing *after* the caller's durable commit, asserted at both layers — the direct call (error returned, registry and journal revision unchanged, transcript still readable) and the RPC surface (`fork`/`clone` report `committed but could not be activated`, keep the committed child, and replay idempotently), plus a checkpoint whose durable commit cannot land (the callback returns the error and writes nothing) |
| `concurrency` | cancel-before-first-poll beats a 30 s timeout with the child killed; Windows job/child termination on timeout; `delete_session` mutually exclusive with in-flight compaction without leaving `deleting`; `repeated_entry_reads…` (two sequential reads against the revision-keyed cache); pre-existing lagged/closed broadcast-stream tests in `grpc/mod.rs`; previous revision: a persistence worker parked by a barrier and then disconnected, with the writer's own `Drop` proving the retirement (no sleep as a synchronisation primitive). **This revision:** the escalation callback's blocking decision loop, answered from another thread through the session's own gate (the run wiring as requester, the GUI's side as decider); the cold-activation announcement is asserted on the shared global stream while other tests broadcast on it concurrently (unrelated events skipped, `Lagged` tolerated) |
| `property` | table-driven input spaces (7 thinking levels, 9 argument shapes); SQLite round-trip write → `list_installed_skills` → `uninstall_skill` → empty; idempotence (double uninstall); read-only rule injection reaches the live rule set and appends rather than replaces; equal responses across repeated reads + cache presence; **this revision:** activation idempotence (the second activation returns the same `Arc` and re-announces nothing) and fork/clone commit idempotence under a failing activation (same child id, `created:false`, entry count unchanged) |
| `platform-cfg` | Windows-only shell timeout/job branch covered **on Windows**; the `cfg(unix)` counterparts remain `platform-unmeasured` here and are authoritative on Linux/macOS CI; `export_html` success is now platform-neutral instead of `cfg(not(windows))`; path assertions compare `canonicalize_lenient` forms so the verbatim spelling cannot make them platform-dependent; **this revision:** the escalation/sandbox callbacks are covered by invoking the production closures directly, precisely because their runtime invocation needs a sandbox backend this host does not have — the lines are covered here without claiming the host can run them |
| `serialization` | typed `ContextMessage` projection (camelCase `runId`/`blocks`/`metadata`, run id lifted out of metadata, other keys preserved); `MessageBlock` `text`; skills `InstalledSkill` JSON (`id`/`scope`/`source`/`version`/`location`); `SyncResult` four-array shape; `RpcResponse` envelope `error_code`/`error_data` (**snake_case on the wire** — asserted, see F3); pre-existing `typed_payload_encodes_real_read_command_envelopes` still covers typed-payload encoding; previous revision: legacy journal `data` shapes (string-encoded vs object vs absent vs non-object) are read without aborting the replay, and a checkpoint/ticket round-trip is asserted through the real SQLite journal. **This revision:** the checkpoint the wiring persists is decoded back out of the journal (`checkpoint_id`, `summary[0].text`), and the `session_created` / `approval_request` / `approval_decision` / `tool_sandboxed` payloads are parsed as JSON and asserted field by field |

All six dimensions have real evidence; none is `N/A` for this subtree. The only unmeasurable area is
other-platform `cfg` code (`platform-unmeasured`, §6).

## 5. Weak tests repaired

All in `agent/src/rpc/session.rs`; each asserted nothing ("should not panic" / "verify no panic") or less
than the code's contract. None was deleted without a strictly stronger replacement.

| old test | replacement |
|---|---|
| `add_session_rule_does_not_panic` | `add_session_rule_records_a_read_only_allow_and_reaches_the_live_rule_set` — decision, read/write access, live rule-set layer, append-not-replace |
| `set_permission_level_invalid` | `…_value_is_stored_verbatim` — stored value + read-back |
| `set_system_prompt_updates` | asserts `loop.system_prompt` **and** the per-run `config.system_prompt` copy |
| `append_system_prompt_does_not_panic` | asserts `"base\nappended"` and no leading newline on an empty prompt |
| `set_tools_filters` | `set_tools_keeps_only_the_requested_catalogue_entries` — filtered names + unknown name ⇒ empty |
| `disable_tools_clears` | seeds tools, then asserts non-empty→empty |
| `disable_builtin_tools` | same seeding/assertion pattern |
| `fork_does_not_panic` + `delete_session_does_not_panic` | merged into `fork_and_delete_leave_the_live_session_untouched` — `Ok` + unchanged history |
| `set_sandbox_policy_updates` | asserts `None` → `Off` → replaced by `Sandbox` |
| `set_thinking_level_updates_field` / `set_thinking_level_off` | replaced by the 7-row budget table |
| `set_ephemeral_toggles` | removed: assertion-free duplicate of the asserting `set_ephemeral` |

No `#[ignore]`/`#[should_panic]` was added or relied on in this subtree.

### Repaired in the previous revision

> Archived (previous revision's weak-test repairs): moved to [docs/archives/testing/module-agent-rpc.md](../archives/testing/module-agent-rpc.md) — superseded/historical text, kept verbatim with its original dates.

### This revision (`todo_a6074c81c7c9`): no test was repaired, and none needed it

The seven new tests were written to fail on a wrong implementation, not to visit lines; each one names a
concrete observable and was checked against the code path it claims:

* the activation tests assert the caller-visible error **and** the durable/registry state around it (a test
  that only asserted `is_err()` would pass on an unrelated failure — the injected message is asserted verbatim);
* the cold-activation test asserts an announcement *arrives*, then that a second activation announces
  **nothing** (a test that only asserted the first event would pass with a duplicate-announce regression);
* the checkpoint test asserts the journal entry (id + type + decoded body) *and* that a failed commit writes
  nothing new, so it cannot pass if the callback swallowed the error;
* the escalation/sandboxed tests assert the published payloads field by field, so a wrong session id or a
  dropped command fails them.

Two things were deliberately *not* done: no assertion was weakened anywhere (the only pre-existing test text
this revision touched is `run_fixture`, split into a delegating wrapper so a second fixture can choose its
session id — the delegated path is byte-identical in behaviour), and no `#[cfg(test)]`-only assertion was
added to production code.

## 6. Waiver ledger — every remaining uncovered file, with category and reason

Line numbers are from the report revision (§2). Categories follow `docs/architecture/channels-test-coverage.md`
as restated in `.future/cov100/plan.md`. **Confidence** says how a reviewer should treat the row: `high` =
the arm cannot be executed here at all; `medium` = reachable in principle but only through a real external
service or a production injection seam that does not exist today — these are the rows to challenge first, so
they are called out rather than buried.

| file : lines | category | reason (and what proves the neighbours ran) | confidence |
|---|---|---|---|
| `agent/src/rpc/commands/mod.rs` : 368 | unreachable-by-construction | the final `_ =>` "unknown command" arm. The first guard in `handle_command_internal` already rejects every name not in `future_rpc::command_policy::command_policy`, and a scripted diff of that table against `commands/mod.rs` shows **all 77** real commands have a routing site; the only policy string without one is `"unknown"`, which the policy function itself rejects (`packages/rpc`'s `long_work_is_not_mistaken_for_a_long_unary_rpc` asserts `command_policy("unknown").is_none()`). `dispatcher_tests::unknown_command_returns_error` covers the outer guard. | high |
| `agent/src/rpc/commands/skills.rs` : 27 | unreachable-by-construction | `_ => unreachable!()` in the skill handler: the dispatcher routes exactly the five skill `cmd_type`s, so no other value can reach this function. | high |
| `agent/src/rpc/commands/providers.rs` : 566 | unreachable-by-construction | `serde_json::to_value` cannot fail for `SuggestSkillPayload { skill: Option<SkillCandidatePayload { String, String }> }` — no layout of two strings produces a serialization error; the code comment says the same. | high |
| `agent/src/rpc/run_snapshot.rs` : 151 | unreachable-by-construction | `let Value::String(text) = … else { unreachable!() }` in `coalesce_deltas`: a pending entry is only created from an event whose `text` was already checked `is_string`, and nothing rewrites it in between. `tool_start_arguments_are_only_significant_when_present_and_non_empty` plus the pre-existing coalescing tests exercise both sides of that filter. | high |
| `agent/src/rpc/commands/observability.rs` : 237 | attribution-artifact | line 237 is the `id,` argument inside the multi-line `RpcResponse::build_fail_code(...)` for `run_snapshot_too_large`. **`observability_tests::get_run_snapshot_rejects_unknown_and_explicitly_falls_back_for_empty_or_oversize` proves the surrounding code ran** — it pushes a snapshot over `EVENTS_PAGE_BYTE_BUDGET` and asserts `error_code == "run_snapshot_too_large"`, the code that this very call produces. | high |
| `agent/src/rpc/commands/run_control.rs` : 250 | attribution-artifact | line 250 is the `session_id = %sess.session_id,` field of the `tracing::info!` expansion in `handle_abort`. **`run_control_tests::abort_works` and `abort_session_on_idle_session_cancels_nothing` both call `handle_abort` and assert its effect**, so the statement executed; llvm-cov attributes the macro's span-end field line as uncovered. | high |
| `agent/src/rpc/prompt_helpers.rs` : 728 | platform-unmeasured | line 728 is `PathBuf::from("/workspace")`, the **unix arm of the `cfg!(windows)` in the test helper `workspace()`** (test code, not production). This measurement runs on Windows, so only the `C:\workspace` arm is compiled/executed. Authoritative platform: the Linux/macOS CI legs. | high |
| `agent/src/rpc/session.rs` (`#[cfg(unix)]` bodies) | platform-unmeasured | e.g. `execute_shell_timeout_kills_the_process_group`, `execute_shell_bounds_inherited_pipe_lifetime` and the `process_group(0)`/`killpg` production branch are `#[cfg(unix)]`: absent from a Windows report entirely (neither 0% nor denominator — `verify.py blindspot future-agent` lists them). Authoritative platform: Linux/macOS CI. | high |
| `agent/src/rpc/approval.rs` (`cfg(unix)` branches) | platform-unmeasured | same: unix-only sandbox/approval branches are not compiled here. Authoritative platform: Linux/macOS CI. | high |
| `agent/src/grpc/mod.rs` : 43-52 | unreachable-in-this-environment | `serve_local` binds the per-user IPC endpoint and serves with `serve_with_incoming_shutdown(incoming, std::future::pending())` — the shutdown future is designed never to resolve, so a test can only leak a live server for the whole process, and the agent singleton lock forbids a second instance. The TCP sibling (`serve_tcp_with`, with a real shutdown) **is** covered by `grpc::tests::serve_with_shutdown_completes_cleanly` and `session_created`-stream tests. | high |
| `agent/src/grpc/mod.rs` : 1171-1173 | attribution-artifact | the `flush` body of the `Capture: std::io::Write` test double in `grpc::tests::verbose_logs_background_commands_at_trace_only`; that named test drives the same impl's `write` (the assertion is on the captured `[grpc] get_agent_info` line) and `MakeWriter` never calls `flush`. | high |
| `agent/src/rpc/commands/settings.rs` : 13-58 | unreachable-in-this-environment | the probe-failure/serialization arms of `probe_sandbox` / `probe_windows_sandbox` / `reset_windows_sandbox`: they require `crate::sandbox::platform_sandbox_probe_product()` (resp. the Windows/reset variants) to return `Err`, i.e. a host whose sandbox probe is broken, and there is no injection seam for the probe. The success path is covered by the pre-existing `probe_*` tests. Authoritative platform: a CI runner with no sandbox host installed. | medium |
| `agent/src/rpc/commands/settings.rs` : 278-299 | unreachable-in-this-environment | the `spawn`-failure arms of `handle_compact`: `spawn` is a `std::thread::Builder::spawn` result, so the arm needs OS thread creation to fail (resource exhaustion). The succeeding path is covered by `settings_tests` compaction tests. | medium |
| `agent/src/rpc/commands/settings.rs` : 500-514, 646 | unreachable-in-this-environment | the "sandbox tier requested but the probe failed" fallback and the parent-session persistence warn: both sit behind the failing-probe condition above. | medium |
| `agent/src/rpc/approval.rs` : 220-273 | unreachable-in-this-environment | the Windows capability-approval flow (`windows_request::prepare` → `ask_user` → Approved/Cancelled/Rejected) needs a Windows sandbox host that actually demands a capability approval for the command; on this host `prepare()` reports no approval needed, so the branch is never entered. The surrounding `ask_user` machinery is covered by the pre-existing escalation tests. Authoritative platform: a Windows runner with the sandbox host installed. | medium |
| `agent/src/rpc/approval.rs` : 812, 837, 842 | unreachable-in-this-environment | three `shorten_home`/quoted-path branches needing `$HOME` itself as the command path, an unterminated quote, and a `~/`-prefixed target respectively; each is reachable only with a command shape the tool-call parser must first accept. | medium |
| `agent/src/rpc/protocol.rs` : 513, 729, 735, 737, 1941 (5 statement lines; 22 on llvm-cov's line metric — see the note below the table) | unreachable-by-construction (513, 735, 737) / unreachable-in-this-environment (729) / attribution-artifact (1941) | **513** is the `unwrap_or_else` fallback of the Flush reply: every `false` return from `flush_event_batch` is immediately preceded by `health.fail(...)`, so `health.error()` is always `Some` there — the closure body cannot run, and the reachable half of that line **is** covered (`event_batch_writer_refuses_durable_writes_after_a_failed_batch` asserts the store error it returns). **735/737** is the *second* identical "journal is permanently owned by session" guard in `configure_journal`; the first guard already rejects a single-threaded rebind, so the second needs two threads to rebind between the two checks — no deterministic seam exists and adding one would be a production change for a test. **729** is the `?` on `EventBatchWriter::new(...)`, i.e. OS thread-creation failure. **1941** is the `Ok(result)` arm of a race-tolerant helper *inside a test*. The batch-writer lines this task targeted are covered: 430 (`recv` disconnected) and 455 (`recv_timeout` disconnected) by `event_batch_worker_exits_when_its_last_sender_disappears`; 461-467 (durable append refused after a failed batch) and 512-514 (flush with a pending event) by `event_batch_writer_refuses_durable_writes_after_a_failed_batch`; the snapshot read path by `a_failed_batch_commit_is_reported_and_blocks_the_read_path`. | high |
| `agent/src/rpc/session.rs` : 830, 883, 961, 1100, 1215, 1221, 1226, 1254, plus 797/798 and the test-module lines 2262, 2298, 3166, 4400, 4437, 4573, 4852 (20 on llvm-cov's line metric — see the note below) | attribution-artifact (all), with 893 and 1221 additionally unreachable-by-construction | **The compaction-ticket arms and the usage-persistence arm of this row are now covered** and have been removed from the ledger: the ticket `finish`/`fail` pair on a failed commit (`manual_compaction_receipt_failure_is_remembered_and_not_retried`), the no-op `Unchanged` ticket (`manual_compaction_with_nothing_left_to_compact_fails_its_receipt`), `commit_checkpoint` on the ephemeral path (`ephemeral_session_checkpoint_commit_failure_is_reported`), and the "summary usage could not be persisted" arm (`a_usage_write_that_cannot_land_fails_the_compaction_receipt` — a session whose stored `session_info` is not an object lets the claim succeed while the usage merge fails, which is exactly this arm's precondition) — each asserting the observable outcome, not the line. Nothing here is `medium` any more, and no injection seam beyond the pre-existing ones was invented: the arm above needed only a store shape, not new production code. What remains is all attribution, not reachable logic. **893** is the `_ => None` arm of the checkpoint `summary` filter: a freshly prepared checkpoint's summary blocks are built by this code and are always `Text`, so no non-text block can reach the fresh-commit path (a *restored* checkpoint is read by the same arm in `compaction/mod.rs`, which is exercised). **1221** is the `continue` for a payload that fails `serde_json::from_str`: the `run_events.payload` column is written only by `append_event` (which serialises a `Value`) and carries `CHECK(json_valid(payload))`, so an unparseable payload cannot exist. **1100/1106** are the "both pipe readers finished" condition and its brace in `execute_shell_at`'s poll loop: the loop can only leave with a status through `break status` (1103-1104), which requires that condition to be true — `windows_shell_reports_exit_status_and_timeout_kills_descendants` and `shell_timeout_is_clamped_to_the_documented_bounds` assert exactly that exit status and captured output, so the statement ran. **797/798** are the argument lines of a `tracing::info!` whose statement ran 4 times; **883/961/1226/1254** are brace/region ends after converging arms, and 2262/2298/3166/4400/4437/4573/4852 are lines in the `#[cfg(test)]` module that llvm-cov attributes to the zero-count `panic!` arm of a passing `assert!` (or to the untaken `cfg!` arm at 2245) — the named tests listed in §3 prove the surrounding statements ran. | medium (825-830) / high (the rest) |
| `agent/src/rpc/mod.rs` : 312, 318, 418-445, 453, 455 on the LCOV view (31 lines; 34 on llvm-cov's line metric — see the note below) incl. the test-module lines 1307, 1310, 1312, 1316, 1338-1348 | attribution-artifact (312, 318, 1307-1348) / unreachable-in-this-environment (418-445, 453, 455) | **The post-commit `activate_persisted_session` failure this task named is covered and has been removed from this row** (the `#[cfg(test)]` hook of §3a.3 plus the four named tests of §3; its caller-visible arms in `commands/session_lifecycle.rs` went with it). **This also corrects the previous revision's claim**: the lines it attributed here (the old 342-343 “activation failure/warn arms”) were branch/region artifacts — the activation statements were already line-covered by the pre-existing fork/clone tests (F15), which is why this file's uncovered count did not move while two other files dropped by 30. What is genuinely left: **312/318** is `create_session`'s foreign-journal arm — `create_session_rejects_a_foreign_broadcaster_without_rebinding_it` enters it and asserts a replacement broadcaster was installed; the unexecuted statement in that arm is the `tracing::error!` for a failed `configure_journal` (needs a store that refuses a journal for a brand-new broadcaster, and the code deliberately continues with the fresh broadcaster, so only a log line is at stake). **418-445, 453, 455** is `reconcile_provider_references`: the “no configured replacement model” warn + `continue` (a registry whose `replacement_model` returns `None` — this host always has a builtin default) and the “session configuration is busy” retry loop / persist-failure arm (needs another thread holding the short config lock, or a failing `set_model`); no seam exists and adding one would be a production change made only for a test. **1307-1348** are in the `#[cfg(test)]` module: the zero-count `panic!` arms of passing `assert!`s and the `Lagged`/`Closed` drain arms of the global-stream tests. | medium (418-445, 453, 455) / high (the rest) |
| `agent/src/rpc/session_prompt.rs` : 1063, 1470 on the LCOV view (2 lines; 5 on llvm-cov's line metric) | attribution-artifact | **The three run callbacks this task named are covered and have been removed from this row**: the `#[cfg(test)]` capture of §3a.4 hands a test the production closures, and the three tests of §3 assert their effects (a durable checkpoint entry with its decoded body, and a failed commit that returns the error and writes nothing; the published `approval_request`/`approval_decision` exchange with a decision returned from the session's gate; the `tool_sandboxed` event for the exact command). What remains are two closing braces / span ends of the wiring, whose surrounding statements the named tests ran — the file went 27 → 5 uncovered. **Caveat (F16):** `on_checkpoint` is *not* reachable at runtime in this wiring (the compaction journal is installed under the same `!is_ephemeral` condition and its ticket always wins the commit), so its coverage comes from invoking the captured production closure, not from a live compaction. | high |
| `agent/src/rpc/commands/session_lifecycle.rs` : 278, 546 on the LCOV view (2 lines; 4 on llvm-cov's line metric) | attribution-artifact | **The fork/clone post-commit activation arms are covered and have been removed from this row** (`fork_reports_a_post_commit_activation_failure_and_keeps_the_committed_child`, `clone_reports_a_post_commit_activation_failure_and_keeps_the_committed_child`, plus `activate_persisted_session_reports_a_post_commit_failure_and_keeps_the_session_readable`): the file went 12 → 4 uncovered. **278** is the `unwrap_or("")` default of `c.as_str()` in `cmd_get_fork_messages`' bare-content arm — the arm itself is covered by `get_fork_messages_handles_legacy_bare_string_content`; the default needs a fork-point content that is neither an array nor a string (a number/object), a shape the reader filters out. **546** is the closing brace of the display-cache block in `cmd_read_session_entries`, whose statements `repeated_entry_reads_are_served_from_the_cached_projection` exercises. | high |
| `agent/src/rpc/commands/session_title.rs` : 521-525 | unreachable-in-this-environment | the success arm of `handle` (`Ok(data) => RpcResponse::ok(...)`): it needs a real LLM to answer with a title. `a_real_title_request_selects_the_configured_provider_and_fails_closed` now covers everything up to and including the request (model resolution, target construction, provider client, runtime dispatch); only the returned title is missing. Authoritative platform: an integration run against the real provider. | medium |
| `agent/src/rpc/commands/session_lifecycle.rs` : fork/clone activation arms | **covered this revision — see the `session_lifecycle.rs` row above** (this line is kept only so a reader of an older revision finds the redirect). | — |
| `agent/src/rpc/session.rs` : compaction ticket arms, cost fallbacks, history-merge helper (73 lines) | unreachable-in-this-environment | **superseded by the row above** (this line is kept only so a reader of an older revision finds the redirect): the ticket arms are covered as of this revision; the remaining lines and their categories are in the `agent/src/rpc/session.rs` row. | high |
| `agent/src/rpc/commands/providers.rs` : 559-561 | unreachable-in-this-environment | the `SkillCandidatePayload` mapping in `cmd_suggest_skill` runs only when `skill_reco::suggest_skill` returns a candidate, which requires a signed-in, in-budget Jev service call; here it always returns `None` and the handler correctly answers `{"skill": null}`. | medium |

### Note on the two line views (why the two numbers differ)

llvm-cov's `Lines` metric (§2, and the gate's basis) counts *every* line whose region set is
uncovered — including the zero-count `panic!` arm of a passing `assert!` and brace ends after a converging
arm. The LCOV/cobertura exports of the same run only emit `DA:` records for a subset of those lines, so the
line numbers quoted in the rows above come from the LCOV export (`cargo llvm-cov report --lcov`,
`coverage/ag-rpc6.lcov`) and are a *subset* of the counted lines: **31 vs 34** for `mod.rs`, **2 vs 5** for
`session_prompt.rs` and **2 vs 4** for `commands/session_lifecycle.rs` (a revision ago: 18 vs 20 for
`session.rs` and 5 vs 22 for `protocol.rs`). The plan's warning applies here in the other direction as well:
do not recount uncovered lines from either export — §2's `uncovered` column is the authoritative figure, and
the LCOV numbers are for locating code. The named tests in §3 are what prove the attribution claims; a
zero-count region on a line is not evidence that the line's statement did not run (F15 is the worked example
of that mistake).

### Report blind spots (sources under the subtree with no measured lines)

`verify.py` warns that 9 of the 26 sources under `agent/src/rpc/` and `agent/src/grpc/` are absent from the
report; six are already named in §3. The remaining three, all `#[cfg(test)] mod *_tests;` units (they do not
exist in a non-test build, so they cannot hide production code, and llvm-cov emits no entry for them):

| file | category | why it contributes no data |
|---|---|---|
| `agent/src/rpc/commands/providers_tests.rs` | attribution-artifact | `#[cfg(test)]`-only module of tests for `commands/providers.rs`; its lines are counted inside no file entry of the export (the same is true of its sibling `dispatcher_tests.rs`/`settings_tests.rs`, which §3 happens to name) |
| `agent/src/rpc/commands/run_control_tests.rs` | attribution-artifact | same: `#[cfg(test)]`-only, covering `commands/run_control.rs` |
| `agent/src/rpc/session_prompt/build_user_message_tests.rs` | attribution-artifact | same: `#[cfg(test)]`-only, covering `session_prompt`'s message builder |

The other six are `commands/{dispatcher,observability,skills,settings,session_lifecycle}_tests.rs` and
`session_prompt/tests.rs`, already named in §3's file table. Nothing here understates coverage of a
*production* line: every production source of the subtree appears in the report.

### Rows a reviewer should challenge first

This revision emptied the three rows that were called out here last time:
`agent/src/rpc/mod.rs` (post-commit activation failure), `agent/src/rpc/session_prompt.rs` (run callbacks) and
`agent/src/rpc/commands/session_lifecycle.rs` (fork/clone activation arms) — all three now carry the named
tests of §3. The activation row also had to be *corrected*, not just emptied (F15). What is left to attack,
cheapest first:

1. `agent/src/rpc/mod.rs` 418-445 — the reconciliation arms. They are real, reachable code with no seam today
   (a `Registry` with no replacement model; a concurrent holder of the short config lock). A reviewer who
   builds the seam this file already demonstrates (`RELOAD_RACE_HOOK`) could move them.
2. `agent/src/rpc/commands/settings.rs` (probe failure — an environment statement, not a seam: try it on a
   runner with no sandbox host installed) and `agent/src/rpc/approval.rs` (Windows capability approval on a
   provisioned host — same).
3. `agent/src/rpc/commands/session_title.rs` (a real provider answer) and `agent/src/rpc/commands/providers.rs`
   559-561 (a signed-in, in-budget Jev service call).

## 7. Findings

* **F1 — `cargo llvm-cov report` alone is unsafe to *measure* with, in a shared checkout.** After a run
  whose test target fails, the profiles are consumed and a standalone `report` merges stale/foreign
  `.profraw` → an all-zero report (`0/60903`, caught by the gate's "broken measurement" check). Always
  measure atomically. (A `report --lcov` read of a *successful* run in a private target dir is safe and is how
  §6's line numbers were obtained; it plays no part in §2/§8.)
* **F2 — the checkout is not stable enough for a whole-crate measurement.** During this segment the crate
  failed to compile (`agent/src/sandbox/rules.rs` `Vec::len` mismatch, `agent/src/tools/mod.rs` unterminated
  character literal, `agent/src/session/repair.rs` `&str`/`String`) and the failing set rotated between runs
  (`session::database`, `session::display`, `session::fork`, `session::legacy_import`, `models`, `tools`).
  None is in this subtree. `models::tests::registry_injects_future_models_from_disk_cache` passes 68/68 in
  isolation (`cargo test … -- models::tests::`) and fails only inside the full parallel suite — pre-existing,
  owned by `agent/src/models/`, not fixed here (out of scope). **This revision's run was green**: one
  whole-`--lib` measurement run of **2012 tests** completed with 0 failures (one pre-existing `#[ignore]`),
  which is why §2's report is the unfiltered one. The crate also compiles clean for this subtree's test
  target; the shared checkout remains a moving target for *other* reasons — see F14.
* **F14 — the shared checkout still carries another worker's in-flight lint/format state.**
  `agent/src/llm/adapters/anthropic.rs:1233` has an unused, needlessly-`mut` local (two `rustc` warnings), and
  `cargo fmt -p future-agent --check` reports unformatted hunks in `agent/src/cli.rs`,
  `agent/src/config/providers.rs`, `agent/src/llm/adapters/anthropic.rs`, `agent/src/session/persistence.rs`,
  `agent/src/session/sqlite_store.rs` and others outside this subtree. Both would fail CI (`-D warnings` /
  `--check`) but are not this task's files: they are reported, not touched. The four files *this* subtree owns
  (`protocol.rs`, `session.rs`, `commands/skills_tests.rs`, `commands/observability_tests.rs`) were formatted
  with `rustfmt --edition 2021` and are now `--check`-clean. This revision adds `mod.rs`,
  `commands/session_lifecycle_tests.rs`, `session_prompt.rs` and `session_prompt/tests.rs` to the list of
  files whose formatting this task owns and checked (`rustfmt --edition 2021 --check`: all four clean), and
  removed the one clippy error inside the subtree (`session.rs` `clippy::useless_vec`, §3). `cargo clippy -p
  future-agent --all-targets -- -D warnings` still fails, but **only** on the three files named above
  (`cli.rs:69`, `llm/adapters/anthropic.rs:1236`, `session/database.rs:888`) — all outside this task's
  declared write set, reported and not touched.
* **F3 — `RpcResponse` error fields are snake_case on the wire** (`error_code`, `error_data`; no
  `rename_all`). CamelCase assertions fail; the canonical spelling is asserted by `protocol.rs`'s own tests.
  A client using camelCase would silently read `null`.
* **F4 — `SandboxTier` compared as a bare string.** `settings.rs` tests `requested_tier == "sandbox"` while
  the rest of the code uses the enum; a rename would silently disable the fallback branch (and it is one of
  the rows I could not cover). Not a defect today.
* **F5 — `TestHome` is load-bearing for skill tests.** `SkillManager::local()` resolves through the FutureOS
  home, so a skill test that forgets `TestHome` writes into the developer's real `~/.future`.
* **F6 — pre-existing liveness deadlines are too tight under instrumentation** (fixed here, in this
  subtree): `session.rs::model_context_downshift_defers_compaction_until_llm_preflight` used a 2 s timeout
  and failed with `Elapsed(())`; widened to 60 s with a comment saying the assertion is liveness, not speed.
  The claim ("selecting a model does not compact until an LLM request") is unchanged.
* **F7 — same class, fixed here**: `session_prompt/tests.rs::wait_for_run_end` polled 500 × 10 ms = 5 s and
  panicked "run did not finish within 5s"; widened to 3000 × 10 ms = 30 s (message updated to match), because
  the instrumented parallel run of the whole crate is several times slower than a single-test run.
* **F8 — one flaky test remains, not mine, not fixed**: `grpc::tests::verbose_logs_background_commands_at_trace_only`
  intermittently fails with an **empty capture buffer** (the panic message is literally empty), while passing
  in isolation. `agent/src/grpc/mod.rs` is **unmodified** in this branch (`git status` shows no change; last
  touch is commit `cb90f5da`), and the test registers its subscriber with `with_default`. Nothing else in the
  crate calls `set_global_default`; the empty buffer points at `tracing`'s thread-local subscriber /
  callsite-interest behaviour under concurrent subscriber registration elsewhere in the suite. It did **not**
  fire in the final measurement run, so the report includes it. It should be diagnosed by someone who can
  change that test (the fix is likely to assert on a signal the test owns rather than on a thread-local
  capture).
* **F9 — a test was passing on the wrong error, and its name was wrong too (fixed in the previous revision).**
  `compact_provider_failure_broadcasts_and_errors` never persisted its session, so the compaction claim
  failed with "session does not exist" before the summarizer ran; the assertion (`is_err()`) held anyway.
  Persisting the transcript exposed the real contract: a summarizer that *errors* does **not** fail the
  command — `prepare_with_handoff_summary` degrades to the deterministic evidence index and commits it with
  `summaryOutcome.status = "evidence_only"` and the provider error as `fallback_reason`. The same class of
  false pass applied to `compact_checkpoint_commit_failure_broadcasts_and_errors` (same missing claim). Both
  are rewritten with the claim succeeding; see §5.
* **F10 — per-file coverage of an untouched file moves between identical runs of untouched code.**
  This revision produced the cleanest measurement of the effect yet, because it ran the *same* command twice on
  a tree that differed only by a token-level clippy fix in one test: `agent/src/rpc/commands/settings.rs`
  measured **52 uncovered (535/587) then 55 (532/587)**, and `agent/src/rpc/session.rs` **19 then 20**
  (`LF` 4021 vs 4019 — the fix's array literal accounts for at most 1 of those lines). Both files are
  untouched by this task and neither delta can come from the code. The previous revision saw the same ±3 on
  `settings.rs` (52 ↔ 55) and a ±1 on `mod.rs`. Its uncovered set is dominated by probe/environment branches,
  so the cause is a load- or order-dependent branch (a cached sandbox probe) rather than a code change — a
  3-line movement on this box is not evidence of anything. **What is *not* noise**: the two files this
  revision targeted moved by −22 and −8 in *both* passes, and the three target files' `DA:0` line sets are
  identical in both LCOV exports.
* **F11 — llvm-cov's line metric and its own LCOV export disagree on this subtree.** For the same run the gate
  counts 34/5/4 uncovered lines in `mod.rs`/`session_prompt.rs`/`session_lifecycle.rs` (and 20/55 for
  `session.rs`/`settings.rs`), while `--lcov` emits 31/2/2 `DA:0` records (and 18/53) — the same held a
  revision ago (`session.rs` 20 vs 18, `protocol.rs` 22 vs 5). The gate's figure is the authoritative one and
  §2 uses it; §6 quotes LCOV line numbers only to locate code (the export quoted is regenerated from the final
  pass' profile data, so its line numbers match this revision's tree). A reviewer who needs exact line sets
  should use the LCOV/cobertura export of the same report revision and not the JSON `segments`. **Direction of
  the error matters for the *seam* rows**: for the three rows this revision emptied, the LCOV view is *more*
  favourable (fewer lines) than the gate's, so the claims in §6 rest on the named tests and their assertions,
  not on either line list.
* **F12 — the compaction usage-persistence arm needed no new seam after all.** The previous revision judged
  `session.rs` 825-830 (`ticket.fail` when the summary usage cannot be persisted) "medium: needs a
  `SessionPersistence` failure hook". It is reachable with the *existing* hooks: the claim in `admit` only
  needs the session row, and `update_session_info_fields` fails when the latest `session_info` content is not
  an object — a store shape a test can create with `manager.save`. Covered by
  `a_usage_write_that_cannot_land_fails_the_compaction_receipt`. No production hook was added, and
  `agent/src/session/` remains untouched.
* **F13 — a schema constraint makes one arm unreachable by construction.** `agent/src/rpc/session.rs` 1221
  (the `continue` for a journal payload that fails `serde_json::from_str`) cannot run: `run_events.payload`
  carries `CHECK(json_valid(payload))` (`agent/src/session/database.rs`) and is written only by
  `append_event`, which serialises a `serde_json::Value`. That is a property of the store's schema, not of
  this code, so the arm should be kept as defence in depth and waived rather than deleted.
* **F15 — the previous revision's `rpc/mod.rs` row was a region-view artifact, and the real gap was in the
  callers (corrected here).** The row claimed `activate_persisted_session`'s "failure/warn arms" needed a
  seam to reach. In the code there are only two failure expressions in that method, the `?` on
  `try_get_session` and the `ok_or_else`, and both were **already line-covered** by the pre-existing
  fork/clone tests (a fork of a deleted parent, etc.). What was actually uncovered was the *callers'* handling
  of a post-commit activation failure (`commands/session_lifecycle.rs` 602-606 and 653-657), which this
  revision covers. That is why `mod.rs`'s uncovered count is flat (34 → 34) while the two files the seam
  drives dropped by 30 lines together: the 9 instrumented lines the new `#[cfg(test)]` seam adds are all
  covered, and no production line's status changed. **Lesson for a reviewer chasing a region list**: a
  zero-count region on a line is not evidence that the statement on it did not run.
* **F16 — the session wiring's `on_checkpoint` callback is shadowed by the compaction ticket (found here).**
  `prompt_internal` installs `on_checkpoint: (!is_ephemeral).then_some(checkpoint_callback)` and
  `compaction_journal: (!is_ephemeral).then(|| …)` under the same condition. In `run_loop.rs` (both the
  automatic site and the provider-limit recovery site) the commit is
  `if let Some(ticket) = &compaction_ticket { ticket.finish(…) } else if let Some(commit) = &ctx.on_checkpoint { commit(…) }`,
  and `prepare_with_journal_and_summary` returns `Some(ticket)` for **every** admission when the journal is
  present (`Replayed` always, `Fresh(Some(_))` once `admit` claims an operation; `Fresh(None)` only when no
  journal was passed). So in this wiring the callback can never be called at runtime, and the checkpoint is
  always committed through the ticket instead. This is not a defect — the checkpoint does get committed, and
  the ticket path is tested — but it means the callback's coverage must come from invoking the closure
  directly (which this revision does, with a real checkpoint and a real persistence boundary) and that the
  field is dead weight in this wiring. Removing it would be a production change outside this task's remit
  (“if a place can only be done by changing production behaviour, stop and report”), so it is **reported
  here** rather than touched; a follow-up could either delete the callback or make it the actual commit path
  for the ephemeral/journal-less case (`!is_ephemeral` is false there, so today the `else` branch is `Ok(())`).

## 8. Gate status — `gate-green`

The exact command from §1 was run against the §2 report:

```
future-agent/ag-rpc: 98.3961% lines (15092/15338) across 17 files in 2 dir(s),
                     246 uncovered line(s) in 16 file(s), target 99.99%
  every uncovered file (16) is waived with a category in docs/testing/module-agent-rpc.md
PASS  (exit 0)
```

The run behind that report was green end-to-end (**2012 passed / 0 failed / 1 pre-existing `#[ignore]`**), so
the number is not a partial-profile artifact.

The subtree is **not** at 99.99%, so this completion rests on the waiver half of the acceptance contract:
**all 16 files with uncovered lines carry a category-bearing row in §6**, which is what the gate's
`_waiver_gate` requires (it names every file by repo-relative path and a category on the same line). The gate
also rejects file-level `OPEN` declarations, so none remain — instead §6 marks each row's confidence and §6's
closing list names exactly which rows a reviewer should try to falsify. The failure modes are reported, not
hidden: 246 lines remain uncovered, none of them claimed as covered, and the three rows this revision was
asked to attack no longer carry a `medium` claim (the `medium` that remains sits on the reconciliation arms of
`mod.rs` and the environment rows, which this task explicitly kept out of scope).

Structural checks that also pass: every uncovered file is named with a repo-relative path, no row relies on a
bare filename, `verify.py doc docs/testing/module-agent-rpc.md 6` passes, and no test in this subtree is
`#[ignore]`d or assertion-free (`0` `#[ignore]` matches under `agent/src/rpc/`). The goal-level audit docs the
`verify.py weak` / `verify.py skipped` commands look for (`docs/testing/weak-test-audit.md`,
`docs/testing/disabled-test-audit.md`) do not exist yet; they aggregate every module and are outside this
task's declared write set, so this document states the subtree-level fact instead of creating them.

**Evidence tokens for this handoff: `lines-100-or-waived`, `dimensions`, `weak-tests-fixed`.** Reading them
against §2/§6: the subtree is not at 100% but every uncovered line is waived with a category (`lines-100-or-waived`);
all six `dimensions` have named evidence in §4 (and this revision added error-path, concurrency, property,
serialization and platform-cfg cases); and this revision repaired no weak test because it wrote none — the
`weak-tests-fixed` evidence for the module is the repair table in §5 plus the check paragraph that follows it.

## 9. Handoff summary

* **Artifacts**: this document; `coverage/agent-ag-rpc-report.json` (the gate report, this revision);
  `coverage/ag-rpc6.lcov` (per-line detail of the same profile data, `--lcov`, non-measuring);
  `coverage/ag-rpc-json-gaps.py`, `coverage/ag-rpc-gaps.py`, `coverage/ag-rpc2-lcov.py` (re-derive §2's table /
  the `DA:0` line sets); test sources listed in §3.
* **New versus the previous revision** (`todo_a6074c81c7c9`): the previous revision had built the two seams for
  `session.rs`/`protocol.rs`. This one adds the two the task named — the id-gated
  `ACTIVATE_PERSISTED_SESSION_FAIL_HOOK` in `rpc/mod.rs` (post-commit activation failure, §3a.3) and the
  run-callback capture in `session_prompt.rs` (§3a.4) — plus **7 tests** and the resulting **−30 uncovered
  lines** (276 → 246; `session_prompt.rs` 27 → 5, `commands/session_lifecycle.rs` 12 → 4), with the `mod.rs`
  row corrected rather than emptied (F15) and a new finding about the `on_checkpoint` wiring (F16).
* **Rejected**: (a) waiving reachable lines as `unreachable-by-construction` — the checks that such a claim
  would need to pass are in §6 only where the evidence exists; (b) building the report from a skip list —
  measured at 96.59%, i.e. it understated the subtree by ~1%; (c) `cargo llvm-cov report` as a separate step
  to *produce* §2 (it is used only to read the line sets of an already-produced private-dir profile); (d) a
  failure hook in `agent/src/session/persistence.rs` — outside this task's declared write set, and F12 shows
  it was not needed; (e) making `EventBatchWriter::sender` an `Option` so the writer could drop it —
  considered and rejected in favour of a seam that leaves the production field alone; (f) **this revision** —
  (i) refactoring the checkpoint closure into a private helper instead of a `#[cfg(test)]` capture (the task
  requires the seam to be `#[cfg(test)]`-gated), (ii) driving `escalation`/`on_sandboxed` through a real
  sandboxed run (needs a sandbox backend this host does not have, which is why the pre-existing test
  early-returns), (iii) arming the activation hook by a plain `Option<String>` for the *child* id of a fork
  (the id is minted inside the commit) — the tests arm the failure on the id the first call returned and
  replay the idempotent commit instead, (iv) deleting the dead `on_checkpoint` wiring (production change,
  out of remit — reported as F16).
* **Uncertainty**: (a) the remaining `medium` rows in §6 (settings.rs probe, approval.rs Windows capability,
  the `mod.rs` reconciliation arms, session_title real answer, providers.rs Jev call) are environment/seam
  statements, not proofs of unreachability — they are listed explicitly so a reviewer can overturn them;
  (b) F16 is a *reading* of the wiring plus the compiler-visible types, not an observed runtime miss — it is
  falsifiable by a test that makes `prepare_with_journal_and_summary` return `Compacted` with `ticket: None`
  while a journal is installed; (c) run-to-run movement on this box is ±3 lines per file (F10), so the
  per-file deltas of ≤1 in the untouched files of §2 mean nothing; (d) the attribution claims for brace/macro
  lines rest on the named tests in §3 running the surrounding statement, not on a line-level proof.
* **Next useful check**: (1) re-measure with the §1 command and confirm `session_prompt.rs` stays at 5 and
  `commands/session_lifecycle.rs` at 4 — a rise would mean the new tests pick up state from the parallel suite
  rather than from the seams; (2) attack the `mod.rs` reconciliation arms (the cheapest real gap left in this
  subtree) with a seam of the same shape as `RELOAD_RACE_HOOK`; (3) the §6 environment rows
  (`settings.rs` probe, `approval.rs` Windows capability) need a host with a sandbox backend, not a seam.
