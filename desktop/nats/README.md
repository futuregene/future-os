# NATS Relay

The relay is already reachable on the public network:

- dev/test: `test.future-os.cn` (`4222` client, `9090` WebSocket)
- production: `future-os.cn` (`4222` client, `9090` WebSocket)

Current desktop/mobile/web remote control uses short-lived, pair-scoped NATS
user JWTs. Relays must run in operator/account JWT mode; the old shared token is
not a multi-tenant security boundary.

Mobile requires `wss://` in both production and test environments. The ports above
are deployment listener details, not instructions to connect mobile over plaintext
`ws://`. The desktop connects directly to the configured NATS endpoint; TLS on
that hop is deployment-controlled, not unconditionally enforced by the client.
Secure that path explicitly. Do not describe all hops as encrypted or recommend
plaintext WebSocket URLs to current mobile clients. Only use test data in test
relay deployments. User pairing instructions: [Remote](../../docs/wiki/en/Remote.md).

The canonical production template and operator runbook live in
`../future-server`:

- `config/nats-jwt.conf.example`
- `docs/remote-control-deployment.md`

Changing the existing test relay from its old shared-token configuration
requires an operator-run NATS container recreation and matching
`platform-service` deployment. Old JetStream data does not need to be retained
for this test cutover.

## Legacy local environment

`nats.conf` and `docker-compose.yml` in this directory remain only for isolated
testing of older shared-token clients:

```bash
cd desktop/nats
docker compose up -d
docker compose logs -f nats
docker compose down
```

Never expose this legacy local configuration to the public network. It is not a
valid environment for testing JWT subject isolation.
