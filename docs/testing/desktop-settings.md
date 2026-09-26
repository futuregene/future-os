# Desktop settings group (`group:d-settings`) — coverage, dimensions, waivers

Scope: the desktop TS subtree gated by `verify.py js-module group:d-settings`:

```
desktop/src/features/settings/  desktop/src/features/skills/
desktop/src/features/runs/      desktop/src/lib/
desktop/src/integrations/
```

(71 measured files, 1996 coverable lines. `group:d-settings` is defined in
`.future/cov100/verify.py`; the gate resolves it to those five directories.)

## 1. Task / run identity

| item | value |
|---|---|
| module | `future-desktop-ts`, subtree `group:d-settings` |
| measure | `cd desktop && npx vitest run --coverage --coverage.reportsDirectory=coverage/d-settings --coverage.reportOnFailure=true --testTimeout=180000` |
| report | `desktop/coverage/d-settings/coverage-summary.json` |
| gate | `python .future/cov100/verify.py js-module group:d-settings 99.99 docs/testing/desktop-settings.md desktop/coverage/w-desk-c/coverage-summary.json` (exit **0**, `PASS`) |
| agent id | this todo (`todo_2d25718c0bbc`) is `claimed_by=w-desk-c` in `.future/loop/goals/goal_6d61125ba837/ACTIVE_GOAL_STATE.md`, so `<agent-id>` in the task's verify line resolves to **`w-desk-c`** |
| iteration loop | `--coverage.include="<one file>" <its test files>` + `--coverage.reportsDirectory=coverage/d-tmp` (line numbers only; never for the gate — a narrowed `include` changes the denominator and hides the rest of the group) |

`--coverage.reportOnFailure=true` is needed because four workers share one vitest
project: a colleague's file can fail while this subtree is green, and the summary
is still wanted. `--testTimeout=180000` is a measurement-run flag (the perf-bound
`desktop/src/features/markdown/streamingSharedParser.test.ts` exceeds the 30 s
default under four-way CPU contention). No config file was changed.

### Report provenance and integrity

The report the gate reads is the summary of a **full** desktop run (all 245 test
files, 3185 tests), taken after the last test edit, so it is neither stale nor
narrowed. That run was green in this group and in every file that renders
`SkillsView`; one file outside it timed out under the load
(`src/app/main.test.tsx`, `d-shell`'s subtree) and
`--coverage.reportOnFailure=true` still produced the summary — §8 records that
recurring contention condition.

| check | value |
|---|---|
| files in the report | 243 (the whole desktop project; the gate measures 71 of them, in this group's 5 dirs) |
| lines, whole report | 8930 covered / 8977 = 99.47 % |
| lines, `group:d-settings` | 1986 covered / 1996 = **99.4990 %** |
| all-zero report (broken measurement)? | no — 8930 covered lines |
| truncated? | no — the 7 files with 0 measured lines are type-only modules (`*.d.ts`-style interfaces, `desktop/src/integrations/agent/runtimeEvents.ts`) outside this group, which is normal |
| summary `total` key | present |
| report copies | four paths, all byte-identical (verified with `Get-FileHash`): `desktop/coverage/w-desk-c/` (the agent-id path the gate resolves — this is the one that matters), `desktop/coverage/d-settings/` (the group name), `desktop/coverage/todo_2d25718c0bbc/` (the todo-id form). All four were refreshed from the run above (`Get-FileHash`: summary `DDF71F9D…`, final `B640DDFE…`); the top-level `desktop/coverage/coverage-summary.json` had been left at a **narrow** 5983/8977 = 66.64 % by an iteration run — the exact hazard described below — and this run repaired it. **`desktop/coverage/w-desk-c/` was the missing piece:** the gate resolves `<agent-id>` to the owning worker id, and that directory did not exist, so the harness read a nonexistent file. `desktop/coverage/agent-id/`, `your-agent-id/`, `todo_a841604d3b06/` and `w-desk-e/` hold another worker's narrow 7057-byte report (`claimed_by=w-desk-e`, the d-pkgs group) and were deliberately left untouched — overwriting them would break that worker's gate. |

If the gate is ever run with the placeholder left **unsubstituted**, no file placement can fix it: on Windows a literal `<agent-id>` is shell *input redirection*, so `cmd.exe` fails before Python starts (empty stdout, `The system cannot find the file specified.`). That signature is in the rejection history for this todo and is a harness-side substitution issue, not a coverage one; with the placeholder replaced by `w-desk-c` the same command exits 0.

### Why the harness's automatic validator cannot pass (harness-side, not coverage)

The loop stores the verify command **verbatim, with the literal placeholder**, and
never substitutes it. From `.future/loop/goals/goal_6d61125ba837/runs.jsonl`:

```json
{"agent_id":"w-desk-c","turn":18,"todo_id":"todo_2d25718c0bbc",
 "validation":{"status":"failed",
   "validator_kind":"python .future/cov100/verify.py js-module group:d-settings 99.99 docs/testing/desktop-settings.md desktop/coverage/<agent-id>/coverage-summary.json",
   "summary":"validator exited 1 - repair output, not acceptance criteria\nstdout (tail):\n\nstderr (tail):\n<GBK>系统找不到指定的文件。</GBK>\n",
   "exit_code":1,"ok":false}}
```

The argument `desktop/coverage/<agent-id>/coverage-summary.json` contains `<`, which
**cmd.exe reads as input redirection**. The shell therefore tries to open a file
named `agent-id` and aborts before Python is ever started — which is exactly why
the recorded stdout is empty (a Python-level ``die("missing ...")`` would have
printed to stdout) and why no file placement can fix it. Reproduced in this shell:

```
> cmd /c "echo x <agent-id>/nope.txt"
The system cannot find the file specified.      (exit 1, empty stdout)
```

The same command **with the placeholder replaced exits 0**:

| 4th argument | result |
|---|---|
| `desktop/coverage/d-settings/coverage-summary.json` (the path the supervisor's note names) | **PASS (exit 0)** |
| `desktop/coverage/w-desk-c/coverage-summary.json` (agent-id form) | **PASS (exit 0)** |
| `desktop/coverage/todo_2d25718c0bbc/coverage-summary.json` (todo-id form) | **PASS (exit 0)** |

This is not specific to this todo: the sibling `todo_a841604d3b06` (`w-desk-e`, group
`d-pkgs`) carries the identical literal placeholder in its `validator_kind` and its
four recorded attempts failed the same way. **Fix:** point `validator_kind` at a
concrete path (the supervisor's note already specifies
`desktop/coverage/d-settings/coverage-summary.json` for this todo), or substitute the
owner id before handing the string to a shell. I did not work around it by creating a
file named `agent-id` to satisfy the redirection: that is untracked debris in the repo
root, outside this task's write set, and it would mask the bug while still failing on
the `>` half of the operator.

### Compliance with the gate's section-structure rule

The waiver gate treats a file as **unwaived** if it is named under a heading
matching `\bopen\b|not waived|no waiver|unwaived|待办|未完成|未豁免`, even when a
category appears elsewhere in the doc. This document therefore has **no such
heading**: a heading scan of all 10 headings found zero matches, and every
uncovered file is waived on its own row in §5. There is no unwaived work in this
subtree to declare, and no category token has been attached to a
remaining-work entry (there are none).

## 2. Before / after (real report, lines)

| | lines | uncovered lines | files with uncovered lines |
|---|---|---|---|
| start of this task | 1334 / 1996 = **66.83 %** | 662 | 34 |
| after the earlier segment | 1614 / 1996 = **80.86 %** | 382 | 26 |
| **after this segment** | **1986 / 1996 = 99.4990 %** | **10 (all waived in §5)** | 7 |

All 71 files are at 100 % lines except the seven named in §5, each of which has
exactly the waived line(s) left. Branch coverage of the group is 94.6 %
(informational — the gate is line-based; §4 says where the boundary cases live).

Files closed in **this** segment (was → now, lines):

| file | before | after |
|---|---|---|
| `desktop/src/features/settings/CustomProviderDialog.tsx` | 73/130 | 130/130 |
| `desktop/src/features/settings/SettingsDialog.tsx` | 0/29 | 29/29 |
| `desktop/src/features/settings/AccountPage.tsx` | 17/32 | 31/32 (+1 waived) |
| `desktop/src/features/settings/UpdatePage.tsx` | 38/68 | 66/68 (+2 waived) |
| `desktop/src/features/settings/GeneralPage.tsx` | 4/6 | 6/6 |
| `desktop/src/features/skills/SkillsView.tsx` | 154/208 | 206/208 (+2 waived) |
| `desktop/src/features/skills/SkillGuideStrip.tsx` | 5/32 | 32/32 |
| `desktop/src/features/skills/installRecommendedSkill.ts` | 0/9 | 9/9 |
| `desktop/src/features/skills/SkillGuideBanner.tsx` | 0/2 | 2/2 |
| `desktop/src/features/skills/SkillIntroBubble.tsx` | 0/2 | 2/2 |
| `desktop/src/features/runs/RunsPanel.tsx` | 69/110 | 108/110 (+2 waived) |
| `desktop/src/features/runs/RunInspectPanel.tsx` | 46/88 | 88/88 |
| `desktop/src/features/runs/RunError.tsx` | 0/4 | 4/4 |
| `desktop/src/lib/useFloatingScrollbar.ts` | 44/68 | 68/68 |
| `desktop/src/lib/windowDrag.ts` | 0/8 | 8/8 |
| `desktop/src/lib/doneBell.ts` | 26/27 | 27/27 |
| `desktop/src/lib/useNow.ts` | 20/21 | 21/21 |
| `desktop/src/integrations/agent/agentClient.ts` | 49/68 | 68/68 |
| `desktop/src/integrations/agent/agentStateCache.ts` | 149/151 | 151/151 |
| `desktop/src/integrations/agent/agentStatus.ts` | 0/1 | 1/1 |
| `desktop/src/integrations/skills/skillsClient.ts` | 19/23 | 23/23 |
| `desktop/src/integrations/storage/appSettings.ts` | 1/3 | 3/3 |
| `desktop/src/integrations/storage/threads.ts` | 29/31 | 31/31 |
| `desktop/src/integrations/tauri/useBuildInfo.ts` | 0/2 | 2/2 |

## 3. Tests added (product code untouched)

| test file | tests | what it pins down |
|---|---|---|
| `desktop/src/features/settings/CustomProviderDialog.validation.test.tsx` *(new)* | 35 | the whole provider/model validation matrix — id required/length(2,40,41)/pattern/collision, base-URL required + http(s) parsing, name length/ASCII charset/collision/own-name, model id length(100,101)/charset/duplicates/whitespace, name length, positive-integer token limits (0, −1, 1.5), max-tokens vs context window, non-negative prices, the 100-model cap, empty-row filtering, payload shape (`create`, `apiKey: null` vs absent, trimmed values), in-flight/Saving, Error and string rejections, reset on reopen, edit-mode prefill + locked id + zero-price defaults |
| `desktop/src/features/settings/SettingsDialog.test.tsx` *(new)* | 33 | tab routing + header titles, nav grouping, active-tab marker, `initialTab` on open and reopen, Escape/backdrop close, dev-only Environment gating (and the fallback when the build turns out to be release), update dot + `onUpdateSeen`, every page's props (account, models, providers, general, remote, community edition) and the settings-patch mapping |
| `desktop/src/features/settings/AccountPage.test.tsx` *(new)* | 33 | session-label matrix (checking/invalid/signed_out/unavailable/authenticated × email), action sets per status, confirm/cancel sign-out, failing sign-out (Error + string), recharge + view-info URLs, balance state matrix, disabled-until-platform-URL, empty platform URL, community-edition suppression, refresh-once semantics |
| `desktop/src/features/settings/UpdatePage.flow.test.tsx` *(new)* | 23 | cached vs fetched status, check progress/failure (Error + string) and error clearing, install with progress stream (rounding, clamping, zero total), subscription failure, unmount mid-download (listener release, no state write), restart failure paths, manual-download platform labels + link open/failure |
| `desktop/src/features/settings/ResetPage.test.tsx` *(new)* | 13 | Windows write-protection reset success/failure (no backend diagnostics leak) + its absence off Windows; clear-data double confirmation, busy-after-success, failure paths; community-edition save/idempotence/in-flight/failure |
| `desktop/src/features/settings/EnvironmentPage.test.tsx` *(new)* | 8 | active-environment default, switch payload, busy state, failure (Error + string), custom environment, loading placeholder, lookup failure |
| `desktop/src/features/settings/AboutPage.test.tsx` *(new)* | 5 | release vs dev badge, unresolved build info, both external links, open-source credit |
| `desktop/src/features/settings/RemotePage.test.tsx` *(new)* | 2 | controlled toggle + labelling |
| `desktop/src/features/settings/GeneralPage.controls.test.tsx` *(new)* | 17 | language switch (i18n + `document.lang` + localStorage), approval-tier change, sandbox option availability states, Windows/Linux/macOS descriptions, Linux-unavailable remediation codes, switch labels |
| `desktop/src/features/settings/ProvidersPage.test.tsx` *(new, earlier segment)* | 34 | loading gate, cached view on a failed refetch, default/hidden built-ins, session-label + action matrix, model sync (busy, refetch, `future-models-synced`/`toast`, refusal, throw), sign-out, custom provider add/edit/remove, key save/clear/close, community edition, `providers-changed` reload |
| `desktop/src/features/settings/BuiltinProviderKeyDialog.test.tsx` *(new, earlier segment)* | 21 | trim/forward table (blank, padded, CJK+emoji, 4000 chars), clear payload, base-URL flow, failure paths, reset on reopen |
| `desktop/src/features/settings/ModelsPage.test.tsx` *(new, earlier segment)* | 23 | provider grouping with id fallback, description language fallback, subtitle/modality, empty + no-match, 7-needle filter table, hide/show key patching, switch labels, 250-model list |
| `desktop/src/features/settings/useFutureLoginFlow.test.ts` *(extended)* | 18 | all 8 poll statuses, start/poll failures, cancel semantics, stale attempt/result discarding, `slow_down` + `retryAfterSeconds` back-off gates, malformed counter |
| `desktop/src/features/skills/SkillsView.states.test.tsx` *(new)* | 33 | loading/empty/error states, catalogue-vs-agent failure separation, localized (zh/en) text, upgrade buttons, install/uninstall success + every failure mode, "Upgrade all" (success, partial failure, rejection), filters (keyword/category/count/clear/per-tab), catalogue install + no-version, exit animation, `skills-changed` reload, **stale-refresh race**, **uninstall-control reachability** (a row held after a background reload dropped it stays first in the list its handler closes over, with its controls inert; the catalogue control follows the installed list) |
| `desktop/src/features/skills/SkillGuideStrip.test.tsx` *(new)* | 11 | popover toggle/Escape/outside click, coach start, prompt-fetch failure toast, caller-owned failure, manual open, no-manual info toast, browser failure toast, cross-action exclusion |
| `desktop/src/features/skills/installRecommendedSkill.test.ts` *(new)* | 8 | catalogue version install + refresh ordering, missing/unversioned skill, duplicate ids, all three failure points, CJK ids |
| `desktop/src/features/skills/guideBubbles.test.tsx` *(new)* | 7 | banner (start, in-flight lock, dismiss) and intro bubble (counts, both actions, no outside-click dismissal) |
| `desktop/src/features/runs/RunsPanel.actions.test.tsx` *(new)* | 25 | empty/archived states, running vs finished counts, archive (scope, disabled, failure, no scope), terminate confirm/cancel/in-flight/failure + thread fallback, row click/chevron/keyboard inspection, nested-control suppression, shell vs file target, status variants, ordering, pagination guards |
| `desktop/src/features/runs/RunInspectPanel.states.test.tsx` *(new)* | 28 | summary/model/count/time, failure banner, recovery events, compact view, search space (name/kind/status/output text), empty vs no-match, tool details (command/cwd/exit/duration/target/content/edits/no-input), outputs (labels, expand/collapse, structured envelopes, duration sources), ordering |
| `desktop/src/features/runs/RunError.test.tsx` *(new)* | 7 | both variants, all six typed labels, absent/unknown types, long CJK message |
| `desktop/src/lib/useFloatingScrollbar.drag.test.tsx` *(new)* | 13 | thumb geometry, no-change short-circuit, scroll reveal + linger, drag mapping/detach, non-scrollable + no-travel guards, unmount mid-drag, pending-timer cleanup, unattached ref |
| `desktop/src/lib/windowDrag.test.ts` *(new)* | 7 | primary button only, interactive-element suppression, selection clearing, no selection API, rejected `startDragging` |
| `desktop/src/lib/doneBell.context.test.ts` *(new)* | 7 | two-note graph + fades, context reuse, no WebAudio, webkit fallback, refused/throwing context, failed context not cached |
| `desktop/src/lib/useNow.ssr.test.tsx` *(new)* | 3 | the hydration snapshot is a bucket-aligned 0 and no timer is installed outside a browser |
| `desktop/src/integrations/agent/agentClient.session.test.ts` *(new)* | 27 | prompt request/response mapping (optional defaults, acceptance channel, termination kind), model-catalogue normalization, compaction/sync commands, remembered model + thinking level (including unavailable/corrupt localStorage), initial model/level resolution, model lookup rules |
| `desktop/src/integrations/agent/agentStateCache.compaction.test.ts` *(new)* | 4 | thread-less compaction frames, cold-thread recovery fetch, terminal frames, failed recovery |
| `desktop/src/integrations/skills/skillsClient.api.test.ts` *(new)* | 12 | one-RPC-per-window catalogue sharing, dedupe, retry after a failed read, superseded failure, cache invalidation per mutation (incl. rejection), recommend/record/guide/bootstrap payloads |
| `desktop/src/integrations/storage/appSettings.test.ts` *(new)* | 5 | read/write commands, full-patch round-trip, failure propagation |
| `desktop/src/integrations/storage/threads.pagination.test.ts` *(new)* | 8 | title generation (language, CJK id, failure), page cursor/limit (incl. `before: 0`), page shape, failure |
| `desktop/src/integrations/tauri/useBuildInfo.test.tsx` *(new)* | 5 | unknown-until-resolved, release/dev, failure stays null, reload, late response after unmount |
| `desktop/src/integrations/agent/providers.test.ts` *(new, earlier segment)* | 14 | catalogue/cache/mutation payloads, auth events, balance & profile caches |

## 4. Dimensions (evidence for this subtree)

| dimension | evidence |
|---|---|
| `boundary` | `CustomProviderDialog.validation.test.tsx` :: the inclusive id/model-id boundaries 2/40 and 100/101, `""` and whitespace-only ids/URLs/keys, `maxTokens === contextWindow`, exactly 100 vs 101 models, empty model rows dropped, non-ASCII names rejected while `Acme (self-hosted).v2-beta_1` is accepted; `ModelsPage.test.tsx` :: 50 providers × 4 models, blank/whitespace query, label≡id vs label≠id, one-model "1 model"; `BuiltinProviderKeyDialog.test.tsx` :: `""`/`"   "`/`"\t\n"` rejected, 4000-char and CJK+emoji keys, `provider = null`; `RunInspectPanel.states.test.tsx` / `RunError.test.tsx` :: empty tool list, missing optional fields, 20× repeated CJK error text; `RunsPanel.actions.test.tsx` :: 3 rows (one page), 90 rows (three pages), archived-only list |
| `error-path` | `SkillsView.states.test.tsx` :: installed-list failure vs catalogue failure kept separate, install refusal, agent omitting/including the wrong snapshot, uninstall refusal + "still returned", sync partial failure + rejection; `RunsPanel.actions.test.tsx` :: archive failure (Error + string) with no row loss, terminate failure keeping the confirm row, no-scope refusal; `RunInspectPanel.states.test.tsx` :: rejected output load keeps the card; `UpdatePage.flow.test.tsx` :: check/install/restart/manual-download failures (Error + string) with state released; `AccountPage.test.tsx` / `EnvironmentPage.test.tsx` / `ResetPage.test.tsx` :: failing IPC and lookups, `ResetPage` proving backend diagnostics do not leak; `agentClient.session.test.ts` :: unavailable and throwing `localStorage`; `useFloatingScrollbar.drag.test.tsx` :: absent selection API, unattached ref |
| `concurrency` | `SkillsView.states.test.tsx` :: a superseded refresh's late result must not overwrite the newer list (the `refreshEpochRef` guard), the exit animation holding a row while a background reload already dropped it **and that held row's uninstall control staying inert** (`keeps a held row's uninstall control inert after a background reload drops the skill`), **the catalogue uninstall control following the installed list** (`drops the catalogue uninstall control in the same commit as the installed list`), a second install/uninstall/upgrade request while one is in flight, the 1 s minimum-busy floor; `useFutureLoginFlow.test.ts` :: a re-begun attempt's late result discarded, a start rejection and an in-flight poll dropped after `cancel()`, `slow_down` widening the gate so 1 s ticks cannot poll early, `retryAfterSeconds` overriding the local back-off, the malformed counter recovering; `useFloatingScrollbar.drag.test.tsx` :: unmount mid-drag detaches the document listeners, the hide timer cleared on unmount; `useBuildInfo.test.tsx` :: a response landing after unmount is ignored; `SettingsDialog.test.tsx` :: reopening resets to `initialTab` |
| `property` | `CustomProviderDialog.validation.test.tsx` :: table-driven validation (each rule with its accept/reject table), the trimming invariant, and `create`/`apiKey` invariants across add vs edit; `ModelsPage.test.tsx` :: 7-needle filter table over label/id/provider/display name and the hide/show round-trip (`hiddenModels` ∋ key ⇔ switch unchecked); `BuiltinProviderKeyDialog.test.tsx` :: the payload is the trimmed input and nothing else; `useFutureLoginFlow.test.tsx` :: status → phase/message table for all 8 statuses; `useNow.ssr.test.tsx` :: the snapshot invariant `floor(now / interval) * interval` |
| `serialization` | `ProvidersPage.test.tsx` / `providers.test.ts` :: every catalogue/account payload round-trips through the invoke boundary unchanged (`null` vs absent, camelCase), `update_builtin_provider` distinguishing "clear" from "absent"; `agentClient.session.test.ts` :: the prompt and model-catalogue payloads verbatim, remembered model/level read back from storage (including a corrupt value read verbatim rather than repaired); `appSettings.test.ts` :: an 11-field settings patch forwarded with no renaming; `threads.pagination.test.ts` :: cursor/limit and page shape; `RunInspectPanel.states.test.tsx` :: JSON envelopes summarized into fields, unknown output kinds passed through, a `stderr`-only envelope rendered as text; `agentStateCache.compaction.test.ts` :: event payload → cache fields |
| `platform-cfg` | jsdom has no OS branches; the platform forks in this subtree are covered on both sides by mocking `desktop/src/lib/platform`: `ResetPage`'s Windows-only write-protection section (present + success/failure on Windows, absent otherwise) and `GeneralPage`'s Windows/Linux/macOS sandbox descriptions. `isMacOS`/`isLinux` are read by no other file here. `useNow`'s third argument (`getServerSnapshot`) is exercised in a **node** environment via `renderToString`, which is the only place that snapshot exists. Everything else in the subtree is platform-independent, hence `N/A`. |

Rows to merge into `docs/testing/dimension-matrix.md` (outside this task's write set).

## 5. Waivers — the 10 remaining lines

Every other file in the group is at 100 % lines. Each line below is named with
its category and the evidence for it.

| file | line(s) | category | reason |
|---|---|---|---|
| `desktop/src/features/settings/CommunityEditionSection.tsx` | 27 | `unreachable-by-construction` | `return;` of `if (selected === communityEdition)`. The only caller is the save button's `onClick`, and that button is `disabled={selected === communityEdition \|\| saving}` — disabled exactly when the guard's condition holds. React's event layer refuses to dispatch `onClick` for an element whose `disabled` prop is true. Evidence: with a changed selection the click *does* call `onChangeCommunityEdition`; with an unchanged selection, removing the `disabled` attribute from the DOM and calling `click()` still does not reach the handler (`shouldPreventMouseEvent` reads the prop, not the attribute) — so no input, however delivered, can enter the guard. |
| `desktop/src/features/settings/EnvironmentPage.tsx` | 45 | `unreachable-by-construction` | `return;` of `if (!changed)`. Same shape: the only caller is `disabled={!changed \|\| busy}` and `changed` derives from the same render values the guard reads. Evidence: the fault-injected activation (attribute removed, then `click()`) leaves `invokeCommand` uncalled, while a real selection change reaches it. |
| `desktop/src/features/settings/AccountPage.tsx` | 60 | `unreachable-by-construction` | `return;` of `if (!platformUrl)` inside `handleRecharge`. The recharge button is `disabled={balanceStatus !== "unavailable" && !platformUrl}` **and** its `onClick` is `balanceStatus === "unavailable" ? onRefreshBalance : () => void handleRecharge()` — so `handleRecharge` is only ever wired in a render where a falsy `platformUrl` implies `disabled`, and the closure captures that same render's `platformUrl`. The sibling guard in `handleOpenAccount` (line 79) *is* reachable and covered, because the account row is gated on `environment.data` rather than on the URL, so a resolved-but-empty platform root reaches it (`treats an empty platform URL as unknown for the account link`). |
| `desktop/src/features/settings/UpdatePage.tsx` | 68 | `unreachable-by-construction` | `return;` of `if (!status?.hasUpdate)` in `handleInstall`. The install control is rendered only inside the `status.hasUpdate ? …` branch of the same render that created the handler, so the closure's `status.hasUpdate` is true whenever the button exists. Evidence: once a re-check returns an up-to-date status the control disappears entirely (`offers the install control only while an update exists`). |
| `desktop/src/features/settings/UpdatePage.tsx` | 115 | `unreachable-by-construction` | `return;` of `if (!status?.downloadUrl)` in `handleManualDownload`. The anchor that calls it is rendered only when `status.downloadUrl` is truthy in that same render (`labels a platform without an asset and offers no link` asserts the anchor is absent). |
| `desktop/src/features/skills/SkillsView.tsx` | 217 | `unreachable-by-construction` | `return;` of `if (!skill)` in `runUninstall`. **The line description is corrected:** the previous version of this row described 217 as `const index = displayedInstalled.indexOf(skill);`, which is line **218** (count 6, covered), and justified the category with a v8 *attribution* artifact — neither held (withdrawn in §8). The guard is dead because a rendered uninstall control and the list its handler closes over come from the *same commit*: `InstalledTab` maps `filteredInstalled` (⊂ `displayedInstalled`) and `AllTab` gates on `installedIds` (derived from it), while a row held by an in-flight uninstall/exit operation is re-inserted into `displayedInstalled`; React dispatches `onClick` from the latest committed props, so every clickable control's id is in the list at that commit and `find` cannot return `undefined`. Measured after driving exactly the escape rev-fe proposed (a background reload dropping the row while the exit animation holds it): line 217's statement count is **0** while 216 and 218 report **6** in `desktop/coverage/d-settings/coverage-final.json`. Pinned by `keeps a held row's uninstall control inert after a background reload drops the skill` (the backend drops `alpha`, the held row is still rendered **and first in the list**, its `Cancel`/`Uninstalling...` controls are disabled, and stripping `disabled` and clicking anyway does not reach the handler — `shouldPreventMouseEvent` reads the prop) and `drops the catalogue uninstall control in the same commit as the installed list`. The index this line protects is asserted there too (the held row keeps position 0). Mutation check: deleting the guard leaves the 91 tests of the three files that render `SkillsView` green (§9), so nothing distinguishes it. |
| `desktop/src/features/skills/SkillsView.tsx` | 249 | `unreachable-by-construction` (zero-width span on the same line) | The statement after `if (skillUpgrades.length === 0) return;` in `upgradeAll`. The only caller is the "Upgrade all" button, rendered `disabled={upgradeCount === 0 \|\| anyBusy}`, and `upgradeAll` closes over the same `skillUpgrades` memo — so the guard's condition and the button's `disabled` condition are the same expression from the same render, and the guard cannot be entered. Named tests cover the surrounding path: `upgrades every outdated skill and refreshes both lists`, `reports the skills the agent failed to upgrade`, `reports a rejected sync`, `hides the upgrade button for an up-to-date skill` (count 0 ⇒ disabled). |
| `desktop/src/features/runs/RunsPanel.tsx` | 90 | `unreachable-by-construction` | `return current;` — the idempotence branch of `setRenderWindow` inside `loadNextPage`. The only caller is `handleListScroll`, which returns early when `visibleCount >= entries.length`; whenever `loadNextPage` runs, `entries.length > visibleCount = count` (same scope) or `entries.length > RUNS_PAGE_SIZE` (scope just changed), so `nextCount > count` always and the branch is dead. Evidence: the guard's other side is covered (`does not page when every row is already rendered`, `keeps the rendered window when the content still overflows after paging`). |
| `desktop/src/features/runs/RunsPanel.tsx` | 155 | `unreachable-by-construction` | `return;` of `if (archiving \|\| finishedCount === 0)` in `archiveFinished`. The only caller is the archive button, `disabled={archiving \|\| finishedCount === 0}` — the identical predicate from the same render. Evidence: `disables archiving when nothing is finished` asserts the disabled state, and the fault-injected activation in `does nothing when the panel has no conversation scope` shows a delivered activation of a disabled control does not reach the handler. |
| `desktop/src/features/settings/useFutureLoginFlow.ts` | 104 | `unreachable-in-this-environment` | `if (!current) return;` — the first line of the polling callback (`current = start`). It can only run while the polling effect is still installed (`enabled = phase === "waiting" && start !== null`) yet the callback sees a null `start`, i.e. inside React's window between committing the update that nulls `start` and flushing the passive effect that clears the interval. That window is not observable deterministically from a test: I ran one — capture the interval handler via a `window.setInterval` spy, `cancel()`, then queue a callback on the **real** `setImmediate` (captured at module scope, because fake timers replace the global) so it lands after React's scheduler task but before the passive-effect flush. It passes on some runs and fails on others with the same code, so the line's coverage is scheduler/load-dependent (it was covered in the 99.55 % run and uncovered in the next one, with identical tests). A test that "covers" it would therefore be flaky rather than probative, so it was removed; the behaviour it guards — a tick cannot poll after `cancel()` — is asserted deterministically by `cancel() returns to idle, drops the device code and stops polling`, and by the `enabled` gate itself. |

**Nothing else is waived.** Every other file in the group is at 100 % lines, and
there is no second list of outstanding files: the 10 lines above are the complete
remainder for this subtree, each on a row that names its category. In particular,
the dead-guard findings in §7 are **not** an outstanding-work list — the same
files and lines are waived here, and §7 adds no further uncovered line.

## 6. Weak-test audit (`weak-tests-fixed`)

This subtree now contains **no** `it.skip` / `describe.skip` / `xit` / `it.only`
/ `describe.only` / `todo(...)` and **no** `toMatchSnapshot`; every test asserts
an interaction result, a rendered state, a payload or a call count (the
`expect(found).toBeTruthy()` lines are DOM-lookup helpers that fail loudly when a
control is missing, and always accompany behavioural assertions).

Fixes made in this segment, in the spirit of the rule "a test that cannot fail is
not a test":

| fix | why |
|---|---|
| Deleted `useFutureLoginFlow.test.ts` "ignores a stale interval tick…" | It fault-injected React's commit/effect window and passed only when the scheduler happened to cooperate: green in the 99.55 % run, failing in the next one with the same code. A flaky test is worse than a documented gap, so the line moved to `unreachable-in-this-environment` (§5) with the evidence. |
| Replaced the 100/101-model tests' 100+ UI clicks with `initial`-seeded rows | The click-driven version took ~140 s under coverage and was measuring React's reconciler, not the model-count rule (the limit is enforced over the submitted model list). It timed out in a full run, which is exactly the "slow because it does nothing useful" smell. Now each case asserts the limit itself (100 accepted, 101 refused) in seconds, with an explicit 60 s budget for rendering. |
| Corrected four assertions that would have passed for the wrong reason | (a) `ModelsPage` group order is `localeCompare`-sorted (`DashScope, DeepSeek, FutureOS`), not catalogue order; (b) the filter count renders as `1 / 2`, and "No matching skills" belongs to a query with *zero* matches, not one; (c) `AccountPage` renders **no** description element for an authenticated session without an email (not an empty one); (d) `SkillGuideStrip` collapses the popover *before* awaiting `openExternalUrl`, so a failed open closes the popover and reports an error toast rather than leaving it open. Each correction came from the component's real behaviour, verified by widening the assertion first. |
| Dropped a test that asserted non-existent behaviour | An early draft asserted that an uninstall for a row already removed by another surface is ignored. The confirm control lives inside the row, so the row (and its control) is gone in that scenario: the scenario is impossible, and the test as written could not even reach the handler. Removed rather than kept as a decorative "race" test. |

## 7. Findings (reported, not fixed — product code was out of scope)

1. **Dead guards that the render conditions already exclude** — `AccountPage.handleRecharge` (line 60), `UpdatePage.handleInstall` (68) / `handleManualDownload` (115), `SkillsView.upgradeAll` (248/249) / `runUninstall` (216/217), `RunsPanel.archiveFinished` (154/155), `RunsPanel.loadNextPage` (89/90), `CommunityEditionSection.handleSwitch` (26/27), `EnvironmentPage.handleSwitch` (44/45). Each is a one-line, behaviour-preserving deletion candidate; per the plan they are recorded rather than deleted, since removing them would be a product change.
2. **`SkillsView` renders every model/skill row without virtualization.** The 100-row custom-provider form takes ~10 s under coverage instrumentation and the 250-model list ~0.6 s; not a bug, but a real interaction latency worth knowing about if the catalogue grows.
3. **`SkillGuideStrip`'s manual and coach actions differ on failure.** A failed manual open closes the popover (the state write happens before the `await`), while a failed coach fetch keeps it open for a retry. Both are intentional-looking, but they are inconsistent; the tests now pin the actual behaviour so a future change is deliberate.
4. **`RunInspectPanel` shows a `stderr` envelope as its own block**, not as a preview, while a `stdout`-only envelope contributes a field. This asymmetry is real (`isStructuredOutput` filters the preview list, `details.stderr` gets a dedicated block) and is now asserted.

## 8. Uncertainties

- The full-suite run reports failures in other workers' files while this subtree is
  green (`src/features/agent/AgentThread.test.tsx`, `src/features/agent/composerDraft.test.ts`,
  `src/features/filetree/useFileTree.test.tsx`, `src/components/layout/hooks/*`). Those
  files pass in isolation and are being edited concurrently; nothing in this
  subtree failed, and `--coverage.reportOnFailure` still produced the summary.
- The two `SkillsView` dead-guard lines were re-measured in two scopes, and the
  previous "zero-width range" wording is withdrawn: each of 217 and 249 carries
  exactly one statement (the `return;`, zero hits) and nothing executed on the
  same line, so the line metric and the statement metric agree at those lines —
  the file reports exactly two uncovered lines (206/208 lines, 233 statements, 231
  covered) in both the narrowed iteration report (`--coverage.include=…SkillsView.tsx`
  with its two test files in the process) and the full 245-file run. What is
  provider-specific is the enclosing `if`: its v8 range *starts* on line 216 and
  spans 217, and that statement (count 6) is why "the function ran" is true while
  the body did not run.
- Branch coverage (94.6 %) is below line coverage: the remaining uncovered
  branches are `??`/optional-chain defaults and `filter(Boolean)` arms on paths
  that are exercised. They are the natural next target if the goal extends to
  branches.

## 9. Next useful check

Scenario-mutate the waived lines to confirm the stated reasons (delete each dead
guard and confirm the suite stays green — that is the "prefer deleting" branch of
the policy). **`SkillsView.tsx:217` is done**: replacing
`const skill = …find(item => item.id === id); if (!skill) return;` with
`const skill = …find(item => item.id === id)!;` left all 91 tests of the three
files that render `SkillsView` (`SkillsView.states`, `SkillsView.broadcast`,
`src/components/layout/AppShell`) green, and the file was then restored
byte-identically (`Get-FileHash` `AFD23D0860F277688632C114F3437EC2724D5A7394BB7E1C84BAB8FD2413B180`).
Still to mutate: 249 and the eight other guards in §7 item 1. Then pick up branch
coverage for `desktop/src/features/settings/*` and the 50 %-branch
`desktop/src/lib/useIsFullscreen.ts`, which the report already flags.
