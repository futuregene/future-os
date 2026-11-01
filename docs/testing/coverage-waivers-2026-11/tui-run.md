# tui · `workbench/run.rs` + `workbench/mod.rs` (rust, llvm-cov) — 2026-11

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov summary percentages are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-tui` (crate `future-tui`), rust **1.99.1** pinned by
`rust-toolchain.toml`. The command, the sanitized environment, and the one lane-only skip are
in `coverage/2026-11/after/tui2/run-cov-tui2.sh` (the same shape as the sibling lane in
`after/tui/run-cov-tui.sh`); `METERS=1` adds the `#[ignore = "meter, not a check"]` pass.

**This file is the ledger for two files — `workbench/run.rs` and `workbench/mod.rs`** — kept
apart from `tui.md` so the two owners' evidence does not overwrite each other. Rows that
belong to `live.rs` are in `tui.md`; rows about `run()`'s process wiring are in `tui.md`'s
"`run()`'s remaining branches" table and are **not repeated here** (they are unchanged: the
same eight branches, the same 19 lines).

**Status of this file: 0 waived.** Everything below is either closed (with the test that
closes it) or *not waived* with the falsifier that would prove it reachable.

## before / after (this task)

| | 2026-10-31 (task start) | r2 (wave 12) | **r4 (this round)** |
|---|---:|---:|---:|
| `workbench/run.rs` DA:0 | 333 | 168 | **142** |
| `workbench/mod.rs` DA:0 | 316 | 287 | **262** |
| lane total DA:0 | — | 1486 | **1458** |

r2 → r4 is this round's work; the run.rs figure also carries the earlier waves, because the
task began at 333. `lcov` for both: `coverage/2026-11/after/tui2/tui2-r4.info`
(`r2`/`r3` kept beside it for the comparison).

## Closed this round — and by what

| was | how it closed |
|---|---|
| `run.rs:523, 536-539` — the rail's `SESSIONS_POLL` arm (`live.begin_sessions_refresh()` behind `frame.layout.sidebar.is_some()`) | **The pty harness, not the in-process one.** The rail is drawn only on a window at least `layout::NARROW_WIDTH` (88) columns wide, and the driver's terminal answers 80 under a redirected stdout — which is every `cargo test` run — so the arm is unreachable from inside the library. The pty is given 120 columns, the reader's settings file opens the rail, and an agent that counts `list_sessions` makes the clock observable. Falsified once at 80 columns (1 read in 20 s, then the assertion fails). Probe: `the_rail_re_reads_its_list_on_its_own_clock` (`tui/tests/driver_run.rs`). |
| `run.rs:385-386` — the step onto a **cached** conversation | a two-step walk whose `get_state` answer moves between the steps (the rail's cursor follows whatever the answer names, so a static answer puts the cursor back and the second step re-opens the first). The conversation left is filed (`is_cached`), and the step back opens it and names it to the agent. Probe: `stepping_away_files_the_conversation_for_the_way_back`. |
| `run.rs:390-391` — the **held** step (`PendingOpen::stepped`, a repeat inside `STEP_TAP_GAP`) | the second step is sent when the *first one's read is on the wire* (a spawned watcher polls the mock's command log), so it is a repeat without a sleep that a loaded machine could push past the gap. The key that follows lands the held step first, which is asserted by where the text ends up: typed before the landing it would be the left conversation's draft, and the box that replaced it would be empty. Probe: `a_held_step_lands_before_the_key_that_follows_it`. |
| `mod.rs:858-860, 1143-1145, 1155-1157, 4071-4073` — `Default`, `hit_at`, `dragging`, `frame_size` | the three frame accessors are what the driver asks of a painted frame and no test asked them: they are now asserted where that frame is composed, and the grab is shown to end with the release. Probes: `a_surface_before_its_first_frame_has_no_frame_to_read`, `a_dragged_divider_lands_under_the_pointer`. |
| `mod.rs:81-115` — `LaunchField::next`/`previous`/`chip_id`/`command` | the ring is walked in both directions (each field is its successor's `previous`, and the ring closes), and each chip's verb is its own id with the palette's `/` in front. Probe: `the_launch_fields_are_a_ring_of_named_chips`. |
| `run.rs:5352, 5440, 5483` — three `assert!` failure arguments **in this file's own tests** | taxonomy **(e)**: a format argument is evaluated only when the assertion fails, so the line reads `DA:0` however often the test passes. The values are bound before the assertion instead. |

Two more loop lines moved from "missed" to "one region": `run.rs:400` and `428` are now single
`}` closing braces with a zero edge (taxonomy **(c)**), not statements — the arms they close
execute (`a_completed_run_is_announced_and_the_activity_row_stops`,
`a_state_refresh_brings_the_parked_approval_back`). They close the way (c) always does: by
single-lining the arm, which is a code change with no behavioural content and is left for a
wave that is not also carrying tests.

## The gate this round was really about: a `PATH` race, not a missing test

`driver_tests::repository_keys` was reported unstable under the full parallel suite. It was
not: `open_git_panel` spawns `git` **by name**, so the product read resolves through `PATH`,
and `live::tests::opening_a_link_spawns_the_opener_and_reports_a_missing_one` swaps that
process-global variable for a directory of opener stubs and then for an empty one. Widening
that window to 800 ms reproduced the failure on every run
(`failed to spawn git: No such file or directory (os error 2)`) and the whole suite is green
with it still widened, once the tests whose *loop* reads the repository hold
`crate::test_env::lock` — the lock every environment mutator takes. Fixtures were never the
problem: they already resolve their git through `test_env::absolute_tool`.

The guard is held across `await` on purpose (`#[allow(clippy::await_holding_lock)]` at each
site): the loop is what spawns the read, nothing the loop runs takes that lock, so the
lint's deadlock cannot occur here while releasing it early reopens the race.

## Remaining in `run.rs` (142) — grouped, none waived

Line numbers are r4; `python3 ../tui2/ranges.py <info> <src> workbench/run.rs` re-prints them
with the source line.

| # | lines | category | the code that would have to be defeated | falsifier | status |
|---|---|---|---|---|---|
| 1 | `118-122, 1485` | **W3** | `load_settings_report`'s notice — the settings file could not be read (`settings_error_notice`) | a *corrupt* `settings.json` in the child's home: reachable from the pty harness, which already hands the child its settings path (`Run::spawn_with`) | **not waived** |
| 2 | `137, 153` | reachable | `--continue` (`live.attach_most_recent()`) and a first message on the command line | `Options { continue_recent: true }` / `Options { initial_prompt: Some(..) }` — the pty child passes only `addr` and `settings` today | **not waived** |
| 3 | `162-166` | **W4/W3** | the startup update check (`tokio::sync::oneshot` + its reader) | `Options { offline: false }`; it reaches the **network**, so it needs an injectable manifest URL before it is worth measuring | **not waived** |
| 4 | `191-192, 200-201` | reachable | the reader thread's resize and signal callbacks in `run()` | a `TIOCSWINSZ` from the pty parent, and a signal to the child (`signal_shutdown.rs` already sends one) | **not waived** |
| 5 | `226, 242-243` | reachable | an explicit theme carried to the next start; `run()` returning `Err` | a theme change before quitting; an agent that refuses the attach | **not waived** |
| 6 | `364-367` | reachable | a conversation step **and another key inside the same burst**: the step lands before the key so the key belongs to the conversation the reader is on | one burst carrying `shift+↓` *and* a plain key (the settle-window case next door only covers the key arriving in a later pass) | **not waived** |
| 7 | `400, 428, 925, 1591, 1620` | **(c)** brace | closing braces whose arm executes but whose zero edge is the `if let` completion | single-line the arm (no behaviour change) | **not waived** |
| 8 | `418, 444, 453-456, 469-479, 499-501` | reachable | the **notification** paths: an `Older` page's paint, a completion's `run_id`, the failure and completion `notify(..)` calls | a driver fixture in which the completion/failure event is the *client's own* run and the queue is empty (`live.take_completion_notification` / `take_failure_notification` gate it). **Measured, not assumed:** `a_completed_run_is_announced_and_the_activity_row_stops` and `a_failed_run_is_announced_as_failed` already send `agent_end` on the run the prompt-ack named, and these lines still read 0 — so the gate returned false on both, and the ack's `run_id` is not by itself enough | **not waived** — next useful check is `adopt_run`/`install_state`'s run map after the prompt ack |
| 9 | `742, 959, 1259, 1348, 1409` | reachable | small `match` tails: `resolve_step`'s non-switch actions, `Outcome::Quit`, a tab key that maps to no section, a filter parse's default, a picker's `Some(next)` | one driver key per arm | **not waived** |
| 10 | `918-919, 1028-1035, 1075-1084, 1088, 1106, 1146, 1153, 1280-1283, 1362-1368, 1772-1773` | **W3 (a)** | the error arms of five panel reads plus `toggle_pin_selected` and its `continue` | a mock that answers `Err` on the read each arm belongs to (the driver's mock answers by command type, so this is one fixture per arm) | **not waived** |
| 11 | `1197-1201, 1208-1213, 1231-1232, 1236-1238, 1243-1249, 1756-1761, 1787, 1793-1802, 1821` | reachable | the mouse-capture and preview verbs: `/preview`, `reset_preview`, `refresh_state`, the clipboard copy's `spawn_blocking`, the picture scope | one driver verb each; `/preview` on a file row needs a workspace with a picture (the loop_edges preview test builds one) | **not waived** |
| 12 | `1374-1376, 1382-1384, 1435-1436, 2141` | reachable | `copy_last` / `copy_message` and the `ImageProtocol::None` arm | a copy key per row kind; a terminal that reports no picture protocol (the driver's does not, so this is one flag away) | **not waived** |
| 13 | `1734-1743` | **W7** | "No driver behind this surface (never in the running app): read it here" — the synchronous fallback the driver never takes because `Live` has a load sender | a `Live` with no `load_tx` (the comment is the claim; it has not been measured) | **not waived** |
| 14 | `2664, 2669, 2697, 2950` | reachable | display helpers (`bubble[0]`, `index + 1`) and `events_of`'s lone-`ESC`-then-`[B` split | one unit case each; `events_of` is a pure function | **not waived** |

## Remaining in `mod.rs` (262) — shape only

Not itemised here yet; the r4 range list groups them in five families, and the first two are
where the cheap lines are:

- **early-return guards** (`return Vec::new()`, `return None`, `_ => {}`): the largest family —
  each is one event the surface refuses, and every one is reachable from a `WbEvent` the test
  builds, so the work is enumerating the refusal table, not defeating a mechanism.
- **overlay/key arms** (`Overlay::DeleteSession => Vec::new()`, the picker's `backspace`,
  preference rows that cycle an index): reachable per key.
- **`panic!` failure arguments in this file's own tests** (`panic!("{actions:?}")` and
  friends): taxonomy **(e)/(c)** — the same hoist this round applied to three of them in
  `run.rs`.
- **mouse and divider arms** (`git.divider`, wheel bursts, `preview_drag`).
- **error arms of panel reads**: the remainder, the same shape as `run.rs` row 10.

The two families with a mechanism rather than a missing key press are the `panic!` arguments
(mechanical hoist) and the git-divider drags (a press on `git.divider` needs the `/git` panel
open, which the driver tests already do).

## How to re-measure this file

```bash
cd coverage/2026-11/after/tui2
TAG=r5 METERS=1 ./run-cov-tui2.sh
python3 missed_run.py tui2-r5.info workbench/run.rs        # counts
python3 ranges.py tui2-r5.info ../../../.worktrees/cov-tui2/tui/src/workbench/run.rs workbench/run.rs
```

`rm -rf target/llvm-cov-target` afterwards: the instrumented build is ~4 GB and nothing but
this lane reads it.
