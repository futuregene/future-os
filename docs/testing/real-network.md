# Real-network runs of the remote lane (2026-10-17)

This record answers one question the rest of `docs/testing/` cannot: does the
remote stack work when the broker, the peer and the Agent are **real processes**
on a real TCP network? It belongs to todo `todo_f5453bf671b4` (REAL-NET); the
coverage record it supplements is
[`module-tauri-remote.md`](module-tauri-remote.md).

Everything below is a command that was run and the output it produced. A result
that failed is reported as a finding, not smoothed over; a scenario that could
not be exercised is named in §7 with the reason.

## 1. Environment

| item | value |
|---|---|
| host | macOS 26.6.2 (25G83), arm64 |
| Rust | 1.97.0 (`rust-toolchain.toml`), `CARGO_TARGET_DIR=desktop/src-tauri/target` |
| broker | `/opt/homebrew/bin/nats-server` **v2.15.0** |
| broker listeners | client `127.0.0.1:64664`, monitoring `127.0.0.1:64665` (fresh ports chosen for this run) |
| broker store | `/tmp/realnet/nats/js`, log `/tmp/realnet/logs/nats.log` |
| Agent | `target/debug/future agent --home /tmp/realnet/agenthome/.future --grpc-addr 127.0.0.1:64732` (pid 76134) |
| model vendor | local OpenAI-compatible stub on `127.0.0.1:64731` (`/tmp/realnet/fake_model.py`, pid 80942): streams text and requests one `shell` call |
| test HOME | `/tmp/realnet/home` for the cargo process; the host and client children get their own `HomeGuard` temp HOME and are removed on exit |
| isolation | nothing under the operator's `~/.future` was read or written; the Agent above is a second instance with its own lock, DB and socket |

The Agent's model file (isolated home only):

```json
{
  "providers": {
    "localmock": {
      "api": "openai",
      "baseUrl": "http://127.0.0.1:64731/v1",
      "apiKey": "local-test-key",
      "models": [{ "id": "mock-1", "contextWindow": 16000, "maxTokens": 1024 }]
    }
  }
}
```

What is real and what is not, before any result is read:

* **real:** `nats-server` process and TCP, Noise v2 handshake and per-record
  AEAD, NKey signing on both ends, the production command loop / event
  publisher / transfer loop, the `future-agent` process (its gRPC contract,
  session database and run journal), and the two remote ends running in
  **separate OS processes**;
* **stubbed:** the model vendor (the local OpenAI-compatible endpoint above);
* **bypassed:** the FutureOS platform's pair-code / claim / refresh endpoints.
  The host mints its v2 invitation locally, exactly the `PairingCreds` the
  platform would return. Pairing *policy* lives behind that HTTP call; the
  transport and handshake it authorises are the shipping ones.

## 2. The ignored real-broker test, run with the broker up

The ticket estimated "40 `#[ignore]`d tests whose reason strings name a real
broker". In this lane the actual count is **six**: four in
`remote/verify_e2e.rs` and two in `remote/publisher/coalesce.rs` (the other
`#[ignore]`s in the repository are sandbox/OS/browser-fixture harnesses, listed
in §7). All six were left `#[ignore]`d — their reason strings are still true —
and invoked with `--ignored`. All six were run; the last one
(`measure_catalog_activity_lane`) needs no external process and is reported in
§3.5.

```bash
cd desktop/src-tauri
export CARGO_TARGET_DIR=$PWD/target HOME=/tmp/realnet/home
export VERIFY_E2E_NATS_URL=nats://127.0.0.1:64664
cargo test --lib remote::verify_e2e::verify_e2e_real_broker_round_trip \
  -- --ignored --exact --test-threads=1 --nocapture
```

**PASS** (0.89 s), with the fixture's own numbers:

```
VERIFY_E2E_FIXTURE {"live":{"undeclared":{"messages":7,"wireBytes":2297,"plaintextBytes":1989,"dataBytes":357},"declared":{"messages":5,"wireBytes":975,"plaintextBytes":755,"dataBytes":230}},"history":{"undeclared":{"full":14305,"paged":14311},"declared":{"full":550,"paged":556}}}
test result: ok. 1 passed; 0 failed; 0 ignored; 2109 filtered out
```

This is the self-contained proof (real handshake, real bridge, real
subscription, both declaration states, both history shapes) running against
`nats-server` instead of `FakeNats`.

## 3. The measurement drivers, on a real run

The two drivers need one real run's journal, its session id, its run id and the
real `get_session_entries` dump. A run was produced on the isolated Agent by a
prompt whose stub answer streams in 20-character pieces and asks for one `shell`
call; the raw `run_events` rows were dumped **while the run was still
completing** (the run journal is compacted into `run_snapshots` at completion),
and the history was read with `grpcurl` after completion:

```bash
# journal (mid-run, raw rows)
sqlite3 /tmp/realnet/agenthome/.future/agent/agent.db \
  "select payload from run_events where run_id='run_548dc17c6cbf4c169eab821f1df220d6' order by idx;" \
  > /tmp/realnet/run/journal.jsonl

# history (after completion; the readiness headers are the client contract)
grpcurl -plaintext -max-msg-sz 268435456 -import-path packages/rpc/proto \
  -proto future.proto \
  -H "x-future-agent-instance: <ready instance>" \
  -H "x-future-agent-data-root: <ready root>" \
  -H "x-future-client-version: <agent version>" \
  -H "x-future-client-protocol: 2" -H "x-future-client-storage-schema: 6" \
  -d '{"id":"e1","type":"get_session_entries","session_id":"<sid>"}' \
  127.0.0.1:64732 proto.FutureAgent/ExecuteCommand
# the reply's getSessionEntries.entries were converted to the desktop relay's
# payload shape by scripts/measure/verify-e2e-bytes.py::to_payload_shape
```

The journal held 19 events (`user_message`, `agent_start`, two `tool_start`,
`tool_delta`, `tool_end`, 13 `text_chunk`) and the history held 5 entries with
`text`, `tool_call` and `tool_result` blocks.

### 3.1 Real traffic over the real broker

```bash
export VERIFY_E2E_JOURNAL=/tmp/realnet/run/journal.jsonl \
       VERIFY_E2E_SESSION=20261017-133735-2a31aaa709bb4928bf9ddb4c1e792dbf \
       VERIFY_E2E_RUN=run_548dc17c6cbf4c169eab821f1df220d6 \
       VERIFY_E2E_ENTRIES=/tmp/realnet/run/entries.json \
       VERIFY_E2E_NATS_URL=nats://127.0.0.1:64664 \
       VERIFY_E2E_NATS_MONITOR=http://127.0.0.1:64665
cargo test --lib remote::verify_e2e::measure_real_broker_bytes \
  -- --ignored --exact --test-threads=1 --nocapture
```

**PASS** (0.93 s). The measurement's own markers:

```
VERIFY_E2E_LIVE      {"journalEvents":19,
  "undeclared":{"messages":19,"wireBytes":10070,"plaintextBytes":9234,"dataBytes":2206},
  "declared":{"messages":17,"wireBytes":5207,"plaintextBytes":4459,"dataBytes":1767}}
VERIFY_E2E_LIVE_PHONE {"coalesced":{"messages":7,"wireBytes":5070,"plaintextBytes":4762},
  "coalescedLean":{"messages":5,"wireBytes":2723,"plaintextBytes":2503},
  "leanMarginal":0.4629}
VERIFY_E2E_HISTORY   {"undeclared":{"full":{"wireBytes":3925,"plaintextBytes":3881,"entries":5},
  "paged":{"wireBytes":3931,"plaintextBytes":3887,"entries":5}},
  "declared":{"full":{"wireBytes":3698,"plaintextBytes":3654,"entries":5},
  "paged":{"wireBytes":3704,"plaintextBytes":3660,"entries":5}}}
VERIFY_E2E_NATS_ACCOUNTING {"receivedMessages":55,"receivedBytes":38631,
  "serverOutMessages":59,"serverOutBytes":38695,"sentMessages":11,"sentBytes":1082}
VERIFY_E2E_LAZY_FETCH {"shellRows":1,"carriedCommandBytes":20,
  "fetchOverheadBytes":405,"pagedShellRows":1,"pagedCarriedCommandBytes":20}
test result: ok. 1 passed
```

The broker's `/connz` counters for the phone connection were read from the real
monitoring endpoint; the delta matches the harness's received-and-decrypted
totals after subtracting the subscription probes, which the test asserts.

### 3.2 The same traffic over the in-process broker

```bash
unset VERIFY_E2E_NATS_URL VERIFY_E2E_NATS_MONITOR
cargo test --lib remote::verify_e2e::measure_real_e2ee_bytes \
  -- --ignored --exact --test-threads=1 --nocapture
```

**PASS** (0.95 s). `VERIFY_E2E_LIVE`, `VERIFY_E2E_HISTORY` and
`VERIFY_E2E_LAZY_FETCH` were **byte-for-byte identical** to §3.1. That is the
cross-check the script's `--broker both` performs: the broker is a relay, so the
decoded numbers must not depend on which broker carried them.

### 3.3 Coalescing over the real journal

```bash
export SYNC_MEASURE_JOURNAL=/tmp/realnet/run/journal.jsonl \
       SYNC_MEASURE_SESSION=20261017-133735-2a31aaa709bb4928bf9ddb4c1e792dbf \
       SYNC_MEASURE_RUN=run_548dc17c6cbf4c169eab821f1df220d6

# (a) the production window (100 ms)
cargo test --lib remote::publisher::coalesce::tests::measure_real_journal \
  -- --ignored --exact --test-threads=1 --nocapture
# (b) a window that actually merges this provider's pacing
SYNC_MEASURE_WINDOW_MS=5000 cargo test --lib \
  remote::publisher::coalesce::tests::measure_real_journal \
  -- --ignored --exact --test-threads=1 --nocapture
```

* **(a) FAILS**: `panicked at coalesce.rs:1007: coalescing must not grow the
  lane`. With the stub's chunks ~100 ms apart the production window merged
  nothing, and the coalesced lane was larger than today's per-event lane.
  **This is a real observation**, recorded rather than worked around: the
  measurement's assertion is doing its job, and the ticket is not authorised to
  change the coalescer. Its cause is the envelope the merged body carries
  (`coalescedCount`) on events that merge with nothing; a production provider
  that streams many tokens per 100 ms is the common case, but a slow provider or
  a sparse run is not excluded. Owner decision: treat as a product finding.
* **(b) PASSES**, and shows the intended effect when fragments do merge:

```
LIVE_LANE {"events":19,"todayBytes":9234,"todayDataBytes":2206,
  "coalescedBytes":4762,"coalescedDataBytes":2074,"published":7,
  "parties":19,"windowMs":5000,"ratio":1.939}
```

### 3.4 Lean trimming over the real journal

```bash
cargo test --lib remote::publisher::coalesce::tests::measure_real_journal_lean \
  -- --ignored --exact --test-threads=1 --nocapture
```

**PASS**:

```
LEAN_LANE {"events":19,"delivered":17,"fullBytes":9234,"leanBytes":4459,
  "leanDataBytes":8021,"published":17,"windowMs":100,
  "reduction":0.5171,"publishedByType":{...equals leanByType...}}
```

i.e. 51.7 % fewer bytes on the real event mix, every delivered event accounted
for exactly once, and the newest index still published.

### 3.5 Catalog activity (the sixth ignored driver)

This one needs no real broker and no external file — its inputs have in-code
defaults (`CATALOG_ACTIVITY_ROWS` 232, writes 20) and it runs on `FakeNats`:

```bash
cargo test --lib remote::verify_e2e::measure_catalog_activity_lane \
  -- --ignored --exact --test-threads=1 --nocapture
```

**PASS** (2.69 s):

```
CATALOG_ACTIVITY {"rows":232,"writes":20,
  "declared":{"sessionsPackets":1,"sessionsPayloadBytes":98411,
              "activityPackets":19,"activityPayloadBytes":3926,"payloadBytes":102337},
  "undeclared":{"sessionsPackets":20,"sessionsPayloadBytes":1970400,
                "activityPayloadBytes":0,"payloadBytes":1970400},
  "payloadSaved":0.9481}
```

i.e. a declared client receives 19 small activity packets instead of 20 whole
catalogues: 94.8 % fewer bytes across 20 writes.

## 4. Two-process pairing, transfer and prompt

The fixture that collapsed host and client into one process cannot see a
transport boundary. `remote/real_network.rs` starts the **production host bridge
in one child process** and the **`remote_peer` client in another**, over the real
broker, with a real `future-agent` behind the host. The host's invitation is
minted locally (v2 `PairingCreds`), so the platform HTTP hop is the only part
not exercised.

```bash
cd desktop/src-tauri
export CARGO_TARGET_DIR=$PWD/target HOME=/tmp/realnet/home \
       REALNET_DIR=/tmp/realnet/run REALNET_LOG_DIR=/tmp/realnet/logs \
       REALNET_NATS_URL=nats://127.0.0.1:64664 \
       REALNET_AGENT_ADDR=127.0.0.1:64732
cargo test --lib remote::real_network::real_network_two_process_host_and_client \
  -- --ignored --exact --test-threads=1 --nocapture
```

**PASS** (4.12 s). What each process observed:

| step | observed |
|---|---|
| host bridge up | `REALNET_HOST_READY pair=pair_e2e-…`; invitation written to `/tmp/realnet/run/host.json` by the host process |
| client pair + connect | `desktopId`, `bridgeInstanceId: bridge_realnet-host-…`, `agentAvailable: true`, 19 host features (incl. `e2ee_v2`, `file_transfer_v1`, `file_download_v2`) |
| upload, multi-chunk (client → host) | 1 200 000 bytes in **3** 512 KiB chunks; the host's reply hash equals the client's SHA-256 of all 1.2 MB |
| upload, attached (client → host) | 25 bytes; the host's reply hash `e134cae7…` equals the client's SHA-256 of the bytes |
| download (host → client) | 960 000 bytes in 2 chunks; SHA-256 `ee8e79a5…` equals the host's declared hash and the source file |
| workspace create | `workspaceId: ws-…` registered by the host, directory `/tmp/realnet/run/workspace` |
| prompt → real Agent | ack `{sessionId, runId}`; `agent_end` received on the sealed event lane after **26 text chunks**, and the accumulated answer contains the unique prompt marker (`answerEchoed: true`) |

The prompt carries the uploaded file as an attachment, so the upload is not only
hash-checked but consumed by the Agent. Client and host are separate PIDs; the
host's Agent is a third process (`future agent`, pid 76134).

## 5. Failure paths that only exist on a real network

```bash
export REALNET_NATS_PID=<pid of the nats-server this run started>
export REALNET_NATS_ARGV='["/opt/homebrew/bin/nats-server","-a","127.0.0.1","-p","64664","-m","64665","-sd","/tmp/realnet/nats/js","-l","/tmp/realnet/logs/nats.log"]'
cargo test --lib remote::real_network::real_network_failure_paths \
  -- --ignored --exact --test-threads=1 --nocapture
```

**PASS** (67 s; the 12 s outage is intentional). The client's report:

```json
{
  "socketClosedThenCommandSucceeded": true,
  "broker": {
    "midTransfer": {
      "completedBeforeKill": false,
      "error": "Remote request failed: request timed out: deadline has elapsed"
    },
    "retriedBytes": 167772166,
    "brokerPid": 90639,
    "hostBack": true,
    "requestDuringOutage": { "ok": true },
    "recovered": true
  },
  "hostRestartResumed": true
}
```

Scenario by scenario:

1. **Socket closed under an open stream.** The client's live session is removed
   and its NATS socket closed (`close_socket_for_test`); the next command has to
   reconnect before it can be answered. `list_sessions` succeeded
   (`true`).
2. **Broker killed mid-transfer.** A 160 MiB download (320 chunks) is started;
   the broker is killed ~5 ms later and kept down for **12 s** — longer than the
   client's 10 s request budget, so the pull fails rather than being masked by
   reconnect buffering: `request timed out: deadline has elapsed`. The client
   then restarted the same broker on the same port; the host bridge came back
   (`hostBack: true`), and a **fresh registration of the same file** downloaded
   all 167 772 166 bytes with the expected SHA-256 (`retriedBytes`).
   *Method note:* an earlier 40 MiB attempt reported `completedBeforeKill: true`
   because the orchestrator's 50 ms file-poll added more latency than the
   loopback transfer took. The kill therefore moved into the client process,
   which can signal the broker immediately after spawning the transfer; the
   first failed attempt is kept here as evidence of why the harness is shaped
   this way.
3. **NATS reconnect after a broker restart.** The same outage exercises it: a
   command issued after the restart succeeded (`requestDuringOutage.ok: true`),
   and the host's bridge answered again (`hostBack: true`).
4. **Peer restarted with the same identity.** Host 1 exits; host 2 starts from
   the credential host 1 persisted **after** pairing (the client's Noise static
   key pinned), and the client's socket is closed so the next connection has to
   use `Noise_IK` — the invitation PSK was spent by the first pairing.
   `hostRestartResumed: true`, and host 2 logged no `peer_unknown` refusals.
   (The first attempt did produce `remote_secure_channel_invalid (peer_unknown)`
   because the harness saved the credentials *before* pairing; that is recorded
   as a harness bug, fixed by persisting the paired credential at shutdown.)

## 6. Commands used to stand the environment up

```bash
mkdir -p /tmp/realnet/{home,nats,logs,run,agenthome/.future/agent}
nats-server -a 127.0.0.1 -p 64664 -m 64665 \
  -sd /tmp/realnet/nats/js -l /tmp/realnet/logs/nats.log &
python3 /tmp/realnet/fake_model.py 64731 &
# models.json as in §1, then:
HOME=/tmp/realnet/agenthome target/debug/future agent \
  --home /tmp/realnet/agenthome/.future --grpc-addr 127.0.0.1:64732 \
  --log-file /tmp/realnet/logs/agent.log &
# a real run whose journal the measurement replays:
HOME=/tmp/realnet/agenthome target/debug/future run \
  --grpc-addr 127.0.0.1:64732 --model localmock/mock-1 --no-title \
  --no-wait --mode json "<prompt>"
```

Every listener used a port chosen for this run; the operator's agent (the one
serving the session) was never contacted. The processes started here were
stopped at the end; their pids were recorded in `/tmp/realnet/*.pid`.

## 7. Not exercised, and why

| not covered | why |
|---|---|
| **TLS and WebSocket NATS transport** | both the host bridge (`supervisor/start.rs:604-605`) and the client (`remote_peer/session.rs:762-763`) call `require_tls(true)` only under `#[cfg(not(test))]`. These runs are a test binary, so they speak **plaintext** NATS. Production TLS (and the `wss://` path) needs a TLS-configured broker and a non-test build; not done. |
| **FutureOS platform pair-code / claim / refresh / revoke** | needs a signed-in FutureOS account and the platform service. The invitation and the peer grant were minted locally. The handshake they authorise is the shipping one, but the HTTP policy layer is not exercised. |
| **The `futureos-headless` / GUI desktop process** | `headless/mod.rs` requires a platform login before it starts the remote bridge. The host here is the same production bridge code, started by the test fixture instead of the app runtime, so Tauri/plugin wiring is not covered. |
| **A real LLM vendor** | the model is a local OpenAI-compatible stub (text + one `shell` tool call). Token streaming shape and tool scheduling are exercised; model quality/behaviour is not. |
| **The real mobile app** | the client is `remote_peer` (the desktop client role), not the React Native phone. They share the wire protocol and crypto, but this is not an on-device run. |
| **`download_prepare` policy** | the host half of a download was registered with `register_download_for_test` (the same record `download_prepare` creates), so the chunk serving and the client's pull/verify are real but the session-attachment allow-list and name/type policy are not exercised here. Uploads use the real `upload_init`/chunk/`upload_complete` path. |
| **Packet loss, latency, NAT rebinding** | loopback only. `tc`/`netem` needs root and a Linux host; no impairment was injected. |
| **A NATS cluster / JetStream / auth callout** | one core `nats-server`, no accounts, no TLS, no clustering. The NKey signs the CONNECT but the server does not verify it against an account. |
| **Revocation while connected** | platform-side; not reachable without the service. |
| **`remote_host` ignored measurement drivers** | `sync_measurement::serve_real_snapshot`, `wire_limits::measure_phone_page`, `lean::generate_render_fixtures`, `lean::measure_real_entries` need a browser fixture, a real desktop DB snapshot or a catalog/journal dump that this environment does not have. They remain `#[ignore]`d; `verify_e2e::measure_catalog_activity_lane` **was** run (§3.5). |
| **Other crates' ignored tests** | `agent/tests/sandbox_smoke.rs` (macOS sandbox), `linux_sandbox_smoke.rs` (needs Linux/bwrap), `channels/tests/agent_integration_test.rs`, `tasks.rs`'s real-agent manual harness — outside this ticket's scope and not needed for the remote lane. |

## 8. Findings worth acting on

1. **Coalescing can grow the lane when nothing merges** (§3.3a): with the
   production 100 ms window and a provider emitting ~100 ms-spaced chunks, the
   published bodies were larger than the per-event lane; the assertion is
   `coalescing must not grow the lane`. Decide whether the merged envelope
   should be omitted for single events, or the assertion relaxed with a
   documented envelope allowance — do not silently change the measurement.
2. **A 12 s broker outage fails an in-flight pull at the client's 10 s budget**
   (§5.2), but the same pull succeeds later against a fresh registration; the
   host bridge recovered and answered `list_sessions` again. This is the
   behaviour the F-TAURI-4/5 classifiers were written for, now observed on a
   real broker.
3. **Peer restart needs the paired credential, not the invitation credential**
   (§5.4): a host that persists `remote_pairing.json` before the handshake
   completes will refuse every `Noise_IK` reconnect with `peer_unknown`. The
   production path saves after pairing; the harness bug (saving before) is the
   same mistake an operator copying the file at the wrong moment would make.

## 9. Coverage consequence

`desktop/src-tauri/src/remote/real_network.rs` is a `#[cfg(test)]` harness whose
tests are `#[ignore]`d because they need an external broker, Agent and model
stub. Under the ordinary suite (CI, the coverage command in
`module-tauri-remote.md` §1) those bodies are `DA:0`, exactly like the ignored
drivers already registered in the ledger. The next coverage round should
classify the file as **`unreachable-in-this-environment` (external-process
harness)** or exclude it by path — it is verification machinery, not product
code, and its uncovered lines are not a product-coverage gap. The six ignored
drivers that pre-existed this file keep their existing ledger rows; this ticket
did not change their `#[ignore]` annotations or assertions.
