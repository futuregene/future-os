# The Vibe panel

Status: **P0 implemented** (grid, membership = pin, order, the three layers, the
dictation layer); P1 (remote conversations, the machine filter, the add dialog, the
two-column reading layer) and P2 are not. This file is the product and interaction
contract. The keys it uses live in
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

* **Adding to the panel is pinning.** The user picks a machine, then a workspace,
  then a conversation that is not pinned yet; the conversation is pinned and lands
  in the panel. Removing from the panel is unpinning. Two entry points, one state, so
  "pinned but not in the panel" cannot happen. The dialog says so in its footer, once
  — a row-level reminder on every line would be noise. On a host, adding is that
  host's own `set_session_pinned`, addressed with its thread id (§9.4's neighbour
  rule: a session id where a thread id is expected pins the wrong record, so a host
  too old to report one is refused rather than guessed at).
* **A pinned **workspace** does not put its conversations in the panel.** A
  workspace pin orders the rail; carrying it in here would drop a dozen
  conversations into the grid at once and lose the overview the panel exists for.
* A pin made anywhere else (the rail's row menu, the host's own UI) shows up here
  on the next read. The panel is a view of the pin, not a cache of it.

Remote pins need that host to be reachable: while it is offline the action is
**refused with a reason**, never queued. A pin is a write to the other machine's
database, and an outbox for it would be a second, weaker truth.

**Which conversations count is the host's own report.** A paired host's catalogue
carries `pinned` per session, and the panel reads it — there is no local list of
"remote conversations I want here". A cell therefore appears when that host says so
and disappears when it stops saying so, on the same next read as everything else.

**What a host's cell can say is what that host reports**, and the panel does not
invent the rest: its catalogue has a title, a workspace, `streaming` and a pin. It has
no creation time (so the cell is undated — see §2), no run failure and no unread
mark, so a host's conversation can be *in* the grid and still be absent from the
attention queue (§9.5). Its own approvals do reach it, as events to that cell, and
are answered in the cell.

**Known limitation, not a backlog item**: a host's conversation cannot enter the
attention queue or the send queue, because neither fact exists on the wire. The
catalogue is that machine's own summary and carries no per-run outcome and no unread
state, and a prompt to it is a different command with different preconditions. Making
the panel show either would be a **catalogue-protocol change on the host side** —
worth doing, but it is its own task with its own compatibility story, not something
this panel can close by being cleverer.

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
the add dialog starts from the machines that are paired.

The choice is a **view preference like the columns** and is remembered per machine
(it is a window-size-shaped decision, not an account one). The menu's row is the only
way in, because a remote cannot drive a dialog; the row is a *value* row (`←`/`→`),
which is what makes "which machine am I looking at" answerable without opening
anything.

The filter lists **paired** machines, not machines that happen to have something
pinned: a filter that vanished when the panel was empty would be missing exactly when
a reader wants to look at a machine.

## 4. The grid

The layout is a pure function of cell count and window size (testable without a
browser): 1 → 1×1, 2 → 2×1 (1×2 on a narrow window), 3–4 → 2×2, 5–6 → 3×2,
7–9 → 3×3, 10–12 → 4×3 — i.e. `ceil(sqrt(n))` columns, capped at four.

A cell *likes* about 520×420, and that number is a **preference, not a floor**: no
arrangement of five cells in a 1280px window can give every cell 520px, so
refusing to draw would be worse than drawing 416px cells. The preference decides
only one thing — when a window is narrow enough that **stacking beats squeezing**,
which is why two cells become 1×2 rather than two slivers while five stay 3×2
(stacking five would make 180px strips).

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

**That key is the composer's own, not a command.** `⏎` already sends in the
editor (`MentionEditor.onSubmit`, a `FIXED_KEYS` entry) and already stands down
while an input method composes. The panel adds only the case the editor had no
answer for — an empty draft closes the layer — and it adds it inside the send
path, not in the command table. A command would have been a second owner on one
press: the dispatcher listens in the capture phase, so it would run before the
editor's submit, and the pair is one bad refactor away from sending twice.

Because a cell's reading chrome mounts no composer at all, "closing the input
layer cannot be undone by an auto-focus" is also structural rather than a flag:
the editor is unmounted, so there is nothing to focus.

### 5.3 What the input layer turns off, and where that capability went

The panel's editor is the app's ordinary composer (contentEditable, IME-tested by
the same tests) with three behaviours disabled while the layer is on:

1. **`@` / `/` / `#` triggers.** Dictation inserts text in one burst, and a
   recognized "slash" or a path would open a menu over the transcript — or worse,
   swallow the next utterance's keys. These stay off.
2. **Skill recommendation.** A recommendation asks before sending, and the first
   `⏎` would then *not* send. In a sofa workflow that reads as a broken key.
3. **Auto-focus outside the input layer.** Nothing to disable in practice: the
   composer is mounted *only* by the input layer, so a cell's reading chrome has no
   editor and no focus effect to run. The layer's boundary is the component
   boundary.

**Where "name a file by voice" went**: not away — the fallback is
`menu → read → @`, i.e. dictate, send, then in the reading layer add the mention
with the keyboard (or the rail's attach action from a second device). Recording
this here on purpose: it is a capability that used to be one keystroke and is now
two, and the next person to wonder where it went should find the answer instead
of the feature half-restored by accident.

### 5.4 A conversation that is already running: the send queue

Pressing `⏎` on a conversation whose run is in flight **holds the sentence** instead of
dropping it. The composer hands the draft over and clears its box; the panel shows
`已排队` on the cell; when the run ends the text goes **back into that conversation's
input box**, the cell says `已放回输入框`, and the reader presses `⏎` if they still
mean it.

**Nothing is ever sent by itself**, and that is the design, not a shortcut. Of the two
possible queues — "send it when the run ends" and "hand it back and let the reader
decide" — this is the safer one: a message dictated from a sofa gets one last look,
there is no window in which a sentence they had second thoughts about is already gone,
and there is no timer to tune.

The five rules (`vibeQueue.ts`):

| Rule | How it holds |
| --- | --- |
| **Visible** | A cell that holds a sentence shows `已排队`; a cell whose sentence just went back shows `已放回输入框` until the reader opens that conversation |
| **Cancellable, and cancelling is not discarding** | The menu's cancel row, or `⌘⇧X` in the input layer, puts the text back in the box. If the box already holds something typed since, the cancellation is **refused** and the sentence stays queued — overwriting a message being written is the one thing worse than waiting |
| **One per conversation** | A second `⏎` is **refused** (the composer keeps the text in its box) rather than replacing the held sentence. Replacing is how a sentence gets swallowed |
| **The last moment is the reader's** | The queue has no send path at all: it only writes `composerDraft` |
| **A failure keeps it** | A run that failed or was interrupted reaches the same "not running" check, so the sentence comes back — never re-sent as a second message |

`Esc` in the input layer still means "leave the layer", and does **not** cancel the
queue: leaving the layer is how a reader stops dictating, taking a sentence back is how
they change their mind about one message. One key for both would mean `Esc` did two
different things depending on a state the reader cannot read off the key.

### 5.5 The contract with the conversation views

Two components carry the panel's rules into the window's own views. Their props are
therefore a contract, not an implementation detail: **the defaults are what keeps an
ordinary conversation ordinary**, and a change to either default is a change to the
whole app.

| Prop | On | Default | Who may pass it | What it does |
| --- | --- | --- | --- | --- |
| `chrome` | `AgentThread` | `"full"` | the panel only | `full` draws the title bar, the find bar and the floating composer; `read` draws the transcript and an approval card, **no composer**; `input` draws the conversation's tail with the composer in the flow |
| `ownsCommands` | `AgentThread` | `true` | the panel only | whether this instance may claim `chat.stop` — off for every cell but the focused one, because one handler per command means "the last mounted wins" |
| `triggers` | `Composer` → `MentionEditor` | `true` | the panel only | whether `@` / `/` / `#` open their menus while typing |
| `onEmptySubmit` | `Composer` | absent | the panel only | what the send key means on an empty draft (close the input layer); absent everywhere else, where an empty submit is simply not a send |
| `controls` | `Composer` | `"full"` | the panel only | `full` is the interactive row (model, thinking, approval, attach); `none` hides it, for the input layer where those controls live in the menu and only a read-only line stays on screen |
| `onQueue` | `Composer` | absent | the panel only | what a send means while the conversation is still running: the draft is handed to the panel's queue instead of being dropped. Absent everywhere else, where a send during a run is simply not a send |
| `chrome` | `RemoteConversationView` | `"full"` | the panel only | `read` drops the conversation's own top bar (the cell draws its own) and puts the approval cards in a strip at the foot instead of over the transcript |
| `selectiveEvents` | `RemoteConversationView` | `true` | the panel only | whether mounting this view narrows that host's event lane to this session. The panel shows several conversations of one host at once, so it must **not** narrow: the lane is a bandwidth bargain for one conversation, and a grid is not one conversation |

Two invariants ride on those defaults, and both are structural rather than
supervisory:

* **`read` mounts no composer**, so "the app has at most one editable element while
  the panel is open" holds because the tree has one editor — not because focus is
  policed.
* **`chrome` defaults to `full` and `triggers` to `true`**, so a caller that knows
  nothing about the panel gets exactly the behaviour it had before the panel
  existed. A new prop follows the same rule: it goes in this table first, then in
  the code.

### 5.6 The values stay visible; only the change moves

The composer's control row — model, thinking level, approval tier, attach — is the
right thing in the main window and the wrong thing in the middle of a dictated
sentence. It is not removed, though, because "which model is this cell running, and
how much is it allowed to do" is one of the questions a person watching five
conversations actually asks.

So the panel splits the row in two:

* **The values are always on screen, read-only**, on one line —
  `DeepSeek V4 Pro · 中 · 完全放开` — in the **same place on the reading layer and on
  the input layer**. A value that moved or vanished between the two layers would be a
  value the reader stops trusting.
* **The change lives in the menu** (§9), on three value rows adjusted with `←`/`→`:
  the model (cycles the catalogue), the thinking level (cycles the levels the model
  has), and the approval tier.

Three rules about who owns what:

* **A cell's model and thinking level are that conversation's**, written through its
  own thread id (`vibeCellSettings.ts`). The main window's `changeModel` persists to
  *the shell's active thread*, which in the panel is whichever conversation happened
  to be open there — using it would silently retune a different cell.
* **The approval tier is the application's, not a cell's.** The menu row says so
  ("applies to every conversation"); a per-cell tier does not exist and the panel
  does not imply one.
* **A host's cell shows no settings line and offers no rows.** Its model and its
  sandbox are that machine's, and the panel changes only what it owns.

The line is read from the agent's cached state for that thread (what the session is
*actually* running), falling back to the shell's selection only until that state has
been read once.

### 5.7 The bar, and the two things it has to get right

The panel is `fixed inset-0`: it is the only surface in the app whose content starts at
x = 0, and the window is a macOS **overlay title bar**
(`titleBarStyle: "Overlay"`, `trafficLightPosition: { x: 14, y: 26 }`), so the traffic
lights are drawn *over* the page at roughly x 14–72. Two consequences, both of which were
wrong once and are now pinned by tests:

* **The bar's content starts past the lights.** 80px (`pl-20`) — the same reserve the
  activity rail uses — on macOS outside fullscreen, and the ordinary 12px everywhere else
  (other platforms, and fullscreen, where the system hides the lights). `barChrome.ts`
  holds the rule; the numbers come from the config, not from taste.
* **The bar is the drag handle.** While the panel is open nothing else visible is
  draggable, so the bar starts a window drag on a press (`startWindowDrag`, which skips
  buttons — the three controls in it keep working).

And one thing about fullscreen that is not geometry: **`set_fullscreen` is an explicit
capability grant**. `isFullscreen` comes from `core:default`, `setFullscreen` does not, and
the button shipped once with the grant missing — the capability layer rejected the call,
the code's `.catch(() => {})` swallowed it, and the button did nothing at all. So the
toggle lives in one function (`fullscreen.ts`), a refusal **reaches the reader** as an
error toast and the console, and `fullscreenCapability.test.ts` asserts the grant next to
the code that needs it.

## 6. Reading layer: blocks, and zoom at one or two columns

`↑`/`↓` in the reading layer move a **block cursor** over the transcript — a user
message, an assistant reply, a tool call, an approval card. The selected block
gets a full-width highlight; `⏎` expands or collapses it (a tool call's
arguments and output). The wheel and the trackpad still scroll by pixels; the
block cursor is what a remote can operate.

Opened from the grid, a conversation fills the screen at **one column** by default:
the transcript at its usual measure, unchanged from the main window. A **second
column** is the existing context panel (`ContextPanel` for a local conversation,
`RemoteFilesPanel` for a host's). Reusing those panels is deliberate: what the right
column shows must not depend on which surface opened it.

It has three states, **remembered per conversation** by the panel's own key
(`readingPanel.ts`):

| State | When it shows |
| --- | --- |
| `auto` (default) | above 1100px: wide enough for a transcript at its measure plus a 400px column |
| `hidden` | never |
| `pinned` | always, whatever the window does — that is what pinning is for |

`⌘\` toggles between `hidden` and `auto` (a toggle, not a cycle through three: one press
that sometimes *shows* a column the reader was hiding is a press they stop trusting);
`⌘⇧\` pins and unpins. The menu has the same choice as a value row, so "which state is
this conversation in" is answerable without pressing anything. The column's width is
fixed at 400px and has no drag handle: the reading layer is a view of one conversation,
not a window with three resizable columns.

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

Two keys are worth repeating here because they are the panel's own grammar rather than a
shortcut: the arrows move the selection, and **`h`/`j`/`k`/`l` move it the same way** —
same commands, second bindings. They never fire over an editor (a modifier-less keystroke
belongs to the field with the caret), so typing one while dictating is typing it.

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

**None of this has been verified on hardware.** The table above is the design, not a
measurement: which button a given remote actually sends is recorded in
[`VIBE-REMOTE-KEYS.md`](VIBE-REMOTE-KEYS.md), and that matrix is empty. The panel ships a
**read-only key readout** for filling it in (`⌥⇧K` three times; see the same file) — it
is not a command and consumes no press.

**What the panel answers today.** Arrows, `OK`, `Back`, the digits and `Menu` work
(P1-1 added the menu); `Home`, the attention queue, the add dialog and the machine
filter are still P1 and are **not** drawn in the hint line until they have a
handler. The `Home` row below is the plan, not a shipped behaviour — and which of
these buttons a given remote actually sends is unverified (§11: the hardware matrix
is empty).

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
| No `Menu` button (sends nothing) | the menu is unreachable **from the remote** — this is now a real loss (P1-1 landed it), and the reason §9.4's `aliases` matters | `⌘⇧K` on a keyboard, or the pointer's `⋯` in the panel's bar. Everything the menu configures (layout, order, remove, open in the window) is reachable there, so a remote without the button is usable but permanently stuck in the automatic layout |
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

### 9.5 The attention queue

The question a person watching several conversations asks most often is "which one
wants something from me". The answer is deliberately narrow — a conversation is not
interesting because it is busy:

| State | In the queue? | Why |
| --- | --- | --- |
| a pending approval (`approvals > 0`, or the run reporting `waiting_approval`) | **yes**, `Approval` | the only state that blocks the agent; it outranks a failure for the same reason |
| a failed run | **yes**, `Failed` | nobody resumes it by waiting |
| a run that finished while the reader was elsewhere (the rail's unread mark) | **yes**, `Finished` | worth a visit; nothing is blocked |
| queued, running, cancelled | **no** | a queue that included "running" would be every cell — a slower version of the arrow keys |
| finished and already read | **no** | it is just a finished run |

Two places answer it, from two distances: the panel's bar counts ("2 need you") and a
cell's header marks *why* it is waiting (`Failed`, `Finished`; an approval already has
its count badge). The key jumps to the **next waiting cell clockwise from where the
reader is** and lands on its **reading layer** — "what does this want from me" is
answered by opening it, and one more press would make the queue pointless.

With nothing waiting the key is **unclaimed** (not registered at all), which is the
panel's rule for every command: a key that would do nothing must stay the app's own.
The menu still answers the question, with a disabled "nothing is waiting" row rather
than an absent one — an empty answer is still an answer when it is what was asked.

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

Automatable — and where each one lives:

| Property | Test |
| --- | --- |
| **The invariant**: with cells on screen exactly one editable element exists once the input layer is open, and none while it is not | `VibeStage.test.tsx` (panel side), `AgentThread.chrome.test.tsx` (view side) |
| The reading chrome mounts no composer at all, so closing the layer cannot be undone by an auto-focus | `AgentThread.chrome.test.tsx` |
| A dictation's `⏎` never sends: `isComposing` and `keyCode 229` send nothing; the `⏎` after `compositionend` sends the trimmed text | `Composer.inputLayer.test.tsx`, `dispatch.test.ts` |
| The three meanings of the send key: empty `⏎` closes without sending; non-empty `⏎` sends; `Shift+⏎` is a newline | `Composer.inputLayer.test.tsx` |
| One layer at a time: `Esc` walks input → reading → grid → closed, and the grid's keys do not run over an editor | `VibeStage.test.tsx`, `dispatch.test.ts` |
| Order: created-time default; a user order wins; a new pin lands by its `createdAt`; an unpin moves nobody; the ring follows the conversation, not the index | `order.test.ts`, `VibeStage.test.tsx` |
| Membership: pinned and live only; a pinned workspace adds nothing; the remove key unpins the focused cell | `VibeStage.test.tsx` |
| The grid table (1×1 … 4×3), the narrow-window stack, the explicit column override, no wrap at an edge | `layout.test.ts` |
| The hint line lists exactly the commands with a handler **and** a binding | `hints.test.ts` |
| Membership across machines: a host's `pinned` only, a status-less host still counts, a filtered machine shows only its own | `vibeCells.test.ts` |
| The panel's machine filter: the menu row walks all → this machine → each host, and the choice is remembered | `VibeStage.test.tsx` |
| A host's cell is read-only: `OK` on it opens its reading layer, never an input layer, and its hint line does not offer one | `VibeStage.test.tsx` |
| Adding: the dialog lists unpinned conversations of the chosen machine and workspace, and pins on the machine that owns them | `VibeStage.test.tsx` |
| A host's view in a cell mounts no composer and does not narrow the host's event lane | `RemoteConversationView.chrome.test.tsx` |
| The settings line shows the cell's own model, on both the reading and the input layer, and the input layer's composer is asked for values only | `VibeStage.test.tsx`, `Composer.inputLayer.test.tsx` |
| The menu's three settings rows exist for a local cell, adjust with `←`/`→`, and are absent for a host's | `VibeStage.test.tsx` |
| The second column: `auto` follows the width, `pinned` ignores it, `hidden` is never there; `⌘\` returns to `auto` rather than to `pinned`; the choice is per conversation and survives a relaunch | `readingPanel.test.ts`, `VibeStage.test.tsx` |
| The bar reserves the macOS traffic-light box on macOS outside fullscreen, and uses the ordinary inset otherwise | `barChrome.test.ts`, plus the harness reporting `padding-left: 80px` and the label at x = 80 |
| The bar is the window's drag surface, and its three buttons still click | `VibeStage.test.tsx` |
| `core:window:allow-set-fullscreen` is granted to the main window, and no `deny-` entry undercuts it | `fullscreenCapability.test.ts` |
| A refused fullscreen reaches the reader as an error toast, not silence | `VibeStage.test.tsx` |
| The queue: one per conversation (a second is refused), visible on the cell, handed back into the box when the run ends, kept across a failure, and cancelled into the box rather than discarded | `vibeQueue.test.ts`, `VibeStage.test.tsx` |
| The composer's half: a send during a run hands the text over and clears the box, a refusal leaves the text in place, and an idle send is still a send | `Composer.inputLayer.test.tsx` |
| A remote's buttons survive a rebinding and a cleared binding | `shortcuts.test.ts`, `catalog.test.ts`, `keybinding.test.ts` |

Two of these were falsified by hand before landing (the assertions were removed,
the tests went red, the assertions came back): the empty-draft close, and
"pinned only".

Still open, and honestly not covered:

* **A running conversation's refusal.** The panel does not queue (P1), and the
  visible reason lives on the composer's own state rather than in a test of the
  panel. Until P1 this is a manual check.
* **Trigger suppression.** The input layer passes no recommendation and the
  triggers are the editor's, but nothing yet asserts that a dictated `@` opens no
  menu; the editor's trigger is text-driven, so this needs a test at the editor's
  level.
* **Per-conversation column preference** (the reading layer's second column) is
  P2 and not implemented.
* **The queue's own states are component-tested, not captured.** The screenshot
  harness can report a run in flight to the *rail* (`?streaming=`), but a local
  composer's "my run is in flight" comes from a streaming assistant bubble in that
  conversation's message stream, which the fixture cannot produce without faking the
  agent. The capture therefore asserts the neighbouring property instead — that the
  queue does **not** steal an ordinary send — and the marker, the hand-back and the
  cancel row are covered in `VibeStage.test.tsx` and `Composer.inputLayer.test.tsx`.

**On hardware: not measured.** The matrix lives in
[`VIBE-REMOTE-KEYS.md` §3](VIBE-REMOTE-KEYS.md), is empty, and every cell says "untested".
Its §1 is the printable per-button checklist, and the panel's read-only key readout is
what fills it in — three `⌥⇧K` presses, then press the remote. Nothing here or in
`KEYBINDINGS.md` may claim a button works until a row there says so, and every row
records who measured it, when, on which machine and which model.

## 12. Storage

| Data | P0 | Later |
| --- | --- | --- |
| Membership | none — it is the pin | — |
| Order, per-conversation column preference, column override | `localStorage` (`future.vibe.order`, `future.vibe.prefs`) | `app.db` tables + a versioned migration (`CLAUDE.md` §7) |
| Keys | the existing shortcut store (`KEYBINDINGS.md` §Storage) | same |

The panel's view preferences follow the same rule as the theme, font size and
panel widths: per machine, read on the render path, in `localStorage`.

**Why the order is not in `app.db` yet, stated precisely** so the next person does not
start in the wrong crate: the desktop's database and its migration chain live in
`host/src/store/` (the desktop's `src-tauri/src/store.rs` is a re-export of
`future_host::store`), and the settings document's schema lives in
`packages/app-settings/` — both **outside `desktop/`**. Moving the order there is a
schema change with a versioned migration, a fresh-install fixture and a bilingual
`ER.md` update; it is a task of its own, not a line in this one. Until then the
`localStorage` keys are the only ones the panel reads, and `vibePreferences.ts` is the
only file that touches them.

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

`AgentThread` grew a `chrome` prop (`full` | `read` | `input`) instead of a second
renderer: a second message renderer would re-earn every scroll, paging and approval
bug already fixed. The three chromes are one component with the surrounding pieces
chosen per layer, and `read` simply mounts no composer — which is what turns the
single-editor invariant into a property of the tree rather than a rule someone has
to police (`VIBE.md` §5).

What P0 actually shipped, against the plan above: `layout.ts` and `order.ts` as
pure modules with their tables asserted in tests; `vibePreferences.ts` as the only
file touching the panel's storage keys; `VibeStage.tsx` holding the layers, the
membership and the geometry; `VibeHintBar.tsx` generating the hint line; and
`VibeStage` wired into `AppShell` behind `vibeOpen`, *replacing* the shell's
columns rather than covering them, so an editor can never stay mounted underneath.

## 14. Staging

What P0 landed, and what it deliberately left:

| Stage | Content |
| --- | --- |
| **P0 — done** | the panel and its grid; scale-respecting cell rendering for local conversations; membership = pin; order = `createdAt` + drag/keyboard reordering; the three layers and the single-editor invariant; the input layer with dictation, the target bar and the three state meanings; `⌘⇧V`, arrows, `⏎`, `Esc`, `1`–`9`; explicit "still running" refusal |
| **P1 — done** | the menu (`Menu`, one column, `←`/`→` adjust a value in place) · the attention queue (§9.5) · the machine filter, a host's conversations as cells, the add dialog and removal across machines · the input layer's controls moved into the menu with the values left read-only on screen (§5.6) · the send queue (§5.4) |
| **P2 — in progress** | **done:** `h`/`j`/`k`/`l` beside the arrows · the reading layer's second column with its three per-conversation states (§6) · the key checklist and read-only readout ([`VIBE-REMOTE-KEYS.md`](VIBE-REMOTE-KEYS.md), matrix still empty) · **remaining:** order in `app.db`, and the measurement pass itself |
| **P2 — later** | order in `app.db`; layout presets; a "learn this button" diagnostics page (the read-only readout is the first half); optional text size on the volume buttons |

## 15. Open questions

* Does a cell's header show the run's model? (Deferred: it is one line, and the
  reading layer already shows it.)
* Do approvals in a *non-focused* cell stay clickable with the pointer? Yes —
  the keys only reach the focused cell, but a pointer may always answer an
  approval. Recorded so the "degraded" rendering does not quietly drop it.
