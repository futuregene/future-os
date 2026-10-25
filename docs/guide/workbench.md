# Workbench — the second TUI

`future-workbench` is a full-screen terminal UI for FutureOS. It is a **second
TUI**, not a mode of `future-tui`: the inline TUI appends rows into the
terminal's scrollback and diffs the tail, while the workbench owns the screen,
repaints absolute rows and resolves mouse clicks against what it painted.
Nothing in this program runs through `app.rs`.

What exists today is the **surface**: the bands, the sidebar, the launchpad, the
overlays, the mouse and the keyboard, painted from deterministic demo data. The
agent adapter (filling the same data from `get_state` / `list_sessions` / the
event stream) is the next slice; the design contract is
[`docs/tui/workbench-spec.md`](../tui/workbench-spec.md).

## Running it

```bash
# the surface, on demo data (the header says "demo")
cargo run -p future-tui --bin future-workbench -- --scenario chat

# one frame at any size, for a screenshot or a layout check
cargo run -p future-tui --example workbench_demo -- --scenario launchpad --width 120 --height 38
cargo run -p future-tui --example workbench_demo -- --scenario chat --plain     # ANSI stripped

# the mouse acceptance check (hit regions at 120x38 and 90x24)
cargo run -p future-tui --example workbench_demo -- --self-test
```

Scenarios: `launchpad` (no session yet), `chat` (a conversation with the work
folded into one summary row), `todo` (a long goal list, to see the sidebar
window and its `↓ N more` row).

## The frame

```
 header (5 rows: pixel whale, wordmark at width ≥ 96, session · workspace ·
 model, backend dot)  ── hairline
 ──────────────────────────────────────────┬──────────────────────
 conversation (bottom-anchored, ↑ 更早)     │ sidebar: 待办 任务 会话
                                            │          用量 技能
 ──────────────────────────────────────────┴──────────────────────
 input box (❯ draft)
 activity: ⠋ 思考 … · 3s · 2 工具                   42 tok/s · 缓存 96%
 status:   model · effort · permission   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 hint:     Esc 回到聊天 · ctrl+b 侧栏 · / 命令 · ? 帮助
```

The launchpad is the *body*, not a dialog over the conversation: with no session
attached the composer fills the surface (logo, wordmark, box, model / effort /
mode / permission chips, shortcuts, recent sessions) and there is no second
input band.

Two breakpoints, both from the contract:

- **width < 88**: the sidebar is dropped; `ctrl+b` opens the current panel
  full-screen instead.
- **height < 24**: the header collapses to one row, the input box keeps only its
  top rule, and the activity row is hidden.

Every frame is exactly `height` rows of exactly `width` display columns (CJK and
emoji counted as two), at any size including 1×1 — that invariant is asserted in
the frame tests and in `ansi::fit`, the one function that pads.

## Keys and mouse

| Key | Action |
|---|---|
| `tab` / `shift+tab` | Launchpad: move between the prompt and the parameter chips. Conversation: move focus to / from the sidebar |
| `←` `→` | Move the draft cursor |
| `↑` `↓` | Scroll the conversation (or the focused panel's list) |
| `pageUp` / `pageDown` | Scroll the conversation a page |
| `ctrl+b` | Sidebar: show + focus, then unfocus, then hide; full-screen on a narrow terminal |
| `/` | Command palette (on an empty draft) — type to filter, `enter` to run |
| `?` | Help overlay (keys + commands) |
| `ctrl+e` | Ask the app for the final-answer / detailed reading level |
| `ctrl+c` | Quit |

| Mouse | Action |
|---|---|
| Click a sidebar tab | Switch panel |
| Click a session row | Open that conversation |
| Drag the divider | Resize the sidebar (30–48 columns), persisted on release |
| Click anywhere in the input box | Focus it |
| Wheel over the conversation | Scroll; at the top it asks for an older page |
| Wheel over the sidebar | Scroll the panel |

Mouse reporting is SGR 1006 and is enabled by this program only; the inline TUI
never turns it on.

## What is not here yet

- **The agent adapter.** Today's data comes from `workbench::demo`. The next
  slice fills `WorkbenchData` from the agent (session, usage, models, skills,
  todos) and submits prompts — the surface's side of that contract is already
  fixed: it paints from `WorkbenchData` and returns `WbAction`s.
- **`future wb`** as a subcommand of the unified `future` binary, and the
  decision about whether the workbench becomes the default terminal entry.
- P1 from the contract: timeline rail, two-column diff card, image preview,
  full-screen session manager, full-screen draft editor.

## Where the code lives

| File | Responsibility |
|---|---|
| `tui/src/bin/workbench.rs` | The entry point: raw mode, alternate screen, event loop, applying actions |
| `tui/src/workbench/mod.rs` | `Workbench` (view state), `WbEvent` / `WbAction`, key and mouse dispatch |
| `tui/src/workbench/screen.rs` | Frame composition: the bands, splicing partial bands into a row, overlays |
| `tui/src/workbench/layout.rs` | Band rectangles, the two breakpoints, the hit map |
| `tui/src/workbench/ansi.rs` | Width-safe text: `fit`, `pad`, `clip`, column slicing |
| `tui/src/workbench/widgets.rs` | Rules, boxes, tabs, bars, badges, spinner, logo, wordmark, the input box |
| `tui/src/workbench/panels.rs` | The five sidebar panels (pure renderers) |
| `tui/src/workbench/launchpad.rs` | The first screen |
| `tui/src/workbench/state.rs` | `WorkbenchData` and its parts |
| `tui/src/workbench/mouse.rs` | SGR 1006 parsing |
| `tui/src/workbench/theme.rs` | `future-dark` / `future-light` |
| `tui/src/workbench/demo.rs` | Deterministic demo data (screenshots, tests) |
