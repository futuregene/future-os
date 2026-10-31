# web-remote (desktop vitest, 远端 / 设置组) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`.

**Scope of this file**: the *remote group* of the web lane —
`src/features/remote/`, `src/features/remote-peer/`, `src/features/settings/`, and
the scattered lines in `src/lib/` and `src/integrations/`. The other half of the
lane (`src/components/layout/`, `features/vibe/`, `features/agent/`,
`features/palette/`, `features/tasks/`) belongs to a sibling worker and is
registered in [`web.md`](./web.md); four of this group's lines were already
registered there and are **not** repeated here (§0).

**Branch**: `test/cov-web2` (cut from `origin/dev-huichen@07b0efab2`, since merged
as `728663ea1` and rebased on top of it — the branch is now **0 commits ahead** of
`origin/dev-huichen`; this file's numbers below were re-measured on that merged
tree and the earlier table is kept as the history of the round that produced the
tests).

**Measurement** (both legs are the same command, same Node, same tree):

```bash
cd desktop
npx vitest run --coverage --coverage.reporter=lcov --coverage.reporter=json-summary
python3 ../coverage/2026-11/tools/lcov-missed.py coverage/lcov.info --json before|after.json
```

| round | tree | `DA:0` (whole lane) | `DA:0` (this group) |
|---|---|---:|---:|
| 1 — before | `origin/dev-huichen@07b0efab2` | 188 | 76 |
| 1 — after | `test/cov-web2@ae5f28bbd` | **142** | **30** |
| 2 — re-measured | merged tree (`728663ea1`), *this* file's rows audited | **111** | **30** |

Round 1's drop is **−46**, all of it from this group. Round 2 changed no product
code and no test: it re-measured on the merged tree (the sibling's waves 5–7 and
the lane's other fixes moved the lane total 142 → 111 outside this group) and
audited every row here — see §R2. This group's residual is unchanged at 30, of
which **22 are the remote-page files** that round 2 was scoped to and **8 are the
settings/lib lines** registered here in round 1.

Suite on the merged tree: **402 files / 5631 tests**, all green; istanbul lines
**99.32 %** (16220 / 16331) with the denominator unchanged, because this group's
work touches no production code — every changed file is a test or a doc.

Of the 30 lines still reading `DA:0` in this group, **4 were already waived in
`web.md`** (§0) and **26 are registered here**. That accounts for all 30: the rest
of the group is at **`DA:0` = 0**, and the lane's total is 111 = 30 + 81 (the
out-of-group files of §10).

## 0. Already registered in `web.md` — deliberately not duplicated

| file:line | where it is registered |
|---|---|
| `desktop/src/lib/theme.ts:229` | `web.md` row 1 |
| `desktop/src/features/settings/AccountPage.tsx:60` | `web.md` row 18 |
| `desktop/src/features/settings/EnvironmentPage.tsx:45` | `web.md` row 19 |
| `desktop/src/features/settings/CommunityEditionSection.tsx:27` | `web.md` row 20 |

Each of those four names its own mechanism and falsifier there. Nothing in this
file restates them, so a reader who counts waivers should count
`22 (web.md) + 26 (here) = 48` and must not double-count these four.

## R2. Round 2 — the post-merge audit of the 22 lines

Round 1's work landed as `728663ea1`, and the group's remote-page files then read
**22** `DA:0` lines:

| file | lines |
|---|---|
| `features/remote-peer/useRemoteTimeline.ts` | 9 — `380, 666, 750, 834, 879, 889, 992, 995, 1000` |
| `features/remote-peer/DeviceChoiceDialog.tsx` | 3 — `104, 107, 198` |
| `features/remote-peer/RemoteComposer.tsx` | 3 — `412, 727, 778` |
| `features/remote/RemoteView.tsx` | 2 — `353, 374` |
| `features/remote-peer/RemoteConversationView.tsx` | 1 — `642` |
| `RemotePeersView.tsx` · `RemoteStandbyNotice.tsx` · `RemoteWorkspaceCreateDialog.tsx` · `useRemoteApprovals.ts` | 1 each — `178`, `50`, `42`, `155` |

Every one of them is already a row in the table above, so round 2's job was not to
write more rows but to **try to falsify the existing ones on the merged tree**.
Three things were done, all re-runnable; the outcome is that nothing moved — the
22 stay waivers — but they now rest on measurements rather than on reading.

### R2.1 The audit (automated, and it fails loudly)

```bash
cd desktop && npx vitest run --coverage --coverage.reporter=lcov && cd ..
python3 coverage/2026-11/after/web2/audit-waivers.py \
    --lcov coverage/2026-11/after/web2/lcov.info \
    --root .worktrees/cov-web2/desktop/src \
    --json coverage/2026-11/after/web2/waiver-audit-round2.json
# → rows: 26  pass: 26  fail: 0
```

Two checks per row, and the second is the one that catches a merge:

1. the line still reads `DA:0` — a row that reads `1` must be **deleted**, not kept;
2. the construct the row quotes is still on the recorded line **or the guard line
   immediately above it** — an unbraced `if (c)` / `return;` carries the `DA:0`
   record on the `return` while the condition sits one line up, so the window is
   two lines wide by construction.

Result on the merged tree: **26 pass, 0 fail** (`waiver-audit-round2.json`). No
row is stale, and no quoted line has drifted since registration.

`coverage/` is gitignored (regenerated output), so the script — like
`coverage/2026-11/tools/lcov-missed.py` — does not travel with a clone, and both
checks are written out here in one line each for the reader who has none of it:

```bash
# 1. still uncovered? (the 22 remote-page rows, from the table below)
awk '/^SF:/{f=substr($0,4)} /^DA:[0-9]+,0$/{split(substr($0,4),a,","); print f":"a[1]}' \
  desktop/coverage/lcov.info | grep -E 'RemoteView.tsx:(353|374)\b'
# 2. still the same construct? print the recorded line and the guard above it
sed -n '352,353p' desktop/src/features/remote/RemoteView.tsx
```

Check 2 is a `grep` of the `construct` column below at that line pair — which is
why every row quotes its code with the line number rather than paraphrasing it.

### R2.2 Four executable probes for the claims that could be measured

Run from a throwaway `src/features/remote-peer/__waiver-probes.test.tsx`
(4 tests, green, deleted afterwards — the bodies below are the whole harness), and
then measured again with `--coverage.include=` on the two files:

```ts
// 107: can the arrow-walk ever have zero rows?
mountDeviceDialog([
  peer({ agentAvailable: false, name: "Build box" }),   // nothing installed
  peer({ control: "standby", name: "Held box" }),       // another device drives it
  peer({ connected: false, error: "refused", name: "Off box" }),
]);
document.querySelectorAll("[role='dialog'] button:not(:disabled)").length
// observed: 1 — "This computer" is never disabled and never removed.

// 104: is the key listener alive while the ref can be null?
await unmount(); document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown" }));
// observed: the dialog is gone and nothing answers — the listener left with it.

// 879 / 992: can either reader be handed a non-object?
pushEvent("a string");                                  // payload is not an object
// observed: no entry, no throw → dropped at the session gate (:665) first.

// 995 / 889 / 1000: can `data` reach the reader as anything but a string?
for (const data of [5, null, undefined]) pushEvent({ data, type: "agent_text", … });
// observed: no entry, no throw → `readRaw` normalises (`JSON.stringify` / `?? ""`).
```

Measured afterwards (probe + the components' own suites, `--coverage.include`):
`DeviceChoiceDialog 104/107/198` and `useRemoteTimeline 879/889/992/995/1000` all
**still read `DA:0`**. That is the positive control the rows were missing: the
adversarial shapes were actually sent through the real components, and the
defensive arms still did not run.

The same run also settles row 15 (`useRemoteTimeline:666`): a frame naming
`sess_2` reaching a `sess_1` transcript draws nothing **and leaves `:666` at 0**,
which is the measurement behind the claim that the listener at `:764-765` drops it
first.

### R2.3 The controls that were already tests

For the "same condition disables the control" family (§1) the control does not
need a new probe — the suite already presses the disabled control and asserts
nothing happened, which is what makes the guarded press unproducible:

| row | existing control (test name) |
|---|---|
| 1 `RemoteView:353` | *does not open a QR code until a device is asked for* — no code, so no Copy button |
| 2 `RemoteView:374` | *hides the id while the switch is off* — no id row, so no Copy button |
| 3 `RemotePeersView:178` | *will not pair an empty link* — asserts the Connect button `disabled` |
| 4 `RemoteComposer:412` | *closes attachment staging while another device holds the machine* — asserts `attach.disabled` |
| 7–9 `DeviceChoiceDialog` | *walks the rows with the arrows and skips a machine that cannot take work* — proves a row always exists to walk; §R2.2 measures the count |
| 10 `RemoteConversationView:642` | *offers no fork on a user entry, nor before any prompt* |
| 11 `RemoteStandbyNotice:50` | *disables the control while the takeover is in flight* |
| 12 `RemoteWorkspaceCreateDialog:42` | *names the machine it writes to and refuses to submit without a path* |
| 25–26 `UpdatePage:82/145` | *offers the install control only while an update exists*; §9's `presentation.repair` derivation |

### R2.4 Outcome

- **0 of 22 converted to work items**: no probe reached a guarded line, and the
  disabled-control family is pressed (and refused) by tests that already exist.
- **0 rows deleted**: none reads `1`.
- Rows 5, 6, 13, 14, 16, 17, 18 stay on their round-1 mechanisms (the field is
  always drawn; two guards on one fact; the shared reader throws `stale_sync_lane`
  first; an attribution artifact; a ref filled during render) — each is re-checked
  by R2.1 on every future run, so drift shows up as a failed audit rather than a
  stale paragraph.

## The 26 lines

Category names (`W1`–`W8`) are the ones defined in `.future/memory/coverage-llvm-cov.md`.
Only **W2** (defensive dead arm), **W5** (attribution artifact) and **W8** (dead
producer) occur in this group; nothing here needs another platform, a broken
filesystem, or a race that cannot be scheduled from a test.

| # | file:line | category | construct | pinned by |
|---|---|---|---|---|
| 1 | `features/remote/RemoteView.tsx:353` | **W2** | `copyCode`'s `if (!pairingCode) return` | `RemoteView.test.tsx` :: *says the pairing code could not be copied instead of claiming it was* |
| 2 | `features/remote/RemoteView.tsx:374` | **W2** | `copyDeviceId`'s `if (!remoteStatus?.desktopId) return` | `RemoteView.test.tsx` :: *says this machine's id could not be copied instead of claiming it was* |
| 3 | `features/remote-peer/RemotePeersView.tsx:178` | **W2** | `handlePair`'s `if (!link) return` | `RemotePeersView.test.tsx` :: *will not pair an empty link* |
| 4 | `features/remote-peer/RemoteComposer.tsx:412` | **W2** | `attach`'s `if (held) return` | `RemoteComposer.test.tsx` :: *keeps the caret in the draft when a conversation row is pressed* |
| 5 | `features/remote-peer/RemoteComposer.tsx:727` | **W2** | text effect's `if (!node) return` | `RemoteComposer.test.tsx` :: *walks the conversation menu upwards, wrapping at the first row* |
| 6 | `features/remote-peer/RemoteComposer.tsx:778` | **W2** | `chooseReference`'s `if (!referenceQuery) return` | `RemoteComposer.test.tsx` :: *offers that host's conversations after `#` and sends the reference as a link* |
| 7 | `features/remote-peer/DeviceChoiceDialog.tsx:104` | **W2** | keydown's `if (!list) return` | `DeviceChoiceDialog.test.tsx` :: *opens this machine with Enter, the row it starts on* |
| 8 | `features/remote-peer/DeviceChoiceDialog.tsx:107` | **W2** | keydown's `if (rows.length === 0) return` | `DeviceChoiceDialog.test.tsx` :: *walks the rows with the arrows and skips a machine that cannot take work* |
| 9 | `features/remote-peer/DeviceChoiceDialog.tsx:198` | **W2** | `chosenRows(null)`'s `return []` | `DeviceChoiceDialog.test.tsx` :: *leaves a key it does not claim to whatever else answers it* |
| 10 | `features/remote-peer/RemoteConversationView.tsx:642` | **W2** | `forkAt`'s `if (!message \|\| message.role === "user") return` | `RemoteConversationView.test.tsx` :: *offers no fork on a user entry, nor before any prompt* |
| 11 | `features/remote-peer/RemoteStandbyNotice.tsx:50` | **W2** | `take`'s `if (taking) return` | `RemoteConversationView.standby.test.tsx` :: *disables the control while the takeover is in flight* |
| 12 | `features/remote-peer/RemoteWorkspaceCreateDialog.tsx:42` | **W2** | `submit`'s `if (!trimmed \|\| busy) return` | `RemoteWorkspaceCreateDialog.test.tsx` :: *names the machine it writes to and refuses to submit without a path* |
| 13 | `features/remote-peer/useRemoteApprovals.ts:155` | **W2** | `readJson`'s `if (typeof value !== "object") return null` | `useRemoteApprovals.test.ts` :: *accepts a request whose body arrived already decoded* |
| 14 | `features/remote-peer/useRemoteTimeline.ts:380` | **W2** | replay's `if (!isCurrent()) return` | `useRemoteTimeline.test.ts` :: *drops a run log that arrives after the conversation was left* |
| 15 | `features/remote-peer/useRemoteTimeline.ts:666` | **W2** | session gate's `return` in `processEvent` | `useRemoteTimeline.test.ts` :: *ignores a pushed event that names another conversation* |
| 16 | `features/remote-peer/useRemoteTimeline.ts:750` | **W5** | `() => !cancelled` passed as the read's `isCurrent` | `useRemoteTimeline.test.ts` :: *re-reads the record when a referenced body cannot be fetched* |
| 17 | `features/remote-peer/useRemoteTimeline.ts:834` | **W2** | `settleRunsInPage`'s `if (!streams) return` | `useRemoteTimeline.test.ts` :: *reads history on open and exposes what it read* |
| 18 | `features/remote-peer/useRemoteTimeline.ts:879` | **W2** | `readRaw`'s `if (typeof value !== "object") return null` | `useRemoteTimeline.test.ts` :: *re-encodes a decoded body and ignores an absent one* |
| 19 | `features/remote-peer/useRemoteTimeline.ts:889` | **W8** | `readRaw`'s `catch { return null }` | `useRemoteTimeline.test.ts` :: *re-encodes a decoded body and ignores an absent one* |
| 20 | `features/remote-peer/useRemoteTimeline.ts:992` | **W2** | `readJson`'s non-object `return null` | `useRemoteTimeline.test.ts` :: *drops a frame whose payload or fields are the wrong shape* |
| 21 | `features/remote-peer/useRemoteTimeline.ts:995` | **W2** | `readJson`'s `if (typeof field !== "string") return field ?? null` | `useRemoteTimeline.test.ts` :: *re-encodes a decoded body and ignores an absent one* |
| 22 | `features/remote-peer/useRemoteTimeline.ts:1000` | **W8** | `readJson`'s `catch { return null }` | `useRemoteTimeline.test.ts` :: *re-encodes a decoded body and ignores an absent one* |
| 23 | `features/settings/CustomProviderDialog.tsx:233` | **W2** | `detectThinkingLevels`'s `if (!onProbe) return` | `CustomProviderDialog.probe.test.tsx` :: *has no detect control when the provider is not reachable from here* |
| 24 | `features/settings/CustomProviderDialog.tsx:303` | **W2** | post-loop `if (probeRunRef.current !== runId) return` | `CustomProviderDialog.probe.test.tsx` :: *drops a failed level probe that arrives after the dialog closed* |
| 25 | `features/settings/UpdatePage.tsx:82` | **W2** | `handleInstall`'s `if (!status?.hasUpdate) return` | `UpdatePage.flow.test.tsx` :: *offers the install control only while an update exists* |
| 26 | `features/settings/UpdatePage.tsx:145` | **W2** | `handleRepairAcknowledgement`'s `if (!recovery) return` | `UpdatePage.flow.test.tsx` :: *treats a refused acknowledgement as a no-op, not as an error* |

## 1. The three "same condition disables the control, so the guard never fires" rows

Rows 1, 2, 3, 4, 12, 23, 25, 26 are one family, and it is worth stating the
family once: the handler's early return repeats a condition the render already
used to disable (or not draw) the only control that calls it.

```tsx
// RemoteView.tsx:600-610 — the Copy button exists only inside this branch
{pairingCode
  ? (
      ...
      <Button onClick={() => void copyCode()}>…</Button>
// RemoteView.tsx:351-353
async function copyCode() {
  if (!pairingCode)
    return;
```

```tsx
// RemoteView.tsx:404-416 — the id row, and its Copy, exist only when both hold
{running && remoteStatus?.desktopId ? ( … onClick={() => void copyDeviceId()} … ) : null}
// RemoteView.tsx:372-374
async function copyDeviceId() {
  if (!remoteStatus?.desktopId)
    return;
```

```tsx
// RemotePeersView.tsx:252-266
<Button disabled={!invitation.trim() || busy !== null} onClick={() => void handlePair()} …>
// RemotePeersView.tsx:175-178
async function handlePair() {
  const link = invitation.trim();
  if (!link)
    return;
```

```tsx
// RemoteComposer.tsx:1029 — `held` is a prop of the disabled condition
<button aria-label={t("attach")} disabled={uploading || held} onClick={() => void attach()}>
// RemoteComposer.tsx:408-412
async function attach() {
  if (held)
    return;
```

`RemoteWorkspaceCreateDialog.tsx:42`, `CustomProviderDialog.tsx:233`,
`UpdatePage.tsx:82/145` are the same shape:
`disabled={busy || !path.trim()}`, `{onProbe ? … : null}`, and the install /
repair controls rendered only while `status?.hasUpdate` / `presentation.repair`.

**Falsifier for the family**: a disabled `<button>` in jsdom swallows the click, so
no test can produce the call that reaches the guard without deleting the
`disabled`/branch (which is a production change, out of policy for a coverage
task). This was *measured* once on the sibling lane rather than assumed —
`AccountPage` in `web.md` row 18 records the experiment: removing the `disabled`
left the line at `DA:0` and broke two assertions. What a *later* reader can do
instead, in one line: `git diff` the render for that control and check that the
guard's condition is a subset of the `disabled`/branch condition. If it is not a
subset, the guard is a work item, not a waiver — that is how
`TasksView:376` was reclassified from waiver to work item in the sibling lane.

## 2. `RemoteComposer.tsx:727` — W2 (the field is always drawn)

```tsx
// RemoteComposer.tsx:724-728
useEffect(() => {
  const node = textareaRef.current;
  if (!node)
    return;
  growToContent(node);
```

The `<textarea ref={textareaRef}>` at `:1014` is rendered unconditionally —
there is no branch, no `held`, no empty state that removes it — so the ref is
populated in the same commit the effect runs after, and `node` is non-null on
every render of this component.

**Falsifier**: render the composer with every prop combination the component
accepts (`held`, `streaming`, `busy`, uploads, a restored draft) and assert the
textarea is present. If any of them removed it, this row must become a test.
The nearest real risk — a *remount* between effects — cannot happen, because the
effect's dependency is `[text]`, not the component's lifetime.

## 3. `RemoteComposer.tsx:778` and `useRemoteTimeline.ts:666` — W2 (a second guard on the same fact)

```tsx
// RemoteComposer.tsx:711 — the menu exists only when there is a query
const referenceMenuOpen = referenceQuery !== null && referenceOptions.length > 0;
// RemoteComposer.tsx:776-778 — its two call sites are both inside that branch
function chooseReference(option: RemoteReferenceOption) {
  if (!referenceQuery)
    return;
```

`chooseReference` is reached from `onChoose` on the menu (`:847`) and from the
Enter handler that is itself wrapped in `if (referenceMenuOpen)` (`:807-809`).
Both are unreachable with a null query.

```ts
// useRemoteTimeline.ts:764-766 — the listener filters the session id first
if (value.desktopId !== desktopId || readString(value.payload, "sessionId") !== sessionId)
  return;
// useRemoteTimeline.ts:665-667 — and `processEvent` re-checks it
if (readString(payload, "sessionId") !== sessionId)
  return;
```

`processEvent` has three callers: the listener above (already filtered), the
content lane's `deliver` (fed only from that listener), and the `kind !== "event"`
arm (`:760-762`), which enters `processEvent` with a **resumed** frame and returns
at the `kind === "resumed"` branch (`:649-664`) long before this line. The
duplicate check is what makes `processEvent` safe to call from anywhere; it is
therefore a guard on a contract, not a reachable branch.

**Falsifier for both**: call `processEvent` with a foreign `sessionId` from a
second entry point (the test would have to add one), or drive `chooseReference`
with the menu closed (the menu is the only affordance that carries it). Both are
reachable *only* through a production change, so both stay waivers — and the
tests named in the table pin the behaviour they protect (a foreign conversation's
frame draws nothing; a row press completes the reference).

## 4. `DeviceChoiceDialog.tsx:104/107/198` — W2 (three arms the dialog's own shape closes)

```tsx
// DeviceChoiceDialog.tsx:100-107
function handleKeyDown(event: KeyboardEvent) {
  const list = listRef.current;
  if (!list)
    return;
  const rows = chosenRows(list);
  if (rows.length === 0)
    return;
```

- `:104` — the listener is installed by `useEffect` (`:100`), i.e. after the
  commit that attached `ref={listRef}` to the scrolling box (`:162`), and removed
  on unmount. The ref cannot be null while the listener is registered.
- `:107` — `chosenRows` selects `button:not(:disabled)` inside that box, and the
  first row (this machine, `:168-174`) is never given `disabled`. So there is
  always at least one row, even on a host whose every peer is unreachable — that
  is what the picker is for.
- `:198` — `chosenRows(null)`'s `return []`. Its callers are `:105` (guarded by
  `:103`) and the mount-time focus at `:72`, which runs in the same commit that
  populated the ref.

**Falsifier**: render the dialog with `peers={[]}` and with every peer disabled
and count `[role=dialog] button:not(:disabled)`; if the count can reach 0, row 8
becomes a work item. The two tests named in the table do exactly this walk (an
unavailable machine is skipped, Enter opens the row it starts on).

## 5. `RemoteConversationView.tsx:642` — W2 (the fork control is drawn only on replies)

```tsx
// RemoteConversationView.tsx:640-642
const message = messages[index];
if (!message || message.role === "user")
  return;
```

`forkAt` is called from a message's own `onFork` (`:521`), and `MessageBlock`
renders that control inside its `!isUser` branch:

```tsx
// features/agent/MessageBlock.tsx:314-333
{!isUser
  ? ( …
        {!streaming && onFork ? (<button … onClick={() => onFork(message)} …>) : null}
```

`index` comes from the `rendered.map` that draws the row, so `!message` is not
reachable either.

**Falsifier**: pass a user message to `forkAt` through a caller that is not
`MessageBlock`; none exists. Worth keeping as a guard — a fork addressed at a
user entry would ask the host to branch from the turn that *asked*, which is the
one id the host refuses to re-fork.

## 6. `RemoteStandbyNotice.tsx:50` — W2 (the source says so)

```tsx
// RemoteStandbyNotice.tsx:39-50
/**
 * One takeover at a time.
 *
 * The control is disabled while it is in flight, so `taking` is the only guard
 * a second press would need …
 */
async function take() {
  if (taking)
    return;
```

```tsx
// RemoteStandbyNotice.tsx:66-70
<button aria-label={t("takeOver")} disabled={taking} onClick={() => void take()} …>
```

The component's own comment states the mechanism: the only affordance is
disabled while `taking` is true.

**Falsifier**: find a second call site of `take` (there is exactly one, `:68`).
If the banner ever gained a keyboard or programmatic trigger, this row becomes a
test *and* a real bug (two takeovers in flight).

## 7. `useRemoteTimeline.ts` — the six that stay, and why

```ts
// 371 — the staleness predicate this group is about
const isCurrent = () => epochRef.current === epoch && streamsRef.current === streams;
```

### `:380` — W2, guarded upstream by the shared reader

```ts
// 378-380
const replay = await fetchRunReplay({ …, isCurrent });
if (!isCurrent())
  return;
```

`fetchRunReplay` delegates the page loop to
`@future-os/thread-projection`'s `readRunReplay`, which calls the same
`isCurrent` **after every page** and throws `stale_sync_lane` (`replay.ts:295-296`
and `:312-313`). Measured: the test *drops a run log that arrives after the
conversation was left* resolves the page after the switch, and the shared reader
throws — the reply is correctly absent, and this line stays 0. The only way to
reach it is staleness between the shared reader's last check and this one, i.e.
inside the microtask boundary, which is not a state a test can put the process
in.

**Falsifier**: a test that resolves the page promise and bumps the epoch *between*
the reader's final check and its return would cover it; there is no seam between
those two statements (both are in the same continuation).

### `:750` — W5, attribution

```ts
// 746-750
read: (event, contentRequest) => requestRemotePeer<ContentPage>(desktopId, {
  type: "get_event_content_range",
  runId: event.runId ?? "",
  contentRequest,
}, sessionId, () => !cancelled),
```

The line is the callback's *tail* — `() => !cancelled` — and the arrow body is
recorded on this line. It is invoked only by the real `requestRemotePeer`, which
uses it to abandon a chunked read; every test in this lane stubs
`requestRemotePeer` wholesale (that is what makes the transport testable), so the
arrow is created and never called. The two neighbours on the same statement
(`:746`, `:749`) do read 1, which is the signature of an attribution artifact
rather than a missing test.

**Falsifier**: call the fifth argument from the stub — a mock that invokes
`isCurrent()` before resolving. That would cover the line while proving nothing
about production, which is why it is not done here.

### `:834` — W2, a ref initialised during render

```ts
// 306-308
const streamsRef = useRef<RemoteRunStreams | null>(null);
if (!streamsRef.current)
  streamsRef.current = new RemoteRunStreams();
…
// 828-834
function settleRunsInPage(streams: RemoteRunStreams | null, …) {
  if (!streams)
    return;
```

The ref is filled on the hook's first render, and the only call site passes it
(`:485`). `null` is the type's allowance, not a state this hook reaches. (The
type is `| null` so the ref can be created lazily; the lazy creation happens
before any code can observe it.)

**Falsifier**: call `settleRunsInPage` directly with `null` — it is module-private,
so that requires exporting it for the test, a production change made purely for
coverage.

### `:879` and `:992` — W2, the payload is always an object

```ts
// 877-879
function readRaw(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
// 990-992
function readJson(value: unknown, key: string): unknown {
  if (typeof value !== "object" || value === null)
    return null;
```

Both are called with the event's `payload`, which is itself read with
`readString(payload, "sessionId")` first (`:665`, `:764`): a non-object payload
returns `null` there and the frame is dropped before either function runs. The
same holds for the content lane's `deliver`, which rebuilds the payload as an
object spread (`:751`).

**Falsifier**: push `{kind:"event", desktopId, payload: "a string"}` — measured
during this round: it is dropped at `:665` (that is the point of the check), so
neither function sees it.

### `:889` and `:1000` — W8, the producer cannot emit the failure

```ts
// 883-890
if (field === undefined || field === null)
  return null;
try {
  return JSON.stringify(field);
}
catch {
  return null;
}
// 996-1001
try {
  return eventPayload(value, field);
}
catch {
  return null;
}
```

- `:889` — `JSON.stringify` throws on a circular structure or a `BigInt`. The
  value here is a field of a payload that crossed the Tauri IPC boundary as JSON,
  so it is acyclic and contains no `BigInt` by construction. A decoded body (the
  case the function exists for) is still plain JSON.
- `:1000` — `eventPayload` cannot throw: its own `JSON.parse` is wrapped in
  `try { … } catch { return null }` (`packages/thread-projection/src/eventPayload.ts:19-22`),
  and `hasEventPayload` is a `hasOwnProperty` test. The callee swallows the one
  failure this line was written for, so the arm is dead code that reads as a
  safety net.

**Falsifier for `:889`**: hand `readRaw` a payload holding a `BigInt` — impossible
through the wire, possible only by pushing an object literal past the event
listener, i.e. by testing a shape the IPC layer cannot produce.
**Falsifier for `:1000`**: make `eventPayload` throw (e.g. a getter that throws on
property access). That is a change to a shared package made to cover a line in a
caller, and it would pin the caller's behaviour against a callee that does not do
this.

### `:995` — W2, `data` is normalised to a string before it is read

```ts
// 765-770 — the lane hands the event on with `data` as a string
contentLane.enqueue({
  type: readString(value.payload, "type") ?? "",
  data: readRaw(value.payload, "data") ?? "",
…
// 994-995
if (typeof field !== "string")
  return field ?? null;
```

`readRaw` re-encodes an already-decoded body (`JSON.stringify`, `:886`) and maps
absent/null to `""`, so by the time `entryFromEvent` reads `data` it is always a
string. The arm exists for a caller that passes the *wire* payload straight to
`entryFromEvent`; today the lane is the only caller.

**Falsifier**: bypass the lane — call `processEvent` with `data` as an object.
`processEvent` is inside the hook and `:751` rebuilds the payload, so this again
needs a production entry point.

## 8. `CustomProviderDialog.tsx:303` — W2 (two guards between the loop and it)

```ts
// 279-303
const accepted: string[] = [];
for (const level of knownThinkingLevels) {
  setProbing({ key: model.key, level });
  try {
    const outcome = await run(level);
    if (probeRunRef.current !== runId)   // 284
      return;
    …
  }
  catch (probeError) { … }
}
if (probeRunRef.current !== runId)       // 302
  return;                                // 303
```

`probeRunRef` has exactly two writers: `++probeRunRef.current` when a run starts
(`:248`) and the open/close effect (`:148`, deps `[initial, open]`). Between the
`:284` check of the **last** iteration and the `:302` check there is no `await`,
no state write and no render — so a run that reaches `:302` is necessarily still
current. (`knownThinkingLevels` is a non-empty module constant, so the loop also
cannot be skipped into this line.)

**Falsifier**: a re-render with a new `initial` scheduled *between* the last
level's answer and this check; React cannot interleave a render into that
synchronous tail.

**Note on the shape of the reading** (this is what makes `:302` look like it
never executes, which it does on every successful run): LCOV attributes an
unbraced `if (c)` / `return;` pair to the *branch*, so both lines read `0` until
the `return` is taken. Measured on the neighbouring pair `:284/:285`, which reads
`1/1` — that return **is** taken, by the pre-existing test *stops probing once
the dialog is closed*, which resolves a level probe after the effect bumped
`probeRunRef`. The pair `:302/:303` therefore reads `0/0` not because the
condition never ran but because nothing can make it true.

## 9. `UpdatePage.tsx:82/145` — W2, and one arm that *was* reachable

```tsx
// 80-82
async function handleInstall() {
  if (!status?.hasUpdate)
    return;
// 143-145
async function handleRepairAcknowledgement() {
  if (!recovery)
    return;
```

- `:82` — the install control is rendered only in the `status?.hasUpdate` branch,
  and the existing test *offers the install control only while an update exists*
  documents the same conclusion ((`check_app_update` saying "up to date" removes
  the button entirely)).
- `:145` — the confirm control is rendered only when `presentation.repair`, and
  `updatePresentation` derives `repair` from `recovery` itself
  (`updatePresentation.ts:9`): `Boolean(result && !result.repairAcknowledged && …)`.

The **reachable** neighbours in the same function were the interesting half and
are now covered: `acknowledged === false` (`:150`, a host that no longer holds
that repair is a no-op, not a failure) and the fallback download button
(`:204`, the generic page opened only when nothing more specific is known). Both
have tests named in the sibling commit.

**Falsifier for `:145`**: render the confirm control with `recovery === null` —
`presentation.repair` would have to be true without it, which is a change to
`updatePresentation`'s contract.

## 10. What this group still owes nothing on

Every line in `src/features/remote/`, `src/features/remote-peer/`,
`src/features/settings/` (outside the four rows in §0), `src/lib/` and
`src/integrations/` is either covered or one of the 26 rows above.

The lane's remaining **112** `DA:0` lines live in **43** files outside this
group, untouched by this branch (measured identical in both legs), and none of
them is claimed here as a waiver:

- the sibling worker's files — `components/layout/` (39: `AppShell` 19,
  `ActivityRail` 12, `hooks/` 8), `features/vibe/` (16), `features/agent/` (23),
  `features/palette/CommandPalette` (4), `features/tasks/` (5);
- files neither branch claims yet — `artifacts/` (3), `filepreview/` (1),
  `filetree/` (2), `markdown/` (1), `runs/RunsPanel` (1), `skills/SkillsView` (2),
  `terminal/` (4).

Those are work items in the campaign's own bookkeeping (`web.md` counts them as
"reachable and not yet written"), not waivers, and a reader who adds this file's
26 to `web.md`'s 22 must still treat the gap between **48** and **142** as work.

## 11. Re-running this measurement

```bash
cd desktop
npx vitest run --coverage --coverage.reporter=lcov
python3 ../coverage/2026-11/tools/lcov-missed.py coverage/lcov.info --top 100
# this group only:
python3 ../coverage/2026-11/tools/lcov-missed.py coverage/lcov.info \
  --filter src/features/remote | head
```

The 26 rows are asserted to still read `DA:0` by this check; a row that reads `1`
means the code changed under it (the guard was removed or a branch opened) and
the row should be deleted rather than kept. That is the same "no stale rows" rule
the ledger applies to every module.
