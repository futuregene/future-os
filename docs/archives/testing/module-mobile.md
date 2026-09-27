# Archived testing records — `docs/testing/module-mobile.md`

Moved out of `docs/testing/module-mobile.md` on 2026-09-27 during the
historical-pollution cleanup (task T4a, goal `goal_39567c2a6b22`,
worktree `docs-testing`). The text below is verbatim, with its original
dates; the live record keeps a one-line pointer where each block was
removed.

---


- **lines-100-or-waived** — **99.78 % lines (8258 / 8276)**, up from 99.67 % at
  the start of the segment. **All 18 remaining uncovered lines in 7 files are
  waived** with a category and a falsifiable reason (§5), and **no file is OPEN**
  (§6). The nine lines that were OPEN at the start of this segment are now
  **covered by tests** — path 1 of the two the contract allows.
- **dimensions** — §4 (earlier segments) covers all six required dimensions;
  §4b adds this segment's cases (the status stream's post-retirement frame, the
  connection core's cancelled-connect / post-barrier / stale-reconnect teardown,
  the probe identity matrix, and the revoked-mid-replacement refusal).
- **weak-tests-fixed** — no assertion-free, snapshot-only, skipped or
  coverage-only test was added. The one fault this segment is mine and is
  recorded as §8.31: an override of the module-level `classifyNatsError` mock
  leaked out of the test that set it and broke six unrelated tests in a *later*
  describe, because `jest.restoreAllMocks()` does not undo a `mockReturnValue`
  on a `jest.fn()` created by a `jest.mock` factory. It is now scoped with a
  `try/finally`.
- **flaky-timeouts-fixed** — **still partial, reported honestly**: the full suite
  was green in every run this segment (143/143, 2679 tests, wall 342 s on a
  loaded box), and no test failed with a timeout. The load-dependent residue of
  §7 remains reproduced-and-unfixed.

---


> **Superseded on 2026-09-26 by §1d.** After this section was written the
> supervisor added a *statement-vs-line divergence check* to the same gate
> (`_statement_check`): a line-based PASS is now refused when
> `uncovered statements - uncovered lines > max(20, uncovered lines)`. The module
> is back to **exit code 1** — 129 uncovered statements against 18 uncovered lines
> — and the historical PASS quoted below must not be read as the current verdict.
> The line-based waiver contract described here is still the rule the gate applies
> *after* the statement check passes.

For **seven consecutive segments** this gate could not return 0 for any JS/TS
module, for a reason outside every worker's write set: `cmd_js` borrowed a
`roots = …` block from `cmd_js_module` that referenced `needles` / `prefixes`,
which exist only in that other function, so every run died with
`NameError: name 'needles' is not defined` before the percentage or the waiver
path was evaluated.

**This segment the supervisor repaired it.** The gate now runs to completion and
produces the verdict quoted in *Gate status* above: completeness passes, the
waived files pass, and the failure names the 3 files whose remaining lines are
real missing tests. That is the correct verdict, and it means the module's only
remaining obstacle is the 46 uncovered lines in §6.

<details>
<summary>The old traceback, kept so the fix is recognisable in a regression</summary>

```
  future-mobile-ts: report covers all 141 expected source file(s)
Traceback (most recent call last):
  File "…/.future/cov100/verify.py", line 1277, in <module>
    sys.exit(main())
  File "…/.future/cov100/verify.py", line 1242, in main
    cmd_js(rest[0], float(rest[1]))
  File "…/.future/cov100/verify.py", line 396, in cmd_js
NameError: name 'needles' is not defined
```

The defect appeared at line 383 when it was first reported and had moved to 396 by
the time the OPEN-heading rule was added — the block was pure dead weight, since
`_assert_report_complete` is called just above it with the module's own root.

</details>

---

# Statement-level segment 2 (2026-09-26, second pass) — 95 statements left (HISTORICAL: superseded by segment 3 below)

---


**Artifact first: the numbers, then the per-file ledger, then what is next.**

| metric | statement-level segment 1 | **this segment** |
|---|---|---|
| statements | 97.50 % (9458/9701), 243 uncovered / 37 files | **99.02 % (9606/9701), 95 uncovered / 24 files** |
| lines | 99.78 % (8258/8276), 18 uncovered / 7 files | **unchanged: 99.78 %, 18 uncovered / 7 files** |
| suite | 143 files / 2784 tests | **143 files / 2818 tests, all green** |

The line figure cannot move: every statement covered in both statement-level
passes shares a line with an already-covered sibling, which is exactly the effect
the supervisor's divergence check exists to expose.

Measurement (one run, both reports from it — no narrow run writes them):

```bash
cd mobile
npx jest --coverage --maxWorkers=4       # 143 suites / 2818 tests, green
# -> coverage/coverage-summary.json + coverage/coverage-final.json
python .future/cov100/v8-uncovered.py mobile/coverage   # statement work list
python .future/cov100/verify.py js future-mobile-ts 99.99
```

## Gate status: FAIL (95 uncovered statements)

```
future-mobile-ts: report covers all 141 expected source file(s)
future-mobile-ts: 99.78% (8258/8276 lines) across 141 files, 18 uncovered line(s) in 7 file(s), target 99.99%
  statement coverage 99.02% (9606/9701) across 141 file(s) in scope, 95 uncovered statement(s) in 24 file(s)
FAIL: the line metric is hiding 77 uncovered statement(s) (18 uncovered lines vs 95 uncovered statements)
EXIT CODE: 1
```

The pass condition is `uncovered statements − uncovered lines ≤ max(20, uncovered
lines)`, i.e. **≤ 38 uncovered statements while ≥ 18 lines stay uncovered**. 95
remain, so this segment **does not claim completion**. 148 statements were covered
to get here (243 → 95); the remainder is listed below statement by statement so the
next segment does not have to rediscover it.

## What this segment covered (per file, with the test that does it)

| file | statements covered | tests (all in existing suites) |
|---|---|---|
| `src/remote/client.ts` | 1 | one new guard test in `client.test.ts`; the rest are waivers below |
| `src/remote/files.ts` | 12 | `limit and integrity guards` in `files.test.ts`: an SVG is a file not an image (L76); a PNG whose reported size runs past its real bytes ends the chunk walk instead of hanging (L125); the attachment **count** and **total-byte** quotas refused before any RPC (L303/305 — reached through `uploadAttachments`, which does not run the picker's own pre-check); the album reports itself **unavailable** (L496); a gallery photo in an unsupported format is refused before it is copied (L546); a pending picker result with no asset is a cancel (L637); the prepare retry ladder **exhausts** instead of looping (L856); a file too large to hash (L986); a native hash that is not a sha256 digest (L990); a JS hash read that returns nothing (L1002); a **stale cache entry of another size** is deleted before the download is written (L1072 — the assertion is the file's own size/bytes, which a leftover longer file would fail) |
| `src/remote/syncEngine.ts` | 11 | `lane lifecycle guards` in `syncEngine.test.ts`: a single oversized live frame is dropped and recovered from the journal (L211); `runCompleteLocally` needs a run id (L336); eviction cancels an armed retry (L358); `clear()` cancels a pending live flush (L373); the queued instruction list is bounded (L430 — 40 announced gaps produce ≤7 fetch rounds); a clock jump past a pending retry clears it instead of replaying twice (L485); a failure reported as the session is hidden arms nothing (L506); a hidden lane with an armed retry neither fires it nor retries (L511); the draft lane reconciles nothing (L520); a reconcile with no run to tail spends no replay (L664); a truncated replay marks the timeline (L754) |
| `src/remote/draftStorage.ts` | 3 | scheduling and match-clearing with no session id (L44/160); a stored draft with no text and no usable attachment reads as absent (L98) |
| `src/remote/pairing.ts` | 3 | an error body that is not JSON still reports the HTTP status (L37); an already-registered device id is reused (L64); a v1 invitation without the transport keys is refused before any request (L80) |
| `src/screens/DesktopsScreen.tsx` | 1 | a startup picker lets the system own the back gesture (L37) |
| `src/features/settings/SettingsScreen.tsx` | 1 | an approval tier cannot be written while the desktop is unreachable, and a second tap during a write writes once (L88) |
| `src/i18n/LanguageSettings.tsx` | 1 | re-selecting the active choice writes nothing (L15) |
| `src/components/TimelineCard.tsx` | 1 | a second copy tap restarts the flash instead of clearing it early (L370 — asserted on the Check glyph before and after the first deadline) |
| `src/features/chat/useQuestionNav.ts` | 1 | a jump re-aligns **once** after the list settles (L250 — asserted on `scrollToIndex` call count at 0/320/5000 ms) |

The dimension cases this segment adds: **error-path** (an unparsable error body, a rejected teardown, a refused quota, an exhausted retry ladder, a stale cache entry, a dropped oversized frame), **boundary** (a PNG shorter than it claims, `-1`/`NaN` sizes, 11 attachments, 21 MiB total, a 32-byte key with a padded spelling, a 512-entry run ledger), **platform-cfg** (Android's `Platform.OS !== "ios"` deferred-save timer in `RenameModal`, coverable only by flipping `Platform.OS` and restoring it in `finally`), **concurrency** (a retry armed and then hidden, evicted while armed, a clock jump racing the timer, a deferred flush racing `clear()`, a second copy tap racing the first flash, a double tap during an in-flight tier write), **serialization** (a v1 vs v2 invitation, a non-canonical base64url key).

## Every still-uncovered statement, with its category

Categories are the contract's: `unreachable-by-construction` (the types or the
single provider make it impossible), `unreachable-in-this-environment` (needs a
native modal lifecycle/reactor behaviour the jest renderer does not model),
`attribution-artifact`, `platform-unmeasured`.

**Waived with a proof (25 statements):**

| file:line | statement | category | reason |
|---|---|---|---|
| `client.ts:63,64` | `local`/unknown support code | unreachable-by-construction | `failureSupportCode` is private and only ever receives the six categories `recordFailure` is called with |
| `client.ts:626` | post-handoff stop check | unreachable-by-construction | `close()`/`setAppActive(false)` retire the generation, so the candidate's `check()` throws first (the new `onFeatures` close test ends in the catch at 651) |
| `client.ts:777,1101` | retry/refresh timer callbacks' terminal guards | unreachable-by-construction | every terminal path calls `clearTimers()` before it, so the armed timer is cancelled; the callbacks null the handle first |
| `client.ts:804` | `signal(ready)`'s stop guard | unreachable-by-construction | all four callers are guarded on the same tick and `transition()` ignores `ready` in terminal states |
| `client.ts:1033` | status-loop `continue` | unreachable-by-construction | `this.connection` changes only with `activeGeneration.retire()`, so `isLiveGeneration` above already ended the loop |
| `syncEngine.ts:595,623,667` | the three `stale_sync_lane` re-checks after an awaited read | unreachable-by-construction | each follows an await whose own `isCurrent` predicate (`replayInto` 688, `applyReplayEvents` 605/610/623 — strictly *stronger*, it also compares `baselineVersion`) has just returned true, and everything between is synchronous; **redundant defensive checks, reported not deleted** |
| `syncEngine.ts:799` | `if (!op) continue;` | unreachable-by-construction | `ops[index]` with `index < ops.length`; pure type-narrowing |
| `syncEngine.ts:931` | `commit`'s guard | unreachable-by-construction | all six call sites assign a non-null `lane.timeline` immediately above, each preceded by an `isCurrent` check on the same synchronous path |
| `useRemoteConnection.ts:119,595` | the `refreshNetworkStateRef` default and cleanup bodies | unreachable-by-construction | the network effect assigns the reference during mount; its cleanup installs the fallback exactly when the only caller (the listener) is removed |
| `useRemoteConnection.ts:417` | post-`drainRevokes` `!active` check | unreachable-by-construction | no `await` separates it from the effect body |
| `useRemoteConnection.ts:531` | `observe`'s `!active` check | unreachable-by-construction | both callers return on `!active` before reaching it |
| `useSessionCatalog.ts:82` | `observeRunEvent`'s type filter | unreachable-by-construction | the first guard admits only `agent_start`/`agent_end`, and `agent_start` returned above |
| `useSessionCatalog.ts:203` | `readSessions`'s `!client` | unreachable-by-construction | its only caller checked the same ref on the same tick and re-checks it before every later iteration |
| `useTimelineController.ts:436` | `laneIsCurrent` default | unreachable-by-construction | the engine always passes `isCurrent` as the second argument |
| `useTimelineController.ts:480` | `if (olderEntries.length === 0) break;` | unreachable-by-construction | the cursor guard above requires `start + olderEntries.length === nextBefore` **and** `start < nextBefore`, so a zero-length page always throws first |
| `useTimelineController.ts:532` | `stale_history_load` | unreachable-by-construction | the last await on that path is the paged read, which re-checks `isCurrent` (its own `stale_sync_lane`/`stale_json_decode`) after every request; the rest is synchronous |
| `files.ts:551` | `attachment_failed` | unreachable-by-construction | `prepareFiles` maps over a one-element array, so its result is never empty |
| `files.ts:863` | `download_prepare_failed` | unreachable-by-construction | `PREPARE_RPC_TIMEOUTS_MS` has two entries and every iteration either breaks with a response or throws |
| `files.ts:966` | `if (!directory.exists) return;` | unreachable-by-construction | its only caller runs `cachedDownload` → `cacheFile`, which creates that directory, before calling it |
| `replay.ts:105` | the second `replay_window_changed` | unreachable-by-construction | line 98 normalises the watermark under the same condition; **redundant, reported not deleted** |
| `projection.ts:981` | `if (!existing \|\| existing.kind !== "message")` | unreachable-by-construction | the `findIndex` that produced the index requires `kind === "message"` |
| `secureChannel.ts:20` | `equal`'s length mismatch | unreachable-by-construction | all four call sites compare fixed-width or `keyBytes`-validated slices |
| `RemoteContext.tsx:588` | `useRemote`'s missing-timeline throw | unreachable-by-construction | `useRemote` calls `useRemoteControls()` first, which throws the same error outside the provider; the single provider supplies both contexts, so only a hand-built partial provider could reach it — a wiring the public API cannot express |
| `DesktopsScreen.tsx:80` | `if (!desktop) return;` | unreachable-in-this-environment | the only callers are inside `<Modal visible={renameTarget !== null}>`; `jest-expo` renders no children while invisible, whereas a native modal keeps them mounted through its dismissal animation (the case the guard exists for) |
| `SessionsScreen.tsx:178` | `if (!name) return;` | unreachable-by-construction | the only caller is `RenameModal`, whose own save refuses a whitespace-only value before calling `onSave` |
| `MarkdownText.tsx:101` | non-`file` reference | unreachable-by-construction | the parser's minimal link mode can only produce `targetType: "file"` (its own `v8 ignore` notes say so) |

**OPEN — must be covered, not waived (70 statements in 14 files):**

| file | count | statements and the next step |
|---|---|---|
| `useTimelineController.ts` | 13 | 208 empty-prefix fast path (**drive `loadHistory` with a seeded paging window whose `endOffset` joins the tail and a cached live item that has no durable id**) · 424 `agent_end` refreshes sessions (**one `handleEvent` test**) · 438 + 785 client gone mid-reconcile (**a `requestRetry` mock that nulls `clientRef.current` for `get_session_entries`; assert an empty timeline and a `not_connected` failure**) · 637 the lane flips with the committed page (**flip `selectedRef` from the engine's own commit subscriber — the resolution and the flip are in one synchronous `commit()`**) · 894 snapshot-flip skips a complete local run (**establish start+end, then `applySessionStreaming(s1,false)` with `streamingRef[s1]=true`; spy that `reconcile` is not called**) · 914/922/928/945/955 the compaction wait (**already-aborted signal; a second concurrent await; two settle sources; a probe whose entry check runs after a settle; a probe that finds the session still compacting, which arms the inner 5s poll**) |
| `useFileDownload.ts` | 12 | 107 a superseded handle cannot reveal · 138 `beginDownload(visible=true)` · **282 Android** `flushPendingPreviewAction` → `platform-unmeasured` on the iOS suite, cover it by flipping `Platform.OS` (same pattern as `RenameModal`) · 413/424 the progress and waiting callbacks with a nulled handle · 420/427/520/530 the invisible-handle arms · 507 the visible-handle arm of the share/save path · 522 `setTransferProgress` on the no-handle path · 609 a cancel between `namedExternalFile` and the platform handoff (**these are the `visible:false` handoff flows; the existing suite only drives `visible:true`**) |
| `usePromptOutbox.ts` | 9 | 60 attachment identity map · 395/396/399 the recovery entry guards · 467 `not_connected` · 539 the continuation admission guard · 562 a recovery error after the client changed · 602/604 the mount-time mount/drain pair (**the existing suite exercises the delivered paths; these are the no-client and already-owned paths — mock a null `clientRef` and a pre-existing `continuationInFlightRef`**) |
| `client.ts` | 1 | the only coverable one left is the `local` fast path (waived above); the remaining 7 are the waivers |
| `SessionList.tsx` | 8 | 159/482 selection add/remove · 174/184/226/232/270 the delete guards (empty target list, offline desktop, a delete already running) · 378 row-press selection (**needs a selection-mode render; `SessionList.test.ts` already has the row helpers**) |
| `useConversationController.ts` | 4 | 104 the settings-event type filter · 107 a non-object JSON payload · 231 `skills_not_connected` · 281 the `download_cancel` catch on a stale conversation |
| `SessionsScreen.tsx` | 2 | **170 Android** `flushPendingNewConversation` (same `Platform.OS` pattern) · 280 the disconnected-state retry button **`onPress`** (press it and assert `reconnect`) |
| `useShareIntake.ts` | 3 | 65/76/82 a pending share belonging to another desktop (**flip `desktopRef` between the awaits**) |
| `MarkdownText.tsx` | 2 | 92/93 an inline image chip in a list item (the two statements that render and press it) |
| `ChatScreen.tsx` | 2 | 70/71 the minimum-visible timer cleanup (**finish a sync while the notice is showing, then change the key/session before the hide timer fires**) |
| `MathFormula.tsx` | 1 | 13 the source-text fallback for a formula the renderer cannot parse (`renderMathSvg` returns null) |
| `PreviewModal.tsx` | 1 | 229 the `busy` guard on an action (**render with `busy` and press an action; assert `dismissPreviewThen`/`downloadOriginal` were not called**) |
| `ProviderKeyPage.tsx` / `CustomProviderPage.tsx` / `SkillsSettingsPage.tsx` | 1 each | the `busy` (and `!provider`) double-submit guards: press save twice in one tick and assert one write |

### Next step (named, not claimed)

1. The 13 `useTimelineController` statements are the largest single block and every
   one has a named construction above.
2. Then `useFileDownload` (12) + `usePromptOutbox` (9) + `SessionList` (8) = 29,
   which alone would take the count to ~24 and **pass** the divergence check; they
   are all `visible:false` handoff flows, selection-mode rows and null-client
   guards, i.e. more test wiring rather than new understanding.
3. Re-run the two commands above; the gate needs ≤ 38 uncovered statements at 18
   uncovered lines. **Update (from segment 3): this cannot be reached by tests
alone** — see the structural finding in segment 3: 18 of the statements are
unreachable by construction, and `_statement_check` has no waiver path.

No production source file was modified. No assertion was weakened, no narrow run
produced either report, and no waiver above is a restatement of "I did not get to
it" — each names the call sites or the guard that makes the statement unreachable.

---


Every test below is in the file's existing suite (no new suite was created).
`mobile/coverage/coverage-final.json` was regenerated by the same full run
(`npx jest --coverage --maxWorkers=4`, 143 suites / 2784 tests, green).

| file | statements covered | new tests (all assertions on observable behaviour) |
|---|---|---|
| `src/remote/client.ts` | 46 | 39 in a new `lifecycle guards around a failing transport` describe: every teardown rejection (`cancelAttempt`, the readiness barrier, `connectSocket`'s catch, `disposeConnection`, the candidate failure, the previous-socket handoff) must still drop the socket; the armed retry is cancelled by a healthy serving probe; a budget that expires during a probe or before a disconnect/reconnect frame fails the client and publishes no presence; a presence callback or a feature announcement that closes the client cannot publish `ready`; the four rotation guards (adopted after close, cancelled mid-write, transport failure with a live socket, failure while closing); the expired-JWT rotation reset; a 513-entry notified-run ledger; a stale-lane failure that lands after the window closed fails at once with no retry episode; a late connect attempt / late failure / late deadline is dropped; frames with no waiter, no secure channel, a dead generation, a stale lane and a spent batch yield are dropped; a presence re-handshake posts one handshake, flips the lane to selective events and is dropped when it lands after the close; a ready client whose socket vanished refuses commands as `not_connected`; a throwing status stream is reported and still retried; plus the real-Noise handshake arms (no message, no confirmation, stopped after the confirmation / while writing credentials) in `secureClient.test.ts` |
| `src/remote/useRemoteConnection.ts` | 19 | an unpair during each of the connect's three awaits stops the attempt; a credential rotation landing while the pairing is removed is not adopted; a replaced client cannot write credentials, reconcile sessions, publish state or start a recovery; a catalogue snapshot the reducer rejects touches no conversation; a bootstrap superseded during the registry read or the credential read never connects; a foreground recovery with no client is not an error, and one that lands after the app left again is dropped; a failed/answered network snapshot that lands after unmount is not logged/recovered; a desktop switch superseded by a second switch never connects the first; a switch to an unpaired desktop fails loudly; a superseded pairing claim is not reported; unpair survives a desktop that never answers; removing the active desktop unpairs, removing an unknown one changes nothing |
| `src/remote/useSessionCatalog.ts` | 19 | a model recovery timer that outlives the catalogue is cleared (on unmount, on a newer refresh, on `reset`, and when its epoch was replaced or the client removed); refreshes without a client are no-ops; a terminal event with an unusable payload (no runId, an unrelated type, truncated JSON, an array) is not announced; the notified-run ledger stays bounded at 512 without losing recent dedup; title generation refuses a missing connection/session, a reply from a replaced generation and a refused/blank title; a rename, a deletion and a workspace cascade acknowledged by a replaced generation are dropped; a revision beacon with nothing to compare against is ignored |
| `src/remote/secureChannel.ts` | 8 | non-canonical/short keys and identifiers are refused; a channel with wrong-sized split keys or id is refused; empty/non-ASCII/oversized AAD contexts and unanswerable reply requests are refused; an oversized handshake message and an unfinished handshake are refused |
| `src/remote/projection.ts` | 9 | a compaction frame with no operation/checkpoint and an unknown `compaction_*` frame are ignored; an empty replay returns the given timeline; a replay whose lane goes stale before or after the fold is refused; a message that is neither a user bubble nor an assistant reply renders nothing (while a stopped or timed assistant row still renders); a live bubble that already has attachments keeps them; a settled placeholder for a durable checkpoint is folded away (and a running one only drops from the live lane) |
| `src/remote/replay.ts` | 2 | a lane that goes stale between the page read and its merge stops the replay; a tail page repeating a snapshot after it was pinned is refused |
| `src/remote/readPages.ts` | 2 | a lane that goes stale never starts — or continues — a chunked read |
| `src/remote/codec.ts` | 3 | an empty QR payload and a wrong-endpoint invitation are refused; v2 invitations need two real 32-byte keys |
| `src/remote/nativePresentation.ts` | 1 | a second background/active round trip after the grace was released cannot release it twice |
| `src/features/chat/readPreviewText.ts` | 1 | a file whose reported length is `-1`/`NaN` is refused before any read |
| `src/features/chat/useCompactContext.ts` | 1 | an acknowledgement for another session is refused instead of waited on |
| `src/features/chat/components/RenameModal.tsx` | 1 | **Android**: the zero-delay fallback flush commits the rename when no `onDismiss` arrives (see §4c) |
| `src/components/MarkdownImage.tsx` | 1 | a double tap on the load button starts only one transfer |
| `src/components/MarkdownText.tsx` | 3 | a local image inside a list item renders an openable file chip; an in-document anchor wrapped around an image never reaches the OS |

---


> **Correction (2026-09-26, superseded by §Segment 49).** The reasoning below is
> **wrong** and is kept only as a record of the error. The attempt it describes
> captured the item's `onPress` and invoked it *without* the sheet's dismiss, so
> `confirmDeleteWorkspace` never ran at all — the measurement could not tell "the
> wrapper latched" from "nobody flushed". Driving the captured `press`+`flush`
> **pair** twice reaches the guard with the ref set: `rev-fe` measured
> `232 hits=[2, 1]` (`docs/testing/review-frontend.md` §4.1), and a test now
> covers it (§Segment 49, `hits=[10, 1]`, register row retired).

Same stale-closure technique as §Segment 27's successful 184: capture the
workspace-delete menu item's `onPress`, start a batch delete, then invoke the
captured handler. Coverage stayed `hits=[7, 0]` — the `ActionMenu` wrapper latches
the item on dismiss, so a stale item cannot fire. Genuinely defended; the
attempted test was deleted (its assertion would have been trivially true).

---

*Also moved on 2026-09-27 by task T4b (the statement-level segment handoff that
followed 1b, and its gate-status snapshot):*

## 1c. Handoff summary (statement-level segment)

The previous segment's gate read istanbul's **line** column and passed on it. The
supervisor then added a cross-check (`_statement_check` in `verify.py`): a PASS on
lines is refused when `uncovered statements - uncovered lines >
max(20, uncovered lines)`, because istanbul credits a line to *any* statement that
ran on it. The same full run that reported 18 uncovered lines (7 files) reported
**243 uncovered statements in 37 files** — 13× the line count — so the green line
metric was hiding real untested error paths, guards and boundary cases (verified
by inspection: `if (x) return;` with the `if` executed 8× and the `return` 0×).
This segment worked that statement list, largest file first.

- **lines-100-or-waived** — the line metric is **unchanged at 99.78 % (8258 /
  8276, 18 uncovered lines in 7 files)**, and that is the expected shape of this
  work: every statement covered here shared a line with an already-covered
  sibling, so the line column *cannot* move. **Statement coverage moved 97.50 % →
  98.67 %** (9458/9701 → **9572/9701**): **114 statements covered**, 243 → **129
  uncovered**, in 37 → **30 files**. The gate **does not pass yet** and this
  segment **does not claim completion**: 129 statements remain, 6 of the 12
  largest work-list files are still untouched (§6b), and the next step is named in
  §10. The statements that were covered are listed per file in §3b.
- **dimensions** — §4c lists this segment's cases: error-path (teardown
  rejections on every close path, a stale lane mid-read/merge, a handshake reply
  with no message or no confirmation, an empty/non-ASCII/oversized AAD context,
  an unreadable terminal payload), boundary (empty event list, empty retention
  prefix, the 513-entry notified-run cap, a 1025-character context, a
  non-canonical 43-character key, a `MAX_SECURE_PLAINTEXT`-sized record, a file
  size of `-1`/`NaN`), platform-cfg (**Android's deferred-save timer** in
  RenameModal, which was uncovered precisely *because* the suite runs as iOS),
  concurrency (an in-flight attempt superseded by close/unpair/an epoch bump, the
  reentrancy guard on an image load, the double-settle guard on a compaction
  wait, a frame whose lane moved on between delivery and decoding) and
  serialization (a v2 invitation's 32-byte key validation, a padded/non-canonical
  base64url key, a compaction frame with no operation or checkpoint).
- **weak-tests-fixed** — no assertion-free, snapshot-only or coverage-only test
  was added; each new test asserts an observable that the guard's removal would
  change (a second dial, a published presence, a repeated handshake, an extra
  failure episode, a reordered/deleted list row, a handle released twice). No
  existing assertion was weakened and no production line was deleted or made
  unreachable for coverage.

---

## 1d. Gate status for this segment (red, by design)

```
python .future/cov100/verify.py js future-mobile-ts 99.99
  future-mobile-ts: report covers all 141 expected source file(s)
future-mobile-ts: 99.78% (8258/8276 lines) across 141 files, 18 uncovered line(s) in 7 file(s), target 99.99%
  statement coverage 98.67% (9572/9701) across 141 file(s) in scope, 129 uncovered statement(s) in 30 file(s)
FAIL: future-mobile-ts: the line metric is hiding 111 uncovered statement(s)
      (18 uncovered lines vs 129 uncovered statements)
EXIT CODE: 1
```

The verifier is right to fail this: 129 statements are still uncovered and only
18 of them also appear in the line metric. **Do not read the previous segment's
`PASS` as still valid** — it was produced before `_statement_check` existed. The
per-file categories for the 129 are in §6b; the ones this segment proved dead are
waived in §5b.
