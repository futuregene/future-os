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
