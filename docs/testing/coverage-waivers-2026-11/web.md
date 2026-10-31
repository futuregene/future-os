# web (desktop vitest) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
summary percentages (istanbul `lines.pct`, llvm-cov "Lines") are not the acceptance number.

**Last measurement**: `test/cov-web` cut from `origin/dev-huichen@d8719bedb` (wave 3 merge),
`npx vitest run --coverage --coverage.reporter=lcov` on Node from `rust-toolchain.toml`'s
neighbour (the desk's Node 24; the suite also passes on 22).

**Result**: `DA:0` = **159** lines in **61** files. **33** of them are waived here
(33 records: 29 `W2` + 4 `W8`; 26 distinct constructs — the rail's four feature rows share one cause); the other **126** are *reachable and not yet written*
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
| 23 | `desktop/src/components/layout/AppShell.tsx:1058` | **W2** | `handlePreviewLeftPanel`'s `if (showLeftPanel) return` | `src/components/layout/AppShell.test.tsx` :: *previews the collapsed rail on hover and drops it again on leave* |
| 24 | `desktop/src/components/layout/AppShell.tsx:1261` | **W2** | `openRemoteConversation`'s `if (conversation.desktopId === null) return` | `src/components/layout/AppShell.test.tsx` :: *opens a remote conversation on the machine that owns it* |
| 25 | `desktop/src/components/layout/ActivityRail.tsx:1107` | **W2** | `stepConversation`'s `if (walkableRows.length === 0) return` | `src/components/layout/ActivityRail.nav.test.tsx` :: *leaves the keys unclaimed on a section with no list* |
| 26 | `desktop/src/components/layout/ActivityRail.tsx:1125` | **W2** | `stepGroup`'s `if (groupStops.length === 0) return` | `src/components/layout/ActivityRail.nav.test.tsx` :: *walks to the next and previous conversation in the drawn order* |
| 27 | `desktop/src/components/layout/ActivityRail.tsx:1165` | **W2** | `stepDevice`'s `if (!onChangeDeviceFilter) return` | `src/components/layout/ActivityRail.remote.test.tsx` :: *shows only the chosen machine's conversations when one is filtered* |
| 28 | `desktop/src/components/layout/ActivityRail.tsx:1360` | **W2** | `featureItems.length > 0` branch's `<NavButton>` open | `src/components/layout/ActivityRail.nav.test.tsx` :: *renders exactly the expected set of nav entries* |
| 29 | `desktop/src/components/layout/ActivityRail.tsx:1365` | **W2** | the same row's `onClick` | same |
| 30 | `desktop/src/components/layout/ActivityRail.tsx:1684` | **W8** | collapsed rail's `featureItems.map((item) => { const Icon = item.icon;` | same |
| 31 | `desktop/src/components/layout/ActivityRail.tsx:1685` | **W8** | the same callback's `return (` | same |
| 32 | `desktop/src/components/layout/ActivityRail.tsx:1691` | **W8** | the same row's `onClick` | same |
| 33 | `desktop/src/components/layout/ActivityRail.tsx:1761` | **W2** | `sortThreads`' `return a.status === "active" ? -1 : 1` | `src/components/layout/ActivityRail.recent.test.tsx` :: *draws every conversation flat, with no heading for where it lives* |

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

## Wave 6 (layout group) — rows 25-33 added

The lane's residual went **168 → 159** with three adapters covered (`ActivityRail`'s host row
from the keyboard, `VibeRemoteCell`'s five actions, `useNewConversation`'s pin flow), and nine
lines in `ActivityRail` were proven unreachable: one `W2` for each of the three walk guards
(their commands are registered only while they have somewhere to go), and the four-row block
driven by a module constant that is deliberately empty — one `W2` for the branch, three `W8`
for the mapper body. `sortThreads`' status tiebreak is the ninth (`W2`: its only caller
pre-filters to `status === "active"`).

## Wave 5 (layout group) — rows 23-24 added

The layout-and-interaction group's own residual went **76 → 57** in this wave (the lane's total
188 → 168), and two of its lines turned out to be guards no caller can reach, so this file gains
rows **23** and **24** (`AppShell.tsx:1058`, `:1261`) — both proven by their call sites being
rendered/called only under the opposite condition. One line was
examined closely and is recorded here as **reachable**, because the difference matters:

- `desktop/src/features/tasks/TasksView.tsx:376` — `confirmDelete`'s `if (!selected) return`.
  `selected` is `store.tasks.find(task => task.id === selectedId) ?? null` (`:261-264`), and the
  dialog that calls this stays up while the store keeps reloading: a task deleted by another
  client (or by the same schedule finishing) removes the row under an open confirmation. That is
  a state a test can build by changing the store between opening the dialog and pressing Delete,
  which is exactly why it is a work item and not a W2 arm.

## 23. `AppShell.tsx:1058` — W2

The left panel's hover-preview is armed by two elements, and **both are rendered only
while the panel is collapsed**:

```ts
1655:               {!showLeftPanel
1657:                     <div
1660:                       onMouseEnter={() => handlePreviewLeftPanel(true)}
1664:               {!showLeftPanel && leftOverlayOpen
1669:                       onMouseEnter={() => handlePreviewLeftPanel(true)}
1670| onMouseLeave={() => handlePreviewLeftPanel(false)}
```

`handlePreviewLeftPanel` (`1056-1060`) starts with `if (showLeftPanel) return;`, so the
only states that can call it are states where the guard is false by construction. It is
a guard written for a third caller that does not exist yet.

- **Pinned by** `src/components/layout/AppShell.test.tsx` :: *previews the collapsed rail on
  hover and drops it again on leave* — the two elements that call it exist only while the
  panel is collapsed.
- **Positive control (stated)**: render the hover edge unconditionally (drop the
  `!showLeftPanel &&` on line 1664) → with the panel shown, a hover reaches line 1058.
- **Falsifier**: any call site outside a `!showLeftPanel` branch.

## 24. `AppShell.tsx:1261` — W2

The rail's merged-row opener splits the two kinds before it calls this shell handler:

```ts
ActivityRail.tsx:946:   const openRow = useCallback((row: MergedConversation) => {
947:     if (row.desktopId === null) {
948:       const thread = threadById.get(row.id);
950:         onSelectThread(thread);
951:       return;                       // ← this machine's rows never leave here
952:     }
953| onOpenRemoteConversation?.(row);
```

and `onOpenRemoteConversation` is only ever fed from that `else` branch (`AppShell.tsx:1474`
binds it). So `openRemoteConversation` (`1259-1262`) can only be called with a non-null
`desktopId`.

- **Pinned by** `src/components/layout/AppShell.test.tsx` :: *opens a remote conversation on
  the machine that owns it* (the positive half: a host's row does reach the shell).
- **Positive control (stated)**: route `openRow`'s local branch through
  `onOpenRemoteConversation` as well → a local row reaches line 1261 and returns.
- **Falsifier**: a second caller of `onOpenRemoteConversation` that can pass a local row.

## 25-33. `ActivityRail.tsx` — W2 ×5, W8 ×3

### 25-27 — the three walk guards

Each walk command is registered only while it has somewhere to go, so its own
empty-list guard cannot be reached:

```ts
1140:  useCommand("thread.previous", () => stepConversation(-1), listShowing && walkableRows.length > 0);
1141:  useCommand("thread.next", () => stepConversation(1), listShowing && walkableRows.length > 0);
1144:  useCommand("group.previous", () => stepGroup(-1), listShowing && groupStops.length > 1);
1145:  useCommand("group.next", () => stepGroup(1), listShowing && groupStops.length > 1);
1181:  useCommand("rail.device.next", stepDevice, listShowing && Boolean(onChangeDeviceFilter) && remotePeers.length > 0);
```

`stepConversation` (`1105-1107`), `stepGroup` (`1123-1125`) and `stepDevice` (`1163-1165`) are
their own only callers' targets — the guard inside each is the registration condition
repeated. The `thread.*`/`group.*` keys are absent with no rows at all, and
`rail.device.next` is absent without `onChangeDeviceFilter` or any paired machine.

- **Pinned by** `ActivityRail.nav.test.tsx` :: *leaves the keys unclaimed on a section with no
  list* (the key does not even fire, and is not consumed), *walks to the next and previous
  conversation in the drawn order* (the positive half), and `ActivityRail.remote.test.tsx` ::
  *shows only the chosen machine's conversations when one is filtered*.
- **Positive control (stated)**: register each command unconditionally (`useCommand(id, fn)`)
  → pressing the key on an empty list reaches line 1107 / 1125 / 1165.
- **Falsifier**: a second caller of `stepConversation`, `stepGroup` or `stepDevice`.

### 28-32 — the feature rows (one W2, three W8)

`featureItems` is a module constant, deliberately empty:

```ts
166: // Data / Skill entries are temporarily hidden from the navigation:
167: // these modules have been deprioritised (see PLAN.md "Next Priorities").
168: const featureItems: Array<{ id: ActivitySection; label: string; icon: LucideIcon }> = [];
```

so `featureItems.length > 0` (`1356`) is always false and `featureItems.map(...)` (`1683`)
never calls its callback. Lines 1360/1365 are the dead branch (W2); 1684/1685/1691 are the
never-invoked mapper body (W8 — the producer is the constant, and it produces nothing).

- **Pinned by** `ActivityRail.nav.test.tsx` :: *renders exactly the expected set of nav entries*
  — the labelled set is asserted verbatim, so a re-enabled feature row makes that test
  fail before it can silently appear.
- **Positive control (stated)**: give the constant one entry → both blocks render and all
  five lines execute.
- **Falsifier**: any assignment to `featureItems` besides the empty literal.

### 33 — `sortThreads`' status tiebreak

`sortThreads` is only ever called on threads already filtered to one status:

```ts
371:   const visibleThreads = useMemo(
372:     () => sortThreads(threads.filter(thread => thread.status === "active")),
373:     [threads],
374:   );
```

so the comparator's `a.status !== b.status` (`1760`) cannot be true and the `-1 : 1` arm
(`1761`) is unreachable. The tiebreak stays as defence for a second caller that does not
pre-filter.

- **Pinned by** `ActivityRail.recent.test.tsx` :: *draws every conversation flat, with no
  heading for where it lives* (the list's own order is what those assertions read).
- **Positive control (stated)**: call `sortThreads` with an unfiltered list (one active, one
  archived) → line 1761 executes.
- **Falsifier**: a caller of `sortThreads` that does not pre-filter by status.

## How to retire a row

A row leaves this file only together with the change that makes its line reachable, and the
change must be accompanied by a test that executes it. Re-run the command in the header and
confirm the line is gone from the `DA:0` list; a row whose line has disappeared *without* such
a test means the measurement moved, not the code — check the denominator (`lines.total`, which
is 16331 for this lane and identical across the campaign's four runs so far).
