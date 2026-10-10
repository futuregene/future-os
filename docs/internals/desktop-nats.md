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
a plaintext relay listener cannot serve the desktop. Only unit tests connect to an
in-process plaintext fake broker; there is no runtime switch that permits a
production plaintext downgrade. Do not describe all hops as encrypted or recommend
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
