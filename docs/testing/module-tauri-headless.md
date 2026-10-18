> **`headless/mod.rs` 182 IS FOLDED IN — this is no longer a measurement with a known-covered line
> in it (2026-10-18, TAU-D27, ticket `todo_fc4a164262ba`).** D20's fixture
> (`tests/headless_cli.rs::a_login_change_while_supervising_stops_the_run_with_that_reason`,
> commit `8efb4fa0c`) closed the line, and the **two-leg** measurement of
> `.future/cov100/pinned/union-d26.info` reads it `DA:1` (§2.13). That run also moved two
> documented oscillators, so the figure there is **236**, not the **237** the earlier probe
> predicted: `238 − 182 − agent_host.rs 265 − server.rs 533 + server.rs 539 = 236`. §2.13 accounts
> for every one of those four numbers and §17 for the ledger work. That is the **previous
> revision**; the merged tree measures **241** (§18), and the difference is `main`'s arms, not a
> regression — the banner below says so in one sentence.
# Module: `desktop-tauri-headless` (`desktop/src-tauri`, headless/agent-supervision/terminal slice)

> **FIGURE (2026-10-18, TAU-D31): 241 uncovered, measured at `16ea6f388` — the merged tree.**
> Report `.future/cov100/pinned/union-refresh4.info` (5 853 166 B, 178 `SF:` records), the
> post-merge two-leg union (gui leg `rc=0`, headless leg `rc=0`, whole crate, `cargo llvm-cov
> clean` between legs, unioned by `lcov-union.py`; the recipe is the `tauri-remote` record's §14.3
> and the post-merge reading is its §15). It is the union the `tauri-appdata`, `tauri-remote` and
> `tauri-shell-seg2` records publish from too — one run, four pairings, differing only in the
> scope filter.
> `reconcile.py` over the twelve scopes reads **MISSING 0, COVERED 0, ABSENT 0**, and all 241
> lines are classified: **240 in §4.1 with a mechanism and 1 OPEN in §4.2** (the force-quit
> waiter brace, whose seam is named — §18.5). §18 is the re-derivation.
>
> **236 → 241 is `main`'s new arms, and the rows moved with them.** The merge added 73 lines to
> `agent_supervisor.rs` in seven hunks (`git diff 0b7b2ca17..HEAD`): `on_close_requested`'s
> `QUIT_CONFIRMED` early return (+4), the force-exit watchdog block
> (`FORCE_QUIT_REQUESTED`/`spawn_force_exit_watchdog`/`force_exit_process`/`arm_force_exit`) plus
> the head of `request_quit`'s `force` arm (+40), the `let operation = if force` branch and its
> changed guard (+2), `main`'s own watchdog test (+13), the `force-maintenance` fixture's test
> registration (+5), a one-line fixture edit (+1) and that case's body (+8). Five of the new
> lines are uncovered in this union (the `cfg(not(test))` exit body and the watchdog
> spawn-failure branch), which is the whole of the increase: **42 + 5 = 47** for that file. The
> other 42 statements are the same code at new positions — `confirm_quit` +4, the waiter's
> dialog +43, the fixture's timeout guard and unknown case +69 — so §4.1's rows were
> **re-derived, not renumbered**: §18 tabulates old → new for every row this segment touched.
> No other file moved. **The twelve scopes are byte-identical at `16ea6f388` and `HEAD`**
> (`git diff 16ea6f388..HEAD` over them is empty), so this is a measurement of the tree the
> reader has, not of a branch that no longer exists (F.101).
>
> **Why 236 at the previous revision.** That figure was **236 at `0b7b2ca17`** (report
> `union-d26`). Against the 238 before it: `182` closed (−1); `agent_host.rs` 265 read `DA:1`
> again (−1, its sixth reading and not claimed); `server.rs` 533 and 539 traded places once more
> (533 covered, −1; 539 returned, +1). `238 − 1 − 1 − 1 + 1 = 236`. §2.13 tabulates that per
> file; each mover is an oscillator §4.1 has named for five measurements, and the ledger claims
> only the member the cited report shows.

> **ARTIFACT STATUS (2026-10-18, TAU-D31).** The current pin is
> `.future/cov100/pinned/union-refresh4.info` (5 853 166 B, 178 `SF:` records, md5
> `5fb223725194aed41049418eaa3e1224`). Unlike the two pins this record replaced it is **not** a
> branch-only artifact: it was rendered on the merged tree and the twelve scopes have not changed
> between `16ea6f388` and `HEAD`, which is checked mechanically in §18.1 rather than asserted.
> The earlier pins stay for the figures this record still explains:
> `pinned/union-d26.info` (the 236 revision, 733 709 B, 173 `SF:` records),
> `pinned/tauri-headless-237-union.info` (the 237 revision) and
> `pinned/union-postmerge2-green.info` (the 238 revision). Note for readers of
> commit `06beded3d`: it recorded "t15 reads 244" because it sampled TAU-D15's **first draft**
> union while it was being written; that draft was discarded (its harness cost 8 residual lines
> instead of 3, §12.3) and the 244 describes no file on disk. The `pinned/` directory exists
> because storing these under `/tmp` was luck, not design — F.100 is why.

Status: **see §18** — the current figure is **241 uncovered at `16ea6f388`** (the merged tree),
of which **240 are classified with a mechanism and 1 is OPEN** (§4.1, §4.2). §2 is the earlier history
(359 → 300 → 283 → 270 → 254 → 240 → 237 → 238 → 233 → 232 → 238 → 236 → 241), kept as
measured history and cited where it explains a row; the step from 236 is `main`'s new arms
(§18.1), not a regression. All figures come from the
**merged gui + headless** lcov run, which is the only honest number for this crate
(§1).
This record covers the slice that one unit owns:

| scope | files |
|---|---|
| `desktop/src-tauri/src/headless/` | `mod.rs`, `agent.rs` |
| `desktop/src-tauri/src/terminal/` | `pty.rs`, `session.rs`, `server.rs`, `manager.rs`, `shell.rs`, `cwd.rs` (+ `test_support.rs` fixtures) |
| entry/discovery adapters | `cli_tool.rs`, `agent_host.rs`, `agent_supervisor.rs`, `skills_bootstrap.rs` |

Owner: `todo_a36bdbba13fb` (TAU-D), goal `goal_160bc247aa63`. Write scope:
exactly the paths above plus this file. `docs/testing/module-tauri.md` (the
peer unit) owns the rest of `desktop/src-tauri`.

## 1. Run identity and the exact measurement

> **CLOSE-OUT (2026-10-18, TAU-D19): this module is complete at 232 for the write
> set it was given.** 231 lines are classified in §4.1 with a mechanism each and
> **one** (`headless/mod.rs` 182) is reachable and OPEN in §4.2. It is not waived:
> closing it needs `desktop/src-tauri/tests/headless_cli.rs` in the unit's write
> set, which was refused in five turns (TAU-D14, -15, -16, -18, -19; D17 spent its
> turn on the probe instead). The 口径 A
> claim for this scope is therefore *"232 lines named, 231 registered unreachable,
> 1 blocked by test-scope"* — **not** 100 %, and the exception is this line. §16.1
> has the measurement that closed the last waiver worth questioning; §4.2 has the
> file and the ~25 lines it would cost.

Baseline revision `8449acf6` (campaign start). Host: macOS 15 (arm64), rustup
toolchain `1.97.0-aarch64-apple-darwin`. Metric: **per-line LCOV `DA:<n>,0`**
(`.future/cov100/covlines.py`), never the `summary` block.

`desktop/src-tauri` is *not* a workspace member and is built in **two feature
flavors**; CI runs both (`Rust tests (Linux) (desktop gui|headless)`) and they
compile different code — so a single-flavor number is not a coverage number for
this crate (`.future/cov100/TAURI.md`). One atomic invocation, both flavors:

```bash
cd desktop/src-tauri
for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-tau
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder
# --test-threads=1 is MANDATORY: the lib suite deadlocks in parallel on this host
# (HOME<->MOCK lock-order inversion, .future/memory/testing-gotchas.md).
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tauri-tau-lcov.info   # merges BOTH profraw sets

python3 .future/cov100/covlines.py total  /tmp/tauri-tau-lcov.info --under desktop/src-tauri/src/headless \
                                                                    --under desktop/src-tauri/src/terminal \
                                                                    --under desktop/src-tauri/src/cli_tool.rs \
                                                                    --under desktop/src-tauri/src/agent_host.rs \
                                                                    --under desktop/src-tauri/src/agent_supervisor.rs \
                                                                    --under desktop/src-tauri/src/skills_bootstrap.rs
python3 .future/cov100/covlines.py detail /tmp/tauri-tau-lcov.info --under desktop/src-tauri/src/terminal
```

The script that runs exactly this is `.future/cov100/measure-tau-tau.sh` (first
segment) and `.future/cov100/measure-tau-tau2.sh` (second segment, which deletes
its own private target dir); their logs are `/tmp/tau-measure.log` and
`/tmp/tau-measure2.log`. `cargo test --lib <filter> -- --test-threads=1` (no
instrumentation) is the inner loop.

Known-host failures that are **not** this unit's regression and must not be
"fixed" here (`.future/cov100/TAURI.md`, `.future/memory/testing-gotchas.md`):
the rotating `remote_peer::transfer*` / `commands::remote_peer::*download*` /
`sandbox_availability` failures under one serialized run.

## 2. Before / after (gate metric: per-line `DA:0`)

_Before_ = the supervisor's merged baseline `.future/cov100/baseline/tauri-merged-lcov.info`
(revision `8449acf6`). Two measured *afters*:

* **after-1** = `/tmp/tauri-tau-lcov.info` (first segment's tests; §3.2): gui lib
  suite **1965 passed / 4 failed / 12 ignored**, headless **1574 passed / 5 failed / 12 ignored**.
* **after-2** = `/tmp/tauri-tau2-lcov.info` (second segment's tests added; §3.1,
  run by `.future/cov100/measure-tau-tau2.sh`, log `/tmp/tau-measure2.log`):
  **gui lib 1980 passed / 0 failed / 12 ignored**, headless **1592 passed / 2 failed / 12 ignored**.
  The two headless failures are
  `remote::tests::runtime_tests::start_runs_the_full_bridge_and_stop_winds_it_down`
  and `remote_peer::transfer_tests::a_chunk_write_on_a_dead_socket_is_recorded` —
  the documented, host-wide, rotating `remote*` family
  (`.future/cov100/GOTCHAS.md`); neither is in this unit's scope. A copy of the
  lcov is kept at `.future/cov100/tauri-tau-after2-lcov.info`; the private target
  dir was deleted by the same script.

| file / scope | before | after-1 | after-2 | after-2 / measured |
|---|---|---|---|---|
| `src/terminal/server.rs` | 75 | 70 | **59** | 59 / 1836 |
| `src/terminal/pty.rs` | 42 | 21 | **18** | 18 / 458 |
| `src/terminal/session.rs` | 26 | 20 | **14** | 14 / 727 |
| `src/terminal/manager.rs` | 20 | 8 | 8 | 8 / 568 |
| `src/terminal/shell.rs` | 11 | 11 | 11 | 11 / 153 |
| `src/terminal/cwd.rs` | 4 | 4 | 4 | 4 / 280 |
| `src/headless/mod.rs` | 160 | 105 | **104** | 104 / 806 |
| `src/headless/agent.rs` | 4 | 4 | 4 | 4 / 25 |
| `src/cli_tool.rs` | 50 | 15 | 15 | 15 / 382 |
| `src/agent_host.rs` | 26 | 24 | 24 | 24 / 272 |
| `src/agent_supervisor.rs` | 108 | 81 | 81 | 81 / 233 |
| `src/skills_bootstrap.rs` | 26 | 17 | 17 | 17 / 171 |
| **scope total** | **552** | **380** | **359** | 359 / 5911 |

**193 lines covered** by the two segments (552 → 359). The `measured` column
grows with each added test (the new test code is itself instrumented):
`terminal` 3895 → 4186 → 4279, `headless` 604 → 818 → 831, `cli_tool` 265 → 382,
`agent_supervisor` 183 → 233. The *after* numbers are therefore not comparable to
the baseline by percentage alone; the uncovered-line counts above are.

Reproduce the split:

```bash
python3 .future/cov100/covlines.py files  /tmp/tauri-tau2-lcov.info \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/terminal \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/headless \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/cli_tool.rs \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/agent_host.rs \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/agent_supervisor.rs \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/skills_bootstrap.rs
```

(`--under` is matched against the path with the `/Users/geilige/future-os/`
prefix stripped, so a run from the worktree needs the `.worktrees/…` form.)

**Scope honesty:** this checkout is shared with three peer units (their edits and
tests are in the same tree), so the *whole-crate* totals of this lcov include
their in-flight code and are not comparable to the baseline revision. The table
above is per-file for this unit's scope, which is comparable: every `before`
value is the same file at `8449acf6`.

### 2.0 Measurement hazard inside this scope (report it, do not hide it)

`terminal/server.rs` **533** and **539** — the `break` of the pump's "the shell is
gone, so writing this input frame failed" arm, one for a `Binary` frame and one
for `Text` — are **not stable**: covered in the after-1 report, uncovered in
after-2, with no code change in that region. The mechanism is a genuine race, not
a stale report: the pump ends the tunnel when it receives `SessionEvent::Exited`,
and those two lines are reachable only if a client input frame is processed
*before* that event in the same `select!`. The real-socket e2e
(`terminal::server::end_to_end::the_pump_answers_pings_carries_input_and_ends_with_the_shell`)
reaches that ordering only when the shell happens to have died before its input
frame is dequeued, so the pair flips between runs — the same ±N hazard the
supervisor flagged for `channels/feishu_ws.rs:254`. Consequence: **this unit's
residual is 359 ± 2**, and a reviewer comparing two runs must expect those two
lines to move. They are classified `unreachable-in-this-environment` in §4.1 with
that mechanism.

**Attribution, corrected by TAU-D10.** The oscillation belongs to **533 and 539**,
and the pair is named as such in §4.1 since this measurement. The line the ledger
used to cite for it — **535**, the closing brace of the `Binary` decode arm — is
not one of the movers: over **fifteen readings** (five runs of
`terminal::server::`, six runs of the single pump test, and three full-suite
artifacts) 535 read **0 every time, including the two runs in which 533 itself
executed**; 533 was covered in 2/5 + 1/3 and 539 in 1/5 + 1/3. So a reader who
checked the cited line and found it uncovered was reading a true measurement of
the wrong line.

**Where the three lines stand in the pinned union** (`tauri-tau-t12-union-lcov.info`): **533 reads
`DA:1` (covered)**, 539 reads `0`, 535 reads `0`. So the row claims **535 and 539** — the two that are
uncovered in *this* pin — and **533 needs no row, because it is covered here**. Corrected 2026-10-18
after the relay pointed out that an earlier draft of this paragraph said 533 "now has one": it does not,
and the tool is right to report `MISSING 0` without it.

**The obligation that follows, stated so the next pin cannot be misread**: 533 is one of the two
historical oscillators (covered in 2/5 + 1/3 runs), so a future pin may read it `DA:0`. **If that
happens, a row claiming `terminal/server.rs` 533 must be created** — the mention in this paragraph and in
`§4.1`/`§4.2`/`§8.3` is prose, and `reconcile.py` counts a mention anywhere in the document, which is how
an unclassified `DA:0` line can pass `MISSING 0`. A mention is not a claim.

### 2.1 The second segment, in the same metric

The `after-2` column of §2 **is** the second segment's measurement; it was taken
by `.future/cov100/measure-tau-tau2.sh` (log `/tmp/tau-measure2.log`, lcov
`/tmp/tauri-tau2-lcov.info` plus a copy at
`.future/cov100/tauri-tau-after2-lcov.info`), which deleted its own private target
dir in the same invocation. It reports **359** uncovered lines for this scope,
i.e. **21 fewer** than after-1.

Uninstrumented evidence (the compiles-and-passes half of the claim), from
`cargo test --lib -- --test-threads=1 terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::`:

```
test result: ok. 189 passed; 0 failed; 0 ignored; 0 measured; 1803 filtered out
```

The lines that segment was written for (all of them were OPEN rows of §4.2 before
it) and the test that covers each:

| line(s) | file | test |
|---|---|---|
| 334 | `terminal/pty.rs` | `the_ps_based_session_query_parses_and_survives_a_broken_ps` (its fixture `ps` now also prints a single-column line) |
| 448, 450 | `terminal/pty.rs` | `wait_for_exit_answers_a_cursor_query_and_then_reaps_the_child` |
| 656-662 | `terminal/session.rs` | `draining_a_quiet_viewer_waits_out_idle_moments` |
| 534, 547 | `terminal/server.rs` | `the_pump_carries_binary_input_and_ends_on_close` (unix) |
| 2397-2404 | `terminal/server.rs` | `client_frames_carry_every_length_form` |
| 2500-2502 | `terminal/server.rs` | `the_fake_socket_shuts_its_writer_half_down_cleanly` |
| 343 | `headless/mod.rs` | `an_accepted_platform_key_needs_no_terminal` |

Those are assertions about behaviour, not line touches: the frame test decodes
every masked payload back and checks both extended length prefixes, the pump test
asserts the markers reached a `cat` child's stdin (so the bytes cannot be the
shell's own echo alone) and that the shell stays listed after the viewer closes,
and the drain test asserts the wait really lasted to its deadline.

The whole-crate suite is **not** green on this host and never has been: the
rotating `remote*` family (`remote_peer::transfer_tests`,
`commands::remote_peer::*download*`, `sandbox_availability`,
`remote::tests::runtime_tests`) fails on a clean `origin/main` too
(`.future/cov100/GOTCHAS.md`, "desktop(src-tauri) 整套 lib 测试在本机 macOS 会死锁 + 2 个
host 相关的确定失败"). after-1 recorded 4 + 5 such failures and after-2 0 + 2; none
is in this unit's scope, and **this unit's scope is green in both flavors** (the
scope-filtered run above, and 1980/1980 in the after-2 gui library suite).

**Post-measurement edits (unmeasured, test code only).** Two regions of
`server.rs` were open after after-2 and needed no fixture: **548**
(`Some(Ok(_)) => {}`, "any other client frame is ignored") is covered by adding an
unsolicited `Pong` frame to the unix pump test, and **533/539** are the
oscillating pair of §2.0 — left alone deliberately, since forcing them would mean
asserting a race. Only 548 was taken, so the next measurement should read
**358 ± 2**.

### 2.2 The third segment, in the same metric

The segment-3 baseline was **re-measured** (the supervisor's instruction: after-2
predates the unit's own gate fixes *and* the `cli_tool` guard of
`docs/testing/flaky-tauri.md`, so its line numbers and its `measured` totals are
not the revision this segment ships on). Command, both flavors, one report — the
§1 recipe with `CARGO_TARGET_DIR=target/cov-tauri-tau3`, driver
`/tmp/tau3-measure.sh`, log `/tmp/tau3-measure-before.log`:

```bash
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tauri-tau3-before-lcov.info   # exit 0
```

| file / scope | before (seg-3) | after-2 (old) | after (seg-3) | after / measured |
|---|---|---|---|---|
| `src/terminal/server.rs` | 58 | 59 | **58** | 58 / 1839 |
| `src/terminal/pty.rs` | 18 | 18 | 18 | 18 / 460 |
| `src/terminal/session.rs` | 14 | 14 | 14 | 14 / 727 |
| `src/terminal/manager.rs` | 8 | 8 | 8 | 8 / 569 |
| `src/terminal/shell.rs` | 11 | 11 | 11 | 11 / 153 |
| `src/terminal/cwd.rs` | 4 | 4 | 4 | 4 / 280 |
| `src/headless/mod.rs` | 104 | 104 | **51** | 51 / 826 |
| `src/headless/agent.rs` | 4 | 4 | **1** | 1 / 25 |
| `src/cli_tool.rs` | 11 | 15 | 11 | 11 / 384 |
| `src/agent_host.rs` | 24 | 24 | **26** | 26 / 272 |
| `src/agent_supervisor.rs` | 81 | 81 | 81 | 81 / 233 |
| `src/skills_bootstrap.rs` | 28 * | 17 | 17 | 17 / 173 |
| **scope total** | **365** | 359 | **300** | 300 / 6198 |

**65 lines fewer** on the same scope, 300 = 365 − 53 (`headless/mod.rs`) − 3
(`headless/agent.rs`) − 11 (`skills_bootstrap.rs`) + 2 (`agent_host.rs`).

Two entries in that table are *measurement* effects rather than code, and both are
named so no reviewer has to guess:

* `skills_bootstrap.rs` **28\*** — the pre-segment-3 run lost that test
  (`skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`)
  to the load-induced 3 s `probe` deadline (`CLI discovery timed out`), which left
  its drain-loop lines uncovered. It passed in the segment-3 run, so the honest
  reading is **17 → 17** for that file and **54** lines closed by the segment's work.
  The same deadline flake fired once in the segment-3 *gui* leg, on
  `cli_tool::tests::a_runnable_cli_on_path_is_discovered_and_probed`; it cost
  nothing measurable (that file's uncovered set is identical in both runs) because
  the headless leg runs the same tests. Both are `FLAKES.md` item 27's surviving
  half — the deadline, not the leak — and the successor todo for it is named in the
  delivery report.
* `agent_host.rs` **24 → 26** — segment 3 closed the attach-only/mode refusals
  (160, 208) and *lost* four external-attach lines (111, 265, 319, 320) that the
  pre-segment-3 run happened to cover: those four are the branch taken when the
  host fixtures' mock Agent answers `select_agent` inside its 2 s budget, so they
  oscillate with load. §4.2 carries them with that mechanism; the residual is
  therefore **300 ± 4** in the same sense §2.0 gives `terminal/server.rs` ± 2.

**The after figure is a union of three executions** — the segment-3 gui leg, the
segment-3 headless leg (both in `/tmp/tau3-measure-after.log`, both
`--no-fail-fast`), and one re-run of the `headless_cli` integration target
(`/tmp/tau3-union-leg2.log`) taken because the *first* integration run of the gui
leg lost its own revoked arm to a fixture bug that is written up in §8.6. All
three share the profile store in `target/cov-tauri-tau3` (no `clean` between
them), and the report is `/tmp/tauri-tau3-union-lcov.info` (`cargo llvm-cov
report` exit **0**). A union of executions can only *lower* the residual — never
report a line as covered that no run executed — which is the §5.3 direction
argument.

### 2.3 `cli_tool.rs` at HEAD — re-measured, and the statement set changed

The segment-3 union (`aff0501b3`) measured a **626-line** `cli_tool.rs`. Two
commits moved it afterwards: `224dcea1e` ("NEW-B deadline seam", 626 → 758) and
`1876ffe78` (the Linux port of the probe fixtures: 11 insertions / 2 deletions,
758 → **767**, the revision this record now describes).

The re-measurement at `36c568972` is the §1/§6a-ter recipe: one render per
flavour, `cargo llvm-cov clean` between the legs, then a union.

```bash
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --output-path /tmp/d8-gui.info -- --test-threads=1
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --no-default-features --features headless \
    --output-path /tmp/d8-headless.info -- --test-threads=1
python3 .future/cov100/lcov-union.py --out /tmp/d8-union.info /tmp/d8-gui.info /tmp/d8-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d8-gui.info /tmp/d8-headless.info   # LEG IDENTITY: ok
python3 .future/cov100/flavor-alive.py /tmp/d8-gui.info /tmp/d8-headless.info        # both carry nonzero hits
```

Driver `/tmp/d8-measure.sh`, log `/tmp/d8-measure.log`, union retained as
`.future/cov100/tauri-tau-d8-union-lcov.info`. **Both legs came back green**:
gui lib **2142 passed / 0 failed / 14 ignored**, headless lib **1717 / 0 / 14**,
and every integration target passed in both (this removes the §8.5 §5.3 caveat
for this revision: there is no red run to reason around).

The file's residual is **still exactly 11 lines, but the set is not the same** —
the previous note's claim that "the 11 uncovered statements are unchanged" was
wrong in two directions. Measured `DA:0` set at HEAD: **83, 92, 285-288, 293,
305, 356, 672, 759**.

| union line (`aff0501b3`) | HEAD line (`36c568972`) | what changed |
|---|---|---|
| 82 | 83 | pure shift (`"future.exe"` under `cfg!(windows)`) |
| 91 | 92 | pure shift (`"bundled"`) |
| 223-226 | 285-288 | shift (the `try_wait` `Err` arm) |
| 231 | 293 | shift (post-exit stderr limit) |
| 243 | 305 | shift (post-exit stdout limit) |
| 294 | **356** | shift; the earlier note wrote 357, which is the following `set_var` statement |
| 338 | **none — now covered** | the `EnvGuard::drop` `None` arm (HEAD 400) is measured `DA>0`: `remote/verify_e2e.rs` arms the same guard with five absent `VERIFY_E2E_*` names |
| 598 | **672** | the fail-only message argument; the earlier note wrote 666, which is `let started` (covered). `1876ffe78` added six lines above it |
| — | **759** | **new statement**, not a shift: the `panic!` body of the `unwrap_or_else` closure the NEW-B seam introduced |

So 356 and 672 correct two numbers the previous note got wrong, 400 is no longer
a residual line at all, and 759 is a statement that did not exist at the union
revision. The mechanisms are in §4.1; the pairing's `expect` is re-recorded from
this report in §4.3.

### 2.4 TAU-D10 — the re-measurement behind the current figure (270)

Same recipe as §2.3, run at `b6cbc666e` with three tests added to this unit's
scope (`cwd.rs` read-only fixture, the admission-status assertion in
`desktop_shutdown.rs`, the live tab at the head of `manager.rs`'s exit order):

```bash
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --output-path /tmp/d9-gui.info -- --test-threads=1
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --no-default-features --features headless \
    --output-path /tmp/d9-headless.info -- --test-threads=1
python3 .future/cov100/lcov-union.py --out /tmp/d9-union.info /tmp/d9-gui.info /tmp/d9-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d9-gui.info /tmp/d9-headless.info   # LEG IDENTITY: ok
python3 .future/cov100/flavor-alive.py /tmp/d9-gui.info /tmp/d9-headless.info
```

Driver `/tmp/d9-measure.sh`, log `/tmp/d9-measure.log`, union retained as
`.future/cov100/tauri-tau-d9-union-lcov.info`. Leg file sets are unchanged (170
gui / 138 headless), so the two reports are the same flavours as §2.3 measured.

* **gui leg: green.** lib **2155 passed / 0 failed / 14 ignored**; every
  integration target passed.
* **headless leg: red, outside this scope.** lib **1719 passed / 7 failed / 14
  ignored**; all seven are `remote_peer::runtime::tests` /
  `remote_peer::transfer_tests`, every one panicking at
  `remote_peer/testing.rs:30` with `web server bind on port 8022 failed: Address
  already in use` — the documented host-wide 8022 contention family, not this
  unit's code. The leg therefore exited **101** and `cargo llvm-cov` did not
  render its report; the report was rendered from the same run's profraw with
  `cargo llvm-cov report --lcov --output-path /tmp/d9-headless.info` (exit 0),
  which reads the existing execution rather than replaying the suite. Every
  other headless target ran and passed, including `tests/headless_cli.rs`
  (12/12), `tests/remote_start_refusal.rs` (4/4) and
  `tests/re_pair_entry_points.rs` (0 tests, feature-gated).

Per-file residuals: §4.3. The direction argument of §5.3 still holds — a union
of executions cannot report a line as covered that no run reached — and the red
`remote_peer` tests touch none of the twelve scopes.

The gate reads the retained union (`.future/cov100/pairings.json`, pairing
`tauri-headless`). Verbatim from `python3 .future/cov100/staletable.py --gate`:

```
tauri-headless             authoritative    module-tauri-headless.md        270    270      0      0       0 section       PASS
```

(columns: `claims 270 / uncovered 270 / STALE 0 / ABSENT 0 / COVERED 0`, the
section needle resolving to §4.1's heading. The command as a whole exits
non-zero on two peer pairings, `channels-wide` and `cli-wide`, whose covered rows
are outside this unit and were already failing before this measurement.)

### 2.5 TAU-D12 — the measurement behind the current figure (254)

Same recipe as §2.4, run at the TAU-D12 revision with the two fixture modules of
§9 in the tree. Both legs were rendered separately, `cargo llvm-cov clean`
between them, and unioned:

```bash
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d12-gui.info
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless \
    -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d12-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d12-gui.info /tmp/d12-headless.info  # ok
python3 .future/cov100/lcov-union.py --out .future/cov100/tauri-tau-t12-union-lcov.info \
    /tmp/d12-gui.info /tmp/d12-headless.info
```

Driver `/tmp/d12-measure.sh`, log `/tmp/d12-measure.log`. The union is retained as
`.future/cov100/tauri-tau-t12-union-lcov.info` (172 files; the pairing
`tauri-headless` points at it). `--no-report` + a separate `report` per leg is
what makes a red leg still yield its profraw (§2.4 had to say that after the
fact); here both legs exited **0**, so no such note is needed.

* **LEG IDENTITY: ok** — gui 170 files / headless 138, exactly the sets §2.4
  measured, so the two reports are the same flavours.
* **gui leg: green.** lib **2167 passed / 0 failed / 14 ignored**; every
  integration target passed. (An earlier run of the same revision lost four
  `remote_peer::transfer_tests` to the host-wide `8022` bind contention; that run
  was discarded and not used for numbers.)
* **headless leg: green.** lib **1719 passed / 0 failed / 14 ignored**, plus the
  integration targets (`tests/headless_cli.rs` 12/12 among them).

| file / scope | D9 (§2.4) | T12 | attribution of the move |
|---|---|---|---|
| `src/terminal/server.rs` | 57 | **58** | +1, oscillation: **539** was `DA:1` in D9 and reads `DA:0` here (§4.1's 535 row names both) |
| `src/terminal/pty.rs` | 18 | 18 | — |
| `src/terminal/session.rs` | 14 | 14 | — |
| `src/terminal/manager.rs` | 7 | 7 | — |
| `src/terminal/shell.rs` | 11 | 11 | — |
| `src/terminal/cwd.rs` | 3 | 3 | — |
| `src/headless/mod.rs` | 44 | 44 | — |
| `src/headless/agent.rs` | 1 | 1 | — |
| `src/cli_tool.rs` | 11 | 11 | — |
| `src/agent_host.rs` | 22 | **20** | −13 production lines closed (83, 88, 116, 147-150, 200, 233, 234, 334-336) and +11: one oscillation (265) and ten lines of the fixture's own fail-only arms (§4.1) |
| `src/agent_supervisor.rs` | 65 | **50** | −23 production lines closed (all of §4.2's row) and +8 lines of the supervisor fixture's fail-only arms (§4.1) |
| `src/skills_bootstrap.rs` | 17 | 17 | — |
| **total** | **270** | **254** | **−16** |

**270 → 254, attributable line by line.** 36 production lines closed (13 + 23),
19 lines added to the residual (11 + 8) and one flake-free run movement (539, +1)
account for the 16: `270 − 36 + 19 + 1 = 254`. The 19 are *test code in the two
files the fixture lives in*, not a regression: each is a must-not-fire arm of the
harness (a timeout guard, a panic on a case the fixture cannot legitimately hit),
classified in §4.1 with the neighbours that are covered as evidence.

**What the total does and does not claim.** 254 is the measured residual at the
T12 revision, not zero. 218 lines are classified in §4.1 and **36 remain OPEN**
in §4.2, all outside the `agent_host`/`agent_supervisor` block, which is now
fully classified: 36 of its 45 OPEN lines are covered and every one of the 19
remaining is named with a mechanism.

The gate reads the T12 union (`.future/cov100/pairings.json`, pairing
`tauri-headless`). Verbatim from `python3 .future/cov100/staletable.py --gate`:

```
tauri-headless             authoritative    module-tauri-headless.md        254    254      0      0       0 section       PASS
```

(columns: `claims 254 / uncovered 254 / STALE 0 / ABSENT 0 / COVERED 0`, the
section needle resolving to §4.1's heading — the same line read `270 270` in §2.4
before this segment. The command as a whole still exits non-zero on the same two
peer pairings, `channels-wide` and `cli-wide`, both outside this unit.)

### 2.6 TAU-D13 — the measurement behind the current figure (240)

Same recipe as §2.5, run at the TAU-D13 revision with the `skills_bootstrap`
block_fixtures module of §10 in the tree. Both legs were rendered separately,
`cargo llvm-cov clean` between them, and unioned:

```bash
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d13-gui.info
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless \
    -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d13-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d13-gui.info /tmp/d13-headless.info  # ok
python3 .future/cov100/lcov-union.py --out .future/cov100/tauri-tau-t13-union-lcov.info \
    /tmp/d13-gui.info /tmp/d13-headless.info
```

Driver `/tmp/d13-measure.sh`, log `/tmp/d13-measure.log`. The union is retained as
`.future/cov100/tauri-tau-t13-union-lcov.info` (172 files; the pairing
`tauri-headless` points at it).

* **LEG IDENTITY: ok** — gui 170 files / headless 138, the same sets §2.5
  measured; `leg-flavour-check.py` printed `LEG IDENTITY: ok - each leg matches
  the flavour it claims`.
* **Both legs are red, and neither red row is in this unit's scope.** gui lib
  **2167 passed / 8 failed / 14 ignored** (609.81 s); headless lib
  **1707 passed / 27 failed / 14 ignored** (589.39 s). 26 of the 27 headless
  failures and all 8 gui failures are one family — `remote::tests::runtime_tests`
  (7), `remote_peer::runtime_tests` (10), `remote_peer::tests` (5),
  `remote_peer::transfer_tests` (4) — all panicking with
  `web server bind on port 8022 failed: Address already in use (os error 48)` at
  `remote_peer/testing.rs:30` / `remote/tests.rs:424`. The cause is outside this
  tree: `lsof -nP -iTCP:8022 -sTCP:LISTEN` during the legs answered
  `futureos_ 16860 … /Users/geilige/future-os/.worktrees/remote-multi-device-runtime/target/debug/deps/futureos_lib-… remote_peer::transfer_tests --test-threads=1`
  — a **peer worktree's** test binary holding the port (the campaign's
  `GOTCHAS.md` records the family). The 27th is
  `tasks::tests::the_headless_loop_runs_a_queued_request_it_is_woken_for`
  (`tasks.rs:1939`, "the woken headless loop never executed the queued request"),
  a load-sensitive timing test in another unit's file. None of these files is one
  of the twelve scopes, so no scope line changed hands because of them; every
  test in the scopes passed in both legs, including the four new cases
  (`skills_bootstrap::block_fixtures::{a_cli_whose_runtime_vanished_after_discovery_is_reported_and_skipped, a_stop_kills_a_live_bootstrap_child_and_ends_the_drain, a_surviving_grandchild_ends_the_drain_at_its_budget, host_fixture}` — the `host_fixture`
  helper runs in the parent as a no-op and in each child as the case). Unlike
  §2.5 there is a red leg to account for, and this paragraph is that accounting:
  the run is used for numbers because the measurements are per line of *this*
  scope, whose own tests all ran; it is not described as green.
* The headless flavour does not compile `skills_bootstrap.rs` (`mod` is
  `#[cfg(feature = "gui")]`), so all of that file's numbers come from the gui leg
  and the union only adds the other eleven scopes.

| file / scope | T12 (§2.5) | T13 | attribution of the move |
|---|---|---|---|
| `src/terminal/server.rs` | 58 | 58 | no net move: the oscillating pair swapped (`533` uncovered here, `539` covered; §4.1's 535 row names both members and the counts behind them) |
| `src/terminal/pty.rs` | 18 | 18 | — |
| `src/terminal/session.rs` | 14 | 14 | — |
| `src/terminal/manager.rs` | 7 | 7 | — |
| `src/terminal/shell.rs` | 11 | 11 | — |
| `src/terminal/cwd.rs` | 3 | 3 | — |
| `src/headless/mod.rs` | 44 | 44 | — |
| `src/headless/agent.rs` | 1 | 1 | — |
| `src/cli_tool.rs` | 11 | 11 | — |
| `src/agent_host.rs` | 20 | 20 | — |
| `src/agent_supervisor.rs` | 50 | **42** | −8: **200-206 and 210** read `DA>0` in this run. **Not claimed as closed** — see §4.1's rewritten row: the "Cleanup failed" waiter dialog was *shown* by a quit whose stop returned `Err`, and 207-209 (the confirmation callback) stayed uncovered because a native `rfd` dialog cannot be clicked from a test. The trigger is load-dependent, so these eight can return to the residual. |
| `src/skills_bootstrap.rs` | 17 | **11** | −10 production lines closed (28-30, 49, 53, 54, 69, 70, 72, 80) and +4 of the new fixture's own fail-only arms (349, 393, 525, 526) = **−6** |
| **total** | **254** | **240** | **−14** |

**254 → 240, attributable line by line.** The D13 fixture closes ten production
lines and adds four ($254 - 6 = 248$ for this file's row), and the remaining
$-8$ is the measured `agent_supervisor` movement above, which this segment does
**not** claim: $254 - 6 - 8 = 240$. Of the thirteen lines this todo was opened for
(28-30, 49-54, 69-70, 72, 80, all `DA:0` in the T12 report), **ten are covered**
(28-30, 49, 53, 54, 69, 70, 72, 80) and **three are the kill-error arm**
(50-52), which the two stop cases cannot execute because `kill()` returns `Ok`
for a live child and `Ok` again for one the plugin has already reaped (§4.1).

**What the total does and does not claim.** 240 is the measured residual at the
T13 revision, not zero. 217 lines are classified in §4.1 and **23 remain OPEN**
in §4.2, all in `headless/` and `terminal/session.rs`; the `skills_bootstrap`
row is gone and every one of its eleven residual lines is named.

The gate reads the T13 union (`.future/cov100/pairings.json`, pairing
`tauri-headless`: `report` re-pointed, `expect` 240/240). Verbatim from
`python3 .future/cov100/staletable.py --gate`:

```
tauri-headless             authoritative    module-tauri-headless.md        240    240      0      0       0 section       PASS
```

(columns: `claims 240 / uncovered 240 / STALE 0 / ABSENT 0 / COVERED 0`, the
section needle resolving to §4.1's heading; the same line read `254 254` after
§2.5. The command as a whole still exits non-zero, on six pairings outside this
unit — four whose `/tmp` reports are no longer retained (`tauri-shell`,
`tauri-appdata`, `tauri-remote`, `tauri-shell-seg2`) and the two justification
mismatches `channels-wide` and `cli-wide` §2.5 already recorded.)

### 2.7 TAU-D14 — the measurement behind the current figure (237)

Same recipe as §2.6, run at the TAU-D14 revision with the spawn-failure case of
§11 in the tree. Both legs were rendered separately, `cargo llvm-cov clean`
between them, and unioned:

```bash
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d14-gui.info
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless \
    -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d14-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d14-gui.info /tmp/d14-headless.info  # ok
python3 .future/cov100/lcov-union.py --out .future/cov100/tauri-tau-t14-union-lcov.info \
    /tmp/d14-gui.info /tmp/d14-headless.info
```

Driver `/tmp/d14-measure.sh`, log `/tmp/d14-measure.log`. The union is retained as
`.future/cov100/tauri-tau-t14-union-lcov.info` (172 files; the pairing
`tauri-headless` points at it).

* **LEG IDENTITY: ok** — gui 170 files / headless 138, the same sets §2.6
  measured; `leg-flavour-check.py` printed `LEG IDENTITY: ok - each leg matches
  the flavour it claims`.
* **gui leg: green.** lib **2176 passed / 0 failed / 14 ignored** (571.76 s) and
  every integration target passed. (T13's gui leg was red on the 8022 family; the
  peer worktree that held the port had finished by this run, §2.6.)
* **headless leg: red, outside this unit.** lib **1727 passed / 8 failed / 14
  ignored** (489.65 s), all eight in the same `remote*` family with
  `web server bind on port 8022 failed: Address already in use (os error 48)`:
  `remote::tests::runtime_tests::start_runs_the_full_bridge_and_stop_winds_it_down`
  (which fails its `web_url` assertion because the bind never happened) and seven
  `remote_peer::runtime_tests::*` (`testing.rs:30`, "the test web port never
  freed"). None of those files is one of the twelve scopes, and every test in the
  scopes passed in both legs, including the new case. The port's holder was not
  captured by `lsof` this time (the run took it and released it inside the leg),
  which is why §2.6's `lsof` evidence is cited rather than repeated.

| file / scope | T13 (§2.6) | T14 | attribution of the move |
|---|---|---|---|
| `src/headless/mod.rs` | 44 | **41** | −3: **313-315** (the spawn-failure arm) covered by §11's case. The fixture inserted 74 lines above the file's `mock_http_server`, so its `Err(_) => return` moved **1131 → 1205**; that renumbering is net zero and the row follows it (§4.1) |
| `src/headless/agent.rs` | 1 | 1 | — |
| `src/terminal/server.rs` | 58 | 58 | — (the 533/535 pair reads the same) |
| `src/terminal/pty.rs` | 18 | 18 | — |
| `src/terminal/session.rs` | 14 | 14 | — |
| `src/terminal/manager.rs` | 7 | 7 | — |
| `src/terminal/shell.rs` | 11 | 11 | — |
| `src/terminal/cwd.rs` | 3 | 3 | — |
| `src/cli_tool.rs` | 11 | 11 | — |
| `src/agent_host.rs` | 20 | 20 | — |
| `src/agent_supervisor.rs` | 42 | 42 | — (the 200-210 arm did **not** re-fire this run, so §4.1's "may come back" warning stays live) |
| `src/skills_bootstrap.rs` | 11 | 11 | — |
| **total** | **240** | **237** | **−3** |

**240 → 237, attributable line by line.** Three production lines closed
(`headless/mod.rs` 313-315) and nothing added: the new case's own body is fully
covered except the statement it exercises, and the test code grew the file from
836 to 883 measured lines. No line moved *into* the residual.

**What the total does and does not claim.** 237 is the measured residual at the
T14 revision, not zero. 217 lines are classified in §4.1 and **20 remain OPEN**
in §4.2 (`headless/mod.rs` 182 and 207-227, `headless/agent.rs` 15,
`terminal/session.rs` 4). §11's two other named targets — the model-catalog
refresh failure (136-138) and the mid-run login change (182) — are *not* both
closed: 136-138 moved to §4.1 with a mechanism measured in the Agent's code, and
182 stays OPEN with a next step that needs a write set this unit does not have.
That split is the honest result of the turn, not a shortcut.

The gate reads the T14 union (`.future/cov100/pairings.json`, pairing
`tauri-headless`: `report` re-pointed, `expect` 237/237). Verbatim from
`python3 .future/cov100/staletable.py --gate`:

```
tauri-headless             authoritative    module-tauri-headless.md        237    237      0      0       0 section       PASS
```

(columns: `claims 237 / uncovered 237 / STALE 0 / ABSENT 0 / COVERED 0`, the
section needle resolving to §4.1's heading; the same line read `240 240` after
§2.6. The command as a whole still exits non-zero, on six pairings outside this
unit — four whose `/tmp` reports are no longer retained (`tauri-shell`,
`tauri-appdata`, `tauri-remote`, `tauri-shell-seg2`) and the two justification
mismatches `channels-wide` and `cli-wide` §2.5 already recorded.)

### 2.8 TAU-D15 — the measurement behind the current figure (238)

Same recipe as §2.7, run at the TAU-D15 revision with §12's fixture in the tree.
Both legs were rendered separately, `cargo llvm-cov clean` between them, and
unioned:

```bash
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d15-gui.info
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless \
    -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d15-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d15-gui.info /tmp/d15-headless.info  # ok
python3 .future/cov100/lcov-union.py --out .future/cov100/tauri-tau-t15-union-lcov.info \
    /tmp/d15-gui.info /tmp/d15-headless.info
```

Driver `/tmp/d15-measure.sh`, log `/tmp/d15-measure.log`. The union is retained as
`.future/cov100/tauri-tau-t15-union-lcov.info` (172 files; the pairing
`tauri-headless` points at it). This run is the **second** attempt: the first used
a 5-line timeout message and a case-dispatch `panic!`, added 8 lines to the file it
was measuring (scope total **244**), and was discarded in favour of the compacted
shape (§12.3). The numbers below are from the second run; the first is not cited
anywhere in this record — but commit `06beded3d` (a concurrent relay) sampled its
union file mid-flight and recorded 244, which is why the header says so.

* **LEG IDENTITY: ok** — gui 170 files / headless 138, the same sets §2.7
  measured.
* **Both legs are red, entirely outside the twelve scopes.** gui lib **2167
  passed / 11 failed / 14 ignored** (629.34 s); headless lib **1733 passed / 4
  failed / 14 ignored** (463.07 s). Every failure is the port-8022 family —
  `remote::tests::runtime_tests::*` at `remote/tests.rs:424`/`:1376` and
  `remote_peer::runtime_tests::*` at `remote_peer/testing.rs:30`, each preceded by
  `web server bind failed [LC002]: … port 8022 … Address already in use` or
  panicking with "the test web port never freed". What was established about the
  holder: at 11:32, five minutes into the run, `ps` showed **another session's**
  libtest from the **main checkout**
  (`/Users/geilige/future-os/target/debug/deps/futureos_lib-45852c9dd7624648`,
  started ~11:27) competing for the same port; it had exited by 11:52, while
  failures continued to accumulate afterwards. A separate long-lived
  `futureos-headless --re-pair` (PID 66871, started 09:27, another session's) was
  checked with `lsof` and holds **only** an outbound NATS connection
  (`192.168.3.162:58442 -> 116.62.200.165:4222`), no listener, so it is not the
  occupant. I did not capture a holder at the moment of every failure — the leg's
  own tests bind and release 8022 continuously — so this record states the one
  concurrent finding and the family's known rotation (`.future/cov100/GOTCHAS.md`)
  rather than claiming a single cause. No scope line changed hands because of it:
  every test in the twelve scopes passed in both legs.

| file / scope | T14 (§2.7) | T15 | attribution of the move |
|---|---|---|---|
| `src/headless/agent.rs` | 1 | **3** | −1: **15** covered by §12's case; +3: the harness's timeout guard (97, 98, 102) |
| `src/headless/mod.rs` | 41 | 41 | — |
| `src/agent_host.rs` | 20 | **19** | −1 **not claimed**: **265** oscillated back (`DA:1` after `DA:0` in D12-T14); §4.1's 266 row carries both readings |
| `src/terminal/server.rs` | 58 | 58 | — |
| `src/terminal/pty.rs` | 18 | 18 | — |
| `src/terminal/session.rs` | 14 | 14 | — |
| `src/terminal/manager.rs` | 7 | 7 | — |
| `src/terminal/shell.rs` | 11 | 11 | — |
| `src/terminal/cwd.rs` | 3 | 3 | — |
| `src/cli_tool.rs` | 11 | 11 | — |
| `src/agent_supervisor.rs` | 42 | 42 | — |
| `src/skills_bootstrap.rs` | 11 | 11 | — |
| **total** | **237** | **238** | **+1** |

**237 → 238, attributable line by line.** One OPEN line closed (`headless/agent.rs`
15), three harness lines entered the residual (97, 98, 102), and one line left by
oscillation (`agent_host.rs` 265). The headline **rises by one** and that is the
honest reading: this segment converted a production line into coverage at the
cost of three must-not-fire arms of a harness that lives in the file it measures —
the D12 pattern, at its smallest scale so far (D12: +19, D13: +4, D15: +3 − 1).

**What the total does and does not claim.** 238 is the measured residual at the
T15 revision, not zero. 219 lines are classified in §4.1 and **19 remain OPEN**
in §4.2 (15 in `headless/mod.rs`, 4 in `terminal/session.rs`). The ticket this
segment ran also asked for `headless/mod.rs` **182**, and the write set was **not**
extended to `tests/headless_cli.rs`, so that line stays OPEN with its next step
written out — the fallback case (§12) is what this turn landed instead.

The gate reads the T15 union (`.future/cov100/pairings.json`, pairing
`tauri-headless`: `report` re-pointed, `expect` 238/238). Verbatim from
`python3 .future/cov100/staletable.py --gate`:

```
tauri-headless             authoritative    module-tauri-headless.md        238    238      0      0       0 section       PASS
```

(columns: `claims 238 / uncovered 238 / STALE 0 / ABSENT 0 / COVERED 0`, the
section needle resolving to §4.1's heading; the same line read `237 237` after
§2.7. The command as a whole still exits non-zero, on six pairings outside this
unit — four whose `/tmp` reports F.100 deleted (`tauri-shell`, `tauri-appdata`,
`tauri-remote`, `tauri-shell-seg2`) and the two justification mismatches
`channels-wide` and `cli-wide` §2.5 already recorded.)

### 2.9 TAU-D16 — no re-measurement, and why that is the honest form

**No code changed in this segment, so no leg was re-run.** The figure is T15's
238 over `.future/cov100/tauri-tau-t15-union-lcov.info`; the pairing still points
there and `expect` is still 238/238. TAU-D16 edited **only this record** (§4.1,
§4.2, §4.3, §13): four lines changed filing and one row's mechanism was corrected.

That is a deliberate choice, not a saved measurement. The alternative — re-running
both legs to reproduce a number that no input to the build changed — would spend
~50 minutes of the host and produce a *different* union file whose only difference
from T15's is which of the oscillating lines happened to be taken (§2.0's ± 2).
The gate's job here is the ledger, and the ledger question — is every one of the
238 named, and is each row's claimed mechanism true — is answered against the
identical report:

```
$ python3 .future/cov100/staletable.py --gate
```

```
tauri-headless             authoritative    module-tauri-headless.md        238    238      0      0       0 section       PASS
```

and the scope re-read after the edits:

```
$ python3 .future/cov100/reconcile.py --report .future/cov100/tauri-tau-t15-union-lcov.info \
    --doc docs/testing/module-tauri-headless.md --scope desktop/src-tauri/src/headless \
    --section '4. Residual uncovered lines'
[unit] files with uncovered lines=2 uncovered lines=44
  mentioned in module-tauri-headless.md: 44
  MISSING (uncovered, not mentioned): 0
```

**What a re-measurement would have been needed for, and was not done:** nothing in
this segment's claims depends on `DA` values it did not read from the T15 report.
The two claims that *would* need one — that 208-216 are reachable with a pty, and
that 97/98/102 stay uncovered — are stated as predictions/measurements of the T15
run respectively, not as new numbers.

### 2.10 TAU-D17 — the measurement behind the current figure (233)

Same recipe as §2.8, run at the TAU-D17 revision with §14's fixture in the tree.
Both legs were rendered separately, `cargo llvm-cov clean --workspace` between
them, and unioned:

```bash
cargo llvm-cov clean --workspace
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d17-gui.info
cargo llvm-cov clean --workspace
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless \
    -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d17-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d17-gui.info /tmp/d17-headless.info  # ok
python3 .future/cov100/lcov-union.py --out .future/cov100/tauri-tau-t17-union-lcov.info \
    /tmp/d17-gui.info /tmp/d17-headless.info
```

Driver `/tmp/d17-measure.sh`, log `/tmp/d17-measure.log`. The union is retained as
`.future/cov100/tauri-tau-t17-union-lcov.info` (172 files; the pairing
`tauri-headless` points at it).

* **LEG IDENTITY: ok** — gui 170 files / headless 138, the same sets §2.8 measured.
* **gui leg: red, outside this unit.** lib **2178 passed / 4 failed / 14 ignored**
  (591.14 s), all four in `commands::remote_peer::tests` (the upload family) with
  the port-8022 cause. **headless leg: green.** lib **1741 passed / 0 failed / 14
  ignored** (447.33 s). So this revision has one green leg, unlike T13/T15.
* The four new cases cost **11-12 s** per leg (`--nocapture` shows each case's
  child doing the real work: an embedded Agent, a `MockPlatform`, a `FakeNats`).

| file / scope | T15 (§2.8) | T17 | attribution of the move |
|---|---|---|---|
| `src/headless/mod.rs` | 41 | **35** | −13 covered (**207, 208-212, 214-216, 224-227**) and +7 harness lines (§4.1). The net −6 is the arithmetic: 13 closed, 7 added |
| `src/headless/agent.rs` | 3 | 3 | — |
| `src/agent_host.rs` | 19 | **20** | +1 **not claimed**: **265** oscillated back to `DA:0` (it read `DA:1` in T15, `DA:0` in D12-T14, `DA:1` in D9). A fast oscillator, now with four readings named in §4.1 |
| `src/terminal/server.rs` | 58 | 58 | — |
| `src/terminal/pty.rs` | 18 | 18 | — |
| `src/terminal/session.rs` | 14 | 14 | — (the four reader arms stay in §4.1, T16's filing) |
| `src/terminal/manager.rs` | 7 | 7 | — |
| `src/terminal/shell.rs` | 11 | 11 | — |
| `src/terminal/cwd.rs` | 3 | 3 | — |
| `src/cli_tool.rs` | 11 | 11 | — |
| `src/agent_supervisor.rs` | 42 | 42 | — |
| `src/skills_bootstrap.rs` | 11 | 11 | — |
| **total** | **238** | **233** | **−5** |

**238 → 233, attributable line by line.** Thirteen production lines closed
(−13), seven harness lines entered (+7), one line returned by oscillation (+1):
238 − 13 + 7 + 1 = 233.

**What the total does and does not claim.** 233 is the measured residual at the
T17 revision, not zero. **232 lines are classified in §4.1 and exactly one remains
OPEN in §4.2** — `headless/mod.rs` 182, which still needs the write set. Every other
line of this scope is either waived with a mechanism or measured covered.

The gate reads the T17 union (`.future/cov100/pairings.json`, pairing
`tauri-headless`: `report` re-pointed, `expect` 233/233). Verbatim from
`python3 .future/cov100/staletable.py --gate`:

```
tauri-headless             authoritative    module-tauri-headless.md        233    233      0      0       0 section       PASS
```

(columns: `claims 233 / uncovered 233 / STALE 0 / ABSENT 0 / COVERED 0`, the
section needle resolving to §4.1's heading; the same line read `238 238` after
§2.8. The command as a whole still exits non-zero, on the same six pairings outside
this unit — four whose `/tmp` reports F.100 deleted and the two justification
mismatches `channels-wide` and `cli-wide` §2.5 recorded.)

### 2.11 TAU-D18 — the measurement behind the current figure (232)

Same recipe as §2.10, run at the TAU-D18 revision with §15.2's drop fixture in the
tree. Both legs were rendered separately, `cargo llvm-cov clean --workspace`
between them, and unioned:

```bash
cargo llvm-cov clean --workspace
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d18-gui.info
cargo llvm-cov clean --workspace
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless \
    -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/d18-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d18-gui.info /tmp/d18-headless.info  # ok
python3 .future/cov100/lcov-union.py --out .future/cov100/tauri-tau-t18-union-lcov.info \
    /tmp/d18-gui.info /tmp/d18-headless.info
```

Driver `/tmp/d18-measure.sh`, log `/tmp/d18-measure.log`. The union is retained as
`.future/cov100/tauri-tau-t18-union-lcov.info` (172 files; the pairing
`tauri-headless` points at it).

* **LEG IDENTITY: ok** — gui 170 files / headless 138.
* **Both legs are green.** gui lib **2183 passed / 0 failed / 14 ignored**
  (574.05 s); headless lib **1741 passed / 0 failed / 14 ignored** (445.83 s). This
  is the first fully green pair since T12: the port-8022 family did not fire,
  which is the ticket's own suggestion (run when no peer worktree is testing)
  happening to hold — not evidence that the family is fixed. §2.10 names the last
  concurrent holder it caught.

| file / scope | T17 (§2.10) | T18 | attribution of the move |
|---|---|---|---|
| `src/terminal/session.rs` | 14 | **13** | −1: **220** covered by §15.2's drop fixture. The fixture added 45 lines above three later waivers, which renumbered 1223/1264/1284/1289 → 1267/1308/1328/1333 (§4.1) |
| `src/terminal/server.rs` | 58 | **59** | +1, oscillation: **539** returned; T18 is the first run where **533, 535 and 539 all read `DA:0`** (§4.1) |
| `src/agent_host.rs` | 20 | **19** | −1, oscillation: **265** covered again (its fifth reading; §4.1) |
| `src/headless/mod.rs` | 35 | 35 | — |
| `src/headless/agent.rs` | 3 | 3 | — |
| `src/terminal/pty.rs` | 18 | 18 | — |
| `src/terminal/manager.rs` | 7 | 7 | — |
| `src/terminal/shell.rs` | 11 | 11 | — |
| `src/terminal/cwd.rs` | 3 | 3 | — |
| `src/cli_tool.rs` | 11 | 11 | — |
| `src/agent_supervisor.rs` | 42 | 42 | — |
| `src/skills_bootstrap.rs` | 11 | 11 | — |
| **total** | **233** | **232** | **−1** |

**233 → 232, attributable line by line.** 220 closed (−1) and 539 returned (+1),
which alone would leave 233; the −1 is 265 covering again. Both movers in the other
direction are oscillators this record has named for four measurements, and the
ledger claims whichever member the report shows.

**What the total does and does not claim.** 232 is the measured residual at the T18
revision, not zero. **231 lines are classified in §4.1 and exactly one is OPEN**
(`headless/mod.rs` 182). The headline alone is not the 口径 A claim; the claim is
that every one of the 232 is named, and §15.2 is what the claim is worth: a waiver
in this table was **wrong** and was retracted by measurement.

The gate reads the T18 union (`.future/cov100/pairings.json`, pairing
`tauri-headless`: `report` re-pointed, `expect` 232/232). Verbatim from
`python3 .future/cov100/staletable.py --gate`:

```
tauri-headless             authoritative    module-tauri-headless.md        232    232      0      0       0 section       PASS
```

(columns: `claims 232 / uncovered 232 / STALE 0 / ABSENT 0 / COVERED 0`, and the
last column `section` is what it must be: the ledger slice in §4.1 names the T18
report by file, which is the CLOSEOUT §5-pre requirement. During this turn it
briefly read `doc` — the slice still named T17 while the pairing checked T18 — and
the gate reports that as a weaker provenance, not as a pass with the same meaning.
The same line read `233 233` after §2.10. The command as a whole still exits
non-zero on six pairings outside this unit.)

### 2.12 TAU-D19 — no re-measurement, and why

**No production or test code changed in this segment** (the two experiments of §16.1
and §16.2 ran outside the crate's suite: a standalone probe and ten filtered
`llvm-cov` runs of one existing test). Per §2.9's rule, the legs were therefore not
re-run, and the figure is T18's **232** over
`.future/cov100/tauri-tau-t18-union-lcov.info`. The pairing still points there with
`expect` 232/232, and the ledger question is answered against the identical report:

```
$ python3 .future/cov100/staletable.py --gate
```

```
tauri-headless             authoritative    module-tauri-headless.md        232    232      0      0       0 section       PASS
```

(columns: `claims 232 / uncovered 232 / STALE 0 / ABSENT 0 / COVERED 0`; last column
`section`, i.e. §4.1's slice names the report it is checked against. Unchanged from
§2.11 — this segment moved no line and claimed none.)

The four filtered runs that *did* happen are evidence about specific lines, not
measurements of the figure, and they are reported as such: §16.1's pty probe and §16.2's
ten pump runs.

### 2.13 TAU-D27 — the measurement behind the previous figure (236)

**What this segment needed from a measurement, and what a probe could not give it.** D20's
fixture had already been *observed* to move `headless/mod.rs` 182 (`DA:0` → `DA:1` in a 3.35 s
instrumented probe, §11.4). PLAN 13 says a probe is not a measurement, and the earlier banner
said so. This segment therefore does not cite a probe: it cites a **two-leg union that contains
the line**, measured after the fixture landed, at a revision whose in-scope code is this
branch's HEAD code.

**The report.** `.future/cov100/pinned/union-d26.info` — 733 709 B, 173 `SF:` records. The recipe
and its retained artifacts are `.future/cov100/measure-d26.sh` (gui leg, `clean`, headless leg,
union, flavour check) and `measure-d26-headless.sh` (the headless leg re-run after a host flake,
gui report kept); the `tauri-remote` record documents them in its §14.3. **Both were re-checked in
this segment rather than taken on trust:** `lcov-union.py` over `d26/gui.info` + `d26/headless.info`
writes a file `cmp`-identical to the pin (md5 `42329a2b17c6bd438247df0f3d908861` for both), and
`leg-flavour-check.py` exits 0 with **LEG IDENTITY: ok** over 171/139. Verified here, not quoted:

* **Revision `0b7b2ca17`**, rev-parse'd from the record that published the union. The seven
  commits between it and this HEAD touch `pairings.json` and two other records only, so
  `git diff 0b7b2ca17..HEAD -- desktop/src-tauri/src/{headless,terminal,cli_tool.rs,agent_host.rs,agent_supervisor.rs,skills_bootstrap.rs}`
  is empty — the figure belongs to this tree. **That was true when this section was written and
  is no longer true after the merge:** `agent_supervisor.rs` gained `main`'s force-exit arms, and
  §18 re-derives the figure (241) and the rows against the merged tree. This section is kept as
  the history of how 236 was established.
* **LEG IDENTITY: ok** — gui 171 files / headless 139, each leg matching the flavour it claims
  (`leg-flavour-check.py`, exit 0). This matters here: the headless leg is the one that runs
  `tests/headless_cli.rs`, which is where D20's fixture lives.
* **`headless/mod.rs` 182 = `DA:1`** in this union. That is the line, measured, not probed.

**238 → 236, per file, from the report.**

| file | 238 (`32ba1e13e`) | 236 (`0b7b2ca17`) | attribution of the move |
|---|---|---|---|
| `terminal/server.rs` | 58 | **58** | no net change: **533** covered (−1) and **539** returned (+1). The pair §4.1 has named since T13 traded members again — the first report where 533 is covered and 539 is not |
| `terminal/pty.rs` | 18 | 18 | — |
| `terminal/session.rs` | 13 | 13 | — |
| `terminal/manager.rs` | 7 | 7 | — |
| `terminal/shell.rs` | 11 | 11 | — |
| `terminal/cwd.rs` | 3 | 3 | — |
| `headless/mod.rs` | 41 | **40** | −1: **182** covered by D20's fixture (§17.2) |
| `headless/agent.rs` | 3 | 3 | — |
| `cli_tool.rs` | 11 | 11 | — |
| `agent_host.rs` | 20 | **19** | −1: **265** covered again — its sixth reading, an oscillator, **not** claimed (§4.1's 266 row) |
| `agent_supervisor.rs` | 42 | 42 | — |
| `skills_bootstrap.rs` | 11 | 11 | — |
| **total** | **238** | **236** | **−2 net = −1 (182) −1 (265) −1 (533) +1 (539)** |

**The figure moved by two, not by one, and the second mover is not this segment's work.** That is
exactly why the probe's predicted 237 is not published: a prediction about one line cannot see an
oscillator. The three movers are the same three this record has flagged as ± 2 since §2.0
(`server.rs` 533/539, `agent_host.rs` 265), and the ledger claims whichever member the cited
report shows — which is also why §4.1's `server.rs` row now claims **535 + 539** rather than
533/535/539: a row that names all three forces a non-zero COVERED count in every report forever,
which is the mechanical disagreement F.103 is about.

**What the four `MISSING` lines turned out to be: a numbering shift, not new code.** The previous
banner said 4 of the 238 were unregistered and that they were "code `#1174`/`#1177` added or
moved". Reading the code at both revisions settles it (§17.1): the *statements* the ledger had at
952/956/981/1205 are at **953/957/982/1206** here, and the four lines that *now* read `DA>0` at
952/956/981/1205 are the `Some`/`Ok`/closing-brace predecessor of each. `git diff -U0` between
the T18 revision and `1a04560f3` shows exactly one hunk above them: `@@ -861,0 +862 @@` — one line
inserted (`devices: Vec::new(),` in a `RemoteStatus` literal, from #1174's per-device status). So
the mechanism written at T18 survives verbatim and only its line number moved. That is the whole
of the registration work; §17 records it against the four lines and the five covered rows.

## 3. What changed
### 3.1 Second segment (2026-10-16, after the first measurement)

Seven more tests, all for the cheapest OPEN rows of §4.2, plus one fixture:

| file | test | what it asserts |
|---|---|---|
| `terminal/pty.rs` | `wait_for_exit_answers_a_cursor_query_and_then_reaps_the_child` | a child that asks for the cursor and then finishes on its own is reaped instead of wedging the wait to its deadline (the reply carries no terminator, so the *echo* is the only trace — and the helper has drained that channel by then; delivery of the reply to the child is asserted one layer up, in `session.rs`) |
| `terminal/pty.rs` | (`the_ps_based_session_query_…`) fixture fix | the fixture `ps` now also prints a single-column line, so the “not a pair” arm is exercised |
| `terminal/session.rs` | `draining_a_quiet_viewer_waits_out_idle_moments` | a live, silent session is not mistaken for a finished one: the drain waits out idle moments to its deadline |
| `terminal/server.rs` | `client_frames_carry_every_length_form` | the frame builder’s three length forms (7-bit / 16-bit / 64-bit) and the masked payloads they carry |
| `terminal/server.rs` | `the_fake_socket_shuts_its_writer_half_down_cleanly` | the test transport’s `poll_shutdown` reports ready and writes nothing |
| `terminal/server.rs` | `the_pump_carries_binary_input_and_ends_on_close` (unix) | both client-input arms (Binary + Text) reach the child’s stdin, an unsolicited `Pong` is ignored rather than typed, and a `Close` frame ends the tunnel while the shell stays running — the Windows-only test could not run here |
| `headless/mod.rs` | `an_accepted_platform_key_needs_no_terminal` | a key the platform accepts short-circuits the login flow without ever asking for a terminal |

Three test-code edits the additions required (none weakens anything):
`server.rs`'s test module imported `std::time::Instant` only under
`#[cfg(windows)]`, which the unix pump test also needs (now unconditional); the
`OP_CLOSE` and `OP_PONG` opcode constants were `#[cfg(windows)]` (now used by both
flavors — `OP_PONG` by the ignored-frame assertion, taken *after* the after-2
measurement, which is the 548 line of §2.1).

### 3.2 First segment

Test-only changes; **no production line was deleted, weakened or `#[cfg]`-ed
away**, and no assertion was relaxed (the one existing-test edit *adds* an input
the test already documents — see `session.rs` below).

| file | change |
|---|---|
| `src/terminal/test_support.rs` | two unix fixtures: `cursor_query_command` (asks for the cursor and *reads* the six-byte reply, printing its byte count) and `query_then_marker_command` |
| `src/terminal/pty.rs` | 6 tests: DSR handshake, pid sentinels + direct signalling, teardown without a process group, SIGKILL escalation, the `ps`-based session query, and the session sweep it feeds |
| `src/terminal/session.rs` | 3 tests (lagged drain, disconnected drain, DSR-answering wait helper) + one added input to `an_exiting_child_still_notifies_a_late_viewer` (queued output ahead of the held exit — the case its own comment describes) |
| `src/terminal/manager.rs` | 4 tests: the whole `ManagerError` contract (code/status/message), the `CwdError` conversion, `Manager::default`, and a DSR-asking tab through `wait_for_child_exit` |
| `src/cli_tool.rs` | 8 tests over a *runnable* CLI fixture: discovery + probe, the capability refusal, the changed-identity refusal, `prepare_path`, the empty display version, and the `probe` limits (timeout / output limit / non-zero exit) |
| `src/agent_supervisor.rs` | 4 tests: the GUI start path under a mock app, `authorize_exit` ⇒ no dialog, recovery-poll == plain poll, and the `AgentStatus` wire shape |
| `src/skills_bootstrap.rs` | 3 tests: the channel-closed drain, the real plugin spawn (marker file = the CLI really ran), and an unusable CLI |
| `src/agent_host.rs` | inline `mod coverage_tests`: a fresh host is idle/unowned and reports no activity; `request_stop` on it is a no-op |
| `src/headless/mod.rs` | 10 tests: the `future init` bootstrap (success, failure, missing CLI), the platform-login branches (rejected key, network failure), the malformed-reply wait, the unencodable-invitation fallback, `pairing_link` refusal, and `print_invitation` |

Test names, by file:

* `terminal/pty.rs`: `a_cursor_query_is_answered_on_the_childs_stdin`,
  `pid_running_separates_live_children_from_the_sentinel_pids`,
  `teardown_without_a_process_group_signals_the_pid`,
  `teardown_escalates_to_sigkill_for_a_child_that_ignores_sigterm`,
  `the_ps_based_session_query_parses_and_survives_a_broken_ps`,
  `the_session_sweep_signals_reported_members_and_escalates`.
* `terminal/session.rs`: `draining_a_lagged_viewer_stops_immediately`,
  `draining_a_detached_viewer_ends_immediately`,
  `a_cursor_querying_child_is_answered_by_the_wait_helper`.
* `terminal/manager.rs`:
  `every_manager_error_reports_its_code_status_and_message`,
  `a_read_only_conversation_keeps_its_own_error_code`,
  `the_default_manager_starts_empty_and_usable`,
  `a_cursor_querying_child_is_reaped_and_recorded_as_exited`.
* `cli_tool.rs`: `a_runnable_cli_on_path_is_discovered_and_probed`,
  `a_missing_capability_is_reported_with_the_cli_identity`,
  `a_cli_that_changes_during_discovery_is_rejected`,
  `prepare_path_prepends_the_discovered_cli`,
  `a_blank_display_version_is_rejected`,
  `a_silent_cli_is_killed_and_reported_as_a_timeout`,
  `a_flooding_cli_is_killed_at_the_output_limit`,
  `a_failing_cli_is_reported_with_its_exit_status`.
* `agent_supervisor.rs`:
  `supervision_through_the_gui_entry_point_is_also_safe_for_a_mock_app`,
  `an_authorized_exit_proceeds_without_a_dialog`,
  `the_recovery_poll_reports_the_same_phase_as_the_plain_poll`,
  `the_status_serialization_matches_the_webview_contract`.
* `skills_bootstrap.rs`:
  `drain_skill_events_ends_when_the_channel_closes_without_termination`,
  `the_bootstrap_spawns_the_cli_through_the_shell_plugin`,
  `a_cli_outside_the_runnable_contract_is_reported_and_ignored`.
* `agent_host.rs`: `a_fresh_host_is_idle_unowned_and_reports_no_activity`.
* `headless/mod.rs`: `the_builtin_skill_bootstrap_runs_the_cli`,
  `a_failing_builtin_skill_bootstrap_is_ignored`,
  `a_missing_cli_skips_the_builtin_skill_bootstrap`,
  `a_rejected_platform_key_is_reported_as_expired_authorization`,
  `a_network_failure_while_checking_the_platform_login_is_not_a_logout`,
  `login_wait_gives_up_after_persistent_malformed_replies`,
  `login_wait_backs_off_after_a_malformed_reply`,
  `an_unencodable_invitation_falls_back_to_the_link`,
  `pairing_link_rejects_codes_that_are_not_v2_invitations`,
  `print_invitation_writes_the_link_and_the_qr_only_when_asked`.

Three of these tests exist because the *fixture* had to be discovered first and
is worth knowing (they are the reason some lines below are covered at all):

* **The shell plugin works under `tauri::test::MockRuntime`.** `mock_builder()
  .plugin(tauri_plugin_shell::init())` + `app.shell().command(path).spawn()`
  really spawns a child on this host, which is what makes
  `run_builtin_skills`'s spawn path (and the drain loop's exit-code handling)
  testable without a real sidecar.
* **A unix shell can be made to ask for the cursor** (`test_support::cursor_query_command`):
  Windows/ConPTY shells query the cursor on their own and block until answered;
  a unix shell does not, so the fixtures ask (`ESC [ 6 n`) themselves and the
  test replies. `cursor_query_command` also *reads* the reply and prints its byte
  count, which is what turns "the helper wrote six bytes" into "the child
  received them".
* **The `ps` session column is unusable on macOS** (finding F1 in §5): a fixture
  `ps` supplies the pid/session pairs instead, which is how the parser and the
  sweep that consumes it are covered.

## 4. Residual uncovered lines (per line, with the mechanism)

Category vocabulary and the policy: `docs/testing/waiver-ledger.md`
(`unreachable-by-construction` needs the invariant, `unreachable-in-this-environment`
needs the host/fixture that cannot be provided here, `attribution-artifact` needs
the neighbouring covered lines as evidence, `platform-unmeasured` needs the OS
that measures it). The already-checked waivers of `docs/testing/module-tauri.md`
§5.1 and `.future/memory/coverage-llvm-cov.md` ("tauri / src-tauri 专坑") are
reused where they apply.

### 4.1 Waived (checked against the code and against the measurement)

Line numbers below are those of the **`union-refresh4`** report
(`.future/cov100/pinned/union-refresh4.info`, measured at `16ea6f388`; §18), the report the
pairing points at. The rows for `agent_supervisor.rs` were **re-derived** at the merge, not
renumbered: §18.2 tabulates every old → new position. The +1 shift §17.1 recorded for the
`with_path_and_broken_agent` helper (#1174's `devices: Vec::new(),` insertion at 862) still
stands and is unchanged by this merge, because `headless/mod.rs` is identical in both pins.
§4.4 is
the explicit crosswalk between the retained after-2 artifact and the D8 union; it exists
because an audit pointed at the retained after-2 artifact reports 27 lines as "unmentioned"
and 13 as "stale" — both are revision effects, not gaps in this ledger.

**Retracted by TAU-D18 (2026-10-18): `session.rs` 220.** A row in this table is
only as good as its mechanism, and 220's was **wrong**. It was filed as
`unreachable-in-this-environment` ("the session owns the pty, so dropping it closes
the master and the next read is `Ok(0)`") — but the reader holds its **own clone**
of the master (`PtySession::reader` → `try_clone_reader`), so dropping the last
`Arc<Session>` kills the child and leaves the reader's cloned fd readable. Buffered
bytes then arrive, `weak.upgrade()` fails, and **220 executes**. Measured: `DA:1`
in the T18 union. The fixture is
`terminal/session::tests::a_dropped_session_ends_its_reader_instead_of_resurrecting`,
and the line is now covered rather than waived (§15.2).

**Closed by the TAU-D17 fixture, then reopened by F.102: 13 lines covered then, 14 residual now.**
`headless/mod.rs` **207, 208-212, 214-216, 224-227** — the pairing hand-over and
the expired-invitation refusal — were covered by §14's three cases, and §4.2's
`207-216, 224-227` row was **deleted**, not narrowed. **That closure did not survive
`32ba1e13e`.** The cases lived in `headless::block_fixtures`, which #1174 invalidated
(the platform stopped surfacing an already-expired code, so the asserted arm could not
fire) and which was dropped rather than rewritten (F.102). The 13 lines are `DA:0`
again in `union-d26`, **and 213 with them** (it was the row's `attribution-artifact`,
and the call it closes no longer runs), so the residual has 14 lines back where the prose
still said "covered". **The row was never restored**: the prose above mentions
their numbers, so `reconcile.py` reported `MISSING 0` while no row classified them —
F.103's shape exactly, *mentioned* read as *registered*. TAU-D27 adds the row (§4.1)
and restores **213** to it from `attribution-artifact`, whose evidence (that 209-212
ran) went with the fixture. The lesson is the one F.103 already wrote down, so this is
the second instance: a prose paragraph that says "covered" cannot substitute for a
row, and deleting a row because a fixture covered it means restoring the row when the
fixture leaves.

**Deleted by TAU-D27 (2026-10-18): the 7 guard lines of that same fixture, and the
reason is F.102.**
The guards belonged to `headless::block_fixtures`, the module commit `32ba1e13e`
dropped. Their rows were therefore naming lines of a module that no longer exists: at
HEAD `headless/mod.rs` is 1 333 lines and a `grep -n block_fixtures` over it returns
nothing. They were also already absent from the published 238 — `reconcile.py` reported
them as `ABSENT` (no `DA:` record at all), which is not evidence of anything, and the
gate never counted a waiver for them. A row over a deleted module is *worse* than no
row, because a reader cannot tell a waivered line from a line that was never
instrumented. Deleting them is the correction; nothing was re-waivered to replace
them, and the residual did not move (they were `ABSENT`, not `DA:0`). The 13 *production*
lines the same fixture used to cover are a different matter and are **not** deleted —
see the paragraph above and their row in §4.1.

**Re-filed by TAU-D16 (2026-10-18): 4 lines, no line count moved.**
`terminal/session.rs` **220, 222, 223, 231** moved here from §4.2, each with its
own mechanism (the `session.rs` 220 row below). The residual is still **238** and
the report is still the T15 union — **no measurement was re-run in this segment,
because no code changed** (§2.9). Only the filing moved: OPEN 19 → 15.

**Closed by the TAU-D15 case (2026-10-18): 1 line by this segment's work.**
`headless/agent.rs` **15** — `check_running`'s `Phase::Failed` arm — is covered by
§12's subprocess case. The same case **added 3** lines to this file, the timeout
guard's `child.kill()` / `wait_with_output()` / `panic!` (**97, 98, 102**), which
only a run where the child never finishes can execute. And one line left this
table **by oscillation, not by this segment**: `agent_host.rs` **265** reads
`DA:1` in T15 after being `DA:0` in D12-T14. That is the documented pair (§4.1's
266 row carries both readings); it is not claimed as closed and the row no longer
covers it as a range.

**Closed by the TAU-D14 case (2026-10-18): 3 lines.**
`headless/mod.rs` **313-315** — the spawn-failure arm of
`bootstrap_builtin_skills` — is covered by
`a_cli_whose_interpreter_vanished_after_discovery_is_reported_and_skipped`
(§11). The case is the one §4.1's own T12 row predicted and the D13 measurement
proved reachable: discovery accepts the CLI, the test removes only the program
its `#!` line names, and the spawn is the first step that can notice. Two other
things moved with the same measurement: the model-catalog refresh (136-138) is
**not** reached by a failing platform catalog and now has its own row below, and
the mock server's accept-error `return` **renumbered 1131 → 1205** because the
new test sits above it.

**Closed by the TAU-D13 fixture (2026-10-18): 10 lines.** §4.2's
`skills_bootstrap` row is gone, and the lines it named are covered by the three
stop/TOCTOU cases of §10 (`skills_bootstrap.rs` 28-30, 49, 53, 54, 69, 70, 72,
80). The same measurement *added* four lines to this table, all in the new
fixture module and all fail-only (349, 393, 525, 526). Three of the row's
thirteen lines were **not** closed: 50-52, the body of
`if let Err(error) = child.take().unwrap().kill()`. Both stop cases take the `Ok`
side — a live child is killed, and a child the plugin has already reaped answers
`Ok(())` because `std::process::Child::kill` is a no-op once the handle is
cleared — so that arm needs a third state this host does not produce (§10.4).
Also new in the **T13** report is **533** on the oscillating side: 539 was `DA:0`
in T12 and reads `DA>0` there, the same pair the other way round, and the 535 row
below names both movers with the counts that predict exactly this. (It reads the
same in T14 and T15.)

**Closed by the TAU-D12 fixtures (2026-10-18): 36 lines.** §4.2's two rows are
gone, and the lines they named are covered by the subprocess cases of §9
(`agent_host.rs` 83, 88, 116, 147-150, 200, 233, 234, 334-336 and
`agent_supervisor.rs` 58-62, 70-75, 92-93, 116, 122, 125-130, 190, 193-194). The
same measurement *added* 19 lines to this table: the two fixture modules live in
the files they drive, and their must-not-fire arms carry mechanisms below. Also
new in this report is **265** on the oscillating side (§4.2's old row called it
covered in D9; it reads `DA:0` here) and **539** on the other (§4.1's 535 row
names both movers and which way each read).

**Closed by the TAU-D10 measurement (2026-10-18):** four lines that the previous
tables called constructions are now covered by tests, and their rows are gone:
`cwd.rs` 206 (`a_read_only_thread_is_refused_by_name` sets the `readonly` column
over a raw connection and asserts the `THREAD_READONLY` refusal),
`agent_supervisor.rs` 65-66 (`closing_admission_rejects_new_commands_without_losing_active_guards`
now polls `agent_status` while admission is closed and no stop operation exists),
and `manager.rs` 361 (`retention_never_evicts_a_running_session` now puts the live
tab at the *oldest* position of the exit order, so the eviction loop really pops
it and skips it). The comment in `cwd.rs`'s deleted-thread test that said
"nothing ever writes `readonly = 1`" was replaced by the fixture above it, because
that sentence described the API surface, not reachability.

**Covered in the same measurement, but not by this unit's tests.** Five lines
left the headless ledger without a new test here: `headless/mod.rs` **147-149**,
**188**, **220-221** and **281**. The PTY fixture this record had prescribed for
147-149/281 now exists in another unit's integration target —
`desktop/src-tauri/tests/remote_start_refusal.rs` calls `openpty` and spawns the
real `futureos-headless` with both streams on a pty (and
`tests/re_pair_entry_points.rs` does the same for the `--re-pair` entry points),
which is exactly what `require_terminal` needed. The remaining three are reached
by those runs' supervision loop (the `available` phase arm and the invitation
bookkeeping). `agent_host.rs` **265** also left (§4.2), on the oscillating
side of its own family. None of these is claimed as a closure of this segment's
work: they are measured covered by tests that landed after the D8 report.

| lines | category | mechanism |
|---|---|---|
| `pty.rs` 161-165, 621-622 | `platform-unmeasured` | The `if cfg!(windows)` UTF-8 code-page block and its mirror assertion in `the_child_environment_is_scrubbed_and_tagged`: `cfg!` compiles both arms, so on macOS the five `command.env(…)` lines exist but never run. A Windows measurement executes them; the *unix* half of the same test is covered. |
| `pty.rs` 434, 456 | `unreachable-by-construction` | The two timeout returns of the test helpers `wait_for` / `wait_for_exit`. Every caller asserts a needle or an exit code that its fixture produces within 20 s; reaching either line means the assertion after it fails, i.e. only a failing run executes it. |
| `pty.rs` 745-747, 787-789, 792 | `unreachable-in-this-environment` | The deadline arm (message argument + 20 ms sleep) of the two teardown tests' reap loops: on this host the signalled child is usually already reaped by the first `try_wait`, so the loop does not spin. It *did* spin in the segment-3 union run, which closed 744, 785 and 786; the lines that stay uncovered are the same arms on the other polls. Needs a host where reaping the killed child is reliably slower than one poll (§8.3). |
| `pty.rs` 843, 886 | `unreachable-in-this-environment` | Fixture restores written as `match saved { Some(v) => set, None => remove }`. The `None` arm needs the variable to have been **absent** from the process environment when the fixture started; `PATH` is always set by the campaign harness (PLAN §2). |
| `skills_bootstrap.rs` 223, 253, 271 | `unreachable-in-this-environment` | The same restore shape (`match saved { Some => set, None => remove }`); the variables these fixtures pin are always present when the guard is created. |
| `headless/mod.rs` 207-216, 224-227 | `unreachable-in-this-environment` | The pairing hand-over and the expired-invitation refusal inside `session()`'s supervision loop. **(207-216)** needs `pairing_link(&status)` to return `Some` *and* `require_terminal()` to succeed — a live platform that minted a single-use code whose expiry the loop surfaces, plus a tty. **(224-227)** needs `invitation_expiry` to be set (the same condition) and then to pass `expiry <= unix_seconds()` while the run is still looping. Neither test route can build that: a lib test cannot drive `session()` at all (it uses the per-process `agent_host::host()`, the remote `SUPERVISOR` and shutdown admission), and `tests/headless_cli.rs` spawns the real binary **without a pty**, so 207's `require_terminal()` refuses before any hand-over. These 13 lines **were covered** at T18 by `headless::block_fixtures`, the pty fixture `32ba1e13e` dropped (F.102: #1174 stopped `status()` from surfacing an already-expired code, so the arm it asserted could no longer fire). That commit's message says the arms "return to `unreachable-in-this-environment` with the mechanism named" — but they were never actually re-registered, and `reconcile.py` cannot see that because the prose still mentions their numbers; that is F.103's exact shape (*mentioned* is not *registered*), which is why TAU-D27 counted the rows' parsed lines against the report instead of trusting `MISSING 0`. **213 is restored to this row** from `attribution-artifact`: its evidence was that the `print_invitation` call at 209-212 ran, and that is precisely what disappeared with the fixture. Fixture that would reach all 14: a pty plus a code that expires while the run is up (the route the F.102 message named). Evidence for the neighbours: 217-223 are covered in `union-d26`, i.e. the loop itself runs and only these two arms do not. |
| `headless/mod.rs` 953, 957 | `unreachable-in-this-environment` | The same restore shape in `with_path_and_broken_agent` — the `None => std::env::remove_var(…)` arms: `PATH` is set by the harness, and `FUTURE_AGENT_GRPC_ADDR` is installed process-wide by the agent-bridge mock (`agent_bridge/test_support.rs:323`). Both `match` heads and both `Some` arms (951/952 and 955/956) carry a counter in `union-d26`; only the `None` side is `DA:0`. **Renumbered +1 by TAU-D27** (952/956 → 953/957): #1174's one-line insertion at 862 shifted this module, and the statements are unchanged. The `DA:0` this row used to report at 952/956 is now reported at 953/957 — the four `DA>0` readings at 952/956 are the `Some` arms, which were never the claimed lines (§17.1). |
| `cwd.rs` 451, 454 | `unreachable-in-this-environment` | The same restore shape; the pinned variable is present for the fixture's lifetime. |
| `session.rs` 526 | `unreachable-by-construction` | `on_eof` collects subscriber keys and then re-looks-up each token **under the same `MutexGuard`** (`inner` is locked at 522 and held through 533). A key taken from `keys()` therefore cannot be missing; the `else { continue }` arm is the compiler's requirement, not a reachable state. Evidence: 522-525 and 527-533 are covered in the same run. |
| `session.rs` 222, 223, 231 | `unreachable-in-this-environment` | The reader thread's `EINTR` arm, its non-`Interrupted` read error, and its thread-spawn failure. **220 was in this row until TAU-D18 and is now covered** (§15.2: the reader holds its own clone of the master, so a drop with output in flight really does land there) — the correction is exactly why the remaining three are stated one by one. **(222)** needs a signal inside one blocking `read`, with no injection point and no `SA_RESTART` control a fixture owns. **(223)** needs the master read to *fail*; on this platform it ends in `Ok(0)` — 217 has **155** hits in the T18 union, 223 has 0, while 220 (the upgrade after a read that carried bytes) has **1**, which is the drop fixture. **(231)** needs `thread::Builder::spawn` to fail (EAGAIN); the spawn result is checked every time and has never failed — same host state as `agent_host.rs` 124-126. Fixture that would reach 222/223: a reader seam in the session; this record does not propose adding a production seam for a test (§6). |
| `session.rs` 611, 796, 853, 1022, 1046, 1267, 1308, 1328, 1333 | `unreachable-by-construction` | Failure-only code: the helper's timeout `panic!` (611), the message arguments of passing assertions (`inner.buffer.len()`, `String::from_utf8_lossy(…)` ×5, `listed.len()`, `started.elapsed()`), the untaken arm of a passing `matches!` (1221), and the `Err(_) => break` of the bounded scan that follows a missing exit event (1022 — reaching it means the assertion two lines later fails). A passing run cannot evaluate them. **Renumbered by +45 in TAU-D18** (1223/1264/1284/1289 → 1267/1308/1328/1333): the drop fixture in §15.2 sits above them. The statements are unchanged. |
| `manager.rs` 533 | `unreachable-by-construction` | The message argument of a passing assertion in the `shutdown_all` fixture; a passing run cannot evaluate it. |
| `manager.rs` 307-310 | `unreachable-in-this-environment` | `shutdown_checked`'s "Terminal children did not exit" error needs a session whose child is still un-reaped after `shutdown_all`, i.e. a process that survives `SIGKILL`. That is an OS state (a child in an uninterruptible wait, or a zombie held by another reaper), not a code invariant — the ledger's own definition puts "a broken filesystem, a full pipe, ENOSPC" in this category — and the fixture that would reach it is a host where `SIGKILL` cannot take effect. Evidence: 300-306 are covered in the same run. |
| `manager.rs` 355, 365 | `unreachable-by-construction` (355) / `attribution-artifact` (365) | (355) `pop_front` cannot return `None` while `exit_order.len() > EXITED_LIMIT ≥ 0`, so the `break` is the compiler's obligation. (365) the closing brace of the loop body, reached only when the eviction branch removes an id: 359-364 are covered in the same run (5-6 hits each, 361 once) and 363-364 are the eviction itself, which is the neighbour evidence the `attribution-artifact` label requires. |
| `pty.rs` (sweep) | — | not a waiver: the `ps`-based session sweep is empty on macOS, so it is *covered* through a fixture `ps` rather than through the platform query — see §5.1. |
| `cli_tool.rs` 83 | `platform-unmeasured` | The `name = if cfg!(windows) { "future.exe" } else { "future" }` arm. `cfg!` compiles both arms, so on macOS the Windows spelling exists but never runs; a Windows measurement executes it. |
| `cli_tool.rs` 92 | `unreachable-in-this-environment` | The `"bundled"` source label needs a usable `future` file **in the directory of the running executable**, i.e. beside the libtest binary in `target/<flavor>/deps/`. Writing one there would also make `resolve()` succeed for `host_environment::tests::a_host_without_a_resolvable_cli_still_starts_logging`, which asserts `resolve().is_err()` with an empty `PATH` and does **not** take `TEST_HOME_LOCK`; a scoped guard therefore cannot protect it under CI's parallel run. Evidence: 93-99 (the `"path"` label) are covered in the same run. |
| `cli_tool.rs` 285-288 | `unreachable-in-this-environment` | The `try_wait`-failed arm of `probe`. Needs `Child::try_wait` to return `Err` for a live child this process spawned; no fixture can make `waitpid` fail there (the process has no `SIGCHLD` reaper). Evidence: 283 (`Ok(Some)`) and 284 (`Ok(None)`) are covered. |
| `cli_tool.rs` 293 | `unreachable-in-this-environment` | The post-exit stderr re-check of the output limit. The poll loop (267-281) already kills a child as soon as *either* stream passes 64 KiB, so reaching this line requires stderr to grow past the limit **between** the loop's last `metadata()` call and the `try_wait` that returns `Some` — a microsecond window with no deterministic fixture. Evidence: 267-280 (the kill + "exceeded its output limit") are covered by `a_flooding_cli_is_killed_at_the_output_limit`. |
| `cli_tool.rs` 305 | `unreachable-in-this-environment` | The post-exit stdout re-check, same window as 293: `stdout.take(LIMIT + 1)` can only read past the limit if the file crossed it after the loop's check. Evidence: 298-304 are covered. |
| `cli_tool.rs` 356 | `attribution-artifact` | The closing brace of `if let Some(path) = std::env::var_os("PATH")` in `prepare_path`; its `None` region. The `Some` path is what the harness produces. Evidence: 354-355 (the `directories.extend`) and 357, 359-361 (`set_var` / `join_paths` / `Ok`) are covered in the same run (358 is a string line with no `DA` record). |
| `cli_tool.rs` 672 | `unreachable-by-construction` | The message argument (`started.elapsed()`) of `assert!(started.elapsed() >= …)` in `a_silent_cli_is_killed_and_reported_as_a_timeout` — evaluated only when the assertion fails. (It was 598 at the union revision; `1876ffe78` moved it to 672, §2.3.) |
| `cli_tool.rs` 759 | `unreachable-by-construction` | The `panic!("without a deadline the fixture's reply must be captured: …")` body of the `unwrap_or_else` closure in `a_reply_that_arrives_after_the_deadline_is_captured_only_without_one`. The closure runs only when the no-deadline probe returns `Err`, which is the failure the surrounding test exists to report. This statement is new since the union revision (the NEW-B seam, §2.3). |
| `skills_bootstrap.rs` 104 | `unreachable-by-construction` | `CommandEvent`'s `#[non_exhaustive]` wildcard `_ => {}` in `handle_skill_event`. The compiler requires the arm and the released enum has no further variants (`.future/memory/coverage-llvm-cov.md`, "`CommandEvent` 是 `#[non_exhaustive]`"). |
| `skills_bootstrap.rs` 50-52 | `unreachable-in-this-environment` | The body of `if let Err(error) = child.take().unwrap().kill()` — the cancellation-failed report. Measured, not assumed: the two stop cases of §10 take both sides of the call and both read the `Ok` side (49 and 53/54 are covered, 50-52 are not). A **live** child we spawned is killable, and a child the plugin's wait thread has already reaped answers `Ok(())`: `shared_child`'s `wait` reaps through `std::process::Child::wait`, and `Child::kill` is a no-op once the handle is cleared — measured on this toolchain with a three-line program (`spawn → wait → kill` prints `Ok(())`). The error needs `libc::kill` to fail for a *live* child the process owns (`EPERM`/`ESRCH` from a signal-ignoring or re-parented child), which this harness cannot construct without a `kill` seam. |
| `skills_bootstrap.rs` 349, 393, 525, 526 | `unreachable-by-construction` | The new fixture's must-not-fire arms (§10): (349) the timeout guard of `run_case` — kill and reap the child that outlived its 120 s deadline; (393) `wait_until`'s trailing `ready()`, the timeout answer whose callers assert on the condition it re-reads; (526) the unknown-case `panic!("unknown case {case}")`; and (525) the fall-through brace before that panic, which the branch above (`return` at 524) never reaches. Each ends the run or is only reached by a failing run — the same class as `session.rs` 611 and `agent_supervisor.rs` 422-424. The passing assertions' arguments were hoisted at 351-354 and 519-523 so they are not residual lines. |
| `agent_supervisor.rs` 33 | `unreachable-in-this-environment` | `ensure_agent_running`'s real body is inside `#[cfg(not(test))]`: it runs in the **non-test** lib copy, i.e. only in a real application run, and it is reached from the GUI entry point (`commands/login.rs`), not from the headless binary, which never calls it. Fixture that would reach it: a GUI application run; the campaign harness has none. The cfg split is a fact about which binary is executed, not an invariant in the code. |
| `agent_supervisor.rs` 41, 45 | `unreachable-in-this-environment` | In the **bin** copy `ensure_agent_running` performs the real embedded-Agent start, so a start that returns `Err` (a held instance lock, an unusable runtime dir) makes `!matches!(…)` true and returns `false` at 45. The old mechanism ("in `cfg(test)` the callee is a constant `Ok(())`") describes the test copy, not the line. Fixture: a GUI application run whose embedded-Agent start fails. Evidence: 35-40 are covered in the same run. |
| `agent_supervisor.rs` 161-164, 166, 168-186 | `unreachable-in-this-environment` | `confirm_quit` builds and shows a dialog: it selects one of two messages, then chains `.dialog().message(…).title(…).kind(…).buttons(…)`, optionally `.parent(&window)`, and calls `.show(…)`. `tauri-plugin-dialog` drives **`rfd`'s native modal dialog**, i.e. a real window on this host that no automated run can dismiss. (TAU-D31 re-derived the position: the row read `157-160, 162, 164-182` on `union-d26`; `on_close_requested`'s new early return added four lines at `134`, so every statement in this function moved **+4** — §18.2. The lines themselves are unchanged: same unreachable call site, same 24 uncovered statements.) |
| `agent_supervisor.rs` 203, 207, 208 | `unreachable-in-this-environment` | `force_exit_process`, the watchdog's terminal action: its body is `#[cfg(not(test))] std::process::exit(0)`, and **both measured legs are test binaries**. In them the statement at 207 does not exist, so 203 (the `fn` line) and 208 (its brace) carry no executed region; 199 — the watchdog closure calling this function — and 197-200 read `DA>0` in the same run, which is the neighbouring evidence. The only binary that executes 207 is a non-test GUI application whose 25 s force budget actually fires, and the campaign has no such run (the same blind spot `server.rs` 115-117 and `agent_supervisor.rs` 33 name). New in this union: `main` added the function (§18.1). |
| `agent_supervisor.rs` 214, 215 | `unreachable-in-this-environment` | `arm_force_exit`'s spawn-failure branch: `spawn_force_exit_watchdog(…).is_err()` calls straight into the fallback. The condition needs `std::thread::Builder::spawn` to fail — `pthread_create` returning `EAGAIN` — and no input this crate controls makes that happen on this host; the same obstacle, and the same refusal, as `agent_host.rs` 124-126 in this ledger. Evidence: the rest of the function is covered in this run (210-213 and 216-217 read `DA>0`: the `force-maintenance` and `quit-repeat` cases call `request_quit(…, true)`, which arms the watchdog and spawns it successfully). New in this union (§18.1). |
| `agent_supervisor.rs` 250-252 | `unreachable-in-this-environment` | The confirmation callback of the "Cleanup failed" native dialog. **Measured again in this union:** 242-249 and 253 read `DA>0` — a quit whose stop operation returned `Err` reached the dialog and called `.show(…)`, which on this host really opens a window (load-dependent; the same arm was `DA:0` in some earlier runs) — while the callback body (250-252) still never runs: `rfd` delivers `confirmed` only from a click on a native modal window, and this GUI session cannot show-and-dismiss one (F.90). The rest of the waiter is covered here too (227-231, 233, 235-237 and 239-249; 232/234/238 have no `DA:` record — `} else {`, `};` and a blank). (Positions: this row read `207-209` on `union-d26`; the merge's force-exit block is +43 over the waiter, §18.2.) |
| `agent_supervisor.rs` 254 | **OPEN** (not waived) | The closing brace of `if !FORCE_QUIT_REQUESTED { … }` in the waiter: it is reached only when the cleanup **failed on a force quit**, because that is the one path that skips the dialog and falls out of the block instead of `return`ing (253). This is a production path, not an environment limit — and unlike the row above, **it needs no dialog host at all**, so the "native window" mechanism that covers 250-252 does not apply to it. No fixture reaches it in this union: the `force-maintenance` case takes the barrier and returns before the waiter exists, and the union's failing stop (242-249) came from an ordinary quit. The missing seam is a `block_fixtures` case that sets `desktop_shutdown::fail_for_test(…)` and calls `request_quit(app, true)`, then waits for the waiter thread: `fail_for_test` is process-global (`STOP_FAILURE` in `desktop_shutdown.rs`), so the case must be a subprocess like every other `block_fixtures` arm. It is cheap and it is **not written in this segment**, which registers rather than measures — the close needs its own run. |
| `agent_supervisor.rs` 259-261 | `unreachable-in-this-environment` | The dialog shown when the waiter thread itself fails to spawn. Same obstacle as the row above — an `rfd` native dialog — and the spawn has to fail first, which no input makes happen (the same `pthread_create` obstacle as the `arm_force_exit` row). Evidence: 255-258 (the `QUIT_CONFIRMED`/`exit` tail of the covered thread body) and 262-263 (the closing brace) read `DA>0`. (Read `216-218` on `union-d26`; +43, §18.2.) |
| `agent_supervisor.rs` 483-485, 487-488, 640-641 | `unreachable-by-construction` | The supervisor fixture's must-not-fire arms (§9): (483-485, 487-488) the timeout guard — kill the child, reap it, then panic with its output; and (640-641) the unknown-case panic with the fall-through brace before it, which the `quit-repeat` branch above never reaches because it returns (639). Each ends the run, so a passing run cannot evaluate it — the same class as `session.rs` 611 and `manager.rs` 533. The harness's passing assertion was hoisted for this reason (494-496 would otherwise be a residual line too). (Read `422-424, 426-427, 571-572` on `union-d26`; +61/+69, §18.2. 486 is a covered format-string line and 489 the call's `);`.) |
| `agent_supervisor.rs` 522 | `unreachable-in-this-environment` | The `starting` case's accept-error panic. The listener is a fresh `127.0.0.1:0` socket this process just bound, set to non-blocking, and nothing closes it, so the loop has exactly two outcomes: `Ok(peer)` once the host connects, or `WouldBlock` and a 10 ms sleep. The `Err` arm needs `accept` itself to fail — `EMFILE`/`ENFILE` from the harness's own fd budget, or `ECONNABORTED` if the peer resets between the host's `connect` and this `accept` — i.e. a resource state or a race, neither of which is an input the fixture can set; and it is a run-ending panic, so a passing run cannot evaluate it. (Read `461` on `union-d26`; +61, §18.2.) |
| `agent_host.rs` 124-126 | `unreachable-in-this-environment` | The `Err` arm of the worker-thread spawn in `start`: `std::thread::Builder::spawn` must fail (EAGAIN). No input the fixture controls makes `pthread_create` fail on this host; the builder (98-104) and the closure (105-123) are covered in the same run, so this arm is the only part of the statement nobody can execute. A thread-exhaustion fixture would be a different claim (it would also starve the harness), and this record does not make it. |
| `agent_host.rs` 216, 223, 250 | `unreachable-in-this-environment` | Three `phase != Starting` re-reads inside `run`: after the probe resolves (216), on entering the service scope (223), and after `initialize` returns (250). Each is a microsecond window between an awaited step and a lock; a stop that lands *during* the probe takes the biased `probe_cancel` arm (193, covered), and the deterministic cancellations the fixture builds (§9) block strictly before these windows. Reaching one needs a stop inside a lock acquisition or inside the `cancellation.check`-free tail of `initialize` — an ordering, not a state, and this record does not claim a race. Evidence: 214-215, 219-226 and 246-249 are covered in the same run. |
| `agent_host.rs` 266 | `unreachable-in-this-environment` | The drain arm's clean continuation: the `break` after `server.as_mut().await?` returned `Ok`. **265 is not claimed here** — it is the most active oscillator in this record and its readings are: `DA:1` in D9, `DA:0` in D12-D14, `DA:1` in T15, `DA:0` in T17, `DA:1` in T18, `DA:0` in the 238 revision and **`DA:1` again in `union-d26`** (sixth reading). 263's `ensure!` is evaluated and its message at 264 runs (one hit each, i.e. the report was **not** `Stopped`), then the arm awaits the server future; **266** additionally needs a `Stopped` drain report while that future is still pending *and* a server that then completes cleanly — no run has ever taken it. |
| `agent_host.rs` 281 | `attribution-artifact` | The closing brace of the `if let Ok(identity) = ready` body. The body (273-280) and the arm's join (283) are covered in the same run; the `if let`'s false side is an `Err` readiness channel, which skips the body and lands on 282. |
| `agent_host.rs` 289 | `unreachable-by-construction` | The message argument (`report.errors.join("; ")`) of the `ensure!` at 286: arguments are evaluated only when the condition fails, so reaching this line means the assertion two lines later fails. The same class as `session.rs` 611. |
| `agent_host.rs` 373 | `attribution-artifact` | The closing brace of the parse branch in `start_timeout` (the `#[cfg(test)]`-only deadline override, §6). The fixture sets a valid millisecond value, so 370-372 execute; 375-376 execute in every other start. Only the malformed-value join region has no counter. |
| `agent_host.rs` 462-464, 466-467, 514, 572, 709-710 | `unreachable-by-construction` | The host fixture's must-not-fire arms (§9): the timeout guard (kill, collect, panic with the child's output), the unexpected-open panic in `blocked_reader`, the accept-error panic, and the unknown-case panic with its fall-through brace. A passing run cannot evaluate any of them. |
| `server.rs` 115-117 | `unreachable-in-this-environment` | `spawn_server_task`'s `#[cfg(not(test))]` half: `crate::runtime::spawn` exists only in the non-test lib copy, and the only process that executes that copy is a real Desktop application run (the headless binary does not compile `terminal/`, so `tests/headless_cli.rs` cannot reach it either). The record's own §4.2 last paragraph calls this a "blind spot … rather than as waivable work" — that is a reachable-in-this-environment statement — and the category now says the same thing. Evidence: 111-114 are covered in the same run. |
| `server.rs` 149, 157, 525, 704, 168, 1184, 1900 | `unreachable-by-construction` / `attribution-artifact` | Four invariants: (149) the connection `Semaphore` is owned by `SERVER` and never closed, so `acquire_owned` cannot fail; (157) `SERVER` is a `OnceLock` set once in `bind` and never cleared, so the `if let Some(server)` guard is always true inside the accept loop; (525) the pump holds `Attachment` (which owns `Arc<Session>`), so its subscriber — and therefore the event sender — outlives the loop and `recv()` cannot return `None`; (704) the header loop runs over `buffer[..head_end]`, a slice that ends before the terminating `CRLFCRLF`, so no line it yields is empty. 168, 1184 and 1900 are closing-brace / `})` regions of test helpers and of the `spawn_server_task` call. Evidence for each: the neighbouring lines (148-156, 500-524, 700-703) are covered in the same run. |
| `server.rs` 535, 539 | `unreachable-in-this-environment` | **The oscillating family: this row claims the two members `union-d26` shows as `DA:0`, and names the third.** 533 and 539 are the two `break`s of "the shell is gone, so this input frame failed to write" (`Binary`/`Text`); 535 is the closing-brace region of the `Binary` decode arm. Reaching any of them needs a client input frame processed **before** the `SessionEvent::Exited` that ends the tunnel in the same `select!` — an ordering, not a state. Measured (and refined in TAU-D19, §16.2): 533 covered in 2/5 whole-module runs, 539 in
1/5, 535 in 0; **sixteen** runs of the pump test alone, including ten consecutive
clean-profile runs, never reached *any* of the three — so the 2/5 and 1/5 belong to
some other test's interleaving, not to that fixture. **The T18 union was the only run where all three read `DA:0`**, which is why the row briefly claimed all three by number; that form was a mistake and TAU-D27 retracts it, because a row that claims a covered line forces a permanent non-zero `COVERED` count (F.103's shape: a green gate over a contradiction). The readings, in order: D9 none but 535; T12 535+539; T13-T17 533+535; T18 all three; the 238 revision 533+535; **`union-d26` 535+539, with 533 reading `DA:1`**. So 533 is covered in this report and is not claimed here. The three have never been covered together, and the 2/5-1/5-0/15 distribution does not predict that they will be. Fixture: none without asserting a race; the only deterministic route is a scripted `select!` ordering seam, which this record does not propose. Evidence: 531, 532, 533, 534, 537 and 538 are covered in the same run (536 has no `DA` record). |
| `server.rs` 140-142, 163-164, 496-497, 519-523 | `unreachable-in-this-environment` | (140-142) `TcpListener::from_std` must fail for a listener `bind` just created; (163-164) a transient `accept` error (EMFILE/ECONNABORTED) must occur; (496-497) the subscriber must be removed between `attach` and `activate`, a microsecond window the pump itself keeps empty; (519-523) a viewer must accumulate > `SUBSCRIBER_LIMIT` (1 MiB) of *unread* pending output before its `activate` call — the pump activates immediately after attach, so only a scheduler stall could produce it. |
| `server.rs` 1349-1353, 1962-1966, 1992-1996 | `platform-unmeasured` | The e2e's own DSR answering blocks (`answered_dsr = true; socket.send(Text("\u{1b}[1;1R"))`). A Windows ConPTY shell emits the query and blocks; the unix shell these tests spawn does not, so the arm never runs here. A Windows measurement executes it — the same reason `docs/testing/module-tauri.md` §6.1 had to teach the e2e to answer it on Windows. |
| `server.rs` 1342-1346, 1135, 1953, 1956, 1988, 2025, 2061 | `unreachable-in-this-environment` | The e2e loops' "some other frame arrived" / "nothing more" arms: the fixture shell emits no Text frame and never errors between the awaited frames, so no other variant is queued in those windows. (1135) `shared_server`'s `if let Err(error) = serve()` body — the *first* caller binds and serves, so a later caller is the only one that can see `serve` fail; the segment-3 runs did not reach it. |
| `server.rs` 1756, 1957-1959, 1989, 2026, 2062, 2617 | `unreachable-by-construction` | Fail-only assertions and loop-exit arms of the e2e: `panic!("socket error: …")`, `Err(_) => break`, and `panic!("the pump never wrote the control frame")`. Their loop guard is the awaited frame itself, so reaching one means the assertion immediately after it fails — only a failing run executes them. |
| `cwd.rs` 253 | `unreachable-by-construction` | The message argument of `assert!(!resolved.to_string_lossy().starts_with(r"\\?\"))` — evaluated only when the assertion fails, the same failure-only class as `session.rs` 611 and `manager.rs` 533. It was labelled `attribution-artifact`, which needs neighbouring covered lines as evidence for a brace/macro region; there are none to offer (251 is covered, 254 has no `DA` record), so the label moves to the class that describes the construct. |
| `shell.rs` 108, 113 | `unreachable-in-this-environment` | `account_login_shell`'s two `None`s need `getpwuid_r` to fail or to return an empty login shell. This host always resolves one for the current uid. |
| `shell.rs` 303 | `unreachable-by-construction` | Both producers of a candidate (the account shell, `SHELL`) already reject an empty value, so `path.as_os_str().is_empty()` is never true. |
| `shell.rs` 315 | `unreachable-in-this-environment` | The `Some(path)` arm for a *relative* candidate requires the candidate loop to reach a non-absolute path, which needs the account login shell (always absolute and executable here) to be rejected first. That is a host state — the same one the next row names — and the two rows must not carry different categories. Fixture: a host whose account login shell is missing or relative, with no usable `/bin/bash` or `/bin/sh` fallback. |
| `shell.rs` 325, 329-332 | `unreachable-in-this-environment` | `tried.push` and the final `SHELL_UNAVAILABLE` error need **every** candidate unusable, including `/bin/bash` and `/bin/sh`. Needs a host without either. |
| `shell.rs` 296, 352 | `attribution-artifact` | Closing-brace regions of the `SHELL`-candidate push (296: the `if !shell.is_empty()` block) and of the `/etc/shells` read (352). Evidence: 291-295 and 344-351 are covered in the same run. |
| `headless/mod.rs` 213 | `attribution-artifact` | The `)?;` line that closes the multi-line `print_invitation(...)` call at 209-212 inside the pairing hand-over. The call **ran and succeeded** in the same run — 209-212 read `DA:1` each and 214-215 (the "single-use" note and `shown_invitation = Some(link)`) read `DA:1` — so the `?`/`;` continuation is a line-table artifact of a call whose statements are covered. Evidence: those five neighbours, same run. |
| ~~`headless/mod.rs` 1447, 1450, 1471, 1472, 1512, 1513, 1514~~ | **deleted (TAU-D27)** | Not a waiver any more, and not because a mechanism was refuted: `headless::block_fixtures` was dropped at `32ba1e13e` (F.102) and `headless/mod.rs` no longer has a line 1447. The row is removed rather than renumbered — a claim about a deleted module cannot be checked by a reader, and these seven lines were already `ABSENT`, never `DA:0`, in the published figure. |

| `headless/agent.rs` 97, 98, 102 | `unreachable-by-construction` | The spawn harness's timeout guard (§12): `child.kill()`, the `wait_with_output()` that recovers the child's output, and the `panic!` that reports it. Reaching any of them means a child that outlived its 120 s deadline, i.e. only a failing run executes them. The guard was **compacted for this reason**: its first version split the panic message over `stdout`/`stderr` bindings and cost five residual lines, and the case's case-dispatch `panic!` plus a `match` arm cost three more — both were rewritten into `err()` + `assert!` (this record's §9.3 rule: a passing `assert!` never evaluates its arguments, so a diagnostic that fits one line costs nothing). 3 lines instead of 8 is the measured difference between the two shapes. |
| `headless/mod.rs` 352, 649-651, 982 | `unreachable-by-construction` | (352) the closing brace of `ensure_login`'s `match fetch_profile()` — the match is exhaustive over `AppError`, so the fall-through region is impossible; (649-651) the body of the future passed to `until_shutdown` in `interrupt_drops_pending_startup_before_it_can_complete`, which the test asserts *never completes* (`completed` is false at 654); (982) the `Err(error) => panic!("the fixture CLI must be accepted by discovery")` arm of a `match` the fixture's CLI must satisfy — reaching it is the failure. **981 → 982 in TAU-D27** (#1174's line at 862; the `Ok` arm that reads `DA>0` at 981 is its predecessor, §17.1). |
| `headless/mod.rs` 1206 | `unreachable-in-this-environment` | The `Err(_) => return` of the mock HTTP server's bounded accept loop, after its `WouldBlock` arm: any other accept error (`ECONNABORTED`, `EMFILE`) is the ledger's own "broken host" class. Fixture: an accept that fails for a reason other than would-block; this host's loopback listener does not produce one. Evidence: the `WouldBlock` arm (1203-1205, which carries the counter in `union-d26`) is covered in the same run, and 1205 — a closing brace — is not this statement. **Renumbered 1131 → 1205** by the TAU-D14 case (74 lines above it) **and 1205 → 1206** by TAU-D27 (#1174's one line at 862); the statement is unchanged. |
| `headless/mod.rs` 836 | `attribution-artifact` | The closing brace of the `async move` block whose last expression is `std::future::pending().await` (835) in `interrupt_cancels_an_already_waiting_authorization_request`. The future is dropped while pending, so it never completes and llvm-cov attributes no hit to the block's end. Evidence: 830-835 and 837 are covered in the same run. |
| `headless/mod.rs` 136-138 | `unreachable-in-this-environment` | The failure branch of the model-catalog refresh in `session()`. **The TAU-D13 row's fixture for this arm was wrong**, and the code says so: `scheduler::refresh_future_models_now` calls the *embedded Agent's* `sync_future_models` RPC (`agent_bridge/queries.rs:643-664`), not the platform. That command always answers `RpcResponse::ok` (`agent/src/rpc/commands/providers.rs:31-59`), and the cache sync it wraps returns `false` instead of propagating a fetch failure (`agent/src/models/future.rs:643-659`). A platform whose catalog endpoint fails therefore yields **`Ok`, `synced: false`** and never reaches 136; an unavailable Agent yields `Ok(zeroed)` (`queries.rs:644-655`). What is left is a failure of the RPC *transport* between a successful `connect_agent()` and the response, or a payload that does not decode — i.e. the Agent dying mid-call. Fixture: a child whose embedded Agent is killed inside that window (a race, not an input), or a fault seam in `sync_future_models`; this record proposes neither. |
| `headless/mod.rs` 354-360, 364, 368-372, 374 | `unreachable-in-this-environment` | The device-code login flow after `require_terminal`: it needs an interactive terminal and a live platform authorization endpoint (`start_with_browser` + `wait_for_login` polling). `wait_for_login` itself is covered directly (§3). |

### 4.2 OPEN — reachable, not waived, next segment's work

**One row, and it is new in this segment: `agent_supervisor.rs` 254.** Everything else in the
241 is registered in §4.1 with a mechanism. The line is the closing brace of `request_quit`'s
`if !FORCE_QUIT_REQUESTED { … }` block, reached only when the cleanup **failed on a force quit**
— the one path that skips the dialog and falls out of the block instead of `return`ing. It is a
production path and it needs **no** dialog host, so the `rfd` mechanism that covers its
neighbours (250-252) does not cover it; that is why this segment moved it out of §4.1 instead of
renumbering it as one of them. Missing seam: a `block_fixtures` case that calls
`desktop_shutdown::fail_for_test(…)`, `request_quit(app, true)`, and waits for the waiter
thread — a subprocess, because `STOP_FAILURE` is process-global. It is cheap and it is **not
written here**: this segment registers, and the close would need its own measurement (§18.5).
Also worth knowing: `main`'s five new lines (§18.1) are `cfg(not(test))` and spawn-failure arms,
so the merge added *this* OPEN row's neighbourhood without adding a second OPEN row.

The previous occupant of this section was `headless/mod.rs` **182**, the "platform login changed or was removed" exit, and
it was blocked by this unit's write set rather than by the code. D20 widened the write set to
`desktop/src-tauri/tests/headless_cli.rs`, wrote
`a_login_change_while_supervising_stops_the_run_with_that_reason` (commit `8efb4fa0c`), and the
union-d26 measurement reads the line `DA:1`. What the test actually establishes, in the order the
line is reachable (§17.2): the child is serving (`Remote: Reconnecting`, and `try_wait()` is
`None`, so the exit is not a startup failure arriving late), `auth.json` is then rotated
**atomically** (write a sibling, `rename` — a partial write would take the corrupt-credentials arm
instead), and the run must exit `1` printing `Platform login changed or was removed. Remote access
is closing`. Two counter-assertions keep it from passing for the wrong reason: the log may not
contain `Remote connection stopped` (that would be a remote phase, not the login check) and may
not contain `PHONE PAIRING` (a stopped run must not invite the account it is abandoning), and the
rotated file is read back to prove the arm refused rather than repaired it.

The five single-test items this table named before the second segment
(pty 334/448/450, session 656-662, server 534-535/547-548, server 2397-2404,
server 2500-2502, headless 343) are now covered — see §2.1.

Not in the rows below, because they are *environments* rather than sections and
are classified with their mechanism in §4.1: `terminal/server.rs` 140-142 (a
transient `accept` error: EMFILE/ECONNABORTED), 163-164 (the same error's
back-off sleep), 496-497 (the viewer removed between `attach` and `activate`),
519-523 (a viewer past `SUBSCRIBER_LIMIT` that is still inactive when the pump
activates it — needs a scheduler stall, not a fixture), **535 and 539** (the
oscillating family of §2.0; 533 is its other `break` and reads `DA:1` in
`union-d26`, so §4.1 claims only the two members this report shows and lists the
readings of all three), 1342-1346 and 1953-2062 (the e2e's "some
other frame arrived" arms), and the `#[cfg(not(test))]` halves (`server.rs`
115-117, `agent_supervisor.rs` 33).

The `agent_host` / `agent_supervisor` rows this table carried until TAU-D12 are
**gone**: §9's fixtures covered 36 of their 45 lines, and the 19 that remain
(plus the two lines that oscillated into the residual) are classified in §4.1
with their mechanisms. The `skills_bootstrap` row is gone too: §10's three cases
covered ten of its thirteen lines, and the other three (50-52) are classified in
§4.1 with the measurement that refuses them. TAU-D14 split the `headless` row:
its model-catalog arm moved to §4.1 (the refresh is an Agent RPC, so a failing
platform catalog never reaches it), and what is left here is **`session()` as a
whole** plus the three rows below. TAU-D15 removed the fourth row
(`headless/agent.rs` 15): §12's subprocess case covers it, and what it added to
this file instead is classified in §4.1.

| lines | why it is not covered yet | next step |
|---|---|---|

*(No rows. Every uncovered line of the scope is classified in §4.1 — §4.3 has the per-file
counts and §17.2 the row that closed last.)*

One class is listed last because it is a *decision*, not a task: the
`#[cfg(not(test))]` halves (`server.rs` 115-117, `agent_supervisor.rs` 33) can
ever only be executed by a real application run, and the campaign treats that as
a blind spot (`docs/testing/module-tauri.md` §5.3) rather than as waivable work;
they are nevertheless registered in §4.1 with that mechanism.

`server.rs` 1349-1353, 1962-1966 and 1992-1996 (the e2e's DSR answering blocks)
and `pty.rs` 161-165 stay `platform-unmeasured` (§4.1): a Windows measurement
executes them, this host cannot by construction.

### 4.3 Reconciliation

Per file: the measured residual (`union-refresh4`, §18), the OPEN count (the rows of §4.2),
and therefore the withdrawn-as-waived count (§4.1). The table is exact by
construction — every line is classified in §4.1 and §4.2 has no row — and a reviewer can
regenerate the line list with
`covlines.py detail .future/cov100/pinned/union-refresh4.info --under <file>`.

| file | residual | §4.2 OPEN | §4.1 waived |
|---|---|---|---|
| `terminal/server.rs` | 58 | 0 | 58 |
| `terminal/pty.rs` | 18 | 0 | 18 |
| `terminal/session.rs` | 13 | 0 | 13 |
| `terminal/manager.rs` | 7 | 0 | 7 |
| `terminal/shell.rs` | 11 | 0 | 11 |
| `terminal/cwd.rs` | 3 | 0 | 3 |
| `headless/mod.rs` | 40 | 0 | 40 |
| `headless/agent.rs` | 3 | 0 | 3 |
| `cli_tool.rs` | 11 | 0 | 11 |
| `agent_host.rs` | 19 | 0 | 19 |
| `agent_supervisor.rs` | 47 | 1 | 46 |
| `skills_bootstrap.rs` | 11 | 0 | 11 |
| **total** | **241** | **1** | **240** |

(Counted from the **`union-refresh4`** report, §18. The arithmetic is exact: the twelve
per-file counts sum to 241, `reconcile.py` reads `MISSING 0 / COVERED 0 / ABSENT 0` over the
same report, and the single §4.2 row is `agent_supervisor.rs` 254.
`terminal/protocol.rs` and `terminal/test_support.rs`
carry no uncovered line and are not rows.)

**TAU-D31: 236 → 241, and one file moved.** `agent_supervisor.rs` 42 → 47 is `main`'s force-exit
work, itemised in §18.1: `force_exit_process`'s `cfg(not(test))` body (203, 207, 208) and
`arm_force_exit`'s spawn-failure branch (214, 215). The other eleven files are unchanged to the
line, and every `agent_supervisor.rs` row was re-derived to its post-merge position (§18.2)
rather than renumbered in place. 236 + 5 = 241 = **240 waived + 1 OPEN**, and the OPEN line is
the refiling of old 211 (§18.5), not one of the merge's five.

**TAU-D27: 238 → 236, and the last OPEN line closed.** `headless/mod.rs` 182 is covered by
D20's fixture, which is the whole of the OPEN column going to zero; the other two movers are
oscillators, not work — `agent_host.rs` 265 covered again (−1) and `server.rs` 533/539
swapping (net 0). 238 − 1 − 1 − 1 + 1 = 236 = 236 waived + **0 OPEN**. §2.13 has the per-file
table and the four line numbers that turned out to be a shift rather than new code.

**TAU-D18 changed one line and corrected one mechanism.** `terminal/session.rs` 220
moved from §4.1 to *covered* (§15.2), which is the first time this record has
retracted a waiver on measurement rather than re-filing it. Two oscillators moved
the other way: `server.rs` 539 returned (+1, and T18 is the first run with 533, 535
*and* 539 all uncovered) and `agent_host.rs` 265 covered again (−1). 233 − 1 + 1 − 1
= 232, and 232 = 231 waived + **1 OPEN**.

**TAU-D17 closed thirteen lines and the OPEN column is now one line.**
`headless/mod.rs` 207, 208-212, 214-216 and 224-227 are covered by §14's three
cases; the fourteenth line of that row, **213**, is an `attribution-artifact`
(§4.1 — the call it closes ran, 209-212 and 214-215 are covered). The fixture
added seven of its own guard lines (§4.1) and one line returned by oscillation
(`agent_host.rs` 265). 238 − 13 + 7 + 1 = 233, and 233 = 232 waived + **1 OPEN**.

**TAU-D16 moved no line: still 238, still the T15 report.** Four lines changed
filing (`terminal/session.rs` 220, 222, 223, 231 joined §4.1 with per-line
mechanisms, §13.2), so OPEN 19 → 15 and waived 219 → 223. At that point §4.2 held
two rows — `headless/mod.rs` 182 (write-set) and 207-227 (harness/pty) — and §13.3
records why the second one was **not** filed as unreachable then: its old
mechanism was measured false for a re-exec'd libtest child. TAU-D17 then closed it
(§14), which is what that measurement predicted.

**237 → 238, attributable line by line** (§2.8 tabulates it per file): one OPEN
line closed (`headless/agent.rs` 15), three harness lines entered (§4.1's 97, 98,
102 — the timeout guard of §12's fixture) and one line left by oscillation
(`agent_host.rs` 265, `DA:1` after `DA:0` since D12; not claimed). `headless/agent.rs`
is now **0 OPEN**, which is the state §12's case was built for.

**The `session()` arms TAU-D14 was sent for did not all close.** 313-315
closed; 136-138 moved to §4.1 with the mechanism the *Agent's* code gives it (a
failing platform catalog yields `Ok`, not `Err`); 182 is genuinely reachable and
is blocked only by this unit's write set (`tests/headless_cli.rs`), which §4.2
states instead of waiving. The bookkeeping is then: waived 217 → 217 (313-315
leave the table, 136-138 enter it), OPEN 23 → 20 (136-138's three lines leave;
nothing else moves), total 240 → 237.

**TAU-D15's bookkeeping, in the same shape.** One OPEN line closed
(`headless/agent.rs` 15, so that file is 0 OPEN), three harness arms entered §4.1
(97, 98, 102), and one line left by oscillation (`agent_host.rs` 265). Waived
217 → 219, OPEN 20 → 19, total 237 → 238. **The total went up**, and §2.8 says why
in one sentence: a production line became covered at the price of three
must-not-fire arms in the harness that covers it.

The earlier history, retained so the numbers are auditable: **283 → 270** closed
thirteen lines against D8 in three kinds of move — 4 by TAU-D10's own tests
(`cwd.rs` 206, `agent_supervisor.rs` 65-66, `manager.rs` 361), 1 by oscillation
(`server.rs` 533, not claimed), and 8 by other units' tests that landed after D8
(`headless/mod.rs` 147-149, 188, 220-221, 281 and `agent_host.rs` 265, which has
since oscillated back out). **270 → 254** is TAU-D12's block: 36 production lines
covered by §9's fourteen cases, 19 fixture lines added, and 539 flipping into the
residual.

So across the fourteen measurements: **552 → 359 → 300 → 283 → 270 → 254 → 240 →
237 → 238 → 233 → 232 → 238 → 236 → 241**, i.e. **311 lines covered** for this scope, **241
waived with a mechanism** and **0 OPEN**. (The 238 in that sequence is the post-merge
re-measurement, which went *up* because F.102 removed a fixture; the 236 is `union-d26`, whose
last OPEN line closed; the 241 is the merged tree, where `main`'s force-exit arms added five
lines — §18.1 says which five, so the increase is attributable rather than rounded.) The
residual stays **± 2** in the sense §2.0 gives it: the `server.rs`
533/539 pair and the `agent_host.rs` drain arm move with scheduling, and the
ledger claims only the member that is uncovered in the report it cites.
`agent_supervisor`'s force-exit arms are a new member of that class and are *not* part of
the ± 2: they are five lines, they are measured in this report, and §4.1 says which obstacle
holds each one.

Remaining work. **One line: `agent_supervisor.rs` 254**, and §4.2 names the subprocess case that
would close it. The item that stood here for
four turns — `headless/mod.rs` 182 — closed when the write set was widened to
`desktop/src-tauri/tests/headless_cli.rs` and D20 wrote the case (§4.2). What is left is not a
task list but the two classes §4.1 names as *decisions*: the `#[cfg(not(test))]` halves (a
real application run; `docs/testing/module-tauri.md` §5.3) and the oscillators (± 2, claimed
per report), plus the one OPEN line above. If a future merge moves a line, the move is a
re-derivation of this table against a new measurement — not a new waiver.

**What the total does and does not claim.** 241 is the measured residual at `16ea6f388` (the
merged tree), not zero, and this record does not call the module "done": 240 of the 241
lines are classified in §4.1 with a mechanism and **one is OPEN** (§4.2). The headline alone is not
the 口径 A claim; the claim is that a reviewer can attack the 241 one row at a time — and
§15.2 is the price of that claim being worth something: one row's mechanism was wrong, and it
was the review of a weak row (not the count) that found it. §17.1 adds the second lesson: a
line number can move under a correct mechanism, and only reading the code at both revisions
tells the two apart — §18.2 is the same check applied to a merge.

### 4.4 Revision crosswalk: the retained after-2 artifact (359)

`reconcile.py` against `.future/cov100/tauri-tau-after2-lcov.info` (359 lines,
twelve scopes) reports `MISSING 27`, `STALE 13` (8 covered + 5 absent). Those
counts are **revision effects**, not 27 unclassified lines: §2.2 and §4 were
re-measured at segment 3, and §8.7 already records `MISSING 0 / STALE 0` for the
union (300 lines). The same audit against the union report is clean. This table
gives every line the after-2 audit names a disposition, so the older artifact is
not left unexplained.

**A. The 27 "MISSING" after-2 lines (uncovered then; what happened to each).**

| after-2 line | file | disposition at segment 3 |
|---|---|---|
| 131, 135, 139, 145, 146, 183, 184, 187, 195, 228, 229, 230, 258 | `headless/mod.rs` | **covered** by the segment-3 `session()`/`ensure_login` tests in `desktop/src-tauri/tests/headless_cli.rs` (`a_deferred_remote_start_keeps_serving_and_stops_on_a_signal`, `a_revoked_pairing_stops_the_run_with_the_reason_and_drops_the_credential`; §8.2, §8.3 rows 128-139, 142, 145-146, 154-170, 171-198, 229-230, 258). No waiver is owed. |
| 810, 912, 916, 941, 1086 | `headless/mod.rs` | **renumbered**, not closed: 810 → **811** and 941 → **948**, 1086 → **1098** (`unreachable-by-construction`, test-only panic/`Err` tails); 912 → **919**, 916 → **923** (`unreachable-in-this-environment`, fixture-restore `None` arms) — §8.3's closing paragraph; §4.1 carries each category. |
| 790, 841, 884 | `terminal/pty.rs` | **renumbered** to **792, 843, 886** (+1/+2 from segment-3 test fixtures), all `unreachable-in-this-environment` (reap-loop deadline arm / `None` restore); §4.1 cites the new numbers. |
| 1262, 1282, 1287 | `terminal/session.rs` | **renumbered** to **1264, 1284, 1289** (+2), `unreachable-by-construction` (message arguments of passing assertions); §4.1 cites the new numbers. |
| 251, 269 | `skills_bootstrap.rs` | **renumbered** to **253, 271** (+2), `unreachable-in-this-environment` (fixture-restore `None`); §4.1 cites the new numbers. |
| 103 | `cli_tool.rs` | **covered** by the segment-3 `desktop/src-tauri/tests/headless_cli.rs` run (the `"development"` source label, §8.3). |

None of the 27 is an unclassified line: 14 are covered at segment 3 and 13 are
the same statements as rows already in §4.1 at their new numbers.

**B. The 8 "COVERED" after-2 candidates (the other 5 STALE rows are the ABSENT group below).**

| after-2 line | file | why the after-2 report shows it covered |
|---|---|---|
| 811, 948, 1098 | `headless/mod.rs` | **line drift**: the statements are after-2's **810, 941, 1086**, which the after-2 report itself shows *uncovered*. Segment-3 test fixes inserted 1/7/12 lines above them; §8.3 records exactly this renumbering. |
| 111, 216, 319, 320 | `agent_host.rs` | **oscillation**: the external-attach `Ready`/`Stopped` returns, taken only when the mock Agent answers `select_agent` inside its 2 s budget. The after-2 run happened to take them; the union run did not. §2.2/§4.2 name the family. |
| 1135 | `terminal/server.rs` | **run difference**: the `shared_server` `if let Err(error) = serve()` body, reachable only by a *later* caller in the same process. The after-2 run reached it; the segment-3 runs did not. §4.1 records it as `unreachable-in-this-environment`. |

The first group is renumbering (the ledger is already correct at the union
numbers); the second and third are the documented ±N families. **No row is
retired**, because every one of these lines is genuinely uncovered in the union
report and carries a mechanism in §4.1/§4.2.

**C. The 5 "ABSENT" after-2 rows (no `DA:` record at that number).**

| after-2 line | file | real line |
|---|---|---|
| 598 | `cli_tool.rs` | after-2 **567**, the `started.elapsed()` message argument of the timeout assertion; segment 3's guard moved it to union **598**, `224dcea1e` to 666, and `1876ffe78` to **672** (§2.3 — 666 is `let started`, which is measured covered). |
| 747, 788, 789, 792 | `terminal/pty.rs` | after-2 **746, 787, 790** (the reap-loop `sleep`/`assert!` lines); segment-3 fixtures shifted them by +1/+2. §4.1 cites the union numbers (745-747, 787-789, 792). |

These are not evidence of a missing line: they are the union numbers read against
the older report. Reading the source at the union numbers confirms each is a real
statement, and §4.1 already carries the mechanism.

**D. The reconciliation rule.** Against the T18 union: `uncovered 232 / mentioned
232 / MISSING 0 / STALE 0 / COVERED 0 / ABSENT 0` (§2.11, and the
`staletable.py --gate` run recorded there; the T17 union it superseded read
`233`, the T15 `238`, the T14 `237`, the T13 `240` and the T12 `254`). Against after-2: `MISSING 0` after
this crosswalk (every line is named), `STALE 8` (the first two groups above) and
`ABSENT 5` (group C; the rows cite the union/D9 numbers). The 13 cannot be
driven to zero **while the same rows remain in the union ledger**, because those
numbers are uncovered in one report and covered — or absent — in the other; the
pairings registry therefore has to point at the report the ledger describes.
That registry change is made: the `tauri-headless` pairing points at
`.future/cov100/tauri-tau-t18-union-lcov.info` with `expect.uncovered = 232`
(`.future/cov100/pairings.json`; it pointed at the T17 union — 233 — the T15 —
238 — the T14 — 237 — the T13 — 240 — and the T12 — 254 — when this crosswalk was
written, and at D9's 270 before that). The D9 union
(270), the D8 union (283) and the segment-3 union
(300) are superseded as pairing targets and kept as artifacts. Numbers in this
section are after-2/segment-3 history; current line numbers are §4.1's.

## 5. Findings

### 5.1 F1 — `session_members` silently returns nothing on macOS (real defect)

`pty::session_members` is the guarantee behind `kill_tree`'s
"background jobs escaped the process group" sweep (the T02 FINDING-2 contract
quoted in the source). On Linux it reads `/proc`; on every **other** unix it runs
`ps -axo pid=,sess=` and keeps rows whose `sess` column equals the session id.
Measured on this host (macOS 15, arm64):

```
$ ps -axo pid=,sess=,pgid= | grep -E '57405|57408'
57405      0 57405 /bin/sh -c sh -c 'sleep 30' & sleep 30   # the PTY child: its own session AND group leader
57408      0 57405 sleep 30                                  # a background job, same group
$ ps -o pid,sess,pgid,command -p $$      # even a plain shell on a tty
56758      0 56758 /bin/zsh -c ...
```

`pgid` is populated, **`sess` is 0 for every process** — so
`session_members(sid)` matches nothing, `for member in &members` never runs, and
the grace loop's `all()` is vacuously true (it returns immediately). In other
words the FINDING-2 teardown holds on **Linux only**, silently: the existing
`teardown_reaches_background_jobs` test is `#[cfg(target_os = "linux")]`, which
is exactly why this was never seen.

*Consequence for this unit's numbers:* `pty.rs`'s sweep lines are unreachable on
macOS **because the platform query is empty, not because the code is dead** — so
they are covered here through a fixture `ps` that reports the pairs the function
is documented to consume
(`the_session_sweep_signals_reported_members_and_escalates`), which also pins
the escalation order (members SIGTERM → grace → fresh enumeration → SIGKILL →
shell last).

*Smallest seam (proposed, not applied — it is a production behaviour change):*
enumerate candidate pids with `ps -axo pid=` (which works on macOS) and ask
`libc::getsid(pid)` per pid — exactly the Linux branch's logic with a portable
pid list. The `sess` column can be dropped entirely. Applying it would make the
macOS sweep real; it needs its own cross-platform test (the Linux branch already
has one) and a decision from the terminal owner, so this record proposes it
rather than inventing it.

### 5.2 F2 — an `#[ignore]`-free test-suite hazard found while writing fixtures

A fixture CLI that appends to its own script while running
(`printf x >> "$0"`) makes the *shell reading that script* execute the appended
word: the child exits `127` (`x: command not found`) and the identity check it
was meant to exercise is never reached. The self-modifying fixture must append
something inert (`printf '\n' >> "$0"`). Not a product bug, but it cost a
debugging cycle and will bite the next person who writes a "the CLI changed under
us" test.

## 6. Seams worth deciding on (nothing here was invented silently)
| where | why it cannot be tested as written | smallest seam |
|---|---|---|
| `agent_supervisor::agent_status` / `active_work_count` (`Phase::Starting`/`Failed`/`Stopped` arms, `desktop_shutdown` branches) | the Agent host is a **process-wide singleton** (`agent_host::host()`); its phase only changes by starting/stopping a real embedded Agent, which the unit's own test process cannot do without breaking every other test in the same binary (the host fixture that can is `src/agent_host/tests.rs`'s subprocess re-exec, owned by another slice) | pass the phase/`Phase` in (or add a `#[cfg(test)]` phase injector beside `request_stop`) |
| `agent_supervisor::confirm_quit` / `request_quit`'s waiter | both call `app.dialog()…show()`, which reaches **`rfd`'s native modal dialog** on this host (a test would open a real window nobody can dismiss) and then `app.exit(0)` | inject the dialog/window effect (`*_with(dialog, exit)`), as the repo already does for relaunch elsewhere |
| `headless::session` (`mod.rs` lines ~128-230) | after `ensure_login` the entry point needs a live platform API, a real NATS/remote session and an interactive terminal; the integration test that drives the *binary* is `desktop/src-tauri/tests/headless_cli.rs`, which is **outside this unit's declared write set** | extend the write set to `tests/headless_cli.rs` and serve the platform endpoints from a fake loopback HTTP server (the shape `future_login`'s own tests use) |
| `skills_bootstrap::drain_skill_events` stop path | needs `desktop_shutdown::is_stopping() == true`, which is a process-global, irreversible flag (`coordinator::begin` sets `ADMISSION.closed` and nothing clears it); flipping it in the shared libtest process makes every later test see a stopping Desktop | a subprocess fixture (same re-exec pattern) or a test-only `reset` that restores admission |
| `agent_host::start`'s startup deadline (`START_TIMEOUT`, 180 s) | the arm at 147-150 needs the wait to expire while the worker is still `Starting`; with the production constant that means a fixture that holds a start for three minutes — in this suite, on both legs, in every future run | **applied in TAU-D12** (§9): `start_timeout()` reads `FUTURE_AGENT_HOST_TEST_START_MS` under `#[cfg(test)]` only and returns `START_TIMEOUT` otherwise, so the override is compiled out of the binary and the fixture still exercises the real deadline logic (cancel, phase, error). It sits beside the existing `#[cfg(test)] tests::before_first_serve_poll` hook in the same file, which is the same pattern. The alternative — a three-minute case — was rejected as a permanent cost to every run |

## 7. Quality gates (PLAN §5.1/§5.2) — run, and their exit codes

CI runs typecheck/lint as **separate required steps**, so a green suite is not a
green PR. For a Rust unit the gates are `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings`; for `desktop/src-tauri` CI runs
exactly the three commands below (`.github/workflows/ci.yml`, job
`Rust fmt + clippy (Linux)`, step `rustfmt (desktop backend)` + step
`clippy (desktop backend)`).

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `cargo fmt --check --manifest-path desktop/src-tauri/Cargo.toml` | **1** | 4 files, **none of them in this unit's write set**: `src/commands/skills.rs`, `src/commands/tasks.rs`, `src/commands/threads.rs`, `src/commands/update.rs` |
| `cargo clippy --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings` | **101** | 5 errors, **none in this unit's write set**: `src/remote/verify_e2e.rs:1349,1366,1395,1406` and `src/remote_host/lean.rs:1437` (`clippy::needless_borrow`) |
| `cargo clippy --no-default-features --features headless --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings` | **101** | the same 5 errors (the headless flavor does not compile `terminal/`, `skills_bootstrap.rs`, `agent_supervisor.rs`) |
| `rustfmt --check` over this unit's 12 files | **0** | — |
| `cargo test --lib -- --test-threads=1 terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::` | **0** | 189 passed / 0 failed / 0 ignored |
| PLAN §5.2 `reconcile.py` (per-line ledger vs the segment-3 union, §8.7) | **0** | 300 mentioned, **0 MISSING**, 0 stale (the §2 after-2 table's 359 is historical; §4.4 is its crosswalk) |

Script for the clippy pair (private `CARGO_TARGET_DIR`, deleted afterwards):
`.future/cov100/gate-tauri-clippy.sh`; logs `/tmp/tau-clippy.log`,
`/tmp/tau-clippy2.log`, `/tmp/tau-clippy3.log`.

**Both gates were red when first run, and this unit's share was real.** The first
run reported 12 clippy errors and 26 rustfmt diffs *inside this unit's files* —
all of them introduced by this unit's own test code:

* `cargo fmt --check`: three places where an edit of mine joined a statement onto
  the function's opening line
  (`headless/mod.rs` `login_denial_expiry_and_http_failure_do_not_authorize`,
  `terminal/pty.rs` `teardown_escalates_to_sigkill_…`,
  `terminal/server.rs` `client_frame`), plus four over-long assertion lines.
  Valid Rust, but exactly what CI's fmt step rejects.
* `cargo clippy -D warnings` (6 real lints, all in test code):
  `clippy::field_reassign_with_default` ×3 (`cli_tool.rs`, `headless/mod.rs`,
  `skills_bootstrap.rs` — `Default::default()` then assigning `commands`);
  `clippy::manual_is_multiple_of` (`terminal/pty.rs`);
  `clippy::needless_borrow` ×2 (`terminal/session.rs`);
  `clippy::unused_mut` (`terminal/session.rs`).

All six were fixed as **semantics-preserving test-code edits** (struct-update
syntax, `is_multiple_of`, dropping a redundant reborrow and the `mut` it made
unnecessary) — no assertion was changed, and the scoped suite is green again
(189/0) after the fixes. The remaining red in both gates is in peer units' files
and is outside this unit's declared write set, so it is **reported here rather
than touched**: a fix in `src/remote/verify_e2e.rs` or `src/commands/*.rs` would
write outside the coordination contract and could collide with the unit that owns
those files.

**Staleness note.** The `after-2` measurement (§2) was taken *before* these gate
fixes. The fixes are test-code-only and change no assertion, so the per-file
uncovered-line counts of §2 are expected to be unchanged; the *line numbers* of
test residues in `cli_tool.rs`, `headless/mod.rs`, `terminal/pty.rs` and
`terminal/session.rs` shifted by a few lines (the struct-update rewrites add
lines). The successor's measurement (`.future/cov100/measure-tau-tau2.sh`)
renumbers them, which is why §4.1 gives each row's mechanism rather than only its
line number.

### 7.1 Segment 3 — the gates re-run on the revision this segment ships

The staleness note above is now discharged: §2.2 is the re-measurement of the
current revision, and these are the gates on the same content. Every command was
run from `desktop/src-tauri` with the environment of §1.

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `cargo fmt --check --manifest-path Cargo.toml` | **1** | 4 diffs, **none in this unit's write set**: `src/agent_bridge/persist.rs:1422`, `src/agent_bridge/reconciliation.rs:830,900,942`. All 12 of this unit's files are clean under `rustfmt --edition 2021 --check` (checked file by file, not crate-wide — `PLAN` §7b). |
| `cargo clippy --all-targets --manifest-path Cargo.toml -- -D warnings` | **0** | — (the peer `remote/verify_e2e.rs` + `remote_host/lean.rs` errors recorded above are fixed: the owners were notified and the gui flavor is clean now) |
| `cargo clippy --no-default-features --features headless --all-targets --manifest-path Cargo.toml -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::` | **101** | 182 passed / **8 failed**, all eight in the same family: `cli_tool::tests::{a_cli_that_changes_during_discovery_is_rejected, a_failing_cli_is_reported_with_its_exit_status, a_flooding_cli_is_killed_at_the_output_limit, a_missing_capability_is_reported_with_the_cli_identity, a_runnable_cli_on_path_is_discovered_and_probed}`, `headless::tests::{a_failing_builtin_skill_bootstrap_is_ignored, the_builtin_skill_bootstrap_runs_the_cli}`, `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`. Every one is the 3 s `probe` deadline (`CLI discovery timed out`) under load 40 — the same flake the pre-segment-3 measurement lost `skills_bootstrap` to (§2.2), now visible as a *set*. **A green run of this same command is the segment-3 evidence for `agent_host`, `agent_supervisor`, `terminal` and `headless`** — the eight failures are all fixture-CLI discovery, so no assertion in this unit's own tests is implicated; but the command as run is red and is reported as red (log `/tmp/tau3-scoped-suite.log`). |
| `cargo test --no-default-features --features headless --test headless_cli -- --test-threads=1` | **0** | 12 passed / 0 failed — the 9 pre-existing subprocess tests plus the 3 segment-3 arms (§8.2); 3 consecutive runs, then once more inside the instrumented union run **at load 65** |
| PLAN §5.2 `reconcile.py` per scope (6 runs, §8.7) | **0** | MISSING 0, STALE 0 (0 COVERED, 0 ABSENT) for all six scopes |

Logs: `/tmp/tau3-fmt.log`, `/tmp/tau3-clippy-gui.log`,
`/tmp/tau3-clippy-headless.log`, `/tmp/tau3-scoped-suite.log`,
`/tmp/tau3-union-leg2.log`.

### 7.2 TAU-D10 — the gates on the revision this segment ships

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `rustfmt --edition 2021 --check src/terminal/cwd.rs src/terminal/manager.rs src/desktop_shutdown.rs` | **0** | — (the three files this segment edited; a first run found two over-long lines in the new `cwd` fixture, reformatted before the run below) |
| `cargo clippy --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo clippy --no-default-features --features headless --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo llvm-cov --lib -- terminal::cwd::tests::a_read_only_thread terminal::manager::tests::retention_never desktop_shutdown::tests::closing_admission -- --test-threads=1` | **0** | 3 passed / 0 failed |

Log `/tmp/d9-clippy.log` (both flavours), `CARGO_TARGET_DIR=target/cov-tauri-d9`.
The instrumented full suites that the measurement ran are the wider evidence:
gui **2155/0**, headless **1719/7** with the seven `remote_peer` port-8022
failures named in §2.4.

### 7.3 TAU-D12 — the gates on the revision this segment ships

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `rustfmt --edition 2021 --check src/agent_host.rs src/agent_supervisor.rs` | **0** | — (a first run found one over-long assertion in the supervisor fixture, reformatted before the runs below) |
| `cargo clippy --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo clippy --no-default-features --features headless --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 agent_host::block_fixtures` | **0** | 8 passed / 0 failed, three consecutive runs (the stop-deadline case costs its 10 s `STOP_TIMEOUT` on purpose) |
| `cargo test --lib -- --test-threads=1 agent_supervisor::block_fixtures` | **0** | 8 passed / 0 failed |
| `cargo llvm-cov --no-report --lib -- --test-threads=1 agent_host::block_fixtures` (and the supervisor filter) | **0** | the same cases under instrumentation, so the measured run is not a different build |

`CARGO_TARGET_DIR=target/cov-tauri-t12` (the measurement's own dir, deleted after
the union was written). The instrumented full suites are the wider evidence and
both were **green** this time: gui **2167/0/14 ignored**, headless
**1719/0/14 ignored** (§2.5).

### 7.4 TAU-D13 — the gates on the revision this segment ships

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `rustfmt --edition 2021 --check src/skills_bootstrap.rs` | **0** | — |
| `cargo clippy --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo clippy --no-default-features --features headless --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 skills_bootstrap::block_fixtures` | **0** | 4 passed / 0 failed (3.32 s): the three cases plus the `host_fixture` helper, which is a no-op in the parent |
| `cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1` (both flavours, §2.6) | **101** (gui), **101** (headless) | the 8022 port family and one `tasks` timing test, all outside the twelve scopes; §2.6 names every failing test, the port's holder (a peer worktree's test binary) and why no scope line changed hands |

`CARGO_TARGET_DIR=target/cov-tauri-t13`. The instrumented suites are red for
reasons this unit does not own; unlike §7.3 they are therefore not cited as the
wider green evidence, and the scope's own tests are: all twelve scope files'
tests, including the four new cases, passed in both legs.

### 7.5 TAU-D14 — the gates on the revision this segment ships

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `rustfmt --edition 2021 --check src/headless/mod.rs` | **0** | — (a first run wanted one line re-wrapped; `rustfmt` applied before the gates below) |
| `cargo clippy --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo clippy --no-default-features --features headless --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 headless::tests::a_cli_whose_interpreter` | **0** | 1 passed / 0 failed (0.48 s), and with `--nocapture` the arm's own line is visible: `Could not run …/bin/future init: No such file or directory (os error 2)` — the arm is taken, not merely compiled |
| `cargo test --lib -- --test-threads=1 headless::` | **0** | 33 passed / 0 failed (2.68 s), the whole module including the new case |
| `cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1` (gui, §2.7) | **0** | — (lib **2176 passed / 0 failed** / 14 ignored; T13's gui leg was 2167 passed / 8 failed on the 8022 family, so the +9 is that family plus this segment's one new test) |
| the same with `--no-default-features --features headless` (§2.7) | **101** | 8 `remote*` failures, all the port-8022 family, none in the twelve scopes; §2.7 names them |

`CARGO_TARGET_DIR=target/cov-tauri-t14` (deleted after the union was written).
The gui leg is green; the headless leg is red outside this unit's files, so it is
named rather than cited as green evidence.

### 7.6 TAU-D15 — the gates on the revision this segment ships

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `rustfmt --edition 2021 --check src/headless/agent.rs` | **0** | — (`rustfmt` applied after the first draft and again after the compaction) |
| `cargo clippy --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo clippy --no-default-features --features headless --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 headless::agent` | **0** | 3 passed / 0 failed: the new case, the `host_fixture` helper (a no-op in the parent) and the pre-existing `shutdown_without_an_owned_service_is_a_noop` |
| the child re-exec, by hand (§12.2) | **0** | `1 passed`, and the arm's own line `Agent host finished owned=false phase=Failed("Cannot start a local Agent: instance lock is occupied or unavailable: …")` |
| the negative control: `--exact …::host_fixture` renamed to a case that does not exist | **0** | **0 sentinels printed** — so the parent's `ran` check is live, not vacuous |
| `cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1` (gui, §2.8) | **101** | 11 `remote*`/`remote_peer*` failures, all the port-8022 family, none in the twelve scopes |
| the same with `--no-default-features --features headless` (§2.8) | **101** | 4 more of the same family |

`CARGO_TARGET_DIR=target/cov-tauri-t15` (deleted after the union was written).
Both instrumented legs are red outside this unit's files, so neither is cited as
green evidence; the scope's own tests are (all twelve files, including the new
case, passed in both legs).

### 7.7 TAU-D17 — the gates on the revision this segment ships

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `rustfmt --edition 2021 --check src/headless/mod.rs` | **0** | — (`rustfmt` applied after each draft) |
| `cargo clippy --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo clippy --no-default-features --features headless --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 headless::block_fixtures` | **0** | 4 passed / 0 failed (11-12 s): the two non-pty cases, the pty case and the `host_fixture` helper |
| the same under `--no-default-features --features headless` (the ticket's control) | **0** | 4 passed / 0 failed (11.60 s) — the `gui` branch of `runtime::spawn` was not the only thing that works |
| the child re-exec, by hand, each case | **0** | each prints its own sentinel plus the arm's own trace (`Remote: Ready (Agent available)` then the refusal; `Phone pairing invitation expired…`) |
| `cargo llvm-cov --no-report --lib -- headless::` (the cost probe, run twice) | **0** | used to price the harness before the full legs: 3 uncovered with two cases, 7 with the pty case, out of 191 measured |
| the full legs (§2.10) | **101** gui, **0** headless | the four `commands::remote_peer` upload tests, port-8022 family |

`CARGO_TARGET_DIR=target/cov-tauri-t17` (deleted after the union was written).

### 7.8 TAU-D18 — the gates on the revision this segment ships

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `rustfmt --edition 2021 --check src/terminal/session.rs` | **0** | — (`rustfmt` applied once) |
| `cargo clippy --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo clippy --no-default-features --features headless --all-targets -j 4 -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 terminal::session::tests::a_dropped_session` | **0** | 1 passed, **four consecutive runs** (0.29-0.85 s) — the fixture is not a race in the test sense |
| `cargo llvm-cov --no-report --lib -- terminal::session::` (the cost probe) | **0** | 26 passed; the new test's 45 lines cost **0** residual and cover 220 |
| the full gui leg (§2.11) | **0** | 2183 passed / 0 failed / 14 ignored — green |
| the full headless leg (§2.11) | **0** | 1741 passed / 0 failed / 14 ignored — green |

`CARGO_TARGET_DIR=target/cov-tauri-t18` (deleted after the union was written).
Both legs are green this time, so unlike §7.6/§7.7 there is no red row to explain.

## 8. Segment 3 (2026-10-17) — the foreground session of the real binary

This segment closes the largest single block in the unit: `headless/mod.rs`
`session` (118-232), the entry point a server actually runs. Its rows were OPEN in
§4.2 with the note "needs the write-set extension to `tests/headless_cli.rs` plus
a fake loopback platform API"; both are in place, and the residue is renumbered
from the segment-3 union report (§2.2).

### 8.1 Why this block needs a subprocess, and why that is the honest way in

`src/headless/mod.rs` has a `#[cfg(test)] mod tests` that drives the parts of the
module which take their inputs as parameters (`ensure_login`, `wait_for_login`,
`print_invitation`, `starting_with_notice`, …). `session` cannot be driven that
way: it *is* the entry point. It reads the real account file, starts the real
embedded Agent, spawns the background observers and the scheduler, and then
supervises the remote bridge for the lifetime of the process. The pieces it drives
are process-wide singletons — `agent_host::host()` (one host per process, with a
one-way `Phase` state machine), the remote supervisor's `SUPERVISOR`, and
`desktop_shutdown`'s irreversible admission flag — so running it inside the
libtest process would leak that state into every other test in the lib target,
which is the failure class `docs/testing/flaky-tauri.md` documents.

Spawning the built `futureos-headless` binary is therefore the only way in, and it
buys what a unit test cannot: the `#[cfg(not(test))]` halves of the crate are
compiled into that binary and executed by it. Measured this segment: the
attach-only refusal and the agent-mode guard in `agent_host` (160, 208), the real
embedded Agent service scope in `agent_host::run`, and — the clearest case —
`cli_tool`'s `"development"` source label, whose candidate list the old ledger had
registered as `unreachable-by-construction` *because a libtest binary never
compiles it*. Segment 3's own report shows that row's line covered (§8.3).

What it did **not** buy, and this is worth stating because it is the boundary of
the method: `agent_supervisor.rs` stays at 81 uncovered. Its `#[cfg(not(test))]`
body (`ensure_agent_running`) is reached only from `commands/login.rs`, i.e. from a
Tauri **command** in the graphical build, and the headless binary never calls it.
A subprocess fixture for the headless path does not help there; that block still
needs the phase fixture §4.2 names.

The fixture is offline and deterministic — no production endpoint is contacted,
and no test can see another test's state:

| piece | what it is |
|---|---|
| account | `{home}/.future/agent/auth.json` with `future.key` + `future.base_url` pointing at the test's loopback listener (the shape the module's own `an_accepted_platform_key_needs_no_terminal` uses) |
| pairing | `{home}/.future/remote_pairing.json`, `handshakeVersion: 2` **with** `secure` (so `load_creds_checked` accepts it and `establish` takes the refresh path), a *real* user NKey seed generated in the test (`nkeys::KeyPair::new_user()`), and `natsUrl` on a closed port |
| platform | a loopback HTTP server owned by the test: `GET /client/v1/account/profile` → 200; every other path answers per arm (5xx, or 401 `invalid_remote_credential`) |
| Agent | `FUTURE_AGENT_GRPC_ADDR` is *removed*, so `select_agent("auto")` sees no external endpoint and takes `LocalStart`: the run starts the real embedded Agent in its isolated `FUTURE_HOME` |

### 8.2 The arms, and what each one asserts

Three tests in `desktop/src-tauri/tests/headless_cli.rs`, gated
`#[cfg(all(unix, feature = "headless"))]` like the file's existing subprocess
tests. The assertions are about behaviour, not line touches.

| test | state | asserted behaviour |
|---|---|---|
| `a_deferred_remote_start_keeps_serving_and_stops_on_a_signal` | platform up, pairing service **down** (5xx) | the run passes login (`Platform login authorized`), reports a **deferred** phase (`Remote: Reconnecting`) instead of exiting, is still serving three seconds later, leaves the saved pairing **byte-identical**, prints **no** invitation without a terminal, and stops cleanly on `SIGINT`; the platform's own request log must show the pairing endpoint was called, so the deferred phase cannot be a dead listener |
| `a_revoked_pairing_stops_the_run_with_the_reason_and_drops_the_credential` | platform up, pairing **revoked** (401 `invalid_remote_credential`) | the run passes login, then **fails** (exit 1) naming the reason (`Remote connection stopped (… CredentialRevoked …)`), prints no invitation, and the unusable credential is **dropped** rather than served forever |
| `a_non_auto_agent_mode_never_starts_an_embedded_agent` | `FUTURE_DESKTOP_AGENT_MODE` = `external`, then `bogus` | both are refused (exit 1) with their own reason, before login: attach-only mode must never start an embedded service, and an unrecognised mode is a configuration error, not a guess |

Measured wall time alone: 5.2 s, 3.0 s and 2.6 s; the whole integration target
12/12 in 8.5-11.8 s over four runs, including one *instrumented* run at load 65.

### 8.3 The lines this segment closed

| line(s) | file | what runs there |
|---|---|---|
| 128-139 | `headless/mod.rs` | `ensure_login` (the profile check against the fake platform), the account key and platform URL captured for the loop, and the tolerated model-catalog refresh |
| 142, 145-146 | `headless/mod.rs` | the headless scheduler start, the built-in-skill bootstrap spawn, and the `--re-pair` guard taken as *false* |
| 154-170 | `headless/mod.rs` | the six background projections, the headless task executor, the interrupted-run/approval reconciliation, and `remote::start` |
| 171-198, 205-206, 218, 223, 229-230 | `headless/mod.rs` | the supervision loop: `check_running`, the logout/platform-change guard, the phase line, the invitation/expiry bookkeeping and the re-read of the pairing from disk |
| 203-204 | `headless/mod.rs` | the terminal-phase exit (`Failed`/`Revoked`/`Stopped` → stop with the reason), from the revoked arm |
| (3 lines) | `headless/agent.rs` | `Agent::ensure_running`'s body against the real embedded Agent, which no test had ever started |
| 160, 208 | `agent_host.rs` | the agent-mode guard and the attach-only refusal, both `#[cfg(not(test))]` |
| 103 | `cli_tool.rs` | the `"development"` source label — the `#[cfg(not(test))]` candidate list exists in the binary the integration test spawns (§8.1) |
| 456, 479, 511, 535 | `cli_tool.rs` | the four hand-written fixture restores, **deleted** by the guard of `docs/testing/flaky-tauri.md` (the new `EnvGuard::drop` `None` arm is the one residue left, listed in §4.1) |
| 744, 785, 786 | `terminal/pty.rs` | two of the reap-loop deadline arms, taken this time (§4.1 keeps the other polls) |
| 548 | `terminal/server.rs` | the "any other client frame is ignored" arm. **Not** 533: that disposition was false as written — 533 is one of the oscillating pair, and in the current pin it reads `DA:1`, so it carries no row (§2.0), measured 0 in 3 of 5 runs at the revision this claim was made on, so it was a run-dependent cover read as a closure. **533 reads `DA:1` in this pin and is not claimed; if a future pin reads it `DA:0`, a row must be created for it (§2.0).** |
| 258 | `headless/mod.rs` | a normal completion of `starting_with_notice`'s awaited future (the slow-start notice never wins, so the call returns at that line) |

Not closures, for the avoidance of doubt: `810`, `941` and `1086` read as *covered* in an early reconcile pass only because the segment-3 report renumbers them (the unit's own gate fixes inserted lines). Under the segment-3 numbering the same three lines are `811`, `948` and `1098`, and the report shows all three **uncovered** — they are cited in §4.1, not closed.

### 8.4 What stays OPEN in that block, with the next step

§4.2's `headless/mod.rs` row carries the 24 lines and the four next steps; the
short version, because the reasons are mechanical rather than stylistic:

* **207-227 (12 lines)** — the pairing invitation, the `Ready` hand-over and the
  expired-invitation refusal need a `RemoteStatus` that carries a pairing code or
  `phase == Ready`, which only a *successful* `remote::start` produces. That needs
  a live NATS broker: `connect_nats` sets `require_tls(true)` outside `cfg(test)`,
  and the crate's in-process fake broker lives in `src/remote/test_support.rs`
  behind `#[cfg(test)]`, so it is not linked into the binary the test spawns. So
  these are `unreachable-in-this-environment` *for this host* — a broker with JWT
  auth and TLS would close them, and the harness is otherwise ready for it.
  **Corrected by TAU-D16 (§13.3):** that reasoning holds for the spawned *binary*;
  a re-exec'd **libtest child** is the `cfg(test)` build, so `FakeNats` *is*
  linked there and `require_tls` is compiled out. What remains missing is a pty
  (for 208-216) rather than a broker.
* **147-149** — `--re-pair` calls `require_terminal()` first. `portable-pty` is a
  dependency, so a PTY fixture is the next step, not a seam.
* **188** — the `available` arm of the phase line: with an unreachable pairing the
  supervisor reports `agent_available: false`, so the arms above take the other
  arm. An arm where the Agent is Ready and the remote start is not would close it.
* **136-138, 182** — the *failure* branch of the model-catalog refresh (the arms
  above had a reachable catalog) and the "platform login changed" exit (needs the
  account key to change mid-run).

### 8.5 The segment-3 measurement

Baseline and after are both in §2.2 (the re-measurement of this revision is what
the supervisor asked for, because after-2 predates the unit's gate fixes and the
`cli_tool` guard). Summary: **365 → 300** uncovered lines on the scope,
`headless/mod.rs` **104 → 51**, `headless/agent.rs` **4 → 1**, `skills_bootstrap`
28 → 17 *of which 11 are a measurement artefact* (§2.2), `agent_host` 24 → 26 with
four oscillating external-attach lines named. The after figure is the union of the
gui leg, the headless leg and one re-run of the integration target; `cargo llvm-cov
report` exit 0; lcov `/tmp/tauri-tau3-union-lcov.info` (beside
`/tmp/tauri-tau3-before-lcov.info`).

**§5.3 statement.** The three executions that make up the after figure each came
back red, and every failure is named with its cause: `remote_peer::runtime_tests`
×6 and `remote_peer::transfer_tests` ×1 (the documented host-wide `8022`
bind-contention family, outside this scope), `cli_tool::tests::a_runnable_cli_on_path_is_discovered_and_probed`
(the 3 s probe deadline under load, in scope, **cost 0 lines** — that file's
uncovered set is identical to the pre-segment-3 run where the same test passed),
and the segment-3 revoked arm itself, whose first instrumented run failed on the
fixture bug of §8.6 and cost `headless/mod.rs` line 203 — recovered by the re-run
that is part of the union. So: no failure of the union leaves a line that the
segment's tests would have covered unaccounted for, and the direction argument
holds (a union of executions cannot report a line as covered that no run reached).

### 8.6 F3 — a fixture that changed which arm it was measuring (macOS `O_NONBLOCK`)

The first instrumented run of the revoked arm did not fail; it **measured
something else**. The child's log showed
`Failed to refresh remote credential: error sending request for url (…auth/token)`
and `Remote: Reconnecting`, i.e. the *deferred* arm of the previous test — while
the assertion waited 300 s for an exit that this arm never produces.

Mechanism, and it is a platform trap worth knowing: on macOS a socket accepted
from a **non-blocking** listener inherits `O_NONBLOCK`, so a handler's `read` that
arrives before the client's bytes returns `EAGAIN`
(`Resource temporarily unavailable (os error 35)`) instead of waiting. The
fixture's per-connection handler treated any read error as fatal, closed the
connection, and the client reported a transport failure — which the entry point
correctly turns into a *deferred* start. The whole arm had quietly become a
duplicate of the wrong test, and the failure looked like a timeout.

Fix: `set_nonblocking(false)` (and an explicit read timeout) before reading, one
thread per connection so a stalled connection cannot hold up the next request, and
an accept loop that records transient errors and keeps listening instead of
returning. The arm now also asserts *which* endpoints the platform served, so this
class of silent swap fails loudly next time. Verified: 12/12 on four runs,
including an instrumented one at load 65 — the load that had produced the wrong
arm. This is the same family as §5.2 F2 (a fixture whose behaviour, not the code
under test, decided what was measured).

### 8.7 PLAN §5.2 tripwire, per scope (segment-3 report)

`reconcile.py` needs one `--scope` per file prefix, so it was run six times
against `/tmp/tauri-tau3-union-lcov.info`:

```bash
for s in headless terminal cli_tool.rs agent_host.rs agent_supervisor.rs skills_bootstrap.rs; do
  python3 .future/cov100/reconcile.py --report /tmp/tauri-tau3-union-lcov.info \
      --scope "desktop/src-tauri/src/$s" --doc docs/testing/module-tauri-headless.md --label TAU-D3
done
```

Result for every scope: **MISSING 0, STALE 0** (0 COVERED, 0 ABSENT) — 300
mentioned, 300 uncovered. Two findings came out of running it as the segment's
first act, both fixed in the rows rather than in the check:

* the previous ledger had a **MISSING** line — `headless/agent.rs` 12 was never
  named (the row said "10-11, 15") — which the renumbered row now carries;
* 16 rows cited lines the *new* report shows covered (the closed lines of §8.3),
  and one cited a line that no longer exists (`cli_tool.rs` 567 → 598 after the
  guard). The steering's warning applied exactly here: a row claiming an uncovered
  line that the report shows covered is a ledger **overstating** its residual, so
  each was moved out of the ledger (into §8.3's prose) rather than re-pointed.



## 9. TAU-D12 (2026-10-18) — one subprocess fixture per module closes the `agent_host` / `agent_supervisor` block

### 9.1 The route that was tried first, and why it cannot produce these cases

The instruction was to try the real binary first — segment 3 closed `agent_host`
160 and 208 through `tests/headless_cli.rs`'s `CARGO_BIN_EXE_futureos-headless`
child — and to say which cases it cannot produce. It cannot produce any line of
this block, and the reasons are structural rather than incidental:

* `agent_supervisor` is `#[cfg(feature = "gui")]`, so the headless binary does
  not compile the module at all. Its `#[cfg(not(test))]` half (`ensure_agent_running`)
  is reached only from `commands/login.rs`, i.e. from a Tauri command in a GUI
  run, and the campaign harness has no GUI run (§4.1's 33 / 41-45 rows keep that
  mechanism).
* the `agent_host` lines left after segment 3 are not *entry-point* arms. They
  need a **phase the headless entry point never produces** (`Ready` on a second
  `start`, `Stopped` at a quit decision), a **stop it never performs**
  (`headless::session` has no stop), an **attach it never tries** (it removes
  `FUTURE_AGENT_GRPC_ADDR` and always starts locally), or a **cancellation window
  inside one start**. A child that runs to `Ready` and exits covers none of them,
  which is exactly what the D9 measurement showed: the 22 residual lines were
  unchanged after the headless-binary tests landed.

So the fixture is the `run_case` pattern of `src/agent_host/tests.rs`, moved into
each module's own `#[cfg(test)] mod block_fixtures`: the case re-executes the
libtest binary (`--exact …::block_fixtures::host_fixture --test-threads=1`) with
its own `HOME`, `USERPROFILE`, `FUTURE_HOME` and `FUTURE_AGENT_SOCKET`. The
fixture lives in the module it drives because `src/agent_host/tests.rs` is
outside this unit's write set — and because the supervisor's cases need the
module's own private state (`QUIT_PENDING`, `begin_quit`) to be observable
through its public behaviour.

### 9.2 The cases, and what each one asserts

Fourteen tests, each a fresh process: seven in `agent_host.rs`, seven in
`agent_supervisor.rs`.

| module | case | state | asserted behaviour |
|---|---|---|---|
| `agent_host` | `stopping` | `desktop_shutdown::begin("fixture")` has closed admission | `start` refuses with "Desktop is stopping" and leaves the host `Idle`/unowned — no worker is spawned |
| `agent_host` | `ready-repeat` | a mock Agent answers `select_agent` (external attach) | the first `start` reaches `Ready` **without owning** the service; a second `start` returns `Ok` in the same phase; `stop` on an attached host drops it to `Stopped` without signalling the owner |
| `agent_host` | `timeout` | a TCP peer accepts and never answers, deadline shortened (§9.3) | `start` fails with "Agent startup timed out", the phase is `Failed`, and the cleanup `stop` reports the same reason |
| `agent_host` | `stop-deadline` | a FIFO at `<sessions>/b.jsonl` blocks the legacy import | `stop` fails after `STOP_TIMEOUT` with "Agent cleanup timed out; restart requires process exit" and the phase is `Failed` |
| `agent_host` | `external-stop` | mock Agent + FIFO at `auth.json` (blocks the external-owner policy check) | a stop landing after the probe but before the phase re-read is a **cancellation**, never a `Ready`: `start` fails, the phase settles `Stopped`, nothing is owned |
| `agent_host` | `cancel-prepare` | FIFO at `auth.json` (blocks the channel-environment default, before `prepare`) | the typed cancellation is folded into a clean `Ok`, the phase settles `Stopped` |
| `agent_host` | `moved-home` | FIFO at `auth.json`; `FUTURE_HOME` changes while the worker is blocked | `prepare` refuses with "FutureOS home changed…", the phase is `Failed`, and nothing is created under the moved root |
| `agent_supervisor` | `starting` | an unanswering peer holds the host in `Starting` | `agent_status` is `"starting"` with no error and no shutdown snapshot, and `on_close_requested` **confirms** (an unreadable count is not zero) |
| `agent_supervisor` | `failed` | a held instance lock makes the worker fail | `agent_status` carries the worker's own refusal as `error`, phase `unavailable` |
| `agent_supervisor` | `unreachable` | nothing listening at all | the poll reports the transport error instead of a phase, and no stop snapshot |
| `agent_supervisor` | `owned` | a real embedded Agent, started and stopped | `ready` with the build version; an owned idle Agent proceeds on close (six activity fields summed); a `Stopped` host proceeds too |
| `agent_supervisor` | `failed-stop` | `fail_for_test` + `stop("fixture")` | the status is `"stopping"` **with the failure text** and a snapshot — the `state == "failed"` arm |
| `agent_supervisor` | `quit-barrier` | `claim_action("update")` holds the barrier | `request_quit` starts no stop and publishes no snapshot |
| `agent_supervisor` | `quit-repeat` | the first quit publishes its stop | the second request is answered by the first (no second waiter, one snapshot) |

The assertions are about behaviour, not line touches: the timeout case asserts
the deadline *and* its reason, the stop-deadline case asserts the elapsed time is
at least `STOP_TIMEOUT`, the moved-home case asserts the moved root stays empty,
and the `starting` case asserts the quit decision (not merely the phase string).

### 9.3 Fixture mechanics that cost a debugging cycle

* **A FIFO is the phase switch.** `auth.json` and `sessions/<id>.jsonl` are read
  with `File::open` + `read`, so making them FIFOs blocks the worker at exactly
  one statement. `ENXIO` on `open(O_WRONLY|O_NONBLOCK)` means "no reader yet",
  so the parent can wait for the worker to arrive instead of sleeping.
* **But `open` and `read` are two different moments** (the F3 class of §8.6).
  `open` returns as soon as *a writer* exists; the import's next act is a
  `cancellation.check()`, and only then the blocking `read`. Cancelling between
  the two makes the import abort at the checkpoint instead of sticking — the
  `stop-deadline` case observed that as a 1-in-5 flake and now lets the reader
  settle into `read` (200 ms) before it cancels. Deterministic since.
* **A unix socket path is at most 104 bytes on macOS.** The first version put the
  fixture socket under `target/host-fixtures/.tmpXXXX/agent.sock` (~111 bytes);
  `connect` then failed with `ENAMETOOLONG`, which `select_agent` classifies as
  "local endpoint could not be safely classified as absent" — a *plausible* error
  from the wrong cause. The socket now lives in the short system temp dir.
* **`app.exit(0)` on `tauri::test::MockRuntime` panics** (`request_exit` is
  `unimplemented!()`), in the waiter thread, after `QUIT_CONFIRMED` is stored.
  Harmless for the two quit cases, but it means those cases cannot assert on
  anything the waiter does after the exit.
* **Assertion arguments are lazily evaluated.** The first version of both
  `run_case` helpers passed `String::from_utf8_lossy(…)` inline to
  `assert!(output.status.success(), …)`, which added four residual lines per
  helper to the very files this segment was measuring; they are hoisted into
  bindings now (§4.1 keeps only the genuinely fail-only arms).

### 9.4 What this closed, and what it did not

36 production lines are covered by these fourteen cases: `agent_host.rs` 83, 88,
116, 147-150, 200, 233, 234, 334-336 (13) and `agent_supervisor.rs` 58-62, 70-75,
92-93, 116, 122, 125-130, 190, 193-194 (23). §4.2's two rows are deleted rather
than narrowed.

19 lines remain, all classified in §4.1: the worker-thread spawn failure
(124-126), three phase re-reads inside one start (216, 223, 250), the drain arm's
clean continuation (265-266), one closing brace (281), the `ensure!` message
argument (289), the test-only deadline override's parse join (373), and the two
fixtures' must-not-fire arms. Two of those moved in this measurement: **265** was
`DA:1` in D9 and reads `DA:0` here, and **539** on the other side of the pair
(§4.1). Neither is claimed as closed, and neither is claimed as a range.

**A fact about the API surface is not a fact about reachability.** D9's row called
265 an "external-attach arm"; it is `server.as_mut().await?` in the drain arm, and
the fixture that *does* attach (`ready-repeat`) cannot reach it. The row now says
what the measurement shows: the arm is taken (263, one hit), the report is not
`Stopped` (264, one hit), so the await is never reached in this run.

## 10. TAU-D13 (2026-10-18) — the `skills_bootstrap` stop and TOCTOU block

§4.2 carried one row for `skills_bootstrap.rs`: 28-30 (the spawn failure after a
successful discovery) and 49-54, 69-70, 72, 80 (the drain's stop path). The row
named two mechanisms and neither was a mock: a process-global stop flag plus a
real `CommandChild`, and a TOCTOU the identity re-check was said to close. This
section records what the measurement did to both claims.

### 10.1 Why the cases are subprocesses, and why no mock was needed

`drain_skill_events` takes `Option<CommandChild>`, a value no mock can produce,
and its stop arm reads `desktop_shutdown::is_stopping()` — set by
`desktop_shutdown::begin`, which closes producer admission **for the whole
process and never reopens it**. Calling it in the shared libtest process would
change every drain that runs after it. So each case re-execs the libtest binary
(`--exact skills_bootstrap::block_fixtures::host_fixture --nocapture
--test-threads=1`) with its own `HOME`, `USERPROFILE`, `FUTURE_HOME` and
`FUTURE_AGENT_SOCKET`, exactly as §9's harness does. Two details are inherited
verbatim because they are not optional:

* the socket lives in the **short system temp dir** — a unix socket path is at
  most 104 bytes on macOS (§9.3's `ENAMETOOLONG` trap);
* every assertion argument is hoisted into a binding, because a passing
  `assert!` never evaluates its message and those lines would land in this very
  file's residual (§9.3's last bullet).

The child is spawned by the **real** shell plugin under `MockRuntime`
(`app.shell().command(&bootstrap).spawn()`); §3 already proves that path works,
and `the_bootstrap_spawns_the_cli_through_the_shell_plugin` uses it.

### 10.2 The cases, and what each one asserts

Four tests in the gui flavour (the module is `#[cfg(feature = "gui")]`, so the
headless leg compiles none of it and reports 0):

| case | state | line(s) it exercises | asserted behaviour |
|---|---|---|---|
| `spawn-failure` | discovery succeeds, then the CLI's `#!` interpreter is removed | 28-30 | `resolve_for("skills.init")` is `Ok` *before* the removal; after it, `run_builtin_skills` reports the spawn failure, never runs `init` (marker absent), never panics |
| `stop-live` | `desktop_shutdown::begin("fixture")` with a live plugin-spawned child | 49, 53, 54, 80 | the drain ends in **under 2 s** (it killed the child, it did not wait out the unconfirmed budget), the exit code is `None`, and the pid is gone (`kill(pid, 0)` → `ESRCH`) |
| `stop-unconfirmed` | a grandchild inherits the plugin's pipes and keeps them open | 69-70, 72, 80 | the drain ends **at or after 2 s** — its own budget, because no `Terminated` event can ever arrive — and reports no exit code |
| `host_fixture` | the helper | — | a no-op in the parent (no case variable) and the case body in each child |

Two mechanics made the cases deterministic:

* **`exec sleep 300`** in the stop-live fixture: without `exec` the shell would
  be the direct child and the sleep would survive the kill, so the pid the test
  checks would be the wrong one.
* **Reap before stopping** in the stop-unconfirmed case: the fixture's direct
  child exits at once, but its `sleep 30` grandchild still holds the pipes. The
  test waits until the pid answers `ESRCH` (a zombie still answers signal 0, so
  this is the plugin's wait thread having reaped it) and only then calls
  `begin("fixture")`; it SIGKILLs the grandchild afterwards so the case does not
  leave a 30 s process behind.

The spawn-failure case answers the row's "needs a fault-injection seam": **no
seam is needed.** The identity re-check pins the CLI file (length, mtime, inode,
ctime) and the fixture leaves all four untouched; the program it names with `#!`
is a symlink to `/bin/sh` that the test removes *after* `resolve_for` returned
`Ok`. Discovery passes, the spawn fails, and the failure is the real one a
half-deleted installation produces.

### 10.3 The lines this section closed, and the three it did not

Ten of the row's thirteen lines are covered: **28-30** (spawn-failure), **49, 53,
54** (the kill and `stopping` being set; 53 is the error block's join),
**69-70, 72** (the budget's report and `break`; stop-unconfirmed) and **80** (the
post-loop "termination was not confirmed"; stop-live).

**50-52 are not**, and the measurement says why: both stop cases read the `Ok`
side of `if let Err(error) = child.take().unwrap().kill()`. A live child we
spawned is killable; a child the plugin has already reaped answers `Ok(())`
because `shared_child` reaps through `std::process::Child::wait` and
`Child::kill` is a no-op once the handle is cleared (checked on this toolchain:
`spawn → wait → kill` prints `Ok(())`). The
error arm needs `libc::kill` to fail for a live child this process owns — an
`EPERM`/`ESRCH` state this harness cannot build without a `kill` seam, which this
record does not add. The row is in §4.1, not waived as if it were covered.

The fixture **added four lines** to the same file's residual (349, 393, 525, 526
in §4.1): its 120 s timeout guard, `wait_until`'s timeout answer, the
unknown-case panic and the brace before it. The instrumented file grew 173 →
348 measured lines, almost all of it the fixture itself; that is test code, not
production growth, and §2.6's table states the move as −10 +4.

One correction propagated out of this measurement: the `headless/mod.rs` 313-315
row claimed the identity re-check closed the TOCTOU. It does not — it pins the
CLI, not its interpreter — so that arm is reachable with this same trick. The row
in §4.1 now says so, and the successor's first case is to land it in
`headless/`'s own fixture.

## 11. TAU-D14 (2026-10-18) — the `headless` spawn-failure arm, and two arms that did not close

TAU-D13's §10 ended on a correction the successor was handed: the identity
re-check `cli_tool::resolve_for` performs pins the CLI **file** (length, mtime,
inode, ctime), not the program its `#!` line names, so a CLI whose interpreter
disappears between the probe and the spawn still passes discovery. That is
`headless/mod.rs` 313-315 — `bootstrap_builtin_skills`' spawn-failure arm — and
this section records closing it, then the two arms the ticket named next and why
they are not both closed.

### 11.1 The case, and what it asserts

`a_cli_whose_interpreter_vanished_after_discovery_is_reported_and_skipped`
(`#[cfg(unix)]`, in `headless::tests`):

1. writes a fixture CLI whose shebang is `#!<tempdir>/sh`, where that path is a
   **symlink to `/bin/sh`**, and which answers `version --json` with the
   `skills.init` capability contract — so discovery accepts it and nothing else
   about it is special;
2. asserts `cli_tool::resolve_for("skills.init")` is `Ok` **before** the removal
   (`with_path_and_broken_agent` pins `PATH` to the fixture directory and makes
   `FUTURE_AGENT_GRPC_ADDR` unparseable, so `refresh_skills` fails fast instead of
   reaching a real Agent);
3. removes only the symlink;
4. runs `bootstrap_builtin_skills()` and asserts it returns normally, that the
   CLI's own identity is unchanged (same length, mtime and inode — the re-check's
   three pins, read before and after), and that the `init` marker was never
   written.

The test does not assert on stderr, but the arm's own diagnostic was read with
`--nocapture` in the gate run (§7.5):
`Could not run …/bin/future init: No such file or directory (os error 2)`. That is
the evidence that 313-315 executed rather than merely compiled — the same
standard §9.2's cases are held to.

### 11.2 What closed, and the one number that moved

`headless/mod.rs` **313-315** are covered: the residual for the file went **44 →
41**, and the unit total **240 → 237** (§2.7). Nothing was added to the residual.

The same measurement moved one line **number**: `mock_http_server`'s accept-error
`return` was 1131 and is now **1205**, because the new test sits above it. It is
still uncovered (`EPERM`/`ECONNABORTED` from a loopback accept is the "broken
host" class), and §4.1's row follows the number rather than claiming it as new.

### 11.3 The model-catalog arm (136-138): the ticket's fixture cannot reach it

The ticket's next item was "the module already runs a mock HTTP server for the
profile; make the catalog endpoint fail while the profile endpoint succeeds".
That is a fixture for a different call. Reading the path:

* `scheduler::refresh_future_models_now()` → `run_explicit(…)` →
  `agent_bridge::sync_future_models()`, which sends the **embedded Agent's**
  `sync_future_models` RPC (`agent_bridge/queries.rs:643-664`);
* the Agent's handler always answers `RpcResponse::ok` — it reports
  `{"synced": bool, "modelCount": n, …}` and never a failure
  (`agent/src/rpc/commands/providers.rs:31-59`);
* the sync it wraps returns **`false`** when the platform cannot be reached or
  there is no key, instead of propagating the error
  (`agent/src/models/future.rs:643-659`);
* an unavailable Agent does not fail the call either: `connect_agent()`'s only
  error yields `Ok(SyncFutureModelsResult { synced: false, .. })`
  (`queries.rs:644-655`).

So a platform whose catalog endpoint returns 500 produces `Ok`, `synced: false`
and **not** 136-138. The arm needs the RPC *transport* to fail after a successful
connect (the Agent dying mid-call) or a payload that does not decode. §4.1 now
carries it as `unreachable-in-this-environment` with that mechanism, and the
§4.2 row's proposed fixture is deleted rather than left as a lure for the next
unit.

This is the third time in this record that a row's mechanism described the API
surface rather than the executed path (§9.4's 265, §10.4's 50-52). The check that
catches it is cheap: read the *caller's* callee, not the caller's name.

### 11.4 The mid-run login arm (182): reachable, blocked by the write set

`session()` captures `account_key` at 129 and, once per second, compares it — and
the platform URL — against a fresh `auth.json` read (179-180). Both
`future_login::future_api_key()` and `future_platform::current_platform_url()`
re-read the file on every call, so **any** rewrite of `auth.json` while the run is
live makes the next iteration return at 182. The mechanism is not in doubt; what
is missing is a test that owns a running child, and that test lives in
`desktop/src-tauri/tests/headless_cli.rs`, which already owns exactly such a child
and a `signed_in_home` writer.

That file is outside this unit's declared write set (`src/headless/` +
`module-tauri-headless.md` + `pairings.json`), so the arm stays in §4.2 with the
next step spelled out, and it is the cheapest OPEN item left. A lib test is not an
alternative: `session()` drives `agent_host::host()`, the remote `SUPERVISOR` and
shutdown admission, the per-process singletons `tests/headless_cli.rs`'s header
already explains.

### 11.5 What this segment did not do

* It did not widen a `cfg(test)` seam, add a `kill`/`spawn` fault hook, or refactor
  production code to make 136-138 testable. The row's classification changed
  because the code says the ticket's fixture reaches a different statement.
* It did not touch `tests/`, `pty.rs`'s reap-spin family, or the 207-227 block
  (still needs a live NATS broker with JWT and TLS — a mechanism TAU-D16
  corrected in §13.3: the test build does not need one).

## 12. TAU-D15 (2026-10-18) — `check_running`'s `Phase::Failed` arm, in a child process

The ticket's first item was `headless/mod.rs` **182**, to be closed in
`desktop/src-tauri/tests/headless_cli.rs`. The declared write set was **not**
extended to that file, so 182 stays OPEN with its next step in §4.2 and this
section is the ticket's stated fallback: `headless/agent.rs` **15**,
`check_running`'s `Phase::Failed(error) => Err(error.into())`.

### 12.1 Why this one needs a subprocess too

`check_running` reads the process-global `agent_host::host()`. The arm needs the
host to *be* `Failed`, and nothing in the crate resets it — production never needs
to, so there is no seam and this record did not add one. A lib test that pushed
the host into `Failed` would leave it there for every later test in the same
process; that is the same process-global problem `desktop_shutdown::begin` poses
for §10 and `QUIT_CONFIRMED` poses for §9, and it has the same answer: re-exec the
libtest binary.

### 12.2 The case, and how "the case ran" is established

`a_failed_host_is_reported_with_its_own_error` → `run_case("failed")` re-execs
`--exact headless::agent::block_fixtures::host_fixture --nocapture
--test-threads=1` with its own `HOME`, `USERPROFILE`, `FUTURE_HOME`,
`FUTURE_DESKTOP_AGENT_MODE=auto` and a `FUTURE_AGENT_SOCKET` in the short system
temp dir (a unix `sun_path` is 104 bytes on macOS).

The child then:

1. takes `agent/agent-instance.lock` with `try_lock_exclusive` — the same trick
   `agent_supervisor::block_fixtures`' `failed` case uses, and a *public behaviour*
   path to `Phase::Failed` rather than a test-only setter;
2. calls `host().start("auto")` and takes `.err()` — the start is refused because
   the lock is occupied;
3. reads `host().phase()` and asserts it is `Failed(_)`, so the arm is reached in
   the state it exists for and not merely after one failing call;
4. calls `Agent.check_running()` and asserts the reported text **equals** the
   start's own error.

**Two pieces of evidence, because neither proves the case ran alone.**

* The child prints `HEADLESS-AGENT-CASE-RAN failed` after its assertions pass, and
  the parent requires that sentinel **and** a zero exit. Without the sentinel, a
  `--exact` filter that selected no test at all would also exit 0 — an exit status
  is not evidence that a case ran. Negative control: renaming the filter to a
  nonexistent case prints **0** sentinels (§7.6), so the check is live.
* The arm's own effect is in the child's own log line:
  `Agent host finished owned=false phase=Failed("Cannot start a local Agent:
  instance lock is occupied or unavailable: Resource temporarily unavailable
  (os error 35); do not start another Agent")`.

### 12.3 The first draft cost 8 residual lines; the second costs 3

The first version of this fixture was measured (that run is not cited anywhere
else) and added **eight** lines to `headless/agent.rs`: five for a timeout guard
that hoisted `stdout`/`stderr` bindings before its `panic!`, one for a `match`
arm `Ok(()) => panic!(…)`, and two for a case-dispatch `panic!` and its brace.
Every one of those is a *must-not-fire* arm, so the file it was measuring grew by
more than the line it covered.

The rewrite applies rules this record had already paid for:

* **one line per failure message** (§9.3, and `lib.rs`'s own note): the timeout
  panic now carries `{output:?}`, which holds both streams, instead of two hoisted
  `String::from_utf8_lossy` bindings — 5 lines → 3;
* **`assert!` instead of a `match` arm** for "the start must be refused": `start
  .err().map(|error| error.to_string())` is a covered expression, and
  `assert!(refused.is_some(), …)` is covered because it runs on every passing run
  — the panic arm disappears;
* **`assert_eq!(case, "failed", …)` instead of `if case == … { } panic!`**: one
  covered assertion replaces two uncovered lines.

Result: `headless/agent.rs` 1 → 3 uncovered instead of 1 → 8, i.e. the fixture now
costs 3 lines for 1 closed rather than 8. The remaining three are the guard's
`kill`, `wait_with_output` and `panic!` — a run where the child never finishes —
and are `unreachable-by-construction` in §4.1.

The cost is not zero and the total went **up** (237 → 238). That is stated in
§2.8/§4.3 rather than hidden: the unit's headline is a count of `DA:0` lines, and
harness code lives in the files it measures.

### 12.4 What this section did not do

* It did not touch `tests/`, so item 1 of the ticket (line 182) is untouched and
  remains the cheapest OPEN line.
* It did not add a reset seam to `agent_host` — that would be a production-shaped
  change made for a test, which this record refuses (§6 keeps seams as decisions).
* It did not re-open `headless/mod.rs` 136-138, which §4.1 holds with the Agent-RPC
  mechanism.

## 13. TAU-D16 (2026-10-18) — a filing decision, and a mechanism that did not survive the check

The ticket was a decision, not a hunt: close `headless/mod.rs` 182, or reclassify
the rest. It named two things to check before acting — the write set for 182 and,
for the rest, that each §4.2 mechanism is true. The second check changed the plan.

### 13.1 Item A (line 182) — blocked exactly as predicted

The declared write set for this turn again lists `src/headless`, this record and
`pairings.json`; `desktop/src-tauri/tests/headless_cli.rs` is still outside it. The
cheap case is therefore not buildable here, and 182 stays OPEN with its three-step
shape in §4.2. Nothing was invented to work around it: no lib test (the
`agent_host`/`SUPERVISOR`/admission singletons in that file's header are why), and
no production seam.

### 13.2 The four `session.rs` lines — moved, with a mechanism each

`terminal/session.rs` **220, 222, 223, 231** were §4.2's "needs a reader fault
seam". Read against the ledger's definition — *"This is not platform-shaped: a
broken filesystem, a full pipe, `ENOSPC`, or **a race window with no injection
seam**"* — each is one of:

| line | what it needs | why this host cannot produce it |
|---|---|---|
| 220 | the last `Arc<Session>` dropped **between** a `read` returning `n > 0` and `weak.upgrade()` | the session owns the pty, so dropping it closes the master first and the next read is `Ok(0)` — 217 has **154** hits in the same run while 219 (a successful upgrade) has **57** and 220 has **0** |
| 222 | a signal landing inside one blocking `read` | no injection point, and no `SA_RESTART` control a test owns |
| 223 | a non-`Interrupted` read error | closing every slave is `Ok(0)` on this platform — 217 is hit **154** times and 223 **0**, i.e. the read path always ends in EOF |
| 231 | `thread::Builder::spawn` to fail (EAGAIN) | host resource state; the spawn result is checked **154** times and fails **0**; same class as `agent_host.rs` 124-126, already filed here |

That is a filing change with a per-line argument, which is what §4.1 requires. It
moved **no line count**: 238 before and after, OPEN 19 → 15, waived 219 → 223.

The weakest of the four is **223**, and it is named as such: "the read fails" is
not an invariant, it is a claim that this platform's pty gives EOF where another
host could give `EIO`. A reviewer who falsifies it should move that one line back
to §4.2 rather than question the other three.

### 13.3 The 14 pairing lines — the ticket's mechanism was false, so they stay OPEN

The ticket's Item B asked me to move `headless/mod.rs` 207-216, 224-227 to §4.1
as `unreachable-in-this-environment` **because** "14 need a live NATS broker with
JWT + TLS". I checked that before filing it, and it is true only for the *spawned
binary*:

* `remote/supervisor/start.rs:614` — `let options = options.require_tls(true);` is
  behind `#[cfg(not(test))]`. The **test build does not require TLS.**
* `remote/mod.rs:25` — `pub(crate) mod test_support;` ships `FakeNats` and
  `MockPlatform` into the test build; a re-exec'd libtest child *is* that build.
* `remote/tests.rs:1365-1375` — `start_runs_the_full_bridge_and_stop_winds_it_down`
  asserts `RemotePhase::Ready` **with a `pairing_code`**, using exactly those two
  doubles. It runs today in the lib suite (§7.6's legs include it; it failed there
  only on the port-8022 family).

So a child process in **this unit's** write set can produce the status the block
needs, and a child is also what resets the singletons whose "process-global" nature
is the stated reason `session()` "cannot be run" by a test (`tests/headless_cli.rs`
header). What is actually missing is a harness and, for part of the block, a pty:

* **224-227 (~4 lines) need no pty.** An *expired* pairing code makes
  `invitation_link` return `None`, so 206-217 is skipped, `paired` is false, and
  223's `invitation_expiry ≤ now` returns at 224. `207` is reachable too: with a
  live code, `require_terminal()` is entered and fails on pipes.
* **208-216 (10 lines) additionally need a pty**, because 207's refusal is the
  gate. `tests/remote_start_refusal.rs` already uses `openpty` for that — from
  outside this write set.

Filing all 14 as `unreachable-in-this-environment` would have been a waiver I had
just measured to be wrong, on a row whose next-step cell would then tell a
successor to wait for a broker that is not needed. They stay in §4.2 with the
corrected cell, and the estimate is stated there: ~5 lines for the no-pty subset
against a harness in the measured file, ~14 for the full block with a pty.

**I did not build that harness this turn**, and the reason is the module's own
cost rule (§12.3): it would put 40-80 lines of mock stack — embedded Agent,
`MockPlatform`, `FakeNats`, a pty, a bounded timeout — inside `headless/mod.rs`,
the file being measured, to close 1-5 lines. That trade is worth *deciding on*, not
worth doing silently at the end of a segment; §4.3's item 2 now carries the
estimate so the next unit can argue with a number instead of a hunch.

**The first probe is cheaper than the harness, and it is where I would start:**
`session()` line 145 calls `crate::runtime::spawn(bootstrap_builtin_skills())`, and
that function's body is `#[cfg(feature = "gui")] tauri::async_runtime::spawn(...)` /
`#[cfg(not(feature = "gui"))]` tokio (`runtime.rs:49-62`). Whether the gui branch
works inside a re-exec'd **libtest** child — i.e. whether `tauri::async_runtime`
lazily initialises without an `App` — is unmeasured, and it decides whether this
harness is a 5-line win or a dead end. One test that calls `session()` in a child
and prints what happens at 145 answers it before any mock stack is written.

### 13.4 What this turn is worth

Two things a reader can use, and neither is a new number:

1. **One §4.2 mechanism was false.** Any unit planning work from that cell would
   have waited for a broker that the test build does not need. The cell now names
   the pty, the two doubles and the line numbers that prove it.
2. **Four lines moved from "somebody should build a seam" to a per-line,
   falsifiable reason** — and the weakest of them is labelled as the weakest.

The count did not move, the report did not change, and this section says so rather
than dressing a re-filing as progress: §2.9 records that no leg was re-run because
no code changed, and §13.3 records the one place the ticket's plan was not followed.

## 14. TAU-D17 (2026-10-18) — `session()` in a child: the pairing block closed, and the two measurement traps it hid

TAU-D16 measured that the pairing block's "no NATS broker on this host" mechanism
was false for a re-exec'd **libtest** child, and §13.3 named the first probe. This
section is that probe, its answer, and the three cases it made possible.

### 14.1 The probe's question, answered

`session()` line 145 calls `crate::runtime::spawn(bootstrap_builtin_skills())`,
which in the **gui** build is `tauri::async_runtime::spawn`. Whether that
initialises without an `App` was the open question. Reading says yes —
`tauri/src/async_runtime.rs:284` is `RUNTIME.get_or_init(default_runtime)`, and
`default_runtime` builds a plain tokio runtime — but reading is exactly what this
record has been wrong about before, so it was run: the child reached line 145,
printed `[skills]`-adjacent progress and went on to the remote start. **The answer
is yes; the gui branch works in a libtest child.**

### 14.2 The three cases

One child (`headless::block_fixtures::host_fixture`), re-exec'd with an isolated
`HOME`/`USERPROFILE`/`FUTURE_HOME` and a socket in the short system temp dir. It
starts `MockPlatform` + `FakeNats` (the crate's own doubles, linked in the test
build), signs in, and then runs `session(&Options::default(), &mut agent)` under a
timeout. What differs per case is the platform script:

| case | script | arm reached |
|---|---|---|
| `pairing-handover` | `/remote/auth/token` → 401 `invalid_remote_credential`; `/remote/pair/code` → a **live** code | `establish` clears the revoked credential and mints a new invitation (`start.rs:515-523`), the status is `Ready` with a code, `pairing_link` returns it, and **207**'s `require_terminal()` refuses — no terminal to show a code on |
| `pairing-expired` | the same, with a code that expired a second ago | `invitation_link` refuses the code, so 206-217 is skipped; `paired` is false (the new invitation is not saved until a phone claims it) and **223-227** stop the run with "invitation expired" |
| `pairing-terminal` | the live-code script, with a **pty** on the child's stdin and stdout | `require_terminal()` passes, and 208-215 print the invitation once, then the single-use note; the parent reads the pty, then **SIGINTs** the child |

Why the revoked-credential script is the entry: with a valid stored pairing,
`establish` returns `(creds, None, None)` — Ready **without** a code — so 206 is
never entered. With no stored pairing at all, `session()`'s own 122 sees `paired ==
false` and refuses at 124 before the remote start. The revoked path is the only
one that both passes 122 and reaches 206 with a code, and it is a real platform
answer rather than a file the harness edits mid-run. That is what makes all three
cases deterministic: no sleeps, no races, 11 s for the four tests.

### 14.3 Two traps, both measured, both now documented by the code

* **A SIGKILLed child contributes no coverage.** The first pty case printed the
  whole invitation and left 208-216 at `DA:0`. An instrumented process writes its
  profile on a clean exit; `child.kill()` is `SIGKILL`, so everything the child did
  was invisible to the report. The fix is a `tokio::select!` on `ctrl_c()` in the
  child and `libc::kill(pid, SIGINT)` from the parent — the same thing an operator's
  Ctrl+C does, and the only shape in which a "the run never returns" case can be
  measured at all.
* **A reader that stops early can change which statement runs.** Stopping the pty
  read at the invitation's label left the QR's `write_all` blocked (the pty buffer
  fills), so the `eprintln!` at 214 never ran and the case looked like a bug in the
  entry point. The loop now keeps draining for 3 s after the label.

Both are the same class as §10.4's kill-after-wait and §8.6's `O_NONBLOCK`: the
fixture, not the code, decides which statement is reached.

### 14.4 What this closed, and the one line it did not

Thirteen lines are covered: **207**, **208-212**, **214-216** and **224-227**.
§4.2's `207-216, 224-227` row is deleted rather than narrowed.

**213** is not covered and is not claimed as such: it is the `)?;` continuation of
the multi-line `print_invitation(...)` call, and the call ran — 209-212 and 214-215
are `DA:1` in the same report. §4.1 files it as `attribution-artifact` with those
five neighbours as evidence. It is the only line of the fourteen that the fixture
cannot produce, and a different formatting of that call would remove it.

The fixture costs **7** lines of its own (§4.1: the pty loop's EOF/error arms, the
SIGINT escalation and the `run_case` timeout guard). The trade is 13 closed for 7
added plus one oscillation (`agent_host.rs` 265, back to `DA:0`) — **238 → 233**,
and the scope's OPEN column goes from 15 to **1**.

### 14.5 What is left, in one sentence

`headless/mod.rs` **182**, the mid-run login change, is the only reachable line
this unit has left, and it needs one file of write scope
(`desktop/src-tauri/tests/headless_cli.rs`) that has been outside the declared set
for three turns. Everything else in the twelve scopes carries a mechanism a
reviewer can attack (the weakest are named: `session.rs` 223's "the pty gives EOF
here", `server.rs` 533/535/539's ordering, `agent_supervisor.rs` 200-210's native
dialog).

## 15. TAU-D18 (2026-10-18) — the record's last line, and a waiver that did not survive review

The ticket for this turn was a decision: get `tests/headless_cli.rs` into the write
set and close `headless/mod.rs` 182, or record the module as complete for this write
set. It also named the record's weakest rows as the thing worth re-reading. Both
happened; the second produced more than the first.

### 15.1 The one OPEN line: refused again, and now recorded as blocked

`desktop/src-tauri/tests/headless_cli.rs` is outside this unit's declared write set
for the **fourth** consecutive turn. So 182 stays OPEN, and per the ticket's Item B
this record now says what that means in one place:

* the scope is **complete at 232 for this write set**: 231 lines classified in §4.1,
  **1 line** (§4.2) that is reachable and blocked only by scope;
* 182 is **not** waived and **not** moved to §4.1 — it is reachable, a reviewer
  should see it as reachable, and calling it unreachable would be the kind of
  mechanism this record has now retracted once (§15.2);
* what closes it is one file of write scope and about 25 lines in it (the
  three-step case in §4.2). Nothing else in the twelve scopes is reachable.

Re-dispatching this same request each turn without that decision is what the ticket
asked to stop; the record now states the floor and the exception instead.

### 15.2 The waiver that was wrong: `session.rs` 220

§4.1 filed 220 as `unreachable-in-this-environment` with this mechanism: *"the
session owns the pty, so dropping the last `Arc<Session>` closes the master and the
next read is `Ok(0)`"*. Reading the callee (`PtySession::reader` →
`try_clone_reader`) shows that is false: the reader thread holds **its own
duplicate** of the master fd, so dropping the session kills the child and closes
the session's copy, while the reader's copy stays readable. Any bytes still in the
pty after the drop are delivered to a `Weak` that now upgrades to nothing — **220
executes**.

That is exactly the failure mode the ticket warned about in a different row and this
record has now paid for five times (§9.4, §10.4, §11.3, §13.3, here): *a mechanism
about the API surface is not a mechanism about the executed path*.

The fixture (`terminal/session::tests::a_dropped_session_ends_its_reader_instead_of_resurrecting`):

1. spawns `sh -c "yes flood"` — a child that keeps a backlog in the pty, so the
   drop cannot land on an idle `read` (a pipeline was the first attempt and was
   wrong: `kill` then only reaches the shell, and the grandchild survives);
2. waits until output has reached the buffer (the reader is demonstrably live);
3. drops the last `Arc<Session>` and asserts the drop is prompt (< 2 s);
4. spawns a **new** session and asserts it still drains its child — a reader that
   had not returned would still be holding the dead session's cloned master.

`DA:1` for 220, measured twice: once in a filtered probe and once in the full T18
union. Cost: **0** residual lines.

The first version of the fixture asserted `!kill(pid, 0)` after the drop and
**failed** — a SIGKILLed child is a zombie that nothing reaps in a test, and a
zombie answers `signal 0`. That is the same trap §10.4 recorded for
`skills_bootstrap`; the assertion was replaced rather than the signal check being
weakened.

### 15.3 What the count moved, and what it did not

**233 → 232.** 220 closed (−1), `server.rs` 539 returned (+1, and T18 is the first
run in which 533, 535 *and* 539 all read `DA:0`), `agent_host.rs` 265 covered again
(−1). The two oscillators are named in §4.1 with their full reading history; the
ledger claims whichever member the report shows, never a range.

**Both legs are green** (2183/0 and 1741/0). The port-8022 family did not fire:
§2.10's suggestion — run when no peer worktree is testing — happened to hold. That
is not evidence the family is fixed, and §2.11 says so.

### 15.4 The remaining weak rows, ranked, so a reviewer starts at the worst

1. `session.rs` **223** — "this platform's pty gives `Ok(0)` where another gives
   `EIO`". The counts support it (217: 155 hits, 223: 0, in the same report), but it
   is a claim about a platform, not an invariant. **The most likely next retraction.**
2. `server.rs` **533/535/539** — an ordering, with 2/5, 1/5 and 0/15 measured. The
   fact that T18 is the first run with all three uncovered is a hint that the
   distribution is rougher than five samples suggested.
3. `agent_supervisor.rs` **200-210** — a native dialog that *did* show in T13 and has
   not since; it is an oscillator, not a constant, and it is filed as such.
4. `session.rs` **222** (EINTR inside one `read`) and **231** (thread-spawn EAGAIN) —
   host states, the same class as `agent_host.rs` 124-126. Unlikely to fall without a
   seam this record declines to add.

## 16. TAU-D19 (2026-10-18) — the module closed out, and two experiments run instead of a sixth request

This turn's ticket offered two branches. The write set still excluded
`tests/headless_cli.rs`, so Branch B was taken (§16.3). Before stopping, the ticket
named two things worth measuring, and both were cheap enough to answer properly.

### 16.1 The `session.rs` 223 experiment: four routes, one answer, and no retraction

TAU-D19 ran the experiment the previous handoff specified, because 223's waiver
("this platform's pty ends in `Ok(0)` where another gives `EIO`") was the last row
resting on a claim about a host rather than an invariant. It was **not** a
throwaway: two segments of this record were spent on assumptions nobody executed.
So the probe was written standalone, run, and then the conclusion written down.

The arm needs the reader's blocking `read` on the pty **master** to return an
error other than `EINTR`. Four documented ways to invalidate a pty, measured on
this host (macOS 15 arm64, Rust 1.97.0), each with a `sleep 30` child unless noted:

| route | call result | what the blocked/served master read returns |
|---|---|---|
| `revoke(2)` on the slave device, read afterwards | **0** (succeeded) | `Ok(0)` — EOF |
| `revoke(2)` landing on an **already blocked** read (silent child) | **0** | `Ok(0)` |
| the same on a child that wrote once first (drained, then blocked) | **0** | `Ok(0)` |
| `TIOCVHANGUP` on the **master** | **0** | `Ok(0)`, and it really woke the read: **323 µs** after the ioctl (1.09 ms with the chatty child) |
| `TIOCVHANGUP` on the **slave** fd | **-1**, errno **25 = `ENOTTY`** | `Ok(0)` after the child exited (the ioctl did nothing) |

So the hangup is not a no-op — it wakes the reader promptly — and the read still
succeeds with EOF. **On this platform every route ends the reader at 217
(`Ok(0) => break`), never at 223.** That is now a measurement with four routes and
timings, not the sentence it was.

Two things this does *not* prove, stated so the row is not read as stronger than it
is: 223 may be reachable on a host whose pty returns `EIO` (Linux does this for
some teardown orders), and this record still cannot say which. The waiver stands,
labelled as it was: a platform claim, now with the platform checked.

### 16.2 The `server.rs` triple: ten independent runs, and what the earlier numbers meant

§4.1 records the pump's three lines as `533` covered 2/5, `539` 1/5 and `535` 0/15
across earlier samples, and T18 was the first run in which all three read `DA:0`.
The ticket asked whether that coincidence means the ordering changed. Ten runs of
the single pump test, **each after `cargo llvm-cov clean --workspace`** so run *n*
cannot inherit run *n-1*'s hit:

```
run 1: 533=0 535=0 539=0      run 6: 533=0 535=0 539=0
run 2: 533=0 535=0 539=0      run 7: 533=0 535=0 539=0
run 3: 533=0 535=0 539=0      run 8: 533=0 535=0 539=0
run 4: 533=0 535=0 539=0      run 9: 533=0 535=0 539=0
run 5: 533=0 535=0 539=0     run 10: 533=0 535=0 539=0
```

**Ten for ten, all three uncovered.** Combined with the six earlier single-test
samples, that is **sixteen** single-test runs in which this fixture never reaches
533 or 539 either. The refinement matters: the 2/5 and 1/5 in the row are not
attributable to *this* test. They came from whole-module runs, i.e. from some other
`terminal::server::` test's interleaving leaving the pump in the state where the
write-after-exit path is taken — and that attribution was never established. The
row now says so instead of implying the pump test covers it sometimes.

What this changes for a successor: running the pump test in a loop is useless. A
deterministic route needs either the ordering seam the record declines to add, or
finding which *other* test in the module leaves the pump writing after exit. That is
a hunt, not a fixture with a promised line.

### 16.3 Branch B: the exception, handed up

The write set was refused for the **fifth** time (TAU-D14, -15, -16, -18, -19 — D17
probed instead of asking), so per the ticket this is Branch B, and it stops here. Two facts belong to the campaign, not to this unit:

1. **One line is neither covered nor registered unreachable.** `headless/mod.rs`
   182 is reachable (its mechanism is proven: `auth.json` is re-read every loop
   iteration) and needs `desktop/src-tauri/tests/headless_cli.rs`, a file this
   unit has never been allowed to write. The 口径 A claim for this scope is
   therefore **"232 named: 231 unreachable-by-registration, 1 blocked by
   test-scope"**, which is not 100 %.
2. **What it would take.** One file added to the write set and ~25 lines in it
   (§4.2's three-step case). If a future turn grants it, the residual becomes 231,
   all waived, and §4.2 becomes an empty table plus a note.

This record cannot write that exception into `.future/cov100/CLOSEOUT.md` or
`ACCEPTANCE.md`: both are outside this unit's declared write set (the same contract
that produced the exception). It is therefore stated here — in §1's close-out box
and in this section — and in the delivery report for the turn that closed the
module, which is where the supervisor reads it.

**Resolved on 2026-10-18 (TAU-D27), and kept here as written rather than rewritten.**
The write set was granted on the fourth request, the case landed as
`tests/headless_cli.rs::a_login_change_while_supervising_stops_the_run_with_that_reason`
(commit `8efb4fa0c`), and `182` reads `DA:1` in the two-leg union (§2.13). The scope is
now **236 = 236 waived, 0 OPEN** (§4.2, §4.3). The paragraph above is the state of the
record *before* that grant; it is not deleted, because "blocked by a contract" is the
reason the line sat there for four turns and a reader comparing turns deserves to see
it stated rather than quietly dropped.

## 17. TAU-D27 (2026-10-18) — the four `MISSING` lines were one insertion, and the figure is folded in

Ticket `todo_fc4a164262ba`, issued under the label "TAU-D25". Recorded here as **TAU-D27** because
TAU-D25 is the `tauri-appdata` delivery of the same day (`module-tauri-appdata.md` §S18.5) and two
deliverables under one id make the campaign's own transcript ambiguous. What this segment did, in
one sentence: it registered four lines that were never new code, deleted seven rows over a module
that no longer exists, registered the fourteen lines a prose paragraph had been covering for, and
re-pointed the pairing at a two-leg union that already contains D20's fixture — **236 = 236
waived, 0 OPEN, 0 unregistered.**

### 17.1 The four `MISSING` lines: one insertion, four statements, unchanged mechanisms

The starting state, verbatim (`reconcile.py` against the then-authoritative
`pinned/union-postmerge2-green.info`):

```
[tauri-headless] files with uncovered lines=12 uncovered lines=238
  mentioned in module-tauri-headless.md: 234
  MISSING (uncovered, not mentioned): 4
     mod.rs:953
     mod.rs:957
     mod.rs:982
     mod.rs:1206
  STALE (doc cites it as uncovered; old doc sections also match unless --section is given): 12
    COVERED (DA>0 - a real disagreement, the actionable half): 5
       mod.rs:952
       mod.rs:956
       mod.rs:981
       mod.rs:1205
       server.rs:539
    ABSENT (no `DA:` record -- NOT evidence: ...): 7
       absent mod.rs:1447 ...
```

The shape invited two readings, and the ticket named both: a one-line insertion that shifted the
ledger (`952 covered, 953 missing` ×4), or four statements that had moved *and* changed. **It is a
shift, and the proof is the code at both revisions, not the number pattern:**

| statement | T18 revision (`e8c9d0f42`) | this tree (`0b7b2ca17`) | `union-d26` |
|---|---|---|---|
| `None => std::env::remove_var("PATH")` | 952 | **953** | `DA:0` |
| `None => std::env::remove_var("FUTURE_AGENT_GRPC_ADDR")` | 956 | **957** | `DA:0` |
| `Err(error) => panic!("the fixture CLI must be accepted by discovery: …")` | 981 | **982** | `DA:0` |
| `Err(_) => return` (the mock server's accept loop) | 1205 | **1206** | `DA:0` |
| the `Some`/`Ok`/`}` line *above* each of them | 951/955/980/1204 | **952/956/981/1205** | `DA>0` |

So the four `COVERED` rows and the four `MISSING` lines are the *same event*, seen from two sides:
the report now counts the predecessor statement and the claimed statement separately, and the
claimed one is still `DA:0`. What moved them is one line of #1174:

```
$ git diff -U0 e8c9d0f42:desktop/src-tauri/src/headless/mod.rs \
               1a04560f3:desktop/src-tauri/src/headless/mod.rs | grep '^@@'
@@ -861,0 +862 @@ mod tests {          <- devices: Vec::new(), in a RemoteStatus literal
@@ -1021,74 +1021,0 @@ mod tests {
@@ -1264,0 +1192 @@ mod tests {
@@ -1332,310 +1259,0 @@ mod tests {
```

`with_path_and_broken_agent` sits at 939 at T18 and at 940 here; everything below 862 shifted +1,
everything above it (352, 649-651, 836) did not and needed no correction. **The mechanisms are
unchanged**, which is why the rows move by one and nothing is re-argued — and why the ticket's
instruction 2 ("do not delete a row merely because the report no longer says `DA:0`") was the
right one: deleting those rows would have un-registered four genuinely uncovered lines and
shrunk the ledger's claim four lines below the measurement.

### 17.2 The rows that had to move, and the one that had to be written

1. **`953, 957`** (was 952, 956): the `None` restore arms — mechanism unchanged, position +1.
2. **`982`** (was 981): the `Err` arm of the fixture-CLI `match` — mechanism unchanged, position +1.
3. **`1206`** (was 1205): the mock accept loop's non-would-block `Err(_) => return` — mechanism unchanged, position +1 (1131 → 1205 by TAU-D14, 1205 → 1206 here).
4. **`server.rs` 535, 539** (was 533, 535, 539): the oscillating family, and the row now claims only
   what the cited report shows. 533 reads `DA:1` in `union-d26`, i.e. it is *covered* here; a row
   that names all three members forces a non-zero `COVERED` count in every future report, which is
   the permanent contradiction F.103 was written about. The row keeps all three readings in its text.
5. **`headless/mod.rs` 207-216, 224-227 — a row that did not exist.** The record's prose said these
   13 lines were "closed by the TAU-D17 fixture" (and 213 was filed as an
   `attribution-artifact` beside them). All 14 are `DA:0` in `union-d26`, because that
   fixture (`headless::block_fixtures`) was dropped at `32ba1e13e` (F.102) and never replaced. The
   prose kept mentioning their numbers, so `reconcile.py` reported `MISSING 0` — **mentioned is not
   registered**, which is F.103's exact sentence, and this is its second instance. They are now a
   `unreachable-in-this-environment` row with the missing seam named (a pty plus a code that expires
   while the run is up), and **213** returns to that row from `attribution-artifact`, whose evidence
   was the `print_invitation` call at 209-212 — the very thing that stopped running.
6. **`headless/mod.rs` 1447, 1450, 1471, 1472, 1512-1514 — seven rows deleted.** They were the
   dropped module's own guards. A row over a module that does not exist cannot be checked by a
   reader, and these seven were already `ABSENT` (no `DA:` record at all), never `DA:0`, so the
   residual does not move when they go.
7. **`182` — newly executed, not moved.** D20's fixture, `DA:0 → DA:1` in `union-d26` (§2.13). This
   is the one line in the whole scope that moved because someone wrote a test, and it is the reason
   §4.2 is now empty.
8. **`agent_host.rs` 265** — the sixth reading of the campaign's most active oscillator (`DA:1`). It
   was never claimed; §4.1's 266 row lists the readings and this one is appended there.

**How the 14 unregistered lines were found, since the campaign's own tripwire did not see them.** Not
by `MISSING` (it was 0 for them) but by comparing *parsed row claims* with the report's uncovered
set, using the tool's own two halves:

```python
import reconcile as R
claims = {(k.split("/")[-1], ln) for k, ln in R.claimed_uncovered(R.slice_section(doc, "4. Residual uncovered lines"))}
unc    = {(k.split("/")[-1], ln) for k, ls in R.key_files(R.report_lines(REPORT, SCOPES), SCOPES).items() for ln in ls}
unc - claims    # 13 mod.rs lines, all in the paragraph that says "covered"
```

That is worth adding to the campaign's toolbox as a *third* column next to MISSING: **row-claims
minus measured** is the set a reader cannot check even though the doc mentions it. Both directions
are now zero for this scope (`uncovered 236 | row-claims 236`, `uncovered-not-claimed: []`,
`claimed-not-uncovered: []`).

### 17.3 The figure: 236 at `0b7b2ca17`, not the 237 the probe predicted

The probe behind the earlier banner measured one line (`182 = DA:1`) and predicted `238 − 1 = 237`.
The two-leg union shows **three** movers: 182 covered (−1), `agent_host.rs` 265 covered (−1) and
`server.rs` 533/539 trading places (−1 +1). **A probe can see the line it watches and cannot see an
oscillator**, which is precisely why PLAN 13 wants a full run before a figure is published: not
because a probe is unreliable about its own line, but because the figure is a sum and the probe is
not a sum.

The chosen figure is **236 at `0b7b2ca17`**, from `.future/cov100/pinned/union-d26.info` — the
two-leg union whose recipe, artifacts and md5s the `tauri-remote` record already documents, and
whose headless leg is the run that carries `tests/headless_cli.rs`. §2.13 tabulates 238 → 236 per
file. Nothing was published off a probe, and the 237 hypothesis is recorded here as a hypothesis
that the measurement refuted by one line.

**What a successor must do, and it is not optional.** `main` has moved to `74c6bd8aa` and those
commits touch this scope. The banner says so; the figure is a branch value. At the merge: re-run
the two legs, then re-derive §4.3's table and this pairing's `expect` from the new report — the
rows' *mechanisms* should survive (they survived one rewrite), and any line that moves will move
for a reason the code can show, as §17.1 shows one did.

### 17.4 The two sentences this segment owes

```
reconcile tauri-headless: MISSING 0, COVERED 0
staletable --gate: failing 0
```

Verbatim from the tools, over the twelve scopes of the pairing and the report it now points at:

```
$ python3 .future/cov100/reconcile.py --report .future/cov100/pinned/union-d26.info \
    --scope desktop/src-tauri/src/headless --scope desktop/src-tauri/src/terminal \
    --scope desktop/src-tauri/src/cli_tool.rs --scope desktop/src-tauri/src/agent_host.rs \
    --scope desktop/src-tauri/src/agent_supervisor.rs --scope desktop/src-tauri/src/skills_bootstrap.rs \
    --doc docs/testing/module-tauri-headless.md --section "4. Residual uncovered lines" \
    --label tauri-headless
[tauri-headless] files with uncovered lines=12 uncovered lines=236
  mentioned in module-tauri-headless.md: 236
  MISSING (uncovered, not mentioned): 0
                                                       # no STALE block: COVERED 0, ABSENT 0
$ echo $?
0
```

```
$ python3 .future/cov100/staletable.py --gate | grep tauri-headless
tauri-headless             authoritative    module-tauri-headless.md        236    236       0      0      0       0 section       PASS
$ python3 .future/cov100/staletable.py --gate | grep 'authoritative pairings'
authoritative pairings: 15 -> clean 13, justified covered rows 2, unverifiable 0, **failing 0**
$ echo $?          # exit status of --gate
0
```

`COVERED 0` is asserted the way the tool states it — no `STALE` block at all — and checked
independently: `split_stale()` over the section returns `COVERED 0 / ABSENT 0`, and the row-claim
comparison of §17.2 returns **236 parsed claims for 236 uncovered lines, both directions empty**.
(The same comparison before §17.2 item 5 returned 223 claims for 236 lines and named the 14 missing
ones — which is the state `MISSING 0` was hiding.) The gate's own `GATE FAILED: 3 pairing(s)` line
is **not** this pairing: it counts reference roles (`channels-wide` [wide], `tauri-shell`
[superseded-ledger], `cli-wide` [wide]), and F.104 is why the exit code and the authoritative
`failing 0` disagree with that headline — those three are other units' pairings and none of them
is in this scope.

## 18. TAU-D31 (2026-10-18) — the merged tree: 236 → 241, and the rows re-derived rather than renumbered

Ticket `todo_6aa85f2841be`. This segment did **not** re-measure (the union below is the campaign's
retained post-merge artifact, the same one the `tauri-appdata`, `tauri-remote` and
`tauri-shell-seg2` records publish from); it read the merged code, re-derived every
`agent_supervisor.rs` row to its post-merge position, and registered the lines the merge added.

### 18.1 The measurement identity, and the whole of the +5

Report `.future/cov100/pinned/union-refresh4.info` — 5 853 166 B, 178 `SF:` records, md5
`5fb223725194aed41049418eaa3e1224`, measured at `16ea6f388` with both legs `rc=0` (gui and
headless, whole crate, `cargo llvm-cov clean` between legs, unioned by `lcov-union.py`).

**The report describes this tree.** Checked, not asserted:

```
$ git diff --stat 16ea6f388..HEAD -- desktop/src-tauri/src/terminal desktop/src-tauri/src/headless \
    desktop/src-tauri/src/cli_tool.rs desktop/src-tauri/src/agent_host.rs \
    desktop/src-tauri/src/agent_supervisor.rs desktop/src-tauri/src/skills_bootstrap.rs
                          # empty: the twelve scopes are byte-identical at 16ea6f388 and HEAD
```

Per file, `union-d26` (236, §2.13) against `union-refresh4` (241):

| file | 236 (`0b7b2ca17`) | 241 (`16ea6f388`) | attribution of the move |
|---|---|---|---|
| `agent_supervisor.rs` | 42 | **47** | `main`'s force-exit arms: 203, 207, 208 (`force_exit_process`'s `cfg(not(test))` body) and 214, 215 (`arm_force_exit`'s spawn-failure branch) |
| the other eleven files | 194 | 194 | no change; `git diff 0b7b2ca17..16ea6f388` over them touches only `agent_supervisor.rs` |
| **total** | **236** | **241** | **+5, all in one file, all new statements** |

The merge's diff on that file is +73/−4 (`git diff 0b7b2ca17..HEAD`), in seven hunks:
`on_close_requested`'s `QUIT_CONFIRMED` early return (+4 at 134); the force-exit watchdog block
(`FORCE_QUIT_REQUESTED`, `spawn_force_exit_watchdog`, `force_exit_process`, `arm_force_exit`) plus
the head of `request_quit`'s `force` arm (+40); the `let operation = if force` branch and the
changed guard (+2); `main`'s watchdog test (+13); the `force-maintenance` fixture registration
(+5); a one-line fixture edit (+1); and that case's body (+8). The five new uncovered lines are
inside the second group; the rest of the new code is covered by `main`'s own tests
(`forced_exit_deadline_does_not_depend_on_agent_cleanup_or_the_ui_loop`,
`a_forced_quit_can_join_cleanup_during_maintenance`).

### 18.2 Old → new, per row, with the statements checked at both revisions

The statements did not change; their positions did. This is the table §4.1's rows were derived
from (`git show 0b7b2ca17:…/agent_supervisor.rs` against the working tree):

| row (old, `union-d26`) | row (new, `union-refresh4`) | shift | why |
|---|---|---|---|
| 33 | 33 | 0 | before the first insertion |
| 41, 45 | 41, 45 | 0 | same |
| 157-160, 162, 164-182 | 161-164, 166, 168-186 | **+4** | `on_close_requested`'s early return inserted 4 lines at 134; `confirm_quit` (old 157) is now 161. 165 and 167 have no `DA` record at either revision (the `} else {` and `};` sub-lines) |
| — | 203, 207, 208 | new | `force_exit_process` (§4.1) |
| — | 214, 215 | new | `arm_force_exit`'s `is_err()` branch (§4.1) |
| 207-209, 211 | 250-252, 254 | **+43** | the same callback body and the brace after the `return`; the new force-exit block is 43 lines above them. 253 (`return;`) is covered |
| 216-218 | 259-261 | **+43** | the spawn-failure dialog |
| 422-424, 426-427 | 483-485, 487-488 | **+61** | the fixture's timeout guard; 486 (the format string) is covered and 489 is the call's `);`. The macOS mock-loop and force-exit tests added 61 lines above the fixture |
| 461 | 522 | **+61** | the `starting` case's accept-error panic |
| 571-572 | 640-641 | **+69** | the `quit-repeat` fall-through brace and the unknown-case panic (the `force-maintenance` case added 8 more lines between 569 and 571) |

Every row above was checked against the code at **both** revisions: same brace, same `panic!`, same
`show(…)`. Nothing was renumbered by arithmetic alone — the four shift amounts differ (+4/+43/+61/+69)
and were read off the diff hunks, which is the failure mode §17.1 warns about in the other direction.

### 18.3 What the unregistered set actually was

`reconcile.py` reported **MISSING 11** for this pairing, and that number is honest but incomplete.
The row-claim comparison (parsed claims against `DA:0` records, both directions) says what happened:

```
claims 236   resolved 236   uncovered 241
uncovered not claimed: 24
claimed not uncovered: 19   (11 covered, 8 absent)
```

The 24 unclaimed lines are the merge's shift plus its new arms. Only 11 of them were *MISSING*,
because `reconcile.py` matches **cited numbers**, not rows, and 13 of the 24 were cited by number
in neighbouring notes (for example `203` in prose about `arm_force_exit`, `214, 215` in the old
row's "212-215 are covered" evidence). That is this campaign's known false-clean shape and the
reason §17.2's own claim comparison exists. This segment closed it by giving every one of the 47
`agent_supervisor.rs` lines a row: after the edits the same comparison reads

```
claims 241   resolved 241   uncovered 241
uncovered not claimed: []   claimed but not uncovered: []
```

### 18.4 The two sentences

```
$ python3 .future/cov100/reconcile.py --report .future/cov100/pinned/union-refresh4.info \
    --doc docs/testing/module-tauri-headless.md --label tauri-headless \
    --section "4. Residual uncovered lines" --scope <each of the twelve>
[tauri-headless] files with uncovered lines=12 uncovered lines=241
  mentioned in module-tauri-headless.md: 241
  MISSING (uncovered, not mentioned): 0
                                # no STALE block: COVERED 0, ABSENT 0
$ echo $?
0
```

`COVERED 0` is asserted the way the tool states it — no `STALE` block at all — and checked
independently by the claim comparison in §18.3 (`split_stale()` over the section returns
`COVERED 0 / ABSENT 0`). The five new rows are the only claims this segment added; the rest moved.

### 18.5 The one line this segment un-waived: `agent_supervisor.rs` 254

The merge's renumbering made a filing question visible that the old numbering hid. Old **211**
was the closing brace of `if !force { … }` in the waiter, and the row that claimed it described
the surrounding native dialog. At this revision the same brace is **254**, and it is not covered
by that mechanism: the force path **skips** the dialog. Trace:

```
244   if !FORCE_QUIT_REQUESTED.load(..) {      // force quit -> false -> block skipped
245-249   dialog … .show(…)                     // never reached on a force quit
253       return;                               // returns only inside that block
254   }                                         // reached when the block is skipped
255   }
256   QUIT_CONFIRMED.store(true, ..);
257   app.exit(0);
```

So 254 executes on a **force quit whose cleanup failed** — reachable in production, executable
with no dialog host, and absent from this union because the `force-maintenance` case returns at
the barrier (230-238) before the waiter exists, and the union's failing stop (242-249) came from
an *ordinary* quit. That is a missing fixture, not an environment limit, so the row moved from
§4.1 to §4.2 (OPEN) with the seam named there: `fail_for_test` + `request_quit(app, true)` in a
`block_fixtures` subprocess.

This is the only row this segment *moved between classes*, and it moved in the direction the
ticket cares about: out of a waiver and into work. It also answers the "if you deleted the code,
would the line move?" test in the useful direction — deleting the `if !FORCE_QUIT_REQUESTED`
guard would delete 254, and deleting only the dialog would not cover it.

**What this segment does not claim.** It did not re-measure (the ticket says not to), it did not
close a line, and it did not retract a mechanism: the 24 `agent_supervisor.rs` statements that were
already unreachable are still unreachable, and the five new ones are `cfg(not(test))` and
spawn-failure arms — the two classes this record has refused to fabricate fixtures for since D19
(§16.3). If a future review finds one of those five mechanisms weak, the cheapest attack is the
`arm_force_exit` pair: a `cfg(test)` seam that forces `spawn_force_exit_watchdog` to return `Err`
would close 214-215 without touching the product path, and §4.1's `agent_host.rs` 124-126 row is
the precedent for why this record has not done it.
