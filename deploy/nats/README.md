# NATS Relay

The relay is already reachable on the public network:

- dev/test: `test.future-os.cn` (`4222` client, `9090` WebSocket)
- production: `future-os.cn` (`4222` client, `9090` WebSocket)

Current GUI/Web remote control uses short-lived, pair-scoped NATS user JWTs.
The test relay must run in operator/account JWT mode. During flow validation it
intentionally uses plaintext `nats://` and `ws://`; only test data is allowed.
The old shared token is not a multi-tenant security boundary.

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
cd deploy/nats
docker compose up -d
docker compose logs -f nats
docker compose down
```

Never expose this legacy local configuration to the public network. It is not a
valid environment for testing JWT subject isolation.
