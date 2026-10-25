# Remote relay / NATS transfer performance — 2026-10-26

## Topology and scope

The self-hosted `future relay` HTTP process handles pairing and issues/refreshes
short-lived NATS credentials. It is not on the steady-state event or file data
path. Clients exchange end-to-end encrypted Core NATS messages with the host;
the self-hosted relay deliberately does not enable JetStream. Optimizing relay
HTTP proxying would therefore not improve remote-desktop traffic.

The default NATS/JWT payload limit is 1 MiB and file chunks are 512 KiB. This
experiment preserves that protocol. Public Internet WSS, handset CPU/battery,
real disk throughput, packet loss, and production-relay RTT were not measured.

## Reproducible setup

- Apple Silicon Mac (Darwin arm64), Rust 1.96.0, nats-server 2.15.0.
- Real Noise-encrypted Rust client/server and real Core NATS on loopback;
  isolated broker per fixture, no production relay or user connection.
- 4 MiB file, eight 512-KiB chunks, SHA-256 verification.
- The explicit fixture delay adds 50 ms of host-side work per download chunk.
  This isolates scheduling and is **not** a simulated public-network RTT.
- Command:
  `cargo test -p future-remote-interop --features network-tests --test transfer_benchmark -- --ignored --nocapture`
- Correctness/backpressure check:
  `cargo test -p future-remote-interop --features network-tests --test transfer_benchmark`

An early TCP-read-delay proxy was rejected: its per-read delays distorted
large-frame transfer and did not model an actual relay. Its wall-clock results
are excluded. The retained fixture injects delay only at the named host read.

## Measurements

Baseline medians use three fresh client/server/broker repetitions per condition.
All values are milliseconds. Debug builds measure relative scheduling, not
release-mode throughput.

| Change, 4 MiB download | 0 ms/chunk | 50 ms/chunk |
|---|---:|---:|
| Original serial client and server | 1201 | 1663 |
| Real four-request client window only | 779 | 1206 |
| Client + bounded four-read server pipeline | 874 | 1122 |

The final bounded implementation was rerun on one warm connection for three
iterations at 50 ms/chunk: **1135, 1154, 1155 ms** (median 1154). Against the
1663 ms baseline median this is a **30.6% reduction**. A like-for-like warm-run
baseline was not retained, so this percentage compares the recorded original
fresh-connection medians with the final warm samples; it is not a production
latency guarantee.

The 4 MiB upload remained sequential (about 1.46–1.72 s in the 50 ms/chunk
condition): its protocol appends chunks in index order. Do not infer a download
speedup for upload.

## Findings and decisions

1. **Rust download concurrency was nominal only.** The four-chunk wave awaited
   every pull before starting the next. `try_join_all` now runs the four pulls
   concurrently while preserving ordered assembly and the existing retry and
   hash/size checks. The mobile TypeScript downloader already used
   `Promise.all`.
2. **The server was the next serializer.** It read, published, and flushed each
   chunk before receiving another. Independent indexed reads now run off the
   async executor with at most four in-flight operations. Upload writes remain
   ordered. Data publication still precedes acknowledgement and the broker
   flush remains intact. The regression test observes four active reads and
   validates the complete file.
3. **Rust large-page reassembly had the same false-concurrency pattern.** Its
   remaining independent chunk reads now also use bounded `try_join_all`.
   This is a code-path correction; no separate large-page timing is claimed.
4. **Removing per-chunk flush is not justified yet.** With both scheduling fixes
   and 50 ms/chunk, medians were 1106 ms with flush and 1080 ms without (about
   2%, overlapping runs). That is insufficient evidence to weaken the existing
   publish-before-ack discipline.
5. **Larger chunks or JetStream are not first-line optimizations.** The former
   approaches the 1 MiB cap and increases buffering; the latter changes the
   delivery/storage model without evidence that durable storage is needed.
   Measure real-link RTT/loss before pursuing either. Upload pipelining needs a
   separate ordered-write or indexed-write design and its own measurements.
6. **Low-latency CPU/copy cost remains.** Four-MiB loopback debug transfers take
   roughly 0.8–1.2 s even without injected work. A release CPU profile should
   distinguish E2EE, base64/JSON, allocation, file I/O and broker overhead before
   further micro-optimization.
