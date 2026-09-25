# Real-data browser sync measurement (2026-09-16)

> ([中文](streaming-sync-browser-measurement.zh-CN.md)) This report keeps the raw
> event-replay baseline. The snapshot + incremental optimization followed; see
> the [new A/B measurements](streaming-sync-snapshot-optimization.md). To keep
> this report reproducible, the old measurement entry now explicitly disables
> `preferSnapshot`; this report's timings must no longer be read as the new
> default path's timings.

## Conclusions and decisions

This run executed the mobile production TypeScript sync code directly, without
estimating RPC counts from database sizes and without injecting a virtual RTT.
A browser can locate the cost of sync reads and replay, but cannot pass for an
end-to-end Hermes / React Native native-rendering or phone-network test.

Three completed historical runs, each repeated three times. A new SyncEngine
was created each time, with no reusable prefix cache; the OS file cache was not
cleared. Results:

| Sample (chosen by completed-run event count) | Total events | Replay logical pages / chunk requests | Measured sync-completion range |
| --- | ---: | ---: | ---: |
| Largest | 111,395 | 112 / 0 | 12.698–12.801 s |
| Second largest | 65,632 | 66 / 0 | 7.544–7.587 s |
| Median | 2,228 | 3 / 0 | 0.239–0.256 s |

**Decision: do not keep expanding chunk or event-page budgets based on the old
estimates. Prioritize phase-by-phase measurement of the backend replay read
path, and re-check under an optimized build.** This round's largest sample's
cost concentrates in replay rather than history, and no sample used
`get_read_chunk`. #647's chunk concurrency had no requests to act on for this
workload — that does not mean it is ineffective for large history pages or
large projections.

These are this round's sample maxima, not the product's maximum latency or the
phone-network ceiling.

## Actual execution chain

```text
Chrome 153 / V8
  → Mobile SyncEngine.reconcile(session, "open", explicitHistoricalRun)
  → Mobile fetchEventsSince / requestReadPage / timelineFromEntries / applyReplayEvents
  → local HTTP requestRetry adapter
  → DesktopHost.execute / PagedReply / paginate_events (real Rust implementation)
  → Agent gRPC (real service)
  → SQLite consistency backup (real history)
```

- Code baseline: `937b70f2`. The desktop adapter is an
  **unoptimized + debuginfo** build from `cargo test --no-default-features`;
  shared business code does not pass through the GUI shell.
- The Agent uses the binary installed on this machine:
  `future v0.0.2-a0373412+local.dirty`. It is not proven equivalent to a clean
  release build; these results must not be generalized to release performance.
- Chrome: 153.0.8010.37, V8 15.3.76.10, macOS. No CPU / network throttling.
- The user's source database is opened read-only and copied to a private
  temporary HOME via SQLite online backup. No authentication info is copied;
  the user's running agent is not modified, restarted, or stopped. The
  measurement agent uses a separate new loopback port.
- The backup contains 229 `completed` runs; the largest, second-largest, and
  median samples were chosen by actual event count. They are not live streaming
  runs. The historical replay target is explicitly specified through
  SyncEngine's public `reconcile` API; `get_state` returns real state, no
  forged activeRun. History reads the session's latest three exchanges at
  snapshot time, not the historical run's own history window.
- HTTP only allows the four read-only commands within this round's sample
  scope, bound to `127.0.0.1`; POST validates same-origin and a custom request
  header. It substitutes NATS/E2EE — it does not reuse the real phone transport
  chain.
- No token simulation, no fake timers, no injected RTT, no model requests. The
  browser UI only displays measurement metrics and does not render session
  bodies.
- Each sync verifies the returned event total, the final committed highWater,
  and success state. The production SyncEngine itself still performs
  continuity/watermark and replay validation.

## Largest sample's phased results

| Metric | Three-run measured range | Meaning |
| --- | ---: | --- |
| State-read phase | 6.3–38.4 ms | real `get_state` |
| History phase | 20.3–23.7 ms | latest three exchanges, one response |
| First timeline commit | 26.7–62.3 ms | data commit, not the first screen frame |
| First replay request | 3.618–3.670 s | browser send to first replay response received/parsed |
| All replay pages fetched | 11.252–11.323 s | including all 112 requests, JSON parsing and merging |
| Sum of backend processing for replay requests | 11.097–11.166 s | DesktopHost execution, Agent gRPC and response JSON serialization; excludes HTTP send |
| Full replay phase | 12.670–12.775 s | after fetching, also validation, projection, cooperative yielding |

The ~1.37–1.51 s after replay fetching is not a pure CPU metric: it includes
browser scheduling/yielding and cannot be used to estimate Hermes CPU time.
The backend number also mixes database reads and Agent/Desktop
conversion/serialization; which internal step is heaviest is not yet separated.

The source pinpoints a next measurement point: `remote_host/business.rs`'s
first `get_events_since` without a watermark goes through
`agent_bridge::get_events_since` full-tail reading; later requests with a
watermark go through `get_events_since_page`. This round measured the first
request being clearly slower, but its internal SQL/deserialization/full-tail
merge was not measured separately — no direct claim of the final root cause or
optimization gain.

## Actual byte counts and old-estimate corrections

The browser calls `arrayBuffer().byteLength` on received JSON responses, not
`length()` on SQLite TEXT:

| Sample | All replay response JSON combined | Largest single response JSON | Current history response JSON |
| --- | ---: | ---: | ---: |
| Largest | 38,782,350 bytes | 402,591 bytes | 238,002 bytes |
| Second largest | 23,479,228 bytes | 400,398 bytes | 111,673 bytes |
| Median | 860,817 bytes | 396,425 bytes | 199,211 bytes |

These numbers include the test adapter's `success/data/error` JSON envelope,
excluding HTTP/NATS framing or encryption overhead. The largest sample's real
response volume is about 37 MiB, different from the 22.37 MB previously
estimated directly from in-library records.

All replay responses were below the 512 KiB chunk trigger threshold. **192 KiB
is the chunk size after chunking is enabled, not the trigger threshold.** The
earlier "112 pages → 179 RPCs" estimate was wrong; this round actually had 112
page requests and 0 chunk requests. History pages also cannot be reconstructed
by simply summing `block_records`: this run went through the real production
presentation path, and all three current history windows were under 512 KiB.

## Reproduction and cleanup

Measurement files:

- `scripts/measure/measure-sync-browser.ts`: imports the real mobile production sync
  code.
- `scripts/measure/measure-sync-browser.html`: metrics-only browser shell.
- `scripts/measure/measure-sync-browser.py`: SQLite backup, isolated agent, and
  read-only loopback probe startup and cleanup.
- `desktop/src-tauri/src/remote_host/sync_measurement.rs`: a default-ignored
  browser test entry, not added to production behavior.
- `streaming-sync-browser-measurement-2026-09-16.json`
  ([archived](../../archives/verification/streaming-sync-browser-measurement-2026-09-16.json)):
  the nine measurement records with session/run identity and bodies removed.

In an isolated worktree with existing project dependencies:

```sh
# Use the toolchain pinned by rust-toolchain.toml.
cd desktop/src-tauri
cargo test --no-default-features --lib serve_real_snapshot --no-run
cd ../..

# Can reuse the installed esbuild; no react-native-web or new dependencies needed.
node_modules/.bin/esbuild scripts/measure/measure-sync-browser.ts --bundle --platform=browser --outfile=target/sync-browser-measurement/bundle.js
python3 scripts/measure/measure-sync-browser.py --test-binary <absolute path of the test executable printed above>
```

Wait for `target/sync-browser-measurement/ready.json` to appear, open the URL
inside, and click the measure button. The script-printed `runnerPid` is the
dedicated process created for this run; after measuring, send it SIGTERM — the
script stops its own two child processes and deletes the private database
snapshot. There is also a 30-minute service lifetime cap. Never stop any
existing agent. Save the metrics elsewhere, then delete this run's
`target/sync-browser-measurement` temporary files.

## Not yet measured

- Cached reopen of a truly live run, with live-event reception and backfill
  overlapping.
- Native phone Hermes, React Native timeline rendering, and the hint bar's
  actual visible duration.
- Real NATS/WSS, encryption, cellular/Wi-Fi, packet loss and reconnection.
- End-to-end results of an optimized build; this round's debug desktop adapter
  affects absolute times and phase ratios.
- Maxima across all sessions: only three runs chosen by event count were
  measured; event content, network, and retries also affect timing.
