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

# every page driven by real key and pointer events (regions, frames, states)
cargo run -p future-tui --example workbench_interaction -- --size 120x38
cargo run -p future-tui --example workbench_interaction -- --size 40x12

# …and the behavioural half: does the row under the pointer open *that* row?
cargo run -p future-tui --example workbench_semantics
```

Scenarios: `launchpad` (no session yet), `chat` (a conversation with the work
folded into one summary row), `workspaces` (twelve workspaces and their
conversations, to see the rail's window and its `↓ N more` row).

## The frame

```
 focused header (2 rows: session · workspace · backend dot, then hairline)
 comfortable header (optional 6 rows: wordmark and full identity)
 ──────────────────────────────────────────┬──────────────────────
 conversation (bottom-anchored, ↑ 更早)     │ rail: 最近 工作区
                                            │ ▸ the conversation's name
                                            │   ~/future-os · 2m
 ──────────────────────────────────────────┴──────────────────────
 input box (❯ multiline draft, grows to 8 visual rows)
 activity: ⠋ 思考 … · 3s · 2 工具                   42 tok/s · 缓存 96%
 status:   model · effort · permission   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 hint:     Esc 回到聊天 · ctrl+b 侧栏 · tab 焦点 · ctrl+r 会话 · ctrl+p 命令
```

The rail is a *task* panel, in two tabs. **最近** (the default) lists every
conversation — its name, then the workspace it is filed under and when it was
last refreshed — newest first. A row names its workspace by *name*
(`future-os`), not by path: the rail is 36 columns wide and a clipped path says
less than the word. **工作区** files the same conversations the way the
desktop's rail does: 置顶 (the workspaces pinned with `p`), then 工作区 (one
heading per workspace, its conversations beneath), then 对话会话 (conversations
that belong to no workspace — the app's own scratch directories). Usage figures
live on the status bar, so no panel repeats them; skills are the palette's. With
the rail focused, `←`/`→` move between the two tabs.

The launchpad is the *body*, not a dialog over the conversation: with no session
attached the composer fills the surface (logo, wordmark, box, workspace / model
/ effort / mode / permission chips, shortcuts) and there is no second input
band. The conversations are the rail's — the launchpad does not repeat them.
`tab` walks the prompt and the chips, `enter` cycles the chip under the cursor,
and typing (or `escape`) hands the keys back to the prompt.

Two breakpoints, both from the contract:

- **width < 88**: the sidebar is dropped; `ctrl+b` opens the current panel
  full-screen instead.
- **height < 24**: the header collapses to one row, the input box keeps only its
  top rule, and the activity row is hidden.

Every frame is exactly `height` rows of exactly `width` display columns (CJK and
emoji counted as two), at any size including 1×1 — that invariant is asserted in
the frame tests and in `ansi::fit`, the one function that pads.

## Language

The surface ships Chinese and English, and the chrome — tab labels, pickers, key
hints, empty states, notices — is drawn in the reader's language. What is *not*
translated is content: a conversation, a session title, a file path, an agent's
output stay exactly as written.

It follows the system until the reader picks one, and the pick is remembered in
`~/.future/tui/settings.json` (`language`: `system` | `zh` | `en`, the same
vocabulary the desktop and the phone use; a missing key means `system`).

- `/lang` opens the picker: **Follow system**, **中文**, **English**.
- The locale comes from `LC_ALL`, then `LC_MESSAGES`, then `LANG`; on Windows,
  from the user's UI language when none of them is set. A missing or `C` locale
  is English.
- `cargo run -p future-tui --example workbench_demo -- --lang en` prints a frame
  in either language; the screenshot capture pins `zh` by writing the settings
  file into its scratch home, so a baseline does not depend on who ran it.

The copy lives in one table, `tui/src/workbench/i18n.rs`, with both spellings on
the same line: a key added in one language only does not compile.

## Layout and personalization

The default **focused density** uses a two-row context header, reclaiming four
rows for the conversation. `Ctrl+O` or `/preferences` opens the preference
panel: switch to the comfortable identity header, move the sidebar left or
right, adjust its 30–48-column width, control mouse capture and completion
notifications, enable reduced motion or linear reading, and rebind global
shortcuts. The panel keeps the selected row and help visible on short terminals.
Choices persist in `~/.future/tui/settings.json` as `density`, `sidebarPosition`,
`sidebarWidth`, `sidebarVisible`, `mouseCapture`, `notify`, `reducedMotion` and
`linearReading`; incremental shortcut overrides live beside it in
`keybindings.json`.

`/theme` retains the six-theme picker, including `high-contrast`; choosing a
theme persists `themeId`. With no stored choice, the terminal's `COLORFGBG`
hint selects its light/dark palette (unknown hints use dark). `NO_COLOR`
disables styling by variable presence, including an empty value. `F8` toggles
**linear reading**: plain text is appended to the terminal's native scrollback,
not to an alternate-screen single-column imitation. Entering replays the loaded
conversation once; streaming appends complete lines without repeating tokens,
and the single-line draft prompt redraws in place. The terminal/screen reader's
own scrollback navigation reads history. Complex overlays (diff, history and
preferences) still use the full-screen workflow; a fully linear menu/screen-
reader interaction audit is not complete. `F9`
independently toggles **reduced motion**. Neither delays streamed content or
changes agent behavior. `Ctrl+B` leaves linear mode when explicitly reopening
the sidebar. These bindings, plus copy, queue and mouse, are contextual global
actions and can be rebound; overlays keep their local keys so a global shortcut
never submits an approval or edits history search accidentally.

Notifications are emitted for a successful run submitted by this TUI when no
queued follow-up remains and the terminal reports focus-out. Approval requests
and failures require attention in either focus state; approvals use their real
request id and terminal failures their run id for duplicate suppression.
Cancellations, replays and other clients' runs do not notify. Unsupported focus
reporting is treated as focused, so foreground completions stay quiet.
`notify.enabled` is the master switch;
`notify.bell` independently controls BEL and `notify.osc9` controls the
terminal's desktop-notification escape. Delivery is best-effort and depends on
the terminal; notification payloads are sanitized, flattened and bounded.

`Ctrl+P` opens the command palette, `Ctrl+R` sessions, and `F1` help. Help and
footer hints show the effective bindings, not stale defaults. In the shortcut
editor, press Enter and then a chord; conflicts swap owners, `r` restores
defaults, and Escape cancels capture. Supported chords include Ctrl/Alt plus an
ASCII letter or digit, F1–F24, Tab, navigation keys and Enter/Escape.

## Keys and mouse

| Key | Action |
|---|---|
| `tab` / `shift+tab` | Launchpad: move between the prompt and the parameter chips (`enter` cycles the chip under the cursor; `escape` or typing goes back to the prompt). Conversation: take the keys into the rail (the cursor lands on the conversation on screen) and back |
| `←` `→` | Move the draft cursor; in a focused rail, switch between its two tabs (最近 ↔ 工作区) |
| `↑` `↓` | Move through draft lines / submission history; in a focused rail, move to the previous / next conversation (headings are skipped) and `enter` opens it |
| `shift+←` / `shift+→` | Switch to the previous / next conversation in the rail's order, wherever the keys are: the transcript follows, and so does the rail's cursor |
| `p` | Rails: pin / unpin the workspace the selected conversation belongs to |
| `ctrl+j` / `ctrl+enter` | Insert a newline. `ctrl+j` is the one every terminal delivers; `ctrl+enter` arrives when the terminal reports modifiers (kitty keyboard protocol, xterm `modifyOtherKeys`, or a Windows Terminal `sendInput` binding), and `shift+enter` needs one of those too. Windows Terminal keeps `alt+enter` for fullscreen and sends plain Enter for `ctrl+enter` unless the reader binds it, e.g. `{"keys": "ctrl+enter", "command": {"action": "sendInput", "input": "\u001b[13;5u"}}` |
| `ctrl+w`, `alt+d`, `ctrl+u`, `ctrl+k`, `ctrl+y` | Delete a word, kill to line start/end, yank |
| `ctrl+z` / `ctrl+shift+z` (or `alt+y`) | Undo / redo draft edits |
| `pageUp` / `pageDown` or `ctrl+↑` / `ctrl+↓` | Scroll the conversation; `esc` returns to the latest output |
| `ctrl+g` or `/queue` | List queued prompts; `x` cancels, `r` / `enter` cancels and recalls into the draft (`ctrl+x` stops the active run, not the queue) |
| `F6` or `/copy` | Copy the last assistant answer as clean content, without terminal chrome |
| `F7` or `/mouse` | Toggle and persist mouse capture off/on for native terminal selection |
| `F8` | Toggle native-scrollback linear reading |
| `F9` | Toggle reduced motion independently |
| `ctrl+b` | Sidebar: show + focus, then unfocus, then hide; full-screen on a narrow terminal. The show/hide choice is written to `sidebarVisible` and is where the next start begins |
| `/` or `ctrl+p` | Command palette (`/` requires an empty draft) — type to filter, `enter` to run |
| `ctrl+r` | Session picker (name, workspace and age of each conversation; the footer names the key) |
| `ctrl+o` | Preferences and shortcut editor |
| `F1` or `?` | Help overlay (effective keys + commands; `?` requires an empty draft) |
| `/lang` | Interface language: follow the system, Chinese or English |
| `ctrl+e` | Final-answer / full-process reading level: a settled reply folds its whole work — the calls and reasoning before *and* after its last prose — into one summary row |
| `ctrl+c` | Quit |

| Mouse | Action |
|---|---|
| Click a sidebar tab | Switch panel (`←`/`→` do the same while the rail has the keys) |
| Click a session row | Open that conversation (a workspace heading only takes the keys) |
| Drag the divider | Resize the sidebar (30–48 columns), persisted on release |
| Click anywhere in the input box | Focus it |
| Wheel over the conversation | Scroll; at the top it asks for an older page |
| Wheel over the sidebar | Scroll the panel |
| Click a workflow row | Open the session, fork point or worktree; choose a Diff file |
| Click a preference or shortcut row | Select it, open the shortcut editor or start key capture |
| Click an approval action | Apply the same allow-once, allow-rule or deny decision as its key |
| Click a palette entry | Run that command (the list a pointer cannot press leaves `ctrl+p` to the keyboard alone) |
| Click a queued prompt | Recall it into the draft, exactly as `enter` does there |
| Click the page around an overlay | Close it; a click on the box itself is absorbed, never passed to the control behind it |

Mouse reporting is SGR 1006 and starts with the persisted preference (enabled by default). `/mouse` releases and restores it at runtime and saves the choice; the terminal is always restored on exit. Draft text, cursor, folded pastes and image attachments are kept per session across overlays, launchpad visits and session switches (for the lifetime of this TUI process, not persisted to disk).

The conversation follows new output by default. Scrolling up detaches from the tail; further output leaves the reading position unchanged and shows a new-output badge. Scroll back to the bottom or press `esc` to follow again.

The driver only repaints dirty frames: there is no idle 120ms repaint timer. Streaming and spinner updates coalesce at 80ms, input and resize repaint immediately, and transcript rendering is cached by width/content revision. Reduced-motion does not schedule animation repaints; actual stream and state changes still repaint. Linear reading bypasses full-screen repaint entirely and appends only new transcript content, redrawing its one-line prompt.

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
| Queue management | `/queue` reads `get_state.queuedRuns`; cancel and recall use the existing `cancel_queued_run`, with no second client-side queue |
| Draft attachments | Reuses `components/input.rs` folded-paste and image-marker machinery; attachments travel with the prompt |
| Approvals | A parked request opens a full card with tool, kind, risk, action/paths, requested action and sandbox boundary. `y` / `n` allow once or deny; `a` first adds the agent-suggested **this-run** workspace rule, then approves. A rule failure leaves the request pending. Escape closes the card without deciding |
| Diff review | `/diff` lists tracked and untracked changes from the session workspace and shows the selected real git patch. Wide terminals keep a fixed file navigator beside an independently scrolling patch. Narrow terminals switch between files and patch with `tab`; `up`/`down` operate the focused view and left/right choose a file. The arrow follows the cursor immediately, and `·` marks the file whose patch is already loaded while the reload is in flight. `r` refreshes. This is the current uncommitted working tree, not per-turn attribution; patches are preview-bounded |
| History search and tree | `/history` filters the current `list_sessions` titles/first-message previews, workspaces and ids; `/tree` uses `parentSessionId`. Enter opens the selected session |
| Overlay lists | One record is one row: `up`/`down` move the highlight exactly one row, and a long title is shortened in place rather than folded (a wrapped record made one keypress skip two rows). Fields that no longer fit are dropped whole — a workspace cut mid-path would read as a different directory — so a narrow box shows title, then title · workspace, then the id. `pageUp`/`pageDown` move the cursor by one screenful (a detail card scrolls instead, and the diff follows whichever pane has the keys) |
| `/worktree` rows | `›` is the selection; `●` marks the worktree the session is already in, so choosing one is visible even when the first row is current |
| Fork | `/fork` lists real user-message points from `get_fork_messages`; enter calls `fork` and opens the child |
| Worktrees | `/worktree` lists real git worktrees and enter sets the session cwd. `/worktree new <branch>` creates under `.worktrees/` and moves the session. No remove/prune/reset/clean operations |
| Existing settings | `/sandbox`, `/permission [all|workspace|none]`, `/compact [instructions]`, `/tools a,b` or `/tools off`, `/providers`; `/skills` and `/model` retain their existing surfaces |
| Sessions panel | `list_sessions`, newest first: 最近 as a flat list, 工作区 filed by directory with the pinned ones leading. Pins persist in `settings.json` (`pinnedWorkspaces`) |
| Usage / context / cost | `get_state`'s usage block, refreshed on the same events |
| `ctrl+n` / `/new`, `ctrl+x` stop, `F5` refresh | |
| Launchpad chips | Click 工作区 / 模型 / 强度 / 权限 / 模式 to cycle them; on the launchpad the workspace and the model/effort choices are staged for the session the first message creates (the agent has no session to set them on yet), while permission and mode cycle the agent's own state. 工作区 cycles the directories this client has conversations in, and the notice row names the full path it chose |
| Theme, reading level | Persisted in `~/.future/tui/settings.json` (`tui/src/settings.rs`) |

## What is not here yet

- **`@` file mentions** — not implemented; pasted image paths and folded pastes use `tui/src/paste.rs`.
- **The command palette** lists the commands the adapter implements. Provider
  and tool management is intentionally compact (`/providers` and `/tools`);
  `/skills` remains the installed-skills list in the palette. Approval allow
  rules are runtime-only; persistent approval-rule file editing remains in the
  desktop.
- Further enhancements: timeline rail, image preview, full-screen session
  manager, full-screen draft editor.

## Where the code lives

| File | Responsibility |
|---|---|
| `tui/src/workbench/run.rs` | The driver: terminal, dirty/coalesced event loop, clipboard and mouse capture |
| `tui/src/workbench/editor.rs` | Byte-indexed adapter over the shared multiline editor; complete per-session draft state |
| `tui/src/workbench/scroll.rs` | Follow-tail, unread-output and return-to-latest semantics |
| `tui/src/clipboard.rs` | Plain assistant-content clipboard output |
| `tui/src/workbench/live.rs` | The agent adapter: `WorkbenchData` in, `WbAction`s applied |
| `tui/src/index.rs` | Arguments, `--help`, print mode, `--list-models`; interactive goes to the driver |
| `tui/src/workbench/mod.rs` | `Workbench` (view state), `WbEvent` / `WbAction`, key and mouse dispatch |
| `tui/src/workbench/screen.rs` | Frame composition: the bands, splicing partial bands into a row, overlays |
| `tui/src/workbench/layout.rs` | Band rectangles, the two breakpoints, the hit map |
| `tui/src/workbench/ansi.rs` | Width-safe text: `fit`, `pad`, `clip`, column slicing |
| `tui/src/workbench/widgets.rs` | Rules, boxes, tabs, bars, badges, spinner, logo, wordmark, the input box |
| `tui/src/workbench/panels.rs` | The rail's two panels (pure renderers over one row model) |
| `tui/src/workbench/picker.rs` | The `/model`, `/theme` and `/sessions` list picker |
| `tui/src/workbench/launchpad.rs` | The first screen |
| `tui/src/workbench/state.rs` | `WorkbenchData` and its parts |
| `tui/src/workbench/mouse.rs` | SGR 1006 parsing |
| `tui/src/workbench/theme.rs` | The six palettes: `dark`, `light`, `one-dark`, `one-light`, `high-contrast`, `dracula` |
| `tui/src/workbench/i18n.rs` | Every piece of chrome, in Chinese and English, and the locale/preference resolution |
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
