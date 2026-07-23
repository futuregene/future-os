# L0 NATS Relay configuration

The remote-control Relay is already deployed on the public network. GUI builds
derive its address from the active Future platform environment:

- dev/test: `test.future-os.cn` (`4222` client, `9090` WebSocket)
- production platform: `future-os.cn` (`4222` client, `9090` WebSocket)

This directory is the reproducible Docker/configuration source for that Relay
and can also be used to start an isolated local instance for infrastructure
work. Normal GUI/Web remote-control testing does not require starting it locally.

> Phase 1 uses a shared access token. Public reachability does not make it a
> secure hostile multi-tenant boundary: server-enforced per-pair subject
> isolation still requires Phase 2 scoped JWTs.

## Simple-pairing access token (Phase 1)
`nats.conf` enables a **shared access token** on both
the client port (4222) and the WebSocket (9090). This is the Phase 1 *simple
pairing* admission gate: the desktop (paired mode) and web clients must present it
to connect. The random `pairId` partitions subjects; the token gates admission.

> It is a **global** token, not per-pair — server-enforced per-subject isolation
> needs Phase 2 JWTs. See `gui/DEV_MD/remote-control-auth.md` §8.9 / §9.

- Set the deployment token in `nats.conf` and keep the global and
  `websocket.authorization` values in sync.
- The GUI and Web client intentionally have no no-auth connection mode.

CLI verification, using the deployed host and token:
```bash
nats --server nats://test.future-os.cn:4222 --token '<relay-token>' server check jetstream
```

## Local infrastructure reproduction

```bash
cd deploy/nats
docker compose up -d
docker compose logs -f nats
docker compose down          # stop containers (keep JetStream data volume)
docker compose down -v       # stop containers + delete data volume (full reset)
```
