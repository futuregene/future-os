# web (desktop vitest) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
summary percentages (istanbul `lines.pct`, llvm-cov "Lines") are not the acceptance number.

**Last measurement**: `test/cov-web` cut from `origin/dev-huichen@d8719bedb` (wave 3 merge),
`npx vitest run --coverage --coverage.reporter=lcov` on Node from `rust-toolchain.toml`'s
neighbour (the desk's Node 24; the suite also passes on 22).

**Result**: `DA:0` = **171** lines in **63** files. **22** of them are waived here
(22 records, 22 distinct constructs); the other **149** are *reachable and not yet written*
— they are work items, listed per file in the campaign's working report, **not** waivers.

```bash
cd desktop
npx vitest run --coverage --coverage.reporter=lcov --coverage.reporter=json-summary
# Metric A with nothing but the lcov file (works from a clean clone):
awk '/^SF:/{f=substr($0,4)} /^DA:[0-9]+,0$/{split(substr($0,4),a,","); print f":"a[1]}' \
  coverage/lcov.info | sed 's|^src/|desktop/src/|'
```

Category names (`W1`–`W8`) are the ones defined in `.future/memory/coverage-llvm-cov.md`:
W1 cfg(windows) · **W2 defensive dead arm** · W3 OS failure injection · W4 race window ·
W5 attribution artifact · W6 ghost line · W7 mock closure · **W8 dead producer**.
That list was written for Rust/llvm-cov; only **W2** and **W8** map onto a vitest/istanbul
lane. **No row here is W3, W4, W5, W6 or W7** — nothing in this list needs another
platform, a broken filesystem, a race window, or a mock shape.

Each row states the code that makes it unreachable, the test that pins that code (where one
exists), and the falsifier. The ledger convention (`docs/testing/waiver-ledger.md`) applies:
**a waiver that cannot name its own falsifier is not evidence.**

## The 22 lines

| # | file:line | category | construct | pinned by |
|---|---|---|---|---|
| 1 | `desktop/src/lib/theme.ts:229` | **W2** | re-read guard: `if (mode !== "system") return` | `src/lib/theme.test.ts` :: *stops after a few attempts instead of polling for ever* |
| 2 | `desktop/src/features/terminal/tabs.ts:160` | **W2** | post-loop fallback `return used.size + 1` | `src/features/terminal/tabs.test.ts` :: *round-trips the view state* |
| 3 | `desktop/src/features/vibe/attention.ts:74` | **W2** | post-loop fallback `return waiting[0]!` | `src/features/vibe/VibeStage.test.tsx` :: *walks the queue in panel order and wraps* |
| 4 | `desktop/src/features/agent/threadMessageCache.ts:49` | **W2** | eviction's `if (oldest === undefined) break` | `src/features/agent/threadMessageCache.test.ts` :: *bounds snapshots and refreshes recency on reads* |
| 5 | `desktop/src/features/agent/NewConversation.tsx:149` | **W2** | effect's `setSelectedWorkspace(first.id)` | `src/features/agent/NewConversation.test.tsx` :: *falls back to the first workspace when the selected one disappears* |
| 6 | `desktop/src/components/layout/DeviceSelector.tsx:80` | **W2** | `takeOver`'s `if (!onTakeControl \|\| takingOver) return` | `src/components/layout/DeviceSelector.test.tsx` :: *says when another paired device is driving a machine* |
| 7 | `desktop/src/features/agent/ThreadHeader.tsx:65` | **W2** | `openUsage`'s `if (!thread?.agentSessionId) return` | `src/features/agent/ThreadHeader.test.tsx` :: *omits the button without an agent session and reports missing usage honestly* |
| 8 | `desktop/src/features/filetree/useFileTree.ts:47` | **W2** | LRU's `if (!oldest) break` | *(static: the cache is keyed by workspace path)* |
| 9 | `desktop/src/components/layout/RailViewSwitch.tsx:78` | **W2** | `viewAt`'s `if (!node) continue` | `src/components/layout/RailViewSwitch.test.tsx` :: *exchanges the two segments when one is dragged onto the other* |
| 10 | `desktop/src/features/vibe/VibeStage.tsx:435` | **W2** | `stepFocus`'s `if (slotCount === 0) return` | `src/features/vibe/VibeStage.test.tsx` :: *does not step past the ends of the row while reading* |
| 11 | `desktop/src/features/vibe/VibeStage.tsx:480` | **W2** | `vibe.open`'s `if (!focused) return` | `src/features/vibe/VibeStage.test.tsx` :: *falls back to the first cell when the tile the ring sits on has been filled* |
| 12 | `desktop/src/features/vibe/VibeStage.tsx:556` | **W2** | `vibe.queue.cancel`'s `if (!focusedLocal) return` | `src/features/vibe/VibeStage.test.tsx` :: *cancels from the input layer's own key* |
| 13 | `desktop/src/features/vibe/VibeStage.tsx:617` | **W2** | `vibe.attention`'s `if (!target) return` | `src/features/vibe/VibeStage.test.tsx` :: *jumps to the next cell that needs a human, and lands on its reading layer* |
| 14 | `desktop/src/features/vibe/VibeStage.tsx:672` | **W2** | `cellBody`'s `if (!catalog \|\| !cell.session) return null` | `src/features/vibe/VibeStage.test.tsx` :: *puts a host's pinned conversations in the grid beside this machine's* |
| 15 | `desktop/src/features/vibe/VibeStage.tsx:752` | **W8** | add-menu row's `if (cell) setFocus(cell)` — the producer is finite | `src/features/vibe/VibeStage.test.tsx` :: *adds a recent conversation from the tile's list, and rings it* |
| 16 | `desktop/src/features/vibe/VibeStage.tsx:943` | **W2** | `open-in-shell` row's `if (!focused) return` | `src/features/vibe/VibeMenu.test.tsx` :: *does not run a disabled row* |
| 17 | `desktop/src/features/vibe/VibeStage.tsx:1271` | **W2** | reading transcript's dblclick `if (!focused) return` | `src/features/vibe/VibeStage.test.tsx` :: *opens a host's conversation in the shell from the reading layer's transcript* |
| 18 | `desktop/src/features/settings/AccountPage.tsx:60` | **W2** | `handleRecharge`'s `if (!platformUrl) return` | `src/features/settings/AccountPage.test.tsx` :: *disables the recharge button until the platform URL is known* |
| 19 | `desktop/src/features/settings/EnvironmentPage.tsx:45` | **W2** | `handleSwitch`'s `if (!changed) return` | `src/features/settings/EnvironmentPage.test.tsx` :: *ignores a switch request while the picker matches the active environment* |
| 20 | `desktop/src/features/settings/CommunityEditionSection.tsx:27` | **W2** | `handleSwitch`'s `if (selected === communityEdition) return` | *(static: the save button is disabled on that same comparison)* |
| 21 | `desktop/src/features/runs/RunsPanel.tsx:155` | **W2** | `archiveFinished`'s `if (archiving \|\| finishedCount === 0) return` | `src/features/runs/RunsPanel.test.tsx` :: *renders the first page and appends another page near the bottom* |
| 22 | `desktop/src/features/terminal/TerminalPanel.tsx:239` | **W2** | `ErrorBar`'s `if (!error) return null` | `src/features/terminal/TerminalPanel.test.tsx` :: *shows a create failure with a retry and no home-fallback shortcut* |

## 1. `theme.ts:229` — W2

The re-read after the native pin is cleared re-checks the mode before touching anything.

```ts
274: export function setThemeMode(next: ThemeMode): void {
275:   ensureSystemListener();
276:   if (systemRecheck) {
277:     clearTimeout(systemRecheck);
278:     systemRecheck = null;
279:   }
280:   mode = next;
```

`mode` is module-private and this is its **only** writer, and it clears the pending
re-read *before* the assignment, so a scheduled `recheckSystemTheme` can only run while
`mode` is still `"system"` — the one state in which line 229's guard is false.

- **Pinned by** `src/lib/theme.test.ts` :: *stops after a few attempts instead of polling for ever*
  — arms the re-read, advances the clock past all four reads, and asserts nothing is left scheduled.
- **Positive control (stated)**: give `setThemeMode` a second writer of `mode` (e.g. move line 280
  above the `clearTimeout`) → the pending callback observes `mode !== "system"` and line 229
  executes. Falsifier: any code path that assigns `mode` without clearing `systemRecheck`.
- Note: the only way it could run in a real engine is a timer already dequeued by the event
  loop in the same turn — not reachable by a deterministic test, which is why it is W2 and not W4.

## 2. `tabs.ts:160` — W2

```ts
155:   const used = new Set(all.map(tab => tab.titleNumber).filter(number => number > 0));
156:   for (let index = 1; index <= used.size + 1; index += 1) {
157:     if (!used.has(index))
158:       return index;
159:   }
160:   return used.size + 1;
```

The loop enumerates `1 … used.size + 1`; `used` holds at most `used.size` distinct values, so
at least one of those indices is free and the loop always returns. Line 160 needs
`used.size + 1` distinct values in a set of size `used.size`.

- **Pinned by** `src/features/terminal/tabs.test.ts` :: *round-trips the view state* (a tab
  carrying a number is the only way `used` is non-empty).
- **Positive control (stated)**: change the bound to `index < used.size + 1` → the loop can
  exhaust without returning and line 160 executes for a fully-numbered panel.

## 3. `attention.ts:74` — W2

```ts
59:   const waiting = keys.filter(key => reasons.has(key));
62:   if (!fromKey) return waiting[0]!;
63:   const from = keys.indexOf(fromKey);
64:   if (from === -1) return waiting[0]!;
65:   for (let offset = 1; offset <= keys.length; offset++) {
66:     const candidate = keys[(from + offset) % keys.length]!;
67:     if (reasons.has(candidate)) return candidate;
68:   }
74:   return waiting[0]!;
```

`waiting ⊆ keys` by construction, and the loop visits every key, so with `waiting` non-empty
it returns inside the loop. Line 74 needs a waiting key that is not in `keys`.

- **Pinned by** `src/features/vibe/VibeStage.test.tsx` :: *walks the queue in panel order and wraps*.
- **Positive control (stated)**: pass a `reasons` map keyed by a cell that is not in `keys`
  (`nextAttention(["a"], new Map([["b","failed"]]), "a")`) → line 74 executes.

## 4. `threadMessageCache.ts:49` — W2

```ts
46:   while (snapshots.size > MAX_CACHED_THREADS) {
47:     const oldest = snapshots.keys().next().value;
48:     if (oldest === undefined)
49:       break;
50:     snapshots.delete(oldest);
```

The loop's own condition guarantees `size > MAX_CACHED_THREADS ≥ 1`, so the map is non-empty
and its key iterator always yields a value.

- **Pinned by** `src/features/agent/threadMessageCache.test.ts` :: *bounds snapshots and refreshes recency on reads*.
- **Positive control (stated)**: set `MAX_CACHED_THREADS` negative (`size > -1` is true on an
  empty map) → line 49 executes on the first read.

## 5. `NewConversation.tsx:149` — W2

```ts
143:   const activeWorkspace
144:     = workspaceOptions.find(workspace => workspace.id === selectedWorkspace) ?? workspaceOptions[0];
147:   useEffect(() => {
148:     const first = workspaceOptions[0];
149:     if (activeWorkspace || !first) return;   // ← 147-149; the body is 149's sibling
```

`activeWorkspace` already falls back to `workspaceOptions[0]`, so whenever `first` exists
`activeWorkspace` is truthy; whenever `first` is missing the guard's second half fires. The
effect body (`setSelectedWorkspace(first.id)`) therefore has no state to run in.

- **Pinned by** `src/features/agent/NewConversation.test.tsx` :: *falls back to the first workspace when the selected one disappears*.
- **Positive control (stated)**: drop the `?? workspaceOptions[0]` fallback from line 144 →
  with a selection that no longer matches, `activeWorkspace` is undefined and the body runs.

## 6. `DeviceSelector.tsx:80` — W2

```ts
153: {held && onTakeControl
154:   ? (
155:       <button
...
158:         disabled={takingOver !== null}
159:         onClick={() => void takeOver(peer.desktopId)}
```

The only caller of `takeOver` is that button, which is rendered *only* when `onTakeControl`
is provided and is `disabled` while a takeover is in flight — both halves of line 80's guard
are already enforced one layer up.

- **Pinned by** `src/components/layout/DeviceSelector.test.tsx` :: *says when another paired
  device is driving a machine* (the row that carries the control).
- **Positive control (stated)**: render the control without `disabled` (or call `takeOver`
  from a second affordance) → line 80 executes.

## 7. `ThreadHeader.tsx:65` — W2

```ts
63:   function openUsage() {
64:     if (!thread?.agentSessionId) return;
72:   useCommand("thread.usage", openUsage, Boolean(thread?.agentSessionId));
```

The command is registered with `enabled = Boolean(thread?.agentSessionId)` and the button is
drawn under the same condition, so the handler cannot be reached for a thread without a session.

- **Pinned by** `src/features/agent/ThreadHeader.test.tsx` :: *omits the button without an agent session and reports missing usage honestly*.
- **Positive control (stated)**: register the command unconditionally
  (`useCommand("thread.usage", openUsage, true)`) and run it on a session-less thread → line 65 executes.

## 8. `useFileTree.ts:47` — W2

```ts
44:   while (cache.size > CACHE_MAX_ROOTS) {
45:     const oldest = cache.keys().next().value;
46:     if (!oldest) break;
47:     cache.delete(oldest);
```

`cache` is keyed by workspace root (an absolute path), so no key is the empty string — the
only falsy value a `string` key can take — and the loop's condition guarantees the iterator
yields. Line 47 (the `break`) needs an empty-string root.

- **Pinned by** *(static: `stateFor(root)` is only called with a resolved workspace path; `FileTreePanel.test.tsx` mounts the panel for that root)*.
- **Positive control (stated)**: mount a tree for the root `""` after filling the cache past
  `CACHE_MAX_ROOTS` → the eviction loop meets the falsy key and breaks.

## 9. `RailViewSwitch.tsx:78` — W2

```ts
76:   for (const id of order) {
77:     const node = nodesRef.current.get(id);
78:     if (!node) continue;
```

`nodesRef` is filled by the two segments' own ref callbacks, and the segments are rendered by
mapping the same `order` — a key in `order` therefore always has a node.

- **Pinned by** `src/components/layout/RailViewSwitch.test.tsx` :: *exchanges the two segments when one is dragged onto the other*.
- **Positive control (stated)**: render one segment without registering its ref → the drag
  walks past it and line 78 executes.

## 10–17. `VibeStage.tsx` — W2 ×7, W8 ×1

The panel registers a command for its **layer**, and the thing the command acts on can be
absent; each guard below is the case its own registration rule already excludes.

- **435 `stepFocus`, W2** — `slotCount = shown.length + (addSlot ? 1 : 0)` and
  `addSlot = hasAddSlot(shown.length) && columns*rows > shown.length`. With
  `shown.length === 0`, `hasAddSlot(0)` is true (`emptySlot.ts:49-51`) and the grid is at
  least 1×1, so `slotCount ≥ 1` always. Pinned by *does not step past the ends of the row
  while reading*. Control: make `hasAddSlot` false for an empty page.
- **480 `vibe.open`, W2** — `focused` is null only when `focusIndex === shown.length`, and the
  only branch producing that value is `ADD_SLOT_KEY` with `addSlot` true (`278-281`), in which
  case `addFocused` is true and the command returns at 475. Pinned by *falls back to the first
  cell when the tile the ring sits on has been filled*. Control: let `focusIndex` exceed
  `shown.length` while `addSlot` is false.
- **556 `vibe.queue.cancel`, W2** — registered as
  `useCommand("vibe.queue.cancel", …, layer === "input" && queue.queued.has(focusedLocal?.id ?? ""))`;
  with `focusedLocal` null the key is `""`, which no queue holds. Pinned by *cancels from the
  input layer's own key*. Control: register it unconditionally.
- **617 `vibe.attention`, W2** — `attention` is built only from `shown` (`344-360`), so
  `attentionCount > 0` implies a shown cell with a reason, and `nextAttention` necessarily
  returns one of `shown`'s keys. Pinned by *jumps to the next cell that needs a human, and
  lands on its reading layer*. Control: hand `nextAttention` a reason for a key outside `shown`.
- **672 `cellBody`, W2** — `members = vibeCellsOf(threads, catalogs, …)`: a member with a
  `desktopId` came from a catalogue in *this* render's `catalogs`. Measured 2026-11-01:
  withdrawing `catalogs` removes the member entirely, so the function is never called. Pinned
  by *puts a host's pinned conversations in the grid beside this machine's*. Control: keep a
  member across the render that drops its catalogue.
- **752 add-menu row, W8** — the row is produced by `recentChoices(threads, catalogs, new
  Set(members.map(cell => cell.key)))`, i.e. from conversations that are *not* members, so
  `cells.find(item => item.key === choice.key)` is `undefined` for every row it can emit: the
  producer is dead, not the consumer. Pinned by *adds a recent conversation from the tile's
  list, and rings it* (which shows the pin happening and the ring staying put). Control: pass
  `members` to `recentChoices` unfiltered.
- **943 `open-in-shell` row, W2** — the row carries `disabled: !focused`, and the menu's walk
  stops only on `:not(:disabled)` items (`VibeMenu.tsx:78-83`) and runs a row only when
  `!row.disabled` (`:84-88`). Pinned by `VibeMenu.test.tsx` :: *does not run a disabled row*.
  Control: let the walk land on a disabled row.
- **1271 transcript dblclick, W2** — the reading layer is only drawn when `shown.length > 0`
  (`1215-1217`), and no key moves the ring to "no cell" while that layer is open: the
  `ADD_SLOT_KEY` state is produced by the add tile, which exists only in the grid layer.
  Pinned by *opens a host's conversation in the shell from the reading layer's transcript*.
  Control: allow the reading layer to survive its last cell.

## 18–22. Settings, runs and the terminal — W2 ×5

Each is a handler whose only caller is a control disabled (or hidden) under the same
condition as the guard.

- **`AccountPage.tsx:60`** — `disabled={balanceStatus !== "unavailable" && !platformUrl}`
  (`:190`), and when `balanceStatus === "unavailable"` the same button calls `onRefreshBalance`
  instead (`:191`). **Measured 2026-11-01**: with `disabled` removed and the suite re-run,
  `DA:60` stays **0** (line 61 executes instead — the clicks that reach the handler all have a
  URL) and the two tests asserting the disabled state fail. So the arm needs *both* the
  attribute removed and a click the suite does not make; the falsifier is a click on a
  disabled button, which jsdom and every browser swallow.
- **`EnvironmentPage.tsx:45`** — the only caller is `disabled={!changed || busy}` (`:84`).
- **`CommunityEditionSection.tsx:27`** — the only caller is
  `disabled={selected === communityEdition || saving}` (`:52`). *(No dedicated test file:
  the page is exercised through `SettingsDialog`.)*
- **`RunsPanel.tsx:155`** — the only caller is `disabled={archiving || finishedCount === 0}` (`:182-184`).
- **`TerminalPanel.tsx:239`** — `ErrorBar` is rendered only under
  `{tabs.createError ? <ErrorBar tabs={tabs} /> : null}` (`:139`).

## Wave 5 (layout group) — no new rows

The layout-and-interaction group's own residual went **76 → 59** in this wave (the lane's total
188 → 171). Nothing in it was judged unreachable, so this file gains no row. One line was
examined closely and is recorded here as **reachable**, because the difference matters:

- `desktop/src/features/tasks/TasksView.tsx:376` — `confirmDelete`'s `if (!selected) return`.
  `selected` is `store.tasks.find(task => task.id === selectedId) ?? null` (`:261-264`), and the
  dialog that calls this stays up while the store keeps reloading: a task deleted by another
  client (or by the same schedule finishing) removes the row under an open confirmation. That is
  a state a test can build by changing the store between opening the dialog and pressing Delete,
  which is exactly why it is a work item and not a W2 arm.

## How to retire a row

A row leaves this file only together with the change that makes its line reachable, and the
change must be accompanied by a test that executes it. Re-run the command in the header and
confirm the line is gone from the `DA:0` list; a row whose line has disappeared *without* such
a test means the measurement moved, not the code — check the denominator (`lines.total`, which
is 16331 for this lane and identical across the campaign's four runs so far).
