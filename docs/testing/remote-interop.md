# Independent Remote interoperability

This private test project composes the shipping `future-remote-client`,
`future-remote-server` and `@future-os/remote-client` APIs. It imports no Desktop,
Tauri, Agent, Mobile, Expo, application stores or application test configuration.
Its host binary is test infrastructure, not a new product CLI or Server entry.

A separately owned `nats-server` process listens on random loopback TCP and
WebSocket ports. Rust roles use the explicit loopback-only `test-support`
transport option; TypeScript supplies a loopback-only WebSocket factory. Neither
production TLS policy nor account/relay authentication is replaced in product
code. The fixture broker deliberately has no JWT/subject ACL enforcement: relay
and application authorization tests remain separate coverage.

The roles run actual NATS subscriptions, Noise XXpsk0/IK, authenticated records,
command admission/deduplication, event publication and chunk transfer. Only
business, credential persistence, account services and file IO use explicit
memory hosts. This is not a replacement for real Store/Agent/approval/sandbox,
native-file, GUI/headless exit or Mobile lifecycle integrations.

## Inputs and reports

The base shared input/expectation file is
[`client.json`](../../packages/remote-client/fixtures/client.json). It defines a
version, request case IDs, event/file inputs and flow report IDs. Both language
drivers reject unknown versions/kinds/fields and duplicate IDs. Objects compare
by JSON value; shared numeric cursors use the JavaScript safe-integer range.
Each successful case prints `remote-interop <language> <case-id> PASS` without
credentials or private key material.

The Rust host's stdout is private JSON-lines IPC containing ephemeral fixture
credentials. The TS driver consumes it internally; never tee, publish or snapshot
that channel. Normal test output contains IDs and outcomes only. The broker,
config directory, role tasks and host process are fixture-owned and cleaned up
by their owners, including error paths.

## Static checks during development

```text
cargo fmt -p future-remote-interop --check
cargo clippy -p future-remote-interop --features network-tests --all-targets -- -D warnings
npm run typecheck --workspace @future-os/remote-interop
npm run lint --workspace @future-os/remote-interop
npm run build --workspace @future-os/remote-interop
```

`build` only compiles the Node bundle; it does not run the test driver. Normal
workspace Cargo builds do not enable `network-tests` or launch any process.

## Automated acceptance in CI

Requires Node 24+, the pinned Rust toolchain, root npm dependencies and
`nats-server` on PATH. The network test must fail if the broker is unavailable;
it never silently skips or falls back to a simulated endpoint.

```text
cargo test -p future-remote-interop --features network-tests --test rust_client_server -- --nocapture
cargo build -p future-remote-interop --features network-tests --bin remote-interop-host
npm test --workspace @future-os/remote-interop
```

The TS command finds the host in root `target/debug` with the platform executable
suffix. `REMOTE_INTEROP_HOST` may supply an explicit path for a CI artifact. No
existing broker, account service or running Agent is used or stopped.

The base common surface covers first pairing and invitation-secret removal,
read/write replies, business refusal, duplicate replay, command-ID conflict,
encrypted events, binary upload/download and IK reconnection/shutdown.

[`isolation.json`](../../packages/remote-client/fixtures/isolation.json) adds five
common report IDs for two real endpoints: duplicate request identity, catalog
ownership despite equal session IDs, event ownership, independent stop and IK
reconnection. Rust uses one Client and two Server supervisors on one broker;
TS uses one Client instance per endpoint with two owned host/broker processes.
The host accepts only a public alphanumeric/underscore fixture label, deriving
separate desktop/pair identities; no credentials are passed in command arguments.
All instances are explicitly stopped and awaited, with owned broker cleanup.
These are core interoperability fixtures; original real Desktop Store/Agent,
multiple application processes and native-file integrations are retained.

The CI Remote job now selects packages by language/dependency and runs both
interop drivers, including these flows. Its new scheduling regression declarations
and interoperability extensions are not yet runtime evidence: no final CI run
has been dispatched during development. Further real network faults, shared
recovery/event/transfer behaviors, application migration audit and matched
performance/manual acceptance remain part of the development plan.

## Migrated Client transport fault cases

[`scripted_requests.rs`](../../tests/remote-interop/tests/support/scripted_requests.rs) is compiled under the same explicit
`network-tests` target. It retains the three original Desktop Client cases for
one lost reply, three exhausted attempts and an unretried write. It uses the
owned real broker and actual PeerSession request/record/reply-context handling;
a scripted encrypted responder deliberately withholds replies. Noise channels
are established in the fixture, not through Server pairing. These cases are
Client transport coverage, distinct from the actual Client/Server flows above.
Each successful case drains the Client and aborts/awaits its responder before
reaping the broker; panic cleanup still aborts the responder and reaps its owned
process. The original retry-whitelist test now belongs to the Client unit suite.
These core cases are removed from Desktop rather than executed again with both
GUI and headless. Their names and all original assertions remain; execution is
pending final CI.

## Held-read transfer shutdown

[`transfer.json`](../../packages/remote-client/fixtures/transfer.json) supplies a
common stop-before-first-chunk case. Both actual Client implementations download
through the actual Server role while its memory FileHost holds the first read.
The role must stop and its download caller must finish before the host read is
released. Afterwards, neither bytes nor a new logical connection may appear.
TS also verifies the local output handle closes, the partial output is removed,
and hashing never runs. Its already-issued concurrent wave differs from Rust's
sequential pulls; no expectation assumes the same number of broker messages.

The fixture host and this Rust test explicitly use two Tokio workers so a
synchronous uncooperative FileHost cannot prevent the control path from releasing
it. The gate holds no credential/store/download-map mutex, is released on Fixture
drop and shutdown, and fails after twenty seconds. Error paths release the gate,
abort/await a pending Rust caller and shut down every owned role/process. These
are memory hosts, not proof of native filesystem/Store/Agent integration. Server
transfer records are checked/cleared on explicit Server shutdown, not by assuming
an already-closed Client can send cancellation. Report output remains IDs and
outcomes; private JSON-lines credentials are never forwarded.

These extensions are statically compiled only until final branch CI acceptance.

## Migrated controller stream regressions

The same Rust target additionally owns the original manually encrypted stream
cases for presence/control state, missing session ID fallback, backlog overflow
(resync once, drop newest), and failed lane replacement. They use the owned real
broker and Client test-support stream API with an explicit empty memory host,
not a Desktop singleton, HOME, application Store or Tauri runtime. Original
assertion sequences remain. The backlog fixture waits for publisher flush then
the subscriber connection's PONG before starting the drain, replacing a timing
sleep with an ordered protocol barrier. Each normal case awaits role shutdown,
drains its connection/publisher and awaits its responder; an owner drop also
cancels the role. These scripted channel tests are distinct from Server-role
pairing/transfer interoperability and retain their original test names.
