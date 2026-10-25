# Paged replies without base64: the `read_binary_v1` carrier

Task record for changing how a paged read's chunk bytes are carried, with the
measurements that decide whether it was worth doing. Not user-facing
documentation; `docs/testing/` is exempt from the bilingual pairing rule, and the
Chinese pair is written anyway because the request asked for it.

## 1. Run identity

| item | value |
| --- | --- |
| repository | `future-os`, worktree `.worktrees/page-encoding` |
| branch | `perf/remote-page-encoding`, cut from `origin/dev-huichen` |
| base commit | `df43a6230` (`Merge branch 'perf/remote-protocol' into HEAD`) |
| machine | macOS 26.6.2, Apple M3 Ultra |
| toolchain | `rustc 1.97.0 (2d8144b78 2026-07-07)`, the channel `rust-toolchain.toml` pins — invoked through `~/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin`, because the Homebrew `cargo`/`rustc` on `PATH` (1.96.0) shadows rustup and does not read the pin |
| node | v26.3.0 |

Commands, in the order §3's numbers were produced:

```bash
# bytes and per-chunk CPU (deterministic, no sockets)
cargo test --release -p future-remote-interop --features network-tests \
    --test page_encoding -- --nocapture

# loopback wall clock, both carriers, same page
cargo test --release -p future-remote-interop --features network-tests \
    --test page_encoding -- --ignored --nocapture

# correctness over the real encrypted path, every carrier
cargo test --release -p future-remote-interop --features network-tests --test page_encoding
```

`--release` is load-bearing for the CPU numbers: in a debug build the base64
decode is a different order of magnitude and would misrepresent it by ~40×.

## 2. What changed on the wire

### 2.1 Before

A paged read's chunk travelled as:

```
page bytes → base64url (× 4/3, and an alphabet gzip cannot model)
           → JSON string field `readChunk.data`
           → serde_json::to_vec of the whole reply envelope
           → gzip (≥ 32 KiB, negotiated as `reply_gzip_v1`)
           → ChaCha20-Poly1305 seal (+28 header, +16 tag)
           → NATS publish
```

The base64 step is not gratuitous: a JSON *string* is the only place a JSON
document can hold arbitrary bytes, and the reply body was a JSON document.
Below that document the wire is binary already — the sealed record is bytes, and
NATS payloads are binary-safe — so the expansion bought nothing at that layer.

### 2.2 After

A client that declares `read_binary_v1` (and only then) receives the chunk as a
frame instead:

```
"FBR1" | u32 big-endian header length | header (JSON) | chunk bytes (raw)
```

The header is the same reply envelope as before — `{"type","success","data",
"error"}` — with `data` = `{"readChunk": {"id","offset","nextOffset",
"totalBytes"}}`. What moved is the *carrier* of the bytes, not the shape of the
page: every field a reader pins and validates is still JSON, and only `data` (the
base64 string) is gone. The frame is then gzipped by the same policy as any other
reply — the compressed input is now text the compressor can model, which is where
the size win comes from, and gzip is *not* removed (a 0.5 MiB page of JSON text
still compresses to ~180 KB).

Implementation points, one per wire decision:

- `packages/remote-protocol/src/json.rs` — the frame's builder and reader
  (`encode_binary_frame`, `split_binary_frame`), bounded by the same
  `MAX_REMOTE_JSON_BYTES` (1 MiB) the reader already enforced. Four ASCII bytes
  that cannot begin a JSON document and are not gzip's magic, so the reader
  decides between the carriers by inspection and never has to be told.
- `packages/remote-protocol/src/lib.rs` — `READ_BINARY_FEATURE`.
- `packages/remote-protocol/src/handshake.rs` — advertised in `SERVER_FEATURES`.
- `packages/remote-server/src/services.rs` — `ReplySink::send_binary(header,
  body)`; its **default** is the legacy base64 page, so a sink with no connection
  behind it (a test double, an embedder) produces exactly the bytes it did
  before.
- `packages/remote-server/src/commands.rs` — `HandshakeState::binary_pages` is
  set from the declaration in `apply_declared_features`, exactly like
  `gzip_replies`/`lean_events`; `NatsReply::send_binary` frames only when it is
  set, and falls back to the base64 page otherwise. The single-flight cache stores
  the encoded payload it actually sent, so a replayed command id replays the same
  carrier. The negotiated gzip flag moved from the single-flight wrapper into
  `handle_command`, the one function every path reaches; §3.3 explains the defect
  that motivated it.
- `packages/remote-server/src/reply.rs` —
  `encode_binary_reply_payload_with_gzip`, sharing the size guard, the 32 KiB
  threshold and the "never expand" check with the JSON encoder.
- `host/src/remote_host/read_pages.rs` — a page is now `Page { header, body }`:
  the host no longer decides the carrier at all, so both `PagedReply` and
  `get_read_chunk` hand bytes to the sink apart from the JSON.
- `packages/remote-client/src/session.rs` — `Reply::{Data, Page}`; the decoder is
  shape-driven (inflate, then look for the magic), so one client reads both
  carriers and a host that never learned the flag keeps working unchanged.
- `packages/remote-client/src/runtime.rs` — `PageChunk` folds "bytes came from a
  frame" and "bytes came from base64" into one shape before any check runs, so
  the snapshot assertions exist once, not twice.
- `packages/remote-client/src/{readPages.ts,remoteJson.ts,client.ts}` — the TS
  client (phone and desktop) decodes the frame, keeps the body bytes beside the
  parsed reply (non-enumerable, so no existing equality assertion sees it), and
  declares the capability only when the desktop's confirmation advertised it.

## 3. Measurements

All page sizes below are the JSON bytes of a `get_session_entries` reply (the
real reply envelope), split at the transport's own 192 KiB chunk, gzipped above
its own 32 KiB threshold, plus the fixed 44-byte seal overhead per message. The
byte figures are the *encoded reply* lengths from the production encoders, which
is exactly the part the change moves; the seal overhead is constant and identical
in both arms. §3.2 then checks them against the socket, because a number computed
from the same code it is meant to judge is only half a measurement.

Three corpora, because base64's cost is a property of the content:

- **prose** — English-looking assistant text, gzips ~2.8×. The everyday case.
- **tool output** — shell transcripts, paths and exit footers; highly repetitive.
- **hex identifiers** — id/hash-heavy text gzip cannot model well.

### 3.1 Wire bytes, same page, both carriers

| corpus | page | chunks | base64 | frame | saved |
| --- | --- | --- | --- | --- | --- |
| prose | 525 072 B | 3 | 184 736 B | 127 690 B | **30.9 %** |
| prose | 4 199 547 B | 22 | 1 478 526 B | 1 019 707 B | **31.0 %** |
| tool output | 528 354 B | 3 | 83 385 B | 38 309 B | **54.1 %** |
| tool output | 4 197 784 B | 22 | 658 580 B | 306 663 B | **53.4 %** |
| hex identifiers | 524 825 B | 3 | 442 656 B | 249 381 B | **43.7 %** |
| hex identifiers | 4 197 969 B | 22 | 3 542 455 B | 1 995 275 B | **43.7 %** |

Reading: **31 %–54 %**, and the spread is the content, not the software. Base64
mainly costs where the content does not compress (`hex`), and costs least where
gzip can model the text either way (`prose`). The 4 MiB rows match their 0.5 MiB
twins to a tenth of a point, which is what says the win is per-chunk and does not
depend on the page's length.

### 3.2 The same page measured on the socket

A counting TCP proxy between the client and the broker, sampled immediately
before the command and immediately after its reply, so nothing but the read is in
the window. The host keeps its own direct connection: routing both ends through
the proxy makes each direction report the whole conversation, which is how the
first attempt at this measurement read ~5.4 MB in *both* directions for a 4.2 MB
page.

| arm | broker → client | client → broker (requests) |
| --- | --- | --- |
| base64 | 1 480 668 B | 7 844 B |
| frame | 1 021 918 B | 7 844 B |
| saved | **31.0 %** | 0 (by design) |

The socket agrees with §3.1 to 0.05 percentage points, and the request leg is
byte-identical in both arms — the change moves replies only, and the client's
twenty-two chunk pulls are the same commands either way.

### 3.3 A defect this measurement found: page chunks were never gzipped

The first run of §3.2 reported 5.41 MB for the base64 arm, against 1.48 MB from
the encoder. The gap was not a measurement artefact: `get_read_chunk` is the one
command that deliberately skips the single-flight reply cache (so it does not
retain a second copy of every chunk), and the negotiated gzip flag was installed
by that wrapper — so every chunk after the first was served **uncompressed**.
Measured on the 4 MiB page: 1 of 22 chunks gzipped, 21 at 262 KB of base64 each
instead of ~90 KB. This has been true for every client since `reply_gzip_v1`
landed, not just for the carrier this record is about.

The flag now lives in `handle_command`, the one function every command path
reaches, instead of in the wrapper. §3.2's figures are measured *after* that fix;
against the old behaviour the base64 arm cost 5.41 MB and the frame arm 4.06 MB
(the 25 % arithmetic bound, because nothing was compressed), which is the state
this change and the fix together replace.

### 3.4 CPU per chunk (release, 192 KiB chunk)

| step | base64 carrier | frame carrier |
| --- | --- | --- |
| reader: decode one chunk | 156 445 ns | 891 ns |
| host: encode one chunk | 190 972 ns | 105 ns |

The frame's "decode" is a slice and a small `serde_json` parse; base64's is
splitting + `serde_json` unescaping + base64 decode of 262 KB. Per 4 MiB page (22
chunks) that is ~3.4 ms of reader CPU and ~4.2 ms of host CPU removed. A phone
pays more than this machine, and its base64 path has no SIMD floor to hide
behind — but this record measured one machine, and a phone number is not
extrapolated from it.

### 3.5 Real encrypted path over loopback, 4 MiB page / 22 chunks

Same page, same client, same broker, same sealed channel; only the fixture's
carrier differs. Two samples, three rounds each (round 0 includes connection
warm-up):

Measured with the gzip fix of §3.3 in place — every chunk is compressed in both
arms, so the host does more work and the wire carries less than the pre-fix runs.

| sample | arm | round 0 | round 1 | round 2 |
| --- | --- | --- | --- | --- |
| 1 | base64 | 56.3 ms | 29.9 ms | 26.1 ms |
| 1 | frame | 46.1 ms | 25.9 ms | 19.2 ms |
| 2 | base64 | 57.1 ms | 30.0 ms | 26.0 ms |
| 2 | frame | 44.6 ms | 24.7 ms | 19.2 ms |

Warm rounds: 28.0 → 22.6 ms (−19 %) in sample 1, 28.0 → 22.0 ms (−21 %) in sample
2. **This is not a network measurement.** A loopback link has no RTT and no
bandwidth limit, so what is left in these numbers is encode work, decode work and
copying — which is §3.4 seen from the outside. On a real link the byte reduction
of §3.1–§3.2 is what would matter, and no such link was measured here (§5).

Every run in §3.5 also asserts the reassembled page is **byte-identical** to the
snapshot in both arms (`serde_json::to_vec(&read) == page`), which is what makes
these two arms the same work rather than two different tasks.

## 4. Why the number is 31 %, not 44.6 %

The 44.6 % figure this replaced came from a 0.5 MiB page whose JSON gzipped to
23.7 KB uncompressed-of-base64 and 42.8 KB after base64 — a page that compressed
**22×**. That is not what a transcript looks like; on that corpus gzip was doing
almost all the work on the raw bytes and base64 was destroying nearly all of it,
which is the best case for this change rather than the typical one.

Measured on real-shaped text the honest range is **31 %–54 %** (§3.1), with
prose — the common case for a conversation page — at the bottom of it. Stated as a
rule: the saving is the 4/3 expansion minus whatever gzip was already recovering
from the base64 alphabet, so it is largest for high-entropy content and smallest
for natural language.

## 5. What was not measured, and why

- **A phone.** The TS client is covered by tests (§6) but not timed; the numbers
  above are Rust on an M3 Ultra. A mobile CPU is slower at base64 by more than it
  is slower at memcpy, so the direction holds, but no phone number is claimed.
- **A weak network.** No RTT/loss fixture exists in this repository, and the
  interesting question there (round trips vs. bytes) is a different change
  entirely. Loopback timings are reported as CPU, not as a link.
- **A user's real conversation.** The pages are generated in the real reply
  shape and size range (0.5 MiB and 4 MiB, the sizes that actually trigger
  paging), not captured from a stored session. A capture would be more faithful
  and was not available to this task.

## 6. What must not have changed, and is asserted

Correctness, not performance — every item below has a test that fails if it
regresses:

| property | where it is pinned |
| --- | --- |
| the first reply pins an immutable snapshot; `id`, `totalBytes`, `offset`, `nextOffset` are all still checked per chunk | `readPages.test.ts` (owner/offset/total), `runtime.rs` `store_chunk`, `host/src/remote/commands.rs` carrier test |
| a chunk from another snapshot is refused even at the same offsets | `readPages.test.ts` "binary chunk from another snapshot", `store_chunk`'s id check |
| a short chunk while the page continues is refused on **both** carriers | `readPages.test.ts` (base64 + binary), `store_chunk` |
| out-of-order chunks write into disjoint ranges; `READ_CONCURRENCY = 4` bounded batches; a failure launches no second wave | `readPages.test.ts` (unchanged, still green) |
| `MAX_READ_BYTES = 16 MiB` and the ≤ 192 KiB chunk bound | `readPages.ts`, `runtime.rs`, `host/read_pages.rs` |
| an undeclared client still receives the base64 page, byte for byte | `host/src/remote/commands.rs::paged_reads_switch_carrier_only_for_a_connection_that_declared_it` (before/after/withdrawn) |
| a declared client that is served a base64 page still reads it | same test's base64 arm; `an_undeclared_connection_receives_the_base64_page` |
| an old desktop that ignores `chunkedRead` is unaffected | the page is only produced for `chunkedRead`, which that code never sets |
| the frame's header is bounded by the reader's 1 MiB cap at build time | `remote-protocol/src/json.rs` |
| a declared connection's *chunk* replies are gzipped like any other reply (the §3.3 defect) | `host/src/remote/commands.rs` carrier test asserts the frame's magic *after* inflation; the interop socket measurement fails if the pages stop compressing |

The host-side test is the load-bearing one: it drives the real desktop host, the
real paging transport and the real reply path, and asserts that the same snapshot
reassembles to **identical bytes** before the declaration, with the frame after
it, and to base64 again once the declaration is withdrawn.

## 7. Gates run

| gate | result |
| --- | --- |
| `cargo test -p future-remote-protocol --lib` | pass (28) |
| `cargo test -p future-remote-server --lib` | pass (174, 5 ignored) |
| `cargo test -p future-remote-client --lib` | pass (66) |
| `cargo test -p future-remote-interop --features network-tests` | pass (all targets, incl. the new `page_encoding` suite) |
| `cargo test -p future-host --lib` | pass in two runs — `--skip agent_bridge` (1442 passed, 0 failed, 10 ignored) and `-- agent_bridge` (376 passed, 0 failed); the two together deadlock in parallel here, see §8 |
| `cargo fmt --check`, `cargo clippy --all-targets` on every touched crate | clean |
| `npx vitest run` (remote-protocol), `npx jest --runInBand` (remote-client) | pass (57 / 667) |
| `make check-docs` | see §8 |

## 8. Suite notes

- **The full `future-host --lib` run deadlocks in parallel on this machine, and
  did so before this change too.** Run as one binary, a family of
  `agent_bridge::*` tests (`import`, `approval`, `headless`, `models`, `observer`)
  each reports "running for over 60 seconds" with almost no progress: they queue
  behind one process-global fixture lock and the HOME↔mock-agent ordering never
  lets the holder go. Split in two, both halves are green and fast:
  `-- --skip agent_bridge` → 1442 passed / 0 failed / 10 ignored in 417 s;
  `-- agent_bridge` → 376 passed / 0 failed in 60 s. `agent_bridge::import::tests`
  alone is 48 passed in 3.6 s. None of those tests touch the remote stack this
  change modifies, and the deadlock is the one already recorded in
  `.future/memory/testing-gotchas.md`; it is reported here rather than worked
  around, because a suite that cannot run as one binary is a finding of its own.
- One test did fail on the first full run and was a real consequence of this
  change, not a flake: `remote::commands::bridge_tests::handshake_roundtrip_activates_the_bridge`
  pins the confirmation's exact `features` array, and the protocol fixture
  (`packages/remote-protocol/fixtures/protocol.json`) pins it again from both
  languages. Both were updated to include `read_binary_v1`, which is the
  advertised-list change this record is about.
