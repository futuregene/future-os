# FutureOS Terminal UI (TUI)

`future tui` is a full-screen terminal UI: it owns the screen, repaints absolute
rows and resolves mouse clicks against what it painted. It replaced the earlier
inline TUI (which appended rows into the terminal's scrollback and diffed the
tail); that UI, its code and its page are gone, not deprecated.

The design contract is [`docs/tui/workbench-spec.md`](../tui/workbench-spec.md).

## Running it

```bash
future tui                          # the terminal UI, attached to your agent
future tui --session <id>           # open a specific conversation
future tui --continue               # the most recently active one
future tui "look at this failing test"   # send a first message on start
future tui -p "what is this error?"      # one question, no surface

# the surface alone, on deterministic demo data (no agent; the header says demo)
cargo run -p future-tui --example workbench_demo -- --scenario chat

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

## What is wired to the agent

The adapter is `tui/src/workbench/live.rs`: it paints from `WorkbenchData` and
applies `WbAction`s, and it reuses the inline TUI's leaves rather than
reimplementing them — `ChatArea` for the conversation (markdown, tool rows,
thinking, the final-answer fold), `session_rows::rows_to_messages` for history,
`GrpcClient` for every call.

| Works | Notes |
|---|---|
| Session attach, `--session`, `--continue` | `get_state` + `switch_session` |
| History with paging | The newest page on attach, older pages when the top is reached (`get_session_entries`, a bounded backward cursor) |
| Prompting and streaming | `prompt` with the coalescing policy; `text_chunk`, `thinking_*`, `tool_*` and `agent_end` drive the transcript |
| Approvals | A parked request shows on the activity row; `y` / `n` answer it. `get_state`'s `pendingApprovals` brings it back after a restart, and `approval_decision` clears it whichever client answered |
| Sessions panel | `list_sessions`, newest first, click to switch |
| Usage / context / cost | `get_state`'s usage block, refreshed on the same events |
| `ctrl+n` / `/new`, `ctrl+x` stop, `ctrl+k` refresh | |
| Launchpad chips | Click 模型 / 强度 / 权限 / 模式 to cycle them on the agent; the first message from the launchpad creates the session |
| Theme, reading level | Persisted in `~/.future/tui/settings.json` (`tui/src/settings.rs`) |

## What is not here yet

- **Todo / jobs panels.** The goal plane is a local CLI (`future goal`) and the
  workbench does not read it yet; the panels say "no todos" rather than
  inventing rows.
- **Skills panel** lists the session's loaded skills by name only (no browser,
  no install).
- **Image attachments and `@` file mentions** — the prompt path has no
  attachment support yet; `tui/src/paste.rs` (folded pastes, clipboard capture)
  is the machinery to build it on.
- **The command palette** lists the commands the adapter implements. The panels
  the inline TUI had for providers, tools, sandbox, skills and worktrees have no
  equivalent yet; the underlying settings are per-session agent state, so the
  desktop app or `future` CLI is the way to change them today.
- P1 from the contract: timeline rail, two-column diff card, image preview,
  full-screen session manager, full-screen draft editor.

## Where the code lives

| File | Responsibility |
|---|---|
| `tui/src/workbench/run.rs` | The driver `future tui` runs: terminal, event loop, keys that need the agent first (approvals, stop) |
| `tui/src/workbench/live.rs` | The agent adapter: `WorkbenchData` in, `WbAction`s applied |
| `tui/src/index.rs` | Arguments, `--help`, print mode, `--list-models`; interactive goes to the driver |
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
| `tui/examples/demo_agent.rs` | The scripted gRPC agent screenshots talk to |

## Screenshots

The terminal UI is captured for real, like the desktop and mobile apps: the
shipped binary runs in a pty of a fixed size against a scripted agent, and the
bytes it writes are replayed into a PNG.

```bash
cargo build -p future-cli --bin future
cargo build -p future-tui --example demo_agent
python3 scripts/screenshots/capture.py tui tui-chat.png     # → .screenshots/tui-chat.png
```

The full recipe, the flags and the two honest limits are in
[screenshots.md](screenshots.md) (§8); `scripts/screenshots/tui_terminal.py
--self-test` checks the cell model without a browser.
