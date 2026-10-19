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

The sole shared input/expectation file is
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

The current common surface covers first pairing and invitation-secret removal,
read/write replies, business refusal, duplicate replay, command-ID conflict,
encrypted events, binary upload/download and IK reconnection/shutdown. Shared
unit drivers with controlled time, lost-response/recovery/renewal/cancellation,
multiple endpoint isolation, further security faults, affected CI wiring and
runtime/performance acceptance remain part of the development plan.
