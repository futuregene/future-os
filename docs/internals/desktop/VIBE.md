# The Vibe panel

Status: **design agreed; implementation staged** (P0 in progress). This file is the
product and interaction contract. The keys it uses live in
[`KEYBINDINGS.md`](KEYBINDINGS.md) — that file owns the keyboard table, this one
owns the panel, and neither repeats the other.

The panel is a **full-screen, sidebar-less grid of pinned conversations**. Each
cell is one conversation. A cell can be opened (zoomed) to full screen, and the
whole thing is meant to be operated from a sofa: a remote control in one hand, a
microphone as the primary input.

## 1. Membership: pinned conversations, and nothing else

A cell is a **pinned conversation** — on this machine (`threads.pinned`) or on a
host that reports its own pin (`pinRemoteConversation`). There is no second
membership list, and that is the whole design:

* **Adding to the panel is pinning.** The user picks a machine, then a
  workspace, then a conversation that is not pinned yet; the conversation is
  pinned and lands in the panel. Removing from the panel is unpinning. Two entry
  points, one state, so "pinned but not in the panel" cannot happen.
* **A pinned **workspace** does not put its conversations in the panel.** A
  workspace pin orders the rail; carrying it in here would drop a dozen
  conversations into the grid at once and lose the overview the panel exists for.
* A pin made anywhere else (the rail's row menu, the host's own UI) shows up here
  on the next read. The panel is a view of the pin, not a cache of it.

Remote pins need that host to be reachable: while it is offline the action is
**refused with a reason**, never queued. A pin is a write to the other machine's
database, and an outbox for it would be a second, weaker truth.

## 2. Order: creation time, plus what the user moved

Default order is **`createdAt` ascending** — the oldest pinned conversation in
cell 1, a newly pinned one appended at the end. It is deliberately the opposite
of the rail's "most recently active first":

> the rail is a timeline; the panel is spatial memory.

A cell must not move because someone replied in it. The user may reorder, and
that order wins:

```
resolved = []
for key in userOrder:            # keys the user has placed, in their order
    if key in members: resolved.push(key)
for key in members sorted by createdAt:
    if key not in resolved:      # newly pinned
        insert at the first position whose createdAt is later
```

* The keys are `local:<threadId>` / `remote:<desktopId>:<sessionId>`, so a rename
  or a reconnect never changes where a cell sits.
* Relative order the user chose is **never rewritten** by the algorithm; new
  members land where they belong in time.
* Unpinning or deleting removes a member and touches nobody else's position.

With two machines in view the `createdAt` values come from two clocks; that only
affects **where a new cell is inserted**, never where an existing cell sits, so
drift can misplace one newcomer at worst.

## 3. Machine filter

The panel shows **all machines** by default, or one machine. Switching does not
reorder: cells come back where they were. Each cell carries a machine badge, and
the add dialog starts from the current filter.

## 4. The grid

The layout is a pure function of cell count and window size (testable without a
browser): 1 → 1×1, 2 → 2×1 (1×2 on a narrow window), 3–4 → 2×2, 5–6 → 3×2,
7–9 → 3×3, 10–12 → 4×3. A cell's floor is about 520×420; the column count is the
one that keeps every cell above it with a shape closest to 4:3.

* **A short last row is left empty, not stretched.** Cell 2 is always the second
  place. The keys can override the column count (`⌘=` / `⌘-`, `⌘0` back to auto).
* **No scrolling.** Twelve cells is the ceiling, and going past it says so
  ("remove one, or switch the machine filter"). A grid you have to scroll is not
  an overview.
* A cell's 24px header — number, title, machine, run state, approval count,
  unread dot — is outside the transcript and is what a click or `↩` targets.
* **Hover is never the only way to reach a control.** Every action a cell offers
  is reachable by the keys, because a remote has no pointer.

## 5. Three layers, one editable element

The panel has three layers. Exactly one of them owns the keyboard at a time, and
the rule that makes that safe is an invariant:

> **While the panel is open, the app has at most one editable element** — the
> input layer's editor. Every other cell's composer is disabled
> (`contentEditable=false`), cannot take focus, and therefore cannot receive a
> dictation.

This is what keeps a microphone usable with a dozen conversations on screen: the
speech engine types into whatever has the caret, so there must be exactly one
candidate. It is verified by a test, not by discipline
(`document.querySelectorAll('[contenteditable="true"]').length === 1`).

| Layer | Scope | Who has the keyboard | The direction keys |
| --- | --- | --- | --- |
| Grid | `vibeGrid` | the panel (a selection ring) | move the ring |
| Reading | `vibeZoom` | the panel (a block cursor) | `↑`/`↓` walk message blocks, `←`/`→` change conversation |
| Input | `vibeInput` | **the input method**, through one editor | the field's (candidates, caret) |

`Esc`/`Back` always steps out one layer, and out of the panel from the grid.
There is no dialog in the stack that the back key does not close.

### 5.1 Entering the panel enters the input layer

`↩` on a cell from the grid opens **that conversation's input layer** — the caret
is in the editor, at the end of the draft, ready for a sentence. Speaking is the
cheapest action this panel has, so it gets the shortest path:

```
grid ──↩──► input (caret in the editor)
 ▲             │ ⏎ = send, empty ⏎ = close
 │             ▼
 └──Esc──── reading (watch the answer stream in)
               │ ⏎ → back to input
```

Sending drops to the **reading layer** and scrolls to the live end, so the
editor never covers the answer it just asked for. Watching only (no speaking) is
`menu → read`, or `↩` then `Esc`.

### 5.2 One key, one meaning

The send key does not change meaning with the draft's state:

* empty draft → **close** the input layer, send nothing;
* non-empty draft → **send**, and go to the reading layer.

No setting, no long press, no modifier. A key that sometimes sends and sometimes
does nothing is a key the user stops trusting.

### 5.3 What the input layer turns off, and where that capability went

The panel's editor is the app's ordinary composer (contentEditable, IME-tested by
the same tests) with three behaviours disabled while the layer is on:

1. **`@` / `/` / `#` triggers.** Dictation inserts text in one burst, and a
   recognized "slash" or a path would open a menu over the transcript — or worse,
   swallow the next utterance's keys. These stay off.
2. **Skill recommendation.** A recommendation asks before sending, and the first
   `⏎` would then *not* send. In a sofa workflow that reads as a broken key.
3. **Auto-focus outside the input layer.** The composer's mount/settle/switch
   focus behaviour is driven by the layer instead, or closing the input layer
   would be undone by the next render.

**Where "name a file by voice" went**: not away — the fallback is
`menu → read → @`, i.e. dictate, send, then in the reading layer add the mention
with the keyboard (or the rail's attach action from a second device). Recording
this here on purpose: it is a capability that used to be one keystroke and is now
two, and the next person to wonder where it went should find the answer instead
of the feature half-restored by accident.

### 5.4 A conversation that is already running

Under P0 there is **no send queue**: pressing `⏎` on a conversation whose run is
in flight is disabled, and the panel says why on the send key's own hint line —
"this conversation is still running" — not by staying silent. A remote's key that
does nothing is indistinguishable from a key that is not bound, and that is the
one impression this panel cannot afford.

P1 adds a per-conversation queue (the existing `pendingPrompt` is a one-shot
hand-off when a conversation is opened, not a queue): a dictated message then
waits for the current run and sends itself. Until then the draft is kept, and
`Esc` never throws it away.

## 6. Reading layer: blocks, and zoom at one or two columns

`↑`/`↓` in the reading layer move a **block cursor** over the transcript — a user
message, an assistant reply, a tool call, an approval card. The selected block
gets a full-width highlight; `⏎` expands or collapses it (a tool call's
arguments and output). The wheel and the trackpad still scroll by pixels; the
block cursor is what a remote can operate.

Opened from the grid, a cell fills the screen at **one column**: the transcript at
its usual measure, unchanged from the main window. A **second column** is the
existing context panel (`ContextPanel` for a local conversation,
`RemoteFilesPanel` for a host's), and it has three states set per conversation —
`hidden`, `auto` (appears when the window is wide enough), and `pinned` (stays
open, remembered for that conversation). Reusing those panels is deliberate: what
the right column shows must not depend on which surface opened it.

## 7. The input layer's screen

The candidate window of a mac input method — and a dictation app's own overlay —
tracks the caret near the bottom of a screen. So the input layer is **not** a
bottom bar:

* a **target bar** at the top: which conversation, which machine (highest
  priority element on screen, because "who am I talking to" must be answerable at
  a glance, and it flashes once after a send so the user sees where it landed);
* the conversation's last two messages, dimmed and non-interactive, for context;
* the editor, centred and slightly above the middle, one size larger than the
  main window's, 4 lines before it scrolls;
* the rest of the screen left empty — the candidate window lives there and covers
  nothing;
* a hint line generated from the command table (only commands that have a handler
  and a binding are listed, so it cannot claim a key that does nothing).

The grid stays visible behind the layer, dimmed: who is running and who is waiting
for an approval is exactly what a person watching several conversations needs to
keep seeing.

## 8. Dictation: six hard constraints

The app does not implement speech recognition; a system or third-party input
method does. What the app owes it:

1. **Never send a composing key.** `isComposing`, `keyCode 229` and WebKitGTK's
   `"Unidentified"` are the input method's (this rule already exists in the
   dispatcher and in the composer). The `⏎` that commits a candidate is not the
   `⏎` that sends.
2. **No triggers.** See 5.3.
3. **Do not change the editor's kind.** The contentEditable editor carries the
   IME fixes (`revealCaret`, `optimizeSpeed`, the trailing zero-width space); a
   "speech-friendly" textarea would be a fresh set of the same bugs.
4. **Focus synchronously, with the caret at the end.** `MentionEditor.focus()` is
   called in the key handler's own turn; a focus deferred by an animation frame
   can arrive after the input method already opened, and the sentence is lost.
5. **A burst of text is normal.** The editor scrolls to the caret; the input layer
   caps the height at 4 lines.
6. **Undo is reachable.** Dictation mishears: `menu → undo` restores the draft to
   the snapshot taken when the layer opened, so a wrong sentence is one keypress
   to discard rather than a manual deletion. There is **no** "send automatically
   after silence" — that misfires on a sofa, and it is out of scope.

## 9. Keys

### 9.1 The keyboard

See `KEYBINDINGS.md` → "Modes". The panel does not keep a second table.

### 9.2 The remote control

A Xiaomi-style Bluetooth remote has about twelve physical buttons; only seven
reach the webview reliably. The mapping is by **layer**, because seven keys have
to cover twenty-odd actions:

| Remote button | Grid | Reading | Input |
| --- | --- | --- | --- |
| `↑` `↓` `←` `→` (repeats while held: the OS does the repeating) | move the selection ring | `↑↓` message blocks, `←→` conversation | the input method's |
| `OK` / center | open the cell → **input** | open the input layer | **send** (empty: close) |
| `Back` | close the panel | back to the grid | back to reading |
| `Menu` | the layer's menu | the layer's menu | the layer's menu |
| `Home` | first cell | jump to the live end | jump to the live end |

Two deliberate choices:

* **`OK` on an approval card opens it; it does not approve.** Approving is
  `OK` → `←`/`→` to "Allow" → `OK`. A remote's mispress rate is high and an
  approval is not undoable, so the irreversible actions cost one extra key.
* **No long press and no double press.** Both need a threshold, neither can be
  shown in a hint line, and a remote's key repeat makes double presses unreliable.
  The one "hold" in the design is the OS's own arrow-key repeat, which costs us
  nothing and needs no explaining.

### 9.3 Degradation: a remote with fewer buttons

Measured (see the matrix in §11), and honest about what stops working:

| Situation | Behaviour | What remains |
| --- | --- | --- |
| No `Menu` button (sends nothing) | the menu is unreachable **from the remote** | `⌘⇧K` on a keyboard; the panel keeps working for reading and dictating, but layout, machine filter, add/remove and order need the keyboard or another device |
| No `Home` button | no "jump to the live end" | the transcript's own scroll-to-bottom control; sending still scrolls there by itself |
| No `Back` button (or `Esc` swallowed) | **the panel still exits**: the exit button in the panel's top bar is always present and clickable, and `⌘⇧V` toggles it. Exiting may never depend on a key that a given remote does not have | pointer or keyboard |
| Only `←` / `→` + one function key (a presentation clicker) | the panel opens, and `←`/`→` change conversation | not enough to be usable; documented so nobody tries |
| Volume `+`/`-` | normally consumed by the OS and never reaches us | not bound; if a model does deliver them they are a candidate for text size (P2) |
| Voice / power | the OS owns them (assistant, sleep) | never bound |

Every button we do bind is measured on hardware, not assumed: the matrix below is
the record, and a model that reports nothing for `Menu` is exactly why 9.3's first
row exists.

### 9.4 The device's keys versus the user's

The remote's buttons are properties of the **device**, so they are `aliases` in
the command table: they keep working after the user rebinds the keyboard chord, and
after clearing it. The settings page shows them beside the recorded chord, read
only. They are still refused as a *recording* for another command in the same
scope, because two commands answering to one button would be as ambiguous as two
commands on one chord.

## 10. What the panel does not do

* No free-form split tree, no drag-resizing a cell (that is a window manager, not
  an overview).
* No scrolling grid; twelve cells is the ceiling.
* No new conversation from inside the panel: create it in the main window and it
  appears here once pinned.
* No ordering or layout sync across machines; the order is per machine, like the
  other view preferences.
* No automatic send after a pause, and no auto-approval.

## 11. Verification

Automatable:

* **The invariant**: with three cells on screen, exactly one editable element
  exists, and after `↩` it is that cell's editor with the caret at the end.
* **Focus is driven by the layer**: closing the input layer is not undone by the
  composer's auto-focus effect (the effect's three triggers are all covered).
* **A dictation's `⏎` never sends**: composing start → 200 characters → `⏎`
  (`isComposing`) sends nothing; `compositionend` → `⏎` sends the trimmed text.
* **The three state meanings**: empty `⏎` closes without sending; non-empty `⏎`
  sends and lands in the reading layer, scrolled to the live end; a running
  conversation refuses with its reason visible.
* **Triggers stay off** in the input layer (`@`, `/`).
* **Order**: created-time default; a user order wins and survives an unrelated
  pin; a new pin lands by its `createdAt`; unpinning moves nobody.
* **Membership**: adding an unpinned conversation pins it (both local and remote
  paths), removing unpins it; a pinned workspace does not add its conversations.
* **Two cells, two drafts**: switching cells never carries text between them.
* **The hint line** lists exactly the commands that have a handler and a binding
  in the current layer — deleting a label turns the test red.

On hardware (manual, recorded in the matrix below):

| Remote model | Platform | `↑↓←→` | OK | Back | Menu | Home | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| *(to be filled on first hardware pass)* | macOS | | | | | | |

## 12. Storage

| Data | P0 | Later |
| --- | --- | --- |
| Membership | none — it is the pin | — |
| Order, per-conversation column preference, column override | `localStorage` (`future.vibe.order`, `future.vibe.prefs`) | `app.db` tables + a versioned migration (`CLAUDE.md` §7) |
| Keys | the existing shortcut store (`KEYBINDINGS.md` §Storage) | same |

The panel's view preferences follow the same rule as the theme, font size and
panel widths: per machine, read on the render path, in `localStorage`.

## 13. Where the code goes

```
desktop/src/features/vibe/
  VibeStage.tsx          the grid and the zoomed cell; the panel's render root
  VibeCell.tsx           one cell's header, content and degraded rendering
  VibeCellContent.tsx    local vs remote (reuses AgentThread's content layer / RemoteConversationView)
  VibeInputLayer.tsx     the input layer: target bar, editor, hint line
  VibeBar.tsx            the thin top strip: machine, layout, exit
  VibeMenu.tsx           the layer's menu (one component, three variants)
  AddToVibeDialog.tsx    machine → workspace → conversation
  layout.ts              gridOf(count, width, height) — pure
  order.ts               resolveOrder / insertOrder — pure
  useVibeOrder.ts        the stored order and its two writes
desktop/src/features/keyboard/   catalog, scopes, dispatch (the keys already exist)
desktop/src/components/layout/AppShell.tsx   the panel in the render tree, not beside it
desktop/src/features/agent/AgentThread.tsx   its content layer, extracted for reuse
```

The panel is drawn **inside** `AppShell`, switching a `viewMode` — not as a second
window and not by unmounting the shell. Every hook that keeps run state, unread
marks, approvals and remote polling alive lives there; a panel that owns its own
copies would show a different truth than the rail.

`AgentThread`'s content layer (transcript + paging + search + approval cards) is
extracted and reused by a cell: a second message renderer would re-earn every
scroll, paging and approval bug already fixed. That extraction is the first code
task, because it is the risk — three streaming conversations on one screen is what
decides whether the panel is usable.

## 14. Staging

| Stage | Content |
| --- | --- |
| **P0** | the panel and its grid; scale-respecting cell rendering for local conversations; membership = pin; order = `createdAt` + drag/keyboard reordering; the three layers and the single-editor invariant; the input layer with dictation, the target bar and the three state meanings; `⌘⇧V`, arrows, `⏎`, `Esc`, `1`–`9`; explicit "still running" refusal |
| **P1** | remote conversations and the machine filter; the add dialog (machine → workspace → conversation, pins on add); zoom at one or two columns with the panel's three states; degraded rendering for non-focused cells; the attention queue; the send queue |
| **P2** | order in `app.db`; layout presets and a column override; `⌘/` cheat sheet; `h/j/k/l`; a "learn this button" diagnostics page; optional text size on the volume buttons |

## 15. Open questions

* Does a cell's header show the run's model? (Deferred: it is one line, and the
  reading layer already shows it.)
* Do approvals in a *non-focused* cell stay clickable with the pointer? Yes —
  the keys only reach the focused cell, but a pointer may always answer an
  approval. Recorded so the "degraded" rendering does not quietly drop it.
