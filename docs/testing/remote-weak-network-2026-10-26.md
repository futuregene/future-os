# Weak-network control-plane cost for remote downloads — 2026-10-26

## The question, and the answer

A proposal on the table would change the remote file transfer from "one pull per
512-KiB chunk" to a range batch of four chunks per pull, with one group
acknowledgement. Its argument is that control messages fall by about 4×, that the
byte saving is nil, and that the value would be in round trips, loss recovery and
host scheduling — none of which had been measured. This page measures the round
trips.

**Measured, on a real encrypted Core NATS link with injected latency and loss:**

- For a 4 MiB page the change saves **one round trip** (300 ms at 300 ms RTT),
  because eight pulls do not cost eight round trips today — the client already
  runs four of them concurrently, so eight chunks cost **two**.
- For a 10 MiB page it saves **three round trips** (900 ms at 300 ms RTT).
- A 1% loss rate roughly **doubles** both numbers: loss costs about one recovery
  window per wave, and batching halves the number of waves.
- On a link where the bytes themselves take time, the saving stays the same in
  milliseconds and shrinks as a fraction: 6.9% of a 4 MiB download on a 10 Mbps
  link, 30% of the same download on a delay-only link.

**One-line answer.** For 4 MiB pages, moving the request/ACK semantics is worth
it only above roughly **250 ms RTT combined with ≥20 Mbps** (≥10% of the
download); for 10 MiB pages the bar is around **60–120 ms RTT** on a normal
mobile link. Packet loss does not lower that bar — it increases the payoff by
about 2×, while making the retry unit four times larger.

## 1. What was measured

| dimension | values |
| --- | --- |
| page | 4 MiB (8 chunks), 10 MiB (20 chunks), exactly as the desktop serves attachments |
| chunk | 512 KiB, the protocol's and every existing benchmark's chunk size |
| round-trip time | 2, 40, 120, 300 ms (one-way delay 1, 20, 60, 150 ms) |
| loss | 0 and 1% of MTU-sized packets (0 and 10 per-mille) |
| repetitions | 3 per clean cell, 2 per lossy cell, on one warm connection each |

Per cell this page reports the **download wall clock**, the **control message
count** (counted at the socket, not estimated) and the **bytes on the wire** in
both directions.

Topology: one loopback `nats-server` per cell, the real `future-remote-client` as
the phone, the real `future-remote-server` supervisor as the host, a real sealed
(Noise) channel, a real 512-KiB chunking and host read path, SHA-256 verified. The
host connects to the broker directly; **only the client's leg runs through the
weak link**, which is the shape of a phone on a mobile network talking to a host
on a good link.

## 2. The fixture, and what it deliberately does not model

`tests/remote-interop/tests/support/weak_link.rs` sits between the client and the
broker and forwards bytes with a per-direction delay and a loss model. Four
choices matter for reading the numbers:

1. **It delays bytes, not reads.** A byte read at `t` is offered to the peer at
   `t + one_way` regardless of read boundaries, so a 512-KiB message is not
   serialized into one delay per buffer. An earlier TCP-read-delay proxy in this
   repository was rejected for exactly that distortion. Consequence: the fixture
   has **unlimited bandwidth**, and everything it shows is the round-trip cost.
2. **A lost packet is a stall, never a deletion.** TCP redelivers every byte in
   order; a byte-deleting proxy would corrupt the NATS frame stream (payload
   lengths are explicit) and would measure a protocol error and a reconnect, not
   recovery. So a lost packet adds `retransmit` — one RTT, the optimistic
   fast-retransmit case — and everything behind it in the stream waits. Losses
   that land while the stream is already waiting share one recovery window, which
   is what one flight recovered by one retransmission episode does.
3. **Loss is deterministic in byte space.** A packet is lost according to a hash
   of `(seed, packet index)`, not a running PRNG, so the same seed loses the same
   byte offsets no matter how reads split (`weak_link`'s unit tests pin this).
   Without that, two runs of one profile are not comparable.
4. **Not modelled, on purpose:** congestion-window collapse after loss, a dropped
   connection and reconnect, a NATS message that never arrives, WSS/TLS framing,
   host disk and scheduling latency, network hardware, and multi-client load. The
   loss numbers are therefore a **lower bound** on what a lossy link costs, and
   the bandwidth column in §5 is an estimate, not a measurement.

Counting is exact: the proxy parses NATS headers (`PUB`/`HPUB`/`MSG`/`HMSG`) and
skips payloads by their declared length, so a control message count is a count of
protocol messages. Acknowledgements are matched to the reply subject of the pull
they answer; a client with a custom inbox prefix (`p.<pair>.rep.<device>`) makes
any "does it look like `_INBOX`" heuristic wrong, which is how the first version
of this fixture undercounted acknowledgements by all of them.

## 3. Reproducing

```text
# the matrix (16 cells, ~2 minutes; real sockets, real delay)
cargo test --release -p future-remote-interop --features network-tests \
    --test weak_network -- --ignored --nocapture weak_link_download_matrix

# the loopback floor at zero delay (the fit's intercept, checked)
cargo test --release -p future-remote-interop --features network-tests \
    --test weak_network -- --ignored --nocapture weak_link_zero_delay_floor

# the correctness half, no `--ignored`: a weak link must not change the bytes,
# the pull count, or the acknowledgement count
cargo test -p future-remote-interop --features network-tests --test weak_network
```

Each measured download prints one `CELL {...}` JSON line with per-repetition
`elapsedMs` and wire counters, so a number in this page can be traced to a run.

Run identity: Apple M3 Ultra (arm64), macOS 26.6.2, rustc 1.96.0, nats-server
2.15.0, release build (`--release`; the debug build's CPU floor is about 16×
higher — 868 ms for 4 MiB at 2 ms RTT — and would swamp the link time). Base
commit `37756ae2` of `origin/dev-huichen`; the fixture and this page are test and
documentation only, no production file is touched.

## 4. Raw numbers

Release build, medians per cell. `wire` is both directions of the proxied leg.

### Clean link

| page | chunks | pulls | acks | control msgs | wire KiB | 2 ms | 40 ms | 120 ms | 300 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 MiB | 8 | 8 | 8 | 16 | 4102 | 55.8 | 213.5 | 488.7 | 1010.9 |
| 10 MiB | 20 | 20 | 20 | 40 | 10253 | 111.5 | 431.8 | 921.4 | 2107.2 |

Zero-delay floor (no injected delay at all): **48.4 ms** (4 MiB), **95.3 ms**
(10 MiB). Least-squares fit over the four RTT points, clean link:

| page | wall clock ≈ | RTTs spent | structure predicts | zero-delay floor |
| --- | --- | ---: | ---: | ---: |
| 4 MiB | 77.5 ms + 3.158 × RTT | 3.16 | 3 | 48.4 ms |
| 10 MiB | 130.2 ms + 6.604 × RTT | 6.60 | 6 | 95.3 ms |

The structure is exact and checkable in the code: `1` round trip for
`download_prepare`, then `ceil(chunks / 4)` waves — `transfer.rs` fetches chunks
in `try_join_all` waves of `CHUNK_CONCURRENCY = 4`, and a wave cannot start before
the previous wave's last chunk and acknowledgement have arrived. Eight chunks are
therefore **two** waves, twenty are five. The measured slope is 5–10% above the
structure, which is what a wave boundary costing slightly more than one clean
round trip looks like.

### 1% loss

| page | 2 ms | 40 ms | 120 ms | 300 ms | packets lost | stall charged (per 40/120/300 ms) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 MiB | 102.4 | 332.5 | 700.6 | 1630.0 | 32 | 80 / 240 / 600 ms |
| 10 MiB | 180.0 | 729.5 | 1620.3 | 3595.4 | 76 | 200 / 600 / 1500 ms |

Read that column against the wave count: the charged stall is **one round trip
per wave** — 2 waves × RTT for 4 MiB, 5 waves × RTT for 10 MiB — because all four
chunks of a wave are in flight together and a flight is recovered by one
retransmission episode. The observed penalty is at or above the charged stall
(+0–50%), since a late acknowledgement also delays the next wave's first pull.

Two things follow, and both matter to the decision:

- **Packet loss does not create retries at this layer.** Pulls, chunk messages and
  acknowledgements are the same 8/8/8 and 20/20/20 in every lossy cell. TCP hides
  packet loss from NATS; the client's three-attempt retry and the SHA-256 check
  exist for connection loss and restarts, not for ordinary loss. The "loss
  recovery" benefit claimed for range batching is therefore about *fewer recovery
  windows*, not about lost messages.
- **Bytes do not move.** A 4 MiB page sits on 4102 KiB of wire in a clean cell and
  4110 KiB at 300 ms with 1% loss; the client sends 3 KiB up in the clean 2 ms cell
  and 6 KiB at 300 ms/1%. Control traffic is 0.1% of the page in the uplink
  direction. Reworking the ACK semantics to save bytes would be pointless, and the
  proposal already says so; this is the number that confirms it.

### A number nobody asked for

The **handshake** costs about 4.5 round trips: 35 / 215 / 569 / 1381 ms at 2 / 40
/ 120 / 300 ms RTT. At 300 ms RTT connecting (1.38 s) costs more than the entire
control plane of a 4 MiB download (1.01 s). If the goal is perceived latency on a
high-RTT link, connection reuse or a pre-warmed connection is a bigger lever than
eight pulls becoming two. That is an observation, not a proposal — nothing about
the connection path is changed here.

## 5. Projection: what a four-chunk range batch would buy

**This section is a projection, not a measurement.** No range batch is
implemented, and nothing in the request/ACK semantics is changed.

| page | today | with 4-chunk pulls | round trips saved | control messages |
| --- | ---: | ---: | ---: | ---: |
| 4 MiB | 1 + 2 waves = 3 | 1 + 1 wave = 2 | **1** | 16 → 4 |
| 10 MiB | 1 + 5 waves = 6 | 1 + 2 waves = 3 | **3** | 40 → 10 |

The wave counts follow the same rule as today (`ceil(pulls / 4)`, four pulls in
flight): 4 MiB needs 2 pulls, which fit in one wave; 10 MiB needs 5 pulls, which
is two waves. Extrapolating with the measured slope instead of the structural
count changes the saving by the wave-boundary overhead: 3.158 − 2 = 1.16 RTT and
6.604 − 3 = 3.60 RTT.

Wall clock saved, in milliseconds and as a share of the measured download:

| saving | 2 ms | 40 ms | 120 ms | 300 ms |
| --- | ---: | ---: | ---: | ---: |
| 4 MiB, 1 RTT, delay-only link | 2 ms (3.7%) | 40 ms (22.9%) | 120 ms (28.1%) | 300 ms (30.1%) |
| 10 MiB, 3 RTT, delay-only link | 6 ms (5.5%) | 120 ms (33.4%) | 360 ms (40.6%) | 900 ms (43.3%) |

The delay-only percentages are the **best case**, because the fixture has no
bandwidth limit. Adding a serialization time for the page (measured floor +
`bytes / bandwidth`) keeps the saving in milliseconds and dilutes it:

| share of the download saved | 40 ms RTT | 120 ms RTT | 300 ms RTT |
| --- | ---: | ---: | ---: |
| 4 MiB on a 25 Mbps link | 2.6% | 6.8% | 12.8% |
| 4 MiB on a 10 Mbps link | 1.1% | 3.2% | 6.9% |
| 10 MiB on a 25 Mbps link | 3.2% | 8.5% | 16.6% |
| 10 MiB on a 10 Mbps link | 1.4% | 3.9% | 8.6% |

Under 1% loss the projection improves by the same wave count: one fewer recovery
window removes one more RTT (4 MiB) and three more (10 MiB), so at 1% loss and
300 ms RTT a 4 MiB download would go 1630 ms → ~1030 ms (−37%) and a 10 MiB
download 3595 ms → ~1800 ms (−50%).

### What the projection ignores

- **Single-chunk retry.** A connection-level retry after a batch failure re-pulls
  2 MiB instead of 512 KiB. The obvious design (retry only the missing chunk)
  needs per-chunk accounting inside the group, i.e. exactly the ACK semantics this
  change would be rewriting, and it is a fourth message per group that this
  projection does not count.
- **Out-of-order and partial group delivery.** The projection assumes a group's
  four chunks all arrive in one round trip, which the unlimited-bandwidth fixture
  guarantees and a real 5–20 Mbps link does not: at 10 Mbps one 2 MiB group takes
  1.7 s to transmit, so the next group's pull would be issued while the first is
  still arriving — a pipeline this fixture cannot show.
- **Host scheduling and disk.** The host reads chunks with four concurrent
  `spawn_blocking` jobs today; a group read changes that shape, and nothing here
  measures disk or queueing.
- **Congestion control.** No window collapse, no queueing delay, no slow start.
  Loss costs one RTT per wave here; on a real link it also halves the window,
  which can cost more than the RTT it saves.

## 6. Conclusion

1. The proposal's premise needs correcting before it is used: **eight pulls are
   not eight round trips**. They are two, because of the existing four-pull
   concurrency window. The change removes **one** round trip from a 4 MiB
   download and **three** from a 10 MiB one, not six and eighteen.
2. In absolute terms that is 300 ms and 900 ms at 300 ms RTT — real, and worth
   something for a 10 MiB attachment on a bad link, marginal for a 4 MiB one.
3. As a share of the download it is 30% (4 MiB) and 43% (10 MiB) on a delay-only
   link, but 7% and 9% on a 10 Mbps link at 300 ms RTT, because the bytes dominate
   the wall clock whenever the bandwidth is realistic.
4. Packet loss does not lower the bar: it raises the payoff about 2× by halving
   the number of recovery windows, while making a failed group retry four times
   as expensive in bytes.
5. Applying the ≥10%-of-the-download bar: **4 MiB pages need RTT ≳ 250 ms with
   ≥20 Mbps; 10 MiB pages need RTT ≳ 60–120 ms on a normal 10–25 Mbps link.**
   Below that, the same engineering effort spent on the 4.5-round-trip handshake,
   or on keeping the connection warm, buys more.
6. If the question is instead "is the request/ACK rewrite *risky* relative to what
   it buys", the measurements say the buy is a small integer number of round
   trips per page, and that the risk surface (group ACK, single-chunk retry,
   backpressure, old-client fallback) is all in the part this fixture cannot
   measure — real loss recovery and real retries.

## 7. What would make this measurement sharper

- A bandwidth-limited link (token bucket in the same proxy) so the projection in
  §5 stops needing a serialization assumption.
- A connection-drop injection, to price the "retry the whole group" failure the
  projection excludes — that is the one number that could reverse the verdict.
- A real RTT/loss measurement from a phone (4G/5G, Wi-Fi) to replace the four
  assumed RTT points with the ones users actually have.
- The same matrix against the mobile TypeScript client, which is the consumer that
  would benefit; this page measures the Rust client only.
