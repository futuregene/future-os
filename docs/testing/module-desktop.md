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
python .future/cov100/verify.py js future-desktop-ts 99.99
```

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

## 2. Before / after (real report, this worktree)

| metric | before (first measurement, this segment) | after |
|---|---|---|
| lines | 63.28 % (5681 / 8977) | **66.64 % (5983 / 8977)** |
| statements | 63.02 % (5921 / 9395) | 66.41 % (6240 / 9395) |
| branches | 55.82 % (4032 / 7222) | **57.79 % (4174 / 7222)** |
| functions | 57.69 % (1424 / 2468) | 61.46 % (1517 / 2468) |
| test files / tests | 145 / 1255 | 153 / 1342 |

Uncertainty worth recording: the task brief quoted "80.51 % lines / 33k lines"
for this module. No coverage configuration existed in this worktree, so that
number was not reproducible here; the 8977-line denominator is the v8
executable-line count for `src/**` (the 33k figure counts raw file lines,
including tests and type-only lines). The 63.28 % baseline above is the first
measurement that includes files no test imports.

This segment did **not** reach the module target. The remaining 2994 uncovered
lines are real work, listed in §6 — they are *not* waived.

## 3. Files changed

Test-only changes plus the measurement config; no production file was modified
(no guard deleted, no `cfg(test)` around production branches, no assertion
weakened).

| file | change | new tests |
|---|---|---|
| `desktop/vite.config.ts` | added `test.coverage` (v8, whole-tree `include`) | — |
| `desktop/src/integrations/agent/providers.test.ts` | new | 17 |
| `desktop/src/components/layout/hooks/useHasProviders.test.tsx` | new | 12 |
| `desktop/src/components/layout/hooks/useContextData.test.tsx` | new | 16 |
| `desktop/src/components/layout/hooks/useUnreadThreads.test.tsx` | new | 10 |
| `desktop/src/components/layout/hooks/useRightPanelWidth.test.tsx` | new | 12 || `desktop/src/components/layout/hooks/useRemoteStatus.test.tsx` | new | 6 |
| `desktop/src/components/layout/hooks/usePendingApprovalCounts.test.tsx` | new | 7 |
| `desktop/src/components/layout/hooks/useUpdateChecker.test.tsx` | new | 7 |

Per-file result after the change (before → after):

| file | line coverage | branch coverage |
|---|---|---|
| `src/integrations/agent/providers.ts` | 16.3 % → **100 %** (43/43) | 0 % → 100 % |
| `src/components/layout/hooks/useHasProviders.ts` | 0 % → **100 %** (41/41) | 0 % → 100 % |
| `src/components/layout/hooks/useContextData.ts` | 0 % → **100 %** (82/82) | 0 % → 95.3 % |
| `src/components/layout/hooks/useUnreadThreads.ts` | 0 % → **100 %** (50/50) | 0 % → 100 % |
| `src/components/layout/hooks/useRightPanelWidth.ts` | 0 % → **100 %** (46/46) | 0 % → 100 % |
| `src/components/layout/hooks/useRemoteStatus.ts` | 0 % → **100 %** (8/8) | 0 % → 100 % |
| `src/components/layout/hooks/usePendingApprovalCounts.ts` | 0 % → **100 %** (16/16) | 0 % → 100 % |
| `src/components/layout/hooks/useUpdateChecker.ts` | 0 % → **100 %** (17/17) | 0 % → 100 % |

Net: +302 covered lines, +142 covered branches, +87 tests, 145 test files
untouched, full suite green (153 files / 1342 tests) and `tsc --noEmit` /
`eslint` clean on every new file.

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

## 5. Waivers

**None added for this module.** A waiver is only written when a line is proven
unreachable (`unreachable-by-construction`), needs an environment that cannot be
produced deterministically (`unreachable-in-this-environment`), or is a
span/brace attribution artifact (`attribution-artifact`). Everything closed in
this segment was closable with a real test; the remaining uncovered lines are
missing tests, not unreachable code, so writing them down as waivers would be
falsifying the ledger.

Two candidate waivers were checked and **rejected** (both turned out to be
coverable, so neither is in the ledger):

| candidate | verdict |
|---|---|
| `useContextData.ts` spinner-timer `if (cancelled) return` — looked unreachable because the effect cleanup calls `clearTimeout` first | covered by fault-injecting the documented race (a browser that has already dequeued the callback): the test would fail if the guard were removed, because the stale timer would blank the replacement thread |
| `useRightPanelWidth.ts` / `useUnreadThreads.ts` storage `catch` arms — `vi.spyOn(Storage.prototype, "getItem"/"setItem")` does **not** intercept jsdom's `sessionStorage` (the spy silently targets a different `Storage`) | covered by redefining `window.sessionStorage` with a throwing getter, which also makes the "does not throw" assertion meaningful |

## 6. Remaining work (not waived)

Uncovered lines after this segment, by file (statement-start lines from
`desktop/coverage/coverage-final.json`, top 20). All of these are missing tests
in large composite components; the plan's per-file recipe (render with mocks,
assert interaction/loading/empty/error states) applies.

| file | uncovered lines |
|---|---|
| `src/components/layout/AppShell.tsx` | 739 |
| `src/features/agent/AgentThread.tsx` | 617 |
| `src/components/layout/ContextPanel.tsx` | 392 |
| `src/components/layout/OnboardingGate.tsx` | 374 |
| `src/features/agent/NewConversation.tsx` | 344 |
| `src/features/settings/ProvidersPage.tsx` | 336 |
| `src/features/remote/RemoteView.tsx` | 288 |
| `src/features/agent/MentionEditor.tsx` | 253 |
| `src/features/agent/Composer.tsx` | 248 |
| `src/features/review/LastRunReview.tsx` | 242 |
| `src/features/agent/ApprovalPrompt.tsx` | 181 |
| `src/components/layout/hooks/useAgentConnection.ts` | 175 |
| `src/features/artifacts/PdfPreview.tsx` | 162 |
| `src/features/settings/SettingsDialog.tsx` | 156 |
| `src/components/ui/DiffView.tsx` | 152 |
| `src/features/review/ReviewPanel.tsx` | 145 |
| `src/features/artifacts/ArtifactsPanel.tsx` | 140 |
| `src/features/review/GitChangesReview.tsx` | 135 |
| `src/features/settings/ModelsPage.tsx` | 130 |
| `src/components/layout/AppShellDialogs.tsx` | 126 |

Anticipated waiver candidates in that set (to be *proven* before being written
down, not assumed): `src/features/artifacts/PdfPreview.tsx` (pdf.js canvas/WebGL
rendering — jsdom has no canvas unless the optional `canvas` package is added,
and adding a dependency is outside this task's write set) and
`src/features/terminal/TerminalView.tsx` (xterm.js needs a real renderer).
Everything else should be closable.

## 7. Weak tests found and fixed

| finding | action |
|---|---|
| `useRightPanelWidth.test.tsx`, first draft: `const { current } = renderHook(…)` destructures the harness **getter**, freezing the value at the first commit — three assertions were passing without exercising anything (e.g. "ignores a non-primary button" passed because `resizing` was read once as `false`) | rewritten to hold the harness object and read `hook.current` per assertion; the same pattern was avoided in the other new files |
| `useRightPanelWidth.test.tsx` / `useUnreadThreads.test.tsx`, first draft: storage-failure tests asserted only "does not throw" plus a value that holds equally when the spy misses; the `Storage.prototype` spy was in fact not intercepting jsdom's `sessionStorage` (found by making the assertion discriminating: a stored 640 px width survived) | replaced with the throwing-`window.sessionStorage` getter, which makes the "does not throw" claim fail if the `try/catch` is removed |
| `useContextData.test.tsx`, first draft: "blanks the context when a fetch fails" used `mockRejectedValueOnce` on the bootstrap call, but the poll had already bumped the refresh generation, so the *current*-generation branch was never reached | split into a poll-free failure test (covers the blanking arm) and a separate stale-generation test |
| no assertion-free, `it.skip`, `xit` or `describe.skip` tests were added | — |

No snapshot-only tests were added (none of the new tests use snapshots).

## 8. Bugs / traps found while measuring

1. **Instrumentation pushes the streaming-parser perf test past its 30 s fence.**
   `streamingSharedParser.test.ts` documents an explicit `30_000` ms budget;
   uninstrumented it finishes in 3.6 s / 4.6 s, under v8 coverage it takes
   24.8 s / 32.9 s and times out, which (with `reportOnFailure: false`) silently
   produces *no* coverage report. Not a product bug, but a measurement trap:
   anyone running `--coverage` with default timeouts gets a failed run and a
   missing `coverage-summary.json`.
2. **`vi.spyOn(Storage.prototype, "getItem")` does not affect jsdom's
   `sessionStorage`** in this Vitest/jsdom combination, while it appears to
   affect `localStorage`-style direct usage. Tests must redefine the
   `window.sessionStorage` property to inject a storage failure.
3. **`renderHook()` returns a getter for `current`.** Destructuring it snapshots
   the first commit, so later state changes are invisible and assertions pass
   vacuously. Applies to every hook test in this repo.
4. `useRemoteStatus` deliberately skips state updates when the poll returns
   structurally identical JSON — this is why the derived `indicator` cannot be
   re-mapped without a changed payload (documented in the new test, and the
   reason the app-level 3 s poll does not re-render the shell).

## 9. Next checks

1. Close `src/components/layout/hooks/useAgentConnection.ts` (175 lines, no test
   file) and the remaining `layout/hooks/*` files; then move to
   `ContextPanel`/`OnboardingGate`, which are the gatekeepers of `AppShell` and
   share its mocks.
2. Re-run `npx vitest run --coverage --testTimeout=180000` after each batch and
   keep `desktop/coverage/coverage-summary.json` as the number of record; verify
   with `python .future/cov100/verify.py js future-desktop-ts <target>`.
3. Decide the two anticipated waivers (pdf.js canvas, xterm) only after
   attempting a jsdom render; if a waiver is unavoidable, record the failed
   attempt in this file before writing the waiver.
4. `packages/markdown`, `packages/thread-projection`, `packages/json-preview`
   are measured through this suite only where `desktop/src` imports them; their
   own line coverage belongs to whoever owns `packages/*` in the plan. Their
   desktop-side behaviour is covered here by
   `src/features/markdown/*.test.ts(x)` (already green) plus the new
   `useUnreadThreads`/`useHasProviders` serialization cases.
