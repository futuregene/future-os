# Workbench TUI — verification record

What was checked, what the screens show, and what is still wrong. Every row of
§2 names its figure, and every frame in `docs/tui/assets/` was captured from a
**live** `future-tui` against an isolated agent — no mock, no demo data.

- **Build**: `dev-huichen` at `ba916d63e`, `FUTURE_VERSION=0.0.2-9f0a8b76c+local
  cargo build -q -p future-tui`.
- **Code under test**: `tui/src/workbench` — 15 modules, 9,076 lines, 119
  `#[test]`s (picker overlay and the six palettes included).
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

| 图 | File | Size | Source | What it shows |
|---|---|---|---|---|
| 图 1 | `workbench-launchpad.png` | 120×38 | live | the launchpad: whale + block wordmark + tagline, framed composer, parameter chips, recent sessions, empty 待办 |
| 图 2 | `workbench-120x38.png` | 120×38 | live | the full frame: header, chat with a real transcript, rail with the 待办 panel, activity, status, hint |
| 图 3 | `workbench-panel-todo.png` | 120×38 | live | 待办 focused (caret in the tab strip), real goal todos with `✔/○/✖` and priority |
| 图 4 | `workbench-panel-usage.png` | 120×38 | live | 用量: `上下文 128.9k/200.0k 64.4%`, gauge, `成本 ≈¥18.84`, `缓存命中 50%` |
| 图 5 | `workbench-panel-skills.png` | 120×38 | live | 技能: 22 skills with descriptions from both skill roots |
| 图 6 | `workbench-chat-scrolled.png` | 120×38 | live | the chat scrolled up by the wheel: `↑ 更早` badge, keys back in the chat (no caret) |
| 图 7 | `workbench-help.png` | 120×38 | live | `?` help overlay (the key table) |
| 图 8 | `workbench-palette.png` | 120×38 | live | `/` command palette with descriptions |
| 图 9 | `workbench-palette-filter.png` | 120×38 | live | the palette filtered to `theme` |
| 图 10 | `workbench-picker-theme.png` | 120×38 | `capture.py tui` | the `/theme` picker: six palettes, `✓` on the one in force, preview footer |
| 图 11 | `workbench-picker-model.png` | 120×38 | `capture.py tui` | the `/model` picker: reasoning slider, provider headings, `ctx · thinking` per row |
| 图 12 | `workbench-picker-sessions.png` | 120×38 | `capture.py tui` | the `/sessions` picker: search line, session row with cwd and age |
| 图 13 | `workbench-light-120x38.png` | 120×38 | live | the same session after the picker applied `light` |
| 图 14 | `workbench-90x24.png` | 90×24 | live | 90 columns still two-column; the header falls back to the text brand; all six hints |
| 图 15 | `workbench-60x20.png` | 60×20 | live | `< 88`: no rail and no dead column; `< 24`: one-row header, framed two-row input, three hints, no activity row |
| 图 16 | `workbench-panel-full-60x20.png` | 60×20 | live | `ctrl+b` on a narrow terminal: the panel full-screen |
| 图 17 | `workbench-drag-inverted.png` | 120×38 | live | **a defect**: a rightward divider drag left the rail at 42 columns |
| 图 18 | `workbench-dead-usage.png` | 120×38 | live | **a defect**: the palette's `/usage` answers `Unknown command /usage` |
| 图 19 | `workbench-dead-skills.png` | 120×38 | live | **a defect**: the palette's `/skills` answers `Unknown command /skills` |

Figures 10–12 are the repository's own deterministic capture
(`python3 scripts/screenshots/capture.py tui` over `future tui --offline` with a
scripted agent). They are not re-shot here because the picker sheets the whole
frame: the live session adds nothing behind it, and two identical runs of that
tool are byte-identical, which is the stronger artifact.

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
| L7 | The first frame at a known size matches the steady-state frame | 图 2 | on this instance: hairline 5 / spacer 6 / tabs 7 at 3 s, 6 s and 9 s. **One earlier instance of the same build painted its first frame one row high** and repaired it on the next redraw — see §6 | pass with a caveat |

### 2.2 Mouse

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| M1 | A click on a sidebar tab switches the panel | 图 4 | `click.py dv5 7 用量` → `clicked '用量' at col=107 row=7`; the panel header became `用量`, the tab strip shows the caret | pass |
| M2 | A click on a row switches the session / applies the picker row | 图 5 | clicking `技能` (col=115 row=7) switched the panel; a click on the `Light` row of the `/theme` picker (col 27 row 13) applied it and closed the box | pass |
| M3 | The wheel scrolls the chat three rows per event, and reaching the top asks for older history | 图 6 | three wheel-up reports at (20,20): the body changed from `栏高与标签中线…` to `Chrome.test.ts +2…` and `↑ 更早 · page up` appeared | pass |
| M4 | The wheel over the rail scrolls the panel, not the chat | 图 3 | with the rail focused, the same three reports moved the 待办 list and left the chat still | pass |
| M5 | Dragging the divider resizes the rail 30..48 and the edge follows the pointer | 图 17 | press at col 84, drag to col 90, release: the divider **moved to column 78** and the rail became **42** — the edge moved away from the pointer | **fail** (P0-1) |
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
| A6 | Git branch in the header | 图 2 | header line 2 reads `/Users/geilige/future-os` and nothing else; `SessionMeta::branch` is hard-coded empty | **fail** (P0-4) |
| A7 | Session title in the header | 图 2 | line 1 `· 全屏多会话视图设计方案` | pass |
| A8 | The right corner degrades by whole pieces | 图 15 vs 图 2 | `● future-tui v0.0.2-9f0a8b76c+local` at 120 → `● future-tui` at 60; never a half Hanzi | pass |

### 2.5 Command surface: palette, pickers, help

| ID | Requirement | 图 | Evidence | Verdict |
|---|---|---|---|---|
| C1 | `/` opens a filterable palette with descriptions | 图 8 | `/new 新建会话`, `/sessions 浏览并切换会话`, … 12 entries | pass |
| C2 | Typing filters the list live | 图 9 | query `theme` leaves `/theme` selected | pass |
| C3 | Three entries open a typed picker | 图 10, 图 11, 图 12 | `/theme` → six palettes with `✓`, `/model` → provider groups plus a reasoning slider, `/sessions` → searchable session rows; all three open from the palette (`run.rs:249`) and from a typed command | pass |
| C4 | The theme picker previews and can be cancelled | 图 13 | moving the highlight to `Light` repaints the frame (`48;5;235` → `48;5;252`); `esc` restores `48;5;235`; `enter` persists | pass |
| C5 | Every palette entry resolves | 图 18, 图 19 | `/usage` → `Unknown command /usage`, `/skills` → `Unknown command /skills`; same for `/workspace`, `/export`, `/transcript` (`live.rs:694`) | **fail** (P0-3) |
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

| Defect | Evidence | Owner |
|---|---|---|
| The divider drag is inverted | 图 17: press col 84 → drag to col 90 → divider ends at col 78, rail 42 (measured twice, both directions) | P0-1 in [workbench-redesign.md](workbench-redesign.md) |
| Five palette entries answer `Unknown command` | 图 18 (`/usage`), 图 19 (`/skills`); same handler gap for `/workspace`, `/export`, `/transcript` | P0-3 |
| The git branch never reaches the header | 图 2: `SessionMeta::branch` is `String::new()` (`live.rs:553`) | P0-4 |
| The first frame can be composed for a stale height | one instance at 120×38 (this build) painted its first frame one row high — hairline 5, tabs 6, no spacer — and the next redraw repaired it; another instance was correct at 3/6/9 s | P0-2 |
| Screens deleted with the inline UI | `/sandbox`, `/worktree`, `/keymap`, `/copy`, external editor — no workbench surface | P1-3 |
| No hover | `MOUSE_ON` omits `?1003h`; `Move` is dropped | P1-1 |
| No image handling in the compositor | no `is_image_line` equivalent, no Kitty/iTerm2 row preservation; not measured live (the session has no image) | P1-11 |
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

# 4. the frames
export FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5511
export SESSION_ID=20261021-005514-e1ee4ba485bc44c5b2d2ea602cd5cd82
tui/scripts/shot.sh chat 120x38
tui/scripts/shot.sh usage 120x38
tui/scripts/shot.sh chat 60x20

# 5. the picker frame, deterministically and without an agent (→ .screenshots/tui.png)
python3 scripts/screenshots/capture.py tui --scenario palette
```

Rules that make the captures trustworthy:

- a **fresh port** and a **private `FUTURE_HOME`** — never the agent a running
  `future tui` uses;
- a scenario that leaves a **sticky notice** (`/usage`, `/skills`) runs last, or
  in its own instance;
- the pty **is** the size (`shot.sh` opens it before the app starts reading it),
  so `120x38` in the command is the frame — there is no resize step;
- every shot is its **own process**, which is why the sticky-notice rule is about
  reading order rather than cleanup;
- a typed command needs a non-empty draft (a leading space), because `/` on an
  empty draft opens the palette — the same route a reader takes.
