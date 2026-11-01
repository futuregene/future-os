# NATS Relay

The relay is already reachable on the public network:

- dev/test: `test.future-os.cn` (`4222` client, `9090` WebSocket)
- production: `future-os.cn` (`4222` client, `9090` WebSocket)

Current remote control — desktop, mobile, and the test-platform-only web client —
uses short-lived, pair-scoped NATS user JWTs. Relays must run in operator/account
JWT mode; the old shared token is not a multi-tenant security boundary.

Mobile requires `wss://` in both production and test environments. The ports above
are deployment listener details, not instructions to connect mobile over plaintext
`ws://`. The desktop connects directly to the server-issued NATS endpoint, and its
client requires verified TLS even when that endpoint uses the `nats://` scheme, so
a plaintext listener cannot serve a **platform** pairing. The one exception is a
pairing a self-hosted relay issued whose endpoint it declared as `nats://`:
`connect_nats` waives required TLS for exactly that shape (see
[self-hosted-relay.md](../guide/self-hosted-relay.md)). The Future platform and
any `tls://` endpoint keep verified TLS however they were issued. Unit tests also
connect to an in-process plaintext fake broker; that fixture path is separate
from the relay exception. The desktop takes its trust anchors from a PEM
snapshot of the platform store it writes itself (`~/.future/app/
relay-root-certs.pem`) instead of letting the NATS client read the platform
store on every connection — that read is what fails intermittently on macOS,
and a failed read used to fail the connection. The snapshot is the platform
store, so it changes no trust decision; deleting it costs nothing.
Do not describe all hops as encrypted or recommend
plaintext WebSocket URLs to current mobile clients. Only use test data in test
relay deployments. User pairing instructions: [Remote](../wiki/en/Remote.md).

The canonical production template and operator runbook live in
`../future-server`:

- `config/nats-jwt.conf.example`
- `docs/remote-control-deployment.md`

Changing the existing test relay from its old shared-token configuration
requires an operator-run NATS container recreation and matching
`platform-service` deployment. Old JetStream data does not need to be retained
for this test cutover.

## Self-hosted alternative

`future relay` (this repository) is the same relay without the platform: it
mints the same pair-scoped NATS user JWTs, from the same claim/token endpoints,
for rooms instead of accounts. A desktop selects it per launch with
`futureos --relay URL` (or `future host --relay URL`); nothing is
persisted, so a launch without the flag uses the Future platform. The choice is
recorded per pairing, so official and self-hosted pairings coexist on one
machine. Runbook, threat model and verification:
[../guide/self-hosted-relay.md](../guide/self-hosted-relay.md).

The broker configuration `future relay init` writes is the one this document's
production template describes, minus the account layer: operator JWT, memory
resolver, no JetStream.

## Legacy local environment

`desktop/nats/nats.conf` and `desktop/nats/docker-compose.yml` remain only for
isolated testing of older shared-token clients:

```bash
cd desktop/nats
docker compose up -d
docker compose logs -f nats
docker compose down
```

Never expose this legacy local configuration to the public network. It is not a
valid environment for testing JWT subject isolation.

## Credential lifetime and renewal

Every pairing's NATS user JWT is issued for 15 minutes (`exp - iat = 900` in the
token the platform mints). A socket is authorized by the JWT it was *opened*
with, so a generation that reaches its expiry is closed by the relay and cannot
authenticate again with the same grant — renewal has to happen before that, on
both halves of the pair:

- the bridge (`future_remote_server::pairing::REFRESH_LEAD`): **240 s**;
- the client that drives another machine (`ROTATION_LEAD_SECS` in
  `future-remote-client`): **240 s**, and a rotation that fails is retried inside
  that window with a 15/30/60 s backoff instead of being written off for the rest
  of the grant.

The client's *connect-time* predicate stays at 60 s (`PeerCreds::needs_refresh`,
pinned by `fixtures/credentials.json` because the phone client shares it): that
margin answers "will a connection started now still be valid", which is a
different question from "how much room does a live link need to replace its
grant".

Measured 2026-11-01 before the change: three outgoing links each lost their grant
at every expiry and were re-minted 2-3 s *after* the relay had cut them
(10:50:44 → 10:50:47, 10:51:45 → 10:51:48, 10:53:35 → 10:53:38) — a blink every
15 minutes, with `User Authentication Expired` / `authorization violation` /
`max reconnection attempts reached` in `desktop.log` each time. After the change
a launch's renewals can be read off `remote_peers.json`: each `iat` lands about
240 s before the previous `exp`, and no socket is cut.
