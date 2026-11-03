# mobile (jest) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov/istanbul *summary* percentages are not the acceptance number.

**Last measurement**: `test/cov-mobile` at `c78099114`, cut from `origin/dev-huichen@075e469c7`,
Node **v22.22.3** (mobile's jest does not start under Node 26).

**Result**: `DA:0` = **10** lines in **5** files, all waived here — 8 records over 7 distinct
constructs, every one of them a branch the product cannot take. The other 35 lines this wave
started with (22 files) are covered by the tests merged in `075e469c7`; `c78099114` added the
three probes below, which cover no new line but make the three controls reproducible.

```bash
export PATH=/opt/homebrew/opt/node@22/bin:$PATH      # v22.22.3
cd mobile && npm run gen-version
npx jest --coverage --runInBand                      # → mobile/coverage/lcov.info

# Metric A, with nothing but the lcov file (works from a clean clone):
awk '/^SF:/{f=substr($0,4)} /^DA:[0-9]+,0$/{split(substr($0,4),a,","); print f":"a[1]}' \
  coverage/lcov.info | sed 's|.*/mobile/||'
```

The campaign's working copies also carry `coverage/2026-11/tools/lcov-missed.py`
(per-directory grouping and a JSON form) — that tree is gitignored, which is exactly why
this file exists in `docs/`: the numbers above are reproducible from the command shown.

Category names (`W1`–`W8`) are the ones defined in `.future/memory/coverage-llvm-cov.md`:
W1 cfg(windows) · **W2 defensive dead arm** · W3 OS failure injection · W4 race window ·
W5 attribution artifact · W6 ghost line · W7 mock closure · **W8 dead producer**. That list
was written for Rust/llvm-cov; only W2 and W8 map onto a jest/istanbul lane. **No row here is
W3, W5 or an environment class** — nothing in this list needs another platform, a network, or
a host capability.

Each row states the code that makes it unreachable, the test that pins that code, and how to
falsify it. The ledger convention (`docs/testing/waiver-ledger.md`) applies: **a waiver that
cannot name its own falsifier is not evidence**.

## The ten lines

| # | file:line | category | construct | pinned by |
|---|---|---|---|---|
| 1 | `mobile/src/components/MarkdownText.tsx:139-140` | **W8** | inline `case "image"` chip for a local path | `mobile/src/components/__tests__/MarkdownText.test.ts` :: *an image inside a sentence › becomes its own block wherever it sits* |
| 2 | `mobile/src/features/chat/useMinimumVisible.ts:11-12` | **W2** | effect-body guard that clears a timer that cannot exist | `mobile/src/features/chat/__tests__/useMinimumVisible.test.ts` :: *an armed hide is dropped by a new notice, a changed floor, and an unmount* |
| 3 | `mobile/src/features/chat/files/useChatFiles.ts:217` | **W2** | progress callback's "dialog not up yet" fallback | `mobile/src/features/chat/__tests__/useFileDownload.test.ts` :: *a tapped attachment's transfer reports into the dialog it is already showing* |
| 4 | `mobile/src/features/chat/files/useChatFiles.ts:224` | **W2** | the same fallback on the waiting-for-network callback | same |
| 5 | `mobile/src/features/chat/files/useChatFiles.ts:318` | **W2** | `fetchDownload`'s no-handle fallback | *(static: the two call sites)* |
| 6 | `mobile/src/remote/useTimelineController.ts:108` | **W2** | `hydrateAttachmentsRef`'s initial no-op | *(static: writer precedes every reader)* |
| 7 | `mobile/src/remote/useTimelineController.ts:236` | **W2** | `loadHistory`'s unused `laneIsCurrent` default | *(static: engine's declared signature)* |
| 8 | `mobile/src/remote/timeline/useCompactionOutcomes.ts:29` | **W2** | `catch` around a callee that never throws | *(static: `eventPayload`)* |

## 1. `MarkdownText.tsx:139-140` — W8 (dead producer)

The arm turns an image node with a local path into a tappable chip. **No consumer can ask
for it**: `inlineRuns` splits every such picture out into its own block before the inline
renderer runs (`src/components/MarkdownText.tsx:171`):

```ts
if (node.type === "image" && (remoteMarkdownImageUrl(node.src) || localFilePath(node.src))) {
  parts = [{ image: node }];
```

The hoist predicate is a strict superset of what this arm requires (`localFilePath(node.src)`
truthy), and every `renderInline` entry point receives that split (recursion at 93/99/105/125/128/131/151,
run renderer at 201). So `case "image"` only ever sees an image with neither a remote nor a
local src, which `return label` at line 138 handles first.

- **Pinned by** `mobile/src/components/__tests__/MarkdownText.test.ts` :: *becomes its own block wherever it sits, and the
  picture can still be opened* — eight placements (bare, mid-sentence, in emphasis, in a
  link, list item, blockquote, heading, relative path), each yields exactly one
  `MarkdownImage`.
- **Positive control (measured)**: narrow line 171 to `if (node.type === "image" && remoteMarkdownImageUrl(node.src))`
  → this file's `DA:0` becomes **0** (139-140 execute) and the test above fails with
  `blocks: 0`.
- **Falsifier**: a call path into `renderInline` carrying an image with a local `src`.

## 2. `useMinimumVisible.ts:11-12` — W2

The effect body re-checks for a stale timer before doing anything. The only assignment to
`timerRef.current` is the armed path's `setTimeout` (lines 26-33), and that path returns the
cleanup (`src/features/chat/useMinimumVisible.ts:34-37`):

```ts
return () => {
  if (timerRef.current) clearTimeout(timerRef.current);
  timerRef.current = null;
};
```

React runs that cleanup before the next body whenever `[active, key, minimumMs, visible]`
changes, and the timer callback nulls the ref itself (line 28). At the top of every body the
ref is therefore already `null`; the guard is a defensive re-check.

- Counter across the full suite (2949 tests): the `if` at line 10 evaluates **102** times, true **0** times.
- **Pinned by** `mobile/src/features/chat/__tests__/useMinimumVisible.test.ts` :: *an armed hide is dropped by a new notice, a
  changed floor, and an unmount* (a new notice, a changed floor, and unmounting while armed).
- **Positive control (measured)**: delete lines 34-37 → `DA:0` for this file becomes **0**
  (11-12 execute) and that test fails.
- **Falsifier**: a React effect re-run that skips its own cleanup.

## 3-4. `useChatFiles.ts:217` and `:224` — W2

Both are the `else` of `if (handle.visible)` inside the transfer callbacks — "the dialog is
not on screen, so raise it". The dialog is raised before the transfer starts, under the same
condition that starts it (`src/features/chat/files/useChatFiles.ts`):

```ts
200:          showDownload(handle, {          // reveal first …
206:        if (!file) {
207:          file = await remote.downloadAttachment(   // … then transfer
```

`handle.visible` is only cleared by `hideDownload` (`useTransferOperation.ts:129`), reachable
only through `confirmDownloadFor`, which at 187 runs **before** 200 and is followed by a
re-reveal; `cancelActiveDownload` aborts and releases the handle without touching `visible`.
And even if the fallback ran, `showDownload` returns early for a handle that is no longer the
active transfer, so it could not produce an effect.

- Counters across the full suite (2949 tests): the `if (handle.visible)` at 216 is **9** true /
  **0** false; the one at 223 is **8** true / **0** false.
- **Pinned by** `mobile/src/features/chat/__tests__/useFileDownload.test.ts` :: *a tapped attachment's transfer reports into the
  dialog it is already showing* — the dialog is asserted non-null before the first byte and
  after each progress/waiting patch.
- **Positive control (measured)**: delete the `showDownload` block at 200-204 → **217 and 224
  become covered**.
- **Falsifier**: a transfer that runs while its handle is hidden (i.e. a caller that starts
  `downloadAttachment` without the reveal above it).

## 5. `useChatFiles.ts:318` — W2

`fetchDownload`'s `else` (no handle → report into the screen-level progress value) is
unreachable because both of its call sites pass a handle —
`useChatFiles.ts:375` (`fetchDownload(info, cachedFile, handle)`) and `:559`
(`fetchDownload(info, cachedPreview?.file ?? null, handle)`), and its only non-handle source,
`openOrShare`'s `existingHandle ?? beginDownload(...)` (343-348), returns with a toast when
`beginDownload` yields null.

- Counter across the full suite (2949 tests): `if (handle)` at 309 is **22** true / **0** false;
  the conditional expression at 318 is `[0, 0]`.
- **Pinned by**: static — the two call sites above. No probe test.
- **Positive control: none.** Dropping the `handle` argument at 375 was tried and is
  *inconclusive*: it also changes the visibility path, so the resulting `DA:0` set is a
  different group of lines, not 318. Recorded here rather than dropped.
- **Falsifier**: a third `fetchDownload` call site without a handle.

## 6. `useTimelineController.ts:108` — W2

`hydrateAttachmentsRef`'s initial value (`async () => undefined`). Its only reader is
`useTimelineController.ts:199`, inside `handleEvent`, guarded by the event type:

```ts
198:      if (event.type === "user_message")
199:        void hydrateAttachmentsRef.current(sid);
```

and the ref is replaced in the mount effect at 686-687 before any connection callback can run.
Nothing resets `.current` back to the initial value, and the hook's result is the only other
holder.

- **Pinned by**: static — writer before reader; `grep -rn hydrateAttachmentsRef mobile/src`
  lists this file and its tests only.
- **Falsifier**: a `handleEvent` invocation during the same commit, before the mount effect.

## 7. `useTimelineController.ts:236` — W2

`loadHistory`'s default parameter `laneIsCurrent: () => boolean = () => true`. The function is
only handed to the sync engine as `requestHistory` (`useTimelineController.ts:586`), whose
declared type takes both arguments and whose two call sites always pass the second
(`packages/remote-client/src/syncEngine.ts:90`, `:610`, `:629`).

- Counter across the full suite: the `default-arg` branch is `[0]`; the body from 237 on is
  covered.
- **Pinned by**: static — the engine's signature.
- **Falsifier**: a one-argument `requestHistory(sessionId)` call.

## 8. `useCompactionOutcomes.ts:29` — W2

The `catch` around `eventPayload(event, event.data)`. The callee swallows parse failures
itself (`packages/thread-projection/src/eventPayload.ts:20-24`):

```ts
export function eventPayload(event: object, raw?: string | null): unknown {
  if (hasEventPayload(event)) return (event as Decoded)[decoded];
  if (!raw) return null;
  try { return JSON.parse(raw) as unknown; } catch { return null; }
}
```

and it cannot throw for the input it gets here: `compactionTerminalOf` reads `event.type`
first (lines 19-24), so `event` is a non-null object by the time the try starts (`null`
would have thrown before it, a primitive returns `undefined` and exits early).

- **Pinned by**: static — the callee.
- **Falsifier**: any `eventPayload` input that throws from inside the try.

## Cross-check against the 2026-10 campaign

The October campaign recorded waivers for the same subtree under a different metric (the
cov100 gate's `extra` decomposition, `docs/testing/waiver-ledger.md` +
`docs/testing/module-mobile.md` §Segment 50). Four of the constructs here were reached
independently there; the line numbers drifted as the files changed:

| this file | October record | note |
|---|---|---|
| `MarkdownText.tsx:139-140` | `module-mobile.md:597`, `MarkdownText.tsx` **92-93** | same construct, same argument (hoist is a strict superset) |
| `useMinimumVisible.ts:11-12` | `module-mobile.md:598` — filed under **`ChatScreen.tsx` 70-71** | **the path in that row is wrong**; see below |
| `useTimelineController.ts:108` | `module-mobile.md:599`, first part (`hydrateAttachmentsRef`) at **302** | same conclusion |
| `useTimelineController.ts:236` | `module-mobile.md:599`, last part (`loadHistory` default) at **436** | same conclusion |

Four of the eight records here are **new** (no October equivalent): the `useChatFiles`
cluster (217/224/318) and `useCompactionOutcomes:29`.

**The defective row.** `docs/testing/module-mobile.md:598` attributes the "timer that can never be
there" argument to `mobile/src/features/chat/ChatScreen.tsx:70-71`, while its own prose cites
`timerRef.current` at 86/87 and a cleanup at 93-96. Measured on this tree:
`grep -c timerRef mobile/src/features/chat/ChatScreen.tsx` → **0**, and that file's 68-71 is a
comment about Android keyboard insets. The construct it describes is
`useMinimumVisible.ts` (its effect at 9-38, cleanup at 34-37). Treat row 598's *mechanism* as
confirmed and its *path* as the defect; do not follow it to `ChatScreen.tsx`.

## Reproducing a control

Every control is a one-line edit, a targeted jest run with `--coverage`, and the
`DA:0` count above — e.g. for row 2:

```bash
cd mobile
# delete the `return () => { … }` cleanup at useMinimumVisible.ts:34-37
npx jest --runInBand --coverage --coverageReporters=lcov \
  --collectCoverageFrom='src/features/chat/useMinimumVisible.ts' \
  src/features/chat/__tests__/useMinimumVisible.test.ts
awk '/^DA:[0-9]+,0$/' coverage/lcov.info | wc -l   # → 0 lines missed
git checkout -- src/features/chat/useMinimumVisible.ts                 # restore
```

## Gate state at the measurement commit

`npx tsc --noEmit` → 0 · `npm run lint` → 0 (0 errors, 0 warnings) ·
`npx jest --runInBand` → 156 suites / 2949 tests green.
