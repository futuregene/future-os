# FutureOS Workbench TUI — Design and Implementation Spec v1 (frozen contract)

> Goal: on top of the existing `future-tui` (Rust, hand-rolled ANSI rendering,
> gRPC client), add a **full-screen workbench** interface that matches the core
> capabilities of dsh-TUI 0.17 and does better on what FutureOS has (goal /
> multiple sessions / skills / remote). **The agent's gRPC interface is not
> changed.**

This file is the frozen contract between workers: type names, file names, layout
constants, key bindings and acceptance commands may not diverge. The
implementation may extend it, but may not change its meaning.

---

## 1. Scope

### P0 (must land this round, accepted by screenshot)

1. Full-screen layout: Header / Chat / sidebar / input box / activity row /
   status bar / hint row.
2. Mouse: click (switch panel, switch tab, click a row, focus the input, click a
   button), wheel (chat / sidebar), drag (the pane divider), hover highlight
   (optional, where the terminal supports it).
3. Launchpad first screen: large logo, large input box, parameter row
   (model / effort / mode / permission, clickable), shortcut entries, recent
   sessions.
4. Sidebar with 4 real panels: todos (goal), sessions, usage, skills.
5. Real metrics in the status bar: model · effort · cwd · git branch · context
   gauge · tokens · cost / cache.
6. `/` command palette (with descriptions, Tab completion), `?` help overlay,
   `/` filter, ↑↓ history.
7. Themes: six built-in 256-colour palettes — `dark`, `light`, `one-dark`,
   `one-light`, `high-contrast`, `dracula` — selectable with `/theme`, which
   opens a picker that previews each one as the highlight moves. The ids are the
   inline TUI's catalogue (`crate::themes::THEME_SPECS`), because
   `settings.json` → `themeId` is one key for both surfaces.
8. **The terminal UI, and it is `future tui`**: the workbench has its own
   full-screen render loop and `future tui` starts it. The previous inline UI is
   **deleted**, not deprecated — its code, its page and its `--legacy` flag are
   gone. It is not a `--layout` switch either: two renderers sharing one output
   stream was the thing that got rejected.

### P1 (skeleton / documentation only this round, completion not required)

Timeline rail (click a turn dot to jump), two-column diff card, collapsible todo
footer, task-completion notification, image preview, full-screen session
manager, `Ctrl+O` detailed/collapsed, full-screen draft editor.

### Non-goals

- No change to `packages/rpc` / proto / the agent's gRPC method signatures.
- No ratatui / crossterm / tui-rs; keep the existing hand-rolled terminal
  backend.
- No rewrite of the markdown / diff / input components' rendering logic; reuse
  the ANSI lines they produce.

---

## 2. File layout (write scopes)

```
tui/src/workbench/mod.rs        // module exports + Workbench state machine + WbEvent/WbAction
tui/src/workbench/ansi.rs       // width/truncate/join/pad (CJK safe)
tui/src/workbench/theme.rs      // Palette, the six themes, semantic colours
tui/src/workbench/widgets.rs    // borders, tabs, progress bars, badges, pixel logo, spinner
tui/src/workbench/layout.rs     // Rect, Panes, hit regions (HitMap)
tui/src/workbench/mouse.rs      // SGR 1006 parsing, MouseEvent, wheel/drag state machine
tui/src/workbench/launchpad.rs  // first screen
tui/src/workbench/screen.rs     // full-screen composition: header/body/sidebar/input/status
tui/src/workbench/panels.rs     // rendering of the 4 sidebar panels (pure functions over PanelData)
tui/src/workbench/picker.rs     // the /model, /theme and /sessions list overlay
tui/src/workbench/state.rs      // WorkbenchData / TodoItem / SessionRow / UsageData
tui/src/workbench/demo.rs       // demo data (for screenshots/self-tests; does not connect to the agent)
tui/examples/workbench_demo.rs  // prints one frame at any size + mouse self-test
tui/src/lib.rs                  // one added line: `pub mod workbench;`
```

`tui/src/workbench/run.rs` is the driver: it owns raw mode, the alternate
screen and the event loop. `index.rs` parses arguments (print mode, `--help`,
`--list-models`) and hands interactive use to the driver; there is no second
surface.

Live integration may only touch: `tui/src/workbench/live.rs`, the fill
functions in `tui/src/workbench/state.rs`, and `run.rs`'s interception of keys
that need the agent first. Existing behaviour in `tui/src/components/**`
**must not** change — their rendered rows are reused, nothing else.

---

## 3. Layout (units: terminal cells)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ row 0..4   HEADER (5)                                                        │
│   x0..9    pixel whale / pixel FUTURE icon (4 rows, 9 cells per row)         │
│   x10..    FUTURE OS  (bold, accent)  ·  workbench                           │
│            session title (dim) · branch/workspace                            │
│   right    ⌘ future-tui vX · backend status dot (●connected)                 │
│ row 4      hairline separator (dim, ─)                                       │
├───────────────────────────────────────────────┬──────────────────────────────┤
│ rows 6..h-8   BODY                            │ SIDEBAR_W = 36 (30..48)      │
│   chat width = W - SIDEBAR_W - 1              │ row 6 tabs: 待办 任务 会话   │
│   x = SIDEBAR_W+1 .. W-1                      │        用量 技能             │
│   2 columns of padding on each side           │ divider x = SIDEBAR_W (│)    │
├───────────────────────────────────────────────┴──────────────────────────────┤
│ rows h-7..h-4  INPUT (4): top border / prompt line / bottom border           │
│   ❯ <draft>                                   right ⛶                        │
│ row h-3        ACTIVITY: ⠋ 思考 3s · 2 工具 · 42 tok/s · 缓存 96%            │
│ row h-2        STATUS: model · effort · ctx bar 6.9k/1.0M 0.7% · ≈¥0.01      │
│ row h-1        HINT: Esc 聊天 · ctrl+b 侧栏 · / 命令 · ? 帮助                │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Width < 88: the sidebar collapses to 0; `ctrl+b` opens a full-screen overlay
  panel; the divider column becomes a 2-column timeline rail.
- Height < 24: HEADER is squeezed to 1 row (wordmark inline), INPUT to 2 rows,
  ACTIVITY is hidden.
- Chat content lays out bottom-up (newest above the input box); scrolling up
  makes an `↑ 更早` badge appear at the top.
- Every width computation must use `ansi::display_width` (CJK/emoji count as 2).

### Visual spec (256 colours)

| Semantic | dark | light | one-dark | one-light | high-contrast | dracula | Use |
|---|---|---|---|---|---|---|
| bg | 233 | 254 | 236 | 231 | 16 | 236 | canvas |
| panel | 235 | 252 | 237 | 254 | 17 | 237 | sidebar / input background |
| border | 238 | 246 | 240 | 250 | 250 | 239 | borders, dividers |
| fg | 252 | 236 | 249 | 236 | 231 | 255 | body text |
| muted | 245 | 243 | 243 | 245 | 246 | 243 | secondary notes |
| accent | 80 (teal) | 30 | 75 | 25 | 51 | 141 | brand, selection, progress |
| iris | 111 | 62 | 73 | 30 | 123 | 117 | links, active tab |
| amber | 179 | 130 | 180 | 130 | 226 | 228 | thinking, warning |
| green | 114 | 28 | 108 | 28 | 46 | 84 | success |
| red | 203 | 124 | 168 | 160 | 196 | 203 | error, danger |
| purple | 141 | 91 | 176 | 91 | 213 | 212 | sub-agent / skills |

The four newer palettes mirror `crate::themes`' colours wherever the inline
`Theme` has the same semantic (`accent`, `fg`, `border`, `dim`, `error`,
`success`, `md_link`, `md_heading`, `md_code`); `bg` and `panel` are the
workbench's own, because a full-screen canvas paints an explicit background
step where the inline theme inherits the terminal's. Ids older workbench builds
wrote (`future-dark`, `future-light`) still resolve, to `dark` and `light`.

Logo: a 9x4 pixel whale (using the three block levels `█ ▀ ▄`), swishing its tail
slightly at 8fps at runtime (the demo may keep it static). The wordmark
`FUTURE OS` uses the 5-row block font and is shown only at width ≥ 96; otherwise
single-line text.

---

## 4. Interaction (mouse + keyboard)

### Mouse (SGR 1006; enable `?1000h ?1002h ?1006h`, optionally `?1003h` for hover)

**The terminal owns the mode, not the workbench.** `Terminal::set_mouse_mode(true)`
is set *before* `start()`, which writes `MOUSE_ON` together with the alternate
screen and bracketed paste; `stop()` writes `MOUSE_OFF` before leaving the
screen, and both the exit failsafe (`restore_terminal_for_exit`) and the panic
hook (`crash.rs`) write it unconditionally — those are the paths that can hand a
mouse-captured terminal back to the shell. It stays off unless a surface asks
for it, so a caller with no mouse targets never takes the reader's selection.

Machine check (tmux): `tmux display-message -p -t <pane> '#{mouse_any_flag}'`
reads `1` while the workbench holds the mode and `0` after it quits.

**A frame must not end in a line feed.** `run::frame_bytes` joins the rows with
`\r\n` and stops: a line feed on the bottom row scrolls the alternate screen up
by one, which hides the header's first row and puts every painted row one cell
above the coordinate the layout assigned it — and the SGR report names the row
the reader *sees*, so every click then lands on the row above its target
(measured at 120x38: clicking a sidebar tab did nothing; clicking the input box
hit the conversation). `workbench::run::tests::a_frame_never_ends_in_a_line_feed`
pins it.

| Target | Behaviour |
|---|---|
| Chat area wheel | 3 rows up/down; reaching the top fires `LoadOlder` |
| Sidebar wheel | scroll the current panel's list |
| Sidebar tab click | switch panel (`待办/任务/会话/用量/技能`) |
| Sidebar row click | session → switch; todo → collapse/expand; skill → insert `/skill` |
| Divider column drag | resize `SIDEBAR_W` (30..48); persist to settings on release |
| Input box click | focus and put the cursor where clicked (byte offset from display width) |
| `⛶` click | open the full-screen draft editor (may be a stub in P1) |
| Tool-card title click | expand/collapse that card (reusing `ctrl+o`'s per-item toggle) |
| Launchpad parameter click | open the matching picker (model/effort/preset/permission) |
| Launchpad shortcut click | run the matching `/command` |
| Hover | highlight tabs/rows/buttons; tooltip after 600ms (when truncated at narrow widths) |

Double-click = two clicks at the same point <400ms apart; triple click = row
selection (P1).

### Pickers (`/model`, `/theme`, `/sessions`)

One overlay component (`picker.rs`) behind all three menus: a titled box with an
incremental-search line (`❯`), group headings (the model list is grouped by
provider, in the agent's order), a `✓` on the row already in force, and the
key hints for that menu on its last row.

| Key / gesture | Behaviour |
|---|---|
| `↑` `↓` | move the highlight, stepping over group headings |
| `pageUp` / `pageDown` | ten rows |
| any other character | joins the search; the list filters and the highlight returns to the top |
| `backspace` | widens the search again |
| `←` `→` | the model picker's reasoning slider (six levels); elsewhere nothing |
| `enter` | apply the highlighted row and close |
| wheel | one row per notch |
| click a row | move the highlight *and* apply: a pointer has no second `enter` |
| `esc` / click outside | close without applying |

The row a picker opens on is the value in force, so `esc` straight after opening
changes nothing. `/theme` previews as the highlight moves — the whole frame
repaints in the palette named — and `esc` restores the palette that was there
when the box opened; only `enter` persists (`settings.json` → `themeId`). The
model picker commits the model **and** the slider's level together (`SetModel` +
`SetThinking`), and its slider row applies the level alone.

`/new` returns to the launchpad instead of creating a session: the header's own
state (connection dot, version) stays, the transcript goes, and the first
message creates the session with whatever model/effort the launchpad shows.

Rows are painted and registered as hit regions from one list and one geometry
(`screen::picker_box` over `box_geometry`), which is what makes a click land on
the row under the pointer rather than one beside it.

### Keyboard

Keep the existing global bindings; add/override:
`ctrl+b` sidebar open/focus/close; `tab` focus within a panel; `shift+tab`
reverse; `ctrl+q` collapse todos; `alt+z` enlarge the sidebar; `/` command
palette; `?` help; `esc` close one layer at a time.

---

## 5. Type contract

```rust
pub struct WorkbenchData {
    pub app_name: String, pub version: String,
    pub connected: bool,
    pub session: SessionMeta,          // id/title/cwd/branch
    pub model: String, pub effort: String,
    pub mode: String, pub permission: String,
    pub usage: UsageData,              // ctx_used/ctx_total/cost/cache/tps
    pub todos: Vec<TodoItem>,          // goal todos
    pub sessions: Vec<SessionRow>,
    pub skills: Vec<SkillRow>,
    pub jobs: Vec<JobRow>,
    pub messages: Vec<String>,         // already-rendered ANSI lines (provided by the live layer)
    pub input: InputView,              // text + cursor byte offset + placeholder
    pub activity: Activity,            // idle|thinking|tool|streaming + detail
    pub scroll: usize,                 // 0 = pinned to the bottom
    pub sidebar: SidebarState,         // panel + width + focus
    pub overlay: Option<Overlay>,      // palette|help|panel-full|picker(kind)
    pub models: Vec<ModelChoice>,      // id/label/provider/context/thinking
    // the picker's own state (kind, query, highlight, slider) lives on the
    // surface, not here: it is what the reader is doing, not what is on screen
}

pub enum WbEvent { Key(KeyEvent), Mouse(MouseEvent), Resize(u16,u16), Tick }
pub enum WbAction {
    Submit(String), Command(String), Scroll(i32), LoadOlder,
    SwitchSession(String), SetModel(String), SetThinking(String),
    SetTheme(String), NewSession,
    SetPanel(PanelId), ResizeSidebar(u16), FocusPane(PaneId),
    ToggleToolCall(usize), PersistSettings(SettingsPatch), Quit, Redraw,
}
```

`Workbench::render(&mut self, d:&WorkbenchData, w:u16, h:u16) -> Vec<String>`:
returns **exactly h rows, each exactly w display columns wide**, as ANSI strings
(padded with spaces when short).
`Workbench::handle(&mut self, e:WbEvent, d:&WorkbenchData) -> Vec<WbAction>`.

---

## 6. Demo / acceptance

`tui/examples/workbench_demo.rs`:

```
cargo run -p future-tui --example workbench_demo -- \
    --scenario launchpad|chat|todo|help --width 120 --height 38 [--ansi|--plain] \
    [--picker model|theme|sessions]
cargo run -p future-tui --example workbench_demo -- --self-test
```

- `--plain`: the text frame with ANSI stripped, for grep assertions.
- `--picker`: the same frame with one of the menus open — a picker can be
  reviewed (and screenshotted) without an agent.
- `--self-test`: builds 120x38 and 90x24 frames and asserts the hit regions:
  clicking (id=sidebar.todo) returns `SetPanel(Todo)`; dragging the divider
  returns `ResizeSidebar`; the wheel returns `Scroll`; clicking the input box
  returns `FocusPane(Input)`; clicking a picker row returns `SwitchSession`.
  When everything passes it prints `self-test ok`.
- Acceptance commands (every worker runs these before delivering):
  `cargo fmt --check -p future-tui` (if the repo does not enforce it, fmt only
  the new files), `cargo build -p future-tui`, `cargo test -p future-tui
  workbench`.

Screenshots: `tui/scripts/shot.sh <scenario> <out.png>` (a size may stand in for
the path: `shot.sh chat 120x38`). It runs the app in a pty of the requested size
and renders the bytes it wrote through `scripts/screenshots/tui_terminal.py`;
without `FUTURE_AGENT_GRPC_ADDR` it uses the scripted `demo_agent`, so the frame
needs no agent, no model and no credentials.

---

## 7. gRPC constraints

Use the existing RPCs read-only. If a field the UI needs is missing from an
existing message, **do not change proto**; mark it *needs adding* in the "gRPC
impact" table of `docs/tui/dsh-tui-parity.md` and give the alternative (e.g.
derive it from `get_state` / session-list events). Confirmed available: session
list, usage/context, models, thinking, tool events, goal (local CLI state),
skills (local directory).

## 8. Experience goals (what the acceptor checks)

1. "What is running right now" is visible at a glance: header activity state +
   activity row + status bar, three layers of information, no flicker at rest.
2. 80% of operations are reachable without moving a hand off the mouse: switch
   panel, switch session, scroll chat, open commands, click a tool card.
3. Neither 120x38 nor 90x24 breaks the layout; CJK text never shifts columns.

---

## 9. Where the side panels read from

Three of the five panels are filled from files this machine owns, not from gRPC
(`tui/src/workbench/local.rs`). Each reader is best-effort: a missing store, a
half-written line or an unreadable file yields fewer rows, never a panic and
never a notice.

| Panel | Source | Rule |
|---|---|---|
| 待办 | `<goal root>/goals/<id>/events.jsonl` | `FUTURE_GOAL_ROOT`, else the nearest ancestor with `.future/goal/goals`, else the git toplevel's — so a session in `.worktrees/<name>` reads the checkout it was cut from. The goal is the most recently active one that has not reached a terminal state. `✔` done · `◉` claimed · `○` open · `✖` blocked/superseded, then `P0`…`P3`, then the text. |
| 技能 | `~/.future/agent/skills`, `~/.agents/skills` | The agent's `get_state` names them; the local read adds the `description:` line (and is the fallback when an agent reports none). |
| 任务 | `<home>/tasks/tasks.db` | Opened `SQLITE_OPEN_READ_ONLY`; a missing database is `no scheduled jobs`, never a newly created store. |

The usage panel composes each row from whole pieces and drops a value that does
not fit (`上下文 18.8k/1.0M 1.9%` → `上下文 18.8k/1.0M` → `上下文 18.8k` → the
label alone). A `…` in the middle of a number reads as a different number, so
nothing here is ever clipped mid-value.

## 10. Screenshots

`docs/tui/assets/`, all rendered from the real compositor:

| File | Size | What it proves |
|---|---|---|
| `workbench-chat.png` | 120×38 | the full frame: header (wordmark, plus `cwd · ⎇ branch` on line 2), sidebar with tabs, divider, framed input box, activity, status, hint |
| `workbench-90x24.png` | 90×24 | still two columns; the header falls back to its text brand; the hairline spans the whole width |
| `workbench-60x20.png` | 60×20 | `< 88`: no sidebar and no dead rail; `< 24`: one-row header, the input box keeps a frame (top edge + sides), the hint row drops whole entries |
| `workbench-panel-todo.png` | 120×38 | 待办 lists the repo's real goal todos (`✔/◉/○/✖`, priority, text) |
| `workbench-panel-skills.png` | 120×38 | 技能 lists the 22 installed skills |
| `workbench-panel-usage.png` | 120×38 | 用量: `上下文` and the numbers are separate pieces, no trailing `…` |
| `workbench-picker-model.png` | 120×38 | `/model` against the live agent: 20 models in 4 provider groups, the reasoning slider, `✓` on the model in force (`1.0M ctx · thinking high` per row) |
| `workbench-picker-theme.png` | 120×38 | `/theme`: six palettes, the frame painted in the one in force (Dracula), `✓` on it |
| `workbench-picker-sessions.png` | 120×38 | `/sessions`: real conversations with workspace and relative age, `✓` on the attached one |

The panel shots come from a live tmux session against an isolated agent
(`FUTURE_HOME` with a copy of the installed skills and the tasks database),
clicked with `tools/click.py`; `tmux display -p '#{mouse_any_flag}'` reads `1`
while it runs and `0` after it quits.

The picker shots come from a live tmux session at 120×38 against the running
agent, opened by typing the command, with the row chosen by a real SGR mouse
click (`tools/click.py`) and the menu reopened afterwards so the shot shows the
clicked value ticked — model and session on throwaway sessions, so nothing else
was switched.
