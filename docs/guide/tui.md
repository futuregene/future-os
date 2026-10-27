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
                                            │ ▸ the conversation's name  ~/future-os · 2m
                                            │
 input box (❯ multiline draft, grows)      │  (the rail runs down beside it)
 activity: ⠋ 思考 … · 3s · 2 工具                   42 tok/s · 缓存 96%
 status:   model · effort · approval   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 hint:     Esc 回到聊天 · ctrl+b 侧栏 · tab 焦点 · ctrl+r 会话 · ctrl+p 命令
```

The rail is a *column*, not a panel that stops one band early: it (and its
divider) run from under the header down to the footer, so the space beside the
composer is the rail's own background rather than bare canvas. The three footer
rows — activity, status, hint — stay full width. The composer band is exactly
the box it draws (top rule, prompt, closing rule); it used to reserve a fourth
row and leave it blank.

The activity row describes the conversation **on screen**: its kind (思考 / 工具 /
生成), how long this client has been watching the run, and how many tool calls
the run has made — one per call, though the agent announces every call twice
while its arguments stream. Switching conversations — a click in the rail,
`ctrl+r`, `shift+←`/`shift+→` — therefore starts the row over, and a conversation
whose run is still in flight (started here, by the desktop, by a queued run)
takes the spinner back rather than showing `idle`.

The rail has **three tabs**: 最近 (conversations by refresh time), 工作区 (filed
by directory) and **文件** — a browser of the session workspace. 文件 lists one level
at a time and reads a directory when it is opened, never before: a repository
with a `node_modules` opens instantly, and each listing is capped at 400 entries
with a line saying so. `↑`/`↓` walk the rows, `enter` opens a directory and
previews a file, `backspace` closes a directory (or jumps to the directory that
holds the row), and a click does what `enter` does on the row it landed on.
`←`/`→` switch tabs on this tab too — a third tab that kept the arrows for
itself was a dead end, since the other two answer them — and the rail's last row
names the keys the panel in force answers.

## Pictures

`ctrl+v` reads the clipboard: an image becomes a **draft attachment** (the box's
title counts it, `· 📎 2 images`), and text is pasted into the draft. A pasted
image is carried as the `[Image #N]` marker the input already understands, so it
is sent with the prompt the same way a dragged-in file is; `alt+v` opens the
newest attached image. `/paste` is the same verb from the palette — it is there
because Windows Terminal binds `ctrl+v` to its own paste (which cannot carry an
image), so on that terminal the key never arrives.

An image is *drawn in the terminal* as half blocks — one `▀` per cell, its
foreground the top pixel of the pair and its background the bottom one — so it
works in any 256-colour terminal with no graphics protocol: no kitty, no iTerm2,
no sixel. Two pixels per cell is the most a text surface can show, and 24-bit
colour is used when the terminal reports it. The preview box (`enter` on an image
in 文件, or `alt+v` for an attachment) shows the picture with its dimensions in
the title, zooms with `+`/`-` (or the wheel) up to 6×, pans with the arrows and
returns with `0`. Documents — Markdown rendered, anything else as lines — scroll
with `↑`/`↓` and `pageUp`/`pageDown` instead. A file it cannot show says why
rather than opening an empty box.

The rail is a *task* panel, in two tabs. **最近** (the default) lists every
conversation — its name, then the workspace it is filed under and when it was
last refreshed — newest first. A conversation whose run is still in flight leads
it under a 活跃 heading, with a spinner in its state slot; the rest follow under
更早, captioned with how many each stretch holds (the desktop's recent view, in
the rail's shape). The two headings are only drawn while there is a split: with
nothing running the list is simply the list. A row names its workspace by *name*
(`future-os`), not by path: the rail is 36 columns wide and a clipped path says
less than the word. **工作区** files the same conversations the way the
desktop's rail does: 置顶 (the workspaces pinned with `p`), then 工作区 (one
heading per workspace, its conversations beneath), then 对话会话 (conversations
that belong to no workspace — the app's own scratch directories). Usage figures
live on the status bar, so no panel repeats them; skills are the palette's. With
the rail focused, `←`/`→` move between the two tabs.

The launchpad is the *body*, not a dialog over the conversation: with no session
attached the composer fills the surface (wordmark, box, workspace / model /
effort / approval chips) and there is no second input band. The block is centred
in the body rather than pinned to its top, and the conversations are the rail's —
the launchpad does not repeat them. `tab` walks the prompt and the chips, `enter`
cycles the chip under the cursor, and typing (or `escape`) hands the keys back to
the prompt. The block teaches no key of its own: the footer's hint row already
names them, and what the box takes beyond a sentence — `/` commands, `@` files,
`#` sessions — is in the placeholder, where the reader is about to type it. The
four chips are the desktop composer's own controls (model, thinking effort,
approval mode) plus the workspace this TUI starts from; there is no separate
permission level, because no other client offers one.

No default shortcut is a function key: `F1`–`F12` need an `Fn` modifier on a
laptop keyboard and are absent on a compact one, so the defaults are
`ctrl+m` model, `ctrl+t` effort, `ctrl+s` approval mode, `ctrl+l` workspaces,
`ctrl+1`/`ctrl+2` the rail's tabs, `alt+h` help, `alt+r` refresh, `alt+c` copy,
`alt+m` mouse, `alt+a` reduced motion. Every one of them is rebindable in
`/preferences`, and the help overlay lists them grouped by what they do rather
than in one flat list.

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
- The locale comes from `LC_ALL`, then `LC_MESSAGES`, then the system's own
  answer — the macOS preferred language (`AppleLanguages`), the Windows UI
  language — and `LANG` last: a Mac's shell takes `LANG` from `/etc/zprofile`,
  so it describes the terminal rather than the Mac, and a Chinese Mac would
  otherwise draw English. A missing or `C` locale is English.
- `cargo run -p future-tui --example workbench_demo -- --lang en` prints a frame
  in either language; the screenshot capture pins `zh` by writing the settings
  file into its scratch home, so a baseline does not depend on who ran it.

The copy lives in one table, `tui/src/workbench/i18n.rs`, with both spellings on
the same line: a key added in one language only does not compile.

## Layout and personalization

The default **focused density** uses a two-row context header, reclaiming four
rows for the conversation. `Ctrl+O` or `/preferences` opens the preference
panel: switch to the comfortable identity header, move the sidebar left or
right, adjust its 30–60-column width, control mouse capture and completion
notifications, enable reduced motion, and rebind global
shortcuts. The panel keeps the selected row and help visible on short terminals.
Choices persist in `~/.future/tui/settings.json` as `density`, `sidebarPosition`,
`sidebarWidth`, `sidebarVisible`, `mouseCapture`, `notify` and `reducedMotion`;
incremental shortcut overrides live beside it in
`keybindings.json`.

With no stored `sidebarVisible` — a first start, or a file written before the
key existed — the rail starts **hidden**: the conversation gets the whole
width, and `ctrl+b` (or `ctrl+1`/`ctrl+2`/`ctrl+3`, which show the rail to reach
their panel) brings it back. Once the reader has shown or hidden it, that choice
is what the next start begins with, and `Restore defaults` lands back on the
hidden layout.

`/theme` retains the six-theme picker, including `high-contrast`; choosing a
theme persists `themeId`. With no stored choice, the terminal's `COLORFGBG`
hint selects its light/dark palette (unknown hints use dark). `NO_COLOR`
disables styling by variable presence, including an empty value. `alt+a`
toggles **reduced motion**: the spinner and its elapsed counter stop animating,
and streamed content still arrives immediately. These bindings, plus copy,
queue and mouse, are contextual global actions and can be rebound; overlays
keep their local keys so a global shortcut never submits an approval or edits
history search accidentally.

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

`Ctrl+P` opens the command palette, `Ctrl+R` sessions, and `alt+h` (`?` in an
empty draft) help. No default binding is a function key: `F1`–`F12` need an
`Fn` modifier on a laptop, a screen-reader chord on some terminals, and are
absent on a compact keyboard — a shortcut that takes three fingers, or does not
exist, is not a shortcut. A reader may still *rebind* one to an F key. Help and
footer hints show the effective bindings, not stale defaults, and the help
overlay groups them (chats / choices / panels / interface / inline keys /
commands) instead of printing one flat list. In the shortcut
editor, press Enter and then a chord; conflicts swap owners, `r` restores
defaults, and Escape cancels capture. Supported chords include Ctrl/Alt plus an
ASCII letter or digit, F1–F24, Tab, navigation keys and Enter/Escape.

`Ctrl+M`, `Ctrl+1` and `Ctrl+2` are delivered only by terminals that report
modifiers — the kitty keyboard protocol, or xterm's `modifyOtherKeys`, both of
which the TUI negotiates at start. A terminal that reports none sends `Ctrl+M`
and `Enter` as the same byte, and `Ctrl+1` as a plain `1`; there `/model` and a
click on the rail's tab strip are the doors to the same places.

## Keys and mouse

| Key | Action |
|---|---|
| `tab` / `shift+tab` | Launchpad: move between the prompt and the parameter chips (`enter` cycles the chip under the cursor; `escape` or typing goes back to the prompt). Conversation: take the keys into the rail (the cursor lands on the conversation on screen) and back |
| `←` `→` | Move the draft cursor; in a focused rail, switch between its two tabs (最近 ↔ 工作区) |
| `↑` `↓` | Move through draft lines / submission history; in a focused rail, move to the previous / next conversation (headings are skipped) and `enter` opens it |
| `shift+←` / `shift+→` | Switch to the previous / next conversation in the rail's order, wherever the keys are: the transcript follows, and so does the rail's cursor. A held key switches once per burst, not once per repeat |
| `p` | Rails: pin / unpin the workspace the selected conversation belongs to |
| `ctrl+j` / `ctrl+enter` | Insert a newline. `ctrl+j` is the one every terminal delivers; `ctrl+enter` arrives when the terminal reports modifiers (kitty keyboard protocol, xterm `modifyOtherKeys`, or a Windows Terminal `sendInput` binding), and `shift+enter` needs one of those too. Windows Terminal keeps `alt+enter` for fullscreen and sends plain Enter for `ctrl+enter` unless the reader binds it, e.g. `{"keys": "ctrl+enter", "command": {"action": "sendInput", "input": "\u001b[13;5u"}}` |
| `ctrl+w`, `alt+d`, `ctrl+u`, `ctrl+k`, `ctrl+y` | Delete a word, kill to line start/end, yank |
| `ctrl+z` / `ctrl+shift+z` (or `alt+y`) | Undo / redo draft edits |
| `pageUp` / `pageDown` or `ctrl+↑` / `ctrl+↓` | Scroll the conversation; `esc` returns to the latest output |
| `alt+q` or `/queue` | List queued prompts; `x` cancels, `r` / `enter` cancels and recalls into the draft (`ctrl+x` stops the active run, not the queue) |
| `ctrl+m` | Model picker (`/model`): every model the agent offers, grouped by provider, with the reasoning slider on its first row |
| `ctrl+1` / `ctrl+2` / `ctrl+3` | The rail's three tabs: 最近 (conversations by refresh time), 工作区 (filed by directory), 文件 (the workspace tree). The rail takes the keys and its cursor lands on the conversation on screen (or on the first row of the tree) |
| `ctrl+v` | Read the clipboard: an image becomes a draft attachment, text is pasted. `/paste` does the same for terminals that bind the key themselves |
| `alt+v` | Preview the newest image attached to the draft |
| `+` / `-` / `0`, arrows, wheel | In the preview box: zoom a picture (up to 6×), pan it, reset; scroll a document |
| `alt+c` or `/copy` | Copy the last assistant answer as clean content, without terminal chrome |
| `alt+m` or `/mouse` | Toggle and persist mouse capture off/on for native terminal selection |
| `alt+a` | Toggle reduced motion |
| `ctrl+t` | Thinking effort: opens the six-level chooser (`/thinking`); on the launchpad the level is staged for the session the first message creates |
| `ctrl+s` | Approval mode: opens the four-tier chooser (`/mode`), the desktop composer's own values — 手动审批 / 沙箱保护 / 自动审批 / 完全访问 |
| `ctrl+l` | Workspaces: opens the workspace chooser (`/workspace`) — every directory this client has a conversation in, plus a row that creates one from the path typed into its search line |
| `ctrl+b` | Sidebar: show + focus, then unfocus, then hide; full-screen on a narrow terminal. The rail's own `›` edge button does the same hide/show with one click. The show/hide choice is written to `sidebarVisible` and is where the next start begins (a first start begins hidden) |
| `/` or `ctrl+p` | Command palette (`/` requires an empty draft) — type to filter, `enter` to run |
| `ctrl+r` | Session picker (name, workspace and age of each conversation; the footer names the key) |
| `ctrl+o` | Preferences and shortcut editor |
| `alt+h` or `?` | Help overlay (effective keys, grouped by area, then the commands; `?` requires an empty draft) |
| `ctrl+g` or `/lang` | Interface language: opens the picker — follow the system, Chinese or English |
| `ctrl+e` | Final-answer / full-process reading level: a settled reply folds its whole work — the calls and reasoning before *and* after its last prose — into one summary row, and reasoning is collapsed to a `▸ thinking` row at the default level (the way the desktop and the phone show it) while the full-process level prints its text. A view toggle for the run it is pressed in, not a preference: every start opens folded, so a peek at the process cannot leave the next terminal expanded |
| `ctrl+c` | Quit |

| Mouse | Action |
|---|---|
| Click a sidebar tab | Switch panel (`←`/`→` do the same while the rail has the keys) |
| Click the rail's `›` edge (or the `‹` it leaves behind) | Hide the rail / bring it back. `ctrl+b` does the same from the keyboard |
| Click a session row | Open that conversation (a workspace heading only takes the keys) |
| Click a tree row | A file previews, a directory opens — the same two verbs `enter` runs |
| Drag the divider | Resize the sidebar (30–60 columns), persisted on release |
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

The driver only repaints dirty frames: there is no idle 120ms repaint timer. Streaming and spinner updates coalesce at 80ms, input and resize repaint immediately, and transcript rendering is cached by width/content revision. Reduced-motion does not schedule animation repaints; actual stream and state changes still repaint.

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
| Draft attachments | Reuses `components/input.rs` folded-paste and image-marker machinery; attachments travel with the prompt. `ctrl+v` / `/paste` read the system clipboard through `paste::ClipboardCapture` (osascript on macOS, PowerShell `-STA` on Windows, `wl-paste`/`xclip` on Linux) on a blocking thread, and an image it writes becomes an `[Image #N]` attachment. Nothing leaves the machine |
| File panel and preview | `alt+r` re-reads the tree; a directory is read when it opens (400 entries per level, said out loud when cut). A preview decodes an image to at most 1024 px on its long edge and draws it as half blocks; text is read bounded and cut on a character boundary |
| Approvals | A parked request opens a full card with tool, kind, risk, action/paths, requested action and sandbox boundary. `y` / `n` allow once or deny; `a` first adds the agent-suggested **this-run** workspace rule, then approves. A rule failure leaves the request pending. Escape closes the card without deciding |
| Diff review | `/diff` lists tracked and untracked changes from the session workspace and shows the selected real git patch. Wide terminals keep a fixed file navigator beside an independently scrolling patch. Narrow terminals switch between files and patch with `tab`; `up`/`down` operate the focused view and left/right choose a file. The arrow follows the cursor immediately, and `·` marks the file whose patch is already loaded while the reload is in flight. `r` refreshes. This is the current uncommitted working tree, not per-turn attribution; patches are preview-bounded |
| History search and tree | `/history` filters the current `list_sessions` titles/first-message previews, workspaces and ids; `/tree` uses `parentSessionId`. Enter opens the selected session |
| Overlay lists | One record is one row: `up`/`down` move the highlight exactly one row, and a long title is shortened in place rather than folded (a wrapped record made one keypress skip two rows). Fields that no longer fit are dropped whole — a workspace cut mid-path would read as a different directory — so a narrow box shows title, then title · workspace, then the id. `pageUp`/`pageDown` move the cursor by one screenful (a detail card scrolls instead, and the diff follows whichever pane has the keys) |
| `/worktree` rows | `›` is the selection; `●` marks the worktree the session is already in, so choosing one is visible even when the first row is current |
| Fork | `/fork` lists real user-message points from `get_fork_messages`; enter calls `fork` and opens the child |
| Worktrees | `/worktree` lists real git worktrees and enter sets the session cwd. `/worktree new <branch>` creates under `.worktrees/` and moves the session. No remove/prune/reset/clean operations |
| Existing settings | `/mode` (or `/sandbox`) opens the approval-mode chooser, `/thinking` the reasoning chooser, `/compact [instructions]`, `/tools a,b` or `/tools off`, `/providers`; `/skills` and `/model` retain their existing surfaces |
| Sessions panel | `list_sessions`, newest first: 最近 split into a 活跃 stretch (a run still in flight, spinner in the row's state slot) and 更早, the headings drawn only while both are non-empty. The list is re-read every five seconds while the rail is on screen, because another client's run reaches this surface no other way. 工作区 filed by directory with the pinned ones leading. Pins persist in `settings.json` (`pinnedWorkspaces`) |
| Usage / context / cost | `get_state`'s usage block, refreshed on the same events |
| `ctrl+n` / `/new`, `ctrl+x` stop, `alt+r` refresh | |
| Launchpad chips | Click 工作区 / 模型 / 强度 / 审批 to cycle them. On the launchpad the workspace, model, effort and approval choices are staged for the session the first message creates (the agent has no session to set them on yet); with a session attached they go over the wire. 工作区 cycles the directories this client has conversations in, and the notice row names the full path it chose |
| Theme | Persisted in `~/.future/tui/settings.json` (`tui/src/settings.rs`); the reading level is deliberately not — `ctrl+e` is per run |

## What is not here yet

- **`@` file mentions** — not implemented; pasted image paths and folded pastes use `tui/src/paste.rs`.
- **A preview is a preview.** The half-block renderer draws a picture at most a
  couple of hundred cells wide, which is a thumbnail of a photo and a legible
  view of a screenshot or a diagram. A reader who needs the pixels opens the
  file in an image viewer; the TUI never transmits the image data anywhere.
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
| `tui/src/workbench/panels.rs` | The rail's three panels (pure renderers over one row model) |
| `tui/src/workbench/picker.rs` | The `/model`, `/theme` and `/sessions` list picker |
| `tui/src/workbench/launchpad.rs` | The first screen |
| `tui/src/workbench/files.rs` | The 文件 panel's tree and what a path can be previewed as |
| `tui/src/workbench/image_view.rs` | An image as half blocks: fit, zoom, pan, 256-colour fallback |
| `tui/src/workbench/state.rs` | `WorkbenchData` and its parts |
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
