# Remote-control key checklist and the self-check readout

→ [中文](VIBE-REMOTE-KEYS.zh-CN.md) (the printable one-page table is in the Chinese
file; this file is the same content in English so the pair stays checkable).

Status: **the checklist and the readout are in place; the measured matrix is empty and
every cell says "untested".**

This file does one thing: turn "what a given button actually sends" from an assumption
into a measurement. The panel's key design (`VIBE.md` §9) rests on a set of assumptions,
and **not one of them has been verified on hardware**. So:

* the first table below is the **checklist** — each button, and the `key` / `code` /
  modifiers the browser *may* produce. That column is a **reference, not a finding**;
  anything that varies by model says so.
* the second is the **measured matrix** — measured values only. An untested cell stays
  empty and says "untested"; it never carries a guess.
* every measured row records **who, when, which machine (OS + desktop version) and which
  model**.

## 1. The checklist

| # | Button | Expected `code` | Expected `key` | Modifiers | Note |
| --- | --- | --- | --- | --- | --- |
| 1 | Up | `ArrowUp` | `ArrowUp` | none | Holding it repeats (the readout shows `repeat`) |
| 2 | Down | `ArrowDown` | `ArrowDown` | none | as above |
| 3 | Left | `ArrowLeft` | `ArrowLeft` | none | as above |
| 4 | Right | `ArrowRight` | `ArrowRight` | none | as above |
| 5 | OK / centre | `Enter` | `Enter` | none | a few models send `NumpadEnter` |
| 6 | Back | `Escape`, or `BrowserBack` / `GoBack` | model-dependent | none | all three have been seen; trust the readout |
| 7 | Home | `Home`, or `BrowserHome` | model-dependent | none | some models send nothing |
| 8 | Menu | `ContextMenu`, or nothing | model-dependent | none | **the least reliable button**; without it the menu is keyboard-only (`⌘⇧K`) |
| 9 | Volume + | `AudioVolumeUp` | model-dependent | none | macOS usually consumes it; it may never appear |
| 10 | Volume − | `AudioVolumeDown` | model-dependent | none | as above |
| 11 | Voice | usually not delivered to the page | — | — | goes to the system assistant |
| 12 | Power | usually not delivered to the page | — | — | sleeps the machine; **never bound** |

Four things to know about that table:

* **`code` is the physical key, `key` is what the layout types.** The app matches on
  `code` (`lib/keybinding.ts`); `key` is only a fallback for engines that send no `code`.
* **"model-dependent" is not laziness.** The same silkscreen ("menu") sends
  `ContextMenu` on one model, `F12` on another, and nothing at all on a third — which is
  exactly why `VIBE.md` §9.3's degradation table exists.
* **Do not add the volume or power buttons to the key table** unless the same model
  produces them three times in a row on the target platform.
* **A held arrow reports `repeat`**: that is the long-press auto-repeat a remote's
  navigation depends on, not an error.

## 2. How to measure

1. **Show the readout.** In a developer build, press `⌥⇧K` three times. In a packaged
   build, allow it once
   (`localStorage.setItem("future.vibe.keyProbe", "1")` in the developer tools), reopen
   the panel, then press `⌥⇧K` three times.
2. **Press the remote's buttons.** The readout floats in the panel's bottom-right corner
   and only displays: arrows still move the panel, which is deliberate — if it
   intercepted keys, what you measured would not be the real behaviour.
3. **Copy the values.** Each line reads `key="…" code="…" mods=…`; "Copy" takes six at
   once, straight into the matrix.

The readout **never participates in command dispatch**: it has no command id, is not in
the command table, and does not change `dispatch`'s decision order. It reads events with
a bubble-phase listener and never calls `preventDefault` / `stopPropagation`.

## 3. The measured matrix (measured values only)

| Model | Platform / desktop version | Operator | Date | Up | Down | Left | Right | OK | Back | Home | Menu |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| *untested* | | | | untested | untested | untested | untested | untested | untested | untested | untested |

## 4. What to change once it is filled in

1. Table 3 above — append measured rows; never edit somebody else's.
2. `VIBE.md` §9.2: a layer that the measurement changes (a model whose OK is really
   `NumpadEnter`) is fixed in the command table's `aliases`, not in prose.
3. `VIBE.md` §9.3: a row that says "planned" becomes "confirmed" once a model is shown
   to lack that button.
4. `VIBE.md` §11: the matrix there points here instead of staying empty.

**Do not** fill a cell with a guess to make the table look finished. A table that says
"untested" is worth more than one with wrong values — the latter is what makes the next
person design against the wrong keys.
