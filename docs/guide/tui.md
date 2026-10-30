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
 queue band (only while prompts wait)      │
 input box (❯ multiline draft, grows)      │  (the rail runs down beside it)
 activity: ⠋ 思考 … · 3s · 2 工具                   42 tok/s · 缓存 96%
 status:   model · effort · approval   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 hint:     Esc 回到聊天 · ctrl+d 侧栏 · ctrl+p 命令 · ctrl+q 偏好设置
```

The hint row teaches keys whose door is not already on screen: `ctrl+r` is not on
it, because the header's own `[ctrl+r] 会话` button spells that chord a row
above, and `ctrl+f`/`ctrl+g` ride on their buttons the same way. Nothing a
reader needs is taught only in the help box, and nothing is taught twice.

Between the conversation and the input box sits the **queue band**, and only
while something is in it. A prompt sent while a run is in flight is not sent:
the agent accepted it and put it behind the running turn, so the band lists it —
one row per prompt, in the order it was queued — and the transcript stays silent
about it until it goes out. That is the whole point of the band: a queued prompt
drawn as a normal user card reads as a message that went out, and `hi` appearing
mid-reply is exactly what a reader cannot tell apart from a sent message.

The band is a box, like the composer under it: two rules, the interior on the
panel background the rail uses. Bare dimmed lines on the canvas read as a
continuation of the transcript, which is what made the first version look pasted
over the conversation. Its title is the count and when the prompts go out
(`排队 2 条 · 当前回复结束后一起发送`), and the right end of that rule is the door
to `/queue`, which cancels (`x`) or recalls (`r` / `enter`) one row at a time;
clicking a row in the band is the same verb as clicking it in that box — the
prompt goes back into the draft and leaves the queue. A long queue shows four
rows and counts the rest, and the band takes its rows from the conversation
rather than growing the frame — the conversation keeps its own bottom, so the
newest rows stay on screen above it (see the `↑ 更早` note below).

**At the boundary they go out together.** The composer submits with
`enqueue_coalescing`: when the running turn ends, the prompts waiting there are
folded into **one** message, in queue order, and answered together — a burst of
supplements is a follow-up, not three turns. Until that moment each one is its
own row (its own `cancel_queued_run`), which is what makes the band a queue a
reader can manage; the fold happens once, at the boundary, and the transcript
then shows the message that actually ran, carrying every text.

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
is sent with the prompt the same way a dragged-in file is; a click on that count
opens the newest attached image (the count is the picture's door — there is no
chord and no typed verb for it, because a `[Image #N]` marker ahead of the verb
would turn the line into a prompt). `/paste` is the same verb from the palette —
it is there
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
or the shown crop changes), not a character mosaic. The preview box (`enter` on an image in 文件, or a
click on the count the composer's title shows for a draft attachment) puts its controls in the frame's first row —
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
less than the word. The name is measured on its own row, so a short one hands
the cells it does not need back to that row's title, and it gets at most a third
of the row with a space always kept between it and the conversation's name: a
longer one (`session-supervisor`) is ellipsised rather than allowed to crowd the
title out, since the title is what the row is for. The `·` and the ages still
line up down the list. **工作区** files the same conversations the way the
desktop's rail does: 置顶 (the conversations pinned with `/pin`, lifted out of
the workspace that would otherwise file them), then 工作区 (one heading per
workspace, its conversations beneath), then 对话会话 (conversations that belong
to no workspace — the app's own scratch directories). Usage figures
live on the status bar, so no panel repeats them; skills are the palette's. The rail
never *holds* the keyboard — there is no focus to step into or out of — so every
key that acts on it carries `shift` (`shift+↕` steps, `shift+pageUp/Down` pages,
`shift+←/→` walks the tabs, plus `ctrl+1`/`ctrl+2` to name one outright) and the
unmodified keys stay where they were: `↕` and the page keys scroll the
conversation, `←`/`→` move the draft caret.

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

The chords are `ctrl+c` stop the run in flight, `ctrl+n` new chat, `ctrl+r`
sessions, `ctrl+d` the rail, `ctrl+x` quit, `ctrl+p` commands, `ctrl+o` model,
`ctrl+t` effort (thinking), `ctrl+s` approval mode, `ctrl+l` workspaces,
`ctrl+f` the files panel, `ctrl+g` the git browser, `ctrl+v` paste, `ctrl+q`
preferences, and `ctrl+1`/`ctrl+2` the rail's two tabs. Everything else is a
typed verb or a pointer target: `/delete`, `/pin`, `/rename`, `/queue`,
`/copy`, `/refresh`, `/reply-detail`, and `?` for help — the help overlay
lists only the actions that have a chord, and the palette names the door for
the rest.

**A palette row is a verb whose typed spelling is the only way in.** A verb
that already answers to a key — a chord (`ctrl+g`), a row in the preferences
box (`ctrl+q` holds the language and the theme) or its own printable key (`?`)
— is not listed: the menu is where a reader looks for what they cannot find
anywhere else, and a second name for something already on the screen only
pushes a verb that has no other door below the fold. So the palette offers
sixteen verbs — listed alphabetically, from `/approval` to `/worktree`, because a
reader opens the menu holding the name of the verb they want — and not `/new`,
`/stop`, `/paste`, `/git`, `/files`, `/mode`, `/sessions`, `/model`,
`/thinking`, `/workspace`, `/quit`, `/help`, `/theme`, `/lang`, `/mouse`,
`/preferences` or the older `/sandbox`. Unlisted is not unhandled: every one of
them still works when typed, so a reader with an old spelling in their fingers
reaches the verb instead of sending `/theme` to the agent as a prompt.

The map stops there because the letters do: a terminal sends Backspace, Tab,
`\n` and Enter for `ctrl+h`/`i`/`j`/`m`; `ctrl+a` and `ctrl+b` are the default
prefixes of `screen` and `tmux`; `ctrl+z` is suspend everywhere else; and
`ctrl+u`/`k`/`w`/`y` are the composer's own editing keys. `ctrl+e` is one of
them too — the shell's end of line — and the map gave it back: pressing it used
to re-lay out the whole transcript, which no caret nudge should ever do (the
reading level it toggled is `/reply-detail` now). The rail takes `d`
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
completion notifications, pick the interface language and
the color theme, and rebind global shortcuts. Rows that step through a few values
change in place; the two that are menus — the language and the palette — open the
same pickers `/lang` and `/theme` do and close back onto the panel, on the row
that opened them. The panel keeps the selected row and help visible on short
terminals. Choices persist immediately in `~/.future/tui/settings.json` as
`sidebarPosition`, `sidebarWidth`, `sidebarVisible`, `mouseCapture`, `notify`,
`language` and `themeId`; incremental shortcut overrides live
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
disables styling by variable presence, including an empty value. These bindings, plus copy,
queue and mouse, are contextual global actions and can be rebound; overlays
keep their local keys so a global shortcut never submits an approval or edits
a picker's search line accidentally.

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
overlay groups them (chats / choices / panels / interface / inline keys) instead
of printing one flat list. The verbs are not in it: the palette is one key away
and already lists them with their descriptions. In the shortcut
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
| `tab` / `shift+tab` | Conversation: step through everything that opens, in reading order — its **links and pictures**, the two **controls** under each message (see below) and the transcript's own **rows** (a folded reply, a burst of work, a tool call, a `▸ thinking` marker). The stop under the cursor is drawn in reverse video, and the transcript scrolls to it — but only when a press steps off the window's edge: with no cursor yet, or after a scroll took the cursor out of view, the walk starts at the **window's** first stop (the last one going back), not at the conversation's first link, and a press that stays inside the page moves no page. A stop the press steps onto above the window is brought up to its top row, one below it down to its last readable row; the frame that follows the press is the only one that scrolls, so the page can be scrolled away from the highlight afterwards. `enter` opens it: a link goes to the browser / switches conversation / opens the preview box (see below); a **row** opens what it hides — a folded reply replays in place as the rows it stood for, under one header that turns from `▸` into `▾` (the row under it carries the rail on down, in the marker's own column); a burst reveals the calls it stood for; a tool call shows its body (up to 200 rows per call, and the `… N more lines` marker at the cap names the rest as `truncated`) — and an `edit` call's body is the patch it makes: the replaced lines in red, the replacements in green, up to three lines of the unchanged text either side (with a `… N lines` marker where more was trimmed away), `@@ n/m` between the changes of a batch, and the row itself carrying the `+N -M` count before it is even opened; a reasoning marker shows its text — and `enter` again (or a click on the header) folds them away. Every row of an opened fold still opens on its own, and what it opens nests one level in under a rail of its own; the answer stays outside the tree, because it is the reply's output rather than part of its process. The tree keeps its own geometry: the rail runs unbroken through the blank rows inside it (a reasoning block's paragraph gap carries the rail and nothing else), and a call's background band starts after the rail instead of across it. Opening changes only what is below the row pressed — that row, and everything above it, stays on the line it was on. Launchpad: move between the prompt and the parameter chips (`enter` cycles the chip under the cursor; `escape` or typing goes back to the prompt). The rail has its own chord (`ctrl+d`) |
| `←` `→` | Move the draft cursor. They never switch a tab: that is `shift+←`/`shift+→` or `ctrl+1`/`ctrl+2` |
| `home` / `end`, `ctrl+a` / `ctrl+e` | Move the draft caret to the start / end. `home` and `end` are line-aware in a multi-line draft; `ctrl+a` and `ctrl+e` are the whole value, which is the shell's own meaning of the two bytes (`end` does the single-line case too) |
| `↑` `↓` | Scroll the conversation a row at a time, always: the transcript is the one thing these keys belong to, and the draft does not take them (a prompt is edited with `←`/`→`, `home`/`end` and the chords below) |
| `shift+←` / `shift+→` | Cycle the rail's two tabs (最近 / 工作区), from anywhere: a hidden rail is shown, and its cursor lands on the conversation on screen. Wraps in both directions |
| `ctrl+1` / `ctrl+2` | Name a tab outright: 最近 / 工作区, in the order the strip paints them (needs a terminal that reports modifiers — `shift+←`/`→` walk the same two tabs everywhere) |
| `shift+↑` / `shift+↓` | Walk the rail, from anywhere. In 最近 every row is a conversation, so each step switches it — the cursor follows, and the transcript opens the one you stop on (80 ms after the last repeat) rather than each one you pass, with the agent's read running beside the surface: the rail, a streaming reply and the keyboard stay live while it answers, and a conversation you have already read comes back from memory at once. A click on a row takes the same road — the read runs beside the surface, never in front of it. The two conversations either side of the one you settle on are read ahead in the background, so the next step finds its page already materialized (measured at a reading pace: **18 ms instead of 120 ms** a step). In 工作区 the cursor also stops on the workspace headings, and a step that lands on one switches nothing: `enter` folds or unfolds it there. One action, one chord: `ctrl+↑`/`ctrl+↓` is not a step — macOS gives it to Mission Control, so the key never reaches the terminal there |
| `shift+pageUp` / `shift+pageDown` | Page the rail's list — the same scroll the wheel over it does. The rail's cursor (the conversation on screen) does not move |
| `p` | A letter in the draft: pinning is `/pin`, with no chord of its own |
| `/delete` | Delete a conversation: the confirm box names the one that would go — the row under the rail's cursor when the rail is on screen, otherwise the one on screen — and `⏎` is what deletes it (`esc` keeps it). The row that takes its place opens, and deleting the last one leaves the launchpad. The `delete` key is the draft's forward-delete, so it is not a door to this |
| `/pin` | Pin or unpin the conversation under the rail's cursor (the 置顶 section of 工作区; a pinned row is drawn there instead of under its workspace, and 最近 keeps it where it was). No chord: `p` is a letter in the draft. Nothing to pin on the launchpad — the conversation the verb is about is the one the first message creates, and it does not exist yet — so the notice row says so. The verb takes no argument, so a bare `/pin` is the pin and **anything after it is a prompt**: `/pin 这个文件` sent from the draft is a message to the agent, not a pin that swallows the sentence |
| `/rename 新标题` | Rename the conversation on screen. The words after the verb are the whole name (`/rename 修 parser`), and it is written to the agent's own `set_session_name` — the title the desktop and the phone edit, in `session_info` beside the conversation — so every client reads the same one. The header and the rail's row take the new name from the request itself rather than a re-read, which is why a rename costs one round trip. A bare `/rename` answers with the usage line: the name is what is being changed, so there is nothing to guess. No chord — `/` is the door |
| `ctrl+j` / `ctrl+enter` | Insert a newline. `ctrl+j` is the one every terminal delivers; `ctrl+enter` arrives when the terminal reports modifiers (kitty keyboard protocol, xterm `modifyOtherKeys`, or a Windows Terminal `sendInput` binding), and `shift+enter` needs one of those too. Windows Terminal keeps `alt+enter` for fullscreen and sends plain Enter for `ctrl+enter` unless the reader binds it, e.g. `{"keys": "ctrl+enter", "command": {"action": "sendInput", "input": "\u001b[13;5u"}}` |
| `ctrl+w`, `alt+d`, `ctrl+u`, `ctrl+k`, `ctrl+y` | Delete a word, kill to line start/end, yank |
| `ctrl+z` / `ctrl+shift+z` (or `alt+y`) | Undo / redo draft edits |
| `pageUp` / `pageDown` | Scroll the conversation a screen at a time. The rail pages with `shift+pageUp`/`shift+pageDown`; `esc` returns to the latest output. A conversation opens on its **last three exchanges** and scrolling past the top reads ten more at a time: a long history is not read to open a conversation, it is read as it is scrolled |
| Conversation that fits | A conversation no taller than the band does not scroll at all — up asks the app for an older page and otherwise leaves the frame alone, because moving it would paint blank rows above the transcript and hide rows the terminal was tall enough to show. `↑ 更早` appears once the oldest loaded row is *above* the band, and the ceiling is that row reaching the top of it. The marker takes the window's **oldest** row, never the newest: a scrolled window used to pay for that extra row by dropping the last message off the bottom, which is the one row a reader following the tail cannot do without |

| Reading above the tail | A reader parked above the newest row keeps their place while the transcript changes height on its own: rows streaming in at the tail, a reply settling into its `▸ 4 tool calls · 2 thoughts` summary, a burst of calls collapsing into one row, a page of older rows arrived above — the band stays on the rows it was showing, and `↓ 新输出` (then `esc`) is how the tail is reached again. The window is a place in the conversation, not a distance from its end; a resize is the one change that rewraps every row, so it is left to keep that distance |
| `enter` | Send the draft. With an **empty** draft there is nothing to send, so it opens the rail's selected row instead: a workspace heading folds or unfolds, a conversation opens. That is the one key the rail shares with the composer, and the empty draft is what tells the two apart |
| `/queue` | List queued prompts — the ones sent while a run was in flight, also listed one row at a time in the band above the composer; `x` cancels, `r` / `enter` cancels and recalls into the draft (`ctrl+c` stops the active run, not the queue) |
| `ctrl+g` or `[ctrl+g] git` in the header | The repository browser: 提交 / 引用 / 改动 / 暂存 tabs, a log across every branch with each row's date-time and committer in columns beside a lane-coloured graph, and the selected row's patch. `1`–`4` pick a tab, `tab` switches list/detail, `PgUp`/`PgDn` page the patch, `y` copies the row. The repository is read beside the surface, not in front of it: opening the panel, changing tab or narrowing its filter keeps the rail, a streaming reply and the keyboard live, and the panel paints its previous rows until the new ones arrive |
| `ctrl+o` | Model picker (`/model`): every model the agent offers, grouped by provider, with the reasoning slider on its first row |
| `ctrl+f` or `/files` | The **files panel**: this conversation's workspace as a tree, in a box over the frame — the same session scope as `/git`. `↑`/`↓` walk it, `⏎`/`→` open, `⌫`/`←` go up a level, `r` re-reads the directory, `esc` closes. The header's `[ctrl+f] 文件` button is the pointer's door to it, and `[ctrl+g] git` sits beside it |
| `ctrl+v` | Read the clipboard: an image becomes a draft attachment, text is pasted. `/paste` does the same for terminals that bind the key themselves |
| a click on `· 📎 N images` in the composer's title | Preview the newest image attached to the draft. The count is the only door: the chords go to the actions pressed while working, and a typed verb cannot be it — the draft's text starts with the `[Image #N]` marker the paste wrote, so `/preview` after pasting was sent to the agent as a prompt |
| `+` / `-` / `0`, arrows, drag, wheel | In the preview box: zoom a picture (up to 6×), pan it, reset; scroll a document |
| `[` / `]` | In the preview box: previous/next picture in the same folder (the same action as its step buttons) |
| `esc` (in the preview box) | Back one level, not out of the tree: a file opened from the files panel or the `/git` browser returns to that panel, and only its own `esc` reaches the chat. A picture opened from the draft had nothing under it, so it closes onto the chat |
| `/copy` | Copy the last assistant answer as clean content, without terminal chrome |
| `[复制]` under a message (or `tab` to it and `enter`) | Copy **that** message's own text: a prompt back into the draft, a reply quoted elsewhere. Every message the reader wrote or the model wrote ends in this row — the desktop's per-message copy button, made permanent because a terminal has no hover to reveal it under — and a click anywhere on the chip is the same press. A reply that only called tools has nothing to copy, so it draws no copy chip |
| `[分支]` under a reply (or `tab` to it and `enter`) | Fork the conversation at the **user message that produced this reply** — the desktop's fork button, which branches a question rather than a reply. The chip waits until the reply has settled (a streaming reply has none), and it knows the point because the agent announces the journal entry it wrote for each accepted prompt (`user_message`); a prompt that is somehow not persisted yet answers "this message is not in the session record yet" instead of forking somewhere else. The same `fork` call `/fork` makes, with the point already chosen |
| `/mouse` | Toggle and persist mouse capture off/on for native terminal selection (the 鼠标捕获 row in `ctrl+q` is the same switch) |
| `ctrl+t` | Thinking effort: opens the six-level chooser (`/thinking`); on the launchpad the level is staged for the session the first message creates |
| `ctrl+s` | Approval mode: opens the four-tier chooser, the desktop composer's own values — 手动审批 / 沙箱保护 / 自动审批 / 完全访问 |
| `ctrl+l` | Workspaces: opens the workspace chooser — every directory this client has a conversation in, a row that creates one from the path typed into its search line (`/workspace <path>` prefills that line), and a row that is a plain chat instead: no workspace at all, which mints `~/.future/workspaces/chat/<session id>` for the conversation and is filed under 对话会话 |
| `ctrl+d` | Sidebar: show / hide (it never takes the keyboard, so there is no third state); full-screen on a narrow terminal. The rail's own `›` edge button does the same with one click. The choice is written to `sidebarVisible` and is where the next start begins (a first start begins hidden) |
| `/` or `ctrl+p` | Command palette (`/` requires an empty draft) — type to filter, `enter` to run. It lists the verbs with no other door (see "A palette row is a verb…" above); one whose chord or preferences row already names it is typed-only and does not spend a row here. From 96 columns the box draws two columns: the surface's verbs on the left, the agent's skills on the right, each under its own heading. `←`/`→` step between them (landing on the neighbouring column's row at the same line); `↑`/`↓`, `tab` and `enter` still walk one list — commands first, then skills. Narrower frames keep the single column with a heading per kind |
| `ctrl+r`, or `[ctrl+r] 会话` in the header | Session picker (name, workspace and age of each conversation). Its search line filters title, workspace and id, so this is where a reader finds a conversation by name — the rail paints no search line of its own. The header's button is both the pointer's door to it and the place the key is spelled — the hint row leaves this one out, so the chord is not taught twice — and it is the one button a narrow frame keeps |
| `ctrl+q` or `/preferences` | Preferences, in the order the box lists them: interface language and color theme first (the two rows that repaint the whole surface — the language leads, because every other label there is written in whatever it says), then sidebar position, width and visibility, mouse capture, notifications, the shortcut editor and restore defaults. The key is also printed in the footer hint row, so it is on screen while the reader is wondering |
| `?` (or the full-width `？`) | Help overlay (effective keys, grouped by area; the key requires an empty draft). Rows are listed only where an action has a chord — a verb with none is the palette's (`/` or `ctrl+p`), which is named in the same box and prints the list once |
| `/lang` | Interface language: opens the picker — follow the system, Chinese or English. Its listed door is the first row of `ctrl+q`, so the palette does not repeat it |
| `/reply-detail` | Process detail (no chord — `ctrl+e` is the draft's end of line): two levels, the desktop's `replyDetail` — **answer only** (a settled reply keeps the answer it ended on, and the reasoning, calls and notes that produced it fold into one summary row above it; a reply that never wrote prose folds onto the slice it stopped at) and **whole process** (every slice keeps its own row, in the order it happened). The terminal opens on **answer only**, the desktop's default, and the level applies to the whole conversation at once — one press re-reads every reply, not the one under the cursor. Neither level shows a tool call's body or a reasoning block's text: a call is one row, a thought a `▸ thinking` marker (or a count in the summary row), and the details open one row at a time with `tab` + `enter` (or a click). A reply folds only once its run has ended (`agent_start` … `agent_end`); while it streams, no work folds at either level — every thought, call and piece of prose keeps its own row, so the transcript only grows or updates in place until `agent_end`, and the level never changes what a running reply looks like. The counts are words in the reader's language — `61 tool calls · 30 thoughts`, or `工具调用 61 次 · 思考 30 次` in Chinese, whose word order and counter (`次`) the templates carry — and so is the bare reasoning marker (`▸ 思考`); the summary row is drawn in the faintest colour the chrome has, one step under body text, so it sits behind the answer it stands above. History loaded from a session folds the same way, so a reopened conversation reads as one answer per reply. A view toggle for the run it is used in, not a preference: every start opens on the folded level |
| `ctrl+c` | Stop the run in flight — the shell's own meaning of the byte. With nothing running it is a no-op rather than an error |
| `ctrl+x` | Quit |

| Mouse | Action |
|---|---|
| Click a sidebar tab | Switch panel (`ctrl+1`/`ctrl+2` or `shift+←`/`shift+→` from the keyboard; a plain `←`/`→` is always the draft caret's) |
| Click the rail's `›` edge (or the `‹` it leaves behind) | Hide the rail / bring it back. `shift+←`/`shift+→` do the same from the keyboard |
| Click a session row | Open that conversation (a workspace heading folds its group) |
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
| Click a queued prompt | Recall it into the draft, exactly as `enter` does there — the band above the composer and the `/queue` box speak the same verb |
| Click the queue band's header | Open `/queue` (the count is the door to the manager) |
| Click the page around an overlay | Close it; a click on the box itself is absorbed, never passed to the control behind it |
| Click the `[× 关闭]` chip | Close the panel — every box paints one at the right end of its top border, and it runs the very handler `esc` does: one level out for the preview (back to the panel the file came from), back to the preferences list from the shortcut editor, and out for everything else. A reader with no keyboard has no other way out, which is why the chip is on all of them, the full-screen `/git` and 文件 panels included |

Mouse reporting is SGR 1006 and starts with the persisted preference (enabled by default). `/mouse` releases and restores it at runtime and saves the choice; the terminal is always restored on exit. Draft text, cursor, folded pastes and image attachments are kept per session across overlays, launchpad visits and session switches (for the lifetime of this TUI process, not persisted to disk).

The conversation follows new output by default. Scrolling up detaches from the tail; further output leaves the reading position unchanged and shows a new-output badge. Scroll back to the bottom or press `esc` to follow again.

The driver only repaints dirty frames: there is no idle 120ms repaint timer. Streaming and spinner updates coalesce at 80ms, input and resize repaint immediately, and transcript rendering is cached by width/content revision. Within a frame only changed rows are written (a width or height change is a full repaint), so streamed tokens do not retransmit the screen. Actual stream and state changes still repaint.

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
| Prompting and streaming | `prompt` with `enqueue_coalescing` — a submission behind a running turn waits in the agent's queue, and the prompts waiting at the turn boundary go out as one message (`start_next_scheduled` joins their texts in queue order); `text_chunk`, `thinking_*`, `tool_*` and `agent_end` drive the transcript |
| Queue management | `/queue` and the band above the composer both read `get_state.queuedRuns`; cancel and recall use the existing `cancel_queued_run`, with no second client-side queue. A queued submission draws no transcript rows until it goes out; at the boundary the run's card grows by the texts it absorbed (a `merged` ack), and a submission the agent reports as never sent (`cancelled`, `superseded`, `session_deleted`) drops its card |
| Draft attachments | Reuses `components/input.rs` folded-paste and image-marker machinery; attachments travel with the prompt. `ctrl+v` / `/paste` read the system clipboard through `paste::ClipboardCapture` (osascript on macOS, PowerShell `-STA` on Windows, `wl-paste`/`xclip` on Linux) on a blocking thread, and an image it writes becomes an `[Image #N]` attachment. Nothing leaves the machine |
| File panel and preview | `r` re-reads the directory in the panel; a directory is read when it opens (400 entries per level, said out loud when cut). A preview decodes an image to at most 2048 px on its long edge — those pixels are what keeps a zoom sharp — and downsamples the bitmap it transmits to the cells on screen; text is read bounded and cut on a character boundary |
| Approvals | A parked request opens a full card with tool, kind, risk, action/paths, requested action and sandbox boundary. `y` / `n` allow once or deny; `a` first adds the agent-suggested **this-run** workspace rule, then approves. A rule failure leaves the request pending. Escape closes the card without deciding |
| Diff review | `d` in the repository browser (`/git`, `ctrl+g`) opens the box — there is no `/diff` verb any more: the 改动 tab is where the same question is asked, so the review is a detail of the browser rather than a second door to it. It lists tracked and untracked changes from the session workspace and shows the selected real git patch. Wide terminals keep a fixed file navigator beside an independently scrolling patch. Narrow terminals switch between files and patch with `tab`; `up`/`down` operate the focused view and left/right choose a file. The arrow follows the cursor immediately, and `·` marks the file whose patch is already loaded while the reload is in flight. `r` refreshes. This is the current uncommitted working tree, not per-turn attribution; patches are preview-bounded |
| Repository browser | `/git` (or `ctrl+g`) is a read-only tig-shaped panel with four tabs: 提交 (`git log` across every branch, tag and remote in topological order, 200 commits, decorations), 引用 (local/remote branches and tags with upstream tracking), 改动 (the working tree, the same porcelain read as the diff review) and 暂存 (`git stash list`). The log is drawn as a lane graph the panel lays out itself — one row per commit (no filler rows for the drawing), verticals and elbows instead of diagonals, a horizontal that breaks around a lane still running rather than striking through it, one colour per branch line, the selected commit a band across the list — with the committer's date-time (minute precision, no zone) and name in fixed-width columns before it, so a lane sits in the same cell in every row. The columns and the graph are separated by a single space. A branch that starts where the one before it ends hangs off the same cell (`●─┤`), so a run of short branches stays two columns wide instead of walking right by two at every merge. A branch another lane already draws — `origin/main` merged into every feature, a branch merged twice — gets no lane of its own: the link is dropped and the page laid out again, because that lane would run to where the other lane lands carrying no commit of its own. A row carries the refs a commit sits on, then the subject — which line of work it belongs to, before what it did; the commit id is not repeated in it because the detail pane prints the whole hash, and a box too narrow for the columns *and* a readable subject drops the committer first and the date-time second, never half a column and the detail beside it paints what it is: hunk headers, added and removed lines, the diffstat's bars, the commit's own header. Each row's detail loads on selection: a commit's stat and patch, a ref's upstream and tip, a file's patch, a stash's diff. `1`–`4` pick a tab, `tab` switches list/detail (a wide box shows both, from 108 columns), `↑`/`↓` walk the rows, `PgUp`/`PgDn` page the **detail** (it is the long pane, so it keeps those keys even while the list holds the cursor), `enter` opens (a ref filters the log to it, a changed file opens in the preview box, a commit moves the keys to its patch), `y` copies the sha/ref/path, `backspace` clears the filter, `r` re-reads. Nothing here stages, commits, checks out or drops a stash, and every query is bounded and killed past its deadline |
| Overlay lists | One record is one row: `up`/`down` move the highlight exactly one row, and a long title is shortened in place rather than folded (a wrapped record made one keypress skip two rows). Fields that no longer fit are dropped whole — a workspace cut mid-path would read as a different directory — so a narrow box shows title, then title · workspace, then the id. `pageUp`/`pageDown` move the cursor by one screenful (a detail card scrolls instead, and the diff follows whichever pane has the keys) |
| `/worktree` rows | `›` is the selection; `●` marks the worktree the session is already in, so choosing one is visible even when the first row is current |
| Fork | `/fork` lists real user-message points from `get_fork_messages`; enter calls `fork` and opens the child. A reply's `[分支]` control is the same call with the point already chosen — it forks at the prompt that produced that reply |
| Export (`/export`) | The agent writes the conversation to one self-contained HTML file: markdown becomes real headings, lists, tables and code blocks; thinking and tool calls start folded (the toolbar expands or collapses them all), and each tool call carries its full input and output with failures marked; the stylesheet is inline and pictures are embedded as `data:` URLs, so nothing is fetched. Light and dark follow the system, with a toggle in the toolbar. `future session export` writes the same file |
| Worktrees | `/worktree` lists real git worktrees and enter sets the session cwd. `/worktree new <branch>` creates under `.worktrees/` and moves the session. No remove/prune/reset/clean operations |
| Existing settings | `/mode` opens the approval-mode chooser, `/thinking` the reasoning chooser, `/compact [instructions]`, `/tools a,b` or `/tools off`. Every chooser takes the words after its verb as its search line (`/model deepseek` opens the list filtered to it). `/compact`, `/export` and `/tools` name the conversation on screen, so on the launchpad they say so instead of acting on the one the reader left — `/tools` is staged and applied to the session the first message creates |
| Manual compaction (`/compact`) | The agent acknowledges and summarises on its own worker, so the window in between is the surface's to show: the activity row reads `compacting` with its clock (a compaction makes no tool calls, so the tally is dropped), the composer's title says sending is paused, and `enter` on a prompt leaves the draft where it is instead of sending one the agent would refuse (`session_busy`). `get_state`'s `isCompacting` is the agent's own flag and is what ends the row if the terminal event is missed (an attach into a compaction already running). `compaction_committed` reports the checkpoint's two counts in the notice row (`62203 → 33281 tokens`) and writes the divider into the transcript, so the cut is visible without a reload — the same row a restart builds from the stored checkpoint. `compaction_unchanged` says there was nothing to compact, `compaction_failed` carries the agent's error |
| Sessions panel | `list_sessions`, newest first: 最近 split into a 活跃 stretch (a run still in flight, spinner in the row's state slot) and 更早, the headings drawn only while both are non-empty. The list is re-read every five seconds while the rail is on screen, because another client's run reaches this surface no other way. 工作区 filed by directory, with the conversations pinned by `/pin` in their own 置顶 block instead of under their workspace (a group left with nothing has no heading). A pin is this client's own choice and persists in `settings.json` (`pinnedSessions`, by session id) |
| Usage / context / cost | `get_state`'s usage block, refreshed on the same events |
| `ctrl+n` / `/new`, `ctrl+c` stop, `/refresh` | |
| Delete a conversation (`/delete`, or `delete` in the rail) | One confirm box, then `delete_session` for the id it named. The successor in the list opens, so the transcript never stays on a conversation that is gone; deleting the last one detaches to the launchpad. A refusal (the agent keeps a session a compaction holds) leaves everything as it was and says so in the notice row |
| Launchpad chips | Click 工作区 / 模型 / 强度 / 审批 to cycle them. On the launchpad the workspace, model, effort and approval choices are staged for the session the first message creates (the agent has no session to set them on yet); with a session attached they go over the wire. The workspace defaults: the directory `future tui` was started from (`cd project && future tui`), or 对话会话 when launched from one of the app's scratch directories or chosen explicitly — a chat mints `~/.future/workspaces/chat/<session id>` when the session is created; `/new` inherits the workspace the previous conversation ran in (a chat is not inherited). The model and effort chips name the product defaults — the agent's default model at the level it starts at — and approval 完全访问; the first message creates the session with exactly that. 工作区 cycles 对话会话 and the directories this client has conversations in, and the notice row names the full path it chose |
| Theme | Persisted in `~/.future/tui/settings.json` (`tui/src/settings.rs`); the reading level is deliberately not — `/reply-detail` is per run |

## What is not here yet

- **A preview is a preview.** The picture is placed over the cells the box has, at up to 6× the fitted size. For pixel-level work open it in the system image viewer; the TUI decodes locally and sends the picture nowhere.
- **The command palette** lists the commands the adapter implements, and the
  installed skills beside them. Tool management is intentionally compact
  (`/tools`); model-provider configuration stays in the desktop. Approval allow
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
