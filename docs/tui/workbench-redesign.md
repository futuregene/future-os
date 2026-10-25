# FutureOS workbench TUI — ordered change list

The engineering plan for the full-screen workbench: what is already true, what
changes next, in which file, in what order, and how each piece is accepted. The
frozen contract (module layout, type names, keymap, layout constants, demo CLI)
is [workbench-spec.md](workbench-spec.md); the evidence for the target and the
gaps is [dsh-tui-parity.md](dsh-tui-parity.md); the per-requirement acceptance
record with screenshots is [verification.md](verification.md). Where this
document disagrees with the contract, the contract wins and this document is
wrong — say so.

Scope: `tui/` only. No `packages/rpc` change, no agent change (parity doc §7 has
the full data-sourcing table).

## 0. Status: what already landed

This plan used to describe a shell that did not exist. It exists now, and this
revision is ordered against the code, not against the intent.

| Landed | Commit | What it settled |
|---|---|---|
| the workbench as a second entry point | `7d62e8990` | the compositor, hit map, launchpad, themes |
| `future tui` **is** the workbench | `2101b3c98` | one surface, no layout switch |
| the inline UI deleted | `d701765ca` | the old page and its flags are gone, not deprecated |
| the mouse works end to end | `e72b1213d` | SGR 1006 + hit map + drag + wheel; the frame no longer ends in a line feed |
| the layout degrades whole | `1767726ee` | no mid-word cuts, no dead rail, the header corner falls back by whole pieces |
| the panels read real stores | `b55bd66eb` | goal todos, skills, jobs, sessions, usage |
| the frame addresses every row absolutely | `77c20b9cb` | a frame taller than the terminal can no longer scroll the screen |
| one picker behind `/model`, `/theme`, `/sessions` | `8d1afa2c7` | six palettes with a live preview, model list with its reasoning slider, session list with search |

State at `ba916d63e`: 15 modules / 9,076 lines / 119 tests in
`tui/src/workbench`, `cargo test -p future-tui workbench` green (119 passed),
`cargo test -p future-tui --lib` 1,013 passed, `workbench_demo --self-test` ok,
and 19 frames in `docs/tui/assets/` (see [verification.md](verification.md)).

Four of those commits are the ones the earlier plan treated as open work, and
they are what makes the rest of this list cheap rather than speculative:

1. **Mouse, SGR 1006, end to end** (`e72b1213d`): the mode is written with the
   alternate screen and written back on stop/crash; reports are parsed into
   `Press/Release/Drag/Move/Wheel`; drawing and hit-testing share one `HitMap`
   (`tui/src/workbench/layout.rs`); a click switches a panel, the wheel scrolls
   the chat or the focused panel, the divider drags. Verified live:
   `verification.md` §2.2, 图 4 and 图 17 (the drag's sign is the one defect
   left in the chain, P0-1).
2. **The five panels read real data, not fixtures** (`b55bd66eb`): 待办 folds the
   goal's `events.jsonl`, 技能 combines `get_state` with both skill directories,
   任务 opens `tasks.db` read-only, 会话 comes from `list_sessions`, 用量 from
   `get_state` (`tui/src/workbench/local.rs`, `live.rs:201`). Verified live:
   `verification.md` §2.3, 图 3–图 5.
3. **Degradation is whole-piece** (`1767726ee`): below 88 columns the rail goes
   and leaves no dead column; below 24 rows the header is one row, the input
   keeps its frame, activity goes and the hints drop whole entries; the header's
   right corner falls back `● app vX` → `● app` → `● vX` → `●`. Verified live at
   120×38 / 90×24 / 60×20: `verification.md` §2.7, 图 2, 图 14–图 16.
4. **Every row is addressed, and the three list verbs have a real menu**
   (`77c20b9cb`, `8d1afa2c7`): a frame can no longer scroll the terminal, and
   `/model`, `/theme`, `/sessions` open one overlay with a search line, group
   headings, a `✓` on the value in force, a reasoning slider and click-to-apply
   rows. Verified live: `verification.md` §2.5, 图 8–图 13.

What is left is not "build the shell". It is four defects inside the shell, then
the parts of the old UI the shell did not carry over, then parity polish.

## 1. Settled decisions that change the earlier plan

**F1 — the flag story is over; there is no `--layout` and no `--legacy`.**
The earlier plan said "`--layout workbench|inline`, default workbench" and the
contract said "inline is deleted, and its `--legacy` flag with it". The contract
won. `future tui` starts the workbench and nothing else; `--layout`,
`--legacy`, `--classic` and `--resume` do not exist, and
`help_describes_this_build` fails if help text names them
(`tui/src/help.rs:96`). **Consequence for this list: never add a renderer
switch back.** A second renderer sharing one output stream was the rejected
design, and a "temporary" flag is how it comes back.

**F2 — the workbench is a compositor, not a component.** Components own rows
(`Component::render(width) -> Vec<String>`); the workbench owns rectangles and
returns exactly `h` rows of exactly `w` columns (`tui/src/workbench/screen.rs:29`).
Consequence: the transcript is adapted, not reimplemented — `live.rs` renders
messages into rows and the compositor paints them into the chat rect.

**F3 — the frame addresses every row; what is left is the size it is composed
for.** `frame_bytes` places each row with `CSI row;1H` (`tui/src/workbench/run.rs:368`)
and never advances the cursor, so a frame taller than the terminal is simply
clipped instead of scrolling the screen — that landed in `77c20b9cb`. What is
not pinned is the *height the first frame is composed for*: measured during this
work, one instance painted its first frame at 120×38 one row high (hairline on
row 5, tab strip on row 6, no spacer) and repaired it on the next redraw, while
another instance at the same size was correct at 3/6/9 s. That is consistent
with a stale size reaching the first compose, not with the writer. Consequence:
the writer stays as it is; P0-2 asks for a test of "the composed row count equals
the height the writer was handed" and, if the size can be stale, a re-read
before the first compose.

**F4 — the mouse is real, and one axis of it is inverted.** The whole chain is
wired: `Terminal::set_mouse_mode(true)` before `start`
(`tui/src/workbench/run.rs:92`), SGR 1006 parsing (`tui/src/workbench/mouse.rs:56`),
a hit map shared with the renderer, and `?1000h?1002h?1006h`. Clicking a tab
switches the panel (measured), the wheel scrolls, and the divider drag changes
`sidebar_w` — **in the wrong direction**: `width = start_width + (x - start_x)`
(`tui/src/workbench/mod.rs:568`) while the divider sits at `W - width - 1`, so
dragging the edge right makes the rail *wider* and the edge moves left. Measured
twice (36 → 42 on a rightward drag, 42 → 36 on a leftward one). One sign, P0-1.

**F5 — five palette entries have no handler; the other three are menus.**
`COMMANDS` lists 12 (`tui/src/workbench/mod.rs:904`); `run.rs:249` intercepts
`/model`, `/theme` and `/sessions` and opens the picker for them (from the
palette and from a typed command alike), and `command()` handles
`quit/new/reply-detail/refresh/help` plus the launchpad chips
(`tui/src/workbench/live.rs:694`). So `/workspace`, `/usage`, `/skills`,
`/export`, `/transcript` still answer `Unknown command` — measured on two of
them (`[S:workbench-dead-usage.png]`, `[S:workbench-dead-skills.png]`). P0-3.

**F6 — deleting the inline UI removed screens, not just pixels.** `/sandbox`,
`/worktree`, `/keymap`, `/copy`, the external editor and the full-screen session
manager have no workbench surface. Two of them are things FutureOS was *ahead*
on (parity doc §6 rows 34, 36). P1-3.

## 2. P0 — defects inside the shell (ordered)

Each item: symptom with evidence, landing point, the test that pins it, the
command that accepts it.

### P0-1 The divider drag moves the rail opposite the pointer

- **Now**: dragging the divider right grows the sidebar (`36 → 42` measured
  under tmux at 120×38); dragging left shrinks it. The edge under the pointer
  moves away from the pointer.
- **Fix**: `tui/src/workbench/mod.rs:568` — `width = start_width - delta`
  (`delta = event.x - drag.start_x`), keeping the `SIDEBAR_MIN..SIDEBAR_MAX`
  clamp. `start_width` and `start_x` stay as they are: the grab point is where
  the divider *is*, so an off-centre grab must not jump the rail.
- **Test**: `a_drag_towards_the_edge_narrows_the_rail` — build a 120×38 frame,
  press on the divider, drag right by 6, assert `ResizeSidebar(30)`; drag left
  by 6 from the default, assert `ResizeSidebar(42)`; both clamp.
- **Accept**: `cargo test -p future-tui workbench`, then the live check in
  `verification.md` §2.2/M5.

### P0-2 The first frame may be composed for a stale height

- **Now**: `frame_bytes` addresses every row absolutely and never advances the
  cursor (`tui/src/workbench/run.rs:368`), so a frame taller than the terminal is
  clipped instead of scrolling the screen — `77c20b9cb` removed the old failure
  mode. What is *not* pinned is the height the first frame is composed for. Two
  measurements frame it:
  - one instance at 120×38 painted its first frame **one row high** (hairline on
    row 5, tab strip on row 6, no spacer) and the next redraw repaired it;
  - a fresh instance of the same build at the same size was correct at 3 s, 6 s
    and 9 s after start.

  Both were taken on the code that addresses rows absolutely, which is why the
  writer is not the suspect: a stale size reaching the first compose explains
  both observations, and nothing else does.
- **Diagnosis first**: where does the first compose get `h`? Does the resize
  event that carries the real size arrive after the first `render`? Decide with
  a test, not a rewrite.
- **Fix, in this order**: (1) a test that the composed row count equals the `h`
  the writer was handed, on the writer side as well as `render`'s
  `debug_assert!`; (2) if the size can be stale at startup, re-read it
  immediately before composing and skip the frame when it changed mid-render;
  (3) only then consider a second paint after the first resize — which is what a
  terminal that repaints on `SIGWINCH` already gets for free.
- **Test**: `a_frame_has_exactly_the_rows_it_was_asked_for`, plus the existing
  `a_frame_never_ends_in_a_line_feed`.
- **Accept**: `cargo test -p future-tui workbench`; manual: start at 120×38 and
  compare the first frame with the steady-state one (`verification.md` §2.1/L7),
  then shrink the window mid-run (120→90) and click a sidebar tab — the click
  must hit the tab, not the row above.

### P0-3 Five commands are advertised and do nothing

- **Now**: `/workspace`, `/usage`, `/skills`, `/export`, `/transcript` →
  `Unknown command` (measured on `/usage` and `/skills`: `[S:workbench-dead-usage.png]`,
  `[S:workbench-dead-skills.png]`). `/model`, `/theme` and `/sessions` are *not*
  in this list: `run.rs:249` turns them into pickers before `command()` sees
  them.
- **Fix**: one of two, per command, in `live.rs::command()`:
  - `/usage` → `SetPanel(Usage)`; `/skills` → `SetPanel(Skills)`;
    `/workspace` → open the workspace picker (or focus the header's cwd);
    these are *navigation* and cost three match arms.
  - `/export` and `/transcript` need a file path and a transcript dump; either
    wire them to the existing exporter in the crate or **remove them from
    `COMMANDS`** until they work.
- **Test**: `every_palette_entry_has_a_handler` — for each `(command, _)` in
  `COMMANDS`, the command must resolve either to a picker (`PickerKind::from_command`)
  or to an arm of `command()` that is not the `Unknown command` fallback. A
  table test is the only way this does not rot again; it also catches the next
  verb that gets a picker but no handler.
- **Accept**: `cargo test -p future-tui workbench`; live: `/` → `usage` → `Enter`
  focuses the 用量 panel and the hint row does **not** say `Unknown command`
  (`verification.md` §2.5/C5).

### P0-4 The git branch never reaches the surface

- **Now**: `SessionMeta { branch: String::new(), … }`
  (`tui/src/workbench/live.rs:553`); the header's second line therefore shows
  cwd only, and `screen.rs:247` already has the `cwd · branch` branch of the
  composition waiting for a value.
- **Fix**: read it where the local facts are already refreshed —
  `Live::refresh_local()` (`tui/src/workbench/live.rs:201`) — with the crate's
  existing `GitCli::branch()` (read-only, timeout, injectable runner), cached
  per cwd and re-read when the session's cwd changes (a worktree session must
  show its own branch). Empty outside a repository, which the header already
  degrades to cwd-only.
- **Test**: `the_branch_is_read_once_per_cwd` — a fake runner counts calls: two
  refreshes at one cwd produce one read, a cwd change produces a second, and a
  failed read leaves the branch empty instead of panicking.
- **Accept**: `cargo test -p future-tui workbench`; live: the header's second
  line reads `/Users/geilige/future-os · dev-huichen` (`verification.md` §2.4/A6).

P0 order matters: P0-1 and P0-3 are user-visible lies, P0-4 is a missing
dsh-parity readout, P0-2 needs a diagnosis before it can be a fix. The three
fixes are single-file changes with one test each; P0-2 starts with a test.

## 3. P1 — what the old UI could do and the shell cannot

Ordered by "a reader hits this in the first ten minutes" first.

### P1-1 Hover: highlight what is clickable

`MOUSE_ON` is `?1000h?1002h?1006h` and `MouseKind::Move` is parsed then dropped
(`tui/src/workbench/mod.rs:584`). Add `?1003h` to `MOUSE_ON` (and keep `1003l`
in `MOUSE_OFF`), track the hovered hit id in the surface, and paint that hit
region's *affordance* — tab label, row background, input border — with the
`iris` token at most once per frame. No tooltip in this step: a tooltip needs a
timer, and a timer that repaints idle frames breaks capability 1 (stillness).
Covered by mouse tests on the hit map, not by a screenshot (hover needs a clock).

### P1-2 The composer's `⛶` affordance and the full-screen draft

There is no affordance and no editor, and the external `$EDITOR` path is no
longer reachable. Land the affordance first (a hit region + a stub overlay that
shows the draft full-width and closes on `esc`), then wire the editor. This is
the composer-continuity capability (§4.7 of parity) and the single cheapest way
to make long prompts usable.

### P1-3 Re-home `/sandbox`, `/worktree`, `/keymap`, `/copy`

These screens exist in the crate and are unreachable from the workbench. Two are
parity rows where FutureOS was **ahead**. Options, in order of preference:
1. an overlay picker (`/sandbox`, `/worktree`) reusing the existing view
   components' rows inside `paint_overlay`, exactly as help/palette do;
2. a sidebar panel for worktree/sandbox state (a 6th tab) when the overlay is
   too much for this round.
`/copy` becomes a chat-rect action (copy the last reply) rather than a screen.

### P1-4 Notifications

`tui/src/notifications.rs` exists; the workbench never calls it. Emit on a run
that settles successfully with no queued follow-up (dsh's rule), and keep the
title's streaming marker. Test on the notification *decision* (a pure function),
not on delivery.

### P1-5 Session manager: pins and a workspace rail

The 会话 panel lists rows with title, model, age and streaming state, and the
`/sessions` picker (`picker.rs`) already searches and switches on a click, so the
filter half of dsh's manager is covered. Missing: pins (persisted in client
settings) and a workspace rail above the list.

### P1-6 Timeline rail

A 2-column rail at the chat's right edge: one tick per user turn, click =
`Scroll` to that turn's row, wheel over it ignored. The hit map already supports
one-cell regions; the transcript adapter must expose turn offsets.

### P1-7 Context bar with composition colours

The gauge is one accent fill. dsh colours the fill by content composition and
clears stale composition after compaction. Needs the composition split from
`get_state`; if it is absent, this stays partial rather than invented.

### P1-8 Settings screen, grouped and searchable

The pickers cover what a reader changes most (`/model` with its reasoning
slider, `/theme`, `/sessions`), and every setting they commit persists. What is
missing is one place that *lists* the rest: a `/settings` overlay with 4–6
groups (appearance, model, permissions, panels, keys). That is the
discoverability piece of capability 8, not a new data path.

### P1-9 History search scoped to the project

Prompt recall exists; `Ctrl+R` search over the project's prompts does not. The
store to search is the session log the agent already exposes.

### P1-10 Steering vs queue vs interrupt

`Enter` submits and queues behind a live run (`live.rs::submit`), `ctrl+x`
aborts. dsh distinguishes steer (`Enter`), queue (`Tab`) and interrupt-and-send
(`Ctrl+Enter`). Do this only after P1-4: the queue badge is what makes the
distinction visible.

### P1-11 Images in the transcript

The workbench has **no** image handling at all: no `is_image_line` equivalent,
no Kitty/iTerm2 row preservation, no attachment chips. The inline UI's guard
(`tui/src/app.rs`, deleted with it) must come back in the compositor before an
image can be painted into a rect — otherwise a sidebar painted over an image row
leaves the image showing through. Not measured live (the captured session has no
image), which is why it is P1 and not P0.

## 4. P2 — parity polish, only after P1 is measured

- `/rewind` and double-`esc` to an earlier turn; `/fork` and `/tree` surfaced in
  the palette.
- Split (two-column) diff cards with one-sided full width.
- Mermaid fences and Unicode LaTeX.
- Image preview with zoom/pan; Sixel.
- In-app selection: double/triple click, per-pane copy, no invented newlines.
- Bilingual chrome (row 23): a product decision, not a translation pass — see
  parity doc §10.3.

## 5. Explicitly not doing

- A second renderer, a `--layout` switch, or a `--legacy` escape hatch (F1).
- ratatui / crossterm / tui-rs: the hand-rolled terminal backend stays.
- A proto change for per-session usage in `list_sessions`, or for any subagent
  surface (parity doc §7.4).
- Subagent, shell-job or plugin panels (parity doc §8, rows 26–28 are `n/a`).
- Re-implementing `/worktree`, `/sandbox`, skills install, print mode or
  `FUTURE_HOME` isolation — they exist; P1-3 is about *reaching* them.

## 6. Data sourcing (what reads what, and how it fails)

Unchanged from the contract (§9) because it is already implemented; repeated
here because every new panel row must follow it.

| Panel | Source | Rule |
|---|---|---|
| 待办 | `<goal root>/goals/<id>/events.jsonl` | `FUTURE_GOAL_ROOT`, else nearest ancestor `.future/goal`, else the git toplevel's; most recently active non-terminal goal; `✔ ◉ ○ ✖` + priority + text |
| 技能 | `get_state.skills` + `~/.future/agent/skills` + `~/.agents/skills` | names from the agent, descriptions from the directories, either alone is enough |
| 任务 | `<home>/tasks/tasks.db` | `SQLITE_OPEN_READ_ONLY`; missing database = "no scheduled jobs", never a created file |
| 会话 | `list_sessions` | title, model, age, streaming; a failed read keeps the last good list |
| 用量 | `get_state.usage` | whole values only, never a number clipped with `…` |
| branch | `git rev-parse --abbrev-ref HEAD` (P0-4) | read-only, timeout, cached per cwd, empty outside a repo |

Every reader is best-effort: a missing store, a half-written line or an
unreadable file yields fewer rows, never a panic and never a notice.

## 7. Risks

| Risk | Mitigation |
|---|---|
| A "temporary" renderer switch returns | `help_describes_this_build` pins the flags' absence; the contract forbids the switch (F1) |
| Mouse regressions are invisible in CI | the hit map is unit-tested at 120×38 / 90×24 / 60×20; the live recipe in `verification.md` §7 is the manual gate |
| The frame shifts under a size mismatch | P0-2 pins the row count, then re-reads the size before composing; the writer's no-trailing-line-feed invariant stays tested |
| Panel readers couple the TUI to another crate's file format | reads are best-effort and versionless; a format change degrades to fewer rows |
| Long sessions | the surface renders only the visible window and pages history on `LoadOlder`; no full-transcript scan per frame |

## 8. Rollout and rollback

There is no rollout switch: `future tui` is the workbench. A defect is fixed on
a task branch and merged — the rollback for a bad surface change is
`git revert` of that commit, not a flag. The frames in `docs/tui/assets/` are
the visual baseline; re-shoot them when a change alters a band
(`tui/scripts/shot.sh`).

## 9. Acceptance for this goal

Per change (worktree, on the task branch):

```bash
cargo fmt -p future-tui --check
cargo clippy -p future-tui --all-targets -- -D warnings
cargo test -p future-tui workbench
cargo run -q -p future-tui --example workbench_demo -- --self-test   # prints `self-test ok`
```

Screens (only when a band changes; recipe in `verification.md` §7):

```bash
# live session, any scenario/size (this document's frames)
tui/scripts/shot.sh chat 120x38     # docs/tui/assets/workbench-120x38.png
tui/scripts/shot.sh usage 120x38
tui/scripts/shot.sh chat 90x24
tui/scripts/shot.sh chat 60x20
tui/scripts/shot.sh help 120x38
tui/scripts/shot.sh palette 120x38

# deterministic, no agent, two runs byte-identical (the repository's own path)
python3 scripts/screenshots/capture.py tui
```

Use both: `capture.py tui` is the gate that a diff means "the UI changed" and
works with no credentials; the live recipe is what proves the panels are reading
this machine's real stores.

Whole-goal acceptance (what `verification.md` records): every row of its §1
table has a frame or a command, `docs/tui/assets/` holds ≥ 8 PNGs, and the four
P0 items above are closed with their tests.

## 10. Open decisions

1. **P0-3**: wire the five navigation commands, or delete them from `COMMANDS`?
   Recommendation: wire `/usage` and `/skills` (three-line match arms), delete
   `/export` and `/transcript` until they write a file, and make `/workspace`
   open a picker or leave it out.
2. **P1-3**: overlay pickers or a 6th sidebar tab for worktree/sandbox? Overlays
   are closer to the deleted screens; a tab is closer to dsh's panel model.
3. **P2 bilingual**: is Chinese-only chrome a deliberate product stance? If yes,
   parity row 23 closes as a decision rather than a gap.
4. Is the 36-column default rail right for a 120-column terminal? It leaves 83
   cells of chat; dsh's eight panels split at about 28-32. Measure before
   changing the constant.
