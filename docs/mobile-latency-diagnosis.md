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

Validation: Mobile typecheck/ESLint and **51 suites / 720 tests** passed; Desktop typecheck/ESLint/Stylelint and **106 files / 937 tests** passed; the shared Markdown package typecheck passed. Android and iOS production Expo exports both succeeded, including Hermes bytecode generation. No native application build/install, physical-device test, emulator frame-rate measurement, or service restart was performed.
