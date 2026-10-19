# Shared Client fixtures

The files under `packages/remote-client/fixtures/` are authoritative inputs and expectations shared by Rust and
TypeScript. They are not copied into Desktop or Mobile.

| File | Purpose | Drivers |
|---|---|---|
| [client.json](../../packages/remote-client/fixtures/client.json) | Real role interoperability: requests/deduplication, pairing, events, file bytes, reconnect and shutdown report IDs | [Independent interoperability project](remote-interop.md) |
| [isolation.json](../../packages/remote-client/fixtures/isolation.json) | Two real endpoints: request/deduplication, catalog/event ownership, stop and reconnect independence | Independent Rust and TS interoperability drivers |
| [credentials.json](../../packages/remote-client/fixtures/credentials.json) | Expiry boundaries, late renewal fencing, continuation and instance isolation | Rust `shared_credentials_tests.rs`; Node/Jest `sharedCredentials.test.ts` |
| [transfer.json](../../packages/remote-client/fixtures/transfer.json) | Stop before a held real Server download read returns; no byte delivery or replacement connection | Independent Rust and TS interoperability drivers |
| [recovery.json](../../packages/remote-client/fixtures/recovery.json) | Actual bounded retry loop, shutdown before/after retries and no late restart | Rust `shared_recovery_tests.rs`; Node/Jest `sharedRecovery.test.ts` |
| [requests.json](../../packages/remote-client/fixtures/requests.json) | Controlled request-policy scenarios: send times, outcomes, retry identity, timeout budgets, recovery/probe counts and cancellation | Rust `shared_request_tests.rs`; Node/Jest `sharedRequests.test.ts` |

Each format has an explicit version and suite/kind checks. Unknown fields,
unsupported cases and duplicate IDs fail. Every driver executes and reports all
IDs in its selected suite; selecting a different suite is not silently skipping
an unsupported case. Expected JSON compares by value, not object-key ordering.

`requests.json` is driven by the production Rust serialization/retry functions
and runtime stop gate. Its exchange supplies only scripted transport outcomes.
The TS driver calls actual `RemoteClient.requestRetry`, request encoding,
response decoding, recovery and shutdown with a memory SDK connection and real
record encryption. It directly installs an established channel; it does not
claim to test Noise pairing. Pairing and actual broker transport belong to the
interoperability project and existing package crypto/secure-client tests.

Both drivers use virtual time. `timeout` completes after the implementation's
actual ordinary request budget; the fixture separately asserts that budget.
Rust permits at most 10 ms accumulated slack for Tokio's millisecond timer
rounding; TS expects exact fake-clock times. Neither relies on real wall-clock
sleeps. `recoveries` counts recovery entry calls; `probes` counts actual probe IO.
A stopped TS role may receive a recovery request while performing no probe.

Language-specific expectations retain existing behavior: TS probes a healthy
connection before a retry and requires `prompt_receipt_v1` for prompt/continue;
the historical Rust session whitelist retries those commands directly. Rust
runtime cancellation and a non-retried TS SDK write may also expose different
error categories. Any unequal expectation requires an explicit `difference`
note. These cases are executed by both drivers, not removed to obtain a smaller
common surface.

Each successful case reports only its language and stable ID. The data contains
public test commands; no credential, invitation secret, key or record bytes are
printed in reports. A suite's case count is distinct from its framework's test
declaration count.

Development checks compile/typecheck/lint the drivers only. Under the current
migration workflow, execution happens during final CI acceptance:

```text
cargo test -p future-remote-client -- --nocapture
npm test --workspace @future-os/remote-client
```

`credentials.json` separates metadata policy from authentication: Rust receives
stored expiry metadata and TS parses JWT metadata; neither decision validates a
signature. The exact 60-second boundary and nonpositive/malformed metadata keep
explicit language expectations. Healthy TS metadata retains object identity;
invalid metadata must not call the refresh service.

Renewal actions map to existing role boundaries: Rust's `renew_credentials`
with `begin_attempt`/`disconnect`/`shutdown`, and TS's `refreshToken` with
`cancelAttempt`/background retirement/`shutdown`. `continuations` counts a fresh
Rust grant returned to the connector, or TS's call to its `open` continuation;
it does not assert that a network connection was established. The continuation
is observed without running another handshake. Actual broker reconnects are a
separate interoperability gate.

Both renewal drivers observe committed grants, writes, continuations and which
instance closed. They preserve a grant committed before close, reject a result
from a closed/retired/superseded attempt, and leave the other instance independent.
Rust additionally observes actual service-future destruction. TS deliberately
uses a host promise that ignores cancellation and checks ownership remains until
it settles; no late credential cache or persistent-store adoption is allowed.
These instance checks do not replace multiple endpoints inside one Rust runtime,
application SecureStore transactions or live recovery fault coverage.

`isolation.json` supplies two distinct endpoint labels/messages, the same request
ID on both endpoints and five stable report IDs. The Rust driver connects one
Client runtime to two Server supervisors on the same owned broker. The TS driver
connects two Client instances to separate owned host/broker processes. Both use
real handshakes, records, routing, replies and events; neither loads an app.
Both keep the same remote session ID on each endpoint so catalog ownership cannot
be inferred from a conveniently unique session ID. Rust also checks the public
catalog's desktop/pair stamps. TS checks the corresponding role's returned catalog.
Stopping one endpoint leaves the other answering; the stopped endpoint reconnects
with its persisted IK identity without changing the survivor. These topologies
are deliberately stated and do not claim cross-process application integration,
relay JWT/ACL enforcement, automatic failure recovery or shared timeline dedup.

`recovery.json` drives the actual Rust supervisor through resume/connect until
an explicit host load fails, before any socket IO. Its production attempt count
is retained; existing test builds scale backoff units to milliseconds. TS drives
actual `open`/retry/deadline/shutdown, with a rejecting transport factory and the
explicit precondition of a previously ready role. It does not stub the scheduler.
TS uses midpoint jitter and the existing three-minute budget; Rust exhausts its
18-attempt task without making the entire Client terminal. These differences are
recorded in the shared material instead of changing either product policy.
Both clocks are controlled, and both drivers observe a further ten minutes
without new attempts after exhaustion/stop. This is loop ownership/budget
coverage, not evidence of successful pairing, live reconnection or event replay.

`transfer.json` stops a real role while the first Server `FileHost` download read
is held. The test-only memory host models an uncooperative synchronous read,
using two runtime workers and a bounded condition-variable gate. No application
filesystem or Agent is involved. Both Client callers must finish cancellation
before release, then observe the late read without byte delivery or a replacement
connection. Rust observes bytes returned to its caller; TS additionally records
every write to its fake local sink, close/removal and hash calls. A TS concurrent
pull wave may already be queued; it is not counted as a post-stop retry.
Server records are released by explicit Server shutdown, not assumed to be
cancelled over a closed connection. Owner drop/shutdown always releases the gate,
and the gate has a failure timeout; no test should leave a blocked worker behind.
The interop-only `client.json`, `isolation.json` and `transfer.json` inputs now
select only their actual interop consumers in affected CI. Unit-driver materials
continue selecting both owning language packages.
