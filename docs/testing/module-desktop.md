# desktop (TS/React) module — coverage, dimensions, waivers

Module id for the acceptance checks: `future-desktop-ts`
(`desktop/src` plus `packages/markdown`, `packages/thread-projection`,
`packages/json-preview` as exercised through the desktop suite).

## 1. Measurement

```bash
cd desktop
npx vitest run --coverage --testTimeout=180000
# -> desktop/coverage/coverage-summary.json  (total line/statement/branch/function %)
# -> desktop/coverage/coverage-final.json    (per-file statement map, for uncovered lines)
# round-2 metric (口径 A) — per-line statement hits, from the run above:
#   (the report is written with the checkout's absolute paths; `covlines-js.py`
#    strips `/Users/geilige/future-os/` itself, so `--under` is the path *below
#    that root* — in a worktree it is `.worktrees/<name>/desktop/src`)
python3 ../.future/cov100/covlines-js.py desktop/coverage/coverage-final.json --under .worktrees/cov100-r2/desktop/src
```

**The self-diagnosing gate — use this form, not the one-line `cd desktop && … &&
python3 … desktop/coverage/…` shortcut.** That shortcut cannot pass: it leaves the
shell inside `desktop/`, so its file argument resolves to
`desktop/desktop/coverage/coverage-final.json`, python exits 1, and the gate
reports a failure while the suite is green. §8.13 has the full diagnosis, its
evidence, and the two false hypotheses it ruled out. The report is also wiped by
every coverage run, so it must be read in the *same* command that produced it:

```bash
cd desktop && npx vitest run --coverage --testTimeout=180000; V=$?; python3 ../.future/cov100/covlines-js.py coverage/coverage-final.json --under .worktrees/cov100-r2/desktop/src | tail -1; P=$?; echo "VITEST_RC=$V PY_RC=$P"; test $V -eq 0 && test $P -eq 0
```

It prints each leg's exit code on the last line, so a failure names itself instead
of leaving the reader to guess which half broke. Segment 14's run (the current
record): `files_with_uncovered=33 uncovered_lines=59 measured_lines=12800
lines%=99.5391` with `VITEST_RC=0 PY_RC=0` (318 test files / 4335 tests) —
repeated twice, agreeing exactly. Segment 13's, for comparison:
`files_with_uncovered=34 uncovered_lines=60 lines%=99.5312` with `VITEST_RC=0`
(317 / 4332); segment 9's: `files_with_uncovered=35 uncovered_lines=61
lines%=99.5234`, `VITEST_RC=0`.

**A run whose vitest leg is not 0 is not a measurement** — §8.19 records a
concurrent-run collision that produced a plausible but partial report (71/40, a
previous segment's state) while exiting 1.

**But the gate (`--verify`) must NOT run the suite, and must not pipe the metric
leg.** A goal validator is killed after 120 s
(`orchestration/goal/src/validator.rs`, `DEFAULT_TIMEOUT`), and this suite takes
15–25 s **idle** — under a full campaign's load it can exceed that, and a validator
killed mid-run leaves `coverage-final.json` half-written, i.e. the zeroed-report
trap that reads as "everything uncovered". The validator is therefore the metric
leg **alone**, with no pipe (a `| tail -1` would make `$?` the status of `tail`,
masking the tool's exit 3):

```bash
python3 .future/cov100/covlines-js.py desktop/coverage/coverage-final.json --under .worktrees/cov100-r2/desktop/src
```

It runs from the goal cwd (so every path is right) in seconds, and exits **3** on a
zeroed, partial, or out-of-scope report — a nonzero exit means "the report you
measured from is not usable", never "coverage failed". Read that exit code, not
just the stdout (§9.5). `tsc --noEmit` belongs in the same handoff pass as the
suite, not in the validator: types are checked by the suite's own run anyway and
the validator's job is the one cheap tripwire.

**And the per-line tripwire is the real machine check** (supervisor's
`reconcile.py`, §2): it reads the report *and* the doc and reports every uncovered
line the doc never names. It is the only check that inspects each line rather than
a number, and it is what `residual-classified` should be judged by.

Run identity for the round-2 segment recorded below: revision `8449acf6` in
`/Users/geilige/future-os/.worktrees/cov100-r2` plus the uncommitted test files
listed in §3; host macOS 26 (arm64), Node v26.3.0, vitest 4.1.11, react/react-dom
19 (development build), `--testTimeout=180000` for the measurement run only.
`desktop/coverage/` is wiped by every coverage run, so keep logs outside it.

Coverage is configured in `desktop/vite.config.ts` (`test.coverage`): provider
`v8`, `include: ["src/**/*.{ts,tsx}"]`, `exclude: ["src/**/*.test.{ts,tsx}",
"src/test/**", "src/**/*.d.ts"]`, `reporter: ["text", "json-summary", "json"]`,
`reportsDirectory: "./coverage"`. Two deliberate deviations from the plan text:

- The plan asked for `all: true`. **Vitest 4.1 removed `coverage.all`** (it is no
  longer in `CoverageOptions`; `all: true` would fail `tsc`), and `coverage.include`
  has taken over its meaning — untested files are reported at 0%. That is what
  makes the number below a whole-tree measurement instead of a
  files-a-test-happened-to-touch measurement.
- The suite is run with `--testTimeout=180000` **only for the measurement run**.
  `src/features/markdown/streamingSharedParser.test.ts` ("bounds parser work on
  100 growing {prose,table} frames") takes 3.6 s / 4.6 s uninstrumented but
  24.8 s / 32.9 s under v8 instrumentation, so it trips the 30 s default on an
  instrumented run. No assertion was touched; only the hang fence is raised.
  Plain `npm run test` (what CI runs) is unchanged. Coverage is not enabled by
  default, so CI is unaffected by the new config.

## 2. Before / after (round 2, 口径 A: per-line `DA:0`)

`desktop/src`, measured with the command in §1. Both columns are whole-tree v8
runs (`include: ["src/**/*.{ts,tsx}"]`), so the denominator is stable and the
numbers are comparable across the campaign.

| metric | baseline @8449acf6 | seg 1 | seg 2 | seg 3 | seg 4 | seg 5 | seg 6 | seg 7 | seg 8 | seg 9 | seg 13 | seg 14 (this) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| uncovered lines (`DA:0`) | 292 | 266 | 187 | 126 | 105 | 99 | 91 | 84 | 71 | 61 | 60 | **59** |
| of which waived (§5) | 0 | 13 | 14 | 32 | 40 | 41 | 43 | 45 | 47 | 57 (4 OPEN — §12) | 56 (the other 4 are OPEN — §5) | **55 (the other 4 are OPEN — §5)** |
| files with uncovered lines | 69 | 54 | 53 | 48 | 47 | 46 | 45 | 44 | 40 | 35 | 34 | **33** |
| measured lines | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 | 12 800 |
| line % | 97.7188 | 97.9219 | 98.5391 | 99.0156 | 99.1797 | 99.2266 | 99.2891 | 99.3438 | 99.4453 | 99.5234 | 99.5312 | **99.5391** |
| test files / tests | 305 / 4150 | 310 / 4189 | 311 / 4234 | 314 / 4273 | 314 / 4294 | 314 / 4300 | 315 / 4307 | 315 / 4312 | 315 / 4320 | 316 / 4331 | 317 / 4332 | **318 / 4335** |

The `seg 9` column's *disposition* figure is shown as re-registered by segment 12
(§5), so that row is a running statement of each line's current disposition rather
than a historical snapshot; its coverage figures (61 / 35 / 99.5234) are what
segment 9 measured, and segments 10–12 are identical to it for every row (they
changed no test and no production line).

Segment 9 is the first in which the residual is **disposed of line by line**:
every one of the 61 lines it left carries a waiver tag with a stated mechanism and
a falsification route (§5) — and after segment 12's rework, 4 of them carry an
**OPEN — not waived** registration instead (§5). Two obligations of the campaign's
end state must not be conflated: *classification* is complete for this module;
*coverage* is not (the objective is `DA:0 = 0`, and 59 lines still execute zero
times). What remained was the referee's job on those 61 — 4 of the 14 added in
segment 9 reverse a previous round's premise (§8.15), segment 10's review (§10)
was the first pass at that job (**0 of the tags *it* attacked fell, 4 mechanisms
were replaced**), and segment 11 attacked the four non-React tags mechanically
(all four survived). The independent verdict then arrived: **REV-DESK
(`review-desktop.md`) falsified three tags — `D25`
(4 lines), `D41` (1 line) and `D54` (1 line)** — the last of them in the review's
later segments, whose own accounting (§32/§33) is 25 tags checked over two passes,
10 of them enumeration-only. All three findings are acted on, and they resolve in
two different ways:

* **`D25` is OPEN** (segment 12): its 4 lines leave the waiver count, because the
  `unreachable-by-construction` category was false — an ordinary Space press
  reaches them — and no consumer exists for a test to assert. It waits on a
  product decision (§8.11).
* **`D41` is covered** (segment 13, §8.18): that falsified tag was an
  *environment* claim, and the environment turned out to be the test harness. The
  line is now reached by an assertion-bearing test beside the hook
  (`useFutureLoginFlow.staleTick.test.ts`), so it leaves the residual entirely:
  61 → **60 uncovered lines, 34 files, 99.5312 %**, from **three full runs that
  agree exactly** (§2's stability rule; line 104's hit count is 5 in every one).
* **`D54` is covered** (segment 14, §8.20): the same shape as `D41` — an absolute
  claim about a *commit* where the code writes a ref synchronously and only
  *queues* the commit; the review's A/B probe took the guarded return, and
  `useThreadMessages.warmSnapshotTimer.test.ts` now does the same in the shipped
  suite. 60 → **59 uncovered lines, 33 files, 99.5391 %**, again from three full
  runs that agree exactly (every other line identical to segment 13's report,
  compared file by file).

So the module's claim is now **covered or waived except 4 lines awaiting a product
decision** — 55 waived + 4 OPEN — and those 55 waived lines are still **pending an
independent verdict, per line**. (The sentence here used to read "44 rows covering
37 lines have not been reviewed at all"; that was written from the review's first
pass, before its later segments landed — §9.1 carries the supervisor's correction
and `review-desktop.md` §32 is the review's own final accounting.)

Segment 10 (same day) adds no coverage and changes no test: it is the adversarial
review of those tags (§10). Segment 11 adds the four non-React tags' checks and
the calibration that follows from them. Segment 12 acts on the review's `D25`
finding and on the stale source comment behind a segment-9 premise — no coverage
change (§3/§8.11/§8.17). **Segment 13 acts on the review's `D41` finding, and
segment 14 on its `D54` finding, by covering the line in each case** — the
module's only coverage changes since segment 9, one line each, so every other
figure is carried over unchanged.

Segment 8 closed 13 lines — the whole first two batches of the work list:

| file | segment 7 | segment 8 |
|---|---|---|
| `features/terminal/TerminalView.tsx` | 4 | **2** (waived, D39–D40) |
| `features/remote-peer/useRemoteSkillRecommend.ts` | 3 | **0** |
| `features/remote-peer/useRemoteSkillCatalog.ts` | 2 | **0** |
| `features/remote-peer/useRemoteConversationSettings.ts` | 3 | **0** |
| `features/remote-peer/RemoteConversationSettings.tsx` | 3 | **0** |

**Measurement stability (recorded because a flaky line moves every number by
±1).** This suite has **no oscillating line**: two independent full runs of the
same code state agree exactly (44 files / 84 lines on all 84, then 40 / 71 after
this segment's work), compared line by line across every file. That is the
counterpart of the `feishu_ws.rs:254`-style hazard the flake inventory lists, and
it means the deltas in this table are the work rather than run-to-run noise.

**`--scope` must be an absolute prefix for an Istanbul report** (§8.14): with the
relative `--scope desktop/src` the tripwire matches no file, prints
`uncovered lines=0 / MISSING: 0`, and **exits 0** — a false clean in exactly the
direction the tool exists to prevent. The check of record for this table:

```bash
python3 .future/cov100/reconcile.py --report desktop/coverage/coverage-final.json \
  --scope "$PWD/desktop/src" --doc docs/testing/module-desktop.md
# -> files with uncovered lines=40 uncovered lines=71
#    mentioned in module-desktop.md: 71 ; MISSING: 0 ; no STALE ; exit 0
```

Segment outcome: **close to fully classified.** 71 lines remain: 47 are
waiver-tagged in §5 with a mechanism, and **24 are unclassified work**, listed per
line in §6. The tripwire proves all 71 are *named*; naming is not a disposition,
so `residual-classified` is still short of complete — but the unclassified set has
gone 94 → 65 → 48 → 39 → 24 across segments 4–8.

The baseline was re-measured in this worktree before any edit and reproduced the
campaign's numbers exactly (292 / 69 / 12 800 / 97.7188), so the deltas above are
the work and not a change of tooling.

The campaign-1 `63.28 % → 66.64 %` table (8977-line denominator, a
files-a-test-touched measurement) is superseded by the table above; its
denominator counted only executable lines the v8 run happened to see, which is
not the 口径 A metric.

The campaign-1 `63.28 % → 66.64 %` table (8977-line denominator, a
files-a-test-touched measurement) is superseded by the table above; its
denominator counted only executable lines the v8 run happened to see, which is
not the 口径 A metric.

## 3. Files changed

Test-only changes plus the measurement config and this record. No production
*behaviour* was modified in any segment (no guard deleted, no `cfg(test)` around
production branches, no assertion weakened); the record's single production edit
is a **comment** correction in segment 12 (§8.17), which changes no code path.
Round-2 segment:

| file | change |
|---|---|
| `desktop/src/lib/editableTarget.test.ts` | new (6 tests) |
| `desktop/src/features/agent/messages/compactionMarkers.test.ts` | new (6 tests) |
| `desktop/src/components/layout/RailNewChatButton.test.tsx` | new (3 tests) |
| `desktop/src/components/layout/hooks/useRemoteDoneBell.local.test.ts` | new (3 tests; the local `agent_end` half, isolated from the process-wide event bus) |
| `desktop/src/components/layout/hooks/useAppSettings.sandboxFallback.test.tsx` | new (2 tests; the suite's `useSandboxAvailability` mock pins the probe *available*, so the unavailable path needs its own file) |
| `desktop/src/features/filepreview/imageZoom.test.ts` | `clampScale(-Infinity)` |
| `desktop/src/features/agent/questionNav.test.ts` | a non-reply row is stepped over |
| `desktop/src/features/agent/mentionMarkdown.test.ts` | overlapping hits emitted once |
| `desktop/src/features/agent/liveStreamTick.test.ts` | a request queued across an out-of-view run |
| `desktop/src/features/remote-peer/mergeConversations.test.ts` | `compareSource` cases |
| `desktop/src/integrations/agent/agentStatus.test.ts` | `subscribeShutdownProgress` subscribe + degraded no-op |
| `desktop/src/components/layout/hooks/useRailSelection.test.ts` | remote rows: tick off, batch by machine, pinned host excluded |
| `desktop/src/components/layout/hooks/useAgentStatus.test.ts` | deferred commit abandoned after unmount |
| `desktop/src/components/layout/hooks/useRemoteDoneBell.test.ts` | non-event / session-less pushes |
| `desktop/src/components/layout/hooks/useAppSettings.test.tsx` | explicit `auto` pick downgraded; one downgrade write under StrictMode |
| `desktop/src/components/layout/DeviceSelector.test.tsx` | every entry applies its filter, manage hand-off, outside-click dismiss |
| `desktop/src/components/layout/ActivityRail.search.test.tsx` | the clear button empties the filter |
| `desktop/src/features/settings/UpdateRecoveryPanel.test.tsx` | a failed restart is reported and retryable |
| `desktop/src/components/layout/OnboardingGate.test.tsx` | **comment only** — its "applies only once" case was documented as reaching the in-flight guard, which React's disabled-prop suppression makes impossible (see §8.5) |

Tests added, by file: `editableTarget.test.ts` 6, `compactionMarkers.test.ts` 6,
`RailNewChatButton.test.tsx` 3, `useRemoteDoneBell.local.test.ts` 3,
`useAppSettings.sandboxFallback.test.tsx` 2, `mergeConversations.test.ts` 3,
`DeviceSelector.test.tsx` 3, `agentStatus.test.ts` 2, `useRailSelection.test.ts` 2,
`useAppSettings.test.tsx` 2, and one each in `imageZoom`, `questionNav`,
`mentionMarkdown`, `liveStreamTick`, `useAgentStatus`, `useRemoteDoneBell`,
`ActivityRail.search`, `UpdateRecoveryPanel`.

### Segment 2 (same day)

| file | change | tests |
|---|---|---|
| `desktop/src/features/filepreview/ImagePreview.test.tsx` | new | 11 |
| `desktop/src/features/remote-peer/remotePromptOutbox.test.ts` | storage round-trip, schema refusal, per-side clear, continuation recovery | 6 |
| `desktop/src/components/layout/hooks/useRemotePeers.test.ts` | failing reads, catalogue backstop, launch-pass guards, malformed pushes, activity scope | 17 |
| `desktop/src/features/filepreview/ImagePreview.tsx` | **production**: the measure effect re-runs once the container exists, and `ResizeObserver` is optional (§8.9) | — |
| `desktop/src/components/layout/hooks/useRemotePeers.test.ts` (cont.) | the misleading heartbeat test rewritten (§7) | — |

Suite total 4189 → 4234 tests (+45), 310 → 311 files, all green; every added test
asserts an effect (no screen-only or call-without-assertion tests).

### Segment 3 (same day)

| file | change | tests |
|---|---|---|
| `desktop/src/features/remote-peer/useRemoteTimeline.test.ts` | guards: no conversation to refresh, the host's `isCompacting`, a typeless frame, an unlisten after unmount, an unsupported host's state read, a short tail, malformed payloads, a re-encoded body, the record superseding a rebuilt reply, a stale read's failure | 11 |
| `desktop/src/features/remote-peer/remoteRunStreams.test.ts` | no-run/no-index frames, a settle for an untracked run, a complete read with nothing indexed, a settled run with nothing renderable | 4 |
| `desktop/src/components/layout/ActivityRail.search.test.tsx` | a workspace's name answering the search (group and row), a remote row's workspace name, a workspace row counting as a result | 3 |
| `desktop/src/components/layout/ActivityRail.remote.test.tsx` | folding a remote fork open and closed, the remote-workspace rename hand-off, the device picker's way out to the machines screen | 3 |
| `desktop/src/features/settings/UpdatePage.test.tsx` | install failure with a failed recovery read, a restart-safe repair, confirming a repair (accepted and refused), the manual-download link | 4 |
| `desktop/src/features/remote-peer/remotePeerClient.test.ts` | pair / connect / disconnect / unpair / rename / preview / receipt commands and their argument shapes | 6 |
| `desktop/src/features/remote-peer/PeerIconPicker.test.tsx` | new: one radio per icon, the current one marked, a pick reported by id, an unknown id marking nothing | 3 |
| `desktop/src/features/remote-peer/remoteComposerDraft.test.ts` | new (file existed empty): per host+conversation drafts, another version discarded, a truncated write discarded | 3 |
| `desktop/src/integrations/tauri/useMaintenanceStatus.test.ts` | new: what the host said, a failed read reported, no poll on top of an in-flight read | 3 |

Suite total 4234 → 4273 tests (+39), 311 → 314 files, all green.

### Segment 4 (same day)

| file | change | tests |
|---|---|---|
| `desktop/src/features/remote-peer/useRemoteApprovals.test.ts` | five wrong-shaped request payloads, a settlement for a card not held, a decision with no host, a subscription resolving after unmount | +11 |
| `desktop/src/features/agent/Composer.tools.test.tsx` | the attachment that justified the send was removed while the recommender was asked | +1 |
| `desktop/src/features/agent/Composer.skillRecommend.test.tsx` | an install that finished after the conversation was left (+ the `settle()` helper) | +1 |
| `desktop/src/components/layout/AppShell.test.tsx` | a local row sharing a selection with a host's, a host refusing one row of a batch (and the rest still deleting), the catalogue title of an open conversation, the id fallback | +4 |
| `desktop/src/features/remote-peer/RemoteComposer.test.tsx` | a recoverable failure queues and says so, a refusal does not, no recovery against a disconnected host, a recovery whose lazy import lands after unmount | +4 |
| `desktop/src/components/layout/MergedRows.test.tsx` | activating a host's row in selection mode ticks it, an empty chat section says so, the rename hand-off's three fields (+ the harness forwarding `onOpenRemote`/`onRenameRemoteWorkspace`) | +3 |

Suite total 4273 → 4294 tests (+21), 314 files, all green.

### Segment 5 (same day)

| file | change | tests |
|---|---|---|
| `desktop/src/features/remote-peer/RemotePeersView.test.tsx` | a peer list that cannot be read, a failed connect, a failed disconnect, a reconnect-all the shell could not carry out | +4 |
| `desktop/src/features/remote-peer/RemotePeerSettings.test.tsx` | the icon hand-off (the `PeerIconPicker` mock now forwards `onChange`, the idiom the file's `RemoteModelsPanel` mock already used) and the two remaining automation switches, each by its own key | +3 |

Suite total 4294 → 4300 tests (+6), 314 files, all green. No production file was
touched this segment — the six lines closed were all reachable through the
existing seams.

### Segment 6 (same day)

| file | change | tests |
|---|---|---|
| `desktop/src/features/agent/composer/useComposerAttachments.test.ts` | **new**: a paste and a file-picker attach while the composer is disabled, and two mid-copy supersession cases (the file after the break is never read; the temporary copy the leaving draft wrote is reclaimed) | 4 |
| `desktop/src/features/agent/useQuestionNav.test.ts` | a press with no transcript mounted (rows before container), a `loadOlder` that throws synchronously, and a reader inside a row taller than the viewport | 3 |

Suite total 4300 → 4307 tests (+7), 314 → 315 files, all green.

### Segment 7 (same day)

| file | change | tests |
|---|---|---|
| `desktop/src/features/tasks/TasksView.test.tsx` | the yearly editor's **month** and **time** (each a control with its own writer), an emptied month field read as January (the `\|\| 1` clamp), the model-without-level refusal branch, and an unparsable one-off date shown as stored | +4 |
| `desktop/src/features/agent/MentionEditor.menus.test.tsx` | the **session** menu's ↓ walk and forward wrap (only ↑ was covered), plus the third schema version of the same walk | +1 |

Suite total 4307 → 4312 tests (+5), 315 files, all green. No production file was
touched — all seven lines closed were reachable through existing seams.

### Segment 8 (same day)

| file | change | tests |
|---|---|---|
| `desktop/src/features/terminal/TerminalView.lifecycle.test.tsx` | the **dequeued-timer race** for both timers: a resize debounce and a retry backoff whose callbacks fire after the view unmounted (injected by making `clearTimeout` a no-op for one turn) | +2 |
| `desktop/src/features/remote-peer/useRemoteSkillRecommend.test.tsx` | the **host's invalidation lane** mocked (it was not, which is why the invalidator never fired): a settings change re-reads the toggle, another domain does not; the same for the skills catalogue; and a **weak test fixed** — the "unavailable toggle" case was answered from the previous render's cache | +2 |
| `desktop/src/features/remote-peer/RemoteConversationSettings.test.tsx` | the hook's **second** subscription closed after unmount (the file's single `releaseListen` only ever released the last-held one), the models-domain invalidation, and both pickers' outside-click dismiss plus the unlisted-model row | +3 |

Suite total 4312 → 4320 tests (+8), 315 files, all green. No production file was
touched — all 13 lines closed were reachable through existing seams.

### Segment 9 (same day)

The classification round: the last 24 unclassified lines, 10 closed and 14
waived. No production file was touched.

| file | change | tests |
|---|---|---|
| `desktop/src/features/agent/AgentActivityList.test.tsx` | line 271 — a burst's command that lands **after the set of calls changed**: the answers are keyed by the set, and the stale answer replaces the whole map, so a test asserts the superseded one is dropped (the current ask's answers are what reaches the rows) | +1 |
| `desktop/src/features/remote-peer/RemoteWorkspaceRenameDialog.test.tsx` | **new file** — the dialog had none. Covers 40 (an *emptied* name sends nothing; the shared shell's Save is closed on `submitting`, **not** on an empty value, so this is a real click — the comment above `submit` claimed otherwise, §8.15) and 51 (the host's refusal stays in the dialog with the text intact) | +3 |
| `desktop/src/features/agent/useSkillRecommendation.test.ts` | 341 (the card is not published for a draft the reader left during the `record` round trip — lifetime injection) and 366 (a source whose day read rejects still leaves submit working) | +2 |
| `desktop/src/features/agent/runtime/useThreadCompaction.test.ts` | line 72 — an acknowledgement that lands after the request was cancelled is discarded | +1 |
| `desktop/src/features/runs/RunsPanel.test.tsx` | line 90 — a **second end-scroll in the same batch**, with the window already at the last page, returns the same state object so React bails out; asserted through the rows (every run once, nothing past the end) | +1 |
| `desktop/src/features/remote-peer/RemoteConversationView.test.tsx` | line 544 — the only transcript shape with **two adjacent non-user rows**: a reply rebuilt from its run's log appended after the previous turn's reply. The walk must pass the first reply to reach the prompt of the run it belongs to | +1 |

Suite total 4320 → 4331 tests (+11), 316 files, all green. The 14 tags added are
D41–D54 in §5 — note that `D41` is no longer a waiver: segment 13 covered the line
(§8.18), and §8.11/§5 record `D25`'s move to OPEN.

### Segment 12 (same day) — the rework after the independent review

No coverage change and no test change: this segment acts on the first independent
verdict (REV-DESK, `review-desktop.md` §1) and on the product question that
verdict's first finding exposes.

| file | change | tests |
|---|---|---|
| `docs/testing/module-desktop.md` | **re-registration**: `D25`'s 4 lines (`ImagePreview.tsx:200,201,205,206`) leave the waiver table and become an **OPEN — not waived** entry with a truthful mechanism (§5); the arithmetic drops 61 waived → 57 waived + 4 OPEN in §2, §5 and §6 | — |
| `docs/testing/module-desktop.md` | §8.11 corrected — the lines are reached by an ordinary Space press (the old "unreachable-by-construction" framing was false); what is missing is a *consumer*, and the docblock's "hold space and drag" is not implemented. The product question is written up for the module owner there | — |
| `desktop/src/features/remote-peer/RemoteWorkspaceRenameDialog.tsx` | **production, comment only** — the third clause of the comment above `submit` ("the Save button is closed for it as well") was false (§8.15): the shared shell closes Save on `submitting` alone, and the file's own test asserts `save().disabled === false` with a blank value. Reworded to what the code does; no executable line changed | — |

Suite total unchanged at 316 files / 4331 tests, all green; segment 12's handoff
runs `tsc --noEmit`, `eslint` on the touched paths and the desktop suite together,
because a green vitest run strips types (`.future/cov100/PLAN.md` §5.1).

### Segment 13 (same day) — `D41` covered, the environment waiver withdrawn

One line of coverage. No production file was touched in this segment either: the
line became reachable by driving the component, not by changing it.

| file | change | tests |
|---|---|---|
| `desktop/src/features/settings/useFutureLoginFlow.staleTick.test.ts` | **new** — the poll callback's null-`start` guard (`useFutureLoginFlow.ts:104`) reached by the commit → passive-flush window. Own harness: `createRoot` without `act()`, fake-timers limited to `setInterval`/`clearInterval`/`Date`, real event-loop turns; the test asserts the window was open (committed null-`start` render with the interval still installed) *and* that the stale tick is a no-op | +1 |
| `docs/testing/module-desktop.md` | `D41` leaves §5's waiver table (and with it the module's only `unreachable-in-this-environment` category) and is recorded in §8.18 with the mechanism, the measurement and the counterfactual; §2/§5/§6 arithmetic updated to 60 lines, 56 waived + 4 OPEN, 34 files; §8.19 records the measurement trap met while re-measuring | — |

Suite total 316 / 4331 → **317 files / 4332 tests**, all green. Measured:
`files_with_uncovered=34 uncovered_lines=60 measured_lines=12800 lines%=99.5312`,
with `VITEST_RC=0 PY_RC=0`; `useFutureLoginFlow.ts:104` goes from `DA:0` to
`DA:5`, in three full runs that agree on every figure.

### Segment 14 (same day) — `D54` covered, the second falsified absolute

One line of coverage, and again **no production file was touched**: the line
became reachable by driving the hook, not by changing it. This is the rework the
review asked for in `review-desktop.md` §19.1 ("either cover the line with a test
in this shape or restate the invariant as a race") — the record takes the first
route, so at the end of this segment `useThreadMessages.ts` has **no uncovered
line at all**.

| file | change | tests |
|---|---|---|
| `desktop/src/features/agent/useThreadMessages.warmSnapshotTimer.test.tsx` | **new** — the loading-timer callback's warm-snapshot return (`useThreadMessages.ts:375`) reached in the load's own ref-write → passive-cleanup window, with a cold-ref control in the same file, the single-caller indicator assertion, and the three-line `loadThreadHistory` diagnostic that keeps the mocked `recoverFailedRuns` synchronous | +3 |
| `docs/testing/module-desktop.md` | `D54` leaves §5's waiver table (strikethrough entry, tag numbering kept) and is recorded in §8.20 with the mechanism, the test, the counterfactual and the stability runs; §2 gains the `seg 14` column and §2/§5/§6/§9/§10 arithmetic move to 59 lines / 55 waived + 4 OPEN / 33 files | — |

Suite total 317 / 4332 → **318 files / 4335 tests**, all green. Measured:
`files_with_uncovered=33 uncovered_lines=59 measured_lines=12800 lines%=99.5391`,
with `VITEST_RC=0 PY_RC=0`; `useThreadMessages.ts:375` goes from statement count 0
to **1**, its `if` at 374 from `[0,2]` to `[1,3]` (the guard's true arm 0 → 1, the
cold arm 2 → 3 — the control's own extra fire), and the report's
uncovered set is **segment 13's minus exactly that one line**, diffed file by
file, in three full runs that agree on every figure. In the single-file inner
loop (`--coverage.include` on the hook) the same readout is `374` `[1,1]`,
`375` = 1 — both arms from this file alone. PLAN §5.1 gates in the same
pass: `npx tsc --noEmit` exit **0**, `npx eslint "src/**/*.{ts,tsx}"` exit **0**
(0 errors; 1 pre-existing warning in `features/remote-peer/RemoteFilesPanel.test.tsx`,
untouched by this segment).

## 4. Dimension evidence

Rows are ready to lift into `docs/testing/dimension-matrix.md`.

| module | dimension | evidence (file :: test) |
|---|---|---|
| desktop | boundary | `useContextData.test.tsx` :: empty run list / no thread / rails-ready gate; `useRightPanelWidth.test.tsx` :: 700 px window (centre floor wins), 1600 px window (max clamp), fractional drag position rounding, `left: undefined` DOMRect; `useUnreadThreads.test.tsx` :: first-seen-already-finished, `cancelled` is not "finished", `undefined` map entries; `useHasProviders.test.tsx` :: catalogues × 5 session statuses table |
| desktop | error-path | `providers.test.ts` :: rejecting `list_agent_providers` keeps the previous cache, rejected balance/profile fetches are not cached, `invokeCommand` normalises a string rejection; `useContextData.test.tsx` :: failing fetch blanks the panel, `ensureWorkspaceGit` failure is tolerated (git is optional); `useUpdateChecker.test.tsx` :: failed check stays silent; `useRemoteStatus.test.tsx` :: failed poll keeps the last status |
| desktop | concurrency | `useHasProviders.test.tsx` :: a superseded reload's late response is dropped (cancellation); `useContextData.test.tsx` :: thread switch mid-flight discards the old thread's rows, a refresh whose `ensureWorkspaceGit` lands after a newer refresh refetches nothing, a cancelled switch's spinner timer cannot blank the thread that replaced it (fault-injected `clearTimeout`), unmount stops polling; `usePendingApprovalCounts.test.tsx` / `useRemoteStatus.test.tsx` :: poll stops on unmount, push listener detached |
| desktop | property | `useHasProviders.test.tsx` :: `showGate === initialLoading \|\| !hasProviders \|\| initPending \|\| forceOnboarding` over 4 catalogues × 5 session statuses; `useContextData.test.tsx` :: poll cadence invariant (fast while a run is live, slow once settled, none while the panel is closed); `providers.test.ts` :: every mutation command forwards its argument shape verbatim |
| desktop | platform-cfg | `N/A` for these files — they contain no platform branches. Windows/macOS/Linux differences in this module live in `src/features/terminal/*` (covered by the existing `macInput`/`keyPolicy`/`shortcut` tests) and in the Tauri backend (measured separately as `desktop-tauri`). |
| desktop | serialization | `providers.test.ts` :: every `ProvidersView`/`FutureAuthState`/`FutureBalance`/`FutureProfile`/`FutureLoginStart`/`FutureLoginPoll` payload is round-tripped through the invoke boundary unchanged (camelCase, `null` vs absent for `apiKey`/`profile`/`downloadUrl`); `useRemoteStatus.test.tsx` :: a structurally identical JSON payload is not re-rendered while a changed one is (the `JSON.stringify` equality gate); `usePendingApprovalCounts.test.tsx` :: structurally equal row sets are dropped by `isEqual`, a same-length different-id set is not; `useUnreadThreads.test.tsx` :: the sessionStorage set survives a JSON round-trip and a corrupt payload degrades to empty |

Round-2 rows added by this segment:

| module | dimension | evidence (file :: test) |
|---|---|---|
| desktop | boundary | `editableTarget.test.ts` :: an element that reports `isContentEditable` only as a property (no attribute) is an editor; a `data-empty-editor` marker hands the arrows back; `imageZoom.test.ts` :: `clampScale(-Infinity)` lands on the floor; `DeviceSelector.test.tsx` :: the three entries in order (all / this computer / host) each apply their own filter; `mentionMarkdown.test.ts` :: a file mention whose path *contains* a session reference is emitted once, with no stray text segment |
| desktop | error-path | `UpdateRecoveryPanel.test.tsx` :: a refused restart is reported in the alert and leaves the button retryable; `agentStatus.test.ts` :: a listener that cannot be registered degrades to a removable no-op; `useRemoteDoneBell.test.ts` :: a push that is not an `event`, or whose payload is not an object, or names no session, rings nothing |
| desktop | concurrency | `liveStreamTick.test.ts` :: a request queued while the view went inactive is not stranded (it runs after the interval); `useAgentStatus.test.ts` :: a deferred commit is abandoned after unmount *and* schedules no further poll (fault-injected `clearTimeout`); `useRailSelection.test.ts` :: a second activation on a ticked remote row unticks it |
| desktop | serialization | `agentStatus.test.ts` :: the `ShutdownProgress` payload is forwarded to the subscriber unchanged |
| desktop | platform-cfg | `N/A` for the new files — the fallback arms they cover are not OS-conditional. |

Rows added by segment 2:

| module | dimension | evidence (file :: test) |
|---|---|---|
| desktop | boundary | `ImagePreview.test.tsx` :: an 800×600 image in a 400×300 overlay reports 100 % at fit, 200 % at "actual size", and a pan stops at the 50 px overhang (the geometry the defect in §8.9 erased); `remotePromptOutbox.test.ts` :: a snapshot written by another schema version and a truncated one both read as "nothing queued"; `useRemotePeers.test.ts` :: a sessions snapshot of unreadable rows yields an empty catalogue, not a stale one |
| desktop | error-path | `remotePromptOutbox.test.ts` :: a corrupt/absent entry, a refused host and a failed recovery all leave the queue consistent, and a failed restart's continuation stays queued for the next attempt; `useRemotePeers.test.ts` :: one unreachable host drops out of both the list read and the catalogue backstop without blanking its siblings, and a failed peer-list read is reported while the last rows stay; `ImagePreview.test.tsx` :: an unreadable image and a rejected read both route to `onError` |
| desktop | concurrency | `useRemotePeers.test.ts` :: the launch pass is abandoned when the view unmounts before the first read answers, runs once per session despite a re-enable, and a subscription whose unlisten arrives after unmount is released; `remotePromptOutbox.test.ts` :: recovery takes the host's receipt instead of continuing the run a second time |
| desktop | property | `useRemotePeers.test.ts` :: 14 type-hostile pushes (a `null` payload, a string where a list belongs, rows that are not objects) each leave the catalogue untouched **and** leave the listener able to apply the next well-formed snapshot — the property being "an unreadable push is not fatal", checked by the follow-up push rather than by "does not throw" |
| desktop | serialization | `remotePromptOutbox.test.ts` :: the queued prompt round-trips through `localStorage` byte-for-byte, and the `version` field is what decides whether a stored entry may be replayed |

Rows added by segment 3:

| module | dimension | evidence (file :: test) |
|---|---|---|
| desktop | boundary | `remoteRunStreams.test.ts` :: a frame that names no run or carries no index reports `untracked` and leaves no projection or cursor behind — yet the next well-formed frame is still the run's beginning; `ActivityRail.search.test.tsx` :: a *workspace name* match keeps its heading and its rows, and counts as a result; `remoteComposerDraft.test.ts` :: a draft written by another version, and a truncated one, both read as "no draft" |
| desktop | error-path | `useRemoteTimeline.test.ts` :: a stale read's `Unsupported command` answer does not mark a capable host unsupported; a short tail is refused rather than folded in; a payload of the wrong shape (a bare string, a number where a record belongs, a non-date timestamp, invalid JSON) is dropped without taking the push lane down; `UpdatePage.test.tsx` :: an install failure whose recovery read also failed reports *both*; a refused repair acknowledgement stays visible and retryable; `useMaintenanceStatus.test.ts` :: a host that cannot be asked is reported rather than looking like "nothing in progress"; `PeerIconPicker.test.tsx` :: an icon id the build does not know marks nothing |
| desktop | concurrency | `useRemoteTimeline.test.ts` :: a rebuilt reply survives the run's end and steps aside only once the page carries the record; an unmount with the subscription in flight releases it; `useMaintenanceStatus.test.ts` :: the 3 s poll does not start on top of an in-flight read, and reads again once it settles |
| desktop | serialization | `remotePeerClient.test.ts` :: every pairing command's *name* and id shape (`remote_peer_pair`/`connect`/`disconnect`/`unpair`/`set_label`/`preview_file`), a partial label patch sends only the fields set, and a prompt receipt is usable only when it names session *and* run |
| desktop | platform-cfg | `N/A` for the new files. |

Rows added by segment 4:

| module | dimension | evidence (file :: test) |
|---|---|---|
| desktop | boundary | `RemoteComposer.test.tsx` :: a prompt sent while the link is down is *queued*, not reported as a failure, and its text stays in the box; `MergedRows.test.tsx` :: an empty chat section says "No chats" rather than drawing a gap, and a row whose thread is missing renders nothing; `Composer.tools.test.tsx` :: an attachment removed while the recommender was pending leaves nothing to send, so nothing is sent |
| desktop | error-path | `useRemoteApprovals.test.ts` :: five wrong-shaped request payloads each produce no card *and* leave the listener working (the next well-formed request lands); a settlement for a card this window does not hold is a no-op; `RemoteComposer.test.tsx` :: a host that *refuses* (a non-recoverable error) shows the reason instead of the queued notice; `AppShell.test.tsx` :: a host that refuses one row of a batch still gets the rest deleted, and the toast names the row and the reason |
| desktop | concurrency | `Composer.skillRecommend.test.tsx` :: an install that finishes after the user left the conversation neither appends `/skill` to the new draft nor sends it; `RemoteComposer.test.tsx` :: a recovery whose lazy `import()` lands after unmount queues nothing (driven with a sync `act` render, because the window is one microtask); `MergedRows.test.tsx` :: activating a host's row in selection mode ticks it instead of opening it |
| desktop | serialization | `AppShell.test.tsx` :: an open conversation is titled from the host's catalogue (and falls back to the session id, never to a blank header); `MergedRows.test.tsx` :: the rename hand-off carries the machine, the workspace id and the *heading's* current name |
| desktop | platform-cfg | `N/A` for the segment-4 files. |

Rows added by segment 5–6:

| module | dimension | evidence (file :: test) |
|---|---|---|
| desktop | error-path | `RemotePeersView.test.tsx` :: a peer list that cannot be read ("keystore unavailable"), a failed connect, a failed disconnect, and a reconnect-all the shell rejected — each reported rather than leaving a row that looks unchanged; `RemotePeerSettings.test.tsx` :: the two remaining automation switches, each asserted to write **only its own key** |
| desktop | boundary | `useQuestionNav.test.ts` :: the reader inside a row taller than the viewport (no row's middle on screen) still gets the reply below it as the next stop; a press with **no transcript mounted** asks for the page and stops looking for rows |
| desktop | concurrency | `useComposerAttachments.test.ts` :: a multi-file paste whose draft is left mid-copy stops at the break (the files after it are never read) **and** reclaims the temporary copy it had already written; `useQuestionNav.test.ts` :: a `loadOlder` that throws synchronously releases the in-flight press, so the next press is not swallowed |
| desktop | serialization | `RemotePeerSettings.test.tsx` :: the picked icon is written back as an **icon id** through the caller's label patch, not as a glyph and not to the host |

Rows added by segment 13:

| module | dimension | evidence (file :: test) |
|---|---|---|
| desktop | concurrency | `useFutureLoginFlow.staleTick.test.ts` :: the **commit → passive-effect-flush window** — after `cancel()` nulls `start`, the interval installed by the previous render is still live and its tick calls the *new* callback; the guard must make that tick a no-op (phase stays `idle`, no poll issued) rather than dereferencing the null it was handed. Driven without `act()` (which is what closes this window), with the window itself asserted open before its effect is read, and with the reserved-slot gate deliberately held open so the guard under test is the only one in the path (verified: the same test fails against a copy with the guard removed) |

## 5. Waivers

Round 2 waives **55 lines in 32 files**, each with the invariant that makes every
input avoid it; the residual's other **4 lines, in 1 file, are OPEN — not waived**
(under the OPEN heading below). Together they are the module's entire 59-line
residual. Three mechanisms are in play: a guard whose condition is
*already entailed* by a sibling condition in the same render; a `undefined`
fallback TypeScript needs and the runtime cannot produce; and shared helper code
whose only call site already validates the value. Every entry names its
falsification route (what a reviewer would have to find to break it). Rejected
candidates are listed under the table.

Categories: **55 lines `unreachable-by-construction`**. The module's single
`unreachable-in-this-environment` line (`D41`) is no longer in this table: the
independent review showed the claim was about the *harness*, not the machine, and
segment 13 covered it (§8.18) — so that category now has no member here. The
other **4 lines are OPEN** and are counted as waived nowhere.

**These tags have been adversarially reviewed** — the unit's own two passes stood
behind all 61 lines of the segment-9 residual, with four mechanisms replaced or
corrected and one new trap found (§10). The independent review then **falsified
three** of them: `D25`, whose 4 lines are re-registered below as OPEN rather than
waived; `D41`, which segment 13 **covered** (§8.18); and `D54`, which segment 14
**covered** (§8.20). The last two are gone from the table rather than
re-registered — a covered line has no category to argue — and all three outcomes
are recorded in §8.11/§8.18/§8.20 and `review-desktop.md` §32. So 55 lines are
waived and **pending a verdict, per line**: see `review-desktop.md` §32 for the
review's own final accounting (25 tags checked across its two passes, 10 of them
enumeration-only) and §9.1 below for the supervisor's correction of the earlier
"44 rows unreviewed" figure, which was written before the review's later segments
landed. Read §10 alongside the table: for four tags the reason stated *here* is
not the reason that actually holds.

| tag | path:line | category | mechanism (the invariant that makes it unreachable) |
|---|---|---|---|
| D1 | `src/components/layout/ActivityRailMenus.tsx:267` | unreachable-by-construction | `handleMenuKeyDown` is wired to exactly three panels (`ThreadItemMenu`, `WorkspaceHeaderMenu`, `ChatSectionMenu`) and each renders at least one unconditional `ThreadMenuItem`; `ThreadMenuItem` never sets `disabled`, so the `[role=menuitem]:not(:disabled)` list is never empty. |
| D2 | `src/components/layout/OnboardingGate.tsx:211` | unreachable-by-construction | the only caller is `onClick={() => void handleStart()}` on a button whose `disabled` prop is `starting \|\| selectedModelId == null`; while `starting` is true the button is prop-disabled and react-dom does not deliver the click at all — proved with a probe (§8.5), including with the DOM attribute removed behind React's back. |
| D3 | `src/components/layout/MergedRows.tsx:441` | unreachable-by-construction | the sole call site is inside `workspaces.map(...)`, and a local workspace group (`desktopId === null`) returns one branch earlier unless `workspaceId === null`; but `groupMergedRows` only sets `workspaceId: null` for *chat* groups, which are not in `workspaces`, and a local workspace row's `workspaceId` is a required `string`. |
| D4 | `src/components/layout/hooks/useAppSettings.ts:137` | unreachable-by-construction | the effect's own write (`changeSettings({approvalTier: "sandbox"})`) optimistically sets the very value `autoFallbackRequired` is derived from, so the requirement is false in the next render — which resets `autoFallbackRef` (line 134) before any dependency can re-run the effect. |
| D5 | `src/components/layout/hooks/useAppSettings.ts:148` | unreachable-by-construction | same shape as D4: `shouldPersistSandboxFallback` requires `approvalTier === "sandbox"`, and the effect's own `changeSettings({approvalTier: "manual"})` clears that before the effect can run again. |
| D6 | `src/features/agent/NewConversation.tsx:149` | unreachable-by-construction | `activeWorkspace = workspaceOptions.find(...) ?? workspaceOptions[0]`, so `!activeWorkspace` implies `workspaceOptions[0]` is falsy — i.e. `!first` — and line 146 returns first. |
| D7 | `src/features/agent/threadMessageCache.ts:48` | unreachable-by-construction | the loop condition is `snapshots.size > MAX_CACHED_THREADS`, so the map is non-empty and `keys().next().value` is a `string`; the `undefined` arm exists only for the `string \| undefined` type. |
| D8 | `src/features/filetree/useFileTree.ts:47` | unreachable-by-construction | `stateFor` is only reached through `root ? stateFor(root) : null`, so every cache key is a non-empty string and `!oldest` is never true for a map of size > 20. |
| D9 | `src/features/markdown/streamingMarkdownBlocks.ts:95` | unreachable-by-construction | the tree comes from `unified().use(remarkParse)...parse(text)`, which attaches `position` to every node; the guard above it already rejects `children.length < 2`, so `children[1]` exists and has a position. |
| D10 | `src/features/settings/AccountPage.tsx:60` | unreachable-by-construction | that button's `disabled` is `balanceStatus !== "unavailable" && !platformUrl` *and* its `onClick` calls `onRefreshBalance` when `balanceStatus === "unavailable"`, so `handleRecharge` can only run with a truthy `platformUrl` from the same render. |
| D11 | `src/features/settings/CommunityEditionSection.tsx:27` | unreachable-by-construction | the switch button is `disabled={selected === communityEdition \|\| saving}` — the same comparison the guard tests. |
| D12 | `src/features/settings/EnvironmentPage.tsx:45` | unreachable-by-construction | the submit button is `disabled={!changed \|\| busy}`; `changed` is computed from the same render's `value`/`activeId`. |
| D13 | `src/features/terminal/tabs.ts:160` | unreachable-by-construction | the loop bound is `used.size + 1` over a set of size `used.size`; by pigeonhole one index in `1..size+1` is absent, so the loop always returns and the fallback is TypeScript's control-flow tail. |
| D14 | `src/components/layout/hooks/useRemotePeers.ts:331` | unreachable-by-construction | `readNumber`'s `typeof value !== "object"` guard has three call sites (374, 393, 397) and all three pass `catalogVersion`, which `handlePresence` has already verified is a non-null object (363–364) — the guard is a type-level tail, and the sibling file's copy of the same helper is the one that needs it, because there it is called with an unvalidated payload. |
| D15 | `src/features/remote-peer/useRemoteTimeline.ts:380` | unreachable-by-construction | the post-`await` re-check of `isCurrent()` in `readRunLog`. The predicate has two halves, and neither can change between `fetchRunReplay` returning and this line: `streamsRef.current` is assigned once per mount (306–308) and never reassigned, and `epochRef` is bumped only at the *entry* of `readPage` — whose every caller (the mount effect, the resume/settle push handlers, `loadOlder`, `refresh`) runs in a macrotask, so no bump can be initiated inside the microtask drain that resumes this continuation. `readRunReplay` itself checks the same predicate after every await and before returning (`packages/thread-projection/src/replay.ts:280,297`), so a lane superseded *during* the read is refused there and lands in the `catch` (line 401, covered). Falsify by finding a `readPage` caller that runs inside a microtask. |
| D16 | `src/features/remote-peer/useRemoteTimeline.ts:799` | unreachable-by-construction | `settleRunsInPage`'s null-streams guard. Its only caller passes `streamsRef.current` (485); that ref is created during the first render (306–308) and never set back to null, and `readPage` — the only function that reaches line 485 — is called from effects and handlers, i.e. after that first render. |
| D17 | `src/features/remote-peer/useRemoteTimeline.ts:844` | unreachable-by-construction | `readRaw`'s `typeof value !== "object"` guard: its only call site (692) passes the push payload, which line 667 has already proven to be a non-null object (a `readString(payload, "type")` that returned a non-empty string). |
| D18 | `src/features/remote-peer/useRemoteTimeline.ts:854` | unreachable-by-construction | `readRaw`'s `JSON.stringify` catch. The value it serialises is `payload.data` of an envelope that arrived through the event bridge's JSON deserialisation, which cannot produce a cycle, a `BigInt`, or a function-valued property — the three ways `JSON.stringify` throws. The sibling branches around it *are* covered (a string body, an absent body, a decoded object body). |
| D19 | `src/features/remote-peer/useRemoteTimeline.ts:957` | unreachable-by-construction | `readJson`'s object guard, for the same reason as D17: its only call site is 923, reached through the same validated payload. (`readJson`'s own `catch` at 965 *is* reachable and is covered, by a `data` that is not JSON.) |
| D20 | `src/components/layout/ActivityRail.tsx:838,843,1117,1118,1124` | unreachable-by-construction | five lines inside the two `featureItems.map(...)` blocks. `featureItems` is a module-level constant initialised to the empty array (146) and referenced only four times (146, 834, 837, 1116) with no push/spread/assignment anywhere in the tree, so both maps have zero iterations for every possible render. |
| D21 | `src/components/layout/ActivityRail.tsx:1182` | unreachable-by-construction | `sortThreads`'s status comparison. Its sole call site (333) is `sortThreads(threads.filter(thread => thread.status === "active"))`, so both arguments to the comparator are always `"active"`. Fragile by nature — a second caller would break it — which is recorded here deliberately rather than left as silent debt. |
| D22 | `src/features/settings/UpdatePage.tsx:76` | unreachable-by-construction | `handleInstall`'s `!status?.hasUpdate` guard. The only caller is the install button, rendered inside the `status.hasUpdate ? … : <p>up to date</p>` branch of the same render, and React swaps that branch out (and with it the handler) as soon as `hasUpdate` goes false. |
| D23 | `src/features/settings/UpdatePage.tsx:137` | unreachable-by-construction | `handleRepairAcknowledgement`'s `!recovery` guard. Its only caller is the "Confirm completed manual repair" button, whose render condition `needsManualRepair` (41) is `Boolean(recovery && …)` — so the same render that binds the handler cannot have a falsy `recovery`. |
| D24 | `src/features/settings/UpdatePage.tsx:150` | unreachable-by-construction | `handleManualDownload`'s `!status?.downloadUrl` guard; the download link that calls it is rendered under `status.downloadUrl` in the same render (261). |
| D26 | `src/features/agent/Composer.tsx:203` | unreachable-by-construction | `handleContextToolSelect`'s three conditions. `toolId !== "compact"`: its only caller is the editor's context-tool menu, fed by `contextTools`, which is a literal array whose single element has `id: "compact"` (191–201) and which is built nowhere else. `!onCompactContext` and `compactionPending`: the same memo returns `[]` unless `onCompactContext` is set and nothing is compacting, so no option exists to select. Falsify by adding a second context tool (the guard is then live code that must stay). |
| D27 | `src/features/agent/Composer.tsx:483` | unreachable-by-construction | `installRecommendedSkill`'s `!reco?.card \|\| installingSkill`. Its only caller is the card's install button, rendered under `skillRecommendation?.card ? … : null` (537–541) and `disabled={installing}` (SkillRecommendCard), and react-dom does not dispatch a click to a prop-disabled element (§8.5). |
| D28 | `src/features/agent/Composer.tsx:516` | unreachable-by-construction | `dismissRecommendedSkill`'s `!skillRecommendation?.card`. Two callers, both inside a card: the card's dismiss button (same render condition, `disabled` while installing) and `submitValue`'s `if (reco.card && !cardHandledRef.current)` immediately above the call. |
| D29 | `src/features/agent/Composer.tsx:427` | unreachable-by-construction | `dropSubmittedDraft`'s three conditions, for a *synchronous* call: `captureOperation()` was taken a few lines above (416) and nothing can bump the lifetime without a render; the earlier `if (!editorRef.current \|\| …) return` (404) already proved the editor exists; and `draftKeyRef` was read into `submittedDraftKey` three lines above (410) with no await in between. |
| D30 | `src/features/agent/Composer.tsx:409` | unreachable-by-construction | `sendNow(evaluatedText)`'s stale-text guard. That form is only reached from the recommendation chain (`submitValue`'s `.then`/`.catch`), and the editor is `disabled={disabled \|\| recommendPending}` (617) for exactly the window in which that chain can run — so the text cannot drift while `isCurrent()` still holds (a draft-key change flips the lifetime and is refused one line earlier). Falsify by finding a way to change the editor's content between the submit and the evaluation resolving. Note a jsdom test *can* reach it by writing `textContent` on the disabled editor; such a test pins an impossible state, and would be an assertion-free result in this campaign's sense, which is why it is not written. |
| D31 | `src/features/remote-peer/useRemoteApprovals.ts:155` | unreachable-by-construction | `readJson`'s object guard: its only call site (68) passes the push payload, which line 64 has already proven a non-null object (`readString(payload, "sessionId")` returned a non-empty string). Same shape as D17/D19. |
| D32 | `src/components/layout/AppShell.tsx:927` | unreachable-by-construction | `openRemoteConversation`'s `conversation.desktopId === null`. It is passed as `onOpenRemoteConversation` into the merged list, whose `renderRow` branches local rows (`desktopId === null`) off to `renderLocalRow` *before* the remote row component that owns `onOpen` (MergedRows.tsx 200–207) — so the callback only ever receives a host's row. |
| D33 | `src/components/layout/AppShell.tsx:778` | unreachable-by-construction | `handlePreviewLeftPanel`'s `if (showLeftPanel) return`. Its three callers are the hover targets rendered under `!showLeftPanel` in the same render (1086, 1095, 1096) — with the panel pinned there is no overlay to hover. |
| D34 | `src/features/remote-peer/RemotePeersView.tsx:146` | unreachable-by-construction | `handlePair`'s `if (!link) return`. One caller, the pair button, whose `disabled` is `!invitation.trim() \|\| busy !== null` (205) — the identical expression, and `invitation` is the state `link` is derived from in the same render. React does not dispatch a click to a prop-disabled element (§8.5). The pre-existing test "will not pair an empty link" asserts the *disabled* state, which is the same mechanism stated from the other side. |
| D35 | `src/features/agent/useQuestionNav.ts:202` | unreachable-by-construction | `landingOffset`'s `!container`. Its only two callers check the same ref in the *same synchronous tick* before calling it: `step` reads `scrollRef.current` at 224 and calls it in a loop, and `navigate` checks at 272 and calls it at 278. Nothing between those points yields, so a non-null container at the caller's check is still non-null here. (The sibling guard at 149 is **not** waived: `pageUp` reads the rows before it looks at the container, which is why the unmounted-press test above reaches it.) |
| D36 | `src/features/agent/useQuestionNav.ts:280` | unreachable-by-construction | `navigate`'s `offset === null`. With the container non-null (checked three lines above), `landingOffset` returns null only when `rows()[target.index]` is undefined — and `target` came from `step(direction)`, whose loop returns a candidate *only* when `landingOffset(candidate, measured) !== null` for its own `rows()` call in the same tick. Both reads see the same DOM, so the index is in range and the offset is a number. Falsify by finding a way for the row count to shrink between the two calls. |
| D37 | `src/features/agent/MentionEditor.tsx:175` | unreachable-by-construction | the mutation-observer effect's `!editor`. The editor element is rendered **unconditionally** (`contentEditable={!disabled}` toggles, the element never does), so its ref is assigned during the commit phase and the effect runs after it — the same reasoning as D16's `streamsRef`. This is deliberately *not* the shape of §8.9's `ImagePreview` bug, where the container was rendered behind a loading branch; here there is no branch to be in. |
| D38 | `src/features/agent/MentionEditor.tsx:412` | unreachable-by-construction | `insertSession`'s `!editor \|\| !context`. Two callers, both reachable only with a session context live in the same tick: line 600 (the key path, under `sessionMenuOpen`, which *is* `hashQuery !== null && sessions.length > 0`, computed by the same `hashContext` from the same DOM) and line 710 (the menu's `onSelect`, rendered only when `sessionMenuOpen`). Falsify by finding a caller outside that menu. |
| D39 | `src/features/terminal/TerminalView.tsx:68` | unreachable-by-construction | the mount effect's `!container`. The `<div ref={containerRef}>` is rendered **unconditionally** (l.417–424: no branch, no early return), so the ref is assigned during the commit phase and the effect runs after it — the D16/D37 shape. Note this is the *opposite* finding to campaign 1's premise for this file: the xterm renderer is already replaced by a hand-written stub (`TerminalView.lifecycle.test.tsx:81`, with `open`/`write`/`onData`/`onResize`/`fit`/`serialize` all faked), so "jsdom has no renderer" was never what made these lines unattractive. |
| D40 | `src/features/terminal/TerminalView.tsx:250` | unreachable-by-construction | `open()`'s `disposed`. Its two call sites are the effect body (l.383, where `disposed` was just initialised to `false` and nothing has awaited) and the retry timer (l.236) — and that one is **guarded by the same check at line 233**, which segment 8 now covers by injecting the dequeued-timer race. So `open()` cannot be entered with `disposed` true. Falsify by finding a third caller. |

Segment 9 added 14 more, and closed the campaign's list of *unclassified* lines
(the residual is now fully tagged). **One of them, `D41`, is no longer a waiver:
the independent review falsified its category and segment 13 covered the line
(§8.18) — it is listed here in strikethrough form only so the tag numbering stays
legible:**

| tag | path:line | category | mechanism (the invariant that makes it unreachable) |
|---|---|---|---|
| ~~D41~~ | `src/features/settings/useFutureLoginFlow.ts:104` | ~~unreachable-in-this-environment~~ **COVERED (segment 13)** | the poll callback's `!current`. The window is real: `usePolling` updates `callbackRef.current` during render but clears the interval in a passive effect cleanup, so between the commit that nulls `start` and React's passive flush the old interval still ticks the new callback. The category was wrong because the *harness*, not the machine, hid it: `act()` flushes the cleanup before any timer can fire, while an un-`act()` render — what the app itself does — leaves the window open on this machine. Now reached by `useFutureLoginFlow.staleTick.test.ts` (§8.18). |
| D42 | `src/features/agent/runtime/useThreadCompaction.ts:31` | unreachable-by-construction | the returned callback's `!thread`. Its only caller is the composer's compact affordance, and `AgentThread` passes that affordance only under `thread?.agentSessionId` (l.550–552) — i.e. the guard's own negation. `AgentThread` *can* render with a null thread (`AppShell` keys it `activeThread?.id ?? "__none"`), but then the composer never receives `handleCompactContext` at all. Falsify by giving the composer a second, unguarded path to it. |
| D43 | `src/features/agent/ApprovalPrompt.tsx:97` | unreachable-by-construction | `decide`'s `deciding`. All four call sites are buttons with `disabled={deciding !== null}` (l.265, 276, 290) or an unconditional `void decide(...)` inside them; react-dom does not deliver a click to a prop-disabled element at all (§8.5). Falsify by finding a keyboard or programmatic path that bypasses `disabled`. |
| D44 | `src/features/agent/ApprovalPrompt.tsx:143` | unreachable-by-construction | `confirmCapabilityRules`' `deciding \|\| !saveSuggestion?.rules`. The single call site (l.277) reads `saveSuggestion.rules ? void confirmCapabilityRules() : openRuleEditor()` — the `rules` half is the ternary's own condition — and its button carries `disabled={deciding !== null \|\| malformedCapability}`. So both disjuncts are entailed by the caller. Falsify by finding a caller outside that button. |
| D45 | `src/features/remote-peer/RemoteConversationView.tsx:540` | unreachable-by-construction | `forkAt`'s `!message \|\| message.role === "user"`. The single call site is `onFork` on a row of the same `rendered` array (l.441, closing over that array and that index), and `MessageBlock` renders the fork affordance **only inside its `!isUser` branch** (MessageBlock.tsx:289–302). So the message exists and is not a user row. Note the divider shape is *not* the route: a compaction-only message early-returns as a bare divider with no affordance at all (MessageBlock.tsx:144). Falsify by rendering the affordance for a user row. |
| D46 | `src/features/skills/SkillsView.tsx:217` | unreachable-by-construction | `runUninstall`'s `!skill`. Both call sites (l.317 installed tab, l.337 all tab) pass an id the row got from the same render's lists, and the uninstall affordance itself is gated by `installedById`, which is built from `displayedInstalled` (l.99–101) — the very list `find` searches. So any id that can be passed is in the list. Falsify by adding a caller whose id comes from a different source than `displayedInstalled`. |
| D47 | `src/features/skills/SkillsView.tsx:249` | unreachable-by-construction | `upgradeAll`'s `skillUpgrades.length === 0`. Its only caller is the "upgrade all" button, `disabled={upgradeCount === 0 \|\| anyBusy}` where `upgradeCount` *is* `skillUpgrades.length` (l.315, 438). Falsify by finding a caller not behind that button. |
| D48 | `src/features/runs/RunsPanel.tsx:155` | unreachable-by-construction | `archiveFinished`'s `archiving \|\| finishedCount === 0`. Its only caller is the archive button, `disabled={archiving \|\| finishedCount === 0}` (l.182) — the same two expressions. Falsify by finding a second caller. |
| D49 | `src/features/artifacts/ArtifactsPanel.tsx:58` | unreachable-by-construction | `handleUpload`'s `uploading`. Its only caller is the upload button, `disabled={uploading}` (l.101), with the label itself switching on `uploading`. Falsify by finding a second caller. |
| D50 | `src/features/artifacts/ArtifactsPanel.tsx:166` | unreachable-by-construction | `handleAttach`'s `!artifact.path`. The one menu item that calls it is spread in under the *same* predicate: `...(artifact.path ? [{ label: t("menu.attach"), onSelect: () => handleAttach() }] : [])` (l.173–174). Falsify by finding a caller outside that menu. |
| D51 | `src/features/filetree/FileTreePanel.tsx:96` | unreachable-by-construction | `handleOpenFile`'s `entry.isDir`. Its only invocation path is `FileTreeNode`'s own click handler, which branches on the very same flag itself — directories go to `tree.toggle(entry.path)`, files to `onOpenFile?.(entry)` (FileTreeNode.tsx:51–57); the recursive render at l.140 only forwards the same prop down. **This corrects §6 of the previous round**, which listed 96 as "reachable with a directory row": the row never reaches the handler. Falsify by adding a second call site or a context-menu action that opens a directory. |
| D52 | `src/features/terminal/TerminalPanel.tsx:238` | unreachable-by-construction | `ErrorBar`'s `!error`. The component is rendered only as `{tabs.createError ? <ErrorBar tabs={tabs} /> : null}` (l.137–138), and `error` is that same `tabs.createError` read one line earlier in the same render pass — the same object, no await between. Falsify by rendering it unconditionally, or by making `createError` clear between the parent's test and the child's read. |
| D53 | `src/features/artifacts/PdfPreview.tsx:102` | unreachable-by-construction | the supersede check's `!container`. `container` is a `const` local read from `containerRef.current` at l.76 and proven non-null at l.77–78; the only awaits before l.101 are `getPage`, whose result is checked for `cancelled` at l.86 — and between that check and l.101 there is no await and no assignment to `cancelled`, so the second disjunct is already known false. **This corrects the previous round's "attempt a real jsdom render before waiving"**: the renderer is already fully faked in `PdfPreview.test.tsx` (`getDocument`/`getPage`/`render` with deferreds), so no attempt was withheld — the guard simply reduces to `cancelled` tested twice. Falsify by inserting an await or a ref re-read between l.86 and l.101. |
| ~~D54~~ | `src/features/agent/useThreadMessages.ts:375` | ~~unreachable-by-construction~~ **COVERED (segment 14)** | the loading-timer callback's `hasWarmSnapshotRef`. The claimed invariant was an **absolute about a commit**: "the commit that warms the ref always flips `loadingThread` to false, the indicator effect re-runs and *clears this timer* before it can fire". Line 252 writes the ref **synchronously** inside the load's own continuation and line 256 only *queues* the commit that makes `loadingThread` false; the cancellation lives in that commit's **passive effect cleanup** (deps `[loadingThread]`, l.401). Between the two the timer is installed *and* the ref is warm, and a callback firing there takes the return at 375 — the same shape as `D41`. Falsified by the independent review (A/B probe: `375` = 0 with the snapshot cold, = 1 with it warm inside the timer's due window; `review-desktop.md` §19.1, reproduced in §27 with a witness assertion), and reached in the shipped suite by `useThreadMessages.warmSnapshotTimer.test.ts` (§8.20). |

### OPEN — not waived

Ledger rule (`waiver-ledger.md`): a file listed under an `OPEN` / `not waived`
heading counts as **not waived**, even if its path also appears in a ledger table.
The row below is therefore **excluded from the 56-line waiver count** and from
every arithmetic statement in this record.

| tag | path:line | status | why it is not waived, and what would close it |
|---|---|---|---|
| `D25` | `src/features/filepreview/ImagePreview.tsx:200,201,205,206` | **OPEN — not waived** (registered as `unreachable-by-construction` until segment 12) | The category was falsified by the independent review (REV-DESK, `review-desktop.md` §1 `D25`), and verified independently by the supervisor's own run of that probe: one Space `keydown` + one `keyup` takes all four lines from `DA:0` to `DA:1`, because the effect at 198–215 registers both window listeners unconditionally. The truthful mechanism is therefore *not* unreachability but the absence of a **consumer**: `spacePressedRef` is written at 201 and 206 and read **only** by the re-entry guard on line 200 (`!spacePressedRef.current`), so no observable behaviour depends on it and no assertion-bearing test can pin it. **The resolution is a product decision (§8.11)**: wire the docblock's "hold space and drag to pan" affordance (then the lines have a real test), or have the module owner confirm the affordance is dead (then the state — and with it the lines — goes). Neither may be done unilaterally, and deleting the state *to make the lines disappear* is the forbidden inverse of the waiver policy. |

**The same review's second finding, `D41`, was resolved by covering the line
(segment 13).** It is recorded here because the *dispute* was real and its
resolution says something about environment categories: this record had objected
that the reviewer's counterexample — a render outside `act()` whose effect cleanup
is deferred past a fired timer — might be "a state the app cannot be in", the rule
§5 applies to `Composer.tsx:409`. The objection does not survive contact with the
mechanism (§8.18): the app itself renders outside `act()`, `act()` is the harness
that *hides* the window, and the window is therefore not a manufactured state but
the ordinary one — which is exactly why an `unreachable-in-this-environment` tag
was the wrong instrument. The line is covered by an assertion-bearing test and is
no longer in the residual at all.

The invariant behind D2/D10/D11/D12 is one shared mechanism worth naming once:
**a handler guard duplicated by the `disabled` prop of its only caller is dead
code, because react-dom does not dispatch to a prop-disabled element** (§8.5).
Four reviewers' counterexamples therefore need a *second* caller for those
handlers, which does not exist in the tree (`grep` for `handleStart`,
`handleRecharge`, `handleSwitch` shows one each).

Rejected waiver candidates (checked, and coverable, so **not** in the ledger):

| candidate | verdict |
|---|---|
| `useAgentStatus.ts` deferred-commit guard, `liveStreamTick.ts` settle re-check, `AgentActivityList` cancellation arm | all three reachable by fault injection or an explicit interleaving; covered with real assertions instead |
| `useAppSettings.ts` 137/148 as "flaky-guard" | rejected as a *reason*: the guard is dead for the reason in D4/D5, not because the test is hard |
| `remoteRunStreams.ts` 123 (`untracked` verdict) and 206 (a settle for an unknown run) | both **covered**, not waived: `apply` is a public method whose input is parsed by hand upstream, so a frame with an empty `runId` or a null `idx` is expressible, and the assertions are the reported verdict plus the run's *absence* (no projection, no cursor) and that the next well-formed frame is still the run's beginning |
| `useRemoteTimeline.ts` 380/799/844/854/957 | waived (D15–D19) only after the call-site invariants above were written out; the sibling copies of the same helpers in `useRemotePeers.ts`/`useRemoteApprovals.ts` are *not* waived where their payload is unvalidated |
| `ActivityRail.tsx` 1182 | waived (D21) rather than covered: the only way to cover it is to change the call site or export the comparator, and neither is a coverage change |
| `PdfPreview.tsx` (pdf.js canvas) and `TerminalView.tsx` (xterm renderer) | **not waived**: the campaign-1 reasoning (jsdom has no canvas) was never proven here, and both still have uncovered lines. They stay in §6 as work. |
| `Composer.tsx` 409 as "covered by a jsdom test that writes into the locked editor" | rejected: the state is one the app cannot be in (the editor is `disabled` for that window, §8.9's lesson applied the other way) — a test that forces it pins nothing a user can reach |
| `AppShell.tsx` 107/109 as "unreachable, the row already carries a title" | rejected: **covered**, by a test that opens a conversation whose catalogue entry has a host-side title — which is also how a conversation opened from ids alone gets a title |
| `MergedRows.tsx` 235/236 as "the checkbox covers ticking" | rejected: **covered** — activating the row itself is a second, distinct way into the same rule |
| `RemotePeersView.tsx` 89/105/120 as "unreachable, the host answers" | rejected: **covered** — all three are `setError` arms the mocks reach by rejecting; only 146 (the pair guard) is entailed |
| `RemotePeerSettings.tsx` 227/258/266 as "the panel is mocked out" | rejected: **covered** — the `PeerIconPicker` mock now forwards its `onChange` (the same idiom the file's `RemoteModelsPanel` mock already used), and the two switches are clicked by their labels |
| `useQuestionNav.ts` 149 and 202 as the same "null ref" waiver | **split, and the split is the point**: 202 is waived (D35 — its callers check the ref in the same tick), while 149 is **covered** because `pageUp` reads the rows *before* it checks the container, so an unmounted press reaches it. Two guards that look identical in the source have different answers. |
| `useComposerAttachments.ts` 80/113/129/143/169 as "only reachable through a disabled editor" | rejected: **all five covered** at the hook, where `disabled` and the operation lifetime are the caller's own arguments — and the interesting assertion is not "it returns" but that a superseded copy *reclaims the temporary file it wrote* and that the files after the break are never read at all. |
| `MentionEditor.tsx` 587–589 as "the menu's arrows are covered" | rejected: **covered** — the ›↓ branch of the *session* menu was the one unexercised half (the suite only pressed ↑), so "the menu responds to the arrows" was a claim about half the control. The new test also pins the forward wrap. |
| `TasksView.tsx` 674/690 as "the yearly editor is covered by the day round-trip" | rejected: **covered** — month and time are separate controls with their own writers; a form that displays a schedule it cannot change is worse than one that omits it. The emptied-month case pins the `\|\| 1` clamp. |
| `TasksView.tsx` 269 as "the refusal test covers it" | rejected: **covered** — that test leaves model *and* level unchosen, so it stops at the model branch; the level branch needs a model with no level, which the new test produces by clearing the level after choosing a model. |
| `TerminalView.tsx` as "xterm cannot run under jsdom" (campaign 1's stated reason for leaving 35 lines there) | rejected on inspection: the suite **already stubs the renderer** (`TerminalView.lifecycle.test.tsx:81`). Three of the four lines closed in segment 8 with an ordinary test; the remaining two are entailed guards (D39–D40), not renderer gaps. |
| `useRemoteSkillRecommend.test.tsx`'s "reads an unavailable toggle as off" as a passed test of the failed read | it was **passing without exercising the failure**: the previous render's resolved read was still in the host-catalogue cache, so the rejected mock was never reached (both outcomes are `false`). Fixed by dropping the cache and asserting the command was re-issued — which is what closed line 32. |
| `RemoteConversationSettings.test.tsx`'s "closes a subscription that arrives after it unmounted" as covering both subscriptions | it does not: this hook registers **two** listeners and the file's single `releaseListen` releases only whichever was held last, so the other's `unlisten` path never ran. The mock now records every resolver and a sibling test drains them all. |

## 6. Residual, per line

**59 lines remain: 55 are the waivers recorded in §5 (tagged `D1`–`D54` minus the
covered `D41` and `D54`), and 4 are the OPEN lines of §5's `D25` entry.** There is
no line left without a disposition — the metric the objective uses (`DA:0 = 0`) is
therefore not met, and the waiver arm is not complete either: the module's claim is
*covered or waived except 4 lines awaiting a product decision*.

Per-line list from the segment-14 run's coverage report (command in §1). The
tripwire in §2 checks this table against the same report: **0 MISSING, 0 STALE** —
it is also what caught `D41` and `D54` leaving: `useFutureLoginFlow.ts` and
`useThreadMessages.ts` no longer appear here at all, because segments 13 and 14
covered their one line each (§8.18, §8.20).

| file | uncovered lines | lines |
|---|---|---|
| `components/layout/ActivityRail.tsx` | 6 | 838 (D20), 843 (D20), 1117 (D20), 1118 (D20), 1124 (D20), 1182 (D21) |
| `features/agent/Composer.tsx` | 5 | 203 (D26), 409 (D30), 427 (D29), 483 (D27), 516 (D28) |
| `features/remote-peer/useRemoteTimeline.ts` | 5 | 380 (D15), 799 (D16), 844 (D17), 854 (D18), 957 (D19) |
| `features/filepreview/ImagePreview.tsx` | 4 | 200 (`D25` **OPEN**), 201 (`D25` **OPEN**), 205 (`D25` **OPEN**), 206 (`D25` **OPEN**) |
| `features/settings/UpdatePage.tsx` | 3 | 76 (D22), 137 (D23), 150 (D24) |
| `components/layout/AppShell.tsx` | 2 | 778 (D33), 927 (D32) |
| `components/layout/hooks/useAppSettings.ts` | 2 | 137 (D4), 148 (D5) |
| `features/agent/ApprovalPrompt.tsx` | 2 | 97 (D43), 143 (D44) |
| `features/agent/MentionEditor.tsx` | 2 | 175 (D37), 412 (D38) |
| `features/agent/useQuestionNav.ts` | 2 | 202 (D35), 280 (D36) |
| `features/artifacts/ArtifactsPanel.tsx` | 2 | 58 (D49), 166 (D50) |
| `features/skills/SkillsView.tsx` | 2 | 217 (D46), 249 (D47) |
| `features/terminal/TerminalView.tsx` | 2 | 68 (D39), 250 (D40) |
| `components/layout/ActivityRailMenus.tsx` | 1 | 267 (D1) |
| `components/layout/MergedRows.tsx` | 1 | 441 (D3) |
| `components/layout/OnboardingGate.tsx` | 1 | 211 (D2) |
| `components/layout/hooks/useRemotePeers.ts` | 1 | 331 (D14) |
| `features/agent/NewConversation.tsx` | 1 | 149 (D6) |
| `features/agent/runtime/useThreadCompaction.ts` | 1 | 31 (D42) |
| `features/agent/threadMessageCache.ts` | 1 | 48 (D7) |
| `features/artifacts/PdfPreview.tsx` | 1 | 102 (D53) |
| `features/filetree/FileTreePanel.tsx` | 1 | 96 (D51) |
| `features/filetree/useFileTree.ts` | 1 | 47 (D8) |
| `features/markdown/streamingMarkdownBlocks.ts` | 1 | 95 (D9) |
| `features/remote-peer/RemoteConversationView.tsx` | 1 | 540 (D45) |
| `features/remote-peer/RemotePeersView.tsx` | 1 | 146 (D34) |
| `features/remote-peer/useRemoteApprovals.ts` | 1 | 155 (D31) |
| `features/runs/RunsPanel.tsx` | 1 | 155 (D48) |
| `features/settings/AccountPage.tsx` | 1 | 60 (D10) |
| `features/settings/CommunityEditionSection.tsx` | 1 | 27 (D11) |
| `features/settings/EnvironmentPage.tsx` | 1 | 45 (D12) |
| `features/terminal/TerminalPanel.tsx` | 1 | 238 (D52) |
| `features/terminal/tabs.ts` | 1 | 160 (D13) |

rows=33 lines=59 waived=55 unclassified=0 open=4

### Unclassified lines: none

There is no unclassified residual left in this module, but not every line is
*waived*: 4 of the 59 are **OPEN — not waived** (§5). Segment 9 took the last 24
and disposed of each: **10 covered** by new assertion-bearing tests and
**14 waiver-tagged** (D41–D54 above; `D41` and `D54` since covered, `D25` since
moved to OPEN) after checking every one against §8.5 and
the two techniques that had closed lines before. Three of the fourteen were
classified only after the *premise of the work item* turned out to be wrong (see
§8.15), which is why the list shrank by classification as much as by coverage.
The three independent findings then split the fourteen: segment 12 moved `D25`'s 4
lines from *waived* to *OPEN* (the category was false and no consumer exists), and
segments 13 and 14 **covered** `D41` and `D54` (the category was false and a
consumer *does* exist — the commit → passive-flush window, §8.18 and §8.20). The
residual is therefore 55 waived +
4 OPEN, and of the 14 tags segment 9 registered, exactly two turned out to be a
line that could simply be covered.

The table above is regenerated from the measurement, not hand-edited: every
uncovered line carries a tag and a disposition (`waived` or `OPEN`), and the
generator refuses to print a table in which any line is untagged.

## 7. Weak tests found and fixed

| finding | action |
|---|---|
| `useRightPanelWidth.test.tsx`, first draft: `const { current } = renderHook(…)` destructures the harness **getter**, freezing the value at the first commit — three assertions were passing without exercising anything (e.g. "ignores a non-primary button" passed because `resizing` was read once as `false`) | rewritten to hold the harness object and read `hook.current` per assertion; the same pattern was avoided in the other new files |
| `useRightPanelWidth.test.tsx` / `useUnreadThreads.test.tsx`, first draft: storage-failure tests asserted only "does not throw" plus a value that holds equally when the spy misses; the `Storage.prototype` spy was in fact not intercepting jsdom's `sessionStorage` (found by making the assertion discriminating: a stored 640 px width survived) | replaced with the throwing-`window.sessionStorage` getter, which makes the "does not throw" claim fail if the `try/catch` is removed |
| `useContextData.test.tsx`, first draft: "blanks the context when a fetch fails" used `mockRejectedValueOnce` on the bootstrap call, but the poll had already bumped the refresh generation, so the *current*-generation branch was never reached | split into a poll-free failure test (covers the blanking arm) and a separate stale-generation test |
| no assertion-free, `it.skip`, `xit` or `describe.skip` tests were added | — |
| `useRemotePeers.test.ts` "does not re-read when the presence revision matches applied revision": pushed the presence **once** and asserted a re-read — which the heal half explains on its own, so the name claimed an invariant the test never exercised (§8.10 explains why the invariant cannot hold as written either) | rewritten to push the same heartbeat twice and assert the reads per tick, with the mechanism named in the test and in §8.10 |
| `OnboardingGate.test.tsx` "applies only once … in flight": its comment claimed the second activation reached the in-flight guard | comment corrected to state what the test pins (React does not deliver the click at all, §8.5); the test itself is unchanged and still pins the prop-disabled invariant |
| `ImagePreview.test.tsx` (new) first draft asserted `translation().x !== 0` after a wheel zoom while the component had never measured its container — the assertion was true of a coordinate space that did not exist | that draft is what exposed the defect; rewritten to assert the geometry (200 % actual size, 50 px overhang) that is only correct once the container is measured |
| `useRemoteTimeline.test.ts` (segment 3), first draft of the stale-read test kept the *same* run in flight for both reads | the test passed for the wrong reason: `readRunLog` refuses a second read of a run already being read (`replayingRef`), so the "fresh" read never happened and the assertion could not distinguish the guard from the dedupe. Rewritten to use two runs, and to assert the reads' `runId`s (`["run_1", "run_2"]`) so the interleaving is what the test says it is |
| `ActivityRail.search.test.tsx` (segment 3), first draft asserted that a workspace *name* match leaves its rows filtered out | wrong, and the failure was informative: `threadMatchesQuery` matches a local thread by its **workspace's** name too (510–512), so the row is a result and belongs on screen. Rewritten to assert what the rules actually promise (heading kept, rows matching by name or title kept) and that the other workspace is gone |
| `ActivityRail.search.test.tsx` (segment 3), first draft asserted "no matching conversations" is absent when a workspace row matched | wrong element: the *chat section's* empty hint is a different node and legitimately stays. The assertion now names the list's own banner (its first child) rather than a string that two elements share |
| `useMaintenanceStatus.test.ts` (new) first draft did not reset the mocked `invokeCommand` between tests | three tests shared a call count, which made the re-entrancy assertion read as three mount reads; fixed with a `beforeEach(mockReset)` — the same class of mistake as asserting a value that holds either way |
| `Composer.skillRecommend.test.tsx` (segment 4), first draft baked the recommended card into the props from the first render | the test then sent on its *first* submit and failed for the wrong reason: with a card already up, the first submit takes the card's "send without it" path. The app never reaches that state (the card only exists after `onEvaluate`), which is what `mountCardOwner` exists for. Rewritten onto a harness that owns the card, like the real screens |
| `RemoteComposer.test.tsx` (segment 4), first draft tried to reach the abandoned lazy import through the suite's `mount()` helper | unreachable that way: the helper awaits a flush, by which time the `import()` has already landed and the guard cannot fire. The test now drives a sync `act` render plus unmount with no flush in between — the actual one-microtask window — and asserts nothing was queued |
| `MergedRows.test.tsx` (segment 4), first draft looked for the row as `button` text | the row's activation surface is `RailRow`, whose `title` is `"conversation · machine"` — found by `title` prefix instead; the same mistake would have made the test "pass" on a row that never got clicked |
| `useRemoteApprovals.test.ts` (segment 4), first draft's `ignores a request with …` cases asserted only the absence of a card | strengthened to also push a well-formed request afterwards and assert it lands: a hook that threw or wedged on the bad frame now fails instead of looking clean |
| `useComposerAttachments.test.ts` (new, segment 6), first draft asserted that a superseded paste neither copies nor *classifies* | wrong about the code and weaker than the truth: the loop stops (so the later files are never read) but the file already written is still classified, and `addAttachmentPaths` then **deletes** it because the draft is gone — which is the property worth pinning. Assertions corrected to the copy count and the reclaimed temp file |
| `useComposerAttachments.test.ts` (new), first draft typed the hoisted mocks without generics | `tsc` rejected the argument-taking ones (`Expected 0 arguments`); the file's mocks now carry `vi.fn<…>` signatures like the other suites, so an arity mistake fails the type check rather than the mock silently ignoring the argument |
| `useQuestionNav.test.ts` (segment 6), first draft of the throwing-`loadOlder` case asserted a *second press* immediately after the first | it failed, and correctly: the failed press still arms the bounded frame loop (waiting for rows that never come), which refuses a second press until the loop ends. The test now lets the loop finish and *then* presses — which is what actually isolates `pagePendingRef` being released |
| `TasksView.test.tsx` (segment 7), first draft of the unparsable-date case asserted the whole row text contains `someday 09:00` | too weak to catch the interesting half: the empty-date row must also produce no stray separator, so the assertion now covers both rows and adds a "no `undefined`, no double space" leg — the kind of thing a `String(undefined)` slip would produce |
| `MentionEditor.menus.test.tsx` (segment 7), first draft of the ↓ walk pressed twice and asserted `1` | passes even if the wrap is broken (it never leaves the middle of the list); rewritten to walk past the last row and assert the wrap to `0`, which is the boundary the ↑ test already pins in the other direction |
| `useRemoteApprovals`/`useRemoteTimeline` read-helper guards | checked case by case rather than waived as a group: `useRemotePeers.readNumber` and `useRemoteTimeline.readRaw`/`readJson` have validated call sites (D14/D17/D19), while `useRemoteTimeline.readString`/`readNumber` and `useRemoteApprovals`'s copies are called with unvalidated payloads and are covered by tests (§6) |
| `RemoteWorkspaceRenameDialog.test.tsx` (new, segment 9): the first draft's "empty name" case asserted only that no error appeared | too weak to catch the finding and wrong about the mechanism: the point is that the shared shell's Save is **enabled** for an empty value, so the click really happens — the test now asserts `save().disabled === false` first, then that no command was sent, nothing closed and nothing toasted. Without the first assertion the case would have passed on a button that could not be clicked at all |
| `RemoteConversationView.test.tsx` (segment 9), first draft tried to reach the adjacent-non-user-row shape with a **compaction marker** | the divider row renders as a bare `CompactionDivider` and carries no fork affordance, so the test found one button instead of two and failed — informatively: it is what established that the marker shape is *not* the route (D45) and that the rebuilt-reply shape is (544 closed). The draft was replaced rather than loosened |
| `useSkillRecommendation.test.ts` (segment 9), the day-read test first asserted only "does not throw" | a resolving value that holds either way: the assertion is now on the **resolution** (`resolves.toBeNull()`), which fails if the outer fence is removed because the hook would then resolve to the candidate |
| `zz-probe-approval.test.tsx` (segment 10, **deleted after use**) | a probe, not a test: it drove `ApprovalPrompt`'s two decision paths and printed the counts. It was removed rather than kept because a kept probe asserts nothing — the finding it produced is §8.16 and the attribute it established is in §10's D43 row |

No snapshot-only tests were added (none of the new tests use snapshots).

## 8. Bugs / traps found while measuring

1–4. (Campaign 1: the instrumented perf-test timeout, `vi.spyOn(Storage.prototype)`
not intercepting jsdom's `sessionStorage`, `renderHook()`'s `current` getter, and
the `useRemoteStatus` equality gate. All still valid.)

5. **react-dom never dispatches a click to a prop-disabled element — even with
the DOM attribute removed behind React's back.** Measured, not assumed:

   ```
   PROBE afterFirst= 1 disabledProp= true domDisabled= false afterSecond= 1
   ```

   The probe renders `<button disabled={busy} onClick={…}>` (react-dom 19,
development build), clicks once, then `removeAttribute("disabled")` and dispatches
a second `MouseEvent` — the handler is not called. jsdom itself *does* deliver a
dispatched click to a disabled button (`node` probe: `disabled dispatchEvent
listener hits: 1`), so the suppression is React's, based on the prop. Consequence
for this module: **any handler guard duplicated by its only caller's `disabled`
condition is dead code** — that accounts for D2, D10, D11, D12 in §5, and for the
*reason* the pre-existing `OnboardingGate.test.tsx` "applies only once … in
flight" case passes: its second click never reaches the handler (the test's
claim about reaching it was corrected in place, §3).

6. **A host that ignores the approval-tier patch makes `useAppSettings` write in
an unbounded loop.** Repro: mock `updateAppSettings` to return a snapshot that
still says `approvalTier: "auto"` while the session is signed out; the fallback
effect's optimistic `"sandbox"` is overwritten by the response, which re-arms the
requirement, which writes again — 40+ writes inside one `act`, i.e. the test
never settles (found while trying to cover D4/D5). Not reachable with the real
backend (it applies the patch and returns the stored settings), so this is a
robustness note, not a filed bug: the `autoFallbackRef`/`sandboxFallbackRef`
guards do not bound the loop, because the requirement itself flips. No test
asserts this (it would hang); it is recorded here with its repro.

7. **`desktop/coverage/` is wiped by every coverage run** (`reportsDirectory`),
so a log redirected into it disappears. Keep logs in `/tmp`.

8. **The coverage report is keyed by absolute path, and `covlines-js.py` strips
`/Users/geilige/future-os/`** — in a worktree every `--under` must therefore be
`.worktrees/<name>/desktop/src`, or the tool reports `measured_lines=0` and looks
like 100 % (it is a silent zero, not an error).

9. **`ImagePreview` never measured its container, so zoom, double-click and pan
did nothing until a window resize — fixed in this segment.** The measure effect
had `[]` deps and bailed out at `if (!el) return`, and the only time it ran was a
commit in which the overlay rendered **nothing** (the bytes are still being read
and the loading notice has not appeared yet), so `containerRef.current` was
`null` forever after. Consequences: `fit = fitScale({0,0}, natural) = 0`;
`actualScale = 0`, so the inline transform became
`translate(0px, 0px) scale(0/0 = NaN)`, which the browser drops — the picture
never changed size; `clampTranslation(…, imageSize = natural.width * 0 = 0)`
pinned the translation to 0, so panning did nothing; and a double click computed
`target = 1 / fit = Infinity`, clamped to `MAX_SCALE`, so the controls jumped to
600 % while the image stayed put.

   Evidence, on the *unfixed* component (jsdom, container stubbed 400×300, image
800×600):

   ```
   fresh mount, after a double click : 600%   (fit = 0, 1/fit = infinity)
   after a window `resize` event     : 100%   (fit = 0.5; the toggle back from 600%)
   ```

   Fix (two lines of behaviour, in `ImagePreview.tsx`): the effect depends on
   `readySrc`, so it runs on the commit that mounts the container; and
   `ResizeObserver` is now optional (`typeof ResizeObserver === "undefined"`), the
   same guard `useComposerInset` / `useStickyAutoScroll` already use.

   **Why it escaped**: the dead effect is also the effect that would have thrown.
   jsdom has no `ResizeObserver`, so as soon as the effect actually ran,
   `filepreview.test.tsx` failed with `ReferenceError: ResizeObserver is not
defined` in 6 of its 41 tests — the suite was green *because* the measurement
   (and the observer with it) never executed. A green suite here was evidence of
   nothing. Both halves are now covered: the geometry by `ImagePreview.test.tsx`
   (with the two seams stubbed) and the observer-less path by the pre-existing
   file.

10. **Every presence heartbeat costs a peer-list + catalogue read (reported, not
changed).** The host publishes presence every 3 s
(`remote/publisher.rs::presence_tick`). On each tick `useRemotePeers` calls
`ensureGate(desktopId, epoch)` → `gate.authenticate(epoch)`, and `authenticate`
**clears the applied revisions**; the `readNumber`-vs-`gate.revision(domain)`
comparison that follows therefore always sees the `-1` floor, so any positive
revision heals → `void refresh()` → one read per tick per paired host. The
`gate.accept(…, true)` immediately below it, and its comment ("so a stale
heartbeat does not keep re-triggering heals"), cannot have that effect, because
the next tick wipes what it recorded. The intended cadence is the 60 s catalogue
backstop.

   Not changed here: it is a behaviour change in a hook whose push protocol is
   shared with the phone, and this unit's mandate is coverage. The natural fix is
   to authenticate only when the epoch *changes* (`CatalogVersionGate` exposes no
   epoch getter today, so it needs one — and its mobile twin is
   `mobile/src/remote/catalogVersion.ts`). Pinned by a characterisation test
   ("re-reads for a lost snapshot, then again on the next tick") so the fix shows
   up as a deliberate test change rather than silently.

11. **`ImagePreview`'s space-to-pan affordance is documented but not wired, and
the state behind it is read only by its own re-entry guard — a product question
for the module owner, recorded and not fixed.** *(Corrected in segment 12: the
independent review falsified the category this item originally justified,
`review-desktop.md` §1 `D25`.)* What is **not** true is the earlier framing
("unreachable by construction"): an ordinary Space press reaches all four lines —
one `keydown` + one `keyup` takes `ImagePreview.tsx:200,201,205,206` from `DA:0`
to `DA:1`, because the effect at 198–215 registers both window listeners
unconditionally. What **is** true is that the state is inert: `spacePressedRef` is
written at 201 and 206 and read only by the re-entry guard on line 200
(`!spacePressedRef.current`), and `hasPannedRef` (75, 167, 182) is likewise
written and never read. Crucially, the pan handlers (161–194) run on **any** drag
— `handleMouseDown` has no Space check, so the affordance the docblock promises
at line 37 ("drag (or hold space and drag) to pan") behaves *identically* with and
without Space. So either the documented "hold space and drag" affordance was never
wired, or the space tracking is vestigial; `DRAG_THRESHOLD_PX`'s "pan rather than
a click" distinction is likewise not consumed by any behaviour. **This is a
product decision** (the same shape as §8.10), put to the owner: implement the
documented affordance (then these lines get real assertions) or confirm the
affordance is dead (then the state, and with it the lines, goes). Until then, do
neither unilaterally, and note that deleting the state *to make the lines
disappear* is the forbidden inverse of the waiver policy. The four lines are
therefore registered **OPEN — not waived** in §5: reachable by an ordinary Space
press, yet with no consumer a test could assert, so covering them means first
deciding what the affordance should do.

12. **`fetchRemoteSessions` must never be counted across a 60 s fake-timer
advance.** Advancing the clock by `CATALOG_POLL_MS` also fires the 5 s peer-list
poll twelve times, and each of those refreshes reads the catalogue too — a naive
"called twice" assertion sees fourteen calls. Every backstop test in this module
now states which other poll it had to silence (a rejecting list read, or a
narrowed timer advance) and why.

13. **The unit's machine-verify command could not pass, and reported "verify gate
rejected" while the suite was green.** There were two independent faults, both in
the `--verify` string rather than in the work, and both are worth knowing for any
unit that hops directories or runs a slow suite.

    (a) **Path.** The command was `cd desktop && npx vitest … && python3
    ../.future/cov100/covlines-js.py desktop/coverage/coverage-final.json …`.
    Because of the leading `cd desktop`, the python leg runs with `cwd=desktop`, so
    its `desktop/coverage/…` argument resolves to `desktop/desktop/coverage/…` and
    it dies with `FileNotFoundError`. Evidence from the self-diagnosing form:
    `VITEST_RC=0 PY_RC=1`, with a `Traceback … FileNotFoundError: [Errno 2] No such
    file or directory: 'desktop/coverage/coverage-final.json'` beside
    `Test Files 314 passed (314) / Tests 4294 passed (4294)`. Use cwd-relative
    paths for *every* leg, or `cd ..` back.

    (b) **Timeout, and a masked exit code.** A validator is killed after 120 s
    (`orchestration/goal/src/validator.rs`, `DEFAULT_TIMEOUT`) unless
    `FUTURE_GOAL_VALIDATOR_TIMEOUT_SECS` is set, and the full suite takes minutes
    under load — so a full-suite validator *always* dies mid-run, which is itself
    how `coverage-final.json` ends up complete-looking and nearly all-zero (the
    supervisor sampled 312/333 files at zero that way). Separately, `P=$?` after a
    pipe measures the status of the pipeline's **last** command, so `python3 … |
    tail -1; echo $?` reports `tail`'s status and hides the tool's exit 3 on an
    invalid report. The validator is now the metric leg alone (§1), no pipe.

    The two hypotheses that were *ruled out*: a flaky test under load (**this suite
did not flake** — both full runs green at load average 22 with 13 workers, 0
failures, no unhandled errors; worth recording for the flake sweep) and a
missing/zeroed report as the *cause* of the path failure (the path never reached a
file at all).

    Lesson: a verify command must be self-diagnosing (print each leg's exit code),
    cwd-correct for every leg, and cheap enough to finish inside the validator's
    budget — a broken path and a killed run otherwise look exactly like a failing
test.

14. **`reconcile.py` mis-parses an Istanbul report with a relative `--scope`, and
silently reports a clean bill of health.** Its Istanbul keys are the report's
absolute paths and it matches them with `str.startswith(scope)`, so
`--scope desktop/src` matches nothing and prints
`files with uncovered lines=0 uncovered lines=0 / MISSING: 0` and **exits 0**.
That is a false pass in exactly the direction the tool exists to prevent (the
supervisor's own example used a relative prefix). Use an absolute scope:

    ```bash
    python3 .future/cov100/reconcile.py --report desktop/coverage/coverage-final.json \
      --scope "$PWD/desktop/src" --doc docs/testing/module-desktop.md
    ```

    With the absolute scope it reads 44 files / 84 lines for this unit, and 0
    MISSING. Worth reporting upstream: the tool should refuse a scope that matches
    no file rather than reporting zero, since "0 uncovered" is otherwise
    indistinguishable from a fully-covered unit.

15. **Four classifications this segment came from checking the work item's own
    premise, and in three of them the premise was wrong.** They are worth stating
    together because each one would have produced a *plausible but false* waiver
    or an unnecessary fixture hunt:

    * `RemoteWorkspaceRenameDialog.tsx:40` — the source comment above `submit`
      says "the Save button is closed for it as well" (i.e. closed for an empty
      name). It is not: the shared `RenameDialog` shell closes Save on
      `submitting` alone (`EntityDialogs.tsx:53`), and the input's Enter key
      calls `onConfirm()` with no check at all (l.71–76). The line is therefore
      **reachable and covered**, and the comment is wrong — the same shape as
      §8.9, where a comment asserted a guard that did not exist. (Had I taken the
      comment at its word, this would be a D2-shaped waiver.) **Segment 12
      rewrote the comment** (§8.17): the first two clauses stand, the third is
      replaced by what the code does — a stale comment that contradicts its own
      test is worse than no comment, because the next reader re-derives the wrong
      premise, which is exactly how `ImagePreview.tsx`'s stale docblock produced
      the wrong `D25` premise (§8.11).
    * `FileTreePanel.tsx:96` — the previous round's work list said "reachable
      with a directory row". It is not: `FileTreeNode`'s click handler branches
      on `entry.isDir` **itself** (directories toggle, files call
      `onOpenFile?.(entry)`), so the handler can never see a directory. Recorded
      as D51 rather than hunted for a fixture.
    * `PdfPreview.tsx:102` — the work list said "attempt a real jsdom render
      before waiving". There was nothing to attempt: the renderer is already
      fully faked in the file's own test (`getDocument`/`getPage`/`render` with
      deferreds), and the uncovered disjunct `!container` cannot be true because
      `container` is a non-null `const` local and the sibling `cancelled` was
      already tested at l.86 with no await between. D53, with the reason stated.
    * `TerminalView.tsx` (segment 8) — same family: campaign 1 left it as "xterm
      needs a real renderer", but the suite already substitutes a hand-written
      stub, and 2 of its 4 lines closed with ordinary tests.

    The pattern for the next worker: when a work item says a line needs a host,
    a fixture or a renderer, **read the suite first** — this repo's line-level
    leftovers have three times turned out to be a wrong premise rather than a
    missing capability.

16. **Two synthetic keydowns inside one `act` callback both reach the *stale*
    listener — a trap for anyone trying to "cover" a guard this way.** Found
    while attacking D43. `ApprovalPrompt` guards its decisions twice: the buttons
    via `disabled={deciding !== null}` and the window listener via its own
    `if (deciding) return` (163). Driving the real affordance one press at a time
    behaves (measured `onDecision` calls: 1, then still 1 on a second press while
    the first is pending, then 2 once it settles) — but dispatching **two**
    keydowns inside one `act(() => { … })` calls `onDecision` **twice**, with the
    buttons already disabled:

    ```
    PROBE A (two Cmd+Enter in one act batch)  onDecision calls = 2   buttons disabled = [Deny, Allowing]
    PROBE B (two Cmd+Enter in separate acts)  after first = 1, after second = 1, after settling then third = 2
    ```

    The reason is a stale closure, not a missing guard: within one batch React has
    not re-rendered, so both dispatches invoke the *old* listener, which holds the
    *old* `decide` whose `deciding` is `null` — so neither the listener's guard nor
    `decide`'s own guard evaluates true. Two separate batches (what two real
    key presses are: separate macrotasks with a render between them) behave
    correctly, which is PROBE B.
    Consequences, stated honestly: (a) this is **not** filed as an app bug — I could
    not construct a real input sequence that produces it, and the residual
    uncertainty is that a browser delivering two keydowns in one task, or a future
    React batching change, would make it real; (b) it **is** a trap for this
    campaign: a future worker trying to cover `ApprovalPrompt.tsx:97` with a
    two-press test would get a passing test that exercises a stale closure and
    would mistake it for pinning the guard. The invariant the campaign relies on
    here is the *listener's* guard (163, covered — 3 hits), not line 97.

17. **A stale source comment was fixed in place, not merely noted, because it had
    already produced one wrong waiver premise (§8.15's first bullet).** The
    comment above `RemoteWorkspaceRenameDialog.submit` claimed the Save button is
    closed for an empty name; the file's own test asserts
    `save().disabled === false` with a blank value, and the shared shell closes
    Save on `submitting` alone (`EntityDialogs.tsx:53`). Segment 12 rewrote the
    comment to what the code does: the local `!name` check exists so the user sees
    nothing happen instead of the host's error, and it is **not** duplicated by
    the button. This is the second time in this campaign that a stale source
    comment produced a wrong waiver premise — the first was `ImagePreview.tsx`'s
    "Space-bar drag to pan" docblock, which produced `D25` (§8.11) — so the
    working rule for the rest of the campaign is: **when a work item's premise
    rests on a comment rather than on code, fix the comment in the same pass, and
    note it.** (A comment edit is not a production behaviour change; it is
    recorded in §3 for that reason.)

18. **The `D41` waiver was wrong because the *harness*, not the machine, hid the
    window — and the line is now covered.** This is the campaign's second
    falsified environment claim, and the more instructive one: the tag said "needs
    a host/fixture this machine cannot provide", and the machine could provide it
    all along.

    **The mechanism, read out of `lib/usePolling.ts`, not assumed:**
    `callbackRef.current = callback` runs during **render**, while the interval is
    installed in a **passive effect** whose cleanup sets `active = false` and
    clears it. So after a commit that nulls `start` — `cancel()`, or the
    `authorized` arm — the new callback is already in the ref while the old
    interval is still installed and its `tick` still sees `active === true`. A
    tick in that gap calls the null-`start` closure, and `if (!current) return`
    (line 104) is what stops it.

    **Why the category was false:** the window is not exotic, it is the *ordinary*
    non-`act()` ordering, i.e. what the app itself does. `act()` — the harness
    every other test in this module uses — flushes the passive cleanup before it
    returns, so it is `act()` that closes the window. Measured, on the same
    sequence with the same timers:

    ```
    without act():  windowOpened=true   -> the still-installed interval ticks the
                                           null-start closure; phase stays "idle"
    with act():     windowOpened=false  -> act() flushed the cleanup first
    ```

    **The test** (`useFutureLoginFlow.staleTick.test.ts`, beside the hook) mounts
    its own root with `createRoot`, fake-timers *only* `setInterval`/
    `clearInterval`/`Date` so React's scheduling stays real, and drives the loop
    with real `setImmediate` turns. It asserts, in order: the flow is `waiting`
    with a non-null `start`; the clock is then moved past the reserved-slot gate
    (2 s) so that line 104 is the *only* guard between the stale tick and
    `current.deviceCode` — otherwise the back-off gate would return first and the
    test would pass with the guard deleted; the gap was open (a committed
    null-`start` render **and** the faked `clearInterval` count unchanged); the
    stale tick then leaves `phase === "idle"` and `message === null`; and the
    cleanup lands on a later turn.

    **Evidence.** `DA:104` 0 → **5**: the five ticks `advanceTimersByTime(5_000)`
    fires into the still-installed interval (in this file's own run `begin()`'s
    tick is the single hit on line 105, so the two are distinguishable: 5 vs 1).
    Stability, because this is the module's one timing-sensitive test: **26 runs**
    of this file alone, **10 runs** with the whole `features/settings` directory in
    parallel, and **three full coverage runs** were green with `DA:104 = 5` every
    time — and the failure mode is loud, not silent, because the test asserts the
    gap was observed before it reads anything else, so a missed window fails
    rather than passing vacuously.

    **The counterfactual, run rather than argued:** the test was also run against a
    copy of the hook with the guard removed (a scratch file in the same directory,
    deleted afterwards — no production edit, and `git status` shows none):

    ```
    with the guard:     phase "idle",  message null
    without the guard:  phase "error"  -> AssertionError: expected 'error' to be 'idle'
    ```

    That is the path the guard prevents: without it the callback runs past the
    reserved-slot gate, evaluates `current.deviceCode` on `null`, and the
    `TypeError` lands in the poll's `catch` — `attemptRef` was already bumped by
    `cancel()`, but the callback captured it *after* the bump, so the catch's
    staleness check does not swallow it and the dialog flips to `error` with a
    null-dereference message. So the test fails if the line is deleted, which is
    the property that makes this coverage and not merely a touched line.

    **The lesson for the campaign's remaining environment claims:** a category
    about *the machine* must name the *harness* it was ruled out under, because the
    harness is usually the thing that can be changed. `unreachable-in-this-
    environment` is true of a line that no render on this machine reaches — not of
    a line that this module's `act()`-based harness cannot reach.

19. **A concurrent coverage run elsewhere in the checkout can produce a
    *plausible* partial report — not a zeroed one — so the numbers alone do not
    identify it.** Met while re-measuring segment 13: a second full
    `vitest --coverage` run that overlapped another process using the same
    `coverage.reportsDirectory` (`desktop/coverage`) lost its intermediate
    `coverage/.tmp/coverage-*.json` files and reported

    ```
    files_with_uncovered=40 uncovered_lines=71 lines%=99.4453   (VITEST_RC=1)
    ```

    — **71 / 40, i.e. segment 8's state**, a number this record contains and one
    that reads exactly like "the last segment's work was undone". The giveaway is
    not the metric (the tool printed it happily) but the run's exit code, plus
    vitest's own error:

    ```
    Unhandled Rejection  Error: Something removed the coverage directory
    ".../desktop/coverage/.tmp" Vitest created earlier. Make sure you are not
    running multiple Vitests with the same "coverage.reportsDirectory" at the
    same time.
    ```

    The rule extends §1's: **the vitest exit code is part of the measurement**
    (`VITEST_RC=0` *and* `PY_RC=0`), and a coverage number whose run did not exit 0
    is not a number — especially when it happens to match a previous segment. The
    three valid segment-13 runs that followed agree exactly (34 / 60 / 99.5312, with
    line 104 = 5 in every one), which is the check §2 prescribes for a suspect
    delta.

20. **The `D54` waiver was wrong for `D41`'s reason — the harness, not the
    machine, hid the window — and the line is now covered.** The tag claimed an
    *absolute about a commit*: "the commit that warms the ref always flips
    `loadingThread` to false, the indicator effect re-runs and *clears this timer*
    before it can fire". Read against the code, the two halves of that sentence
    happen in **different phases**:

    * `hasWarmSnapshotRef.current = true` (l.252) is a **synchronous ref write**
      inside `reloadThreadHistory`'s own continuation, one microtask after the
      load's last await;
    * `setBlockingLoad(false)` (l.256) only **queues** the render, and the
      cancellation that would clear the timer lives in that commit's **passive
      effect cleanup** (the effect keyed on `loadingThread`, deps `[loadingThread]`
      at l.401).

    So between the ref write and React's commit the timer is still installed
    *and* the ref is already warm. The claim's error is the same one `D41` made
    (item 18): it reasoned about the ordering of *commits* while the code writes a
    ref and merely schedules one — and the independent review measured exactly
    that gap (A/B in the review's own harness: `375` = 0 in the cold control, = 1
    in the attack; `review-desktop.md` §19.1, independently reproduced in §27).

    **The test** (`useThreadMessages.warmSnapshotTimer.test.tsx`, beside the
    hook): mounts with a cold pending load so the indicator effect arms its show
    timer, advances to `DELAY - 1`, resolves the page, drains the microtask chain
    by hand — React's commit for the queued state is a **macrotask**, so no
    microtask drain can reach it — then advances the last millisecond so the timer
    fires with the ref warm. Like the `D41` file it mounts its own root with
    `createRoot` outside `act()` and fake-timers only
    `setTimeout`/`clearTimeout`/`Date`, because `act()` flushes the very cleanup
    the window needs to still be pending.

    **What the assertion is, and why not merely "375 executed".**
    `setLoadingIndicator(true)` has exactly one call site — inside this timer,
    past the guard — so the guard's whole observable effect is that a snapshot
    already warm when the timer fires must not put the indicator on. The test
    therefore asserts the end state (`loadingIndicator` false *after* the queued
    commit lands, with `loadingThread` still true at the moment the timer fired as
    the witness that no render of the completed read had been committed), plus a
    cold-ref control in the same file that takes the other arm. No DOM attribute
    is stripped, no React internal touched, and no production line changed: the
    line was reachable all along through the hook's real call path (`messages`
    ends as `["u1"]`, i.e. the load completed, it did not merely load a ref).

    **Evidence.** In the full run of record, `useThreadMessages.ts:375` 0 → **1**
    statement hit, and its `if` at 374 `[0,2]` → `[1,3]`: the guard's true arm
    goes 0 → 1 (the attack) and the cold arm 2 → 3 (this file's control fires the
    timer one more time past the guard) — i.e. the new file supplies **both** arms
    of the branch, which is what makes the cold control part of the evidence
    rather than decoration. Read on its own (`--coverage.include` on the hook) the
    same file reports `374` = `[1,1]`, `375` = 1. Stability, because this file is
    timing-sensitive in the same way the `D41` file is: **12 consecutive runs** of
    the file alone, **2 runs** of the whole `features/agent` directory in parallel
    (77 files / 1166 tests each), and **three full coverage runs** that agree on
    every figure (33 / 59 / 12800 / 99.5391 %, 318 files / 4335 tests) were green
    — and the report's uncovered set is segment 13's **minus exactly that one
    line**, diffed file by file.

    **The counterfactual, run rather than argued:** the same test was run against
    the hook with lines 374–375 deleted (the working-tree file edited and restored
    byte-for-byte — the pre/post `sha256` agree and `git status` shows no
    modification), which is the honest way to show the assertion has teeth:

    ```
    with the guard:    3 passed
    without the guard: AssertionError: expected true to be false  (at the
                       post-commit `expect(current.loadingIndicator).toBe(false)`)
    ```

    Note *which* assertion fails: the mid-test read still sees `false` (the
    indicator state update has not committed yet), which is exactly why this file
    asserts the settled value and not the transient one. The control and the
    diagnostic tests stay green under the mutant, so the failure is the warm-ref
    window and not the harness.

    **Two harness traps recorded here because both cost a debugging cycle.** (a)
    `recoverFailedRuns` must stay **synchronous** in the mock, as it is in
    `history/runHistoryProjection`: mocked `async`, `loadThreadHistory` returns a
    Promise where it needs an array and swallows the resulting `TypeError` in its
    own `catch`, so the read becomes `{ status: "failed" }`, line 252 is never
    reached, and a `DA:0` read would be meaningless. The file therefore carries a
    three-line diagnostic (`loadThreadHistory` called directly against the same
    mocks → `status === "loaded"` and one message) so that a future edit to the
    mock fails loudly instead of silently disarming the cover. (b) The guarded
    return is only *taken* if the ref write happens before the timer fires; that
    is why the test asserts `failed` (the mocked projection at the end of the
    load) was called before the last millisecond is allowed to pass, and why the
    committed `loadingThread === true` witness is asserted after the fire.

## 9. Next checks

1. **W1 is no longer closed, and it is no longer at `DA:0 = 61`.** The independent
   review (REV-DESK, `review-desktop.md`) falsified three of the registered tags —
   one in each direction its category demanded — and each is now acted on:
   **`D25` → OPEN** (4 lines, a product decision, §5/§8.11), **`D41` →
   covered** (1 line, §8.18) and **`D54` → covered** (1 line, §8.20). So the
   residual is **59 lines: 55 waived + 4 OPEN**.
   Its segment 1 attacked 17 registration rows with executing probes; its
   segment 2 then took the remainder and reports that **at the tag level nothing is
   left unchecked** — all 54 tags (`D1`–`D54`) / 61 lines carry a verdict with the
   check named (§12/§13). The classification is
   complete, self-consistent, and has survived two passes of my own (§10 plus the
   four non-React tags) — and the independent review, which is the reason the
   claim can now be called *reviewed* rather than merely author-classified. What
   the review does **not** claim is uniform evidence strength: by its own final
   accounting (`review-desktop.md` §32) **25 tags carry a verdict across its two
   passes and 10 of them are enumeration-only**, and it names three uncertainties
   it could not close (`D18`'s Rust-side nesting
   limit, `D30`/`D38`'s same-tick premise, `D53`'s dependence on pdf.js's
   synchronous `getViewport`). §10's calibration table is why that distinction is
   stated rather than hidden: a sibling unit's author-classified rows were
   falsified **16 of 24** by an independent judge working with executing inputs,
   while another unit's 13 survived 13/13. So the honest remaining obligation is a
   **stronger-evidence pass over those enumeration-only tags**, not a re-review
   of the whole module, and it is recorded as such in §10's risk order below.
   Supervisor note (closeout): this item used to say "44 of the waived tags have
   never been reviewed", written from segment 1 before segment 2 landed; corrected
   against `review-desktop.md` §12/§13, whose `D1`–`D54` list contains no gap.
   If you are that reviewer: attack in §10's risk order (D15, D35/D36, D10, D53 —
   the timing- and enumeration-dependent tags; `D41` is settled, and its fate is
   the reason to be sceptical of the *other* claims that a harness cannot reach
   something), enumerate call sites yourself with `grep` rather than trusting §5's
   mechanisms, and build counterexamples in a scratch checkout (no writes inside
   this worktree). Leave the mechanically verified tags (D7, D8, D9, D13) and the
   single-caller family until last.
   Do **not** induce a falsifying production change to free a line for coverage —
   that is the forbidden inverse of the waiver policy (a coverage change may only
   ever be a *result*); such changes are what a reviewer looks *for* in the code as
   it stands. Do not re-derive this classification and do not hunt fixtures.
2. Run the per-line tripwire (§2) after every change to this table — it is the
   only check that inspects each uncovered line, and it catches both a line the
   doc forgot and a line the doc still cites after later work covered it. Use an
   **absolute** `--scope` (§8.14).
3. **The remaining 59 lines are one work item per file, not one per line** (33
   files, 20 of them single-line — that figure is recomputed from the segment-14
   report, where segment 13's text said 23 and its own report gave 21). If the
   objective's `DA:0 = 0` is to be
   pursued further, the next move is to attack the *tags* rather than to look for
   more fixtures: pick a tag, try to construct its counterexample, and only then
   either cover the line or restate the invariant. §8.5 and the two techniques in
   §9.1 below are what have kept closing lines; the ones that fall will fall that
   way — and `D41`'s fate (§8.18) and `D54`'s (§8.20) are the standing reminders
   that the *cheapest* counterexample may be to change the harness rather than to
   build a fixture, whenever the waived reason is an absolute about *phases*
   (a commit, an effect, a tick) rather than about a value.
4. Decide §8.10 (a read per presence heartbeat) and §8.11 (the space-to-pan
   affordance) with the module owner; both are product decisions, and **§8.11 is
   now the module's only open item that blocks the objective** — its 4 lines are
   registered OPEN, not waived (§5), so the campaign's claim for this module is
   *covered or waived except 4 lines awaiting a product decision*. The resolution
   is one of the two the owner named: wire the documented "hold space and drag"
   behaviour (then a real test exists), or confirm the affordance is dead (then the
   state goes, with the owner's confirmation — never as a coverage edit).
5. Re-run §1 after each batch and keep `uncovered_lines` from `covlines-js.py` as
   the number of record; the suite is green at 318 files / 4335 tests after
   segment 14, so any new red is that segment's regression. **Read the tool's exit
   code, not just its stdout**: a single-file `npx vitest run <file> --coverage`
   leaves every other file at zero hits, so the tool exits **3** and its
   percentages are meaningless — but the one file you ran is still readable from
   the report, which is what makes it a good inner loop. Only a full run produces
   a number to record, **and the vitest exit code is part of that number** (§8.19:
   a run that collided over `desktop/coverage` reported a plausible 71/40 — a
   previous segment's state — while exiting 1). For the same reason, this suite's
   **stability is worth re-checking** whenever a delta looks surprising: two full
   runs of one code state have agreed exactly so far (§2), so a difference is real.
6. `packages/markdown`, `packages/thread-projection`, `packages/json-preview` are
   measured through this suite only where `desktop/src` imports them; their own
   line coverage belongs to the packages unit.

## 10. Adversarial review of the waivers (segment 10)

Segment 9 left a fully classified residual; segment 10 attacked it. **Result: 0
of the tags *it* attacked fell — 61 lines still stood at the time** — so this
section carries no new coverage. *(Superseded in part by the independent review:
REV-DESK reviewed 25 of the tags across its two passes and three fell — `D25`,
whose 4 lines are re-registered as OPEN in §5; and `D41` and `D54`, whose lines
segments 13 and 14 covered (§8.18/§8.20, both of them tags *this* section had
recorded as SURVIVED — see the corrected row below). Segment 10's own result
against the reasons it attacked stands as stated.)* What it does carry is
*evidence per tag* and four sharper mechanisms,
because three of the four tags named for attack turned out to rest on a reason
weaker than the one that actually holds. A reviewer who attacked the stated
reason would have been right to doubt it and would still have been unable to
build the counterexample.

Method, per tag: read the source, then a mechanical check of the central claim
(`grep` for every caller / every writer of the ref), and — where the claim is
about *behaviour* rather than structure — an executable probe that drives the
real callers and counts the effect. Nothing here is "verified by inspection" in
the hand-waving sense: each row names the check that was run.

| tag | how it was attacked | verdict |
|---|---|---|
| **D15** `useRemoteTimeline.ts:380` | traced every writer of `epochRef` (bumps at 428 `readPage` entry, 592 the reset effect) and the one await inside `readRunReplay`; then tried the sequence that should reach it: a push opens a run-log read, a `RUN_SETTLED` push (`useRemoteTimeline.ts:730`) runs `readPage` while that read is in flight | **SURVIVED, reason replaced** (below) |
| **D21** `ActivityRail.tsx:1182` | `grep -rn sortThreads desktop/src/` → exactly two hits: the definition and one call site (333), whose expression is `threads.filter(thread => thread.status === "active")` | **SURVIVED** (unchanged) |
| **D53** `PdfPreview.tsx:102` | read the whole effect body for any await or any other write to `cancelled` between the check at 86 and the check at 101; the cleanup at 125–130 *does* set `cancelled = true`, so the question is only whether it can run in that window | **SURVIVED, reason refined** (below) |
| **D54** `useThreadMessages.ts:375` | executable probe (§segment 9): the only promising sequence ends with `loadingThread === false`, i.e. through the effect's other branch | **SURVIVED (segment 10) — OVERTURNED: the independent review falsified it (`review-desktop.md` §19.1/§27), and segment 14 covered the line (§8.20)** |
| D43/D44 `ApprovalPrompt.tsx:97,143` | looked for the pattern that has falsified tags before — a caller that bypasses a `disabled` button. Found one that *is* a second caller (`window.addEventListener("keydown")`, 173–176 `/` 179–183) and then tested it with a probe | **SURVIVED, attributed + new trap** (§8.16) |
| D10 `AccountPage.tsx:60` | `grep` for the button's `disabled` — and it is **not** `!platformUrl` | **SURVIVED, reason replaced** (below) |
| D11 `CommunityEditionSection.tsx:27` | button `disabled={selected === communityEdition || saving}` — the guard's own expression | **SURVIVED** (unchanged) |
| D12 `EnvironmentPage.tsx:45` | button `disabled={!changed || busy}` — the guard's own expression | **SURVIVED** (unchanged) |
| D14 `useRemotePeers.ts:331` | checked all three call sites (374, 393, 397): each passes the same `const catalogVersion` that the enclosing function proved to be a non-null object 11 lines earlier (363–364) | **SURVIVED, proof tightened** |
| D22 `UpdatePage.tsx:76` | the install button's `disabled` (244) is `needsRecovery \|\| maintenanceError \|\| needsManualRepair` — *not* `!status?.hasUpdate`. The entailment is the **render condition** at 205 (`{status.hasUpdate ? … : …}`) | **SURVIVED, reason corrected** |
| D42 `useThreadCompaction.ts:31` | `grep` for every caller: one (AgentThread.tsx:171), passed to the composer only under `thread?.agentSessionId` (551) | **SURVIVED** (unchanged) |
| D46/D47 `SkillsView.tsx:217,249` | `grep` for call sites: `runUninstall` two (317, 337) both fed by `installedById`-gated rows; `upgradeAll` one (319), behind `disabled={upgradeCount === 0 \|\| anyBusy}` where `upgradeCount` *is* the guard's expression | **SURVIVED** (unchanged) |
| D48 `RunsPanel.tsx:155` | button `disabled={archiving \|\| finishedCount === 0}` — the guard's own expression (182) | **SURVIVED** (unchanged) |
| D49/D50 `ArtifactsPanel.tsx:58,166` | `disabled={uploading}` (101); the attach menu item is spread under `...(artifact.path ? … : [])` (173–174) | **SURVIVED** (unchanged) |
| D51 `FileTreePanel.tsx:96` | read `FileTreeNode`'s click handler: it branches on `entry.isDir` itself (51–57) and only calls `onOpenFile` for files | **SURVIVED** (unchanged) |
| D52 `TerminalPanel.tsx:238` | the component is module-private (not exported) and rendered only under `tabs.createError` (137–138) | **SURVIVED** (unchanged) |

### Reasons replaced or corrected (the substantive output)

**D15 — the real reason is not "macrotask-driven callers".** The claim in §5 was
that a superseded lane lands in the `catch` at 401. That is true, but it is not
*why the re-check at 380 is dead*: the abort inside `readRunReplay` is a
convenience, and if a lane were superseded just after the last internal check the
`catch` would still not be what ran. The reason that holds is structural:
**`readRunReplay` (`packages/thread-projection/src/replay.ts`) has exactly one
`await` (line 282, the page read), and the entire tail after its last
`isCurrent()` check (297) — the loop remainder, the break, the prefix validation
and both returns — is synchronous.** So the last internal check and
`readRunLog`'s own check are separated by microtasks only, and microtasks drain
before any macrotask can bump `epochRef` (428 / 592, both reached from timers,
event handlers or an effect flush). The two checks cannot disagree.
*Falsification route, tightened:* find an `await` added to that tail, or a
`restore`/write path that bumps the epoch from inside a microtask.

**D53 — the reason is that the window is synchronous, not that `container` is a
`const`.** `container` is non-null by construction, but the *second* disjunct
(`cancelled`) is the interesting one and it looked falsifiable: the cleanup at
125–130 really does set `cancelled = true`, and effect cleanups run on
re-render — so a page flip during the `getPage` await is a real supersede. What
makes the line dead is narrower: between the `cancelled` check at 86 and the
check at 101 there is **no await and no yield**, and a React cleanup cannot run
inside a synchronous block. *Falsification route:* insert any `await` between
those two lines (e.g. `await page.getViewport(...)`, which the pdf.js API
permits) and the line becomes reachable.

**D10 — the entitlement is a disjunction, not a duplicated prop.** The button's
`disabled` is `balanceStatus !== "unavailable" && !platformUrl` (190), which is
*not* the guard's expression, and the guard is `!platformUrl`. The entailment
needs both halves: the `onClick` (191) picks `onRefreshBalance` exactly when
`balanceStatus === "unavailable"` — the one case the `disabled` disjunction
exempts — and picks `handleRecharge` otherwise, where `!platformUrl` disables the
button. So the two branches together cover the guard exactly. A reviewer looking
for `disabled={!platformUrl}` would not find it and could wrongly call this
waiver broken; §8.5's rule is the simpler *shape*, not a licence to skip the
check. *Falsification route:* make the `onClick` ternary stop matching
`balanceStatus` (e.g. route both arms to `handleRecharge`).

**D22 — the entailment is the render condition, not the `disabled` prop.** The
install button's `disabled` (244) is about recovery/maintenance state entirely;
`!status?.hasUpdate` is enforced by the enclosing `{status.hasUpdate ? … : …}`
(205), i.e. the button does not exist without it. The same correction applies to
D23 and D24, whose buttons are likewise gated by the enclosing render rather than
by `disabled`. *Falsification route:* render the install button outside the
`status.hasUpdate` branch.

**D14 — the proof is now mechanical, and the sibling doubles as the control.**
All three call sites pass the same `const catalogVersion` validated at 363–364,
so no call can reach the guard with a non-object. The control is `readString` in
the same file: its identically shaped guard (323–324) **is covered**, because one
of its callers really is reached with an unvalidated payload. A dead guard and a
live guard, two implementations 7 lines apart — which is the evidence that this
tag's distinction is drawn by measurement rather than by convenience.

### The four non-React tags (segment 11)

Segment 10 reviewed the React guards. Segment 11 attacked the four tags where
"unreachable" is *least* plausible — pure logic with no render, no `act()`, no
event listener to blame: three module-level LRU caches and one remark AST walk.
Each was checked mechanically rather than argued.

| tag | the claim | the check | verdict |
|---|---|---|---|
| **D7** `threadMessageCache.ts:48` | `keys().next().value` is always a `string` because the loop condition is `snapshots.size > MAX_CACHED_THREADS`, so the map is non-empty; the `undefined` arm exists only for the type | read the guard: it is `=== undefined`, and a map with `size > 0` always yields a first key | **SURVIVED** — and this is the *safe* form of the pattern |
| **D8** `useFileTree.ts:47` | every cache key is a non-empty string, so `!oldest` is never true | `grep` for the function and its callers: `stateFor` has **exactly one** call site (l.84), `const state = root ? stateFor(root) : null` — a truthy `root` is a non-empty string, and `cache.set` is only ever handed that `root` | **SURVIVED** — the clause holds, but see below |
| **D9** `streamingMarkdownBlocks.ts:95` | the tree comes from a real remark parse, which attaches `position` to every node, and the guard above already rejects `children.length < 2` | read the processor (l.16): `unified().use(remarkParse).use(remarkMath).use(remarkMathFence).use(remarkGfm).use(remarkLatexMath).use(remarkCjkEmphasis).use(remarkAutolinkBoundary).use(remarkUnclosedLink)` — **no position-stripping plugin**, and the producer (l.159) is `streamingMarkdownProcessor.parse(text)`, i.e. a full parse, not a hand-built tree. Checked the §10 hypothesis that a perf-tuned streaming path strips positions and makes this line *always* taken (which would have meant table checkpoints never work) — it does not | **SURVIVED** |
| **D13** `tabs.ts:160` | pigeonhole: the loop bound `used.size + 1` admits exactly one index that a set of size `used.size` cannot all contain | re-derived the bound: for `index` in `1..size+1` there are `size+1` candidates and at most `size` are in `used`, so `!used.has(index)` returns before the fallback | **SURVIVED** — the strongest of the 61 (a counting argument, not a call-site enumeration) |

**D8's clause is worth isolating, because it is the one place in the 61 where the
guard is written loosely and is saved by a caller, not by its own condition.** The
line is `if (!oldest) break;` — the *falsy* test, where D7's identical loop uses
`=== undefined`. `oldest` is a cache key, so the loose test also accepts an empty
string; if one ever reached the cache, the loop would break early and
`CACHE_MAX_ROOTS` would stop bounding the cache (a slow leak, since the map is
module-level). It cannot be reached today only because of the caller's `root ?`
test. That is a *latent* fragility rather than a coverage question — no line
counts it and no waiver covers it — and I am recording it as such rather than
either filing a bug (a workspace root is never `""` in this product) or deleting
the branch (that would be a production edit for a metric). If a future caller can
pass `""`, the right fix is `=== undefined` plus a key-emptiness check, and the
line would then be reachable and testable.

### Confidence in this classification — calibrate before trusting it

**Two passes of my own approved all 61 tags. That is weak evidence, and this
campaign has measured how weak.** Of the reviewer verdicts so far:

| unit | waivers/residual rows | independently falsified | rate |
|---|---|---|---|
| channels (judge REV-CH, `review-rust.md` §"round 2 (cov100)") | 24 | **16** — with *executing* inputs, each documented with a command and observed count | 67 % fell |
| mobile (judge REV-FE, `review-frontend.md` §R2) | 13 | 0 | 13/13 survived |
| desktop (this module) | 25 of 61 tags checked by the review (its own accounting, `review-desktop.md` §32; 10 of them enumeration-only) | **3 — all acted on**: `D25` (4 lines, re-registered OPEN in §5), `D41` (1 line, **covered** in §8.18) and `D54` (1 line, **covered** in §8.20) | 6 of the 24 lines those 17 executing tags cover (25 %) |

The channels result is the important one: an author's own classification, made in
the same careful style mine was, was wrong two times in three. Mobile's survival
shows a 100 %-surviving set is possible — so neither rate is a forecast for these
59; what they establish is that **self-approval is not the evidence the repo
policy asks for** (`waiver-ledger.md`: "The gate checks STRUCTURE only. Whether
each reason is true is the reviewer's job … a waiver that survives is evidence, a
waiver that falls is rework"). The desktop rate is measured on the review's 25
tags and landed between the two — below the channels prior and well above zero:
the reviewer overturned the waiver on **6 of the 24 lines** those 17 *executing*
tags cover
(4 in `D25`, now OPEN; 1 in `D41` and 1 in `D54`, both covered), which is the
honest reading of a
set whose first independent sample is not clean.

So the end state of W1 is reported as: **`DA:0 = 59` — not met, and not claimed**;
the residual is **55 waived + 4 OPEN**, and the waiver arm is **pending an
independent verdict per line** (44 registration rows, 25 of which the review's two
passes have now touched — `review-desktop.md` §32). Ordered by how likely each is
to fall under an executing-input attack (mine are structural where they can be, so
the weakest are the ones that depend on *timing* or on an enumeration I did by
reading):

1. **D41** `useFutureLoginFlow.ts:104` — **FALSIFIED, then settled by covering the
   line.** It was the only tag whose category was
   `unreachable-in-this-environment`, i.e. it *was*
   reachable in a browser that preempts a macrotask between commit and
   passive-effect flush, and REV-DESK built exactly that harness: the line hit 5
   times with the window asserted open. This record first disputed the *fixture* —
   the reviewer's counterexample is a render outside `act()` — and lost that
   argument on the mechanism: `act()` is what closes the window, so the fixture was
   the ordinary case, not a forged one. Segment 13 covered the line (§8.18), so the
   tag is gone rather than restated, and the *question* is no longer open. It
   stays first in this list because its lesson transfers to every remaining tag
   that rests on what a harness can reach rather than on what the code can do.
2. **D15** `useRemoteTimeline.ts:380` — rests on `readRunReplay`'s post-check tail
   being await-free. Needs a watermark-carrying host and a settled-run push racing
   an in-flight log read; I could not build it, which is not the same as its not
   existing.
3. **D35/D36** `useQuestionNav.ts:202,280` — the same-tick family, and its
   *identically shaped* sibling at 149/202 proved **coverable** while 202 did not.
   A family where one member fell is the right place to look for another.
4. **D10** `AccountPage.tsx:60` — the entailment is a disjunction over the
   `onClick` ternary's two arms; if the arms can ever diverge from
   `balanceStatus`, the guard is live. Enumerate the call sites rather than reading
   the reasoning.
5. **D53** `PdfPreview.tsx:102` — "no await between l.86 and l.101" is only as
   durable as the pdf.js API; a version where `getViewport` awaits, or a path that
   re-reads the ref, falsifies it.
   *Sibling of (1), worth naming here:* this tag is also a timing claim about a
   window, and `D41` and `D54` are the precedents for how such claims fail — the
   window was real in both cases and the category was wrong about *why* it could
   not be reached (§8.18, §8.20).

The four verified above (D7, D8, D9, D13) and the mechanical single-caller tags
(D21, D42, D46, D47, D48, D51, D52) are the *strongest* and should be attacked
last — or not at all.
