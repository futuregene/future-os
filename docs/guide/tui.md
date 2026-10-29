# FutureOS Terminal UI (TUI)

`future tui` is a full-screen terminal UI: it owns the screen, repaints absolute
rows and resolves mouse clicks against what it painted. It replaced the earlier
inline TUI (which appended rows into the terminal's scrollback and diffed the
tail); that UI, its code and its page are gone, not deprecated.

The design contract is [`docs/tui/workbench-spec.md`](../tui/workbench-spec.md).

## Running it

```bash
future tui                          # the terminal UI (its start directory is the new conversation's workspace)
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
 header (2 rows: session · workspace · doors · build, then hairline)
 ──────────────────────────────────────────┬──────────────────────
 conversation (bottom-anchored, ↑ 更早)     │ rail: 最近 工作区
                                            │ ▸ the conversation's name  ~/future-os · 2m
                                            │
 input box (❯ multiline draft, grows)      │  (the rail runs down beside it)
 activity: ⠋ 思考 … · 3s · 2 工具                   42 tok/s · 缓存 96%
 status:   model · effort · approval   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 hint:     Esc 回到聊天 · ctrl+d 侧栏 · ctrl+p 命令 · ctrl+q 偏好设置
```

The hint row teaches keys whose door is not already on screen: `ctrl+r` is not on
it, because the header's own `[ctrl+r] 会话` button spells that chord a row
above, and `ctrl+f`/`ctrl+g` ride on their buttons the same way. Nothing a
reader needs is taught only in the help box, and nothing is taught twice.

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
`ctrl+r`, `ctrl+↑`/`ctrl+↓` — therefore starts the row over, and a conversation
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

Pictures **in the conversation** are their names, linked to the file — a line of
text, never a picture drawn inline. A picture that came with a message paints the
attachment's name as a link to its path, in the bubble it came with; a markdown
`![caption](path)` an agent writes paints its caption (its path, when it has none)
the same way, and a path written in prose that names a picture which exists is a
target too. `tab` steps onto that line (one stop per picture) and `⏎` — or a click
on it — opens the file in the preview box, which is where the picture is: full
size, zoomable, pannable. A picture **you** sent comes back the same way — the
entry keeps the attachment's path rather than its bytes, so it is the file on disk
that is shown, if it is still there. Nothing is reserved in the transcript for a
thumbnail, so a message with four screenshots is four lines rather than four
blocks, and the answer around it stays in view. The transcript's own bitmap
machinery is the box's alone: bytes are read and decoded on a background thread
when something asks to see them, and kept only while that box is open.

An image is placed by the terminal's *graphics protocol*: a real bitmap at a cell
(kitty's `a=T` placement, transmitted once per source and only re-placed on later
frames; iTerm2's inline-image protocol, re-sent only when its cells are rewritten
or the shown crop changes), not a character mosaic. The preview box (`enter` on an image in 文件, or
`/preview` for an attachment) puts its controls in the frame's first row —
`[zoom out] [2x] [zoom in] [fit]` and `[previous] 2/5 [next]`, every one of them
clickable — and answers `+`/`-` (or the wheel) up to 6×, `[`/`]` for the
previous/next picture in the same folder, and `0` to fit. A zoomed picture pans
with the arrow keys or by **dragging it with the mouse**. A terminal with no
graphics protocol gets the dimensions and a sentence saying so instead of an
approximation, and a folder holding one picture shows no step buttons at all.
Documents — Markdown rendered, anything else as lines — scroll with `↑`/`↓` and
`pageUp`/`pageDown` instead. A file it cannot show says why rather than opening
an empty box. The bytes are read and decoded on a background thread: the box
opens at once with a loading row, and an encoded picture past 64 MB is refused
with its size instead of being loaded.

The rail is a *task* panel, in two tabs. **最近** (the default) lists every
conversation — its name, then the workspace it is filed under and when it was
last refreshed — newest first. A *conversation* is a session that holds a
message: the agent lists a session from the moment it is created, before its
first prompt, and one opened and never spoken in has no name and no prompt to be
called by — only an id, whose first eight characters read as a date
(`20261019`). The desktop's rail leaves those out of its list as well, so the
rail here does too, and the row appears once the first message lands. A
conversation whose run is still in flight leads it under a 活跃 heading, with a
spinner in its state slot; the rest follow under
更早, captioned with how many each stretch holds (the desktop's recent view, in
the rail's shape). The two headings are only drawn while there is a split: with
nothing running the list is simply the list. A row names its workspace by *name*
(`future-os`), not by path: the rail is 36 columns wide and a clipped path says
less than the word. **工作区** files the same conversations the way the
desktop's rail does: 置顶 (the workspaces pinned with `p`), then 工作区 (one
heading per workspace, its conversations beneath), then 对话会话 (conversations
that belong to no workspace — the app's own scratch directories). Usage figures
live on the status bar, so no panel repeats them; skills are the palette's. With
the rail focused, the two tabs are still `shift+←`/`shift+→` and nothing else: a
plain `←`/`→` is the draft caret's from every state, because the prompt stays on
screen (and can hold the keys' next keystroke) while the rail is up.

Both tabs list the same conversations; a fold (`enter` on a workspace heading, or
a click on one) is remembered in `~/.future/tui/settings.json`
(`foldedWorkspaces`) and is already in force on the next start. Searching
sessions is the history picker's job: `ctrl+r` lists them with its own filter
line, so the rail paints no search line of its own — `/` from the rail opens the
command palette, as `/` does everywhere else on the surface.

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
approval mode) plus the workspace this TUI starts from. The workspace inherits:
`cd project && future tui` opens on that **start directory** (so the first
message creates the conversation there), a launch from somewhere *ambient* — the
home directory, a temp directory, the filesystem root — reopens the workspace
chosen last time (`lastWorkspace` in `settings.json`, so `future tui` typed
wherever the terminal happened to sit no longer files conversations under a
directory named `tmp`), a launch from one of the app's own scratch directories
(or an explicit 对话会话 choice) is the **plain chat** — no workspace, filed
under the app's own `~/.future/workspaces/chat/<session id>` — and `/new`
(`ctrl+n`) keeps the workspace the conversation was just in (a chat is
not inherited: the next message mints its own chat directory). A remembered
workspace that no longer exists (a removed worktree) is skipped. The other three
chips name the product defaults a fresh screen opens on: the agent's default
model at the level that model starts at, and 完全访问 (full access). The first
message creates the session with exactly what the chips name, so a chip never
names something the session will not run under; there is no separate permission
level, because no other client offers one.

**One prefix: `ctrl`.** No default uses `alt` — on macOS the Option key composes
a character unless the reader turns on "Use Option as Meta", so an `alt`
shortcut is one that silently does nothing on the terminal this surface is
developed against — and no default is a function key (`F1`–`F12` need `Fn` on a
laptop and are absent on a compact keyboard).

The fifteen chords are `ctrl+c` stop the run in flight, `ctrl+n` new chat,
`ctrl+r` sessions, `ctrl+d` the rail, `ctrl+x` quit, `ctrl+e` the process detail
(answer only / whole process), `ctrl+p` commands, `ctrl+o` model, `ctrl+t`
effort (thinking), `ctrl+s` approval mode, `ctrl+l` workspaces, `ctrl+f` the
files panel, `ctrl+g` the git browser, `ctrl+v` paste, `ctrl+q` preferences.
Everything else is a typed verb or a
pointer target: `/delete`, `/queue`, `/copy`, `/mouse`, `/preview`, `/refresh`,
`/lang`, and `?` for help — the help overlay prints `—` in the key column for
those, and its command list names the door.

The map stops there because the letters do: a terminal sends Backspace, Tab,
`\n` and Enter for `ctrl+h`/`i`/`j`/`m`; `ctrl+a` and `ctrl+b` are the default
prefixes of `screen` and `tmux`; `ctrl+z` is suspend everywhere else; and
`ctrl+u`/`k`/`w`/`y` are the composer's own editing keys. The rail takes `d`
(the dock it slides out of) — the letter deleting a conversation gave up, since
a delete asks first and is rare enough to live on `/delete` and the rail's own
`delete` key.

`ctrl+c` is the one letter whose meaning is not ours. Raw mode turns `ISIG` off,
so the byte arrives as a key rather than as a signal, but the reader's
expectation of it comes from the shell: **interrupt the foreground turn**, not
close the shell. Bound to the run in flight it does exactly that, and with
nothing running it does nothing at all — no request, no notice. Leaving is
`ctrl+x`, a chord no other terminal program teaches, so it is never pressed by
reflex; quitting is also the lossless direction, since it leaves the session
and its run untouched on the agent. Neither is an overlay's to swallow: the quit
chord is checked before the overlay handles a key, and the interrupt chord stays
with the surface, so a modal can never take away the way out.

Every chord is rebindable in `/preferences`, and the help overlay
lists them grouped by what they do rather than in one flat list.

## Files and mentions

`@` completes files from the conversation's workspace into the draft. A path
with spaces is inserted quoted (`@"docs/my note.md"`), and submission reads that
as one path. The agent receives the files' contents in `<file>` blocks (at most
eight files, 256 KiB each); a file that is too large, binary or unreadable is
named with the reason instead of being silently dropped. Ordinary prose that
merely contains an `@` — an e-mail address, for example — is not a mention.
The same completion popup offers `#` for conversations.

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

The header is a two-row context line — the conversation's name and workspace, the
buttons that open the chat list and the two panel browsers, and the connection
state — with the hairline under it, leaving the rest of the frame to the
conversation. A row that runs short spends itself on buttons: the build number
goes first, then the brand, then the panel buttons from the tail, and the chat
list's button last of all — the rail is a keyboard surface that a narrow frame
does not put beside the conversation, so even a phone-sized terminal can open
the list of chats with a click.
`Ctrl+Q` or `/preferences` opens the preference panel, and the panel is the
list of everything this terminal client remembers: move the sidebar left or
right, adjust its 30–60-column width, show or hide it, control mouse capture and
completion notifications, enable reduced motion, pick the interface language and
the color theme, and rebind global shortcuts. Rows that step through a few values
change in place; the two that are menus — the language and the palette — open the
same pickers `/lang` and `/theme` do and close back onto the panel, on the row
that opened them. The panel keeps the selected row and help visible on short
terminals. Choices persist immediately in `~/.future/tui/settings.json` as
`sidebarPosition`, `sidebarWidth`, `sidebarVisible`, `mouseCapture`, `notify`,
`reducedMotion`, `language` and `themeId`; incremental shortcut overrides live
beside it in `keybindings.json`. A width the reader dragged is written back as it
was — the file's range is the divider's own 30–60, so the next start opens the
rail exactly as wide as it was left.

With no stored `sidebarVisible` — a first start, or a file written before the
key existed — the rail starts **hidden**: the conversation gets the whole
width, and `shift+←`/`shift+→` (which show the rail to reach
their panel) brings it back. Once the reader has shown or hidden it, that choice
is what the next start begins with, and `Restore defaults` lands back on the
hidden layout, hands the language back to the system and the palette back to the
terminal's light/dark hint.
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
ASCII letter or digit, F1–F24, Tab, navigation keys and Enter/Escape, and
`ctrl+↑`/`ctrl+↓` (the conversation step — a shifted arrow is not delivered by
every terminal, which is why the step is not on one).

`Ctrl+M`, `Ctrl+1` and `Ctrl+2` are delivered only by terminals that report
modifiers — the kitty keyboard protocol, or xterm's `modifyOtherKeys`, both of
which the TUI negotiates at start. A terminal that reports none sends `Ctrl+M`
and `Enter` as the same byte, and `Ctrl+1` as a plain `1`; there `/model` and a
click on the rail's tab strip are the doors to the same places.

## Keys and mouse

| Key | Action |
|---|---|
| `tab` / `shift+tab` | Conversation: step through everything that opens, in reading order — its **links and pictures** and the transcript's own **rows** (a folded reply, a burst of work, a tool call, a `▸ thinking` marker). The stop under the cursor is drawn in reverse video, and the transcript scrolls to it. `enter` opens it: a link goes to the browser / switches conversation / opens the preview box (see below); a **row** opens what it hides — a folded reply replays in place as the rows it stood for, under one header that turns from `▸` into `▾` (the row under it carries the rail on down, in the marker's own column); a burst reveals the calls it stood for; a tool call shows its body; a reasoning marker shows its text — and `enter` again (or a click on the header) folds them away. Every row of an opened fold still opens on its own, and what it opens nests one level in under a rail of its own; the answer stays outside the tree, because it is the reply's output rather than part of its process. Launchpad: move between the prompt and the parameter chips (`enter` cycles the chip under the cursor; `escape` or typing goes back to the prompt). The rail has its own chord (`ctrl+d`) |
| `←` `→` | Move the draft cursor — in every state, a focused rail included: the rail does not answer them, so a reader standing in the prompt never watches 最近 ↔ 工作区 switch under a caret that should have moved. The tabs are `shift+←`/`shift+→` |
| `↑` `↓` | Move through draft lines / submission history; in a focused rail, move the list cursor (a section caption is skipped, a workspace heading is a stop `⏎` folds). The conversation itself is switched by `ctrl+↑`/`ctrl+↓`, which needs no `⏎` |
| `shift+←` / `shift+→` | The rail's two tabs (最近 / 工作区), wherever the keys are: the rail is shown and takes the keys, and its cursor lands on the conversation on screen. Wraps |
| `ctrl+↑` / `ctrl+↓` | Switch to the previous / next conversation in the rail's order, wherever the keys are: the rail's cursor follows every step, and the transcript opens the one you stop on (80 ms after the last repeat) rather than each one you pass. A conversation you have already read comes back from memory with no wait at all. `shift+↑`/`shift+↓` are the same step — the older spelling, kept because a terminal that does not report the modifier sends a bare arrow for it — and they stop working if the reader binds the step to a chord of their own |
| `p` | Rails: pin / unpin the workspace the selected conversation belongs to |
| `/delete` or `delete` in the rail | Delete a conversation: the confirm box names the one that would go — the row under the rail's cursor when the rail has the keys, otherwise the one on screen — and `⏎` is what deletes it (`esc` keeps it). In a focused rail `delete` is the same door. The row that takes its place opens, and deleting the last one leaves the launchpad |
| `ctrl+j` / `ctrl+enter` | Insert a newline. `ctrl+j` is the one every terminal delivers; `ctrl+enter` arrives when the terminal reports modifiers (kitty keyboard protocol, xterm `modifyOtherKeys`, or a Windows Terminal `sendInput` binding), and `shift+enter` needs one of those too. Windows Terminal keeps `alt+enter` for fullscreen and sends plain Enter for `ctrl+enter` unless the reader binds it, e.g. `{"keys": "ctrl+enter", "command": {"action": "sendInput", "input": "\u001b[13;5u"}}` |
| `ctrl+w`, `alt+d`, `ctrl+u`, `ctrl+k`, `ctrl+y` | Delete a word, kill to line start/end, yank |
| `ctrl+z` / `ctrl+shift+z` (or `alt+y`) | Undo / redo draft edits |
| `pageUp` / `pageDown` | Scroll the conversation ten rows — always, including while the rail has the keys (`ctrl+d`): paging the transcript is the one thing these keys mean. The rail moves its own cursor with `↑`/`↓`, `ctrl+↑`/`↓` switches conversations, and the wheel over the rail scrolls it; `esc` returns to the latest output |
| `/queue` | List queued prompts — the ones sent while a run was in flight; `x` cancels, `r` / `enter` cancels and recalls into the draft (`ctrl+c` stops the active run, not the queue) |
| `ctrl+g` or `[ctrl+g] git` in the header | The repository browser: 提交 / 引用 / 改动 / 暂存 tabs, a lane-coloured graph log across every branch, and the selected row's patch. `1`–`4` pick a tab, `tab` switches list/detail, `PgUp`/`PgDn` page the patch, `y` copies the row |
| `ctrl+o` | Model picker (`/model`): every model the agent offers, grouped by provider, with the reasoning slider on its first row |
| `ctrl+f` or `/files` | The **files panel**: this conversation's workspace as a tree, in a box over the frame — the same session scope as `/git`. `↑`/`↓` walk it, `⏎`/`→` open, `⌫`/`←` go up a level, `r` re-reads the directory, `esc` closes. The header's `[ctrl+f] 文件` button is the pointer's door to it, and `[ctrl+g] git` sits beside it |
| `ctrl+v` | Read the clipboard: an image becomes a draft attachment, text is pasted. `/paste` does the same for terminals that bind the key themselves |
| `/preview` | Preview the newest image attached to the draft (no chord: the fifteen go to the actions pressed while working) |
| `+` / `-` / `0`, arrows, drag, wheel | In the preview box: zoom a picture (up to 6×), pan it, reset; scroll a document |
| `[` / `]` | In the preview box: previous/next picture in the same folder (the same action as its step buttons) |
| `esc` (in the preview box) | Back one level, not out of the tree: a file opened from the files panel or the `/git` browser returns to that panel, and only its own `esc` reaches the chat. A picture opened from the draft had nothing under it, so it closes onto the chat |
| `/copy` | Copy the last assistant answer as clean content, without terminal chrome |
| `/tool-output` | Tool call bodies: every call's output underneath its row (up to 200 rows per call), or folded away again. Collapsed, *every* call is one row — a failed one included: it carries a `✗` and no red band, and its reason appears with the other bodies. The `… N more lines` marker at the cap names it as `truncated` — this verb has no chord, and `ctrl+g` answers the git browser |
| `/mouse` | Toggle and persist mouse capture off/on for native terminal selection |
| 偏好设置 → 动态 | Toggle reduced motion (persisted; no chord) |
| `ctrl+t` | Thinking effort: opens the six-level chooser (`/thinking`); on the launchpad the level is staged for the session the first message creates |
| `ctrl+s` | Approval mode: opens the four-tier chooser, the desktop composer's own values — 手动审批 / 沙箱保护 / 自动审批 / 完全访问 |
| `ctrl+l` | Workspaces: opens the workspace chooser — every directory this client has a conversation in, a row that creates one from the path typed into its search line (`/workspace <path>` prefills that line), and a row that is a plain chat instead: no workspace at all, which mints `~/.future/workspaces/chat/<session id>` for the conversation and is filed under 对话会话 |
| `ctrl+d` | Sidebar: show + focus, then unfocus, then hide; full-screen on a narrow terminal. The rail's own `›` edge button does the same hide/show with one click. The show/hide choice is written to `sidebarVisible` and is where the next start begins (a first start begins hidden) |
| `/` or `ctrl+p` | Command palette (`/` requires an empty draft) — type to filter, `enter` to run |
| `ctrl+r`, or `[ctrl+r] 会话` in the header | Session picker (name, workspace and age of each conversation). The header's button is both the pointer's door to it and the place the key is spelled — the hint row leaves this one out, so the chord is not taught twice — and it is the one button a narrow frame keeps |
| `ctrl+q` or `/preferences` | Preferences: sidebar position, width and visibility, mouse capture, notifications, reduced motion, interface language, color theme and the shortcut editor. The key is also printed in the footer hint row, so it is on screen while the reader is wondering |
| `?` (or the full-width `？`) | Help overlay (effective keys, grouped by area, then the commands; the key requires an empty draft). Rows are listed only where an action has a chord — the verbs with none keep their typed door in the command list at the end of the same box |
| `/lang` | Interface language: opens the picker — follow the system, Chinese or English |
| `ctrl+e` | Process detail: two levels, the desktop's `replyDetail` — **answer only** (a settled reply keeps the answer it ended on, and the reasoning, calls and notes that produced it fold into one summary row above it; a reply that never wrote prose folds onto the slice it stopped at) and **whole process** (every slice keeps its own row, in the order it happened). The terminal opens on **answer only**, the desktop's default, and the level applies to the whole conversation at once — one press re-reads every reply, not the one under the cursor. Neither level shows a tool call's body or a reasoning block's text: a call is one row, a thought a `▸ thinking` marker (or a count in the summary row), and the details open one row at a time with `tab` + `enter` (or a click). A reply folds only once its run has ended (`agent_start` … `agent_end`); while it streams, no work folds at either level — every thought, call and piece of prose keeps its own row, so the transcript only grows or updates in place until `agent_end`, and `ctrl+e` never changes what a running reply looks like. The counts are words in the reader's language — `61 tool calls · 30 thoughts`, or `工具调用 61 次 · 思考 30 次` in Chinese, whose word order and counter (`次`) the templates carry — and so is the bare reasoning marker (`▸ 思考`); the summary row is drawn in the faintest colour the chrome has, one step under body text, so it sits behind the answer it stands above. History loaded from a session folds the same way, so a reopened conversation reads as one answer per reply. A view toggle for the run it is pressed in, not a preference: every start opens on the folded level |
| `ctrl+c` | Stop the run in flight — the shell's own meaning of the byte. With nothing running it is a no-op rather than an error |
| `ctrl+x` | Quit |

| Mouse | Action |
|---|---|
| Click a sidebar tab | Switch panel (`shift+←`/`shift+→` from the keyboard; a plain `←`/`→` is always the draft caret's) |
| Click the rail's `›` edge (or the `‹` it leaves behind) | Hide the rail / bring it back. `shift+←`/`shift+→` do the same from the keyboard |
| Click a session row | Open that conversation (a workspace heading only takes the keys) |
| Click a tree row | A file previews, a directory opens — the same two verbs `enter` runs |
| Click a link or a picture in the conversation | Open it — the same three answers `⏎` gives: browser, that conversation, or the preview box |
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

The driver only repaints dirty frames: there is no idle 120ms repaint timer. Streaming and spinner updates coalesce at 80ms, input and resize repaint immediately, and transcript rendering is cached by width/content revision. Within a frame only changed rows are written (a width or height change is a full repaint), so streamed tokens do not retransmit the screen. Reduced-motion does not schedule animation repaints; actual stream and state changes still repaint.

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
| Repository browser | `/git` (or `ctrl+g`) is a read-only tig-shaped panel with four tabs: 提交 (`git log --graph` across every branch, tag and remote, 200 commits, decorations), 引用 (local/remote branches and tags with upstream tracking), 改动 (the working tree, same porcelain read as `/diff`) and 暂存 (`git stash list`). The log is drawn as a lane graph — one colour per branch line, the selected commit a band across the list — and the detail beside it paints what it is: hunk headers, added and removed lines, the diffstat's bars, the commit's own header. Each row's detail loads on selection: a commit's stat and patch, a ref's upstream and tip, a file's patch, a stash's diff. `1`–`4` pick a tab, `tab` switches list/detail (a wide box shows both, from 108 columns), `↑`/`↓` walk the rows, `PgUp`/`PgDn` page the **detail** (it is the long pane, so it keeps those keys even while the list holds the cursor), `enter` opens (a ref filters the log to it, a changed file opens in the preview box, a commit moves the keys to its patch), `y` copies the sha/ref/path, `backspace` clears the filter, `r` re-reads. Nothing here stages, commits, checks out or drops a stash, and every query is bounded and killed past its deadline |
| History search and tree | `/history` filters the current `list_sessions` titles/first-message previews, workspaces and ids; `/tree` uses `parentSessionId`. Enter opens the selected session |
| Overlay lists | One record is one row: `up`/`down` move the highlight exactly one row, and a long title is shortened in place rather than folded (a wrapped record made one keypress skip two rows). Fields that no longer fit are dropped whole — a workspace cut mid-path would read as a different directory — so a narrow box shows title, then title · workspace, then the id. `pageUp`/`pageDown` move the cursor by one screenful (a detail card scrolls instead, and the diff follows whichever pane has the keys) |
| `/worktree` rows | `›` is the selection; `●` marks the worktree the session is already in, so choosing one is visible even when the first row is current |
| Fork | `/fork` lists real user-message points from `get_fork_messages`; enter calls `fork` and opens the child |
| Worktrees | `/worktree` lists real git worktrees and enter sets the session cwd. `/worktree new <branch>` creates under `.worktrees/` and moves the session. No remove/prune/reset/clean operations |
| Existing settings | `/mode` (or `/sandbox`) opens the approval-mode chooser, `/thinking` the reasoning chooser, `/compact [instructions]`, `/tools a,b` or `/tools off`, `/providers`; `/skills` and `/model` retain their existing surfaces |
| Sessions panel | `list_sessions`, newest first: 最近 split into a 活跃 stretch (a run still in flight, spinner in the row's state slot) and 更早, the headings drawn only while both are non-empty. The list is re-read every five seconds while the rail is on screen, because another client's run reaches this surface no other way. 工作区 filed by directory with the pinned ones leading. Pins persist in `settings.json` (`pinnedWorkspaces`) |
| Usage / context / cost | `get_state`'s usage block, refreshed on the same events |
| `ctrl+n` / `/new`, `ctrl+c` stop, `/refresh` | |
| Delete a conversation (`/delete`, or `delete` in the rail) | One confirm box, then `delete_session` for the id it named. The successor in the list opens, so the transcript never stays on a conversation that is gone; deleting the last one detaches to the launchpad. A refusal (the agent keeps a session a compaction holds) leaves everything as it was and says so in the notice row |
| Launchpad chips | Click 工作区 / 模型 / 强度 / 审批 to cycle them. On the launchpad the workspace, model, effort and approval choices are staged for the session the first message creates (the agent has no session to set them on yet); with a session attached they go over the wire. The workspace defaults: the directory `future tui` was started from (`cd project && future tui`), or 对话会话 when launched from one of the app's scratch directories or chosen explicitly — a chat mints `~/.future/workspaces/chat/<session id>` when the session is created; `/new` inherits the workspace the previous conversation ran in (a chat is not inherited). The model and effort chips name the product defaults — the agent's default model at the level it starts at — and approval 完全访问; the first message creates the session with exactly that. 工作区 cycles 对话会话 and the directories this client has conversations in, and the notice row names the full path it chose |
| Theme | Persisted in `~/.future/tui/settings.json` (`tui/src/settings.rs`); the reading level is deliberately not — `ctrl+e` is per run |

## What is not here yet

- **A preview is a preview.** The picture is placed over the cells the box has, at up to 6× the fitted size. For pixel-level work open it in the system image viewer; the TUI decodes locally and sends the picture nowhere.
- **The command palette** lists the commands the adapter implements. Provider
  and tool management is intentionally compact (`/providers` and `/tools`);
  `/skills` remains the installed-skills list in the palette. Approval allow
  rules are runtime-only; persistent approval-rule file editing remains in the
  desktop.
- Further enhancements: timeline rail, full-screen session manager and
  full-screen draft editor.

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
