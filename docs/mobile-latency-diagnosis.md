# Mobile history-loading latency: reproduction and verification

Date: 2026-09-12. Baseline: `c5e87eec`. Local platform: Windows.

## Diagnosed causes

1. Mobile's cold-open sync waited for the entire active-run replay before committing already-available history. Its 15-second history-loading timer could therefore expire even though history succeeded and every individual replay request succeeded.
2. Each remote replay page consumed at most 100 events by default, but Desktop first read the complete remaining Agent tail, then paginated it. Long runs repeatedly decoded and transferred the same suffix over local gRPC.

Basic probes did not reproduce a transport outage: recent history reads took approximately 5–151 ms; another eight long-history samples took approximately 82–415 ms. Computer-side NATS WebSocket connections took 145/158 ms and PING/PONG approximately 15–18 ms. These are not measurements of the phone's network, handshake, or rendering.

## Fix

- Publish a cold-open history preview before awaiting replay. Do not advance the replay cursor or mark the lane established. A failed replay leaves history readable and retries its full prefix. Ignore stale state/history completions after a lane is replaced.
- Preserve the existing replay protocol: the first full-tail read establishes its fixed watermark. Continuations carrying that watermark and an event cursor request one bounded Agent page instead of draining the suffix. Preserve Agent `hasMore` as well as the remote byte budget, and stop at the original watermark when the task keeps generating.
- Native full-tail consumers and legacy offset callers keep their existing behavior. No protobuf changes, new credentials, or pairing changes are required.

The first snapshot is still a full-tail read; this change removes repeated full-tail reads, not that initial cost. Oversized single-exchange history/projection hardening is separate from these two verified fixes.

## Measurements

A read-only probe called the real running Agent over the current user's named pipe for the same retained task and fixed watermark: 7,290 events, 73 cursor pages.

| Read strategy | Total events read | Local probe elapsed |
|---|---:|---:|
| Baseline: read full remaining tail on every page | 269,370 | 6,649 ms |
| Fixed strategy: initial full snapshot, bounded continuations | 14,480 | 496 ms |
| Bounded-everywhere control, without initial snapshot cost | 7,290 | 280 ms |

The probe reproduces 100-event cursor progression to isolate local read amplification; it is not a phone end-to-end benchmark. Elapsed time includes local RPC and probe protobuf handling, but excludes Desktop JSON processing, relay/phone networking and React Native rendering. Real byte-budget cuts may require additional pages. The services were not restarted or replaced during these probes.

A deterministic probe executed the actual Mobile `SyncEngine` and `fetchEventsSince` code with 4,800 synthetic events and explicitly simulated 350 ms per-page latency:

| Observation | Baseline | Fixed |
|---|---:|---:|
| History available | 0 ms | 0 ms |
| First timeline commit | 16,800 ms | 0 ms |
| Timeline absent at 15 seconds | yes | no |
| Replay request count | 48 | 48 |

The zero is fake-clock time, not a real-world zero-latency rendering claim. Durable automated regressions now cover early history, replay failure/retry, and the actual history-loading hook remaining free of timeout while replay is delayed for 20 seconds.

## Local checks

- Mobile typecheck and ESLint: passed.
- Mobile Jest: **49 suites, 702 tests passed**. On Windows, passed `--testMatch '**/__tests__/**/*.test.ts'` to avoid mixed-separator expansion of `<rootDir>`; all Mobile suites ran.
- Desktop `cargo fmt --check`: passed.
- Desktop `cargo clippy --all-targets -- -D warnings`: passed.
- Desktop replay-focused tests: 11 event-reader tests plus the remote command/Agent integration regression passed. The latter checks initial watermark, intermediate `hasMore`, and termination despite newly appended events.
- Desktop binary test: passed.
- Desktop full library suite: **1,110 passed, 26 failed**. An unmodified-baseline run in the same worktree/environment produced **1,108 passed, the identical 26 failures**. The two added Rust tests account for the increase in passes. Baseline binary test passed; its one doc test is ignored.

The unchanged Windows failures comprise three path assertions; sixteen `git_review` fixtures failing `File::set_modified` on read-only handles; four shadow-repository fixture/platform failures; and three remote fake-server scheduling assertions. They were not hidden, skipped, or repaired as unrelated changes. Linux CI and the three-platform build checks remain the merge gate.

## Remaining validation boundary

No real-phone failing request trace was captured in this session. These results establish the reproduced history/replay defects and their automated fixes, not a claim that every mobile connection failure is resolved. Installing updated Mobile and Desktop builds is required to exercise both fixes on a physical device.

## Follow-up: switching and foreground synchronization (no physical device)

The next Mobile-only change shares the opening `get_state` promise between model controls and the sync engine, removes the redundant opening attachment-history fetch, and rejects obsolete navigation responses. Model/thinking commands capture the intended session before asynchronous preference writes. Draft preference loading cannot navigate over a newer selection.

Only the selected conversation performs history/replay/projection work. Hidden text events do not create timeline lanes, while title, terminal and approval notifications still update the catalog. Reopening always refreshes durable history and the active prefix, so a task that finished or requested approval while hidden is recovered. Reconnect restarts visible lanes, and replay checks lane validity before and after each page. An already-issued request may finish or time out; no subsequent pages are issued for the obsolete lane. This does **not** change the wildcard transport subscription or eliminate its incoming JSON decode cost.

Deterministic regressions use actual Mobile hooks/sync/replay code with scripted RPC promises and fake timers:

- With a simulated 350 ms state response and 350 ms history response, the opening state is requested once, history once, and history commits at 700 ms. This verifies two sequential round trips rather than the previous duplicate-state dependency; it is not a phone rendering benchmark.
- 100 hidden sessions receiving 10,000 text deltas cause zero history/replay requests and no new timeline allocations. Opening one of them loads its latest durable state.
- Reconnect with multiple cached lanes refreshes only the selected lane; reopening an idle hidden session refreshes its completed history.
- Late A success/failure cannot overwrite B's model or clear B's pending state. Deferred preference saves cannot send A's command to B.
- Hidden/replaced replay stops at the in-flight page; approvals are restored when opened; live events arriving during an open do not enqueue a duplicate opening read.
- A warm conversation's ten-exchange display window stays bounded through an immediate restart, with older history still reachable by pagination.

Local validation: Mobile typecheck and ESLint passed; 50 suites / 718 tests passed. One initial full run hit an unchanged SessionList beforeEach 5-second timeout; both its isolated rerun and subsequent full runs passed without changing timeout settings. No physical device, emulator frame-rate measurement, production service restart, or model request was used. Markdown incremental rendering and fine-grained Context subscriptions remain separate optimization work.

## Follow-up: incremental Markdown and render isolation (no physical device)

Mobile now uses a shared streaming parser with one current checkpoint per message/segment. It retains completed top-level nodes, reparses the mutable tail, and incrementally reparses the last GFM table row while retaining completed rows and header/alignment objects. Transient fragments bypass the shared settled-document cache. Replacement, finalization, bracket-bearing source (conservative reference-definition safety), and unsupported checkpoint shapes use canonical full parsing. A large single paragraph/list/code block can still require parsing that entire mutable block; this is not a claim of constant-time parsing for every Markdown input.

`MarkdownText` memoizes completed blocks and table rows. Remote control state is separated from transcript state while preserving `useRemote` for transcript consumers. Navigation and share intake subscribe only to controls. The composer receives stable control state/callbacks and compares approval item identities, so ordinary text commits do not redraw it, but changed streaming state, callbacks and approval content remain observable.

The shared parser's 28 tests include character-by-character equivalence to canonical parsing across 21 mixed Markdown fixtures, immutable block/row identity, replacement/reference/finalization handling, cache isolation, deep-nesting fallback and two bounded-work loads. One isolated Node/Vitest run on Windows measured initial content plus 100 appends:

| Load | Full-parser input characters | Incremental input characters | Full parse elapsed | Incremental elapsed |
|---|---:|---:|---:|---:|
| 200 stable paragraphs plus growing tail | 721,544 | 12,544 | 2,337 ms | 49 ms |
| 200-row table plus appended rows | 516,367 | 10,598 | 3,081 ms | 74 ms |

Both paths bypass settled-document caching for this comparison; full parsing uses the canonical parser, incremental parsing uses actual production code. The character-count assertions are deterministic. Timings are single-run host observations, not phone FPS, p95 latency, rendering time, or a universal speedup guarantee.

React test-renderer checks show 100 transcript commits with no additional control-consumer renders; streaming start/end still render controls. A composer test similarly checks that 100 unchanged approval-array reconstructions do not redraw its docked cards, while new callbacks and streaming state do. Incrementally rendered and finalized Markdown matches ordinary rendering.

Validation: Mobile typecheck/ESLint and **51 suites / 720 tests** passed; Desktop typecheck/ESLint/Stylelint and **107 files / 943 tests** passed; the shared Markdown package typecheck passed. Android and iOS production Expo exports both succeeded, including Hermes bytecode generation. No native application build/install, physical-device test, emulator frame-rate measurement, or service restart was performed.

## Follow-up: oversized reads, cache budgets and refresh coalescing

History/replay callers advertise `chunkedRead`. If a logical read result exceeds 512 KiB, Desktop retains an immutable JSON snapshot and returns 192 KiB binary slices encoded as base64url. Mobile checks the snapshot identity, exact offsets, chunk size, total length and current sync lane before reassembling and decoding the complete result. This preserves the existing history-page grouping and projection cursor: splitting one run into independently projected message pages could otherwise lose an overlapping assistant bubble. Ordinary replies and old Desktop replies remain compatible.

Limits are explicit: 16 MiB per assembled read, 32 MiB of cached snapshot payloads, 120-second expiration, and ownership by bridge/session/run. Chunk requests pass the normal authentication/access checks but do not duplicate every chunk in the ten-minute command reply cache. Snapshots are pruned and cleared with transfer lifecycle maintenance. All final command replies additionally enforce a 1 MiB **uncompressed JSON** ceiling; unsupported/oversized results return an explicit error rather than relying on NATS to reject publication. This does not remove the Agent's existing gRPC limit or the existing per-entry presentation content caps. Data beyond the new read limit fails explicitly; it is not silently truncated by this transport.

Mobile navigation now evicts least-recently-opened inactive conversation caches toward eight sessions / 16 MiB estimated serialized UTF-16 payload. Cursor state, retries, queued operations and the hook's corresponding timeline/paging/error records are removed together. The selected conversation and optimistic draft are protected; these are explicit exceptions rather than truncating content being read. Size estimates are cached until the timeline changes and evaluated on navigation, not on every streamed token; they are not exact JavaScript heap measurements.

Concurrent session-list refreshes share one in-flight request and a trailing read when another event arrived during it. A 100-call burst test produces two requests, not 100, and ends with the fresh result. Client/epoch changes fence old responses.

Validation on Windows:

- Mobile typecheck and ESLint passed; **52 suites / 736 tests passed**. An unchanged SessionList beforeEach timeout on the first cold run passed on isolated and full reruns without relaxing its threshold.
- Rust fmt and clippy (`--all-targets -- -D warnings`) passed. Four new Rust tests cover byte-exact Unicode snapshot reconstruction, ownership/offset/expiry/capacity, the final reply guard, and actual Desktop host/NATS-reply/Agent-mock paths for a large projection and a single 13-entry exchange.
- Full Desktop library suite: **1,113 passed / 27 failed**. An unmodified `3034fcc0` run in the same worktree/environment produced **1,109 passed / the identical 27 failures**. The extra bridge-start timing case also passed in isolation. Binary tests passed; the existing doc test remains ignored. These Windows baseline failures were not hidden or skipped.
- Android and iOS production Expo exports, including Hermes bytecode, succeeded.

The tests also cover malformed/over-budget chunks, navigation cancellation, cache eviction followed by fresh reload, stale work completing after eviction, and old-client response compatibility. No physical-device performance claim is made. Updated Mobile and Desktop builds are both required for oversized-read chunking.

## Follow-up: large replay folding and history look-ahead

The Mobile replay reducer now owns one deduplication set per replay, rather than copying the growing set per event. Shared run projectors support arrival-ordered append without snapshot construction, explicit snapshots, and independent forks. Contiguous run events build one display snapshot; user/approval/error/notice and run boundaries flush it to preserve the single-event reducer's ordering. Existing `ingest` batch sorting/duplicate semantics remain unchanged.

Replay folds cooperatively, yielding between slices after 512 events, approximately 256 KiB of event data, or an 8 ms work target. These checks occur between events: initial state forking, a single large payload and final snapshot construction are not preemptible, so 8 ms is not a guaranteed maximum task duration. The committed timeline/projector and cursor remain untouched while replay runs. A finished result installs its timeline and cursor together; cancellation/restart discards partial work. Projection snapshots retain their accumulator so subsequent live text appends to, rather than replaces, their prefix.

History paging no longer waits for a fixed 1.5-second marker deadline. It still waits for requested IDs to commit, a native layout observation and the existing 100 ms quiet window, which can extend while layout keeps changing. Deliberate scrolling may request one page within one viewport of the history boundary (capped at 600 layout points). The existing transaction and per-gesture guards prevent programmatic scrolling/momentum from cascading through multiple pages. Idle text updates no longer rebuild the whole paging ID index.

Validation uses real reducer/projector/hook implementations with synthetic events, not a physical device:

- 10,000 / 50,000 / 100,000 text events (plus start/end) each produced one display snapshot and exact text/token totals. An independent timer ran before completion (after 1,024 events in the recorded run).
- A 5,002-event comparison produced 5,002 snapshots with single-event folding versus one with batching. A recorded full-suite run measured 210 ms single-event, 15 ms synchronous batch, and 136 ms cooperative batch including timer scheduling. These are host observations, not phone latency or FPS, and no wall-clock ratio is a test gate.
- Tests cover 300 interleaved tool calls, thinking, usage, compaction, approvals, errors, truncation, duplicates/out-of-order/untracked events, cancellation without mutating the committed prefix, restart without partial cursor publication, and projection-to-live-tail continuation.
- Paging tests cover early one-page look-ahead, fast-page completion without the old cooldown, native-layout barriers, stale session completions, and 100 idle text updates across 1,000 rows without ID-index reads.
- Mobile typecheck/ESLint and **53 suites / 752 tests** passed after syncing main; Desktop typecheck/ESLint/Stylelint and **109 files / 959 tests** passed. Shared thread-projection typecheck and Android/iOS production Hermes exports passed.

This optimizes reconstruction after fetching replay, not the existing first full-tail read or retention of all fetched event pages. Block-level virtualization inside a giant message, data-source byte paging, and a bounded active-history window remain separate work. No native install or physical-device performance measurement was performed.
