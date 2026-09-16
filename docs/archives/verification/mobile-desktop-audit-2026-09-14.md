# Mobile / Desktop Remote review (2026-09-14)

> This is a faithful paragraph-by-paragraph English translation of the historical snapshot [Mobile / Desktop Remote 审查（2026-09-14）](./mobile-desktop-audit-2026-09-14.zh-CN.md)（2026-09-14，commit `617a7da1`）. Conclusions, dates and commit boundaries are preserved verbatim; this translation is not a new verification result.

## Scope and evidence boundaries

Source review started at `7ded5702` and synced to `d363beb6` before commit. This is a risk-driven source review, protocol cross-check and full mobile automated test pass — not a proof of freedom from defects for every line of code, every device or live services.

Key areas checked:

- Mobile `client.ts`, `useRemoteConnection.ts`, `pairing.ts`, `storage.ts`: connection generations, candidate handover, renewal, trusted pairing entry points, isolation of multiple Desktop credentials.
- `secureChannel.ts` and Desktop `remote/commands.rs`, `remote/transfer.rs`: encrypted channel readiness, command and reply routing, file chunking, duplicate-command semantics.
- `syncEngine.ts`, `usePromptOutbox.ts`, `useSessionCatalog.ts`, `readPages.ts` and Desktop `remote_host/business.rs`, `read_pages.rs`: send receipts, replay watermark, paging, catalog and approval state.
- `files.ts`, `useConversationController.ts`, `useFileDownload.ts` and Desktop `remote_host/files.rs`, `session_files.rs`: cache identity, cancellation, late results, size limits, Windows paths and protected-file boundaries.
- App/chat input and drafts, system share import, update flow; key IO and file-access paths of Android `ShareIntentStore.kt`, `FileHandlerModule.kt`. The remaining UI is covered by existing mobile tests; no per-component physical-device validation is claimed.

The first-round #604 product-code changes are limited to mobile TypeScript; Desktop Rust, the NATS message format, pairing trust and approval-execution rules are unchanged. The second-round changes are listed separately at the end.

## Fixed / hardened

| Item | Original behavior and impact | Fix and regression evidence |
| --- | --- | --- |
| P1: preview metadata reused across Desktops / sessions | The global index used only path, filename and variant; identical paths on two computers, or identically-named relative paths in two sessions, could directly show the cached file of another origin, skipping the current Desktop's prepare | Index isolated by RemoteClient and session; test cases for same path across different Desktops and different sessions. Disk storage still deduplicates by content hash — identical bytes from different origins are not stored twice |
| P2: link still shows old content after model overwrites a same-path file | Markdown file links could skip prepare by default; unlike immutable attachments, generated files can be updated in place | File links revalidate metadata by default while still reusing the byte cache for identical content hashes; tests cover both explicit directory refresh and the Markdown default path |
| P2: prepare result not reusable after first download | prepare registers metadata first; when the UI checked the cache before byte download, the entry was deleted because the file did not exist yet; after download completed, prepare had to run again | Cache misses retain bounded metadata, at most 128 entries per client; tests cover prepare → miss → file on disk → hit and capacity eviction |
| P2: cancelled prepare still occupies a Desktop temporary transfer | The upper-layer AbortSignal completed cancellation first, the lower-layer request succeeded later and its transferId was never released; pre-cancelled calls also issued the RPC first | Check cancellation before issuing; observe late success replies and send download_cancel best-effort; tests cover both orderings. Permanently lost replies still rely on the Desktop TTL; no claim of cancelling an already-unknown transferId |
| P2: download re-prompts after cancellation / unmount | File reads, downloads or errors completing after cancellation — handoff did not verify the handle and could overwrite a new preview or pop a stale error; on iOS a handoff already being dismissed no longer belongs to the active ref | Handoff validates object identity and cancellation; cancellation and unmount both invalidate the pending display handle; original-image prepare cancellation releases the download channel in finally. Tests cover late reads, unmount errors, retry after original-image cancellation and the iOS onDismiss queue |
| P2: approval replies from an old Desktop pollute new state | After set_approval_tier / approval_decision succeeded, the current setter / syncEngine was used directly without checking the request origin | Client identity validated before write-back; tests cover tier and timeline replies after replacing the Desktop. Only the phone display is corrected; the Desktop's actual approval result is unchanged |
| P2: late attachment prepare backfills a new session | selectedRef was only read when the request started; navigation identity was not checked after the response | Client, session and conversation epoch checked; on mismatch the original transfer is cancelled and a cancellation error returned; three navigation-change categories tested separately |
| Defensive hardening: download chunk size | Non-positive / non-integer chunkBytes could cause an error loop; chunk length was only validated after the whole file completed | Finite, safe-integer and total-size caps checked before download; expected length validated per chunk before writing to disk. Tests cover 0, negative, NaN, Infinity, fractional chunkBytes; existing size/hash-mismatch tests still pass |

Test entry points:

- `mobile/src/remote/__tests__/files.test.ts`
- `mobile/src/remote/__tests__/useConversationController.test.ts`
- `mobile/src/features/chat/__tests__/useFileDownload.test.ts`

## First-round follow-up issues and optimization suggestions (not fixed in #604; later handling at the end)

These items were not "green-washed" by this test pass; they should be reproduced, evaluated and covered by tests separately.

1. **Android share import should throttle during the copy, not check size after the copy (priority P2).** `ShareIntentStore.take` calls `input.copyTo(output)` first and only then compares against the 25 MiB cap; an oversized or endless data source can consume significant disk first, and the error branch does not explicitly delete the partially written file. Suggest a cumulative byte cap, batch total/count caps and finally cleanup, with a native regression using a controlled ContentProvider. The source check order is confirmed in code; no disk-exhaustion or native fault injection was performed this pass.
2. **Session-title overrides need an explicit invalidation protocol (P2, weak-network timing pending reproduction).** `useSessionCatalog.applySessionSnapshot` always prefers titleOverrides. If the phone misses a later Desktop rename event, the catalog snapshot may fail to correct the old name. Suggest evicting overrides via an authoritative version/acknowledgement watermark rather than simply deleting them, which would reintroduce the old race of stale snapshots overwriting a fresh rename.
3. **Resource reclamation for attachment conversion and the export directory (P3).** `files.ts` multi-file Promise.all does not reclaim completed temporary conversions when one item fails; `futureos-exports` has no capacity cleanup like the preview cache. Temporary-file ownership is needed, balancing unsent drafts and URIs still held by external apps, to avoid premature deletion.
4. **Test hygiene (P3).** Fully-passing legacy connection/catalog tests still emit React act/overlapping-act warnings, and some fault injections produce expected console.warn/error output. Suggest converging act lifecycles one by one and asserting expected logs, rather than globally silencing console, which would mask real failures.

## Verification

Windows x64 worktree; Node **24.21.0** (same major as CI Node 24) executed directly:

```powershell
# in mobile/; temporarily select Node 24 without replacing the user's global Node
npm exec --yes --package=node-win-x64@24.21.0 -- node ../node_modules/typescript/bin/tsc --noEmit
npm exec --yes --package=node-win-x64@24.21.0 -- node ../node_modules/eslint/bin/eslint.js .
npm exec --yes --package=node-win-x64@24.21.0 -- node ../node_modules/jest/bin/jest.js --runInBand
```

- TypeScript, ESLint: passed.
- Jest: **81 suites, 1023 tests passed** (including #603 tests that came in via sync). This PR adds 20 regression cases.
- The same scope also passed on the local Node 26.4.0; the Node 24 result above is the local evidence aligned with the CI major version.
- Rust was not modified, so the full Desktop Rust test matrix was not rerun for this PR; the protocol cross-check is source evidence, not a claim of new passing Rust tests.
- No Android/iOS native builds, real-device camera/album, system share dialogs, lock-screen background, real NATS/platform disconnects or Desktop/Agent restart joint testing was performed; no user pairing keys were read and no fault injection into the running Agent.

Suggested physical-device acceptance: same path with different content on two Desktops; identically-named relative links in two sessions; cancel mid-download and immediately open another file; return to the list / switch Desktop during prepare; return while the iOS download dialog is dismissing; phone offline-and-reconnect during a Desktop rename; Android share sources above 25 MiB.

## Second round: remaining issues fixed (based on `617a7da1`)

This section is a follow-up increment; it does not rewrite first-round unverified content as first-round-passed.

- **Android share copy cap and rollback**: extracted the `ShareFileCopier` JVM IO boundary actually used; at most 25 MiB written per file, per-batch cumulative read budget 50 MiB and at most 10 items attempted. At most one extra byte distinguishes an EOF exactly at the cap from an over-limit source; over-limit, zero progress, empty files and read/close/output-open errors all close streams and clean up partial output, and later metadata failures also clean up. Budget consumed by failed reads is not refunded, preventing multiple oversized sources from repeatedly consuming disk. The existing composer limits of 10 MiB/file and 20 MiB/message are unchanged.
- **Title eventual consistency**: title overrides are written to a ref synchronously; a confirmed push removes the corresponding override. Catalog requests capture the override object at request time, and the returned snapshot may remove it only if that object is still the current one. A merge-style refresh is triggered after a successful rename; old requests keep the new override, and only later reads confirm it. Explicit reads allow same-version confirmation but still reject older versions and wrong epochs; live pushes keep strict deduplication. Covers rename events missed by overrides, same-version replies, old reads issued before the rename and later refresh races.
- **Attachment conversion failure cleanup**: all three batch conversion paths (file, album, share) wait for all conversions to finish and, on failure, clean up only the newly generated temporary outputs of that batch; user originals, existing draft attachments and retryable share inputs are untouched. Oversized/empty conversion outputs are also deleted. When the SDK throws without returning an output URI, its internal temp path cannot be inferred — such cleanup is the SDK's responsibility.
- **System export cache**: each export uses a separate directory carrying its creation time, preserving the original filename but not overwriting same-named files already handed to other apps. Cleanup runs only on later exports and removes copies at least 24 hours old; source files and drafts are excluded from cleanup. Capacity is 100 MiB / 128 items; copies still within the retention period are not deleted to make room, and insufficient capacity returns failure. Preparation operations are serialized and failed copies cleaned up. Directory timestamps avoid deleting a just-exported file when copying preserves an old mtime. If a system app still holds the old URI after 24 hours, continued readability is not guaranteed; this is not permanent file archiving — use the system save action.
- **Test lifecycle**: removed nested async act in connection-test callbacks and awaited the async teardown of unbind/unmount; catalog-test async startup moved into act. Both test groups keep real console.error output and assert zero calls, preventing unwrapped/overlapping act warnings from creeping back in. Fault-injection product diagnostic logs remain visible; console is not globally silenced.

Native IO tests run with `python scripts/test-mobile-share-io.py`; Java 17+ reused, JDK via javac, JRE-only falls back to a temporarily pinned Eclipse compiler. JUnit/Hamcrest/compiler are fetched from Maven Central with verified pinned digests; the temp directory is cleaned up on exit. The Android module also declares the JUnit test dependency and adds a path-scoped `Mobile native IO` CI. It verifies real Java file IO, not posing as a full APK/Kotlin/ContentResolver integration test.

Second-round local validation: Windows x64 / Node 24.21.0, TypeScript and ESLint pass, Jest **81 suites, 1040 tests pass** (15 added this round, including list regression cases synced to `e026af6c`); Java 17.0.20.1 real IO tests **9 pass**. Test dependency temp directories were cleaned up automatically.

The second round still did not perform Android/iOS native builds or physical-device tests; key supplementary verification: malicious/oversized ContentProvider, share permission invalidation, disk full during copy, Desktop rename while offline, long-held export URIs. Unmerged commits on local `main` were not modified.
