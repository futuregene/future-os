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
   (workspace / model / effort / approval, clickable), shortcut entries, recent
   sessions.
4. Sidebar with 2 real panels: recent conversations (name, workspace, age) and
   the same conversations filed by workspace (pinned / workspaces / chats).
   **文件** is not a third tab: it is the session workspace as its own overlay
   (`/files`, or the 文件 button in the header; no chord — the ctrl-only map has
   no letter left for it) — one level at a
   time, a directory read when it opens, `↑`/`↓` to walk, `enter` to open a
   directory or preview a file, `backspace` to close a directory or walk up to
   the one holding the row. It is scoped to the conversation on screen, the way
   `/git` is, and re-reads when the session moves. Its rows are the same
   `HitMap` rows the rail registers, so a click opens what it landed on.
5. Real metrics in the status bar: model · effort · cwd · git branch · context
   gauge · tokens · cost / cache.
6. `/` command palette (with descriptions, Tab completion), `?` help overlay,
   `/` filter, ↑↓ history.
7. Themes: built-in 256-colour palettes — the six the contract shipped (`dark`,
   `light`, `one-dark`, `one-light`, `high-contrast`, `dracula`) plus the
   desktop's palette family (`ayu`, `catppuccin`, `everforest`, `gruvbox`,
   `kanagawa`, `nightfox`, `nord`, `rose-pine`, `solarized`, `tokyo-night`,
   `vesper`, and `-light` renditions for five of them) — selectable with
   `/theme`, which opens a picker that previews each one as the highlight moves.
   The ids are the inline TUI's catalogue (`crate::themes::THEME_SPECS`),
   because `settings.json` → `themeId` is one key for both surfaces.
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
tui/src/workbench/panels.rs     // rendering of the 2 sidebar panels (pure functions over a row model)
tui/src/workbench/picker.rs     // the /model, /thinking, /mode, /workspace, /theme, /sessions and /lang list overlay
tui/src/workbench/files.rs      // the 文件 panel's tree, and what a path previews as
tui/src/workbench/syntax.rs     // the preview box's code colouring: language, tokens, runs
tui/src/workbench/image_view.rs // an image as half blocks: fit, zoom, pan
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

The labels in this drawing are the Chinese bundle; the surface ships Chinese and
English and draws its chrome in the reader's language (`i18n.rs`, `/lang`).
Content — a conversation, a session title, a path — is never translated.

## 3. Layout (units: terminal cells)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ row 0..4   HEADER (5)                                                        │
│   x0..     FUTURE OS  (bold, accent)  ·  workbench                           │
│            session title (dim) · branch/workspace                            │
│   right    ⌘ future-tui vX · backend status dot (●connected)                 │
│ row 4      hairline separator (dim, ─)                                       │
├───────────────────────────────────────────────┬──────────────────────────────┤
│ rows 6..h-4   BODY                            │ SIDEBAR_W = 36 (30..48)      │
│   chat width = W - SIDEBAR_W - 1              │ row 6 tabs: 最近 工作区      │
│   x = SIDEBAR_W+1 .. W-1                      │ divider x = SIDEBAR_W (│)    │
│                                               │ …and the rail runs on down   │
│ rows h-6..h-4  INPUT (3): border / prompt / border, beside the rail          │
│   ❯ <draft>                                   right ⛶                        │
├───────────────────────────────────────────────┴──────────────────────────────┤
│ row h-3        ACTIVITY: ⠋ 思考 3s · 2 工具 · 42 tok/s · 缓存 96%            │
│ row h-2        STATUS: model · effort · 审批模式 · ctx bar · ≈¥0.01          │
│ row h-1        HINT: Esc 聊天 · ctrl+d 侧栏 · ctrl+r 会话 · ctrl+p 命令      │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Width < 88: the sidebar collapses to 0; `/recent` opens a full-screen overlay
  panel; the divider column becomes a 2-column timeline rail.
- Height < 24: HEADER is squeezed to 1 row (wordmark inline), INPUT to 2 rows,
  ACTIVITY is hidden.
- Chat content lays out bottom-up (newest above the input box); scrolling up
  makes an `↑ 更早` badge appear at the top.
- Every width computation must use `ansi::display_width` (CJK/emoji count as 2).

### Visual spec (256 colours)

| Semantic | dark | light | one-dark | one-light | high-contrast | dracula | Use |
|---|---|---|---|---|---|---|
| bg | 232 | 254 | 232 | 231 | 16 | 232 | canvas |
| panel | 234 | 252 | 234 | 254 | 17 | 234 | menu, picker and overlay fill (the rail, the chat and the composer keep the canvas) |
| border | 238 | 246 | 240 | 250 | 250 | 239 | borders, dividers |
| fg | 252 | 236 | 249 | 236 | 231 | 255 | body text |
| muted | 245 | 240 | 245 | 241 | 246 | 245 | secondary notes |
| accent | 80 (teal) | 30 | 75 | 25 | 51 | 141 | brand, selection, progress |
| iris | 111 | 62 | 73 | 30 | 123 | 117 | links, active tab |
| amber | 179 | 130 | 180 | 130 | 226 | 228 | thinking, warning |
| green | 114 | 28 | 108 | 28 | 46 | 84 | success |
| red | 203 | 124 | 168 | 160 | 196 | 203 | error, danger |
| purple | 141 | 91 | 176 | 91 | 213 | 212 | sub-agent / skills |

The table above is the six palettes the contract froze, and those numbers are
still binding for those ids (the acceptance screenshots were taken against
them). The palettes added afterwards — the desktop's own family and the light
renditions of five of them — are the same `Palette` shape in
`tui/src/workbench/theme.rs` (`PALETTES`, one entry per id in
`crate::themes::THEME_SPECS`), with their hues taken from
`desktop/src/theme/palettes.json` and their backgrounds stepped the same way.

Two rules hold across the whole table, pinned by
`workbench::theme::tests::text_roles_clear_aa_on_their_own_canvas` and
`every_dark_canvas_is_the_same_near_black`:

- **The canvas is the surface's, and it is black.** The frame paints `bg` on
every row to the last column (`workbench::run::canvas_rows`). It used to paint
only the panels, so the canvas was whatever the reader's terminal profile drew
— Ghostty ships `#282c34`, a mid slate — and every dark theme read washed out
regardless of the palette. Every dark palette now uses the same near-black
`232` with `panel` one step above (`234`); `high-contrast` keeps true black
(`16`/`17`). A theme's identity is its hues, not a lighter canvas.
- **Every text role clears WCAG AA (4.5:1)** against both the canvas and the
panel, and `fg` clears 7:1 on the canvas. This is what lifted `muted` — at
2.9:1 on `one-dark`'s and `dracula`'s own panel it was decoration rather than
text — and, where a dark palette's accent colour sat below 4.0, that colour too
(`gruvbox` iris, `nord` red, `solarized` purple).

The **light** palettes keep their hues because the numbers are shared with the
desktop app's table; only their `fg`/`muted` were lifted. Four of them still
carry an accent below AA on their own canvas, reported rather than silently
changed: `light` accent 30 (2.8:1), iris 62 (3.3:1), amber 130 (3.1:1), green
28 (3.1:1); `one-light` iris 30 (3.4:1), amber 130 (3.7:1), green 28 (3.7:1);
`catppuccin-light` iris 31 (3.6:1), purple 99 (3.5:1); `everforest-light` iris
66 (3.4:1), green 64 (3.7:1), red 131 (3.9:1), purple 132 (3.8:1);
`gruvbox-light` iris 66 (2.9:1), amber 94 (4.3:1), purple 95 (4.1:1);
`rose-pine-light` red 131 (3.9:1), purple 99 (3.5:1); `solarized-light` iris 30
(3.4:1), purple 61 (4.4:1). Raising them means editing the desktop's palette
table as well, which is a separate change.

Every palette mirrors `crate::themes`' colours wherever the inline
`Theme` has the same semantic (`accent`, `fg`, `border`, `dim`, `error`,
`success`, `md_link`, `md_heading`, `md_code`), except where the inline value
was below AA on this surface's canvas and the workbench lifted it (`muted`,
and three dark palettes' accent colours). `bg` and `panel` are the workbench's
own, because a full-screen canvas paints an explicit background step where the
inline theme inherits the terminal's. Ids older workbench builds
wrote (`future-dark`, `future-light`) still resolve, to `dark` and `light`.

There is no logo or icon in the TUI. The chat header's wordmark `FUTURE OS`
uses the 5-row block font and is shown only at width ≥ 96; otherwise it is
single-line text. The launchpad instead centres a two-row, letterspaced name
and tagline in the conversation column, keeping the input as the visual focus.

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
| Chat link / picture click | open it: a URL in the browser, a `futureos://session/…` link switches conversation, a local file opens in the preview box |
| Launchpad parameter click | open the matching picker (workspace / model / effort / approval) |
| Launchpad shortcut click | run the matching `/command` |
| Hover | highlight tabs/rows/buttons; tooltip after 600ms (when truncated at narrow widths) |

Double-click = two clicks at the same point <400ms apart; triple click = row
selection (P1).

### Pickers (`/model`, `/thinking`, `/mode`, `/workspace`, `/theme`, `/sessions`, `/lang`)

One overlay component (`picker.rs`) behind all seven menus: a titled box with an
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

A launchpad with nothing staged names the **product defaults**, not the values of
the conversation `/new` left behind: the plain chat (no workspace — the agent's
own `agent/workspace`, the directory the rail's 对话会话 section files, created on
the way), the agent's default model and the level that model starts at (both read
from `list_models`: `isDefault`, `defaultThinkingLevel` — the same resolution
`new_session` performs), and 完全访问 (`off`). Carrying the previous
conversation's directory, model and approval into the next one made `/new` a copy
of what came before instead of a fresh start — and the default approval is
applied to the created session rather than only named, so a chip never names a
mode the session will not run under.

With no session attached — the launchpad, or a TUI started against an agent that
has none — `enter` in a picker **stages** the choice instead of sending it: the
agent answers `set_model`/`set_thinking_level` with `session not found — pass a
valid session_id` when nothing is attached, so the pair becomes `SetPending*`.
The header and the launchpad's chips show the staged value, it is written to
`settings.json` as `defaultModel`/`defaultThinkingLevel`, and the first message
asks for the session with it; once that session exists the agent is the only
source and the staging is cleared. The launchpad's workspace, model, effort and
approval chips follow the same rule: `set_cwd` and `set_sandbox_policy` need a
session like the other two setters do, so a staged mode is applied right after
the session exists.

`/thinking`, `/mode` and `/workspace` are the three choosers a reader reaches
without opening the palette: they answer to `ctrl+t`, `ctrl+s` and `ctrl+l` as
well. `/thinking` lists the six levels, `/mode` the desktop composer's four
approval tiers with their one-line explanations, and `/workspace` every
directory this client has a conversation in plus a first row that creates one
from the path typed into its search line (`enter` there is `set_cwd`, the same
verb as choosing an existing one). Under that typed row sits the one row that is
not a directory: 对话会话, a plain chat — the conversation nobody picked a
workspace for. `enter` there parks the session in the agent's own
`agent/workspace` (the directory the rail's 对话会话 section already names), or
stages it for the session the next message creates, and the app makes that
directory itself: a directory the reader typed is theirs to have created, and a
typo deserves the agent's error, but this path is nobody's to make. The
approval tiers are the desktop's own
values — manual / sandboxed / automatic review / full access; there is no
separate TUI "permission level" chip, because no other client offers that
choice.

Rows are painted and registered as hit regions from one list and one geometry
(`screen::picker_box` over `box_geometry`), which is what makes a click land on
the row under the pointer rather than one beside it.

### Keyboard

**One prefix: `ctrl`.** Every global default is a `ctrl` chord (or an arrow with
`shift`): `ctrl+o` model chooser; `ctrl+t` thinking-effort chooser; `ctrl+s`
approval-mode chooser; `ctrl+l` workspace chooser; `ctrl+f` the files panel
(this session's own workspace); `ctrl+g` the git browser; `ctrl+d` the rail's
open/focus/hide; `ctrl+q` preferences; `ctrl+v` read the clipboard (an image becomes a draft attachment, text is pasted); `tab`/`shift+tab` step
the conversation's links and pictures (`enter` opens the one under the cursor —
browser, that conversation, or the preview box), and on the launchpad move between
the prompt and its chips; `/` command palette; `?` help; `esc` close one layer at
a time. `shift+←`/`shift+→` walk the rail's two tabs and do the
sidebar's open/focus/hide job when the rail is closed.

No default is `alt` — on macOS Option composes a character unless the reader
turns on "Use Option as Meta" — and the letters left over after the collisions
run out, so the remaining actions have **no chord**: deleting a conversation (`/delete`, or
`delete` in the rail — it asks first), the queue (`/queue` — the prompts sent
while a run was in flight), the interface language (`/lang`), a refresh
(`/refresh`), copy (`/copy`), mouse capture (`/mouse`), the newest attachment
(`/preview`), the rail's two panels (a click, or `shift+←`/`shift+→`), and
reduced motion (the preferences overlay). The
help overlay prints `—` in the key column for those. The collisions are the
reason: a terminal sends Backspace, Tab, `\n` and Enter for `ctrl+h`/`i`/`j`/`m`;
`ctrl+a`/`ctrl+b` are the `screen` and `tmux` prefixes; `ctrl+z` is suspend;
`ctrl+u`/`k`/`w`/`y` are the composer's own editing keys. `shift+←` / `shift+→` walk
the rail's two tabs from anywhere — they show a hidden rail and take the keys —
and `shift+↑` / `shift+↓` switch to the previous / next conversation. A focused
rail spends its last row on exactly those two chords
(`shift+←→ 页签 · shift+↑↓ 切换会话`): `↑↓` alone only moves the list cursor and
`⏎` only opens the row it stands on, so the row that says how to *change* the
conversation is the one thing the rail cannot leave to guesswork. The files
panel is its own box and spends a row on its own set
(`↑↓ 选择 · ⏎ 打开 · ⌫ 上一层 · r 刷新 · esc 关闭`).

No **default** is a function key: `F1`–`F12` need an `Fn` modifier on a laptop
and are absent on a compact keyboard. Rebinding to an F key or to `alt` stays
supported — only the shipped defaults are `ctrl`.
The help overlay lists `ACTIONS` **grouped** (chats / choices / panels /
interface), then the non-rebindable inline keys, then the commands.

A held `shift+↑`/`shift+↓` is one switch, not one per key repeat: the key returns
`WbAction::SessionStep(delta)`, the driver reads the whole input burst and
applies the net step once through `Workbench::step_session`. The rail's cursor
moves with every step — that is the thing the reader is watching — and the
conversation is opened once the keys have been quiet for 80 ms
(`run::OPEN_SETTLE`), so a held key never loads the conversations it passes
through.

The adapter keeps the last eight conversations it showed, rendered rows and all
(`Live`'s cache). A switch to one of them paints those rows *before* it asks the
agent anything, then reconciles with one `get_state` + `get_session_entries` pair
(both addressed by name, so they go out together): a page that comes back
unchanged keeps the rows it is compared against, and one that changed rebuilds
the transcript. That pair replaces the three sequential round trips a switch used
to take — `switch_session`, `get_state`, then the page — because `get_state`
already hydrates the session it is asked about.

### Pictures in a text surface

An image is drawn as **half blocks**: one `▀` per cell, foreground the top pixel
of the pair and background the bottom one. That needs colour and nothing else, so
it works in any 256-colour terminal — no kitty graphics, no iTerm2 images, no
sixel — and 24-bit colour is used when the terminal reports it. It is a
*preview*: a couple of hundred cells is a thumbnail of a photo and a legible
screenshot.

Decoding lives in the adapter (`image`, the same crate the agent, host and
desktop link) and happens once per file, downscaled to 1024 px on its long edge:
a zoom redraws pixels already in hand. Text previews are read bounded
(`MAX_TEXT_PREVIEW_BYTES`) and cut on a character boundary. A file that cannot be
shown says why in the box rather than opening empty.

A text preview is **coloured by language**. `syntax.rs` classifies every line
into keywords, types, called names, strings, numbers, comments, attributes and
keys; it names tokens, never colours, and `theme::Palette::token` decides what a
token looks like — so highlighted code follows the theme in force instead of
arriving in a dark highlighter's palette on a light canvas. The classification
happens once, when the file is opened, and each line travels with its runs.

It is deliberately not a parser. What a reader needs is the visible shape of a
line: comments, literals (including Rust raw strings, Python docstrings and
JavaScript template literals that run past their line) and declarations, at the
cost of one forward walk per line and one bit of state between lines. An
unrecognised extension is painted exactly as it came rather than guessed at.
Tabs are expanded to four spaces: a literal `\t` in a painted row would move the
cursor to the terminal's own tab stop and tear the frame's columns.

---

## 5. Type contract

```rust
pub struct WorkbenchData {
    pub app_name: String, pub version: String,
    pub connected: bool,
    pub session: SessionMeta,          // id/title/cwd/chat/branch
    pub model: String, pub effort: String,
    pub mode: String,                  // manual|sandbox|auto|off (the desktop's four tiers)
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
    pub tree: FileTree,                // the 文件 panel: rows + which directories are open
    pub preview: Option<PreviewView>,  // what the preview box is holding (pixels / text / why not)
    // the picker's own state (kind, query, highlight, slider) lives on the
    // surface, not here: it is what the reader is doing, not what is on screen
}

pub enum WbEvent { Key(KeyEvent), Mouse(MouseEvent), Resize(u16,u16), Tick }
pub enum WbAction {
    Submit(String), Command(String), Scroll(i32), LoadOlder,
    SwitchSession(String), DeleteSession(String), SetModel(String), SetThinking(String),
    SetApproval(String), SelectWorkspace(String),
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
    [--picker /model|/thinking|/mode|/workspace|/theme|/sessions|/lang]
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

> **Pending re-shoot.** The 19 frames below were captured before the canvas was
> painted (`232`/`234` on every row, §3) and before the AA pass on `muted`, so
> they still show the old per-terminal canvas. The colours in §3 are the source
> of truth; re-shoot with the recipe in `docs/tui/verification.md` (§7 there,
> `FUTURE_AGENT_GRPC_ADDR` + `tui/scripts/shot.sh`), not with the demo agent. A
> single `shot.sh chat 120x38` against the new build shows the difference: the
> frame owns its background instead of inheriting the profile's.

| File | Size | What it proves |
|---|---|---|
| `workbench-chat.png` | 120×38 | the full frame: header (wordmark, plus `cwd · @branch` on line 2), sidebar with tabs, divider, framed input box, activity, status, hint |
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
