# Workbench TUI — verification record

> **Historical record (2026).** This page records what was checked for a particular
> workbench build. Figures and tables are not the current acceptance suite; run
> the commands in the [TUI guide](../guide/tui.md) and `cargo test -p future-tui`
> for today's behaviour.

What was checked, what the screens show, and what is still wrong. Every row of
§2 names its figure, and every frame in `docs/tui/assets/` was captured from a
**live** `future-tui` against an isolated agent — no mock, no demo data.

- **Build**: `dev-huichen` plus this round's `fix/workbench-defects` (the four
  P0 defects of §6 are fixed there), `FUTURE_VERSION=0.0.2-9f0a8b76c+local
  cargo build -q -p future-tui`.
- **Code under test**: `tui/src/workbench` — 16 modules, 9,737 lines, 129
  `#[test]`s (picker overlay and the six palettes included). All 16 live frames
  below were re-shot from this build in one pass, through `tui/scripts/shot.sh`
  and `scripts/screenshots/capture.py tui`: same header, same fixes, same
  session, one renderer.
- **Session**: `20261021-005514-e1ee4ba485bc44c5b2d2ea602cd5cd82`
  (`全屏多会话视图设计方案`, 2,360 entries, cwd `/Users/geilige/future-os`).
- **Instance**: `FUTURE_HOME=/tmp/dvhome` — copies of `settings.json`,
  `models.json`, `skills/` and a `tasks.db` backup, plus that one session's rows;
  **no credentials**. Agent: `future agent --grpc-addr 127.0.0.1:5511` (same
  version string as the client, which the client enforces).
- **Terminal**: a pty sized per shot (120×38 / 90×24 / 60×20), `TERM=xterm-256color`.
- **Recipe**: `tui/scripts/shot.sh` (see §7), a wrapper over
  `scripts/screenshots/capture.py tui`. Each PNG is the byte stream the TUI wrote
  to its pty, replayed through a terminal cell model by
  `scripts/screenshots/tui_terminal.py`.

## 1. Figure index

> **Stale after the canvas change.** These frames were captured before the
> workbench painted its own canvas, so they show the reader's terminal
> background rather than the palette's. Re-shoot with the recipe in §7 when the
> next verification pass runs; the numbers in the visual spec
> (`docs/tui/workbench-spec.md` §3) are current even where these images are not.

| 图 | File | Size | Source | What it shows |
|---|---|---|---|---|
| 图 1 | `workbench-launchpad.png` | 120×38 | live | the launchpad: whale + block wordmark + tagline, framed composer, parameter chips, recent sessions, empty 待办 |
| 图 2 | `workbench-chat.png` | 120×38 | live | the full frame: header (`⎇ dev-huichen` on line 2), chat with a real transcript, rail with the 待办 panel, activity, status, hint |
| 图 3 | `workbench-panel-todo.png` | 120×38 | live | 待办 focused (caret in the tab strip), real goal todos with `✔/○/✖` and priority |
| 图 4 | `workbench-panel-usage.png` | 120×38 | live | 用量: `上下文 128.9k/200.0k 64.4%`, gauge, `成本 ≈¥18.84`, `缓存命中 50%` |
| 图 5 | `workbench-panel-skills.png` | 120×38 | live | 技能: 22 skills with descriptions from both skill roots |
| 图 6 | `workbench-chat-scrolled.png` | 120×38 | live | the chat scrolled up by the wheel: `↑ 更早` badge, keys back in the chat (no caret) |
| 图 7 | `workbench-help.png` | 120×38 | live | `?` help overlay (the key table) |
| 图 8 | `workbench-palette.png` | 120×38 | live | `/` command palette with descriptions |
| 图 9 | `workbench-palette-filter.png` | 120×38 | live | the palette filtered to `theme` |
| 图 10 | `workbench-picker-theme.png` | 120×38 | live | the `/theme` picker: six palettes, `✓` on the one in force, preview footer |
| 图 11 | `workbench-picker-model.png` | 120×38 | live | the `/model` picker: reasoning slider, provider headings, `ctx · thinking` per row |
| 图 12 | `workbench-picker-sessions.png` | 120×38 | live | the `/sessions` picker: search line, session row with cwd and age |
| 图 13 | `workbench-light-120x38.png` | 120×38 | live | the same session after the picker applied `light` |
| 图 14 | `workbench-90x24.png` | 90×24 | live | 90 columns still two-column; the header falls back to the text brand; all six hints |
| 图 15 | `workbench-60x20.png` | 60×20 | live | `< 88`: no rail and no dead column; `< 24`: one-row header, framed two-row input, three hints, no activity row |
| 图 16 | `workbench-panel-full-60x20.png` | 60×20 | live | `ctrl+b` on a narrow terminal: the panel full-screen |
| 图 17 | `workbench-drag-fixed.png` | 120×38 | live | the divider dragged from column 84 to column 90: the seam lands **on 90** and the rail gives up six columns (36 → 30) — the edge follows the pointer |
| 图 18 | `workbench-usage-panel.png` | 120×38 | live | the palette's `/usage` ran: the rail shows 用量 (`上下文 128.9k/200.0k 64.4%`), the caret marks the focused strip, and the hint row carries **no** `Unknown command` |
| 图 19 | `workbench-skills-panel.png` | 120×38 | live | the palette's `/skills` ran: 技能 panel (`22 个技能`), again with no notice |

Figures 10–12 are the three picker scenarios (`picker-model`, `picker-theme`,
`picker-sessions`), shot live like the rest of the set — the same overlay that the
previous revision captured with the scripted agent, now reproducible through
`shot.sh` and rendered by the same cell-exact renderer.

Every frame in this set was re-shot after the renderer's alignment gate was
fixed; the previous PNGs were rasterized while glyphs were still flowed as text.
The measured difference is in the divider: the longest unbroken vertical run of
the sidebar border went from **3 text rows** (out of 24) to **24**.

## 2. Requirements, one by one

### 2.1 Layout

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| L1 | 120×38 renders every band: header (4-row whale art + hairline), body with rail, framed input, activity, status, hint | 图 2 | rows 1–4 header art, 5 hairline, 6 spacer, 7 tab strip, 8–31 conversation, 32–34 bordered input (`╭─ 输入 …╮`, `│❯ …`, `╰───╯`), 35 spacer, 36 activity `· idle … 缓存 50%`, 37 status, 38 hint — exactly 38 rows | pass |
| L2 | The chat never collides with the rail; the divider is a single column; chat padding is 2 cells | 图 2 | chat text spans columns 4–82, the divider is column 84, the rail spans 85–120 (36 columns) | pass |
| L3 | 90 columns keeps two columns and falls back to the text wordmark | 图 14 | header row 1 reads `FUTURE OS · 全屏多会话视图设计方案`, divider at column 54 (rail 36) | pass |
| L4 | 60×20 degrades: no rail, no empty column, one-row header, framed two-row input, three whole hints, no activity row | 图 15 | header `FUTURE OS · … ● future-tui`, no divider, `╭─ 输入…╮` + `│❯ …│` (sides only), status row, hint `ctrl+b 侧栏 · / 命令 · ? 帮助` | pass |
| L5 | The model label may clip, the numbers do not | 图 15 | `future/deepse… ██████░░░░ ctx 128.9k/200.0k 64.4% · ≈¥18.84` — gauge and all numbers whole | pass (deliberate, §5.4) |
| L6 | A terminal too narrow for a rail still reaches every panel | 图 16 | `ctrl+b` at 60×20 draws 待办 full-screen in a box, with `↓ 2 more` | pass |
| L7 | The first frame at a known size matches the steady-state frame | 图 2 | the driver refreshes the terminal's size once before the loop's first `draw` (`run.rs`, `event_loop`), so the first compose — layout *and* hit map — is built from the size the terminal has now, not the one cached at construction. Measured: nine captures from 300 ms to 3 s at 120×38; the first painted frame (≈1.35 s) already reads hairline 5 / spacer 6 / tabs 7 / activity 36 / status 37 / hint 38, and three runs that resized the pane 120×37 → 120×38 mid-startup each painted the settled frame. The stale window itself is pinned by `refresh_size_sees_a_pane_resized_after_construction` | pass (P0-2, §6) |

### 2.2 Mouse

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| M1 | A click on a sidebar tab switches the panel | 图 4 | `click.py dv5 7 用量` → `clicked '用量' at col=107 row=7`; the panel header became `用量`, the tab strip shows the caret | pass |
| M2 | A click on a row switches the session / applies the picker row | 图 5 | clicking `技能` (col=115 row=7) switched the panel; a click on the `Light` row of the `/theme` picker (col 27 row 13) applied it and closed the box | pass |
| M3 | The wheel scrolls the chat three rows per event, and reaching the top asks for older history | 图 6 | three wheel-up reports at (20,20): the body changed from `栏高与标签中线…` to `Chrome.test.ts +2…` and `↑ 更早 · page up` appeared | pass |
| M4 | The wheel over the rail scrolls the panel, not the chat | 图 3 | with the rail focused, the same three reports moved the 待办 list and left the chat still | pass |
| M5 | Dragging the divider resizes the rail 30..48 and the edge follows the pointer | 图 17 | press at column 84 of 120, drag to column 90, release: the divider **lands on column 90** and the rail goes 36 → 30 — the seam ends under the pointer. Read back from the captured frame (`frame-probe`): `divider = 89 (0-based) → column 90, rail 30`. A leftward drag mirrors it (unit test: 90 → 84, rail back to 36), and past either end the width clamps at 48/30 instead of running off (`the drag clamps and never runs off the frame`) | pass (P0-1, §6) |
| M6 | The terminal owns mouse reporting only while the workbench runs | 图 2 | `tmux display -p '#{mouse_any_flag}'` → `1` while the session runs, `0` after `ctrl+q` exits | pass |
| M7 | A click in the chat returns the keys from the rail | 图 6 | the caret in the tab strip disappears after the click and the wheel then scrolls the chat | pass |
| M8 | Hover | — | `MOUSE_ON` is `?1000h?1002h?1006h`; `MouseKind::Move` is parsed and dropped (`mod.rs:584`) | n/a — no hover yet (P1-1) |

### 2.3 Sidebar

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| S1 | Five tabs, the active one named, the rail persistent at ≥88 columns | 图 2, 图 14 | `待办 任务 会话 用量 技能`; divider at 84 (120 wide) and 54 (90 wide) | pass |
| S2 | 待办 reads the real goal and marks state and priority | 图 3 | `待办  4 待办 / 17 合计`; rows `✔ P0 T2 文档：docs/tui/dsh-tu…`, `✖ P0 T4 文档与截图验收…`; long rows are clipped with `…` (text, never numbers) | pass |
| S3 | 用量 shows whole values and never clips a number | 图 4 | `上下文 128.9k/200.0k 64.4%`, `成本 ≈¥18.84`, `缓存命中 50%`, `输出 —` | pass |
| S4 | 技能 lists the machine's skills with descriptions | 图 5 | `技能  22 个技能`, `/future-slides  Create and revise presentations …` | pass |
| S5 | The panel scrolls and says how much is left | 图 16 | `↓ 2 more` at the rail's foot | pass |
| S6 | The focused rail is distinguishable from the unfocused one | 图 2 vs 图 3 | the caret `›` appears before the tab strip only while the rail has the keys (`tab` and a chat click both move it) | pass |

### 2.4 Status bar and activity

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| A1 | Model, effort and permission on the left | 图 2 | `future/deepseek-flash · medium · all` | pass |
| A2 | Context gauge, used/total and percent, right-aligned | 图 2 | `████████░░░░ ctx 128.9k/200.0k 64.4%` | pass |
| A3 | Session cost | 图 2 | `· ≈¥18.84` | pass |
| A4 | Cache-hit rate | 图 2 | activity row's right end: `缓存 50%` | pass |
| A5 | Activity is still at rest | 图 2 | `· idle`, no spinner frames across three captures 1.5 s apart | pass |
| A6 | Git branch in the header | 图 2 | header line 2 reads `/Users/geilige/future-os · ⎇ dev-huichen` — the session's own cwd, then the branch read there (`workbench/git.rs`: `git -C <cwd> rev-parse --abbrev-ref HEAD`, 600 ms deadline, cached per cwd, empty in a non-repository so the line degrades to the bare path) | pass (P0-4, §6) |
| A7 | Session title in the header | 图 2 | line 1 `· 全屏多会话视图设计方案` | pass |
| A8 | The right corner degrades by whole pieces | 图 15 vs 图 2 | `● future-tui v0.0.2-9f0a8b76c+local` at 120 → `● future-tui` at 60; never a half Hanzi | pass |

### 2.5 Command surface: palette, pickers, help

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| C1 | `/` opens a filterable palette with descriptions | 图 8 | `/new 新建会话`, `/sessions 浏览并切换会话`, … 12 entries | pass |
| C2 | Typing filters the list live | 图 9 | query `theme` leaves `/theme` selected | pass |
| C3 | Three entries open a typed picker | 图 10, 图 11, 图 12 | `/theme` → six palettes with `✓`, `/model` → provider groups plus a reasoning slider, `/sessions` → searchable session rows; all three open from the palette (`run.rs:249`) and from a typed command | pass |
| C4 | The theme picker previews and can be cancelled | 图 13 | moving the highlight to `Light` repaints the frame (`48;5;235` → `48;5;252`); `esc` restores `48;5;235`; `enter` persists | pass |
| C5 | Every palette entry resolves | 图 18, 图 19 | `/usage` and `/skills` focus their rail panel (and open it full-screen below 88 columns) — 图 18/19 show the panel with **no** notice on the hint row, and both frames are byte-identical to the frames a click on the same tab produces (图 4/图 5); `/workspace` answers `工作区 /Users/geilige/future-os · 随会话固定 · 换目录：在目标目录启动 future tui，/sessions 换已有会话`; `/transcript` answers `完整记录：/export 导出整份会话（HTML） · /reply-detail 在聊天里看完整过程`; `/export` runs the agent's HTML exporter and puts the path it wrote on the notice row; `/help` opens the same key table `?` does (it used to resolve to a do-nothing arm — resolving is not the same as doing). `every_palette_entry_has_a_handler` fails when a palette entry is not exactly one of {picker, surface verb, app verb} | pass (P0-3, §6) |
| C7 | A click inside a panel never invents a command | 图 3 | a row of a read-only panel (a todo, a skill, a job) now just takes the keys. It used to return `panel.row.<n>` as a command, which the app had no arm for and answered `Unknown command /panel.row.3` — the same lie as C5, one mouse click away | pass (fixed with P0-3) |
| C6 | `?` opens the key table, `esc` closes one layer | 图 7 | two sections: the keys (including the two mouse rows) and the command list with descriptions; `esc` returned to the chat and the draft was untouched | pass |

### 2.6 Themes

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| T1 | Six 256-colour palettes, ids shared with the inline catalogue | 图 10 | `Dark`, `Light`, `One Dark`, `One Light`, `High Contrast`, `Dracula`; `✓` on the one in force; `themeId` resolves `dark`/`light` (the old `future-dark`/`future-light` spellings still map) | pass |
| T2 | Switching rewrites the whole frame, not parts of it | 图 2, 图 13 | every painted background belongs to one palette; `48;5;235` (dark panel) → `48;5;252` (light panel), text `38;5;236` in light | pass |
| T3 | The choice survives a restart | 图 13 | only `enter`/a row click persists, into `<home>/tui/settings.json` → `themeId`; `theme::resolve` reads it back | pass (code + live preview/cancel) |

### 2.7 Degradation ladder

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| D1 | Width: ≥88 keeps the rail, <88 drops it without a dead column | 图 2, 图 14, 图 15 | rail 36 at 120 and 90; at 60 the chat runs to column 60 | pass |
| D2 | Width: the panels stay reachable when the rail cannot fit | 图 16 | `ctrl+b` full-screen panel | pass |
| D3 | Height: ≥24 keeps activity + six hints; <24 hides activity and keeps three | 图 14, 图 15 | at 90×24 `· idle … 缓存 50%` plus all six hints; at 60×20 no activity row, three hints | pass |
| D4 | Height: the input box keeps a frame at <24 | 图 15 | top edge + `│❯ … │` sides, no bottom edge | pass |
| D5 | Hints drop whole entries, never half a word | 图 15 | `ctrl+b 侧栏 · / 命令 · ? 帮助` — `Esc 回到聊天`, `tab 焦点`, `ctrl+n 新会话` removed whole | pass |
| D6 | CJK never shifts a column | 图 2, 图 14, 图 15 | the divider stays one column at every size; every CJK row's right edge is aligned | pass |

## 3. Frame invariants

- Every row is addressed absolutely (`CSI row;1H`) and the frame never advances
  the cursor (`tui/src/workbench/run.rs:368`), so a frame taller than the
  terminal is clipped rather than scrolling the screen — a scroll would shift
  every painted row a cell above the coordinate an SGR report names, which is
  exactly the failure `M1` depends on being gone.
- The composed frame is exactly `h` rows of exactly `w` display columns
  (`render`'s `debug_assert_eq!(rows.len(), h)`); §2.1/L1 counts the bands at
  120×38.
- The demo's `--self-test` passes at 120×38 and 90×24 (`self-test ok`) — the
  geometry and hit-region gate in CI.

## 4. Machine checks

| Check | Command | Result |
|---|---|---|
| build | `FUTURE_VERSION=0.0.2-9f0a8b76c+local cargo build -p future-tui` | ok |
| fmt | `cargo fmt -p future-tui --check` | clean |
| workbench tests | `cargo test -p future-tui workbench` | **119 passed**, 0 failed |
| crate tests | `cargo test -p future-tui --lib` | **1,013 passed**, 0 failed |
| geometry/hit self-test | `cargo run -q -p future-tui --example workbench_demo -- --self-test` | `self-test ok` |
| clippy | `cargo clippy -p future-tui --all-targets -- -D warnings` | **fails in `future-agent`**, not `future-tui`: `agent/src/session/compaction_ops.rs:79` trips `clippy::nonminimal_bool` on this toolchain. `git diff origin/main -- agent/` is empty, so it is pre-existing and unrelated |
| reproducibility | `tui/scripts/shot.sh usage 120x38` on the same session, twice | the two PNGs are byte-identical, and match 图 4 |

## 5. Deliberate differences from dsh-TUI

These are choices, not gaps. Each would be a regression to "fix" without
deciding it first.

1. **No second renderer and no layout flag.** `future tui` is the workbench;
   `--layout`, `--legacy` and `--classic` do not exist and a test fails if the
   help text names them (`help.rs:96`). dsh keeps an inline mode; FutureOS
   deleted it rather than share one output stream between two renderers.
2. **No subagent, shell-job or plugin panels.** The agent has no subagents and
   the wire has no job list. The 任务 panel shows this machine's *scheduled
   tasks*, which is a different thing and is labelled as such.
3. **Overlays sheet the frame, they do not dim it.** `paint_overlay` paints the
   panel background over the rows it covers (图 7, 图 8, 图 10–图 12): the
   conversation is not visible behind an overlay. dsh dims the backdrop and keeps
   the text under it legible. Ours is cheaper and unambiguous at 60 columns; it
   loses the "you are still in the chat" cue.
4. **The model label may clip; numbers never do.** We lay the gauge out first and
   clip the left label with `…` (图 15). At 60 columns that reads as
   `future/deepse…`.
5. **Chinese-only chrome.** dsh switches language live; the workbench is
   Chinese-only. Whether that is a product stance or a gap is an open decision
   (parity doc §10.3).
6. **`y`/`n` for approvals, no card widget.** A parked approval owns those keys
   and the activity row names the tool (`live.rs:586`); dsh draws an approval
   card with buttons.
7. **No mouse-driven copy.** The workbench holds SGR reporting for the whole
   session, so selecting text needs the terminal's own modifier; dsh implements
   per-pane selection and semantic copy.
8. **One picker for three verbs, not three screens.** Model, theme and sessions
   share one overlay with one keymap; dsh has a separate `/model` page and a
   separate session manager. Fewer surfaces, less to learn, and a click applies
   in both.

## 6. Defects found while verifying

### 6.1 Fixed in this round (the four P0s)

Every row below has a code change, a test that fails without it, and a frame from
a live session at 120×38. The figures named here are the **new** ones: the three
defect frames this record used to show (`workbench-drag-inverted.png`,
`workbench-dead-usage.png`, `workbench-dead-skills.png`) are deleted, because a
picture of a bug that no longer exists is a picture of nothing.

| Defect | What was wrong | Fix | Evidence |
|---|---|---|---|
| The divider drag was inverted (P0-1) | the drag **added** the pointer's displacement to the rail's width, so a rightward drag widened the rail and pushed the seam left: press col 84 → drag col 90 → seam at col 78, rail 42 | the displacement is subtracted (`mod.rs` `handle_mouse`, `Drag`), the grab is measured from the press cell, and the width clamps to 30..48 | 图 17: seam lands on col 90, rail 30. `a_dragged_divider_lands_under_the_pointer` (columns, both directions) and `the_drag_clamps_and_never_runs_off_the_frame` |
| Five palette entries answered `Unknown command` (P0-3) | the palette advertised `/usage`, `/skills`, `/workspace`, `/export`, `/transcript` and the app forwarded all five to the agent, which has no such commands | a verb that names something on screen is answered by the surface (`Workbench::surface_command` → `VerbOutcome`): the two panel verbs focus the rail (full-screen below 88 columns), `/workspace` and `/transcript` say what is actually true, `/export` runs the agent's exporter, `/help` opens the key table; a read-only panel row no longer invents `panel.row.<n>` either | 图 18, 图 19 — and both are **byte-identical** to the click-driven frames 图 4/图 5, i.e. the two routes land in the same state. `every_palette_entry_has_a_handler`, `usage_and_skills_focus_their_panel_and_never_reach_the_agent`, `a_narrow_terminal_gets_the_panel_full_screen`, `workspace_and_transcript_explain_themselves`, `help_opens_the_key_table` |
| The git branch never reached the header (P0-4) | `SessionMeta::branch` was hard-coded `String::new()` (`live.rs`), so the header's second line could only ever show the cwd | `workbench/git.rs` reads `rev-parse --abbrev-ref HEAD` in the session's own cwd (600 ms deadline, `-C`, detached HEAD named `@sha`), cached per cwd and re-read when the workspace moves; the header draws `cwd · ⎇ branch` and degrades to the bare cwd outside a repository | 图 2: `/Users/geilige/future-os · ⎇ dev-huichen`. `the_branch_is_read_once_per_cwd`, `a_failed_read_is_an_empty_branch_not_a_stale_one`, `a_real_repository_names_its_branch` |
| The first frame could be composed for a stale height (P0-2) | the cached terminal size dates from construction and from the last resize notification, so a pane that changed in between produced one frame at the old height that the next redraw repaired | the driver refreshes the size once **before** the loop's first `draw`, so the first compose — layout and hit map together — uses the size the terminal has now | nine captures from 300 ms to 3 s at 120×38: the first painted frame is already the settled one (hairline 5 / spacer 6 / tabs 7); three 120×37 → 120×38 mid-startup runs each painted the settled frame; `refresh_size_sees_a_pane_resized_after_construction` pins the mechanism. Caveat kept on purpose: **not reproduced** on this build in those runs — what changed is that the mechanism that can produce it is gone, not that a frame was caught and repaired |

### 6.2 Still open

| Defect | Evidence | Owner |
|---|---|---|
| Screens deleted with the inline UI | `/sandbox`, `/worktree`, `/keymap`, `/copy`, external editor — no workbench surface | P1-3 |
| No hover | `MOUSE_ON` omits `?1003h`; `Move` is dropped | P1-1 |
| No image handling in the compositor | no `is_image_line` equivalent, no Kitty/iTerm2 row preservation; not measured live (the session has no image) | P1-11 |
| The full-screen panel names no way out | 图 16, and the same box `/usage` opens below 88 columns: the title line is `╭─ 待办 ╮` with no `esc`/`ctrl+b` footer. The keys work and the hint row names `ctrl+b`, so it is not a dead end, but the box should say so itself | P2 (new) |
| A hidden rail stays hidden when the terminal widens | observed; `ctrl+b` restores it (hidden → shown+focused). Deliberate: visibility is the reader's choice | by design, recorded here |

## 7. Reproducing this record

```bash
# 1. an isolated data root, no credentials
export FUTURE_HOME=/tmp/dvhome
mkdir -p "$FUTURE_HOME/agent" "$FUTURE_HOME/tasks"
cp ~/.future/agent/{settings.json,models.json} "$FUTURE_HOME/agent/"
cp -R ~/.future/agent/skills "$FUTURE_HOME/agent/skills"
sqlite3 ~/.future/tasks/tasks.db ".backup $FUTURE_HOME/tasks/tasks.db"

# 2. one real session, copied row-by-row (sessions / entries / message_blocks / runs)
#    — see the ATTACH + INSERT script in the parity doc §1. Any completed session works.

# 3. a matching agent on a fresh port (the client refuses a version skew)
FUTURE_VERSION=0.0.2-9f0a8b76c+local cargo build -q -p future-tui -p future-cli
FUTURE_HOME=$FUTURE_HOME ./target/debug/future agent --grpc-addr 127.0.0.1:5511 &

# 4. the frames: one scenario each, all sixteen, from one build
export FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5511
export SESSION_ID=20261021-005514-e1ee4ba485bc44c5b2d2ea602cd5cd82
A=$PWD/docs/tui/assets
tui/scripts/shot.sh launchpad      120x38 $A/workbench-launchpad.png
tui/scripts/shot.sh chat           120x38 $A/workbench-chat.png
tui/scripts/shot.sh todo           120x38 $A/workbench-panel-todo.png
tui/scripts/shot.sh usage          120x38 $A/workbench-panel-usage.png
tui/scripts/shot.sh skills         120x38 $A/workbench-panel-skills.png
tui/scripts/shot.sh scrolled       120x38 $A/workbench-chat-scrolled.png
tui/scripts/shot.sh help           120x38 $A/workbench-help.png
tui/scripts/shot.sh palette        120x38 $A/workbench-palette.png
tui/scripts/shot.sh palette-filter 120x38 $A/workbench-palette-filter.png
tui/scripts/shot.sh picker-model    120x38 $A/workbench-picker-model.png
tui/scripts/shot.sh picker-theme    120x38 $A/workbench-picker-theme.png
tui/scripts/shot.sh picker-sessions 120x38 $A/workbench-picker-sessions.png
tui/scripts/shot.sh chat           90x24  $A/workbench-90x24.png
tui/scripts/shot.sh chat           60x20  $A/workbench-60x20.png
tui/scripts/shot.sh full-panel     60x20  $A/workbench-panel-full-60x20.png
tui/scripts/shot.sh drag           120x38 $A/workbench-drag-fixed.png
tui/scripts/shot.sh command-usage  120x38 $A/workbench-usage-panel.png
tui/scripts/shot.sh command-skills 120x38 $A/workbench-skills-panel.png
tui/scripts/shot.sh light          120x38 $A/workbench-light-120x38.png  # persists the palette: last

# 5. the picker frame, deterministically and without an agent (→ .screenshots/tui.png)
python3 scripts/screenshots/capture.py tui --scenario palette
```

Every scenario name above is a key of the table in `scripts/screenshots/capture.py`
(`TUI_SCENARIO_KEYS`, `TUI_SCENARIO_TABS`, `TUI_SCENARIO_DRAGS`); an unknown name
is an error, not a silently different frame. The two panel scenarios (`usage`,
`skills`) click the tab — the workbench has no key for it — and the two
`command-*` ones run the palette entry, which is the path §2.5/C5 is about. `A`
is absolute on purpose: `shot.sh` resolves a relative output under its own
`--out` directory, which would nest `docs/tui/assets/` inside itself.

Rules that make the captures trustworthy:

- a **fresh port** and a **private `FUTURE_HOME`** — never the agent a running
  `future tui` uses;
- a scenario that leaves a **sticky notice** runs last, or in its own instance —
  no palette entry leaves one now, so a notice in a frame means something went
  wrong;
- the `light` scenario runs **last** and its palette is reset afterwards
  (`rm -f "$FUTURE_HOME/tui/settings.json"`): applying a palette persists
  `themeId`, and every later shot would inherit it;
- the pty **is** the size (`shot.sh` opens it before the app starts reading it),
  so `120x38` in the command is the frame — there is no resize step;
- every shot is its **own process**, which is why the sticky-notice rule is about
  reading order rather than cleanup;
- a typed command needs a non-empty draft (a leading space), because `/` on an
  empty draft opens the palette — the same route a reader takes.
