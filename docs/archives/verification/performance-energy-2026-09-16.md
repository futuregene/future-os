# Mobile TS performance and energy-work reduction — 2026-09-16

## Scope and limitations

This change reduces live projection work, ordinary text-update frequency, redundant presentation timers, and Markdown reparsing. It is not a measured battery-life improvement.

Measurements ran in a real visible Chrome 153 browser on macOS (V8, reported hardwareConcurrency 32), using bundled **production Mobile TypeScript** and actual local historical conversation data. Baseline: `8d96b076`. Current: the code accompanying this report. Three rounds per scenario, alternating A/B order. No fake clock or synthetic/repeated token contents were used. Tables report medians.

The harness is not the native React Native app: it does not measure Hermes, native text layout, FlatList recycling, IME, native back gestures, NATS/encryption, real mobile radio/network latency, or power consumption. Its heartbeat is a 16 ms JS timer-lateness probe, not frame rate or native touch-to-paint latency. Burst elapsed time includes production scheduling and projection, not just CPU time. The paced scenario deliberately supplies one real event every 10 ms; these are **not** the original event timestamps.

## Implemented changes

- Coalesce ordinary text/thinking/tool-argument deltas over 80 ms (approximately 12.5 updates/s under steady traffic), instead of 16 ms. Control/approval/error/end events bypass that wait while retaining queue order.
- Within each bounded live batch, copy dedup/projector state once and materialize a snapshot per contiguous run, not per token. Retain the 64-op / 8 ms cooperative budget; backlog continuations yield without another 80 ms wait.
- Remove the second 32 ms typewriter timer. Display already-coalesced committed text directly. Preserve reduced-motion handling for newly inserted blocks' native fades.
- Only the trailing segment receives streaming presentation; completed text segments do not keep live-presentation subscriptions.
- Tick elapsed-time labels once per second rather than twice, and stop their timers in the background.
- Skip timeline projection during the connection's background grace period. Existing foreground reconciliation restores missed durable data; backgrounding does not abort the remote run.
- Preserve stable Markdown blocks/rows with inline links, images and task lists. Fall back to canonical whole-document parsing when a newly parsed definition/footnote can affect earlier blocks, including definitions nested in containers. Preserve local-file/image reference collection.

## Real corpus and correctness

The loopback harness opens the local agent SQLite database **read-only** and selects completed runs. It does not start, stop, or attach to the user's agent. No credentials are read. Raw traces/reply contents are served only to the local browser, held in memory, and neither displayed nor committed. IDs in the harness are aliases. No raw private fixtures are included in the repository.

| Sample | Actual events | Event payload characters | Composition |
|---|---:|---:|---|
| Longest completed run | 152,394 | 37,814,677 | 25,065 thinking deltas; 125,235 tool deltas; 1,575 text chunks; tool lifecycle, usage, user/start/end events |
| Medium completed run | 19,814 | 4,254,145 | 12,039 thinking deltas; 6,565 tool deltas; 1,081 text chunks; tool lifecycle, usage, user/start/end events |

Each trace is checked for contiguous original indices and a real terminal `agent_end`; events are not silently reindexed. The full real prefix is projected before live-tail measurements. All A/B terminal timeline content digests and cursor high-water marks match a full baseline replay. Stable-key serialization avoids confusing object key order with semantic differences. Only top-level UI wall-clock-derived `startedAt`/`durationMs` fields are excluded from timeline content hashes.

Canonical content fingerprints:

- Longest: `d0d2eb232acb4570af917ae3a5b976a5598154a108155b773d3589a1ec65ce40`
- Medium: `58bfc5e9752a62ffcea5e82815f9f45704c3c36fc2c283f679fba9fcfd6ef7d6`

## Browser results

### Real live-tail processing

| Scenario | Baseline | Current |
|---|---:|---:|
| Longest: 1,001 queued events after a 151,393-event prefix | 2,712.6 ms | 80.1 ms |
| Longest: commits for that burst | 106 | 17 |
| Longest: burst heartbeat p95 / maximum lateness | 8.1 / 10.3 ms | 2.0 / 2.0 ms |
| Medium: 1,001 queued events after an 18,813-event prefix | 381.0 ms | 68.9 ms |
| Medium: burst heartbeat p95 / maximum lateness | 6.9 / 7.2 ms | 1.7 / 1.7 ms |
| Longest: 201 real tail events paced at 10 ms — commits | 102 | 28 |
| Medium: 201 real tail events paced at 10 ms — commits | 102 | 28 |

Paced elapsed time did **not** meaningfully improve: longest 2,307.0 → 2,336.6 ms; medium 2,320.5 → 2,335.6 ms. It is dominated by the controlled delivery timers. The relevant improvement is fewer snapshot/UI notifications for the same correct result, not faster upstream model generation.

### Real Markdown replies

The longest event trace is not necessarily the longest Markdown response. Its 4,811-character final reply and the medium trace's 2,303-character reply contain no square brackets. Initial measurements showed no Markdown benefit on those samples (cumulative baseline/current: 18.6/21.6 ms and 25.4/41.2 ms). They are not used to claim parser acceleration. Subsequent code avoids unnecessary reference/definition walks on reference-free fragments.

Separately selected the two longest local assistant text blocks containing `[` and replayed each original reply as approximately 100 growing frames (no duplicated paragraphs). Final streaming ASTs/references match canonical parsing in both versions.

| Actual reply | Cumulative parse time baseline → current | p95 frame parse baseline → current | Maximum frame parse baseline → current |
|---|---:|---:|---:|
| 9,287 characters | 226.6 → 37.2 ms | 4.2 → 0.9 ms | 5.1 → 1.2 ms |
| 5,381 characters | 130.9 → 9.3 ms | 2.2 → 0.2 ms | 3.3 → 0.3 ms |

## Automated validation

- Mobile TypeScript check and ESLint pass.
- Mobile: 97 suites / 1,366 tests pass.
- Shared Markdown package TypeScript check passes.
- Desktop direct-consumer streaming Markdown suite: 28 tests pass, including every-character parsing parity, late definitions, tables, deep nesting, replacement and finalization.
- New checks cover steady commit rate, immediate approval/end state, background projection suspension plus foreground recovery, no second text-animation timer, one-second/background-paused duration labels, stable bracket-bearing prefixes and reference correctness.

## Reproduce locally

From the repository root, with the normal JS development dependencies installed:

```sh
node scripts/measure-mobile-performance.mjs 8d96b076
python3 scripts/serve-mobile-performance.py
```

Open the loopback URL in `target/mobile-performance/ready.json` in a visible browser. Use the real-trace A/B button, then the real-Markdown button. The latter retains preceding aggregate results. Inspect `target/mobile-performance/results.json` for per-round metrics. Do not run CPU-heavy tests/builds concurrently with the browser measurement. The selected local corpus can change, so compare fingerprints/counts before comparing measurements across machines or dates.

The default server launcher prints its child PID; terminate **only that measurement server** when finished and close the test tab. It has a 30-minute lifetime bound. All generated artifacts are gitignored. No Rust agent or model request is required.

## Battery and native-device acceptance still required

The demonstrated energy proxies are fewer commits/repeated projections, no secondary typewriter wakeups, lower duration-timer frequency and no background timeline projection. Radio traffic and model execution are unchanged. No battery percentage or watts were measured.

A release-device follow-up should compare equal workloads and brightness/refresh settings on representative Android/iOS phones: CPU/GPU activity, thermal behavior, foreground/background energy, frame pacing, and actual return/stop/keyboard latency. Also test file-preview overlays and large native Markdown layouts; those remain separate optimization opportunities.
