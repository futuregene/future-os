# Remaining mobile performance work — 2026-09-16

This round follows #664/#665. It implements the remaining cache, draft, transport scheduling, file-transfer/hash, large-block rendering and background-traffic work. It does not claim measured battery savings or native-device frame rates.

## Implemented

### Cache accounting

- Timeline LRU sizing now walks object/array/Map/Set contents without creating a serialized copy and includes explicit closure-owned projector storage plus queued event bytes. Shared objects are visited once. This is conservative accounting, not exact VM heap measurement.
- The existing 8-session / 16 MiB policy remains; the currently selected conversation is still an exception rather than silently truncating visible content. Oversized inactive caches may reload on reopen.
- The shared settled Markdown cache now has an 8 MiB charged source/AST budget in addition to its 512-entry count limit. Sources over 128 Ki UTF-16 code units bypass it. Charges are serialized structure size plus block overhead, not a claim of an exact 8 MiB heap ceiling.

### Draft storage

Typing coalesces at 250 ms with a 2-second maximum wait during continuous input. Navigation/unmount and background transitions flush. Reads, explicit share saves, send clears and acknowledgement comparisons are ordered barriers; delayed writes cannot resurrect a cleared draft or erase a newer edit. Abrupt OS termination before an asynchronous flush can still lose the most recent buffered edit, as with any debounced persistence strategy.

### Network/decode scheduling

- Buffered subscription iteration yields after bounded record count, byte count or elapsed work, before another decrypt/decode burst. Empty SDK queues already await network I/O, so ordinary slow tokens do not acquire an extra timer each.
- Candidate-generation activation drains bounded batches in order. New arrivals queue behind that prefix; retiring a generation fences its remaining callbacks.
- Native JSON parsing is retained. Large command replies/history snapshots yield between UTF-8 decoding and native JSON graph creation, allowing an obsolete read to be cancelled between phases. Neither native phase is itself preemptible.

### File transfer and hashing

- Fetch immutable indexed download chunks in waves of at most four and approximately 2 MiB of in-flight data; write completed waves in order. All wave promises settle safely, failures/cancellation prevent later writes, and final size/SHA-256 checks remain mandatory.
- New Android/iOS native `hashFile` implementations stream 64 KiB reads on a background queue, enforce the existing 10 MiB limit and reuse native read-permission checks. File bytes do not cross into JS on this path.
- Older app binaries use bounded incremental SHA-256 via the already-used `sha256-universal` dependency (now declared directly). It yields and checks cancellation. Native permission/read failures are not silently bypassed. Native hashing is bounded but not interruptible mid-call; cancelled results are rejected on return.

### Giant blocks

Tables beyond a small initial row window paint that many rows and keep the rest behind an explicit control rather than mounting every row: a nested vertical viewport loses the pan gesture to the message list, the same failure the long code block below was fixed for. Long code blocks stay bounded by wrapping (never a horizontal scroll region: a phone-width block is what keeps a long CJK line readable) and by collapsing to a clipped 16-line preview whose toggle paints every bounded chunk inline, so the tail is one tap away rather than hidden behind a nested vertical viewport that loses the gesture to the message list. Code chunks are bounded by characters and line count, including minified single lines and large runs of empty lines; UTF-16 surrogate pairs remain intact. Continuation markers are presentation only and full-source copy preserves the original code.

### Negotiated detailed-event interest

`selective_events_v1` allows supporting phones to subscribe to detailed events only for the selected session. Background/list views retain low-rate run/approval/configuration notices on the existing `state.>` namespace. Intentional unsubscription is not treated as a network failure. Legacy desktops retain wildcard behavior; legacy phones retain the original full-event feed. No pairing reset or broader authorization is introduced.

The maintained protocol specification is [CONNECTION.md](../../internals/desktop/CONNECTION.md#23-messages-transport-and-permissions). Detailed Desktop-to-broker publication remains for compatibility; the saving is irrelevant detailed delivery to supporting phones. This feature requires the updated Desktop as well as updated mobile JS.

## Real JSON experiment and rejected candidate

A read-only local SQLite trace supplied the first **50,000 real events** of the previously measured 152,394-event completed run, serialized as an events response: **8,141,437 bytes**. No invented/repeated event contents. The existing loopback probe ran three alternating A/B rounds in visible Chrome 153 on macOS (reported hardwareConcurrency 32), baseline `85fa2ac4`. These are V8/loopback measurements, not Hermes, NATS radio traffic or native UI timings.

An initial custom cooperative JS JSON parser was correct but regressed: baseline median **16.6 ms**, candidate **106.0 ms**; heartbeat maximum-delay medians **1.8 ms → 7.4 ms**. That parser was removed rather than shipped as an optimization.

Final retained native-parser implementation:

| Round | Baseline elapsed | Current elapsed | Baseline heartbeat max lateness | Current heartbeat max lateness |
|---|---:|---:|---:|---:|
| 1 | 15.0 ms | 13.9 ms | 0.5 ms | 1.8 ms |
| 2 | 15.4 ms | 15.1 ms | 2.0 ms | 2.0 ms |
| 3 | 13.7 ms | 13.8 ms | 1.7 ms | 1.2 ms |

Treat this as comparable throughput plus a cancellation/task boundary, **not evidence of a significant speedup**. All decoded content digests matched:
`52497c31fce090983e5a10aedaba9b78fe1e87a314cef216c691637c3f3c8003`.

Reproduce with `node scripts/measure-mobile-performance.mjs 85fa2ac4`, then `python3 scripts/serve-mobile-performance.py`, open the emitted loopback URL and use “Measure large real JSON decoding”. The server never starts/stops the real agent, reads no credentials, and never commits raw conversations. Stop only the printed measurement PID and close its tab afterward.

## Validation and limits

- Mobile typecheck + ESLint + **105 suites / 1,469 tests** passed after syncing upstream; tests cover foreground subscription restoration.
- Desktop frontend typecheck, ESLint, stylelint + **127 suites / 1,080 tests** passed for shared-package consumers.
- Desktop Rust on pinned 1.97.0: fmt, clippy all-targets with warnings denied, and cargo tests passed (1,266 library + 1 binary + 5 integration tests; existing ignored tests remain ignored).
- Android `:future-file-handler:compileReleaseKotlin` printed `BUILD SUCCESSFUL`; the generated worktree class was verified to contain `hashFile`. The command channel timed out after build success rather than returning cleanly, so the compiler output/artifact, not the outer timeout status, is the evidence.
- iOS Swift syntax parsing passed. Full iOS compilation and device execution are **not verified** because Xcode is not installed here.

Regression workloads include 100 coalesced edits, stale send acknowledgement vs newer drafts, Map/Set and closure-owned cache data, 500 queued encrypted records, bounded activation with new arrivals, retired generations, real Noise/AEAD status notices with mocked NATS delivery, background approvals/end events, 10 MiB incremental hashing against an independent SHA-256 oracle, native-hash cancellation/result fencing, reverse-order parallel chunk replies, a 5,000-row table and long/minified/empty-line code sources. Rust also verifies dual status/legacy publication through its transport fixture.

Remaining acceptance is device/environment validation: power, thermal behavior, native nested scrolling and lifecycle, full iOS build, and deployed old/new Desktop/mobile pairing combinations. Memory charges are estimates; extremely large single native parse operations are still non-preemptible. These limitations are not represented as completed battery or device-performance measurements.
