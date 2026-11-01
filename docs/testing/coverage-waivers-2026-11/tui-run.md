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

**Status of this file: 7 lines waived in wave 6** (see "Waived this wave" — 9 when it was written,
3 retracted in wave 10 because they are measured), **10 as W7 in wave 7** (see "W7:
`open_git_panel`'s …"), **3 as W7 in wave 8** (see "W7 in wave 8"), **1 as W7 in wave 9**
(`mod.rs:3594`, the drag's zero-width edge), **7 in wave 10** (wave 6's "closing braces", off by
one line each and re-classified as arm bodies — 8 until wave 11 moved `run.rs:1288` out, see
"OPEN") and **3 in wave 11** (`mod.rs:3138` and the press chain's total fallthrough
`mod.rs:3804-3805`) — **31 lines**, each with its mechanism, positive control and falsifier, and
`waived_still_dark.py` asserts all of them are still `DA:0` in a named pass. `mod.rs:1563` is
**not waived**: it is a line-attribution artefact and its section says what would falsify that.
Everything else below is either closed (with the test that closes it) or *not waived* with the
falsifier that would prove it reachable. **One row is neither**: `run.rs:1288`
(`WbAction::OpenApproval`, which has no producer) is **OPEN as a product decision**, not waived —
see "OPEN" below.

The rows that remain at zero in `mod.rs` — **42** at `r26`/`r28` — and in `run.rs` — **94** — are
this ledger's open list. They are described **by shape**, not one row at a time: the "Remaining in"
sections were written when the counts were 236 and 95, so read their headings as the shape they
name, and take every count from the table above.

## before / after (this task)

| | 2026-10-31 (task start) | r2 (wave 12) | r4 | r8 (wave 5) | r9 | r10 (wave 6) | r12 (wave 7) | r19 (wave 8, one commit stale) | r20 | r21 (wave 9) | **r22 (wave 10)** | **r23 (wave 10)** | **r25 (wave 11, rebased)** | **r28 (wave 11)** |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `workbench/run.rs` DA:0 | 333 | 168 | 142 | 136 | 116 | 113 | 95 | 94 | 94 | 94 | **94** | **94** | 94 | **94** |
| `workbench/mod.rs` DA:0 | 316 | 287 | 262 | 236 | 221 | 221 | 221 | 97 | 57 | 55 | **55** | **55** | 55 | **42** |
| `workbench/screen.rs` DA:0 | — | — | — | — | — | — | — | — | — | 85 | 87 | **87** | 87 | **84** |
| lane total DA:0 | — | 1486¹ | — | 1359¹ | 1321¹ | 1316¹ | 1298¹ | 1336² | 1298² | 1293² | **1297²** | **1295²** | 1300² | **1279²** |

¹ `METERS=1` passes. ² `METERS=0`. The two are **not** the same lane: a `METERS=0` pass drops
the 151 lines inside the `#[ignore = "meter, not a check"]` tests, so `1336` here is higher than
`1298` there while both files it fronts are lower. Compare within a column shape only — see
"Wave 8" for the evidence and for the `r16`/`r17` reproducibility pair, and "Wave 9" for why the
`97` above reads 57 on the same tree one commit later.

Each `rN` is one measurement pass; `lcov` for all of them is in
`coverage/2026-11/after/tui2/tui2-rN.info`, and every figure in this file was re-derived from
the one it cites (`missed_run.py` for a count, `missed_exact.py` for the rows — `ranges.py`
groups anything within three lines and can print covered lines inside a range). Wave 5 rebased
onto `origin/dev-huichen` first (16 commits from the sibling lanes), so `r5` — not `r4` — is
that round's baseline: run.rs **138**, mod.rs **262**.

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

## Remaining in `run.rs` (95 at r12; 136 was why these rows were written)

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
| 10 | `918-919, 1028-1035, 1075-1084, 1088, 1106, 1146, 1153, 1280-1283, 1362-1368, 1772-1773` | **W3 (a)** | the error arms of five panel reads plus `toggle_pin_selected` and its `continue` | a mock that answers `Err` on the read each arm belongs to (the driver's mock answers by command type, so this is one fixture per arm) | **closed in wave 6 except one tail** — `1028-1034`, `1075-1085`, `1146`, `1362-1364` have their cases; the `Err(join)` arms read `≥1` because a single-expression arm carries the `matches!`/`match` decision's count (see "the metric's own blind spot"), so only the multi-line one, `1366-1369`, is left and it is waived |
| 11 | `1197-1201, 1208-1213, 1231-1232, 1236-1238, 1243-1249, 1756-1761, 1787, 1793-1802, 1821` | reachable | the mouse-capture and preview verbs: `/preview`, `reset_preview`, `refresh_state`, the clipboard copy's `spawn_blocking`, the picture scope | one driver verb each; `/preview` on a file row needs a workspace with a picture (the loop_edges preview test builds one) | **not waived** |
| 12 | `1374-1376, 1382-1384, 1435-1436, 2141` | reachable | `copy_last` / `copy_message` and the `ImageProtocol::None` arm | a copy key per row kind; a terminal that reports no picture protocol (the driver's does not, so this is one flag away) | **not waived** |
| 13 | `1734-1743` | **W7** | "No driver behind this surface (never in the running app): read it here" — the synchronous fallback the driver never takes because `Live` has a load sender | a `Live` with no `load_tx`, `open_git_panel` called directly (the comment *is* the mechanism, now measured at `live.rs:2694`/`2483` and registered in "W7: `open_git_panel`'s …") | **waived in wave 7 (W7)** |
| 14 | `2664, 2669, 2697, 2950` | reachable | display helpers (`bubble[0]`, `index + 1`) and `events_of`'s lone-`ESC`-then-`[B` split | one unit case each; `events_of` is a pure function | **not waived** |

## Remaining in `mod.rs` (**42** at r26/r28; 55 at r22/r23; 236 when this section was written) — shape only

**Wave 10 re-derived the list by function** (`clusters.py` in the lane dir), and **wave 11 ate the
three biggest of them** (see "Wave 11"): 55 lines in 27 functions, of which six held 29. What is
left, by function, at `r26`:

| lines | function | the cluster |
|---:|---|---|
| 4 | `handle_overlay_key` | `2496`, `2509`, `2801`, `2838` |
| 4 | `handle_surface_mouse` | `3541` and `3594` (both waived), `3804-3805` (**waived in wave 11**) |
| 4 | `click` | `3872`, `4046`, `4054-4055` |
| 3 | `selected_stop` | `1934-1935`, `1939` |
| 3 | `handle_mouse` | `3413` (waived), `3476`, `3490` |
| 2 | `handle_key` | `1473` (waived), `1563` (artefact) |
| 1 | `handle_workflow_key` | `3138` (**waived in wave 11**) |

Wave 11 closed **7 of the 8** lines in `handle_workflow_key`, **all 4** of `step_stop` and **2 of
the 6** in `handle_surface_mouse` (`3718`, `3752`); the other 4 across those two functions are the
three waivers below plus wave 8's two.

Seven of the lines that were left at r22 sit inside functions whose names read like this file's
own test fixtures (`row_of`, `plain(&rows)`, a `println!` dump, an `unreachable!()` arm). That is a
lead, not a conclusion: the attribution is "nearest preceding `fn`", so each needs the same first
question — is that helper called at all, and from where — before a case is written against it.

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

## Wave 6 (2026-11-01): the doors that refuse a read

Two passes this wave: `r9` (taken from the working tree while both owners of this ledger were
writing into it, so it carries both halves) and `r10` (after the wave's last test commit,
`f3c2759fc`, which also added one line above the approval gate — from `918` on, `r10`'s numbers
are each one line later than `r9`'s, and every figure below is `r10`'s).

| | r8 | r9 | **r10** |
|---|---:|---:|---:|
| `workbench/run.rs` DA:0 | 136 | 116 | **113** |
| `workbench/mod.rs` DA:0 | 236 | 221 | **221** |
| lane total DA:0 (with meters) | 1359 | 1321 | **1316** |

Eighteen of the twenty `run.rs` lines `r9` closed are the four cases below (`python3 exact.py
tui2-r8.info tui2-r9.info workbench/run.rs`); `run.rs:1280` (the review's `SelectDiff` re-read)
and `:1772` (the panel's row detail) closed in the sibling half. Nothing opened in either pass,
and `r11` — the same lane re-run on the committed tree — repeats `113`/`221` (lane `1317`), so the
wave's number is reproducible rather than one lucky pass.

| arm | the read that fails | probe |
|---|---|---|
| `run.rs:1028-1034` | `/worktree`'s `probe_repo` — the **tool** is missing, not the repository: `git` is spawned by name, so a `PATH` with no `git` on it is the `Ok(Err(..))` arm | `a_repository_read_that_failed_reaches_its_panel` |
| `run.rs:1146` | the working-tree review's `diff_review::load` — the workspace the panel was read from is gone by the time `d` asks for it | `a_review_of_a_workspace_that_went_away_says_so` |
| `run.rs:1362-1364` | the clipboard refusing the repository's `y` — a `PATH` with a **stub `git`** and no clipboard tool, so the panel loads and only the copy cannot run | `a_copy_the_clipboard_refuses_is_reported` |
| `run.rs:1075-1085` | the palette's `/pin` row, which is the **only** door to the loop's `/pin` arm: a typed `/pin` is answered in the surface (`toggle_pin_selected`, `mod.rs:4346`), so `driver.command("/pin")` never reaches `run.rs:1074` | `the_palettes_pin_reaches_the_rail_the_draft_answers_in_the_surface` |

### The ±1 in the lane total, and where it came from

`r10` and `r11` disagree by one line, and it is not the tree: `cmp_info.py` puts the whole
difference on `git_panel.rs` (47 vs 48), and `diff_lines.py` names the line —
**`git_panel.rs:1077`**, `focus`'s `Err(error) => view.detail = vec![spoken(lang, error)]`.

That arm runs when a cursor move re-reads one row's patch and the read fails. It was covered
only **when a read happened to break under the whole suite**, and this file's own fixtures made
that likelier than it sounds: they spawned `git` **by name** (`Command::new("git")`, three
call sites in `git_panel`'s tests), so a peer test's `PATH` swap — the one
`live::tests::opening_a_link_spawns_the_opener_and_reports_a_missing_one` holds the environment
lock for — could turn a read they expected to succeed into the failure arm. A measurement that
depends on a race is not a measurement; it is what made any single-pass lane total ±1.

Closed from both ends, in `1ab7bb41f`:

* the fixtures resolve `git` absolutely (`test_env::absolute_tool`, same fix as `run.rs`'s
  fixture in wave 5), so the read a test means to make is the read it makes;
* `a_detail_that_cannot_be_read_is_one_spoken_line` states the arm's own premise — a workspace
  that is gone, a row selected, and the gutter holding exactly the one line that says so.

Verified in isolation rather than by a whole-lane pass: `cargo llvm-cov -p future-tui --lib --
git_panel::tests::a_detail_that_cannot_be_read` (own `CARGO_TARGET_DIR`, so a sibling run cannot
rebuild the shared one mid-pass) reports `DA:1` on `1077` and `DA:0` on `1076` (`Ok(lines)`) —
i.e. the test took the failure path it is named for. **`r12`, the whole-lane confirmation, did
not produce a usable artifact:** its meters pass died with `signal_shutdown … (never executed)`
because the second `--agent-id cov-tui2` run rebuilt the shared `target/llvm-cov-target`
underneath it. The next lane pass on a quiet worktree settles the last digit; the line itself
is no longer in doubt.

### What the palette's `/pin` showed: a sentence nothing prints

`toggle_pin_selected` answers a launchpad pin with `WbAction::Notice(NoticePinNewSession)`
(`mod.rs:2156-2159`). The **typed** door reaches the loop's own `Notice` arm (`run.rs:1390`,
`live.notice = Some(..)`), but the **palette's** door hands the action to `Live::apply`, whose
`WbAction::Notice` arm is `{}` (`live.rs:1979`) — the reader who pins from the palette with
nothing on the rail is told nothing at all. Measured, not inferred: the case asserts the
invariant that survives a fix (`pinnedSessions` is not written) and records the gap here instead
of asserting the silence as though it were correct.

### The metric's own blind spot on `match` and `matches!` arms — read this before waiving one

`DA:≥1` on an arm is **not** proof the arm ran, and it cost this wave one wrong reading before it
was caught. Three measurements from `r10`:

- `run.rs:1373-1374` (`if matches!(action, WbAction::CopyLast)` and the `let` under it) read
  **258** — the decision's own count — while `1375-1377` (the body's remaining lines) read **0**.
  The arm is never entered (see the waiver below): the `matches!` expansion's spans cover the
  condition and the first statement, so a one-statement arm reads as reached forever.
- `run.rs:959` (`Outcome::Quit => Ok(true)`) reads **1** and `960` (`Outcome::Continue =>
  Ok(false)`, the arm every `/new` takes — `/new` is `to_launchpad`, `live.rs:2194-2200`) reads
  **0**. The two arms trade their counts.
- the single-expression `Err` arms — `1146`, `1106`, `1153`, `1281`, `1773` — all read **≥1**
  although nothing in the suite produces a `JoinError` or a `git` failure at those sites.

The consequence for this ledger: **a waiver is taken on the *tail* of an unreached arm, never on
its first line**, and the falsifier is always "a case that makes the tail run". Lines the metric
cannot see at all (`-`: no `DA` record) are not counted by metric A either way.

### Waived this wave (9 lines)

| lines | why no test can reach it | falsifier |
|---|---|---|
| `1366-1368` | the `Err` side of the git copy's `spawn_blocking(..).await` (`clipboard::copy_text`): a `JoinError` needs the blocking closure to **panic** — `copy_text` returns `Result` and does not panic on any input the panel passes. Its three render lines stay at `0` because the arm is multi-line (the blind spot above hides only single-statement arms); `1369` (the call's closing `),`) is measured, so the range stops at `1368` — wave 10 retracted the last line of it | any input that makes `copy_text` panic (then this is a product defect, not a missing test) |
| `1374-1376` | `WbAction::CopyLast` has one producer — the keymap's `Action::Copy` (`mod.rs:1550`) — and the loop answers that action itself before `handle` runs (`run.rs:936-941`), so a `Copy` binding never reaches the arm and a reader who rebinds the chord lands in the intercept for the same reason. `1377`, the `if`'s closing brace, is measured; wave 10 retracted it from the range | a `CopyLast` produced by something other than the keymap's `Action::Copy` (a mouse or paste path would do it) |
| `959` | a `match` over `Outcome` whose arms are constant expressions: the decision's span lands on `959`, and `/new` returns `Continue` (`live.rs:2194-2200`), so the `Quit` arm never runs; `Quit` comes only from `WbAction::Quit` (`live.rs:1880`) or the `quit` command (`live.rs:2189`). `960` (`Outcome::Continue`) **is** measured — wave 10 retracted it from this row, and with it the claim that "`960` reads 0" | an `Outcome` from `Live::apply` that is neither of the two arms |

Eight more lines of this wave were first classified as **not** waived — `1036`, `1084`, `1107`,
`1154`, `1282`, `1284`, `1349`, `1774`, "`}` / `};` closing braces with a zero edge (taxonomy
**(c)**, like `400`/`428`/`926` above)". **Wave 10 measured that list and it is off by one on
every row**: the zeros are one line *above* each of those braces, and they are arm bodies, not
braces:

| zero line (measured, r22) | what it is | mechanism | positive control | falsifier |
|---|---|---|---|---|
| `1035` | `Err(error) => live.notice = Some(error.to_string()),` | the `JoinError` side of `spawn_blocking(probe_repo).await` (`1018-1025`): the closure calls a `Result`-returning function and cannot unwind, so only a runtime shutdown produces this arm | `1028-1034`, the `Ok(Err(..))` arm — the failure that *can* happen — is measured | any closure that panics |
| `1106` | `Err(error) => live.notice = Some(error.to_string()),` | the same shape around `create_worktree` (`1093-1103`) | the `Ok(..)` arm `1105` is measured | ditto |
| `1153` | `Err(error) => live.notice = Some(error.to_string()),` | the same shape around the `/diff` read (`1134-1150`); its `Ok(Err(..))` sibling is `1146` | `1146` measured | ditto |
| `1281` | `Err(error) => live.notice = Some(error.to_string()),` | the same shape around the review read at `1269-1272`; the inner `Ok(Err(..))` is `1280` | `1280` measured | ditto |
| `1773` | `Err(error) => git.detail = vec![error.to_string()],` | the same shape around `git_panel::detail` (`1765-1768`) | `1772`, the parse failure, is measured | ditto |
| `1083` | `return Ok(true);` under `if matches!(live.apply(other).await, Outcome::Quit)` | `toggle_pin_selected` (`mod.rs:2155-2170`) returns only `Notice(..)` or `SetSessionPinned{..}`, so the `other =>` arm at `1081` exists for exhaustiveness and `apply` never sees a `Quit` | both real outcomes are measured: the launchpad notice and the pin applied | a third variant returned by `toggle_pin_selected` |
| `1348` | `_ => (GitSection::Commits, None, 0, true),` | the guard at `1309-1316` admits exactly five variants; `SelectGit` continues at `1319`, the other four are matched at `1323`/`1332`/`1338`/`1339`, so no action reaches `_` | the panel opens through each of the four (`OpenGit`, `GitSection`, `GitFilter`, `GitRefresh`) and `SelectGit` through `open_git_detail` | the guard widening to a sixth variant |

`1288` was the eighth row of this table and **wave 11 took it out**: it is not a waiver, because
nothing about the line says "unreachable by structure" — the variant it tests simply has no
producer, which is a product question. It is now the OPEN row below.

## OPEN — `WbAction::OpenApproval` has no producer (`run.rs:1287-1289`)

Not a waiver. A waiver says "no test can reach this and here is why that is right"; this says
"the code has no door and somebody has to decide which door it should have".

**The measurement.** `grep -rn "WbAction::OpenApproval" tui/src` returns exactly three sites: the
enum variant (`mod.rs:278`), the `{}` listing in `Live::apply` (`live.rs:1892`, the arm that says
"this layer does nothing with it"), and the branch this row is about (`run.rs:1287`). Nothing
constructs it, so `run.rs:1288` reads `DA:0` in **every** pass — that is a fact about the tree,
not about the suite.

**What the card's real doors are** — five, and none of them goes through the variant:

| door | where | why it is not `OpenApproval` |
|---|---|---|
| a `get_state` read landing while the session was busy | `run.rs:423-427` | the driver has the approval in hand (`live.approval()`) and sets the overlay itself |
| an event arriving with a pending approval | `run.rs:459-462` | same |
| `enter` with an approval parked and no overlay | `run.rs:912-925` | the card opens *or* the key answers it (`y`/`n` are handled just above); the decision belongs to the loop, not the surface |
| the `/approval` command, typed | `run.rs:982-988` | it must `refresh_state().await` first, and the surface owns no I/O |
| the `/approval` row in the palette | same arm, reached as `WbAction::Command("/approval")` | **this is what replaced the variant**: a command the surface can spell is a `Command`, and the palette row already is one |

**Two options, and what each buys.**

| | (A) delete the variant | (B) give it a producer |
|---|---|---|
| the change | drop `WbAction::OpenApproval` (`mod.rs:278`), its `{}` listing (`live.rs:1892`) and the branch (`run.rs:1287-1289`) | wire something that paints outside the keyboard: a "待确认" chip in the status row that answers `OpenApproval`, so the card has a pointer door |
| what it buys | the enum's implied contract — *every action has a producer* — becomes checkable, and `run.rs:1288` leaves the metric by ceasing to exist | the pointer gains a way into the card; the line becomes a measurement |
| the cost | a future contributor who wants a surface-side door must add the variant back | a new UI affordance, and one the keyboard already covers (`enter`, `run.rs:916-925`), so the chip is the only non-duplicate shape it could take |
| the risk | none observable: no producer means no behaviour to lose | a chip nobody asked for; worse, `apply`'s `{}` arm invites a second, drifting copy of the opening logic |

**Recommendation: (A)**, on the grounds that the *variant already lost* to `Command("/approval")`
— the surface's own spelling of a verb that needs a driver-side read is a command, which is
exactly the rule the neighbouring `/fork`, `/git` and `/queue` rows follow. (B) is only worth it
if a visible pending-approval chip is a product goal; it is not one this ledger can assert.

**Falsifier.** Anyone constructing `WbAction::OpenApproval` — a click handler, a key table entry,
a palette row that returns the variant instead of a `Command`. Then `run.rs:1288` lights up (so
the row leaves this section on a measurement, not on an argument) and the product question becomes
"should this door exist", with the answer visible on screen. Until then, **no test may be written
against `1288`**: a case that fabricates the action to reach it would be measuring a state the
program cannot produce.


### Row 8 (W8) re-checked, and it stands

`live.rs` is unchanged this wave: `take_completion_notification` still removes the run's
candidate (`.position(..).and_then(|index| self.completion_candidates.remove(index))`,
`live.rs:1607-1611`) **before** `should_notify`, and both loop sites call it before
`take_failure_notification`, so `run.rs:453-456` and `:476-479` stay dark and the waiver's
falsifier is unchanged. A failed run is still announced by nothing.

## Wave 7 (2026-11-01, r12): the copy's two sentences, and the two preview doors

`run.rs` **113 → 95** (−18, nothing opened: `python3 exact.py tui2-r11.info tui2-r12.info
workbench/run.rs`), `mod.rs` unchanged at 221, lane `1317 → 1298`. The closed set is
`1208-1213`, `1231-1232`, `1236-1238`, `1787`, `1793-1797`, `1821` — and `r13`, the same lane on
the same tree, repeats `95`/`221` (`1298`), so the number is reproducible rather than one lucky
pass.

| lines | what runs | case |
|---|---|---|
| `1787`, `1793-1797` | `/copy`'s answer actually reaching `copy_to_clipboard`: the success sentence when a tool took it, the refusal naming the tool when none could start | `a_copy_of_the_answer_says_which_way_the_clipboard_went` |
| `1208-1213` | the bindings table's `r`: a save that fails carries the settings writer's own sentence | `a_keybinding_that_cannot_be_saved_is_reported` |
| `1236-1238` | `PreviewAttachment`: the picture a reader pasted into the draft opens the box on the conversation | `an_attachment_preview_opens_the_box_on_the_draft_picture` |
| `1231-1232`, `1821` | `PreviewChatFile`: `tab` walks the answer's links, `enter` on the file one opens the box over the conversation | `a_file_the_answer_links_opens_the_box_over_the_conversation` |

**What the copy arm had really been missing.** `last_assistant_text` reads the transcript's
messages, and the fixture's road was a streamed `text_chunk` — which `append_to_last_message`
appends to an assistant message and **drops when there is none** (`chat_area.rs:1683-1696`, measured:
the arm read `Err` on all eight previous calls). The answer has to come from the agent's own
entries, so the fixture grew `Driver::attach_conversation_holding(cwd, entries)`; a test that wants
messages now says which. `/copy`'s success branch needed nothing else.

## W7: `open_git_panel`'s "no driver behind this surface" fallback (`run.rs:1734-1743`)

**Mechanism, as the code states it.** The panel's read is the driver's
(`live.begin_git_read`, `live.rs:2687-2705`), and that answers `false` on exactly one condition —
`self.load_tx` is `None` (`live.rs:2694`). `load_tx` is `Some` only after `Live::set_load_sender`
(`live.rs:2483`), which `run()` calls before the loop starts and `Driver::attach_at` calls for the
tests. So every surface that can open the panel in the running app has a driver behind it, and the
synchronous read at `1734-1743` is the fallback for a `Live` built without one.

**Positive control.** Everything around the fallback is measured many times over: `open_git_panel`
is entered 58 times by the repository cases (the box opens, its section and filter land, the
cursor's row is re-read in `moving_in_the_repository_panel_reads_the_row_under_the_cursor`, the
worktree row re-reads the panel in `entering_a_worktree_moves_the_session_to_it`), and the failures
of the *asynchronous* read are the wave-6 cases. What is unmeasured is only the branch where the
read cannot be dispatched at all.

**Why not a test.** Reaching it means building a `Live` without a load sender (the `live::tests`
shape), calling `open_git_panel` directly and asserting a frame — a configuration `run()` cannot
produce. It would pin the fallback's existence rather than any behaviour a reader can reach, which
is the definition of a W7 line in this ledger.

**Falsifier.** Any path to `open_git_panel` that a reader can take without `Live::set_load_sender`
having run — a second entry point into the panel, or a driver that stops handing one over — makes
this a live branch and the waiver void.

## Wave 8 (2026-11-01, r19): the pointer — the whole second half of this file

`workbench/mod.rs` **97** at `r19`, down from **176** at `r13b`/`r14` (**−79**); `run.rs` unchanged
at **94** (this wave touched no run.rs arm; wave 7 had already closed `1208-1213`, `1231-1232`,
`1236-1238`, `1787`, `1793-1797`, `1821`). The lane total moved `1360` (`r16`, the first pass of
this wave) → `1339` (`r18`) → **`1336`** (`r19`).

**The lane number changed meaning in this wave, and that is a measurement finding, not a
regression.** Every `.info` before `r16` (`r10`…`r14`, and `r13b` — byte-identical to `r15`)
carries the `#[ignore = "meter, not a check"]` bodies, because those passes ran with `METERS=1`
and the merged profile kept them; `r13b` and `r15` are in fact the *same* profile printed twice.
A `METERS=0` pass in a freshly cleaned target dir (`r16`) therefore reads `live.rs` at **337**
rather than 188 — the 151 lines of `session_rail_cost_meter` and its neighbours, which is what a
tests-only lane should read. `r16`…`r19` are all `METERS=0` and comparable with each other;
`r14`'s `1250` is not comparable with `r19`'s `1336`. Reproducibility: `r16` and `r17` are the
same tree two passes apart — `1360` vs `1358` lane, `111` vs `111` on `mod.rs` — so `mod.rs` is
stable to the line and the lane carries a ±2 that lives in `live.rs`, not here. **Wave 10 named
that arm**: it is `refresh_worktrees`'s failure edge, which errs only while a peer test has moved
the process cwd (see "Wave 10", where `r22`/`r23`/`r24` hold `mod.rs` at 55 and `run.rs` at 94 and
the `screen.rs` ±2 is still open).

| lines | what runs | case |
|---|---|---|
| `3818-3825`, `3848`, `3852` | a chooser by pointer: the wheel walks it a row a turn (and previews the theme it lands on), a press on the box's own frame is absorbed, and motion is not a gesture | `the_pointer_in_a_chooser_walks_the_list_and_its_frame_absorbs_a_press` |
| `3385-3387`, `3389`, `3392-3393` | a thinking row applied where a session is (`SetThinking`) and staged where it is not (`SetPendingThinking`); the approval row's one setter | `a_thinking_row_applies_where_a_session_is_and_is_staged_where_it_is_not` |
| `3787-3788`, `3802-3805`, `3804-3805` | a click on a preferences row moves the cursor there; a workflow row takes the row *and* applies it (`enter`) | `a_click_on_a_list_row_takes_the_row_the_way_enter_does` |
| `3558-3570` | the wheel over a box is that box's key: the patch scrolls while it holds the keys, the file list walks when it does, and every other box reads one row a turn | `the_wheel_over_a_box_answers_the_way_its_keys_do` |
| `3185` | `pageUp` on a detail view comes back by a page | same case |
| `1479-1481`, `1505`, `1542-1544`, `1550`, `1552` | the five verbs that ship with `""` (help, language, delete, copy, mouse) reached by a chord a reader binds, plus the delete that has nothing to name and answers with the notice | `a_chord_a_reader_binds_reaches_the_verbs_that_ship_without_one` |
| `1421`, `1437`, `1445`, `3872-3876`, `3879`, `3882-3884` | the mention menu hands back the keys and the clicks it does not own: `↑` at the top stays, a letter still edits the draft, `tab` with nothing matching inserts nothing, the box's frame absorbs, and a click past it closes the menu and still lands | `the_mention_menu_gives_back_every_key_and_every_click_it_does_not_own`, `a_click_past_the_mention_menu_closes_it_and_still_lands` |
| `3507-3510`, `3601-3602`, `3604`, `3623`, `3594` | a pointer with nothing held: a release with no grab, motion during neither, and a drag that lands back on the cell it took hold of (all four answer with nothing) | `a_pointer_with_nothing_held_moves_nothing` |
| `4037-4039`, `4042-4044`, `4052-4055` | the rail's rows by pointer: a workspace heading folds and remembers it, a section caption takes the keys and stops, a launchpad chip runs the verb its id names | `a_click_on_the_rail_takes_the_row_it_landed_on`, `a_click_on_a_launchpad_chip_runs_the_verb_it_names` |
| `3650-3651` | the patch re-states that it holds the keys (its hit region exists only on a frame narrow enough to show one pane, and only while the patch *is* that pane) | `a_click_on_the_patch_keeps_the_keys_on_it` |
| `3661-3662`, `3665-3666` | a preference row that is a menu: the click moves the cursor to the row and opens it, so `esc` lands back on `/preferences` at that row | `a_click_on_a_preference_row_opens_the_menu_it_names` |

`screen.rs` also fell **93 → 86** across the same passes: those frames (80×30 and 160×38, the
narrow diff and the split repository browser) are the sizes the new cases paint at, and the
hit-map rows they reach had no other case visiting them.

## W7 in wave 8: three lines that no reader can reach

Each is a guard the *caller* already made, and each is waived on that mechanism — not on "it is
defensive code".

| lines | mechanism | positive control | falsifier |
|---|---|---|---|
| `mod.rs:1473` `Some(Action::Quit) => return vec![WbAction::Quit]` | `handle_key` answers `action(key) == Some(Quit)` eight lines earlier (`1464`) with the same `WbAction::Quit`, before the overlay check and before this `match`; the arm is a second copy of that answer and `match`'s subject is unchanged in between, so no key reaches it. | Every quit the suite exercises lands on `1464`: `ctrl+x` and the palette's quit both read `vec![WbAction::Quit]`, and `every_overlay_refuses_the_keys_it_does_not_own` shows a modal does not swallow it. | Any assignment to `self.overlay` or `self.keys` between `1464` and `1472` — the two are adjacent and both reads are of `self.keys.action`, so a reordering that put the overlay check first makes this arm the live one and voids the waiver. |
| `mod.rs:3413` `return Vec::new();` under `if !data.mouse_capture` | `handle_mouse` has exactly one caller (`handle`, line 1276) and that arm is `WbEvent::Mouse(event) if data.mouse_capture`, so the parameter this guard tests has already been tested to be `true`. `handle`'s own `WbEvent::Mouse(_) => Vec::new()` (line 1277) is the same answer for the other case, and *it* is measured. | `rebound_context_keys_and_a_released_mouse_still_reach_the_surface` sets `data.mouse_capture = false` and reads the empty answer — through 1277, which is the live line for that reader. | A second call site for `handle_mouse`, or the guard at 1276 losing its `if` — either makes 3413 a live branch. |
| `mod.rs:3541` `return Vec::new();` under `let Some(overlay) = self.overlay else` | `handle_surface_mouse` has exactly one caller (`handle_mouse`, line 3433) and that call is guarded by `self.overlay.is_some() && !matches!(self.overlay, Some(Overlay::Picker(_)))`. The `else` is therefore the case the caller excludes. | Every overlay case in this ledger enters `handle_surface_mouse` with `overlay` already `Some`: the close-chip case, the wheel cases and the drag cases all read the box they opened. | A second entry into `handle_surface_mouse` (a panel that paints without setting `overlay`, say), or the `is_some()` in the caller's guard being dropped. |

**Resolved in wave 9 — both now have a per-region read.** `mod.rs:3594` is a real branch (the
`None` path of the drag's `checked_div`) that no frame can reach, and is waived as **W7**;
`mod.rs:1563` is a line-attribution artefact of the `match`'s scrutinee. See "Wave 9" for the
segment tables, the mechanism, the positive control and the falsifier of each.

## Wave 9 (2026-11-01, r20/r21): the ledger was a commit stale, 97 → 55

This wave was handed `mod.rs` at **97** with eight rows named to close. **Six were already
covered**, and the reason is a measurement error in this file rather than a missing test:
`tui2-r19.info` was built from a tree one commit older than the source its rows were then printed
against.

- `r19` is from **14:15**. `acea6952c` was committed at **14:20** and `dfaad08d2` ("the mention's
  debug and narrowing, a document that scrolls, and three clicks that name nothing") at **14:31**.
- Re-measured on the committed tree, `r20` reads `mod.rs` **57** with no test added in between, and
  the eight named rows fall where the two later commits put them: `244-251` (`Debug for Mention`,
  asserted by `format!("{open:?}")` in `a_narrowed_mention_starts_its_highlight_over`),
  `2991-2992` / `2998-3000` / `3032-3036` (the repository panel's `0`, `tab`+`up` and `home`, all
  pressed by `the_repository_list_pans_and_its_cursor_jumps_home`), `3906-3907` (the press on
  `input`), `3939`/`3951`/`3964` and `4322` by `dfaad08d2`'s own cases. Of the eight, **two were
  real** — `2938` and `3939` — and both are closed below.

**Rule this wave adds: an `.info` is evidence only for the tree that built it.** `missed_exact.py`
prints the *current* source against the *file's* line numbers, so a stale `.info` never looks
broken — it prints plausible text against the wrong lines (here `8107` read as
`vec![WbAction::Redraw]`, which it is, sixteen lines later). Before quoting a number, compare
`git log -1 --format=%ci` with the `.info`'s mtime; every row in this file should name the commit
its pass measured.

Two passes ten minutes apart, `r20` and `r21` — **not** a reproducibility pair: the two cases
below were committed between them (`366cab908` 14:44, `eaa3514d7` 14:45; `r20` 14:37, `r21` 14:47),
so `mod.rs` **57 → 55** here is those two cases landing, not stability. The pair that holds the
tree still is `r22`/`r23` in wave 10, and it moves the same two files by **zero** lines.

| | r19 (stale) | **r20** | **r21** |
|---|---:|---:|---:|
| `run.rs` DA:0 | 94 | **94** | **94** |
| `mod.rs` DA:0 | 97 | **57** | **55** |
| lane total DA:0 | 1336 | **1298** | **1293** |

| was | how it closed |
|---|---|
| `mod.rs:2938` `self.panel_scroll[slot] = index;` | `home_brings_the_files_window_back_to_the_row_it_names`. The files panel and the rail's 文件 section share one `panel_scroll` slot, and the wheel over the rail is the only thing that lowers it (`select_file` is what raises it again), so the case wheels the rail down a burst, opens `/files`, puts the cursor on the last row and presses `home`: both the cursor *and* the window are asserted at 0. Without the line the window stays at 3 and the assertion fails. |
| `mod.rs:3939` `vec![WbAction::Redraw]` under `if actions.is_empty()` after a clicked link | `a_click_that_names_a_link_the_frame_no_longer_holds_is_a_repaint`. A press resolves an id from the last painted frame, and a reply rewriting the row it is on keeps the row count while changing the content — so a `chat.link.1` born of a frame with two links names nothing once there is one. The cursor is left at `None`, `stop_reveal` is false, and the answer is a repaint. (The span cache is keyed on `data.revision` and the row count, not on content: writing `messages` without bumping `revision` leaves the old spans in place and the click still opened the link — how the first draft of this case failed, and worth knowing before writing another.) |

### `mod.rs:3594` — a real branch no frame can reach (**W7**)

`llvm-cov show` prints region *entries*, and every one of `3593`, `3594`, `3595` is one — so the
question is which region each belongs to. `llvm-cov export` answers it (columns are 1-based, the
third field is the count):

| segment | source | what it is |
|---|---|---|
| `line=3588 col=52 count=24` | `if let Some(percent) = (want * 100 + inner / 2).checked_div(inner) {` | the `if let`, entered 24 times |
| `line=3593 col=33 count=20` | `}` | the *inner* `if`'s false edge — 24 − 4, the four drags that moved the ratio |
| `line=3594 col=29 count=0` | `}` | **the `if let`'s `None` edge** — the only counter on it, and it is 0 |
| `line=3596 col=32 count=21` | `return Vec::new();` | the press that is not a drag |

So `3594` is not a brace artefact: it is the `None` path of `checked_div`, which is `inner == 0`.

**Mechanism.** `git_split_span.0` has exactly two writers — the `(0, 0)` default (`mod.rs:924`)
and `set_git_geometry(inner, left_w)` (`mod.rs:4196`), whose only caller is the split painter at
`screen.rs:2512` with `inner = columns.saturating_sub(2)` under `columns >= GIT_SPLIT_COLUMNS`,
which is **108**. A frame that painted a `git.divider` therefore leaves the span at **≥ 106**. The
`(0, 0)` default cannot pair with a drag either: `git_drag` is set only by a press on the
`git.divider` hit region (`mod.rs:3773-3781`), a region the same paint created.

**Positive control.** `dragging_the_git_divider_re_balances_the_panes` and its two siblings drive
the drag both ways: `3588`, `3589` and `3590` read 24, `3591`/`3592` 4, `3593` 20. Every other
edge of the same `if let` is measured; only the zero-width one is not.

**Falsifier.** Any third writer of `git_split_span`; `GIT_SPLIT_COLUMNS` at or below 2; or a paint
that calls `set_git_geometry` before `split` is decided. Each makes `inner == 0` reachable with a
drag in flight and voids this waiver.

### `mod.rs:1563` — the scrutinee's counter, not a decision

| segment | source | what it is |
|---|---|---|
| `line=1561 col=18 count=351` | `_ => {}` | the *previous* match's fall-through: 351 keys reached this one |
| `line=1563 col=15 count=0` | `key` | the only region on the line — the scrutinee, cols 15-17 |
| `line=1564 col=13 count=351` | `"ctrl+q"` | the first arm's *pattern* comparison, on the very next line |

The value cannot be compared 351 times without being read, so there is no control-flow path
through `1563` that the arms do not take: the scrutinee's copy was folded into the comparison
chain and its counter sits on one edge of that chain. The same construct elsewhere in this file
shows the same signature from the other side — `match key` at **2954** reads **2** against **106**
on its own `"tab"` arm — so a `match key` line whose count is far below its arms is this file's
normal, not a defect.

**What this does not claim:** the `"ctrl+q"` arm's *body* never runs (the reader's quit is
`ctrl+x`), and `1564` still reads covered — a string arm's pattern shares the line with its body.
That is the mirror direction of "The metric's own blind spot on `match` and `matches!` arms"
above, and one more reason this file reads arms by what they *do* and not by the line they sit on.

**Falsifier.** A second region on `1563` — any segment besides the scrutinee at col 15 — staying 0
while the arms run.

### The per-region read, and the binary it has to be pointed at

```bash
cd /Users/geilige/future-os/.worktrees/cov-tui2
LC=~/.rustup/toolchains/1.99.1-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin/llvm-cov
# the lib target is the one r21-test.log names under "Running unittests src/lib.rs"
BIN=target/llvm-cov-target/debug/deps/future_tui-80b655b233b84e48
$LC export "$BIN" -instr-profile=target/llvm-cov-target/cov-tui2.profdata > /tmp/x.json
# then: segments are [line, col, count, hasCount, isRegionEntry, isGapRegion]
```

**The binary matters.** `cargo llvm-cov` leaves several `future_tui-*` executables in `deps/`, and
pointing this at the wrong one does not fail — it prints the source with almost every line at
zero (measured: 282 non-zero region lines of 14 536 on the wrong binary, 8 792 on the right one),
which reads exactly like a lane where nothing ran.

## Wave 10 (2026-11-01, r22/r23/r24): three passes on one tree, the flake named, three ranges retracted

Wave 9 ended with a numbered pair (`r20`/`r21`) that **was not** a pair: two cases were committed
between the passes. This wave measured the tree three times with nothing committed in between, and
once more with the one test that mutates the process-wide cwd removed. Tree for all four: `e6e2c0474`
(docs-only) on top of `eaa3514d7`; the code is `eaa3514d7`'s.

| | r21 (wave 9) | **r22** | **r23** | **r24** (skip-cwd) |
|---|---:|---:|---:|---:|
| `workbench/mod.rs` DA:0 | 55 | **55** | **55** | **55** |
| `workbench/run.rs` DA:0 | 94 | **94** | **94** | **94** |
| `workbench/live.rs` DA:0 | 335 | **337** | **335** | **337** |
| `workbench/screen.rs` DA:0 | 85 | **87** | **87** | **87** |
| `index.rs` DA:0 | 39 | **39** | **39** | **55** |
| lane total DA:0 | 1293 | **1297** | **1295** | **1313** |

`r24` = `run-cov-tui2-skipcwd.sh`, which is `run-cov-tui2.sh` with one extra `--skip` on
`build_initial_prompt_relative_path_uses_cwd`. It is not a candidate lane number (one test's 16
lines in `index.rs` leave with it); it exists to test the mechanism below.

**The two files this ledger audits are stable to the line.** `mod.rs` **55** and `run.rs` **94** in
all four passes, and the DA:0 *sets* are identical: `identical.py tui2-r22.info tui2-r23.info
workbench/mod.rs` finds 8 891 lines in both files with the same 55 at zero (86 rows differ only in
their covered counts), and the same for `run.rs` (4 123 lines, 94 at zero, 291 count-only rows).
So a covered-line count is not a reproducible number and a zero set is — which is the whole reason
metric A reads `DA:0` and not a percentage.

**The ±4 in the lane total is two files, and the flips are four lines, each named.**
`same_tree.py` compares two passes line by line and prints the source of every line that crosses
zero. `r22`→`r23`: exactly `live.rs:3685` and `live.rs:3688` (0 → 2). `r21`→`r22`: those two
(1 → 0) **and** `screen.rs:2491` (1 → 0) with `screen.rs:3022` (4 → 0). Nothing else in 64 568
lines moves across zero.

- `live.rs:3685` `Err(error) => Err(error),` and `:3688` `self.notice = Some(… WorkflowFailed …)`
  are `refresh_worktrees`'s failure arm. `probe_repo` (`mod.rs:1018-1025`'s blocking closure) errs
  only when **both** candidates fail: the session's cwd *and* `std::env::current_dir()`
  (`worktree.rs:576-609`). The crate has exactly one test that moves the process cwd
  (`index.rs:1437` `build_initial_prompt_relative_path_uses_cwd`, `set_current_dir` at `1444`,
  restored at `1446`) and it holds only `test_env::lock()`, which the workbench tests do not take —
  so for the few milliseconds it is in that temp directory, a concurrent `refresh_worktrees`
  fails, and the count depends on the interleaving (measured 0, 1, 2 across the three runs).
  **Positive control:** `r24`, the same lane with that test skipped, reads **0** on both lines —
  the arm goes dark when the cwd mutation is gone, which is what the mechanism predicts.
  **Falsifier:** those two lines staying 0 with the test present, or any other test that calls
  `std::env::set_current_dir`.
- `screen.rs:2491` (`vec![t(lang, Key::GitNoSelection) …]`) and `:3022` (`palette.dim(&chunk)`,
  the "similarity index" dim list) were measured in `r21` and read 0 in `r22`, `r23` and `r24`.
  The source of `screen.rs` did not change between those passes (the only commit between `r21` and
  `r22` is the docs commit), and `r24` removes the cwd test without changing them — so this is
  **not** the same mechanism and it is **not** explained yet. What it does change is the reading of
  any row that cites `r21`: three later passes agree on 0, so the `r21` reading is the outlier and
  no row in this file may be closed on `r21` alone. **Next check:** which case paints a git panel
  whose `detail` is empty and which paints a diff containing a rename, and whether either reads the
  filesystem through a path that the `test_env` lock does not cover.

**Three ranges retracted, and a taxonomy correction** (both in wave 6's section): `run.rs:960`,
`:1369` and `:1377` were inside waived ranges and are measured — the ranges now stop at `959`,
`1368` and `1376`. The eight rows wave 6 filed as "closing braces" are off by one line each and are
arm bodies — the `JoinError` arms of five `spawn_blocking(..).await` calls, plus `1083`, `1348` and
the dead `OpenApproval` branch at `1288`; each now has its own mechanism, positive control and
falsifier rather than a shared taxonomy label. `waived_still_dark.py` is the check that keeps this
honest: it asserts every row this file waives is still `DA:0` in a named pass, and it is what
found the three.

```bash
cd coverage/2026-11/after/tui2
python3 waived_still_dark.py tui2-r22.info    # exits non-zero if a waived row got measured
python3 per_file.py tui2-r21.info tui2-r22.info tui2-r23.info tui2-r24.info
python3 same_tree.py tui2-r22.info tui2-r23.info
python3 identical.py tui2-r22.info tui2-r23.info workbench/mod.rs
```

## Wave 11 (2026-11-01, r25/r26/r27/r28): three functions eaten, and the two lines that were only ever painted by accident

Rebased onto `origin/dev-huichen` (`b7cf2931d`, eight commits from the sibling lanes) **before**
measuring anything: `r25` is this round's baseline and every figure below is from it or later. The
rebase touched `live.rs` only, so `run.rs`/`mod.rs`/`screen.rs` line numbers carried over and the
wave-10 rows still name the same lines.

| | r25 (baseline) | **r28** |
|---|---:|---:|
| `workbench/mod.rs` DA:0 | 55 | **42** |
| `workbench/run.rs` DA:0 | 94 | **94** |
| `workbench/screen.rs` DA:0 | 87 | **84** |
| lane total DA:0 | 1300 | **1279** |

`live.rs` reads 337 at `r25` and 335 at `r28` — **the same ±2 wave 10 named** (`live.rs:3685`,
`:3688`, `refresh_worktrees`' failure arm racing `index.rs`'s cwd test). It is not this round's
work and it is not in this round's numbers: compare within a row shape, as above.

### The three functions, line by line

**`handle_workflow_key` — 8 → 1** (the one left is a new waiver, `3138`). Two cases:

| was dark | what it is | the case that closed it |
|---|---|---|
| `3216-3221` | `a` on the card **with** a suggested rule: `ApprovalDecision{approved: true, rule: true}` and the card closes | `the_workflow_boxes_move_scroll_and_answer` — it had only the *refusal* (`a` with no rule), which is the other side of the same `if` |
| `3251` | `enter` on the card: `_ => vec![WbAction::Redraw]`, after `self.overlay = None` | same case. `enter` dismisses; it never approves — a pending approval is answered by `y`/`n`/`a`, and the loop re-opens the card on the next state refresh |

**`step_stop` — 4 → 0**, one case: `shift_tab_starts_the_walk_at_the_window_bottom_edge`. The
four lines were the `delta < 0` arm of the "no cursor, or a cursor the keys took away" branch
(`rows.iter().rposition(|row| *row < end).or_else(..).unwrap_or(count - 1)`): a backwards press
starts at the **last** stop above the window's bottom edge, where the forward one starts at the
first inside it. The suite only ever walked forwards from a dropped cursor
(`tab_starts_from_the_window_after_a_scroll_took_the_cursor_away`), so the backwards edge had no
case. The new one asserts the landed *row* (40, then 20, then the window's edge after a `pageUp`),
which is the discriminating claim: the forward arm would answer 0.

**`handle_surface_mouse` — 6 → 4**, two cases and three waivers:

| was dark | what it is | case / disposition |
|---|---|---|
| `3718` | a press on `palette.row.N` whose row the query took away: `None => vec![WbAction::Redraw]` | `a_press_on_a_palette_row_the_query_took_away_repaints` — paints the palette, types `zzzz` without painting again, presses the region the *old* frame registered |
| `3752` | a press on a `git.row.N` that is a graph continuation: `return Vec::new()` — the cursor stays | `the_git_panel_answers_the_pointer`, extended with the keyless row `the_git_cursor_steps_over_rows_with_no_object` uses |
| `3594` | a drag whose offset rounds back to the ratio already set | **measured by `dragging_the_git_divider_re_balances_the_panes`** (a drag with `moved == 0`), which is the *Some* edge only — the line still reads `DA:0` because its zero counter is the `checked_div` **None** edge (wave 9's per-region read). Waiver unchanged |
| `3541` | `let Some(overlay) = self.overlay else { return Vec::new() }` | waiver from wave 8, unchanged (the caller's guard is the enumeration) |
| `3804-3805` | the press chain's tail: `Vec::new()` for an id no branch claimed | **waived in wave 11**, see below |

### Three new waivers

| lines | mechanism | positive control | falsifier |
|---|---|---|---|
| `mod.rs:3138` `_ => 0,` in `handle_workflow_key`'s `count` | the function has exactly one caller (`handle_overlay_key`, `mod.rs:2480-2486`) and that guard admits exactly `Approval`, `Diff`, `Fork`, `Worktree` — the four arms above `_`. The tail is total by construction | all four overlays reach `count`: the approval cases read `3137`, the diff cases `3136`, the fork and worktree cases `3134`/`3135` | a fifth overlay added to the caller's `matches!`; then `3138` is a *measurement* waiting for a count |
| `mod.rs:3804-3805` `}` / `Vec::new()` at the end of the press chain | every id any overlay paints is claimed by a branch above it: `OVERLAY_CLOSE`, `overlay.surface`, `approval.{y,n,a}`, `diff.patch`, `diff.file.N`, `files.row.N`, `git.{row.N,tab.N,detail,divider}`, `keybinding.row.N`, `palette.row.N`, `preferences.{row.N,language,theme,reset,shortcuts}`, `preview.*`, `queue.row.N`, `workflow.row.N`. `hit_after(x, y, overlay_mark)` only answers with regions the overlay itself painted, so a click that lands anywhere else returns `None` at `3629-3638` — the "clicked the dimmed page" branch, which *is* measured | the pointer cases walk the claimed set: `the_git_panel_answers_the_pointer`, `the_command_palette_runs_the_entry_under_the_pointer`, `the_approval_buttons_do_not_hide_in_scroll`, `dragging_the_git_divider_re_balances_the_panes`, the preview controls and the queue row's recall | any overlay that paints an id outside that list, or `hit_after` being called with a mark of 0 (a page id would then reach the chain) |

Both are "the chain is total" waivers, which this file treats as the weakest kind — they are taken
only after the *claimed* set was enumerated from the painters (`screen.rs`'s `hit_map().push`
sites) rather than assumed.

### `screen.rs:2491` and `:3022` — the flip wave 10 could not explain, and what it was

Wave 10 measured `r21` at 1 and 4 on these two lines, and `r22`/`r23`/`r24` at 0 on both, with the
source unchanged — and wrote "not explained yet". **Explained, with a positive control, and then
removed from the lane's variance.**

- `2491` is `git_box`'s placeholder (`vec![t(lang, Key::GitNoSelection)…]`) for a view whose rows
  exist and whose `detail` is empty. `3022` is the `palette.dim(&chunk)` that paints git's own
  metadata lines (`index …`, `new file mode`, `similarity index`, `rename from/to`, …) — one
  `return` shared by the whole word list, so any of them counts.
- **The only suite member that can paint them is
  `every_repository_section_opens_without_an_error`**, and it did so through its session's `cwd`:
  it used `Driver::attach_conversation()`, whose workspace is `CARGO_MANIFEST_DIR` — the worktree's
  `tui/` directory, i.e. **the checkout the sources are in**. Its 改动 section has rows only when
  that checkout is **dirty**, and its detail is a working-tree patch only when there is one to
  read. So the reach depended on the machine's state, not on the suite: `r21` was measured while
  the wave-10 doc edit was uncommitted, `r22`–`r25` on a clean tree.
- **Positive control, measured.** `./one.sh` (new, in the lane dir) runs one test against the
  already-instrumented objects and writes its own lcov. With one tracked file edited and that test
  as the only case: `screen.rs:2491` reads **1**, `:3022` reads **0**; on a clean checkout both
  read **0**. (`:3022` stayed 0 in that probe because the *selected* row's patch came back empty —
  see the defect below — which is also how `r21` can read 1 and 4 at once from two paints.)
- **The interval it exposed is a defect, not a fixture.** `git status --porcelain` prints paths
  *relative to the repository root*; `diff_review::load` passes that path to `git diff -- <path>`,
  whose pathspec is *relative to the session's directory*. At the root they agree; a session
  opened in a subdirectory therefore gets **no patch for any 改动 row**, which is exactly the
  empty `detail` that paints the placeholder. Recorded, not fixed (it is product behaviour this
  task may not change): `a_row_with_no_body_keeps_the_detail_pane_a_box` asserts it *and* the
  control that proves it is the rooting — the same tab read from the repository root paints the
  patch. Fixing `load` turns that case into the patch case and leaves the placeholder to a view
  whose rows carry no object at all; the test fails on `detail.is_empty()`, which is the intent.
- **Both lines are now driven on purpose**, from fixtures, so no pass depends on the checkout:
  `the_stash_tab_paints_git_s_rename_metadata` (a 储藏 entry read as a whole patch: `similarity
  index`, `rename from`, `rename to` → `3022` three times) and
  `a_row_with_no_body_keeps_the_detail_pane_a_box` (the placeholder → `2491`). In isolation
  (`cargo llvm-cov … -- a_row_with_no_body`, then the stash case) `2491` reads **4** and `3022`
  reads **6**.
- **And the door is closed**: `every_repository_section_opens_without_an_error` now states its own
  premise (`repo_with_two_commits()`) instead of reading the checkout, so the lane no longer has a
  number that depends on whether the tree was dirty. That is the same class of fix as wave 5's
  `absolute_tool` and wave 10's cwd test.

### What this wave did not do

`handle_overlay_key` (4), `click` (4), `selected_stop` (3) and `handle_mouse` (3) are the next
clusters and none of them was touched. `run.rs` is unchanged at 94 — its 94 are the same rows the
wave-10 list names, and `1288` is now OPEN rather than waived.

## How to re-measure this file

```bash
cd coverage/2026-11/after/tui2
git -C ../../../.worktrees/cov-tui2 log -1 --format='%ci %h'   # the tree the pass will measure
TAG=r28 METERS=0 ./run-cov-tui2.sh
python3 missed_run.py tui2-r28.info workbench/run.rs        # counts
python3 missed_exact.py tui2-r28.info /Users/geilige/future-os/.worktrees/cov-tui2/tui/src/workbench/mod.rs workbench/mod.rs
python3 clusters.py tui2-r28.info /Users/geilige/future-os/.worktrees/cov-tui2/tui/src/workbench/mod.rs workbench/mod.rs
python3 total.py tui2-r28.info                              # the lane's own DA:0
python3 waived_still_dark.py tui2-r28.info                  # exits non-zero if a waived row got measured
```

**One case at a time**, without a whole pass and without rebuilding: `one.sh <tag> <filter>` runs
the already-instrumented objects for one test (or a filter) and prints the two `screen.rs` rows
this file argues about; replace its `at.py` call with any other line check. It is how wave 11
settled "which test paints this": `./one.sh sections-clean
every_repository_section_opens_without_an_error` on a dirty checkout, then on a clean one.

`missed_exact.py` takes an **absolute** path for the source (a relative `../../../` breaks as soon
as the script is run from anywhere but this directory) — wave 10 hit that. It rather than
`ranges.py` for the lines that follow: `ranges.py` merges anything within three lines, so a printed
range can hide covered lines inside it (`1550-1552` read as three misses when `1551` was measured
twice). `METERS=0` unless the row you are checking is a meter: a `METERS=1` pass and a `METERS=0`
pass are not the same lane number (see the wave-8 note above — they differ by the 151 lines inside
the ignored tests).

**Record the commit with every pass.** A `.info` is evidence for one tree only, and the rows print
the current source either way, so a stale pass is invisible until a row is chased that was already
closed (wave 9 chased six). `r19` was one commit behind its own description; `r20`/`r21` were the
first two passes whose tree was named (and they were **not** the same tree — wave 10 corrected
that); `r22`/`r23` are the first genuine same-tree pair in this ledger.

`rm -rf target/llvm-cov-target` afterwards: the instrumented build is ~4 GB and nothing but
this lane reads it.
