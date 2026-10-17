# Front-end waiver review (`rev-fe`) — mobile statement register, the JS statement-waiver channel, and desktop dimension claims

Reviewer task: `todo_b32b86008a82` (goal `goal_6d61125ba837`). Reviewer is **not**
the author of the artefacts below. No file outside `docs/testing/review-frontend.md`
and `mobile/coverage/` was written; no production source and no existing test was
modified.

## 0. Run identity and method

| item | value |
|---|---|
| worktree | `D:/future-os/.worktrees/cov100` |
| artefacts reviewed | `docs/testing/module-mobile.md` (§ *The per-statement waiver register*, L3033–L3062), `.future/cov100/verify.py` (`_statement_check` / `_hidden_statements` / `cmd_js`), `docs/testing/desktop-{agent,shell,settings,panels,packages}.md` § dimension evidence + waiver ledgers |
| mobile measurement used | `mobile/coverage/coverage-final.json` + `coverage-summary.json` (istanbul, 141 files): **99.81 % lines = 8260/8276** (16 uncovered lines in 7 files) and **99.52 % statements = 9654/9701** (47 uncovered statements in 18 files); the 30 hidden statements are the subset of those 47 whose line the line metric already credits as covered |
| desktop measurement used | `desktop/coverage/d-agent/d-pkgs/…/coverage-final.json` + `coverage-summary.json` (v8 provider) |
| gate re-runs | `python .future/cov100/verify.py js future-mobile-ts 99.99` run from a **throwaway mirror ROOT** (`mobile/coverage/_revfe_tmp/`: the unmodified doc copy + copies of both coverage JSONs + a directory junction to the real `mobile/src`) — so the real doc and the real reports were never touched. Hashes of `mobile/coverage/coverage-final.json` and `coverage-summary.json` were compared before/after: unchanged. |
| mobile test run used for the falsification | one throwaway probe (`mobile/coverage/_revfe_reach.test.ts`, deleted after use; source reproduced in §7), run with `npx jest --testMatch "**/coverage/**/_revfe_reach.test.ts"`, plus a private `--coverage --coverageReporters=json --coverageDirectory=coverage/_revfe_cov` for the hit counts. No workspace-wide coverage run. |

Recomputation, not trust: the 30-row hidden set was **recomputed from
`coverage-final.json` with `verify.py`'s own rule** (`_hidden_statements`, needles =
`None`, i.e. every file in the report) by an independent script
(`mobile/coverage/_revfe_probe.py`), and the register rows were re-parsed with
`verify.py`'s own `waived()` rule (`| basename:line |` in the **first** cell and a
category keyword at the **start of the second** cell).

## 1. Conclusion — **conditional pass**

* **The 30-row register is structurally exact**: 30 hidden statements, 30 live
  register rows, a 1:1 match, **no empty registration, no duplicate row, no merged
  row, no unregistered hidden statement** (§2.1). The gate accepts it, and the
  acceptance is not an artefact of a loose match (control in §3).
* **One row is falsified**: `SessionList.tsx:232` is **reachable** and was measured
  executing (`hits=[2, 1]` — the guard's `return` ran once). It must be covered by
  a test, not waived (§5.1). Its stated reason is the *same class* of argument the
  register's own §Segment 27 had already destroyed for `:184`.
* **Several reasons are "true but not for the stated reason"**, one is
  non-responsive to its own line, one is mis-categorised against the repo's waiver
  policy, and one file's waiver over-claims a coverage state while its *claim* is
  fine (§5.2). None of those changes the classification of their statements.
* **The statement-waiver channel in `verify.py` is bypassable in five ways** (§4),
  two of which I would fix (a line-number substring over-match, and no
  reconciliation of register rows against the hidden set).
* **Desktop**: 6 of 7 sampled claims reproduced exactly as written; 2 of 2 sampled
  waiver ledgers held, both with a mechanism that the ledger states imprecisely
  (§6).

Everything below says what was measured, what was only read, and what stayed
unfalsified.

## 2. The mobile 30-row statement register (Task 1)

### 2.1 Structural reconciliation (my own recomputation)

```
hidden statements (gate set)            30
live register rows found in doc         30
matched 30 / 30
register rows matching nothing in the hidden set   none
rows whose first cell holds >1 token               none
duplicate hidden targets                           none
```

So there is no **empty registration** (a row naming a covered statement) and no
blanket row hiding several statements. The hidden set and the register are the same
set, which is the strongest thing the gate's shape check can produce.

Every hidden line has the same shape: **two statements on the line, the second with
0 hits** (a one-line `if (…) return;` / `throw`). The condition's evaluation count,
measured from the module's own report (`mobile/coverage/_revfe_counts.py`), is the
first number; the body's is always 0:

| statement | hits | statement | hits | statement | hits |
|---|---|---|---|---|---|
| `DesktopsScreen.tsx:80` | [5, 0] | `files.ts:551` | [6, 0] | `syncEngine.ts:931` | [492, 0] |
| `MarkdownText.tsx:101` | [5, 0] | `files.ts:863` | [6, 0] | `useFileDownload.ts:413` | [7, 0] |
| `PreviewModal.tsx:229` | [4, 0] | `files.ts:966` | [15, 0] | `useFileDownload.ts:424` | [8, 0] |
| `RemoteContext.tsx:588` | [104, 0] | `projection.ts:981` | [2, 0] | `usePromptOutbox.ts:396` | [33, 0] |
| `SessionList.tsx:232` | **[7, 0]** | `secureChannel.ts:20` | [8127, 0] | `useRemoteConnection.ts:119` | [289, 0] |
| `SessionsScreen.tsx:178` | [2, 0] | `syncEngine.ts:595` | [146, 0] | `useRemoteConnection.ts:417` | [104, 0] |
| `client.ts:626` | [145, 0] | `syncEngine.ts:623` | [281, 0] | `useSessionCatalog.ts:82` | [525, 0] |
| `client.ts:777` | [18, 0] | `syncEngine.ts:667` | [10, 0] | `useSessionCatalog.ts:203` | [23, 0] |
| `client.ts:1033` | [6, 0] | `syncEngine.ts:799` | [906, 0] | `useTimelineController.ts:480` | [4, 0] |
| `client.ts:1101` | [14, 0] | | | `useTimelineController.ts:532` | [89, 0] |
| | | | | `useTimelineController.ts:945` | [3, 0] |

### 2.2 How the 30 split by the kind of reason they give

| reason kind | rows | falsifiable how |
|---|---|---|
| **type-requirement** (`noUncheckedIndexedAccess` / optional parameter / declared-nullable / wider union) | `files.ts:551`, `files.ts:863`, `projection.ts:981`, `syncEngine.ts:799`, `useFileDownload.ts:413`, `useFileDownload.ts:424`, `client.ts:1033`(partly), `useSessionCatalog.ts:203`, `MarkdownText.tsx:101` | read the declaration + the writer set |
| **product/config decision** | `MarkdownText.tsx:101` | read the parser's dead feature flag |
| **single spans of synchronous code after the last `await`** | `useTimelineController.ts:480`, `:532`, `useRemoteConnection.ts:417`, `syncEngine.ts` (partly), `client.ts:626` (partly) | JS run-to-completion makes these provable, not merely plausible |
| **"UI/rendered state makes the trigger impossible"** ← **the class that already fell once** | `SessionList.tsx:232`, `DesktopsScreen.tsx:80`, `PreviewModal.tsx:229`, `SessionsScreen.tsx:178`, `useRemoteConnection.ts:119`(lifecycle) | **needs a construction, not an argument** |
| **React-lifecycle / environment** | `useRemoteConnection.ts:119`, `useTimelineController.ts:945`, `DesktopsScreen.tsx:80`, `RemoteContext.tsx:588` | construction or measurement |
| **exhaustive call-site enumeration** | `client.ts:777`, `:1101`, `syncEngine.ts:595/623/667/931`, `usePromptOutbox.ts:396`, `files.ts:966` | audit every call site |

I attacked the four rows at the bottom of that table that were cheapest to construct
against, plus the whole "UI-disabled trigger" class the register itself flags.

## 3. Gate channel under attack (Task 2)

Method: the real `verify.py js future-mobile-ts 99.99` was executed from the mirror
ROOT for each mutation of the **doc copy** only. Baseline verdict:
`statement waiver: all 30 hidden statement(s) carry a per-statement category row … -> accepted`
(the command as a whole still exits non-zero in the mirror because the *file-level*
waiver rows resolve absolute paths against the mirror `ROOT`; the statement channel
is exercised before that and prints its own verdict, which is what is quoted below).

**Control** — delete one register row → `statement waiver: 1 of 30 hidden statement(s)
are NOT registered` and the missing row is named. So the harness can see a broken
register; the ACCEPTs below are not a broken harness.

| attack | what was written into the doc copy | gate verdict | needs fixing? |
|---|---|---|---|
| (a) extra row for an **already covered** statement (`MarkdownText.tsx:1`) | 31 rows | **ACCEPTED** — no finding produced, no warning | Optional. Harmless to the gate, but it means the gate cannot see **empty registration** (a row claiming a waiver for code that is already tested). The current register has none (§2.1). |
| (b) line-number **prefix** `client.ts:6` for hidden `client.ts:626` | row rewritten | **REJECTED** (`client.ts:626` reported missing) — correct, the needle must be contained | no |
| (b′) line-number **superstring** `client.ts:6260` (line does not exist) | row rewritten | **ACCEPTED** — `client.ts:626` waived by a row naming a non-existent line | **yes.** `needle in cells[0]` is an unbounded substring test; any typo/extra digit (`client.ts:10330` waives `client.ts:1033`, `files.ts:5510` waives `files.ts:551`) silently rescues a real gap. Fix: match an exact token, e.g. `re.search(rf"(?<![\w.]){re.escape(base)}:{line}(?!\d)", cells[0])`. |
| (b″) token hidden in **prose** in the first cell (`\| see the guard at client.ts:626 (*) \|`) | row rewritten | **ACCEPTED** — the "first cell" need not be a statement cell at all | **yes**, same fix as (b′): require the first cell to *be* the token. |
| (c) category in cell 2 with an **empty reason** cell (`\| \`client.ts:626\` \| unreachable-by-construction \| \|`) | row rewritten | **ACCEPTED** — cells[2] is never inspected | minor; the register's own contract is "category **and reason**". A non-empty-length check on cells[2] is cheap. |
| (c′) `unreachable-by-constructionXYZ` in cell 2 | row rewritten | **ACCEPTED** — `startswith` accepts any suffix | minor; compare the cell against the keyword exactly (or against `keyword` followed by a separator). |
| (d) **one row for two** hidden statements, second row deleted | 29 rows | **ACCEPTED** | minor. The channel's docstring says "*EACH hidden statement is named on its own `basename:line` row*"; a merged row violates that and cannot be detected. Either enforce one token per cell[0], or drop the "its own row" wording. |
| (d′) the **same row twice** | 31 rows | **ACCEPTED** (no duplicate detection) | no |
| basename collision | — | *checked, not found*: none of the 16 hidden-statement basenames occurs twice under `mobile/src` (excluding `__tests__`), so the basename-only key costs nothing today | no |

Net: **three** of the attacks (b′, b″) are the same root cause (substring match); the
cheapest single fix is the exact-token regex above, plus a reconciliation step
"every `basename:line` token in the register must be in the hidden set" — that one
fix also kills (a) and (d) and makes the register self-checking. I would fix the
substring match and (optionally) the empty-reason check; the rest are cosmetic.

## 4. Task 1 result — falsified, and reasons that are wrong for a different reason

### 4.1 FALSIFIED — `SessionList.tsx:232`

Register row (L3037): `` | `SessionList.tsx:232` | unreachable-by-construction | same
shape as the retired `:184` row below, for the workspace confirmation | ``

`SessionList.tsx:232` is `if (deletingRef.current) return;` — the **first statement**
of `confirmDeleteWorkspace`. Its only caller is the workspace sheet's delete item,
`onPress: () => confirmDeleteWorkspace(menuWorkspace)` (`:467`).

**Construction (executed, not argued).** The sheet (`ActionMenu`) does not run an
item's action when the item is pressed: it *queues* it —

```tsx
onPress={() => {
  if (!action.onPress || pending.current) return;   // ActionMenu.tsx:111
  if (action.keepOpen) { action.onPress(); return; }
  pending.current = action.onPress;
  onClose();
  if (Platform.OS !== "ios") setTimeout(flush, 0);
}}
```

— and `flush()` (the `Modal`'s `onDismiss`) is what finally calls it. A probe that
captures that **press + flush pair while the sheet is usable** and then drives the
pair **twice** reaches `confirmDeleteWorkspace` on the second round with
`deletingRef.current === true`:

1. open the workspace sheet, capture `press = <delete item>.props.onPress` and
   `flush = <sheet Modal>.props.onDismiss`;
2. `press(); flush();` → confirmation #1;
3. confirm it (`danger` button + the dialog's `onDismiss`) → the workspace delete is
   in flight, `deletingRef.current = true`;
4. `press(); flush();` again — the wrapper's `pending` ref was cleared by the flush
   in step 2, so the press is not latched out, and the flush calls the stale
   `confirmDeleteWorkspace`, whose guard reads the **live ref** and returns.

Measured with a private coverage run of that single probe:

```
  231  hits=[8]
  232  hits=[2, 1]     <- the guard evaluated twice, the `return` ran once
  233  hits=[1]        <- exactly one dialog queued
```

So the statement is **reachable**, and the module suite's `[7, 0]` only means no
existing test drives the pair twice. **The row must become a covered statement** — a
test of the shape above. The probe's discriminating power is a *deduction*, not a
mutation run (removing the guard would mean editing `mobile/src`, which this task
forbids): with line 232's `return` removed, the round-2 flush would call
`confirmDeleteWorkspace` again and queue a second confirmation, so the
`visibleDialogs` assertion would be 1 instead of 0. The measured `233 hits=[1]`
(exactly one dialog queued across both rounds) is the behaviour that assertion rests
on.

Why the earlier falsification attempt failed: its own note says it "captured the
workspace-delete menu item's `onPress` …, then invoked the captured handler". That
invocation only *re-queues* the action (`pending.current = action.onPress;
onClose()`), so `confirmDeleteWorkspace` never ran and the measurement could not
distinguish "the wrapper latched" from "nobody flushed". The register's stated
reason ("the `ActionMenu` wrapper latches on dismiss") is therefore **not supported
by the experiment it cites**; and the analogy to `:184` was pointing at the right
class — `:184` fell to exactly this captured-press+flush technique in §Segment 27.

Cross-check that the *shape* is genuinely the one that falls: `ActionMenu`'s latch
is `pending.current`, and it is reset by `flush`, so a *pair* is not latched (only a
press that is never flushed is). That is the same "resettable latch" mechanism
§Segment 27 documented for `useAppDialog`'s `closing`.

### 4.2 Reason-quality defects (classification unchanged, but the text is wrong)

| row | defect | evidence |
|---|---|---|
| `SessionsScreen.tsx:178` | the register's reason is **non-responsive**: it explains that "*the parent's `submitRename` maps `renameTarget` → `renameTarget ? renameTarget.sessionId : undefined`, so the **session** is always defined here*", but line 178 guards a **name** (`if (!name) return;`). The honest reason is the one used elsewhere in the same doc: `renameSession`'s only caller (`submitRename`, `:196`) passes an already-`trim()`ed non-empty name (`:193-194`), which `RenameModal` had itself refused to empty (`RenameModal.tsx:73`). | `renameSession` has exactly one call site (grep: `SessionsScreen.tsx:196`); the guard is genuinely unreachable → claim confirmed, reason replaced |
| `PreviewModal.tsx:229` | the reason cites "**(101)**" for "the menu is cleared in the same render that makes `activeDownload` non-null". The clearing statement is **line 109** (`if (menu && (menu.stack !== previews \|\| activeDownload !== null)) setMenu(null);`) and `busy` is the prop set at **line 143**. Line 101 is `const top = previews.length - 1`. | source read; the claim itself is confirmed (§4.3) |
| `usePromptOutbox.ts:396` | "`pendingRecoveryRef` is set after `sendingRef` and both are cleared in one synchronous `finally`; the guard at 395 always returns first" is **incomplete**: `sendingRef.current = false` also happens at line **367**, in `sendMessage`'s `finally`. The missing link is the mutual exclusion — `sendMessage` refuses while the lane is held (`:251 if (sendingRef.current) throw new Error("send_busy")`) and `recoverPendingPrompt` refuses while a send holds it (`:395`) — so the two owners cannot overlap. | source read; claim confirmed with the sharper argument |
| `client.ts:626` | "`candidate.activate()` → `check()` two lines above … so the abort surfaces in the `catch`" only covers the **pre-activate** window (a `close()` from `onFeatures`, which is called at `:624`, before `activate()`). It does **not** cover the window *inside* `activate()` — and that window is real: `ConnectionGeneration.activate()` calls `drain()`, which runs buffered `deliver()` callbacks synchronously (`connectionGeneration.ts:53-70`). The claim is nevertheless true for a different reason: the delivered closure is deliberately asynchronous — `owner.deliver(() => { … void Promise.resolve().then(() => … this.callbacks.onEvent(…)) }, …)` (`client.ts:882-898`) — so consumer code cannot re-enter `close()` before line 626. | source read of both files |
| `useFileDownload.ts:413`, `:424` | registered `unreachable-by-construction`, but the parameter is **optional in the type** (`handle?: DownloadHandle`), so the type system *does* express the state and the arm is only impossible because both call sites pass a handle. Under the repo's own policy wording ("the types make the arm impossible"), the row fits the policy poorly. Either cite the call-site invariant in the row (as the current reason does) or use a category that means "no call site can produce it". | `useFileDownload.ts:486` signature + the register's own reason text |
| `files.ts:966` | the reason is **right by a longer route than it states**: `prunePreviewCache` is called at `:1070`, i.e. *before* the literal `cacheFile(info)` at `:1071`; the directory already exists because `verifiedCachedDownload` (`:1019`) → `cachedDownload` (`:980`) → `cacheFile` (`:910-912`, `if (!directory.exists) directory.create(…)`) creates it earlier in the same call. The same wording trap exists conversely in `useFileTree.ts:47` (§5, row 7). | source read |

### 4.3 Rows I verified and could **not** falsify (with the method used)

| row | claim | method | result |
|---|---|---|---|
| `files.ts:551` | `const [prepared] = …` is `undefined` only if `prepareFiles` returns a shorter array | `prepareFiles` (`:282-296`) maps `Promise.allSettled` over a **one-element** input and throws on any rejection; `noUncheckedIndexedAccess` is what puts `undefined` in the type | confirmed (type-requirement) |
| `files.ts:863` | `response` cannot still be null after the loop | `PREPARE_RPC_TIMEOUTS_MS = [10_000, 20_000] as const` (`:663`) is non-empty and the last iteration throws (`:856`) | confirmed |
| `files.ts:966` | `prunePreviewCache` never sees a missing directory | see §4.2 — `cacheFile` creates it via `cachedDownload` before `:1070` | confirmed (mechanism corrected) |
| `projection.ts:981` | the guarded index always holds a `kind === "message"` item | `findIndex` predicate at `:972-978` requires `item.kind === "message"`; the array is not mutated in between | confirmed (type-requirement) |
| `secureChannel.ts:20` | `equal` never gets two different lengths | module-private; the three call sites are `request.subarray(0,4)` vs `MAGIC` (4), `wire.subarray(0,4)` vs `MAGIC`, `wire.subarray(4,20)` vs `this.id` (16, and `wire.length < HEADER=28` is rejected first), `this.noise.rs` vs `keyBytes(...)` (32) | confirmed |
| `syncEngine.ts:799` | `lane.ops` never holds a falsy element | every write is a literal push (`:214`, `:303`) or a requeue of a slice (`:794`, `:840`); `:207` is a `filter` (preserves elements) | confirmed |
| `useSessionCatalog.ts:82` | `event.type !== "agent_end"` is always false there | `:75` admits only `agent_start`/`agent_end`; `:78` consumes `agent_start` with `return` | confirmed (redundant guard) |
| `useSessionCatalog.ts:203` | `readSessions` never sees a null client | the callback is not on the hook's public surface; the only call site is `:246`, inside a `do/while` whose condition re-checks `clientRef.current === client`, behind `refreshSessions`'s own guard at `:233-234` | confirmed for the present call sites; the reason is a caller enumeration, not a type fact |
| `useTimelineController.ts:480` | `olderEntries.length === 0` cannot be reached | the guard at `:470-475` requires `start + olderEntries.length === nextBefore` **and** `start < nextBefore` (otherwise `history_tail_backfill_cursor_invalid`), so the length is ≥ 1 | confirmed (arithmetic) |
| `useTimelineController.ts:532` | `stale_history_load` is unreachable there | the last `await` on the path is `readHistoryPage(…, isCurrent)`, whose own post-await checks reject first; nothing but synchronous statements follows | confirmed (JS run-to-completion; no interleaving point exists) |
| `useRemoteConnection.ts:417` | `!active` is still false | `:411 let active = true` … `:417` — no `await` between (the `drainRevokes()` call is `void`-ed, not awaited), so the cleanup cannot have run | confirmed |
| `usePromptOutbox.ts:396` | `pendingRecoveryRef.current` non-null ⟹ a prior `return` at `:395` | `:411` sets `sendingRef` before `:451` sets `pendingRecoveryRef`; both are cleared in one `.finally` (`:446-450`); the only other `sendingRef = false` (`:367`) is gated by `:251`'s mutual exclusion | confirmed (§4.2 sharpens the reason) |
| `client.ts:626` | see §4.2 | source of `ConnectionGeneration` + the deliver closure | confirmed (mechanism corrected) |
| `RemoteContext.tsx:588` | unreachable without a hand-built partial provider | `:586 useRemoteControls()` throws the same message at `:580` first; both contexts are module-private and only `RemoteProvider` populates them | confirmed |
| `MarkdownText.tsx:101` | no reference can reach the renderer with a non-`file` `targetType` | `parseFutureEmbed` matches `/^futureos-(file)$/` (`:509`, so `match[1] === "file"`), `parseFutureUrl` returns `null` because `isFutureReferenceType` is `void value; … return false` (`:608-615`) and every other construction site writes `targetType: "file"` | confirmed (product decision, deliberately re-enablable) |
| `DesktopsScreen.tsx:80` | the Save handler cannot exist with `renameTarget === null` | `Modal` renders children only when visible (`node_modules/react-native/Libraries/Modal/Modal.js:238/283`) and the modal is `visible={renameTarget !== null}`; the worker additionally *measured* `button("chat.save")` → `undefined`. A *stale* handler is not a counterexample: it closes over the `submitRename` of a render in which `renameTarget` was non-null | confirmed |
| `PreviewModal.tsx:229` | `busy` is false in every render that creates the handler | `:109` clears `menu` **during render** whenever `activeDownload !== null`, so a render cannot expose both a menu action and `busy === true`; an existing test already asserts no menu labels while busy (`previewMenuWhileBusy.test.tsx:73`); a *stale* handler keeps the old `busy === false` | confirmed |
| `syncEngine.ts:799`, `:931` | see above / call-site enumeration | `:931`'s six call sites were not all read; the two I read (`:627-629`, `:667-670`) are consistent | partially confirmed |
| `syncEngine.ts:595`, `:623`, `:667` | a lane that passed the outer check cannot fail the inner one | those three lines follow an `await replayInto/tailReconcile` whose callee re-checks below the last `await` | not falsified; the argument is a callee-exit enumeration I read only in outline |
| `client.ts:777`, `:1101` | both timer guards are cancelled before they can fire | `:775-779` / `:1099-1103` read; the exhaustive arming/clearing argument lives in `module-mobile.md`, *Segment 21 — `client.ts:777`/`:1101` proven by complete call-site enumeration* | not falsified (not re-measured) |
| `client.ts:1033` | `this.connection === connection` whenever `isLiveGeneration` holds | `:1033` read; `isLiveGeneration` (`:396`) requires the generation to be the active/candidate **live** one, and both are written in the same synchronous block on handoff | not falsified |
| `useRemoteConnection.ts:119` | the `useRef(async () => …)` initialiser body can never run | `:119` read; the only reader is `:455` inside `recoverLifecycle`, reachable only from native events, and the real implementation is installed in a mount effect | not falsified (React-lifecycle argument) |
| `useTimelineController.ts:945` | `probe` cannot run after `settle` | `settle` (`:927-935`) clears both timers and deletes the map entry before resolving; `probe` (`:943-945`) is only scheduled by those timers | not falsified (the poll scheduling was not read) |
| `useFileDownload.ts:413`, `:424` | both call sites pass a handle | call sites not re-read by me (§4.2 records the category issue) | not falsified |
| `sessionList`-class rows `DesktopsScreen.tsx:80`, `SessionsScreen.tsx:178`, `SessionList.tsx:232` | see above | `:232` **falsified** | — |

## 5. Desktop spot checks (Task 3) — 7 claims, 2 waiver ledgers

I did not attempt a full falsification of the desktop ledgers: my write scope has no
desktop test path, so a *construction* against a desktop claim is impossible for me
(I can only read code, read the existing tests, and read the coverage data). That is
a real limitation of this review, not a judgement that the rows are fine.

| # | group | claim (abridged) | verification | result |
|---|---|---|---|---|
| 1 | shell | boundary / off-by-one: width thresholds asserted **on both sides** — "at the 1280 threshold", "one pixel below 1280", plus the 768 edge | `components/layout/hooks/hooksEnv.test.tsx:261-273`: an `it.each` table with 1920/1280/1279/768/767/480 → `expect(harness.current.width).toBe(288|256|224)` | **confirmed** — real assertions on both sides |
| 2 | shell | property: `ActivityRailSelectionToolbar` (`selected`,`total`) matrix incl. `0 of 0` reading as "nothing selected" rather than "all selected" | `ActivityRailSelectionToolbar.test.tsx:61-68`: 0/0 → `checkbox.checked === false`, `label === "Select all"`; `:70-88` walks 0/4→1/4→4/4→0/4; `:90-99` scope-grows; `:101-105` delete disabled at 0/3 | **confirmed** — behaviour, not just render |
| 3 | settings | boundary: inclusive id boundaries 2/40 and 100/101; `maxTokens === contextWindow`; model-id 101 rejected | `CustomProviderDialog.validation.test.tsx:176-196` (1/41 rejected, 2/40 accepted, `toHaveBeenCalledExactlyOnceWith`), `:310-314` (`"m".repeat(101)` → "too long"), `:431` (max tokens > context window) | **confirmed** |
| 4 | agent | platform-cfg: the sandbox-tier copy forks on `isWindows`/`isLinux`, derived from `navigator.userAgent`; the earlier "N/A by construction" was **corrected** | `features/agent/Composer.platform.test.tsx` exists and pins `../../lib/platform` with **getters** (with a note that the first version of the test silently passed nothing); its header states the correction. I read the harness (L1-60) but not the assertion bodies | **partially confirmed** — mechanism and existence verified; per-platform assertions not read |
| 5 | agent | waiver retired: the file had **0 `attribution-artifact`** rows (the `useAgentThreadState.ts:201/232/265` row "was wrong") | `desktop/coverage/d-agent/coverage-final.json`: lines 201 `[1]`, 232 `[1]`, 265 `[1]` — covered; the doc records the retirement at L1043-1052 | **confirmed** |
| 6 | packages | waiver `parseFutureMarkdown.ts` 230/232/245 (`list`/`paragraph`/`table` arms) is `attribution-artifact`: the arms' bodies run although v8 reports 0 for the labels | `desktop/coverage/d-pkgs/coverage-final.json` for that file: no statement entry for 230/232/245, bodies 231 `[13]`, 233 `[44902]`, 237 `[44902]`, 243 `[44900]`, 246 `[14]`; `coverage-summary.json` reports the file at **226/226 lines (100 %)**, i.e. the ledger is a **branch** ledger, exactly as its preamble says | **confirmed** |
| 7 | panels | waivers `tabs.ts:160` (`return used.size + 1`) and `useFileTree.ts:47` (`if (!oldest) break`) are `unreachable-by-construction` | `tabs.ts:154-161`: the loop scans `1 … used.size+1` for the first integer not in `used`; by pigeonhole one exists, so `:160` is dead. `useFileTree.ts:44-49`: the only way `keys().next().value` is falsy is a key of `""`, and `:84 const state = root ? stateFor(root) : null` filters the empty string before it can enter the cache → `:47` dead | **both confirmed**, but #7's reason is *imprecise*: "`cache.size > CACHE_MAX_ROOTS` so the key is always defined" proves *definedness*, while the guard tests *falsiness*; the missing link is line 84. Same family of wording gap as `client.ts:626` and `files.ts:966` |

### 5.1 A desktop row I would put on the next worklist (not falsified, but the construction exists)

`desktop-settings.md` waiver: `desktop/src/features/skills/SkillsView.tsx:217`.

Measured (`desktop/coverage/coverage-final.json`): line 216 `[2]`, line 217 `[0]`,
line 218 `[2]`. The source is

```
214|  const runUninstall = useCallback(async (id: string) => {
215|    const skill = displayedInstalled.find(item => item.id === id);
216|    if (!skill)
217|      return;
218|    const index = displayedInstalled.indexOf(skill);
```

So (a) the row's own description of line 217 ("`const index = displayedInstalled.indexOf(skill)`")
is **off by one** — that statement is line 218; line 217 is the `return;` — and (b)
the category is questionable: the row's reason ("every call site passes an id taken
from `displayedInstalled`") is an `unreachable-by-construction` argument, not an
attribution artifact (nothing here is a brace/span next to a macro whose surroundings
a named test proves ran — although a named test *did* run the surroundings twice).

Whether it is really unreachable is the interesting part, and it is the same class
that has now fallen twice: the same document claims (its `concurrency` row) that the
**exit animation holds a row in the DOM while a background reload has already dropped
it from `displayedInstalled`**. If such a held-open row's Uninstall is pressed, the
handler receives an `id` that `displayedInstalled` no longer contains, and line 217
fires. That is a concrete construction; I did not execute it (no desktop test path
in my write scope). Recommended next step: assert on a held-open row after a
reload, or re-classify the row.

## 6. Findings summary

1. **`mobile/SessionList.tsx:232` must be covered**, not waived (§4.1). Constructed
   and measured; the register's reason for it is the same class its own §Segment 27
   falsified for `:184`.
2. **`verify.py`'s statement-waiver channel is over-permissive** (§3): exact-token
   matching + a reconciliation of register rows against the hidden set is the
   recommended fix.
3. **Six rows state a reason that is not the reason** (`SessionsScreen.tsx:178`
   non-responsive, `PreviewModal.tsx:229` stale line number, `usePromptOutbox.ts:396`,
   `client.ts:626`, `files.ts:966`, `useFileTree.ts:47`-style false-friend wording).
   Classifications are unaffected; the ledger is harder to audit than it needs to be.
4. **One row's category does not fit the repo's policy wording**
   (`useFileDownload.ts:413/424`: an optional parameter is not "the type makes it
   impossible").
5. **One desktop waiver needs a decision** (`SkillsView.tsx:217`: off-by-one
   description, questionable category, and a plausible held-open-row construction).
6. No evidence was found that the mobile register hides untested *reachable* code
   other than `:232`; the remaining 29 rows are either arithmetic/type facts, spans
   of synchronous code after the last `await`, or measured-cold.

## 7. Reproducing the falsification

Deleted after use (scratch); recreate under `mobile/coverage/` and run
`cd mobile; npx jest --testMatch "**/coverage/**/_revfe_reach.test.ts"`. The mock
block is copied verbatim from `mobile/src/screens/__tests__/SessionList.test.ts`
(`jest.mock` of `RemoteContext`, `react-i18next`, `lucide-react-native`,
`AsyncStorage`, `react-native-safe-area-context`); the body is:

```ts
test("SessionList.tsx:232 is reachable via captured press+flush while a delete is in flight", async () => {
  let resolveDelete!: () => void;
  mockRemote.deleteWorkspace.mockReturnValue(new Promise<void>(done => { resolveDelete = () => done(); }));

  act(() => button("sessions.workspaceActions:Project").props.onPress());      // open the workspace sheet
  const menu = tree.root.findByType(ActionMenu);
  const sheetModal = menu.findByType(Modal);
  const press = button("sessions.deleteWorkspace").props.onPress as () => void;  // CAPTURE
  const flush = sheetModal.props.onDismiss as () => void;                        // CAPTURE

  act(() => press());   act(() => flush());        // round 1: queue + flush -> confirmation #1
  await act(async () => { confirmAlert(); });      // delete in flight, deletingRef.current = true
  expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);

  act(() => press());   act(() => flush());        // round 2: same closures -> line 232's `return`

  expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
  expect(tree.root.findAllByType(Modal)
    .filter(n => n.props.visible && n.findAllByType(DialogSurface).length > 0)).toHaveLength(0);
  await act(async () => { resolveDelete(); await Promise.resolve(); });
});
```

Hit counts were read with
`npx jest --testMatch "**/coverage/**/_revfe_reach.test.ts" --coverage --collectCoverageFrom="src/screens/SessionList.tsx" --coverageReporters=json --coverageDirectory=coverage/_revfe_cov`
(the private report dir and `--collectCoverageFrom` are what keep this from touching
`mobile/coverage/coverage-final.json`; both shared files were hash-compared afterwards
and were unchanged).

Gate attacks: `python mobile/coverage/_revfe_attack.py` rebuilds
`mobile/coverage/_revfe_tmp/` (doc copy + JSON copies + `mobile/src` junction) and
runs the real gate for each mutation; `_revfe_probe.py` and `_revfe_counts.py`
reproduce §2.1/§2.2 and need no gate run.

## 8. Limits of this review (what I did not do)

* **Desktop**: I could not construct anything — no desktop test path in the declared
  write set — so desktop claims were verified by reading the test bodies, the source
  and the v8 reports. 7 claims out of hundreds and 2 waiver ledgers out of five were
  sampled; `desktop-agent.md` (157 KB) and `desktop-shell.md` (118 KB) were sampled,
  not read.
* **Mobile rows not falsified**: `client.ts:777/1033/1101`, `syncEngine.ts:595/623/667/931`,
  `useTimelineController.ts:945`, `useRemoteConnection.ts:119`, `useFileDownload.ts:413/424`,
  `useSessionCatalog.ts:203` — read, not constructed. Several are pure
  synchronous-span or type facts and I would not spend a construction on them; the
  timer/pump guards of `client.ts` are the ones where a construction would still be
  worth the effort.
* I did **not** run the module's full test suite or any full coverage job (resource
  policy); the jest runs above are one test each.
* I did **not** check whether the register's line numbers match the *current* source
  in every case — I did for the rows I read; `PreviewModal.tsx:229` already shows one
  stale internal reference, which suggests a mechanical line-number re-check of the
  whole register by its author is worth doing.
* Dimension evidence was not audited for *coverage* of the dimension vocabulary
  itself (`verify.py dimensions` needs `docs/testing/dimension-matrix.md`, outside my
  scope).

## 9. Next useful check

1. Cover `SessionList.tsx:232` with the probe above (add it to
   `mobile/src/screens/__tests__/SessionList.test.ts`), then re-run the module gate —
   the register should drop to 29 rows.
2. Fix `verify.py`'s `waived()` to exactly match `basename:line` tokens and to reject
   register rows that name nothing in the hidden set (§3).
3. Decide `SkillsView.tsx:217`: build the held-open-row construction, or re-classify
   and correct the line description (§5.1).
4. Re-check `client.ts:777/1101` with a construction (a timer armed, a terminal
   transition, then fire the timer) — the register's argument is an enumeration, and
   enumerations are what fell for `:184`/`:232`.

---

**Handoff tokens (for the supervisor's evidence check):** `lines-100-or-waived`,
`dimensions`, `weak-tests-fixed`.

---

# cov100 round 2 (REV-FE) — adversarial review of the 13 mobile waivers, revision `8449acf6`

*Reviewer task:* `todo_e5b82f038ada` (goal `goal_160bc247aa63`). The reviewer is
**not** the author of `docs/testing/module-mobile.md` §Segment 50 and did not
write, weaken or delete anything under `mobile/src`. **The only file written
inside the campaign worktree is this one.** Everything else lives in a scratch
checkout under `/tmp` (§R2.6).

*Everything above this line is the 2026-08-27 round-1 review at the same path and
is preserved deliberately — this campaign appends, it does not overwrite history
(the same convention `docs/testing/module-mobile.md` follows with its segments).

## R2.0 Run identity and method

| item | value |
|---|---|
| worktree | `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`), revision `8449acf6` + supervisor test snapshots |
| artefact reviewed | [docs/testing/module-mobile.md](module-mobile.md) §Segment 50 §3 — 13 residual `mobile/src` lines, each `unreachable-by-construction` with an enumerated-call-site or predicate-identity mechanism |
| metric under review | 口径 A: per-line LCOV `DA:<line>,0` from `mobile/coverage/lcov.info`, counted by `.future/cov100/covlines.py` |
| scratch checkout | `/tmp/rev-fe` — `git archive 8449acf6 \| tar -x` (53 MB), then `rsync` of the *current* `mobile/src` over it, `node_modules` symlinked from the campaign worktree. Probe tests were written **only** there |
| probes | 6 files, 27 tests, all green; sources + sha256 in §R2.6 |
| what a "verdict" means here | **SURVIVED** = I built the strongest counterexample I could and it failed to reach the line, with the attempt and its measurement recorded. **FELL** = I reached the line with an input the app can produce. **UNVERIFIABLE** = I could not build the input here; the note says what I would need |

Method, in order, for every line:

1. **Re-derive the claim from the source myself.** Not from the module record's
   summary: every mechanism in §R2.2 was re-read at the cited lines, and every
   call-site enumeration was re-run with my own `grep` over `mobile/src` (the
   record's tables were treated as claims, not evidence).
2. **Attempt the construction**, in the scratch checkout, against the real
   production source (unmodified copies) with the module's own harness patterns
   (`useFileDownload.test.ts`'s mock set, `useTimelineController.test.ts`'s
   harness, `client.test.ts`'s fixtures).
3. **Measure the line**, not just the behaviour: run the probe under
   `--coverage` and read `DA:<line>,<hits>` from `coverage-final.json`
   (`/tmp/rev-fe-notes/da_for_lines.py`). A probe that passes *and* leaves the
   target at 0 is the evidence that the waiver holds; a probe that pushes the
   target above 0 is a finding.
4. Where the only reachable construction is a **type or visibility violation**
   (a third union member, a `private` method), I built that too, and reported it
   as the *boundary* — that measures where the guarantee lives, it does not
   falsify the waiver, and it is labelled as such.

## R2.1 Ground truth: the residual set is exactly the 13 lines claimed

Recomputed independently from the unit's own report (copied out of
`mobile/coverage/` before any later run could wipe that directory):

```bash
cp mobile/coverage/lcov.info mobile/coverage/coverage-final.json /tmp/rev-fe-notes/
cd /Users/geilige/future-os/.worktrees/cov100-r2
python3 .future/cov100/covlines.py detail /tmp/rev-fe-notes/lcov.info --under src   # exit 0
python3 .future/cov100/covlines.py total  /tmp/rev-fe-notes/lcov.info --under src   # exit 0
```

```
src/components/MarkdownText.tsx: 139,140
src/features/chat/questionNav.ts: 67
src/features/chat/useMinimumVisible.ts: 11,12
src/features/chat/files/useChatFiles.ts: 201,208,305
src/remote/client.ts: 70,71
src/remote/useTimelineController.ts: 79,207
src/screens/SessionsScreen.tsx: 1166

covered=9846 measured=9859 uncovered=13 lines%=99.8681
```

Both tool invocations exited **0** (a nonzero exit would invalidate the number
whatever it printed — campaign measurement guard). The 13 lines are exactly the
13 in the module record: no line is missing from it, and it names no line the
report does not have. The per-line hit counters quoted in §R2.2 come from the
same `coverage-final.json`, read by my own script (a line's *statement* counters
**and** its *function* counters, since four of the 13 are function bodies).

## R2.2 Verdict per line

### R2.2.1 `src/components/MarkdownText.tsx:139` — **SURVIVED**

**Mechanism as claimed.** `inlineRuns` (171) peels every image whose
`remoteMarkdownImageUrl(src) || localFilePath(src)` is truthy into its own
`MarkdownImage` run before `renderInline` is ever handed the parent's children;
`renderInline`'s `"image"` case (136) re-evaluates the same `localFilePath`, so a
*local* src can never be standing at 138 with a non-null path, and the chip at
139 is unreachable. Measured by the unit: 171 = 417, peel = 24, inline arm
entered once (a `data:` src), 138 = 1, 139 = 0.

**My re-derivation.** All four call sites of `renderInline` are inside
`InlineContent` (289 list item, 326 heading, 376 paragraph, 449 table cell), and
`InlineContent` always calls `inlineRuns` first; the rest of the calls are
recursive on `node.children` of nodes that came out of `inlineRuns`, where a
wrapper is rebuilt as `{...node, children: run.nodes}` — i.e. already filtered.
So the whole question is whether *every inline context* really flows through
`inlineRuns`. That is what I attacked.

**Attempt.** A `file://` image placed in every inline context the parser can
produce — paragraph, **table cell**, link, emphasis, heading, list item,
blockquote, link-nested-in-emphasis, a relative path under a file-preview's
`imageBasePath`, and a bare paragraph image — asserting that a tappable chip
renders (i.e. the peel fired) in each.

```bash
cd /tmp/rev-fe/mobile
npx jest --runInBand --cacheDirectory /tmp/rev-fe/.jest-cache src/components/__tests__/revfe_markdown.test.ts
# 12 passed, 12 total
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/components/__tests__/revfe_markdown.test.ts p2
```

**Observed.** The peel fired in all ten local-image contexts, including the two
that a "the peel only covers the paragraph path" argument would have missed:

| line | hits in my probe | meaning |
|---|---|---|
| 171 | **27** | the peel predicate was evaluated 27 times |
| 172 | **11** | 11 images were peeled into their own run |
| 204 | **11** | 11 `MarkdownImage` runs rendered (the chip comes from there) |
| 136 / 138 | 1 / 1 | the single inline arrival (the `data:` case) takes the **null** arm |
| **139** | **0** | never reached |
| **140** | **0** | never reached |

**Interesting detail found while attacking this.** The "chip" a reader sees for a
`file://` image is not line 139's `fileChip` at all: it is `MarkdownImage`'s own
`<Text onPress=… style={styles.link}>` fallback (`MarkdownImage.tsx:52`, used when
there is no image loader) — that is why the existing test *"an image whose source
is a path on the desktop is a chip that opens it"* passes while 139/140 stay at
zero. The two chips look different on screen (`styles.fileChip` is a padded
rounded chip, `MarkdownImage`'s is an underlined accent-coloured label), so the
waiver is describing a branch that, if it ever became reachable, would render a
*second, differently styled* chip for the same input. That is an argument for the
branch being dead code rather than a bug.

**Verdict: SURVIVED.** Ten inline contexts; the peel holds in every one.

### R2.2.2 `src/components/MarkdownText.tsx:140` — **SURVIVED**

**Mechanism as claimed.** Same invariant: it is the `onPress` of the chip line
139 never renders.

**Attempt / observed.** Same probe and same coverage run as R2.2.1. Line 140
carries both a statement (`col=41`) and the arrow function (`col=35`); both are
**0** while 139's sibling arm is reached exactly once (the `data:` case) and
returns at 138.

**Verdict: SURVIVED** (one attempt, shared with 139; nothing separate is
constructible — 140 is only reachable through 139).

### R2.2.3 `src/features/chat/questionNav.ts:67` — **SURVIVED**

**Mechanism as claimed.** `QuestionRung` is a closed two-member union
(`{kind:"question"} | {kind:"replyTail"}`, line 32–34). Line 62 rejects unequal
kinds; 63 consumes both-`question` and returns; 65 consumes both-`replyTail` and
returns; with equal kinds over two members both are already handled, so no value
of the declared type reaches 67. Measured: 62 = 140/60, 63 = 80, 65 = 43, 67 = 0.

**My re-derivation.** I re-read the union and grepped every construction of a
rung in the app: the only producers are `questionNav.ts:137/138` (`{kind:"replyTail"}`
and `{kind:"question"}`) and the rungs consumed by `useQuestionNav.ts` — no
producer takes `kind` from data, a parse, or a persisted payload. So the union is
honest at runtime, not only in the type.

**Attempt A (the app's input space).** Drive every exported entry point
(`sameRung`, `rungIndex`, `rungPosition`, `questionIndices`) with both members,
equal and unequal payloads, in both argument orders.

**Attempt B (the boundary).** Push a *third* kind through the exported
`sameRung`, which TypeScript forbids (`as unknown as QuestionRung`).

```bash
npx jest --runInBand --cacheDirectory /tmp/rev-fe/.jest-cache \
  src/features/chat/__tests__/revfe_questionnav.test.ts -t "A: every value"
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/features/chat/__tests__/revfe_questionnav.test.ts p3b_a "A: every value"
npx jest ... -t "B: a third kind"
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/features/chat/__tests__/revfe_questionnav.test.ts p3b_b "B: a third kind"
```

**Observed.**

| run | 61 (fn) | 62 | 63 | 65 | **67** |
|---|---|---|---|---|---|
| A — app-legal values | 11 | 11 / 4 | 7 | 3 | **0** |
| B — third kind (`as unknown`) | 1 | 1 / 0 | 1 | 1 | **1** |

**Verdict: SURVIVED, with the boundary named.** The guarantee is the *type*, and
it is enforced (`tsc --noEmit` in the module's checks): a value outside the union
does reach 67 (measured), but no such value exists in this repository. The check
at 67 is therefore a defensive fall-through — correct to keep, correctly
classified. Worth knowing for whoever adds a third rung kind: this line becomes
live and load-bearing the moment they do.

### R2.2.4 `src/features/chat/useMinimumVisible.ts:11` — **SURVIVED**

**Mechanism as claimed.** Single writer + paired clear: `timerRef` is written at
27 (arming, in this same effect) and 35–36 (this effect's own cleanup), and React
runs the cleanup immediately before every re-run, so at line 10 the ref is null.
Measured by the unit: 10 = 91, 11/12 = 0, cleanup clear 35/36 = 8.

**My re-derivation.** The invariant is a property of React's effect protocol, not
of the types: the only non-null write (27) happens in a run that also *returns*
the cleanup, and React runs that cleanup before the next body of the same effect,
or on unmount. Two corollaries I checked: (a) a run that early-returns at 22 or 24
never arms a timer, so it cannot leave a non-null ref behind; (b) a fresh mount
gets a fresh ref.

**Attempt.** Every transition shape that can re-run the effect while a timer could
be pending — five toggles of `active` alone, a notice-`key` swap while a hide
timer is armed, a changed `minimumMs` while armed, an elapse followed by a re-arm,
and an unmount while armed (28 effect runs in total).

```bash
npx jest --runInBand --cacheDirectory /tmp/rev-fe/.jest-cache src/features/chat/__tests__/revfe_minvisible.test.ts
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/features/chat/__tests__/revfe_minvisible.test.ts p3a
```

**Observed.** 9 = 28, **10 = 28**, **11 = 0**, **12 = 0**, 27 = 11 (the timer body
fired 3 times), 34 = 11, 35 = 11 / 8 (the cleanup cancelled an armed timer 8
times), 36 = 11. Every one of the 28 re-runs found a null ref — and the *cleanup*
is what did the clearing.

**Observation (not a finding, no rework).** Lines 10–13 are redundant
defensive code: the cleanup at 34–37 already cancels and nulls. It is harmless
and arguably intentional (it makes the effect body robust if the arming ever
moves), but it is also why the line can never be covered by a test: any test that
made the ref non-null at line 10 would first have to defeat the protocol above.

**Verdict: SURVIVED.**

### R2.2.5 `src/features/chat/useMinimumVisible.ts:12` — **SURVIVED**

**Mechanism as claimed.** The same single-writer/paired-cleanup invariant; 12 is
the nulling half of the same `if` whose `clearTimeout` is at 11 (and which the
cleanup at 35/36 performs).

**Attempt / observed.** Same probe and run as R2.2.4: 11 = **0**, 12 = **0**
across 28 effect runs, while 35/36 execute 11 times each (and the arming at 27
eleven times). There is no input that reaches either line without first reaching
the other, so this line's verdict cannot differ from R2.2.4's.

**Verdict: SURVIVED.**

### R2.2.6 `src/features/chat/files/useChatFiles.ts:201` — **SURVIVED** (this was my strongest FELL candidate)

**Mechanism as claimed.** `handle.visible` is already true whenever this
transport reports: `openAttachment` calls `showDownload(handle, …)` at 184 and
only then `remote.downloadAttachment` at 191, with **no `await` between them**,
and `showDownload` sets `handle.visible = true` synchronously. Measured by the
unit: 200 evaluated 7× with its `else` at 0, and a large-file/metered probe still
reported 200's true arm with 201 = 0. The author named this line one of the two
weakest and asked for it to be attacked first.

**My first hypothesis — and why it was wrong.** I read 184 as living *inside*
`if (warning)`, which would have opened a hole: for any size below
`DOWNLOAD_CONFIRM_BYTES` (1 MiB, `downloadPolicy.ts:10`) `downloadWarning`
resolves `null` without consulting the network, 184 would not run, and the
transfer would still be invisible when the transport reports. I wrote a probe for
exactly that (5 KiB, WIFI, cache miss, transport reporting progress), and the
probe *refuted me*: the dialog was already on screen before the report. Reading
the file again with the columns visible, 184 sits at indentation 10 — a child of
`if (!file)` (167), **not** of `if (warning)` (170, body at 12). The comment
block at 181–183 sits between them and made the `nl`-based read ambiguous. The
waiver is *stronger* than its stated reason: `showDownload` runs for **every**
cache miss, warning or not. (Recorded here because the grouped mechanism in the
module record, read quickly, suggests the warning-gated shape — the next reviewer
should not have to rediscover this the way I did.)

**Attempt (four constructions, all against the real source).**

| # | construction | why it could have worked |
|---|---|---|
| A | 5 KiB cache miss (no warning), transport reports progress | the path the author never drove |
| B | the same, reporting **waiting** (`withTransferRetry` → `onWaiting`, `remote/files.ts:749`) | attacks 208's sibling under the same input |
| C | cache hit (`cachedAttachment` returns the file) | the transport is never called, so no report exists |
| D | cancel transfer A while it is parked in `prepareAttachment`, start B, let A resume | the only way to reach 184 with `activeDownloadRef` held by some *other* handle, which would make 184's early return leave A invisible |

```bash
npx jest --runInBand --cacheDirectory /tmp/rev-fe/.jest-cache src/features/chat/__tests__/revfe_usechatfiles.test.ts
# 4 passed, 4 total
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/features/chat/__tests__/revfe_usechatfiles.test.ts p1
```

**Observed.**

| line | hits in my probe | meaning |
|---|---|---|
| 184 | **3** | revealed before the transport in A/B/D |
| 191 | **3** | the transport was really called |
| **200** | **1** | the *true* arm ran: the handle was already visible at report time |
| **201** | **0** | not reached by any of the four |
| 206 | 1 | (waiting callback, same shape) |
| **207** | **1** | the true arm again |
| **208** | **0** | not reached |

Attempt A/B asserted the discriminating state *before* the report —
`activeDownload` is `{phase:"downloading", completedBytes:0, totalBytes:5120}`
(i.e. line 184 produced the dialog, with no question asked: `confirmDownload` was
never called) — and after it, the patch from the callback landed. Attempt D
asserted the transport was called **once** (for B only): A's own aborted check
(line 169) stops it before 184, so a cancelled transfer cannot leave an invisible
handle in flight.

**Verdict: SURVIVED.** The report always meets `visible === true`: the only writer
of `visible = false` is `hideDownload` (`useTransferOperation.ts:129`), whose only
reachable entry point in this flow is `confirmDownloadFor` at 171 — *before* 184,
which then sets it back. I could not construct an input that reorders them, and I
tried the two most plausible ones (the no-warning path; a stolen active slot).

### R2.2.7 `src/features/chat/files/useChatFiles.ts:208` — **SURVIVED**

**Mechanism as claimed.** The waiting callback's sibling of 201: same invariant,
same flow (207 evaluated 8× with its `else` at 0).

**Attempt / observed.** Constructions B and D above, plus A. The waiting callback
is reached through a *real* production event — `withTransferRetry` calls
`onWaiting` before a retry delay — and my probe still measured **207 = 1,
208 = 0**: the transfer was already visible at 184, so the report took the
`updateDownload` arm (`phase: "waiting_network"` was asserted to land).

**Verdict: SURVIVED.**

### R2.2.8 `src/features/chat/files/useChatFiles.ts:305` — **SURVIVED**

**Mechanism as claimed.** `fetchDownload` is private — *not* in the hook's
returned API — and both of its call sites pass a handle: 362 with `openOrShare`'s
(constructed at 331, guarded at 332) and 546 with `openLocalPath`'s (guarded at
511). The arm needs `handle === undefined`, and nothing supplies it. The contrast
with its neighbour: "a handle present but invisible" *is* constructible through
the public `existingHandle` parameter (giving 303 one hit), "no handle at all" is
not. Measured: 296 = 22, 304/305 = 0.

**My re-derivation.** Re-grepped `fetchDownload` over all of `mobile/src`: the
definition at 264, references at 362, 546, and the dependency arrays at 438/581 —
and it is **absent from the hook's return object** (604–623), which I read in
full. So the claim "private" is literally true, and the only way to hand it an
undefined handle would be a direct call from inside the module, of which there is
none.

**Attempt.** The three constructions that could in principle leave a caller
handle-less:
(a) `openOrShare` invoked **without** the optional `existingHandle` — it then
    calls `beginDownload` (331); if that returns `null` (another transfer is
    active, 42) the function returns at 332 with a toast and never reaches
    `fetchDownload`. Measured behaviourally in my probe D (the "stolen slot" run
    drives exactly the `beginDownload → null` guard through `openAttachment`).
(b) `openLocalPath` without a handle — 510 `beginDownload`, guarded at 511.
(c) reaching `fetchDownload` from outside — impossible: it is not exported, and
    importing the module's internals is not something the app does.

```bash
# same probe file as R2.2.6; the guard paths are exercised by attempt D
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/features/chat/__tests__/revfe_usechatfiles.test.ts p1
grep -rn "fetchDownload" mobile/src --include='*.ts' --include='*.tsx' | grep -v __tests__
```

**Observed.** In the probe run: 362/546 = 0 (those entry points are not driven by
it), and in the unit's own suite 296 = 22 with **305 = 0** — the guard at 296 is
evaluated on every transport report while its `else` never runs. My `grep` list
is exactly the four references above; `useChatFiles.ts:604–623` (the returned
object) contains `openAttachment`, `openFileLink`, `openLinkedFile`,
`downloadOriginal`, `openOrShare`, … and **no** `fetchDownload`.

**Verdict: SURVIVED.** One nuance for the record: the arm is one refactor away
from being live (exposing `fetchDownload`, or dropping the 332/511 guards), which
is presumably why the author contrasted it with 303 explicitly.

### R2.2.9 `src/remote/client.ts:70` — **SURVIVED**

**Mechanism as claimed.** `failureSupportCode(category)` is called only from
`recordFailure`, whose eight call sites pass literals from the closed set
`{credential_revoked, generation_unhealthy, service_authorization, network,
credential_expired, protocol}`; none passes `"local"`. Measured: 69 = 16, 70/71 = 0.

**My re-derivation (enumeration, not trust).**

* `failureSupportCode` is module-private (`client.ts:63`, no `export`) — grepped
  the whole of `mobile/src`: the only references are its definition and three
  call sites (265, 284, **296**). Line 296 is the one the record's phrasing
  ("called only from `recordFailure`") skips over: it is called from
  `finishFailureEpisode` with `episode.category` — but `failureEpisode` is an
  instance field written only by `recordFailure` (251–260), never persisted and
  never read from outside, so the same closed set governs it.
* `recordFailure` is a `private` method (249). Its eight call sites: 677
  `"credential_revoked"`, **688–692 a ternary between two literals**
  (`"generation_unhealthy"` / `"service_authorization"`), 725
  `"service_authorization"`, 756 `"generation_unhealthy"`, 780 `"network"`, 1074
  `"credential_expired"`, 1077 `"service_authorization"`, 1081 `"protocol"`. In
  1074/1077/1081 the *detail* is the server-supplied `code`, while the
  **category** is still a literal — so no server payload can choose it.
* The six categories each have an explicit arm at 64–69, and 68 also catches any
  detail containing `"protocol"`; 70 and 71 are what is left.

**Attempt.** The only construction is a call to the private method itself
(`recordFailure("local", …)`), which no production input can produce. I built it
in the scratch checkout, on top of the module's own `client.test.ts` fixtures, and
asserted the *behaviour* it drives: the failure-episode log line carries
`supportCode: "LC001"` for `"local"` and `"LC999"` for a category no call site
passes.

```bash
npx jest --runInBand --cacheDirectory /tmp/rev-fe/.jest-cache \
  src/remote/__tests__/revfe_client.test.ts -t "Z: the LC001"
# 1 passed, 142 skipped
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/remote/__tests__/revfe_client.test.ts p3d "Z: the LC001"
```

**Observed.** With the private-method call: **70 = 3**, **71 = 1** (and the
`console.warn` assertions passed — the codes are what the arms produce). With the
app's own entry points, the same lines are 0 in the unit's 163-suite run, where
every *other* arm (64–69) is measurably exercised.

**Verdict: SURVIVED, with the boundary named.** The guarantee is
`private` + the literal set at the eight call sites — a *visibility and
call-site* invariant, not a type-level one. Two lines, and it is a one-token
change away from being live (any new call site with a new category lands on
LC999 by design), so it is correctly classified rather than incorrectly waived.

### R2.2.10 `src/remote/client.ts:71` — **SURVIVED**

**Mechanism as claimed.** The same closed set from the other side: the `LC999`
fallback needs a category no call site produces; with the six arms above it, it
is unreachable *today*.

**Attempt / observed.** Same run as R2.2.9: the cast call produced **71 = 1**,
proving the fallback works when reached; the app's call sites cannot reach it
(literal enumeration above, and 0 across the unit's whole suite while 64–69 are
all nonzero: 95, 51, 43, 29, 21, 16).

**Verdict: SURVIVED.** This is the one waiver whose mechanism is explicitly about
*today's* call set rather than a construction ("the type cannot exclude a future
one — the invariant is about the call sites"). I agree with that framing and with
the classification: it is dead code that exists to label an unclassified failure,
not an unreachable branch hiding a bug.

### R2.2.11 `src/remote/useTimelineController.ts:79` — **SURVIVED** (mechanism corrected)

**Mechanism as claimed.** `hydrateAttachmentsRef`'s initializer body is dead: its
only reader is line 170 (from a client event), and the effect at 632 overwrites
the ref in the same commit that mounts the hook — "before the socket those events
arrive on has even opened". Measured: 78 created 838×, the initializer body 0.

**My re-derivation — the stated reason is not the governing one.** The hook does
not own or open a socket: it takes `clientRef` from its caller, and the *client
object* is constructed inside `useDesktopConnection`'s async effect
(`useDesktopConnection.ts:230–260`, `new RemoteClient(...)`). `useTimelineController`
is called at `useDesktopRuntime.ts:144` and `useDesktopConnection` at `:204`, so
the connection hook's effects — the ones that build the client and register its
`onEvent` (269) — are registered **after** this hook's effects, including 632. Two
consequences: (a) `clientRef.current` is `null` until a later effect has run, so
no event can arrive before the ref has been replaced, regardless of whether a
socket is open; (b) that ordering, not the socket's connection state, is the
invariant. It is a real guarantee, but it is a *hook-order* guarantee, and it is
one refactor away from moving (e.g. hoisting the client into a provider that
outlives this hook would put the initializer back in play).

**Attempt T1 (the app's real ordering).** Mount the controller, establish a
timeline, then deliver the event that reads the ref — a `user_message` for the
selected session — through `handleEvent`, and assert that a durable history read
is issued (only the *replaced* ref can issue one; the initializer does nothing).

**Attempt T2 (the boundary).** Read `hydrateAttachmentsRef.current` and call
`handleEvent` **during the first render**, i.e. before the mount's passive
effects flush — the only ordering in which the initializer is still installed.

```bash
npx jest --runInBand --cacheDirectory /tmp/rev-fe/.jest-cache \
  src/remote/__tests__/revfe_timeline.test.ts -t "T1:|T3:"      # app ordering
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/remote/__tests__/revfe_timeline.test.ts p3c_not2 "T1:|T3:"
npx jest ... src/remote/__tests__/revfe_timeline.test.ts        # includes the boundary
sh /tmp/rev-fe-notes/revfe_covprobe.sh src/remote/__tests__/revfe_timeline.test.ts p3c_boundary
```

**Observed.**

| run | 78 | **79 (fn / stmt)** | 170 | 633 (replacement) | 206 (created) | 207 (fn col 4) |
|---|---|---|---|---|---|---|
| T1 + T3 (app ordering) | 7 | **0 / 0** | 1 | 3 | 7 | 2 |
| all three (boundary included) | 8 | **1 / 1** | 2 | 3 | 8 | 2 |

In the boundary run, T2 also asserted that `hydrateAttachmentsRef.current` was a
*different* function object after the flush than the one it captured at render
time, and that no history request was issued by the render-time call — i.e. the
captured function really was the do-nothing initializer.

**Verdict: SURVIVED** (unreachable in the app's composition), **with a
mechanism correction**: the record's reason should name the client's construction
and the hook order (`useDesktopRuntime.ts:144` vs `:204`), not the socket's state.
The practical risk the correction names — hoisting the client into a longer-lived
provider — is worth a sentence in the record if that refactor is ever proposed.

### R2.2.12 `src/remote/useTimelineController.ts:207` — **SURVIVED**

**Mechanism as claimed.** `loadHistory`'s `laneIsCurrent = () => true` default is
an arity convenience: the function is handed to the engine as `requestHistory`
(552), the engine's only call site (`syncEngine.ts:598`) always passes the
predicate, and no in-repo caller omits the argument. Measured: created 838×, the
default-arg body 0.

**My re-derivation.** `grep -rn "loadHistory\|requestHistory" mobile/src`:
`syncEngine.ts:85` declares `requestHistory(sessionId, isCurrent)` with **both
parameters required**, `syncEngine.ts:598` is the only call, and `loadHistory`
appears at 206 (definition), 552 (handed over) and 630 (dependency array) only —
it is **not** in the hook's returned API, so no screen can call it with one
argument either.

**Attempt T3.** Drive a real history load through the engine (`reconcileSession`
→ the engine's lane → `requestHistory`) and assert the page landed in the
timeline; the coverage reader then shows the function was called while its
default was not.

**Observed.** 206 = 7 (created), `fn col=4` = **2** (loadHistory itself really
ran, twice), and both counters for the default arrow — `fn col=61` and
`stmt col=67` at 207 — were **0** in both runs (T1/T3 and all three). The probe's
assertions (`historyCalls() > 0`, `timeline.items.length > 0`) passed, so the
call path that would have had to supply a default is the one that ran.

**Verdict: SURVIVED.** The default argument is unreachable *by arity*: the only
producer of calls is a two-parameter interface.

### R2.2.13 `src/screens/SessionsScreen.tsx:1166` — **SURVIVED**

**Mechanism as claimed.** Dead prop wiring: the only page that calls
`onOpenConversation` is `TasksSettingsPage`, whose only route is
`SettingsScreen`'s `case "tasks"` — reached only from `renderDeviceHome()`, i.e.
only when `deviceMode === true`, i.e. only when a `deviceId` prop was passed
(133). This screen renders `<SettingsScreen>` with **no** `deviceId` (1160–1167),
so `renderPhoneHome()` renders instead and the arrow's body at 1166 cannot run.
The record also records it as a finding (a latent UX gap: the run's-conversation
affordance is missing from the session list's own settings).

**My re-derivation — full enumeration of every way a "tasks" route could appear:**
`TITLED_SETTINGS_PAGES`/`SettingsRoute` declare `"tasks"` as a route name (36–50)
and the switch handles it at 234 **before** the `default:` at 283 — so the switch
itself imposes no device-mode condition; the question is only who can push it.

| route source | line | can it run without `deviceId`? |
|---|---|---|
| initial `routes` state | 99 `useState([{name:"home"}])` | no — only `"home"` |
| `push({name:"tasks"})` | **377 — the only one in the repository**, inside `renderDeviceHome()` (316, reached from `default:` at 284 only when `deviceMode`) | no |
| every other `setRoutes` | 101 (the same `push`), 112 (`slice(0,-1)`) | no |
| imperative handle | 25/116 `SettingsScreenHandle = { goBack }` — no route entry point | no |
| deep links / navigation params | `SessionsScreen` seeds `settingsOpen` state only; no level parameter exists | no |

**Corroborating, mechanical checks.** `SettingsScreen.test.ts:325` already
asserts the phone home has **no** tasks link (`link("desktopSettings.tasks")` is
`undefined`), and the same file reaches the tasks page only in device mode
(686–700). The live twin of this wiring — `DesktopsScreen.tsx:235–241`, which
passes `deviceId={editing}` and the same `onOpenConversation` — is the code path
that is measured; the copy at 1166 is the one that is not.

**Attempt.** I could not mount `SessionsScreen`'s settings stack in the scratch
checkout without rebuilding its full mock set (see §R2.5), so this verdict rests
on the enumeration above plus the unit's own whole-suite measurement (1166 = 0
across 163 suites, including the suites that open this modal) rather than on a
probe of mine. I am flagging that explicitly rather than dressing it up.

**Verdict: SURVIVED** (routing enumeration complete; no probe of my own).

## R2.3 Summary

| # | line | claimed mechanism | my attempt | decisive observation | verdict |
|---|---|---|---|---|---|
| 1 | `MarkdownText.tsx:139` | predicate evaluated twice; peel removes every local image | `file://` image in 10 inline contexts (incl. table cell, link, nested emphasis) | 171=27, 172=11, 204=11, **139=0** | **SURVIVED** |
| 2 | `MarkdownText.tsx:140` | the `onPress` of the chip that 139 never renders | same run | **140 fn+stmt = 0** | **SURVIVED** |
| 3 | `questionNav.ts:67` | closed two-member union; 62/63/65 consume both | every app-legal value; then a third kind | app-legal **67=0**; third kind **67=1** | **SURVIVED** (type boundary) |
| 4 | `useMinimumVisible.ts:11` | single writer + paired cleanup | 5 transition shapes, 28 effect runs | **11=0**, cleanup clear 35=11/8 | **SURVIVED** |
| 5 | `useMinimumVisible.ts:12` | as above | same run | **12=0** | **SURVIVED** |
| 6 | `useChatFiles.ts:201` | `visible` set at 184 before 191, no await between | 4 constructions (no-warning path, waiting, cache hit, stolen slot) | 184=3, **200=1, 201=0** | **SURVIVED** |
| 7 | `useChatFiles.ts:208` | same invariant | waiting-callback variants | **207=1, 208=0** | **SURVIVED** |
| 8 | `useChatFiles.ts:305` | private function, both call sites guarded | grep + guard paths; `beginDownload → null` route | 296=22, **305=0**; not in the returned API | **SURVIVED** |
| 9 | `client.ts:70` | closed literal set at 8 call sites | enumeration incl. line 296; private-method cast | 64–69 all >0; **70=0** in-app, 70=3 via the cast | **SURVIVED** (visibility boundary) |
| 10 | `client.ts:71` | as above (the fallback) | same run | **71=0** in-app, 71=1 via the cast | **SURVIVED** |
| 11 | `useTimelineController.ts:79` | ref replaced in the mounting commit | app ordering (T1) + pre-effect boundary (T2) | T1 **79=0**; T2 **79=1** | **SURVIVED** (mechanism corrected) |
| 12 | `useTimelineController.ts:207` | arity: only caller passes the predicate | engine-driven history load | loadHistory fn=2, **default fn/stmt = 0** | **SURVIVED** |
| 13 | `SessionsScreen.tsx:1166` | route reachable only in device mode | full enumeration of route sources | only `push("tasks")` is at 377 in `renderDeviceHome` | **SURVIVED** (no probe) |

**Falls: 0.** No waiver needs rework, and I found no line that a test in the
module could cover as written.

## R2.4 Corrections and observations (no rework implied)

1. **`useChatFiles.ts:184` is not warning-gated** (§R2.2.6). The record's grouped
   mechanism reads as though `showDownload` were part of the confirmation path;
   it is a child of `if (!file)` and runs for every cache miss. The waiver is
   correct — this only makes it stronger — but the next reviewer will otherwise
   re-run my mistake.
2. **`useTimelineController.ts:79`'s stated reason is not its governing one**
   (§R2.2.11): the client is constructed in an effect registered after this
   hook's effects, so the initializer is unreachable because of *hook order*, not
   because a socket has not opened. A refactor that hoists the client into a
   longer-lived provider would put the initializer back in play.
3. **`useMinimumVisible.ts:10–13` is redundant** (§R2.2.4): the cleanup at 34–37
   already cancels and nulls. Harmless, but it is the reason the line cannot be
   covered — and if the arming ever moves out of this effect, the line becomes
   both reachable and load-bearing.
4. **Three of the 13 are "type/visibility-closed" rather than
   "construction-closed"** (`questionNav.ts:67`, `client.ts:70/71`) — measured
   boundaries: each reaches 1 in the scratch checkout the moment the declared
   type or the `private` modifier is violated. That is the correct reading of
   `unreachable-by-construction` for them, and it is worth saying out loud
   because it tells a future author exactly which one-token change revives them.
5. **`docs/testing/waiver-ledger.md` still says "(see its own §waiver ledger)"
   for module-mobile** (already noted by the unit, §Segment 50 §6.5). It is
   outside both my write scope and the unit's; the supervisor should point it at
   §Segment 50 §3.

## R2.5 What I could not check

* **The whole mobile suite in my own checkout.** I ran only my 6 probe files
  there (27 tests). The whole-suite numbers I quote (163 suites / 3246 tests, the
  per-line counters of the unit's run) come from the unit's own report, which I
  verified *independently* for the thing that matters — the residual set and the
  totals, via `covlines.py detail|total` (§R2.1). I did not re-run 163 suites
  under coverage.
* **`SessionsScreen.tsx:1166` end-to-end.** My verdict is a route enumeration
  plus the existing `SettingsScreen.test.ts:325` assertion; I did not mount
  `SessionsScreen`'s settings modal in the scratch checkout, which would need its
  full remote/navigation mock set.
* **iOS-specific ordering.** All my probes ran with `Platform.OS = "android"`.
  The `201`/`208` invariant is platform-independent (`showDownload`/`hideDownload`
  write `visible` without branching on platform; only the *promise* inside
  `hideDownload` is iOS-conditional), but I did not measure the iOS branch of
  `handoffDownloadModal`/`flushPendingDownloadModal`.
* **The other units' waivers** (desktop, `src-tauri`, the Rust crates) — outside
  this todo; the round-1 desktop review above is a different campaign's artifact.
* **Whether each *count* quoted by the module record is reproducible** — I read
  the unit's `coverage-final.json` for the 13 target lines and reproduced the
  record's numbers where I cite them (139/140, 171/172, 62/63/65/67, 11/12,
  200/201, 207/208, 296/305, 70/71, 79, 206/207, 1166), but I did not audit every
  counter in the surrounding prose.
* **Production reachability of the module's own gates** (capability flags,
  `desktop_settings_v1`, etc.) — not part of the waiver mechanisms.

## R2.6 Reproduction

The scratch tree is not part of the repository (and per the campaign rules,
`/tmp` is not a write target of the campaign). To rebuild it exactly:

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2
mkdir -p /tmp/rev-fe && git archive 8449acf6 | tar -x -C /tmp/rev-fe
rsync -a --delete mobile/src/ /tmp/rev-fe/mobile/src/           # current mobile/src
cp mobile/src/version.generated.ts /tmp/rev-fe/mobile/src/
ln -sfn "$PWD/node_modules"        /tmp/rev-fe/node_modules      # root hoist, needed by jest.config.js
ln -sfn "$PWD/mobile/node_modules" /tmp/rev-fe/mobile/node_modules
```

**Why `git archive` and not `git worktree add`.** This checkout is shared with
eight peer workers and PLAN §3 forbids state-changing git here; `worktree add`
writes into the shared `.git/worktrees/` and leaves a `git worktree list` entry
peers and the supervisor have to reason about. `git archive 8449acf6` yields the
same tree with no mutation of the shared repository, which is what the isolation
was for. No `add`, `commit`, `checkout`, `reset`, `stash`, `switch`, `worktree`,
`clean` or `fetch` was run in the campaign worktree.

Probe files (scratch only; sha256 of the exact bytes reviewed):

| file | sha256 | tests |
|---|---|---|
| `/tmp/rev-fe/mobile/src/features/chat/__tests__/revfe_usechatfiles.test.ts` | `4972d4e81b808ab4f71df500f123964eebd3ecab94c4e6ecd764b3b5b35a169f` | 4 (A–D) |
| `/tmp/rev-fe/mobile/src/components/__tests__/revfe_markdown.test.ts` | `4375cabe2c6f7431e786d3cdcc620b0a9d97432739330d48128c4f96c690ef64` | 12 |
| `/tmp/rev-fe/mobile/src/features/chat/__tests__/revfe_minvisible.test.ts` | `1f60990ef8ef137700648d2881668a7183d432c15a8c8e30531263d67eeb4337` | 5 |
| `/tmp/rev-fe/mobile/src/features/chat/__tests__/revfe_questionnav.test.ts` | `e3bae8211199828cb406f9966f19f6ff6a3e1fcad762cd5e9cd46dded6c682b7` | 2 (A, B) |
| `/tmp/rev-fe/mobile/src/remote/__tests__/revfe_timeline.test.ts` | `df2fd711f9629e5ac14ab507a8ed4c393b52c02d5ec47454bb0440b9b84fee3c` | 3 (T1–T3) |
| `/tmp/rev-fe/mobile/src/remote/__tests__/revfe_client.test.ts` | `dce8bc26e84023c2998c13d21b84abff86d47f3650af1825380b17e7bffd8581` | 1 (Z) + the module's own 142 |

The mock scaffolding is copied from the module's existing files
(`useFileDownload.test.ts`'s `expo-*`/`remote/files`/`remote/files` mock set,
`useTimelineController.test.ts`'s harness, `client.test.ts`'s fixtures); the
probe-specific part of each file is the test bodies quoted in §R2.2.

Commands, in full:

```bash
# line-hit measurement and behaviour, from the scratch tree
cd /tmp/rev-fe/mobile
# behaviour
npx jest --runInBand --cacheDirectory /tmp/rev-fe/.jest-cache <probe-file> [ -t "<name>" ]
# line-hit measurement (writes a private coverage dir, prints DA for the target lines)
sh /tmp/rev-fe-notes/revfe_covprobe.sh <probe-file> <out-name> [ "<name pattern>" ]
python3 /tmp/rev-fe-notes/da_for_lines.py <out-name>/coverage-final.json   # called by the script
# ground truth, from the campaign worktree
cd /Users/geilige/future-os/.worktrees/cov100-r2
python3 .future/cov100/covlines.py detail /tmp/rev-fe-notes/lcov.info --under src
```

The script runs jest with `--collectCoverageFrom` limited to the nine files
under review and writes its coverage into `/tmp/rev-fe-cov/<out-name>/`, so the
campaign worktree's `mobile/coverage/` is never touched (it is wiped by every
coverage run — the campaign's own trap).

## R2.7 Compliance

* **Write scope.** The only file written inside the campaign worktree is this
  one. Probe sources, coverage output, jest logs and the copied report all live
  under `/tmp/rev-fe`, `/tmp/rev-fe-notes`, `/tmp/rev-fe-cov`.
* **No production or test file under `mobile/src` was modified** — neither in the
  campaign worktree nor (beyond the probes added) in the scratch checkout.
  Nothing was deleted, weakened, `#[ignore]`d or skipped to make a measurement
  green; no assertion in an existing test was touched.
* **Formatting.** No `cargo fmt` (whole-crate or otherwise) was run — this unit
  is JS-only, and the campaign's per-file rule was never at issue.
* **Git.** Read-only commands only (`status`, `log`, `show`, `archive`,
  `rev-parse`, `merge-base`); no state-changing git command in the shared
  checkout.
* **Disk.** No instrumented target dir exists for this unit (`mobile` is a jest
  unit). `df -h .` showed 30 GB free on `/System/Volumes/Data` throughout; I
  deleted nothing and reclaimed nothing — not mine and not a peer's.
* **Measurement guard.** Every number I report is from a tool run whose exit code
  was checked: `covlines.py detail|total` both exited **0** (a nonzero exit
  invalidates the number whatever it printed — both runs are reproduced in
  §R2.1), every jest invocation exited **0**, and the 13-of-9859 residual is
  0.13 %, far below the "≥95 % zeroed" threshold that would signal a broken
  report.

## R2.8 Handoff

**Result: 13 / 13 waivers SURVIVED. No rework for any unit.** Three mechanism
corrections (§R2.4 items 1–3) and one boundary note (§R2.4 item 4) are worth
folding into `docs/testing/module-mobile.md` §Segment 50 by its author — none of
them changes a category.

*Suggested successor:* **none required** from this review (no line fell). If the
supervisor wants the residual artefact to carry these corrections, the successor
is a small documentation edit by the mobile unit:
1. reword `useChatFiles:201/208`'s mechanism to say line 184 is a child of
   `if (!file)` (so it holds on the no-warning path too);
2. reword `useTimelineController:79`'s mechanism to name the hook-order/client
   construction invariant (`useDesktopRuntime.ts:144` vs `:204`);
3. add the measured boundary sentences for `questionNav:67` and
   `client.ts:70/71`;
4. point `docs/testing/waiver-ledger.md` at §Segment 50 §3 (still outside the
   mobile unit's write scope).
