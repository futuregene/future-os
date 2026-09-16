# Desktop streaming-output latency investigation (2026-09-12)

> This is a faithful paragraph-by-paragraph English translation of the historical snapshot [Desktop 流式输出延迟排查（2026-09-12）](./desktop-stream-lag-20260912.zh-CN.md)（2026-09-12，commit `84fb84ab`）. Conclusions, dates and commit boundaries are preserved verbatim; this translation is not a new verification result.

## Conclusions and verification scope

- Investigation session: `20260912-111503-622625`; baseline: `9452a3ed`; fix branch: `claude/desktop-stream-lag`.
- **Reproduced and fixed**: the production Markdown worker pulled in a DOM-only dependency and failed to start; worker results that fell persistently behind were discarded entirely; the dev-environment StrictMode rebuilt the worker but kept the busy state.
- **Supported but not fully end-to-end attributed**: fast, long reasoning output amplifies frontend parsing load. There is no evidence that cache overflow occurred in these DeepSeek sessions.
- Verification was performed by this implementer, not an independent review. No paid model calls were made, no existing agent/desktop was restarted or replaced, and no real session database was modified.
- Local checks completed; installed Windows WebView2 + DeepSeek live re-testing remains. The controlled experiments below must not be equated with all user lag having disappeared.

## 1. Production worker fails at startup

`streamingMarkdown.worker.ts` depends on `decode-named-character-reference@1.3.0` via remark. Vite's browser condition selects `index.dom.js`, whose top-level code executes `document.createElement('i')`. Web Workers have no `document`.

Original evidence:

- Main workspace `desktop/dist/assets/streamingMarkdown.worker-C4VrdgNY.js`, built 2026-09-12 11:00:52, contains that call.
- Running this **actual production bundle** with a newly added check script exits 1: `ReferenceError: document is not defined`.
- Local Chrome 153 / Vite pages also logged the same worker error three times, with zero successful responses. The hook then hit `MAX_WORKER_FAILURES = 3` and stayed on synchronous parsing.
- This is not a defect specific to one model; model output speed and accumulated length affect the cost of the degradation.

Fix: Vite aliases that dependency exactly to the DOM-free `index.js` resolved by Node, for both the main page and the worker (dev mode relies on pre-bundled shared modules). The dependency is declared explicitly as a dev dependency, avoiding reliance on accidental transitive hoisting layouts.

`npm run build` now automatically runs `scripts/check-streaming-worker.mjs`, which starts the real bundle in a DOM-less context and checks one chunked response. Ordinary Vitest/Node module tests select a different package export than the browser, so they previously could not catch this bundling error.

The fixed artifact `streamingMarkdown.worker-KqABNlCu.js` passes DOM-less startup and chunk verification.

Artifact SHA-256:

- Original artifact: `87BDEC3879B6FE3956A9094DFC5F0AB1CAE0770B961A2F0B6D28AF5EC27C075B`
- Fixed artifact: `F40E5611968C71480A1E0167A217F76A8674F932CDC0CA3C169550B8DBA856D2`

## 2. Worker result starvation

The original hook only accepted `response.id === latestId`. Every text update incremented latestId, even when the new task merely queued. If input outpaced parsing, every actually-completed result was stale. `provisionalProjection` could only keep splicing new text into the old tail block while the main thread parsed ever-longer text.

Fix: accept completed results that are still a prefix of the current text, letting finished blocks advance continuously and leaving only the unparsed suffix in the mutable tail block; non-prefix text replacement still rejects old results.

### Controlled browser comparison

Real `MarkdownContent`, real parser worker; fixed 60 text appends totaling 54,816 characters. Worker responses were artificially held two input ticks behind, so that background-tab timer throttling could not change the experiment conditions. Both groups used the fixed DOM-free bundling; only the result-acceptance condition differed.

| Metric | Original latest-only condition | Prefix-result acceptance fix |
|---|---:|---:|
| Successfully delivered worker responses | 29 | 30 |
| Worker errors | 0 | 0 |
| Chunk count | 1 | 116 |
| Mutable tail block characters | 54,816 | 2,290 |
| Concatenated content matches input | yes | yes |
| Longest long task observed | 51ms | no >=50ms long task observed |

This is a single controlled mechanism comparison, not a model-throughput benchmark or a latency measurement of the original user's environment. An earlier attempt with fixed 160ms delay was affected by background timer throttling and did not reliably produce sustained lag; that attempt was not used for performance conclusions.

The regression test `makes block progress when every worker response trails the token stream` failed on the original logic (chunk count always 1) and passes after the fix; there is also a text-replacement anti-cross-write test.

## 3. StrictMode lifecycle

The dev entry enables React StrictMode. Cleanup terminated the worker but did not clear `activeRef`; on re-setup the new worker thought the old task was still running and only queued, never dispatching. A new test reproduces the second worker receiving 0 requests; after the fix clearing the busy state in cleanup, it receives 1 request.

This is an additional dev-mode issue; it is not used to explain the behavior of the production executable.

## 4. DeepSeek original-session statistics and cache assumptions

The local agent/app databases were queried via SQLite URI `mode=ro` plus `PRAGMA query_only=ON`, outputting only event categories, timestamps, counts and errors — no prompt or output bodies were copied. The most recent 8 `future/deepseek-flash` runs were examined:

- In natural-second peaks by event timestamp, 152–310 events/second; event counts cannot be treated directly as token counts, and natural-second peaks cannot rule out shorter instantaneous bursts.
- `run-20260912-110338-665082`: 27,491 events, of which 23,264 thinking_delta, 3,945 tool_delta, 50 tool_end, **no text_chunk**. The final error was hitting the 50-round tool-call limit, not body text lost on the desktop side. Agent-vs-desktop terminal persistence times differed by about 19ms; this does not represent per-frame UI latency.
- `run-20260912-111008-9382ed`: 7,521 events, of which 7,075 thinking_delta, about 27,061 reasoning characters, **no text_chunk**; finally cancelled by the user.
- Some earlier cancelled runs were recorded as completed on desktop but cancelled in the agent — another terminal-state mapping phenomenon, not modified in this pass.

Actual capacities/throttling in the code:

- Agent current-run replay ring: 2,000 events; out-of-range events can be replayed from the persistent journal — exceeding 2,000 does not drop body text.
- Broadcast ring: 4,096 events; falling too far behind explicitly reports DataLoss and reconnects.
- gRPC single-message limit: 32 MiB; desktop requests paginate by event count/byte budget.
- Desktop live projection has a 100ms coalesced refresh, so small display delays are by design.

The hypothesis that paging continuously chases newly appended events was considered, but the recorded scale/rate plus the explicit worker reproduction did not support it as the primary cause; no cache sizes were adjusted. Existing data lacks synchronized UI frame times and subscription cursors, so transport/backpressure issues cannot be absolutely excluded.

## Check and reproduction entry points

In `desktop/` of the fix worktree:

```powershell
npm run lint
npm test
npm run build
```

Results: full ESLint + `tsc --noEmit` pass; 101 test files / 873 tests pass; production build and the new worker smoke check pass. The build still has the existing CSS `::highlight` optimization warning; tests have non-failing output such as the Node localStorage experimental notice.

The bundling failure can be independently reproduced while the old artifact remains (argument is the assets directory):

```powershell
node scripts/check-streaming-worker.mjs D:/future-os/desktop/dist/assets
```

The key permanent regression lives in `desktop/src/features/markdown/useStreamingMarkdownBlocks.test.ts`. Temporary browser entries, database statistics scripts and CDP reader scripts were cleaned up, and the test Vite server was stopped; no model tasks or paid background tasks remain.

Next step: build the fix into the desktop executable and, when the user next restarts the desktop app, re-test the same long reasoning/tool-call scenario; if lag persists, record agent idx, desktop read cursor and WebView long-task times together to locate the remaining link.

## Supplement: freeze after running A → B → A

On 2026-09-12, within the same investigation session, the user supplied explicit session-switching steps. With `84fb84ab` (already containing #564) as baseline, a separate branch `claude/desktop-reattach-stall` reproduced and fixed two further state-restoration problems, **independent of Markdown, output speed or event-buffer capacity**.

1. **A still running**: `threadMessageCache` restored the `pending-*` streaming bubble created by the local send. `upsertStreamingPreview` always looked up bubbles by `stream_<runId>`; `streamingBubbleBase`, seeing a same-run message with another UI id, treated it as an already-persisted end message and rejected every update. Fixed by taking over the original id for the same-run `streaming` message, preserving DOM identity and subsequent incremental refreshes.
2. **A finished in the background**: the initial history refresh did not enable terminal-state reconciliation, and `reconcileThreadHistory` unconditionally kept cached streaming messages. The new component's first read of recentRun was already terminal, so no active → terminal transition ever fired a finishing refresh. Fixed by allowing authoritative history to reconcile the cache on initial load; new writes during the request remain protected by the existing request-time baseline.

Reproduction entry point:

```powershell
cd desktop
npx vitest run src/features/agent/useRunReattach.switch.test.tsx
```

The tests use real `useThreadMessages`, `useRunReattach`, projector, cache and reconciliation; only the storage IPC and Tauri event transport are replaced. A → B → A is executed via React keyed mount/unmount. Both initial cases fail on the original code: after switching back it still shows `before switch`, and after background completion it is still `streaming`. After the fix, verified:

- Deltas arriving while away are received, and new events keep arriving after switching back; two consecutive switches keep a single original-id bubble.
- A's content does not enter B.
- Background `completed`, `failed` and `cancelled` states all end the cached streaming state.

Final checks this round: 102 test files / 877 tests, ESLint, TypeScript, production build, worker smoke — all pass. This is frontend integration reproduction and implementer self-checking, not yet click-through end-to-end validation on an installed WebView2; no real database was modified, no paid model called, no agent restarted and no user-running desktop replaced.
