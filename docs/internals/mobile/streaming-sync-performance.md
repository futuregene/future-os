# Streaming session-entry sync latency

> ([中文](streaming-sync-performance.zh-CN.md)) Latest: cold start / cache
> invalidation has gained "semantic snapshot + incremental" recovery, no longer
> downloading and replaying all token events of the current run by default.
> Implementation, old-desktop/oversized-snapshot fallback boundaries, and the
> real-browser A/B are in
> [snapshot recovery optimization](streaming-sync-snapshot-optimization.md).
> The earlier measurement and change records are kept below.

## 2026-09-14: measurement and optimization

### Measurement boundaries

No Android device was connected this round and no usable iOS simulator existed
on this machine, so **this is not phone end-to-end measurement**.

- The latency benchmark runs the real
  `SyncEngine → fetchEventsSince → requestReadPage → applyReplayEvents`.
- RPC uses a controlled mock, with Jest virtual clocks injecting a fixed delay
  per RPC. Paging simulates the existing Desktop defaults of 100 events and a
  512 KiB per-event-page byte cap.
- The workload is 10,000 events (one `agent_start`, 9,999 short text deltas),
  with no packet loss, retries, or large tool outputs; the run is still
  streaming after sync completes.
- Virtual-clock numbers reflect serial RPC waits, excluding real network,
  server database queries, encryption, phone CPU, React Native rendering, and
  animation costs.
- Local replay times are measured separately with real clocks in the existing
  `batchedReplay.test.ts`, representing only this machine's Jest/JS environment
  — not Hermes or a real device.

### Bottleneck

`useConversationController.selectSession()` calls `SyncEngine.open()`. Full
session-entry sync runs in order:

1. `get_state`: determine the current active run.
2. `get_session_entries`: load history for the latest three user exchanges.
3. `get_events_since`: backfill that run from `sinceIdx = -1` up to a fixed
   watermark.
4. Apply replay and queued live events; the sync hint clears only after
   completion.

A cache also needs to confirm the latest content; before the optimization both
`open` and `reconnect` did a full refresh. The later incremental optimization
below: history is still refreshed, but the current run's prefix is reused when
safety conditions hold. A readable cache must not be used to declare sync
complete early.

Mobile originally passed no `limit` to `get_events_since`, falling back to the
Desktop's common **100 items/page** default. 10,000 small events produce **100
serial RPCs** — even at 150ms each, backfill alone takes 15 seconds. First
history usually shows much earlier, so it appears as "content is there, yet
still syncing for a long time".

Code references:

- `mobile/src/remote/syncEngine.ts`: `open`, `runReconcile`, `needsHistory`.
- `mobile/src/remote/replay.ts`: the serial paging loop and fixed watermark.
- `desktop/src-tauri/src/remote_host/business.rs`:
  `DEFAULT_MESSAGE_PAGE_LIMIT = 100`, `MESSAGES_PAGE_BYTES = 512 * 1024`,
  `paginate_events`.
- `agent/src/rpc/protocol.rs`: run records beyond the in-memory ring can be
  backfilled from the persistent journal, so backfill volume is not limited to
  the 2,000 ring events.

### This round's changes and results

Mobile explicitly requests **1,000 events/page**. Desktop's independent byte
limit, chunked reads, fixed watermark, session-switch cancellation checks,
retries, and sync-completion conditions are unchanged. With large tool outputs
the server may still return pages smaller than 1,000 events; the client keeps
reading by cursor — a page under 1,000 events must not be treated as an end
condition.

| Injected delay per RPC | Old backfill requests | New backfill requests | Old sync completion | New sync completion |
| --- | ---: | ---: | ---: | ---: |
| 20ms | 100 | 10 | 2,040ms | 240ms |
| 150ms | 100 | 10 | 15,300ms | 1,800ms |

The total waits above include the state and history RPCs — 102 vs 12 RPCs.
Controlled-scenario wait reduced by about **88.2%**. First history-preview
commit times are unchanged at 40ms / 300ms; "commit" here is not the actual
first screen frame.

One sample of the independent replay benchmark: 10,002 / 50,002 / 100,002 text
events take about **39 / 201 / 393ms**, generating one render snapshot per
group with cooperative JS yielding. This shows that, for this workload,
accumulated network round-trips deserve optimization priority over local text
replay; it cannot rule out complex tool content or Markdown rendering
bottlenecks on real devices.

### Reproduction

With repo dependencies configured, run from `mobile/`:

```sh
npm test -- --runTestsByPath src/remote/__tests__/streamingSync.perf.test.ts
npm test -- --runTestsByPath src/remote/__tests__/batchedReplay.test.ts
```

`streamingSync.perf.test.ts` verifies complete text, the final cursor,
streaming state, and exactly 10 backfill requests. The old-baseline comparison
is a record of running the same benchmark before `REPLAY_PAGE_EVENTS` / `limit`
existed; the current test's 10-request assertion prevents regressing to the
common small page.

### Real-device diagnostics and follow-up boundaries

New log prefix: `[remote] session timeline sync timing`.

- Each reconcile attempt records `reason`, `attempt`, `elapsedMs`, `stagesMs`,
  and `outcome`, timed with a monotonic clock.
- `stagesMs.get_state`, `history`, `replay` separate the state, history, and
  backfill phases; `replay` **includes network paging and local replay** — not
  pure network timing.
- `outcome` distinguishes success, failure, and stale work from a switched
  session.
- Dev builds record every attempt; release builds only record attempts taking
  at least 1 second; only identifiers and timings are included, no chat content
  or credentials.
- A single attempt's total elapsed excludes prior queueing, backoff between
  attempts, later live-queue application, and UI drawing — it must not be read
  directly as the hint bar's full visible duration.

Real-device re-tests should cover: cold open, cached reopen, long tool outputs,
Wi-Fi/cellular, packet-loss retry, session switching, and a run ending
mid-read. When still slow, check the phased logs first:

- `get_state` slow: check Desktop response and connection recovery; the
  existing per-request timeout is 10 seconds, and transport retries can still
  produce long waits.
- `history` slow: check large exchanges and 192 KiB snapshot chunk transfer.
- `replay` slow: check event scale, large tool data, network/encryption, and
  phone JS processing.

Round one only optimized paging; the cache-prefix reuse below and round
three's bounded-concurrency chunked reads came later. Real-device throughput of
parallel chunks and transient load still need measurement. Completeness checks
must not be skipped, nor the hint hidden early, to shorten the hint time.

## Round two: safe cache-prefix reuse

### Strategy

Entry / reconnect still reads `get_state` first and refreshes the recent
history, to update attachments, old-message completion states, and the paging
window. Only when all of the following hold does the current run's
`get_events_since` start from the committed `highWater`:

- The server still reports the same active run, and the cache is still
  streaming.
- The cache once built a complete baseline with a cursor-confirmed complete
  prefix.
- The current run's projector accumulated state and the corresponding assistant
  row both exist; the cache is not just visible text.
- No known truncation hints, snapshot replacements, or prefix-rebuild notices.

The current-run assistant mirror inside the latest history is replaced by the
cached complete prefix, avoiding a duplicate reply; the authoritative history's
user messages and attachments are kept. Increments are then applied to a forked
projector, committing timeline and cursor together only after success.

### Correctness protections

- First open, cache loss, incomplete prefix, run ended, or switching still do a
  full recovery. If the run ends between `get_state` and the history read, the
  history's terminal reply wins and must not be overwritten by a stale
  streaming prefix.
- `resend` / `prefix` / `truncated` immediately invalidate the old baseline;
  hiding a session only invalidates the cache without a background full read.
- The invalidation version number also blocks stale in-flight results; an
  immediate reopen cannot accidentally reuse an invalidated cache.
- Increments must have matching run identity and contiguous indices, reaching
  the server's fixed watermark. On missing segments, truncation, or
  cursor-boundary mismatch, no partial result is committed — the old UI and
  cursor are kept, retrying from `-1` via the existing backoff.
- Ordinary transport failures do not discard a complete prefix without reason;
  retries can keep requesting the same increment.
- When the server returns a complete projection, verify its identity and
  boundaries and replace the current run wholesale; the old cursor and
  dedup records reset accordingly, preventing events being dropped because the
  cursor is above the new snapshot.
- With old Desktops without a watermark, increment-event contiguity is still
  checked; an incomplete legacy full prefix does not qualify for incremental
  reuse.
- Switching away, reconnecting, or a cancelled cooperative replay never
  partially modifies the committed projector or cursor. Overlaps between
  increments and queued live events are deduplicated; the end event's timing
  and token stats are kept.

### Controlled benchmark

Cache 10,000 events first, add 200 while away, then re-enter. The control is
the same code with the cache cleared doing a full read; both sides use round
one's 1,000 events/page.

| Delay per RPC | Full: 10,200 events / 11 backfill RPCs | Incremental: 200 events / 1 backfill RPC |
| --- | ---: | ---: |
| 20ms | 260ms | 60ms |
| 150ms | 1,950ms | 450ms |

Total waits still include the state and history RPCs. Backfilled events
decrease by about **98%**, and this scenario's simulated wait by about **77%**.
These remain virtual-clock / mocked-RPC results, not phone end-to-end
measurements.

Test entry points:

```sh
npm test -- --runTestsByPath src/remote/__tests__/incrementalOpen.test.ts src/remote/__tests__/streamingSync.perf.test.ts
```

Regression tests cover incremental vs cold-start full results being identical
(text, thinking, tools), empty increments, attachment refresh, cache loss, run
switch/end, background snapshot invalidation, stale in-flight replies,
increment gaps, snapshot fallback, transport retry, large replay cancellation,
and duplicate terminal events.

## Round three: bounded-concurrency chunked reads for large snapshots (2026-09-15)

### Remaining serial waits

The first two rounds reduced the event-page count and the backfilled event
volume. Large histories or projections can still trigger the Desktop's chunked
snapshot transfer: responses over 512 KiB are saved as immutable snapshots,
192 KiB per chunk, 16 MiB total cap. `requestReadPage` originally awaited RPCs
chunk by chunk; cached reopens still refreshing history bear that accumulated
latency.

Now the first chunk is validated, the snapshot id and total size pinned, then
up to **4 chunks** per batch are read concurrently. Only independent offsets of
the same immutable snapshot are parallelized — no parallelizing cursor-
dependent event paging, no changing the sync-completion condition or hiding the
hint early, and no increase in total request count or data volume.

- Each chunk still validates snapshot id, total size, requested offset, decoded
  length, and nextOffset; out-of-order replies write their own byte ranges, and
  JSON is parsed and returned only when complete.
- session/run/bridge identity fields are kept; ordinary responses and
  old Desktops without chunk support behave unchanged.
- After failure or session switch, no next batch starts. The at-most-4 in-flight
  requests may still complete or run the transport's own retries; late results
  never publish a partial timeline; late exceptions from concurrent requests
  are also handled.
- At most 4 in-flight chunks per read; never request the whole large snapshot
  at once. The total buffer stays under the original 16 MiB limit.

### Controlled before/after

The same `readPages.test.ts` latency test ran before and after the change: JSON
with 2,359,296 ASCII text characters, 13 chunks after encoding. The first
chunk serial, the remaining 12 read in 3 batches.

| Delay per RPC | Old serial read | 4-chunk concurrency | Requests |
| --- | ---: | ---: | ---: |
| 20ms | 260ms | 80ms | 13 both |
| 150ms | 1,950ms | 600ms | 13 both |

This scenario's simulated wait decreased by about **69.2%**. This measures only
the real `requestReadPage` under mock RPC / Jest virtual clocks — not the full
SyncEngine, real-device end-to-end, or the UI hint's visible time; real
bandwidth, encryption, JSON-decode CPU, and rendering are excluded. Small
responses gain nothing, and pure bandwidth bottlenecks are not promised the
same proportional improvement.

```sh
cd mobile
npm test -- --runTestsByPath src/remote/__tests__/readPages.test.ts
```

Tests cover the concurrency cap, out-of-order Unicode assembly, identity and
boundary validation, cancellation, snapshot expiry/transport failure, late
exceptions, and legacy compatibility. Still needed: using the
`[remote] session timeline sync timing` phased logs on sessions with large tool
outputs to verify Wi-Fi/cellular, weak-network retries, and low-end phone
transient load; this benchmark alone cannot conclude all long hints come from
chunked reads.
