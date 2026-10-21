# Keyboard shortcuts

Status: **implemented (P0–P2)**. One command table, one dispatcher, one settings
page, one command palette, one cheat sheet, user-editable bindings stored per
machine, and macOS menu items that carry no accelerators. Not yet: a per-command
"also inside the terminal" switch, and a `leader`-key model.

This document is the reasoning; the code is small enough to read next to it.

## Why one table

Before this, the app answered keys from five unrelated places: the terminal's
toggle in `features/terminal/shortcut.ts`, find-in-conversation inside
`ThreadSearch` (a capturing `window` listener that stopped propagation), Escape
through `components/ui/overlayStack`, the approval prompt's own pair, and
`Enter`/`Shift+Enter` inside `MentionEditor`. Each was correct on its own terms
and none of them knew about the others — so a new shortcut could collide with an
existing one silently, no shortcut could be discovered, and nothing could be
rebound.

A command table fixes all three at once: the dispatcher, the settings page and
(eventually) tooltips read the same list, so "documented", "assignable" and
"works" cannot drift apart.

## Layout

```
desktop/src/lib/keybinding.ts            parse / serialize / match / label / judge (no DOM, no i18n)
desktop/src/features/keyboard/
  catalog.ts                             the command table: id, category, default, terminal policy
  shortcuts.ts                           the user's overrides (localStorage) + resolution; the only file to change
                                         when bindings move to app_settings
  commands.ts                            who owns which command right now (registry + useCommand)
  dispatch.ts                            the rules: is this event ours, and which command is it
  useDispatcher.ts                       the one window listener (capture phase)
  ShortcutHost.tsx                       the listener as a component, so a view's test can install it
desktop/src/features/settings/
  ShortcutsPage.tsx                      the settings page
  ShortcutRecorder.tsx                   the "press a new combination" control
  shortcutRecorderRules.ts               what the recorder accepts and how it refuses
```

`lib/` stays free of React and i18n (`keybinding.ts` is pure); `features/keyboard`
is the app-level infrastructure; the settings page is a client of both.

## The model

A `KeyBinding` is a set of modifiers plus an `event.code`:

* `mod` — the platform accelerator: `⌘` on macOS, `Ctrl` on Windows and Linux. One
  default (`mod+n`) is therefore valid on all three platforms, and a user's
  rebinding stays "the platform's accelerator" wherever the settings are read.
* `ctrl`, `alt`, `shift` — explicit, because a macOS user also wants a literal
  `⌃` (and `⌥`), and because `Ctrl+Alt` on a Windows layout is often `AltGr`.
* the key is the **physical** key (`KeyN`, `Comma`, `BracketLeft`), not the
  character. `event.key` is what the layout types: on AZERTY `Shift+,` is `?`, on
  a German layout `z` and `y` trade places, and a Dvorak user's `Ctrl+Z` would sit
  on a different physical key. `key` is only a fallback for engines that send no
  `code` (some WebKitGTK builds, synthesised events).

Stored text is canonical (`mod+shift+bracketright`): modifiers in a fixed order,
then the code. Anything the parser does not understand is dropped on read, never
guessed at.

## The dispatch rules

`dispatch.ts` answers one question — *should this event run a command, and
which one?* — and never executes anything. In order:

1. **An event an earlier handler consumed** is not ours (`defaultPrevented`).
2. **An IME owns the keyboard while it composes**: `isComposing`, Chromium's
   `keyCode 229`, and WebKitGTK's `key: "Unidentified"`. Acting on a composing
   key fires commands while the user types Chinese. (The terminal's own IME
   history — the composed text sent twice — is in `embedded-terminal.md`.)
3. **AltGr is typing**: Windows and Linux report it as `Ctrl+Alt`, so a binding on
   that pair would fire on `{`, `[` or `@`.
4. **A binding with no handler is not handled.** No conversation open means no
   `ThreadSearch`, so `mod+f` stays the platform's own instead of being swallowed
   for nothing.
5. **A modal or a floating menu holds the keyboard** (`overlayStack`,
   `keyboardOwners`). Both are by *presence*, not focus — WebKit does not focus a
   button on click, so a focus test would answer "nobody owns the keyboard" while
   a menu is plainly on screen.
6. **Holding a key repeats only what says it may** (`repeat`): walking a list
   wants the repeat, toggling a panel does not.
7. **Inside the terminal, only commands marked `inTerminal` fire.** `Ctrl` is the
   accelerator on Windows and Linux, and the terminal is a shell: `Ctrl+B` is
   "back one character", `Ctrl+F` "forward one", `Ctrl+A` "line start", `Ctrl+K`
   "kill to end", `Ctrl+[` is `ESC`. Taking any of them over would break line
   editing in the one place the user is typing a command line.

The listener is installed in the **capture** phase so xterm — which attaches its
handler to its own textarea and cancels what it handles — cannot swallow a
command before the dispatcher sees it. A key it handles is stopped there:
`preventDefault` keeps the browser's meaning away, `stopPropagation` keeps the
focused view from acting on it again.

## Navigation has two levels, because the list does

The rail lists two kinds of thing — **workspaces** (each holding conversations)
and **conversations** — plus the forks that hang under the conversation they came
from. One key cannot serve all of that: a flat "next conversation" makes a
workspace with 20 conversations 20 presses to cross, and a "next workspace" alone
cannot reach a conversation at all.

So the walk is layered, and each layer answers its own question:

| Layer | Keys | Question it answers |
| --- | --- | --- |
| fine | `mod+[` / `mod+]` | *which conversation, wherever it sits?* |
| coarse | `mod+shift+[` / `mod+shift+]` | *which workspace, or which conversation?* |
| inside a conversation | `Page Up` / `Page Down` | *which question in this transcript?* |

Why these keys: `mod+[` / `mod+]` is the pair every browser already uses to go
back and forward, so it is a habit rather than a memorised chord, and holding
`⇧` for the bigger step is the same instinct one notch harder. An earlier
version put the fine move on `mod+alt+↑/↓` to stay clear of text-editing chords:
accurate, but three fingers for the move a reader makes most often. Both remain
rebindable, so a layout that cannot type `[` cheaply (German, where it needs
`AltGr`) can move them without a code change.

Both levels read **one** description of the list (`walkBlocks` in
`ActivityRail.tsx`): a block is a conversation with the forks drawn under it, and
a workspace's block is the whole workspace. The fine move flattens the blocks the
current section draws; the coarse move steps between all of them. Two separate
derivations is exactly how the first version broke — in the merged list the fine
move followed the tree order while the coarse move followed the grouped order, so
`⇧]` skipped a conversation that `]` walked into.

Details that matter:

* **The coarse move crosses the two sections.** A chat conversation and a
  workspace are siblings in the user's model even though the rail draws them one
  above the other, so the coarse walk goes pinned → workspaces → chats and back.
  Opening a conversation is what switches the section: the shell derives it from
  the conversation's own mode, so nothing else has to know.
* **The fine move stays inside the section that is drawn.** It walks the rows the
  eye can see: the workspaces’ rows while the workspace section is open, the chats
  while the chat section is. That is what "the next conversation on screen" means,
  and it is why the coarse move is the one that crosses over.
* **A conversation inside a workspace belongs to that workspace's stop**, so
  "previous" from a nested conversation goes to the stop *before* the workspace,
  not to the workspace it is already in.
* **A stop with no conversations is still a stop** (an empty workspace is listed,
  so it is selectable); a stop that holds a fork lands on its root.
* **Neither move wraps.** Running off either end does nothing, and the key is not
  consumed; the list is a list, not a ring.
* **Both are unclaimed when there is nowhere to go** (one stop, or a section with
  no list), so the keys are never swallowed for a no-op.

## Modes: a keymap that exists only while a mode is on

A global keymap is right for the things that are always true. A **mode** needs the
opposite: while it is on, a key should mean what the mode says, and it must be
allowed to sit on a key the global keymap already uses. `scopes.ts` is that.

| Scope | Active | Holds |
| --- | --- | --- |
| `global` | always | everything above: new conversation, navigation, panels, terminal, settings |
| `vibe` | the panel is open | layout, device filter, add/remove, attention queue, overlay full screen |
| `vibeGrid` | the grid has the keyboard (no cell zoomed) | the direction keys, `↩`, `1`–`9` |
| `vibeZoom` | one cell fills the screen | `Esc` back out, the block cursor, the side panel's two keys |
| `vibeInput` | the input layer: the caret is in a dictation editor | `Esc` back and the menu — the one layer that runs **over** an editor. Sending is **not** here: see below |

Resolution is **innermost scope first**, then outwards, then `global`. So:

* a mode's key **shadows** a global one while the mode is on, and the global
  meaning is back the moment it closes — no conflict, by construction;
* two commands may share a key **in different scopes**, and the recorder only
  refuses a collision **within one scope** (`conflictingCommand` is scope-aware);
* a bare key (`←`, `↩`, `Esc`, a digit) is only ever assignable in a non-global
  scope, and the dispatcher runs one **only when its scope is active and the caret
  is not in a text field**. `Shift` alone does not make a chord: `Shift+Enter` is a
  newline. The exception is the input layer, whose commands are marked
  `overEditable`: it is the layer where the caret *is* in a field, and its `⏎` is
  what sends. Nothing else may reach over an editor.

The panel's own keys (all rebindable; `vibe.cell` is the one exception, see
below). `mod` is `⌘` on macOS and `Ctrl` elsewhere:

| Action | Default | Scope |
| --- | --- | --- |
| Open or close the panel | `mod+shift+v` | global |
| Move the selection | `← ↑ → ↓`, or `h j k l` | vibeGrid |
| Enter the selected cell → its **input layer** | `↩` | vibeGrid |
| Close the panel | `Esc` / `Back` | vibeGrid |
| Leave the zoomed cell | `Esc` / `Back` | vibeZoom |
| Previous / next conversation in the zoomed cell | `←` / `→`, or `h` / `l` | vibeZoom |
| Previous / next message block | `↑` / `↓` | vibeZoom |
| Open the input layer | `↩` | vibeZoom |
| Send the dictation (empty draft: close; a run in flight: queue it) | `↩` — the composer's own fixed key, not a command | vibeInput |
| Leave the input layer | `Esc` / `Back` | vibeInput |
| Take a queued sentence back into the box | `mod+shift+x` / the menu's cancel row | vibeInput / vibe |
| The layer's menu | `mod+shift+k` / `Menu` | vibe |
| Jump to the live end of the conversation | `Home` | vibe |
| Go to a numbered cell | `1`–`9` | vibeGrid |
| Previous / next cell | `mod+[` / `mod+]` | (reuses `thread.previous`/`next`; the panel registers them, the rail does not) |
| Cycle the grid layout | `mod+shift+g` | vibe |
| More / fewer columns, automatic | `mod+=` / `mod+-` / `mod+0` | vibe |
| Move the selected cell earlier / later | `mod+alt+←` / `mod+alt+→` | vibe |
| Side panel: show/hide, pin | `mod+\` / `mod+shift+\` | vibeZoom |
| Switch machine filter | `mod+shift+d` | vibe |
| Jump to the next cell that needs you | `mod+shift+a` | vibe |
| Add a conversation to the panel | `mod+shift+e` | vibe |
| Remove the selected conversation | `mod+shift+backspace` | vibe |
| Overlay (app-wide) full screen | `mod+ctrl+f` | global |
| All shortcuts | `mod+/` | global |

Three notes on this table:

* **`⏎` sending is not a command.** It is the composer's own fixed key
  (`MentionEditor.onSubmit`, listed under `FIXED_KEYS`), and it already stands down
  while an input method composes. The dispatcher listens in the capture phase, so a
  `vibe.input.send` command would run *before* the editor's submit and the same
  press would have two owners — the one way to reliably send a message twice. The
  panel's contribution is only what the editor had no answer for: an empty draft
  closes the layer. Same key, one owner.
* **`Menu`, `Back` and `Home` are the remote control's buttons, not a keyboard's.**
  They are `aliases` on their command: they keep working after the reader rebinds
  the chord and after clearing it, the settings page shows them read only, and they
  are still refused as somebody else's recording in the same scope. A remote's
  button is a property of the device, not a preference. Which button does what in
  which layer, and what stops working on a remote that lacks one, is the panel's
  own document (`VIBE.md` §9.2–9.3).

* **`mod+[` / `mod+]` are the same commands as the rail's**, not a second pair. A
  command's *meaning* comes from whichever view owns it: inside the panel the
  panel registers `thread.next` as "next cell", outside it the rail registers it
  as "next conversation". The registry keeps one handler per command, so the two
  cannot both be live.
* **`h`/`j`/`k`/`l` are second bindings on the four movement commands**, not four more
  commands: the movement is the same movement. They are listed read-only in the settings
  page (like `vibe.cell`), because a rebindable pair would let a reader shadow them
  without seeing what they shadowed. They never fire over an editor — a modifier-less
  keystroke belongs to the field that has the caret — so typing `h` in a dictation is
  an `h`.
* **`1`–`9` is one command with nine bindings** (`vibe.cell`), because a cell is
  chosen by the digit printed on it. The settings page shows them as a read-only
  key list: recording a single key for a nine-key command would ask a question the
  page cannot answer. `vibe.cell`'s handler receives the matched binding and reads
  the digit from it (`runCommand(id, binding)`).

The panel's design, its grid/order model and its dictation model live with the
feature (`docs/internals/desktop/VIBE.md`); this file owns only its keys. One
design revision is worth recording here: that proposal reused `mod+alt+↓/↑` for
"next cell" because the rail's fine move was there at the time. The fine move has
since moved to `mod+[` / `mod+]`, so the panel reuses those — the panel's "next"
and the rail's "next" are the same command on the same key.

## Finding a key: the palette and the cheat sheet

Three entries exist around finding and changing the others, all global and all
rebindable:

| Action | Default | What it answers |
| --- | --- | --- |
| Jump to a command or conversation (`palette.open`) | `mod+k` | *do this* / *go there* |
| Show all shortcuts (`shortcuts.help`) | `mod+/` | *what can I press* |
| Keyboard shortcut settings (`shortcuts.settings`) | `mod+shift+,` | *change one* |

`settings.open` keeps `mod+,` and keeps opening Settings on its **first** page:
`mod+shift+,` is an addition, not a move, so the page a reader sees when they open
settings is unchanged. The cheat sheet's header carries the one button it needs —
*Edit shortcuts* — and that button **runs the `shortcuts.settings` command** rather
than opening Settings itself, so there is one path to that page (`runCommand`),
rebindable and reachable from the palette and the macOS menu like any other.

It is also findable by words its label does not contain: the palette matches
`keywords.<id>` as well as the label (`shortcuts.json`), so "keybind", "hotkey" and
"rebind" reach the shortcut settings row. An alias only ever *finds* a row — a
label match always ranks above one — so adding keywords cannot push the command
someone searched for by name down the list.

**The palette is not a second `mod+shift+f`.** `mod+shift+f` filters the rail's
list in place — it narrows what is already on screen; the palette navigates,
anywhere, from anywhere. Both stay.

What it lists (`features/palette/paletteItems.ts` decides, the dialog renders):

* **commands** whose handler is registered right now, in the table's order. One
  with no handler is not a row at all: the palette promises to do what it says,
  the same rule the dispatcher applies to a key with no owner. Availability is
  decided when the dialog *opens*, not when the shell last rendered — handlers are
  installed by effects, so a list captured on a first render would be empty;
* **conversations** — this machine's and the paired machines' — and
  **workspaces**, each as its own section, never ranked against the commands: a
  chat called "terminal notes" must not outrank the command named "terminal".

Matching is a substring, ranked prefix → word-start → anywhere, and inside one
rank the caller's order survives (`matchRank`). The rows are walked with `↑`/`↓`,
run with `↩`, dismissed with `Esc` or with the palette's own key, which the dialog
reads from the live binding (`paletteToggle.ts`) because while it is open it *is*
the overlay the dispatcher stands down behind.

**The cheat sheet lists the live keys, not the defaults.** It reads the same
store the dispatcher does (`useShortcutOverrides`), so a rebinding shows up in it
while it is open — a card printed from `catalog.ts` would be a card of the old
keys, which is the one thing a cheat sheet in an app with editable keys cannot be.
A command the user unbound has no key, so it has no row; a multi-key command shows
as its range (`1 … 9`).

## The macOS menu

`desktop/src-tauri/src/menu.rs` builds the menu bar, and two of its items are this
document's business: **File → New Conversation** and **File → Settings…**.

**Neither carries an accelerator, on purpose.** Both are commands in the table
(`chat.new`, `settings.open`), and the user may rebind them. An accelerator written
into the menu keeps printing the old key after a rebinding — a menu item that says
`⌘N` while `⌘N` opens nothing is worse than one that names no key. So the item
emits the **command id** (`menu-command`), and the window runs it through the same
`runCommand` the dispatcher and the palette use (`AppShell`'s `menu-command`
listener). One handler per command, whichever way it was asked for.

An id this build does not know is ignored rather than guessed at, and a window
that is not up yet has no handler, so the event is a no-op there.

## Where each command is registered

The shell registers the commands that are its own (navigation, panels, settings).
Everything that belongs to a view is registered by that view while it is mounted
(`useCommand`):

| Command | Registered by | Why there |
| --- | --- | --- |
| `chat.new`, `panel.*`, `section.*`, `settings.open` | `AppShell` | shell state |
| `thread.find` | `features/agent/search/ThreadSearch` | only exists with a conversation open |
| `chat.stop` | `features/agent/AgentThread` | the abort lives in `useAgentThreadState`, and only while a reply streams |
| `composer.focus` | `features/agent/Composer` | only the composer knows whether it can hold a caret |
| `group.previous` / `group.next` / `thread.previous` / `thread.next` / `rail.search` | `components/layout/ActivityRail` | the drawn order is the rail's own |
| `terminal.toggle` | `features/terminal/useTerminalPanel` | needs the panel's open state; also the one command the terminal lets through |

Registration *is* availability: nothing registered, nothing claimed. That is why
`chat.stop` does nothing — and consumes nothing — when no reply is streaming.

## Storage

Bindings live in `localStorage` under `future.shortcuts.v1`, as **differences
only**: `{"version":1,"overrides":{"chat.new":"mod+shift+KeyK"}}`, with `null`
meaning "the user removed this binding". A command with no entry keeps following
its default, so improving a default in a later release reaches everyone who did
not override it.

Why localStorage rather than `~/.future/app/app.db`:

* bindings are per machine *and per platform* — a rebinding made on Windows must
  not follow the account to a Mac, and means nothing to a phone;
* they are read on a keystroke, not on a data path;
* every other preference of that shape in this app (theme, font size, panel
  geometry, terminal panel height) is already there, for the same reasons.

`features/keyboard/shortcuts.ts` is the only file that reads or writes that key.
Moving the document to the backend's `app_settings` later means changing
`read()`/`write()` there and nothing else.

## Defaults (v1, global keymap)

`mod` is `⌘` on macOS and `Ctrl` on Windows/Linux. The Vibe panel's keys are a
separate keymap — see *Modes* below.

| Command | Default | |
| --- | --- | --- |
| New conversation | `mod+n` | |
| Stop generating | `mod+.` | registered only while a reply streams |
| Focus the composer | `mod+l` | |
| Find in this conversation | `mod+f` | was a private listener in `ThreadSearch` |
| Previous / next conversation | `mod+[` / `mod+]` | the fine move: every drawn conversation row; repeats |
| Previous / next workspace or conversation | `mod+shift+[` / `mod+shift+]` | the coarse move: hold ⇧ for the bigger step; repeats |
| Filter the conversation list | `mod+shift+f` | jumps to the field, does not navigate |
| Go to Chats / Workspaces / Skills / Scheduled tasks / Remote | `mod+1` … `mod+5` | |
| Show or hide the sidebar | `mod+b` | |
| Show or hide the context panel | `mod+shift+b` | |
| Show or hide the terminal | `mod+j` | was hard-coded in `features/terminal/shortcut.ts` |
| Open settings | `mod+,` | |

Fixed keys (listed in Settings, not assignable): `Enter` send, `Shift+Enter`
newline, `Esc` close/reject, `⌘/Ctrl+Enter` approve, `Page Up`/`Page Down`
transcript (the third navigation level), arrows + `Enter` in an open menu.

## What the recorder refuses

* **No modifier** — a bare key is ordinary typing, and `Shift` alone is how
  everyone types a capital letter.
* **Reserved combinations** — the platform's text editing and window chords
  (`⌘C/V/X/A/Z`, `⌘Q/H/M/W`, `⌥⌘Esc`, `Alt+F4`, `⌘Space`, `⌘⇧3/4/5`) and the
  webview's reload and developer tools (`⌘R`, `⌥⌘I`). A binding the platform eats
  looks assigned and does nothing.
* **A combination another command already has** — the refusal names that command.
* **AltGr** — see the dispatch rules.

`Backspace`/`Delete` while recording removes the binding, `Esc` cancels.

## Platform notes

| | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Accelerator | `⌘` | `Ctrl` | `Ctrl` |
| Label | `⌃⌥⇧⌘K` | `Ctrl+Shift+K` | `Ctrl+Shift+K` |
| Engine | WKWebView | WebView2 (Chromium) | WebKitGTK |
| Specifics | `⌘` combinations do not produce control characters, so the terminal *could* receive more of them later (P3) — v1 still lets only `terminal.toggle` through | `AltGr` arrives as `Ctrl+Alt` and must not fire commands | WebKitGTK reports an IME-consumed key as `Unidentified`; a modified key during composition cancels the preedit (needs a second press, documented in `embedded-terminal.md`) |

Global OS-level hotkeys (a shortcut that works while another app is focused) are
deliberately **not** part of this: `tauri-plugin-global-shortcut` takes keys from
every other application on the machine, which is a different feature and a
different risk.

## Tests

| File | What it pins |
| --- | --- |
| `lib/keybinding.test.ts` | parsing, canonical text, `mod` on both platforms, physical codes vs layout characters, labels, reserved list |
| `features/keyboard/catalog.test.ts` | unique ids, legal and distinct defaults, the terminal whitelist, `en`/`zh` strings for every command and category |
| `features/keyboard/dispatch.test.ts` | the rule order (IME, AltGr, repeat, overlay, terminal, no handler), with no DOM |
| `features/keyboard/shortcuts.test.ts` | differences-only storage, reset, portable `mod`, unreadable stored values |
| `features/keyboard/useDispatcher.test.tsx` | the real window listener: handled keys consumed, rebinding effective on the next press, the shell's keys untouched inside the terminal |
| `features/keyboard/scopes.test.tsx` | scopes ranked by layer, two holders of one scope, release independent |
| `features/keyboard/modeKeymap.test.tsx` | a mode's key fires only inside it, one key in two scopes each staying its own, a bare key not taking a text field |
| `features/settings/ShortcutsPage.test.tsx` | recording, refusal (modifier / reserved / conflict), cancel, unbind, restore, search |
| `features/palette/paletteItems.test.ts` | matching and ranking, commands kept apart from conversations, empty sections dropped |
| `features/palette/CommandPalette.test.tsx` | listing only what can run, filtering, arrows/`↩`, dismissal by its own key |
| `features/palette/ShortcutCheatSheet.test.tsx` | live keys, a rebinding followed while open, an unbound command absent |
| `ThreadSearch*`, `Composer.focus`, `AgentThread`, `ActivityRail.nav`, `AppShell` | each view's own command, through the dispatcher |

### What was actually verified by hand, and what was not

Every rule above is covered by the automated tests in the table. One rule was
also checked against a running app by hand, and the one that was not is called
out below — a reader deciding whether to trust a rule needs to know which kind of
evidence stands behind it, and "covered by tests" is not the same claim as "seen
working on the engine a release ships".

**Verified by hand — the terminal keeps the shell's keys.** In the **real
Chromium webview with a Linux user agent** (a headless session driving the real
app over CDP): with the terminal focused, `Ctrl+[` delivered `0x1b` (ESC) to the
pseudo-terminal and `Ctrl+]` delivered `0x1d`, and neither was taken by the app.
This is Chromium, not WebKitGTK — the engine a packaged Linux build actually runs.

**Not verified by hand — Linux + WebKitGTK + a Chinese IME.** The rule "a key
pressed mid-composition does not fire a command" is covered at the logic level
only: `dispatchShortcut` has cases for `isComposing`, `keyCode 229` and
`key: "Unidentified"`, the three shapes those engines report
(`dispatch.test.ts`). It has **not** been exercised against WebKitGTK with
ibus-libpinyin, the combination that produced the original terminal bug this rule
descends from (`embedded-terminal.md`). Neither a Linux desktop nor a container
able to run a WebKitGTK webview with an IME was available on the machine this was
written on.

> **TODO.** To close it, on a Linux desktop with `ibus-libpinyin` (or any IME
> that composes CJK): run the app, open a conversation, start composing a word,
> then press a modified key a command is bound to (e.g. `Ctrl+B`). Expected: no
> command runs and the preedit box stays. Repeat inside the embedded terminal,
> where the expectation is the same and the stale failure mode is the composition
> being committed twice (`keyPolicy.ts`). When it is done, the two outcomes belong
> in this section.

## Going further (not implemented)

* A per-command "also works inside the terminal" switch, default off, for users
  who want `⌘N` while the terminal has focus on macOS. Deliberately not in v1:
  `Ctrl` is the accelerator on Windows and Linux, so taking keys from the shell
  costs it readline.
* A `leader`-key model (opencode style). The data model leaves room; v1 has no
  timeout, no HUD and no chords.
