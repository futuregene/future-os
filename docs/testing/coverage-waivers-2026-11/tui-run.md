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

| | 2026-10-31 (task start) | r2 (wave 12) | r4 | **r8 (wave 5, final)** |
|---|---:|---:|---:|---:|
| `workbench/run.rs` DA:0 | 333 | 168 | 142 | **136** |
| `workbench/mod.rs` DA:0 | 316 | 287 | 262 | **236** |
| lane total DA:0 (with meters) | — | 1486 | — | **1359** |

Each `rN` is one measurement pass; `lcov` for all of them is in
`coverage/2026-11/after/tui2/tui2-rN.info`, and every figure in this file was re-derived from
the one it cites (`missed_run.py` for a count, `ranges.py` for the rows). Wave 5 rebased onto
`origin/dev-huichen` first (16 commits from the sibling lanes), so `r5` — not `r4` — is this
round's baseline: run.rs **138**, mod.rs **262**.

## Wave 5 (2026-11-01): mod.rs −26, run.rs −2/−8 that turned out to be dead producer

| was | how it closed |
|---|---|
| `mod.rs` — eleven `let [WbAction::X(..)] = actions else { panic!(..) }` arms | A `panic!` on a `let … else` arm is a line **no passing test can reach**, so it reads `DA:0` however often the case passes (taxonomy (e)/(c)). One `one(actions, closure)` helper builds its message on the passing path and narrows through a closure whose non-matching arm runs for every action that is not the expected one. Probes: the eleven cases it replaced (`multiline_editing_and_queue_actions_use_shared_state`, the `ctrl+a`/`ctrl+e` pair, the palette's `tab` fill, the five mention completions, `a_paste_is_an_edit_and_never_a_key`). |
| `mod.rs:1265, 1277` and the per-overlay `_ => …Redraw` fallback | `every_overlay_refuses_the_keys_it_does_not_own`: sixteen overlays × four chords nothing on the surface claims (function keys, an unheard-of modifier combination). A refusal is nothing to do **or at most a repaint** — never an edit, a submission, a command or a decision, which is what the table asserts. |
| `mod.rs:1070, 2591-2596, 2607, 2613, 2621, 3151, 3158, 3185, 2563-2564, 2679-2685, 2812-2813` — the layout menu's ten rows, and the navigation keys of every box | `every_preference_row_answers_enter_and_the_menus_refuse_a_sideways_press` walks the surface's own `PREFERENCE_ROWS`: every row answers `enter`, and the four rows that open a menu of their own (`Language`, `Theme`, `Shortcuts`, `Reset`) refuse `left`/`right` instead of flipping through a list one press at a time. `navigation_scrolls_a_box_without_closing_it` presses `up`/`down`/`k`/`j`/`pageup`/`pagedown`/`home`/`end` in each of eleven boxes and asserts the box is still on screen and the answer is never a quit, a submission, a decision or a verb. |
| `run.rs:444, 472-473` — a completion and an alert that arrive behind another event | `a_burst_announces_what_is_in_it` — see the row-8 finding below. |

### What row 8 actually was (and what is left of it)

The ledger's row 8 said the notification lines never ran and named "`adopt_run`/`install_state`'s
run map after the prompt ack" as the next check. It was neither: the driver fixture's prompt ack
was `{"run_id","accepted_state"}`, and `payloads_ext::RunAck` also requires `run_epoch`, so the
ack **failed to decode** — the reader was told `发送未得到确认：missing field \`run_epoch\`` and
no run was ever filed as this client's own. With the field in the fixture, the completion gate
answers `true` (measured: `468-469` and `498-501` execute) and the failure notice is the only
part still at zero — for a different reason:

**`take_completion_notification` removes the run's candidate before it checks the state**
(`live.rs`: `.position(..).and_then(|index| self.completion_candidates.remove(index))` runs
before `should_notify`, whose reply for `state == "failed"` is `false`). Both loop sites call the
completion gate first and the failure gate second, so `take_failure_notification` can never see a
candidate the loop has not already consumed, and `notify(.. NotifyKind::Failed ..)` at
`run.rs:453-456` and `run.rs:476-479` is **unreachable from the loop** — `alert_for` covers
`error` and `approval_request`, not `agent_end` with `state: "failed"`, so a failed run is
announced by nothing at all.

That is a product defect in `live.rs` (the sibling worker's file), not a measurement gap: the fix
is to consume the candidate only when the state check passes — or to try the failure gate first —
and both lines open. **Falsifier for a waiver taken on the wrong grounds:** a `Live`-level case in
which a run is filed, its `agent_end` carries `state: "failed"`, and `take_failure_notification`
returns `true`; that case passes today only if the candidate is never offered to the completion
gate first, which the loop always does.

## Closed in an earlier wave — and by what

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

## Remaining in `run.rs` (136) — grouped, none waived

Line numbers are r8 for the product lines (the test module below them grew, the code above it
did not); `python3 ../tui2/ranges.py <info> <src> workbench/run.rs` re-prints them with the
source line.

| # | lines | category | the code that would have to be defeated | falsifier | status |
|---|---|---|---|---|---|
| 1 | `118-122, 1485` | **W3** | `load_settings_report`'s notice — the settings file could not be read (`settings_error_notice`) | a *corrupt* `settings.json` in the child's home: reachable from the pty harness, which already hands the child its settings path (`Run::spawn_with`) | **not waived** |
| 2 | `137, 153` | reachable | `--continue` (`live.attach_most_recent()`) and a first message on the command line | `Options { continue_recent: true }` / `Options { initial_prompt: Some(..) }` — the pty child passes only `addr` and `settings` today | **not waived** |
| 3 | `162-166` | **W4/W3** | the startup update check (`tokio::sync::oneshot` + its reader) | `Options { offline: false }`; it reaches the **network**, so it needs an injectable manifest URL before it is worth measuring | **not waived** |
| 4 | `191-192, 200-201` | reachable | the reader thread's resize and signal callbacks in `run()` | a `TIOCSWINSZ` from the pty parent, and a signal to the child (`signal_shutdown.rs` already sends one) | **not waived** |
| 5 | `226, 242-243` | reachable | an explicit theme carried to the next start; `run()` returning `Err` | a theme change before quitting; an agent that refuses the attach | **not waived** |
| 6 | `364-367` | reachable | a conversation step **and another key inside the same burst**: the step lands before the key so the key belongs to the conversation the reader is on | one burst carrying `shift+↓` *and* a plain key (the settle-window case next door only covers the key arriving in a later pass) | **not waived** |
| 7 | `400, 428, 925, 1591, 1620` | **(c)** brace | closing braces whose arm executes but whose zero edge is the `if let` completion | single-line the arm (no behaviour change) | **not waived** |
| 8 | `453-456, 476-479` | **W8** (dead producer) | the failure notice in both call sites: `take_completion_notification` **removes** the run's candidate before it checks the state (`live.rs`: `.position(..).and_then(|i| self.completion_candidates.remove(i))` runs before `should_notify(.., state, ..)`), and both loop sites call the completion gate first — so `take_failure_notification` never sees a candidate, and `alert_for` covers `error`/`approval_request` but not `agent_end state:"failed"`. A failed run is announced by nothing | a `Live`-level case where a filed run's `agent_end` says `failed` and `take_failure_notification` answers `true` — which today holds only if that candidate is never offered to the completion gate first | **not waived** — product defect in `live.rs` (see "What row 8 actually was"); the fix opens all eight lines |
| 8a | `469` | reachable | the burst drain's completion (`completion = event.run_id…`) | an `agent_end` with `state:"completed"` that arrives **behind** another event in the same breath (the single-event arm at `444` is now covered) | **not waived** |
| 9 | `742, 959, 1259, 1348, 1409` | reachable | small `match` tails: `resolve_step`'s non-switch actions, `Outcome::Quit`, a tab key that maps to no section, a filter parse's default, a picker's `Some(next)` | one driver key per arm | **not waived** |
| 10 | `918-919, 1028-1035, 1075-1084, 1088, 1106, 1146, 1153, 1280-1283, 1362-1368, 1772-1773` | **W3 (a)** | the error arms of five panel reads plus `toggle_pin_selected` and its `continue` | a mock that answers `Err` on the read each arm belongs to (the driver's mock answers by command type, so this is one fixture per arm) | **not waived** |
| 11 | `1197-1201, 1208-1213, 1231-1232, 1236-1238, 1243-1249, 1756-1761, 1787, 1793-1802, 1821` | reachable | the mouse-capture and preview verbs: `/preview`, `reset_preview`, `refresh_state`, the clipboard copy's `spawn_blocking`, the picture scope | one driver verb each; `/preview` on a file row needs a workspace with a picture (the loop_edges preview test builds one) | **not waived** |
| 12 | `1374-1376, 1382-1384, 1435-1436, 2141` | reachable | `copy_last` / `copy_message` and the `ImageProtocol::None` arm | a copy key per row kind; a terminal that reports no picture protocol (the driver's does not, so this is one flag away) | **not waived** |
| 13 | `1734-1743` | **W7** | "No driver behind this surface (never in the running app): read it here" — the synchronous fallback the driver never takes because `Live` has a load sender | a `Live` with no `load_tx` (the comment is the claim; it has not been measured) | **not waived** |
| 14 | `2664, 2669, 2697, 2950` | reachable | display helpers (`bubble[0]`, `index + 1`) and `events_of`'s lone-`ESC`-then-`[B` split | one unit case each; `events_of` is a pure function | **not waived** |

## Remaining in `mod.rs` (236) — shape only

Not itemised line by line; the r8 range list groups them in five families, and wave 5 removed
the first one's easiest half:

- **early-return guards** (`return Vec::new()`, `return None`, `_ => {}`): the largest family,
  and the one the refusal tables now walk for the *generic* case (a stray chord, the three
  non-key events, the layout menu's rows, the navigation keys of every box). What is left is
  the guards that need a particular **state** rather than a particular key — a step at the end
  of the rail, a panel row that is a heading, a picker with no rows — each reachable, none
  defeated by a mechanism.
- **overlay/key arms** (`Overlay::DeleteSession => Vec::new()`, the picker's `backspace`, the
  keybinding editor's indices): reachable per key, in a state the box only reaches after
  another action.
- **`panic!`/`expect` failure arguments in this file's own tests**: what `one` did not cover —
  a `match` arm or a `let … else` whose message is a separate argument.
- **mouse and divider arms** (`git.divider`, wheel bursts, `preview_drag`).
- **error arms of panel reads**: the remainder, the same shape as `run.rs` row 10.

## The pty rail check costs one poll (5 s); it stays, with the cost written down

`the_rail_re_reads_its_list_on_its_own_clock` (`tui/tests/driver_run.rs`) is the **only** way to
cover `run.rs:523, 536-539`: the rail is drawn only on a window at least
`layout::NARROW_WIDTH` (88) columns wide, and the in-process driver's terminal answers 80 under a
redirected stdout. The alternative was a test seam in `terminal.rs` (a forced size per terminal),
i.e. product machinery for one arm.

**Measured, with timestamps printed from the test itself** (2026-11-01, this host):

| segment | time | whose |
|---|---:|---|
| child spawn → first frame | **5.1 s** | the fixture, not the check: it is the same for this harness with the window size and the seeded settings both removed, and the sibling `an_approval_streamed_to_a_real_terminal_rings_the_desktop` paints in 0.3 s because its agent streams an event at once — an idle stream (`pending()`) is what costs the 5 s. Unattributed beyond that. |
| first frame → the re-read | **5.0 s** | **the check**: one `SESSIONS_POLL`, which is the product's own constant |
| ctrl+x → the child is gone | **0.06 s** | the fixture |
| libtest total for the target (4 cases, parallel) | **15.2 s** | was 5.4 s for the same target before this case, measured on the real-entry case alone |

So the case adds **5 s**, and the target went 5.4 s → 15.2 s: 5 s of that is the check, the rest is
this fixture's startup/teardown overlapping differently under parallel scheduling. Worth it while
the four lines have no other route; if that ever changes, the two levers are (a) a
`FUTURE_TUI_*` poll override the child reads — a product seam, not a shorter wait — and (b)
folding the check into the real-entry case, which already holds a child (at the cost of coupling
"the exit path writes settings.json" with "the rail re-reads its list").

The 5 s is not a sleep the test chose: the assertion is that the read happens on the loop's own
clock, the clock is `std::time::Instant` inside the child (so `tokio`'s paused clock cannot
shorten it), and any shorter wait would prove something else. It is `#[cfg(unix)]` at the
harness's level, so Windows pays nothing.

## How to re-measure this file

```bash
cd coverage/2026-11/after/tui2
TAG=r5 METERS=1 ./run-cov-tui2.sh
python3 missed_run.py tui2-r5.info workbench/run.rs        # counts
python3 ranges.py tui2-r5.info ../../../.worktrees/cov-tui2/tui/src/workbench/run.rs workbench/run.rs
```

`rm -rf target/llvm-cov-target` afterwards: the instrumented build is ~4 GB and nothing but
this lane reads it.
