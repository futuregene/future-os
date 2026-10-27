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
 header (2 rows: session · workspace · backend dot, then hairline)
 ──────────────────────────────────────────┬──────────────────────
 conversation (bottom-anchored, ↑ 更早)     │ rail: 最近 工作区
                                            │ ▸ the conversation's name  ~/future-os · 2m
                                            │
 input box (❯ multiline draft, grows)      │  (the rail runs down beside it)
 activity: ⠋ 思考 … · 3s · 2 工具                   42 tok/s · 缓存 96%
 status:   model · effort · approval   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 hint:     Esc 回到聊天 · ctrl+d 侧栏 · ctrl+r 会话 · ctrl+p 命令 · ctrl+q 偏好设置
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
`ctrl+r`, `shift+↑`/`shift+↓` — therefore starts the row over, and a conversation
whose run is still in flight (started here, by the desktop, by a queued run)
takes the spinner back rather than showing `idle`.

The rail has **two tabs**: 最近 (conversations by refresh time) and 工作区 (filed by
by directory). Everything the rail lists is true for the whole agent; the
conversation's own workspace is a **separate panel** (文件), because it is not a
fact about the agent but about the conversation on screen.

**文件** (`/files`, or the 文件 button in the header) opens the
session workspace as a box over the frame: 文件 lists one level at a time and
reads a directory when it is opened, never before: a repository with a
`node_modules` opens instantly, and each listing is capped at 400 entries
with a line saying so. `↑`/`↓` walk the rows, `enter` (or `→`) opens a directory
and previews a file, `backspace` (or `←`) closes a directory (or jumps to the
directory that holds the row), `r` re-reads the directory and `esc`/`q` close
the box. A click does what `enter` does on the row it landed on. Switching
conversations re-reads the tree: the panel is scoped to the session, the same
way `/git` is.

## Pictures

`ctrl+v` reads the clipboard: an image becomes a **draft attachment** (the box's
title counts it, `· 📎 2 images`), and text is pasted into the draft. A pasted
image is carried as the `[Image #N]` marker the input already understands, so it
is sent with the prompt the same way a dragged-in file is; `/preview` opens the
newest attached image. `/paste` is the same verb from the palette — it is there
because Windows Terminal binds `ctrl+v` to its own paste (which cannot carry an
image), so on that terminal the key never arrives.

An image is placed by the terminal's *graphics protocol*: a real bitmap at a cell
(kitty's `a=T` placement, transmitted once per source and only re-placed on later
frames), not a character mosaic. The preview box (`enter` on an image in 文件, or
`/preview` for an attachment) puts its controls in the frame's first row —
`[zoom out] [2x] [zoom in] [fit]` and `[previous] 2/5 [next]`, every one of them
clickable — and answers `+`/`-` (or the wheel) up to 6×, `[`/`]` for the
previous/next picture in the same folder, and `0` to fit. A zoomed picture pans
with the arrow keys or by **dragging it with the mouse**. A terminal with no
graphics protocol gets the dimensions and a sentence saying so instead of an
approximation, and a folder holding one picture shows no step buttons at all.
Documents — Markdown rendered, anything else as lines — scroll with `↑`/`↓` and
`pageUp`/`pageDown` instead. A file it cannot show says why rather than opening
an empty box.

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
attached the composer fills the surface (the name, the box, workspace / model /
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

**One prefix: `ctrl`.** No default uses `alt` — on macOS the Option key composes
a character unless the reader turns on "Use Option as Meta", so an `alt`
shortcut is one that silently does nothing on the terminal this surface is
developed against — and no default is a function key (`F1`–`F12` need `Fn` on a
laptop and are absent on a compact keyboard).

The fifteen chords are `ctrl+c` quit, `ctrl+n` new chat, `ctrl+r` sessions,
`ctrl+d` the rail, `ctrl+x` stop, `ctrl+e` fold/unfold the process, `ctrl+p`
commands, `ctrl+o` model, `ctrl+t` effort (thinking), `ctrl+s` approval mode,
`ctrl+l` workspaces, `ctrl+f` the files panel, `ctrl+g` the git browser,
`ctrl+v` paste, `ctrl+q` preferences. Everything else is a typed verb or a
pointer target: `/delete`, `/queue`, `/copy`, `/mouse`, `/preview`, `/refresh`,
`/lang`, and `?` for help — the help overlay prints `—` in the key column for
those, and its command list names the door.

The map stops there because the letters do: a terminal sends Backspace, Tab,
`\n` and Enter for `ctrl+h`/`i`/`j`/`m`; `ctrl+a` and `ctrl+b` are the default
prefixes of `screen` and `tmux`; `ctrl+z` is suspend everywhere else; and
`ctrl+u`/`k`/`w`/`y` are the composer's own editing keys. The rail takes `d`
(the dock it slides out of) — the letter deleting a conversation gave up, since
a delete asks first and is rare enough to live on `/delete` and the rail's own
`delete` key. Every chord is rebindable in `/preferences`, and the help overlay
lists them grouped by what they do rather than in one flat list.

Two breakpoints, both from the contract:

- **width < 88**: the sidebar is dropped; `/recent` (or `shift+←`) opens the current panel
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

The header is a two-row context line — session, workspace and the backend dot,
with the hairline under it — leaving the rest of the frame to the conversation.
`Ctrl+O` or `/preferences` opens the preference panel: move the sidebar left or
right, adjust its 30–60-column width, control mouse capture and completion
notifications, enable reduced motion, and rebind global
shortcuts. The panel keeps the selected row and help visible on short terminals.
Choices persist in `~/.future/tui/settings.json` as `sidebarPosition`,
`sidebarWidth`, `sidebarVisible`, `mouseCapture`, `notify` and `reducedMotion`;
incremental shortcut overrides live beside it in
`keybindings.json`.

With no stored `sidebarVisible` — a first start, or a file written before the
key existed — the rail starts **hidden**: the conversation gets the whole
width, and `shift+←`/`shift+→` (which show the rail to reach
their panel) brings it back. Once the reader has shown or hidden it, that choice
is what the next start begins with, and `Restore defaults` lands back on the
hidden layout.
`/theme` opens the theme picker: the six palettes the TUI has always shipped
(`dark`, `light`, `one-dark`, `one-light`, `high-contrast`, `dracula`) plus the
desktop's palette family (`ayu`, `catppuccin`, `everforest`, `gruvbox`,
`kanagawa`, `nightfox`, `nord`, `rose-pine`, `solarized`, `tokyo-night`,
`vesper`, and `-light` renditions of catppuccin, everforest, gruvbox, rose pine
and solarized). Choosing one persists `themeId` and recolors both surfaces. With
no stored choice, the terminal's `COLORFGBG`
hint selects its light/dark palette (unknown hints use dark). `NO_COLOR`
disables styling by variable presence, including an empty value. **Reduced
motion** (the preferences overlay's 动态 row) stops the spinner and its elapsed
counter from animating, while streamed content still arrives immediately. These bindings, plus copy,
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

`Ctrl+P` opens the command palette, `Ctrl+R` sessions, and `?` (in an empty
draft) help. No default binding is a function key or an `alt+` chord — see
"One prefix" above for why — and a reader may still *rebind* to either. Help and
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
| `←` `→` | Move the draft cursor; in a focused rail, switch to the neighbouring tab (最近 ↔ 工作区) — the same two `shift+←`/`shift+→` step through from anywhere |
| `↑` `↓` | Move through draft lines / submission history; in a focused rail, move the list cursor (headings are skipped). `enter` opens the row under it; switching the conversation itself needs no `enter` — that is `shift+↑`/`shift+↓` |
| `shift+←` / `shift+→` | The rail's two tabs (最近 / 工作区), wherever the keys are: the rail is shown and takes the keys, and its cursor lands on the conversation on screen. Wraps |
| `shift+↑` / `shift+↓` | Switch to the previous / next conversation in the rail's order, wherever the keys are: the transcript follows, and so does the rail's cursor. A held key switches once per burst, not once per repeat |
| `p` | Rails: pin / unpin the workspace the selected conversation belongs to |
| `/delete` or `delete` in the rail | Delete a conversation: the confirm box names the one that would go — the row under the rail's cursor when the rail has the keys, otherwise the one on screen — and `⏎` is what deletes it (`esc` keeps it). In a focused rail `delete` is the same door. The row that takes its place opens, and deleting the last one leaves the launchpad |
| `ctrl+j` / `ctrl+enter` | Insert a newline. `ctrl+j` is the one every terminal delivers; `ctrl+enter` arrives when the terminal reports modifiers (kitty keyboard protocol, xterm `modifyOtherKeys`, or a Windows Terminal `sendInput` binding), and `shift+enter` needs one of those too. Windows Terminal keeps `alt+enter` for fullscreen and sends plain Enter for `ctrl+enter` unless the reader binds it, e.g. `{"keys": "ctrl+enter", "command": {"action": "sendInput", "input": "\u001b[13;5u"}}` |
| `ctrl+w`, `alt+d`, `ctrl+u`, `ctrl+k`, `ctrl+y` | Delete a word, kill to line start/end, yank |
| `ctrl+z` / `ctrl+shift+z` (or `alt+y`) | Undo / redo draft edits |
| `pageUp` / `pageDown` or `ctrl+↑` / `ctrl+↓` | Scroll the conversation; `esc` returns to the latest output |
| `/queue` | List queued prompts — the ones sent while a run was in flight; `x` cancels, `r` / `enter` cancels and recalls into the draft (`ctrl+x` stops the active run, not the queue) |
| `ctrl+g` or `[ctrl+g] git` in the header | The repository browser: 提交 / 引用 / 改动 / 暂存 tabs, a graph log, and the selected row's patch. `1`–`4` pick a tab, `tab` switches list/detail, `y` copies the row |
| `ctrl+o` | Model picker (`/model`): every model the agent offers, grouped by provider, with the reasoning slider on its first row |
| `shift+left` / `shift+right` | The rail's two tabs, one step at a time: 最近 (conversations by refresh time) and 工作区 (filed by directory). The rail takes the keys and its cursor lands on the conversation on screen; clicking a tab is the same switch |
| `ctrl+f` or `/files` | The **files panel**: this conversation's workspace as a tree, in a box over the frame — the same session scope as `/git`. `↑`/`↓` walk it, `⏎`/`→` open, `⌫`/`←` go up a level, `r` re-reads the directory, `esc` closes. The header's `[ctrl+f] 文件` button is the pointer's door to it, and `[ctrl+g] git` sits beside it |
| `ctrl+v` | Read the clipboard: an image becomes a draft attachment, text is pasted. `/paste` does the same for terminals that bind the key themselves |
| `/preview` | Preview the newest image attached to the draft (no chord: the fifteen go to the actions pressed while working) |
| `+` / `-` / `0`, arrows, drag, wheel | In the preview box: zoom a picture (up to 6×), pan it, reset; scroll a document |
| `[` / `]` | In the preview box: previous/next picture in the same folder (the same action as its step buttons) |
| `/copy` | Copy the last assistant answer as clean content, without terminal chrome |
| `/mouse` | Toggle and persist mouse capture off/on for native terminal selection |
| 偏好设置 → 动态 | Toggle reduced motion (persisted; no chord) |
| `ctrl+t` | Thinking effort: opens the six-level chooser (`/thinking`); on the launchpad the level is staged for the session the first message creates |
| `ctrl+s` | Approval mode: opens the four-tier chooser, the desktop composer's own values — 手动审批 / 沙箱保护 / 自动审批 / 完全访问 |
| `ctrl+l` | Workspaces: opens the workspace chooser — every directory this client has a conversation in, plus a row that creates one from the path typed into its search line |
| `ctrl+d` | Sidebar: show + focus, then unfocus, then hide; full-screen on a narrow terminal. The rail's own `›` edge button does the same hide/show with one click. The show/hide choice is written to `sidebarVisible` and is where the next start begins (a first start begins hidden) |
| `/` or `ctrl+p` | Command palette (`/` requires an empty draft) — type to filter, `enter` to run |
| `ctrl+r` | Session picker (name, workspace and age of each conversation; the footer names the key) |
| `ctrl+q` or `/preferences` | Preferences and shortcut editor. The key is also printed in the footer hint row, so it is on screen while the reader is wondering |
| `?` | Help overlay (effective keys, grouped by area, then the commands; `?` requires an empty draft) |
| `/lang` | Interface language: opens the picker — follow the system, Chinese or English |
| `ctrl+e` | Final-answer / full-process reading level: every reply folds its work — the calls and reasoning before its last prose, and, once it settles, the work after it too — into one summary row (`▸ 4 tool calls · 2 thoughts`). A reply that is still running folds the same way, keeping only the call in flight as a row; a reply that never writes prose folds onto the slice it stopped at. Reasoning is collapsed to a `▸ thinking` row at the default level (the way the desktop and the phone show it) while the full-process level prints its text. A view toggle for the run it is pressed in, not a preference: every start opens folded, so a peek at the process cannot leave the next terminal expanded |
| `ctrl+c` | Quit |

| Mouse | Action |
|---|---|
| Click a sidebar tab | Switch panel (`←`/`→` do the same while the rail has the keys) |
| Click the rail's `›` edge (or the `‹` it leaves behind) | Hide the rail / bring it back. `shift+←`/`shift+→` do the same from the keyboard |
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
| File panel and preview | `r` re-reads the directory in the panel; a directory is read when it opens (400 entries per level, said out loud when cut). A preview decodes an image to at most 2048 px on its long edge — those pixels are what keeps a zoom sharp — and downsamples the bitmap it transmits to the cells on screen; text is read bounded and cut on a character boundary |
| Approvals | A parked request opens a full card with tool, kind, risk, action/paths, requested action and sandbox boundary. `y` / `n` allow once or deny; `a` first adds the agent-suggested **this-run** workspace rule, then approves. A rule failure leaves the request pending. Escape closes the card without deciding |
| Diff review | `/diff` lists tracked and untracked changes from the session workspace and shows the selected real git patch. Wide terminals keep a fixed file navigator beside an independently scrolling patch. Narrow terminals switch between files and patch with `tab`; `up`/`down` operate the focused view and left/right choose a file. The arrow follows the cursor immediately, and `·` marks the file whose patch is already loaded while the reload is in flight. `r` refreshes. This is the current uncommitted working tree, not per-turn attribution; patches are preview-bounded |
| Repository browser | `/git` (or `ctrl+g`) is a read-only tig-shaped panel with four tabs: 提交 (`git log --graph`, 200 commits, decorations), 引用 (local/remote branches and tags with upstream tracking), 改动 (the working tree, same porcelain read as `/diff`) and 暂存 (`git stash list`). Each row's detail loads on selection: a commit's stat and patch, a ref's upstream and tip, a file's patch, a stash's diff. `1`–`4` pick a tab, `tab` switches list/detail (the wide layout shows both), `enter` opens (a ref filters the log to it, a changed file opens in the preview box, a commit moves the keys to its patch), `y` copies the sha/ref/path, `backspace` clears the filter, `r` re-reads. Nothing here stages, commits, checks out or drops a stash, and every query is bounded and killed past its deadline |
| History search and tree | `/history` filters the current `list_sessions` titles/first-message previews, workspaces and ids; `/tree` uses `parentSessionId`. Enter opens the selected session |
| Overlay lists | One record is one row: `up`/`down` move the highlight exactly one row, and a long title is shortened in place rather than folded (a wrapped record made one keypress skip two rows). Fields that no longer fit are dropped whole — a workspace cut mid-path would read as a different directory — so a narrow box shows title, then title · workspace, then the id. `pageUp`/`pageDown` move the cursor by one screenful (a detail card scrolls instead, and the diff follows whichever pane has the keys) |
| `/worktree` rows | `›` is the selection; `●` marks the worktree the session is already in, so choosing one is visible even when the first row is current |
| Fork | `/fork` lists real user-message points from `get_fork_messages`; enter calls `fork` and opens the child |
| Worktrees | `/worktree` lists real git worktrees and enter sets the session cwd. `/worktree new <branch>` creates under `.worktrees/` and moves the session. No remove/prune/reset/clean operations |
| Existing settings | `/mode` (or `/sandbox`) opens the approval-mode chooser, `/thinking` the reasoning chooser, `/compact [instructions]`, `/tools a,b` or `/tools off`, `/providers`; `/skills` and `/model` retain their existing surfaces |
| Sessions panel | `list_sessions`, newest first: 最近 split into a 活跃 stretch (a run still in flight, spinner in the row's state slot) and 更早, the headings drawn only while both are non-empty. The list is re-read every five seconds while the rail is on screen, because another client's run reaches this surface no other way. 工作区 filed by directory with the pinned ones leading. Pins persist in `settings.json` (`pinnedWorkspaces`) |
| Usage / context / cost | `get_state`'s usage block, refreshed on the same events |
| `ctrl+n` / `/new`, `ctrl+x` stop, `/refresh` | |
| Delete a conversation (`/delete`, or `delete` in the rail) | One confirm box, then `delete_session` for the id it named. The successor in the list opens, so the transcript never stays on a conversation that is gone; deleting the last one detaches to the launchpad. A refusal (the agent keeps a session a compaction holds) leaves everything as it was and says so in the notice row |
| Launchpad chips | Click 工作区 / 模型 / 强度 / 审批 to cycle them. On the launchpad the workspace, model, effort and approval choices are staged for the session the first message creates (the agent has no session to set them on yet); with a session attached they go over the wire. 工作区 cycles the directories this client has conversations in, and the notice row names the full path it chose |
| Theme | Persisted in `~/.future/tui/settings.json` (`tui/src/settings.rs`); the reading level is deliberately not — `ctrl+e` is per run |

## What is not here yet

- **`@` file mentions** — not implemented; pasted image paths and folded pastes use `tui/src/paste.rs`.
- **A preview is a preview.** The picture is placed over the cells the box has, at up to 6× the fitted size. For pixel-level work open it in the system image viewer; the TUI decodes locally and sends the picture nowhere.
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
| `tui/src/workbench/widgets.rs` | Rules, boxes, tabs, bars, badges, spinner, the input box |
| `tui/src/workbench/panels.rs` | The rail's panels and the files box's rows (pure renderers over one row model) |
| `tui/src/workbench/picker.rs` | The `/model`, `/theme` and `/sessions` list picker |
| `tui/src/workbench/launchpad.rs` | The first screen |
| `tui/src/workbench/files.rs` | The 文件 panel's tree and what a path can be previewed as |
| `tui/src/workbench/image_view.rs` | Picture arithmetic for the placed bitmap: fit, zoom, pan, and the crop a zoomed box shows |
| `tui/src/workbench/state.rs` | `WorkbenchData` and its parts |
| `tui/src/themes.rs` | The palette catalogue: `Theme` values plus the id/label/dark table `/theme` reads |
| `tui/src/workbench/theme.rs` | The workbench `Palette` per catalogue id (22), and the semantic colours the screen paints with |
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
