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
7. Themes: two built-in 256-colour palettes, `future-dark` / `future-light`,
   selectable with `/theme`.
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
tui/src/workbench/theme.rs      // Palette, the two themes, semantic colours
tui/src/workbench/widgets.rs    // borders, tabs, progress bars, badges, pixel logo, spinner
tui/src/workbench/layout.rs     // Rect, Panes, hit regions (HitMap)
tui/src/workbench/mouse.rs      // SGR 1006 parsing, MouseEvent, wheel/drag state machine
tui/src/workbench/launchpad.rs  // first screen
tui/src/workbench/screen.rs     // full-screen composition: header/body/sidebar/input/status
tui/src/workbench/panels.rs     // rendering of the 4 sidebar panels (pure functions over PanelData)
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

| Semantic | future-dark | future-light | Use |
|---|---|---|---|
| bg | 233 | 254 | canvas |
| panel | 235 | 252 | sidebar / input background |
| border | 238 | 246 | borders, dividers |
| fg | 252 | 236 | body text |
| muted | 245 | 243 | secondary notes |
| accent | 80 (teal) | 30 | brand, selection, progress |
| iris | 111 | 62 | links, active tab |
| amber | 179 | 130 | thinking, warning |
| green | 114 | 28 | success |
| red | 203 | 124 | error, danger |
| purple | 141 | 91 | sub-agent / skills |

Logo: a 9x4 pixel whale (using the three block levels `█ ▀ ▄`), swishing its tail
slightly at 8fps at runtime (the demo may keep it static). The wordmark
`FUTURE OS` uses the 5-row block font and is shown only at width ≥ 96; otherwise
single-line text.

---

## 4. Interaction (mouse + keyboard)

### Mouse (SGR 1006; enable `?1000h ?1002h ?1006h`, optionally `?1003h` for hover)

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
    pub overlay: Option<Overlay>,      // palette|help|panel-full
}

pub enum WbEvent { Key(KeyEvent), Mouse(MouseEvent), Resize(u16,u16), Tick }
pub enum WbAction {
    Submit(String), Command(String), Scroll(i32), LoadOlder,
    SwitchSession(String), SetModel(String), SetEffort(String),
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
    --scenario launchpad|chat|todo|help --width 120 --height 38 [--ansi|--plain]
cargo run -p future-tui --example workbench_demo -- --self-test
```

- `--plain`: the text frame with ANSI stripped, for grep assertions.
- `--self-test`: builds 120x38 and 90x24 frames and asserts the hit regions:
  clicking (id=sidebar.todo) returns `SetPanel(Todo)`; dragging the divider
  returns `ResizeSidebar`; the wheel returns `Scroll`; clicking the input box
  returns `FocusPane(Input)`. When everything passes it prints `self-test ok`.
- Acceptance commands (every worker runs these before delivering):
  `cargo fmt --check -p future-tui` (if the repo does not enforce it, fmt only
  the new files), `cargo build -p future-tui`, `cargo test -p future-tui
  workbench`.

Screenshots: `tui/scripts/shot.sh <scenario> <out.png>` runs the demo in tmux →
`capture-pane -e` → renders a PNG (the script may depend on `tools/ansi2png.py`
in the workspace; the path is overridable via `SHOT_PY`).

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
