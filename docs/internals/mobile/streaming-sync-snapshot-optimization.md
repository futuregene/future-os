# Cold start / cache invalidation: snapshot + incremental optimization and real A/B (2026-09-16)

> ([中文](streaming-sync-snapshot-optimization.zh-CN.md))

## Completed behavior changes

Normal reopen with a complete cache keeps using the existing highWater
incremental path. When no usable baseline exists and recovery must start from
`-1`, mobile no longer downloads all token events of the current run by
default:

1. Existing history loading is kept: the latest **3 user exchanges**; scrolling
   up still uses the history interface.
2. The first `get_events_since` carries `preferSnapshot: true` and
   `chunkedRead: true`.
3. The new Desktop asks the Agent for `get_run_snapshot`, getting the merged
   semantic result and a consistent cursor N.
4. Mobile then reads one increment from N, pinning its boundary M, covering
   events produced while the snapshot was being downloaded.
5. The "snapshot + complete increment N…M" is restored as one projector and
   committed together with the cursor. Queued live events are then deduplicated
   by cursor; terminal state and stats apply exactly once.

This is not "keeping the last few tokens" or "hiding the sync hint directly",
and the saved history/model context is not modified. It keeps the current run's
complete visible semantic state while skipping the transfer and per-item
processing of the many intermediate increments from its generation.

**Boundaries that remain**: three rounds of history is not three raw messages,
nor a strict byte cap. The current run's large tool outputs are still inside
its snapshot; no tool-detail lazy loading was added this round. Special cases
— old desktops or snapshots over the budget — still compatibly fall back to the
original paged replay; no claim that full download never happens in all cases.

## Real A/B results

Using the same SQLite consistency backup, the same new Agent executable, the
same new DesktopHost test program, and Chrome, two modes were compared:

- `raw-baseline`: forces `preferSnapshot=false`, the original full raw-event
  replay.
- `snapshot`: enables the new path. Not comparing a new release against an old
  debug build.

Three samples — largest, second-largest, and median by event count — were
chosen from 236 completed runs, each measured three times, 18 total. The second
round inverted the mode order to avoid raw always being measured first. A new
SyncEngine each time, no reused client prefix; OS file cache not cleared.

| Raw event count | Raw replay time | Snapshot + increment time | Replay-path requests (incl. chunks) | Recovered semantic records |
| ---: | ---: | ---: | ---: | ---: |
| 111,395 | 12.790–12.846 s | **0.403–0.428 s** | 112 → **6** | 787 |
| 65,632 | 7.487–7.607 s | **0.267–0.278 s** | 66 → **5** | 474 |
| 2,244 | 0.226–0.237 s | **0.049–0.060 s** | 3 → **2** | 60 |

The heaviest sample's sync-completion median **12.8211 s → 0.4109 s**, down
about **96.8%** (~31×). That sample's replay-path actual response volume
**38,782,350 → 1,204,056 bytes**, down about **96.9%**.

Request-count semantics: the largest sample is "1 snapshot request + 4
follow-up chunk requests + 1 increment-verification request", 6 total; plus one
state and one history request each. The 4 follow-up chunks go through #647's
bounded-concurrency reads, not serial chunk-by-chunk again. The small sample
needs no chunks — just the snapshot and increment requests.

Byte counts are measured by the browser's `arrayBuffer().byteLength`,
including chunk base64 and the app JSON envelope, excluding HTTP framing — and
certainly not NATS/E2EE network bytes.

### Consistency checks

All 18 runs completed successfully, and:

- Final committed highWaters were 111394, 65631, 2243 respectively, with
  prefixComplete true.
- The new path does not hand raw events back to SyncEngine as replay results;
  it delivers a restorable projection.
- The raw and snapshot modes' complete `timeline.items` JSON hashes match,
  covering these samples' visible fields — bodies, thinking, tool state,
  attachments, and terminal stats.
- Results are stable across three alternating-order rounds on the same snapshot
  and binaries. Data integrity was not skipped by only verifying "the hint
  disappeared".

### Measurement boundaries, not overstated

- Agent: Rust 1.97.0, `cargo build -p future-agent --release`; version string
  `future-agent 0.0.0-6809f472+local.dirty`. Measurement binary SHA-256:
  `6d938c79bee1da7d93b7360493ac9d4ef6380f7db29cf4f1bf7b01adc0a920ea`.
- Desktop adapter: `cargo test --no-default-features`, still unoptimized +
  debuginfo. Both groups share this build — absolute times and phase ratios
  cannot be generalized directly to a release Desktop.
- Chrome 153 / V8, macOS, local HTTP → real DesktopHost → real isolated Agent
  gRPC → real data backup. HTTP substitutes NATS/E2EE; no injected RTT/fake
  clocks, no throttling.
- Samples are completed runs with the replay target explicitly specified via
  SyncEngine's public reconcile API — no forged activeRun. The history window
  is the session's latest history at backup time, not the window at generation
  time.
- This measures historical runs building snapshots from the database; a
  continuously running current run can read its maintained in-memory projection
  directly — that path has concurrency/consistency tests, but this round does
  not pass for real-device live-streaming measurement.
- No phone Hermes, React Native rendering, cellular/Wi-Fi, NATS encryption, or
  packet loss was tested. 0.41 s is this round's largest-sample median, not a
  phone SLA or the latency ceiling for all sessions.

## Protocol and compatibility

### Agent

New read-only `get_run_snapshot`, using the existing RpcCommand's
`type/sessionId/runId` and the common JSON response carrier; no protobuf fields
added or reused. Added to the shared command policy's SafeRead/Storage
classification and does not load LLM history context.

Return shape:

```json
{
  "runSnapshot": true,
  "events": [],
  "watermark": 1000,
  "nextSinceIdx": 1000,
  "hasMore": false,
  "projection": {
    "runId": "r",
    "cursor": 1000,
    "events": [
      {"type": "agent_start", "runId": "r", "idx": 0, "data": "{}"},
      {"type": "text_chunk", "runId": "r", "idx": 1000, "data": "{\"text\":\"merged reply\"}"}
    ]
  }
}
```

- Current run: clones the in-memory projection and cursor inside the same event
  stamping lock, avoiding "new cursor with old body".
- Earlier runs: one SQLite transaction reads the consistent prefix, merged in
  batches inside the Agent — **the whole raw log is not downloaded to the
  phone**. The current-run lock is released while reading/merging history; the
  historical fold is not inside the active broadcast critical section.
- Before recovery, raw index contiguity and run identity are verified; known
  journal gaps or health errors are not disguised as complete snapshots.
- Batch folding has the same semantics as the existing live projection: merges
  adjacent same-stream text/thinking/tool deltas, keeps boundaries, tool
  terminal state, approvals, usage, error, and agent_end; the raw provider
  `text_delta`/`text_chunk` duplication follows existing projection rules and
  is excluded. Batch merging parses each delta once and serializes each
  segment once, avoiding repeatedly serializing a growing string during
  historical replay.

### Desktop / Mobile

- Snapshots are enabled only for `sinceIdx=-1`, the first non-pinned request,
  from opt-in clients that also support chunking.
- Large snapshots keep going through the existing immutable `readChunk`, with
  session/run/bridge owner validation kept.
- Mobile verifies snapshot identity, monotonic event indices, and the snapshot
  watermark before opening an independent fixed-watermark increment window; the
  tail must be strictly contiguous and reach the boundary.
- On snapshot/increment mid-failure, session switch, gaps, or identity change,
  no partial projector/cursor is committed. Encountering another replacement
  snapshot in the tail retries recovery instead of mixing two baselines.
- Fixed the earlier projection branch possibly appending the same user_message
  from history again: merge by identity, keeping the authoritative history
  attachments.
- When error has already stopped the projector, streaming=true is not forcibly
  restored just because agent_end has not arrived.
- The existing sync log marks `replayPlan.mode="snapshot"` when a projection is
  adopted; raw paging is `full`, healthy cache is `incremental`. A failed
  attempt may still keep the initial recovery plan — the plan field alone must
  not be used to judge what was transferred before a failure.

### Explicit fallbacks, no silent degradation

- Old Desktops ignore `preferSnapshot`; original paging works as before.
- A new Desktop hitting an old Agent's explicit `unknown command:
  get_run_snapshot` falls back to original paging.
- A snapshot with no semantic events, or whose serialized result exceeds the
  **8 MiB** budget, returns an explicit code from the Agent and the Desktop can
  fall back to original paging. This does not breach the gRPC/16 MiB
  transferred-snapshot cap and does not truncate a snapshot while pretending
  the cursor is complete.
- Other network, storage, or snapshot-validation errors do not trigger a silent
  full download; they follow the existing failure-recovery mechanisms.
- This means old desktops and exceptionally large snapshots can still be slow.
  The segmented-projection / tool-detail lazy-loading protocol — "only download
  the last few entries of arbitrarily large single-round content" — was not
  completed this round.

## Verification record

- Mobile: type-check, lint, 95 suites / **1,322 tests** pass.
- Agent: fmt, clippy `--all-targets -D warnings`, 1,772 lib tests plus the
  crate's other default test targets pass (existing ignored items kept).
- Desktop Rust: fmt, clippy `--all-targets -D warnings`; 1,246 lib tests with
  default GUI features plus main and headless_cli tests pass. New tests also go
  through the real bridge chunk path.
- RPC: 115 lib tests plus wire roundtrip/additive compatibility tests pass;
  the shared command policy's consumers — CLI, channels, loop, TUI — default
  test targets were also checked.
- One multi-crate chain command was interrupted at the 200-second cap, and a
  follow-up chain at the 400-second cap; these are not test-assertion
  failures. Only the unfinished targets were re-run; the final TUI cli_smoke
  passed separately (5 tests), and doc tests completed. Timed-out commands were
  not recorded as all-green.
- New coverage: concurrent cursor/snapshot consistency, run switching,
  persistent-history vs in-memory projection consistency, bad-journal
  rejection, empty/oversized explicit fallback, old-Agent compatibility, chunk
  owner boundaries, no increments, increment gaps/wrong run, terminal-event and
  live overlap dedup, tool/thinking recovery, user-entry dedup, UI and old
  cursor kept on failure.

## Reproduction and artifacts

- `scripts/measure-sync-snapshot.ts`: A/B browser entry.
- `scripts/measure-sync-browser.py`: isolated launcher, new `--agent-binary`
  option to specify the newly built independent Agent; it does not replace the
  system install or stop the user's agent.
- `streaming-sync-snapshot-ab-2026-09-16.json`: 18 de-identified metrics plus
  binary info.

```sh
# In this branch's worktree, build with the pinned toolchain
cargo build -p future-agent --release
cd desktop/src-tauri
cargo test --no-default-features --lib serve_real_snapshot --no-run
cd ../..
node_modules/.bin/esbuild scripts/measure-sync-snapshot.ts --bundle --platform=browser --outfile=target/sync-browser-measurement/bundle.js
python3 scripts/measure-sync-browser.py --test-binary <absolute path of the test executable above> --agent-binary <absolute path of the new future-agent>
```

Open the ready.json URL in the browser and click the A/B button. Afterwards
stop this run's runner, confirm its two child processes stopped and the private
SQLite copy was deleted; keep the de-identified metrics and clean up the
temporary logs and bundle.
