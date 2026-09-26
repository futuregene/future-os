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
| `client.ts:777`, `:1101` | both timer guards are cancelled before they can fire | `:775-779` / `:1099-1103` read; the exhaustive arming/clearing argument lives in the doc's §Segment 21/§2266 | not falsified (not re-measured) |
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
