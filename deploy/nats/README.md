# Local L0 dev NATS (remote-control relay testing)

> L0 = no auth, **local/trusted-network only — do not expose to the public internet**. See `docs/remote-control-*.md`.

## Prerequisites
- Docker (Docker Desktop or colima): `docker version` should work.
- `nats` CLI (for stream creation and validation):
  ```bash
  brew install nats-io/nats-tools/nats
  # Verify: nats --version
  ```

## Start NATS
```bash
cd deploy/nats
docker compose up -d
docker compose logs -f nats   # Wait for "Server is ready" + JetStream enabled, then Ctrl-C
```

## Verify connectivity + JetStream
```bash
nats server check jetstream        # defaults to nats://localhost:4222
# Or open http://localhost:8222 in a browser (monitoring page)
```

## Simple-pairing access token (Phase 1)
`nats.conf` enables a **shared access token** (`devpairingtoken` by default) on both
the client port (4222) and the WebSocket (9090). This is the Phase 1 *simple
pairing* admission gate: the desktop (paired mode) and web clients must present it
to connect. The random `pairId` partitions subjects; the token gates admission.

> It is a **global** token, not per-pair — server-enforced per-subject isolation
> needs Phase 2 JWTs. See `gui/DEV_MD/remote-control-auth.md` §8.9 / §9.

- Change the token in `nats.conf` (keep the `websocket.authorization` block **in sync**).
- For the original **no-auth** dev mode (desktop `dev` mode, hand-typed pairId, no
  token), comment out **both** `authorization` blocks in `nats.conf`. Use only on
  localhost — admission control is then OFF.

Connect with the token (CLI verification):
```bash
nats --token devpairingtoken server check jetstream
nats --token devpairingtoken sub 'p.DEVPAIR.evt.>'
```

## Create dev event stream (fixed pairId, e.g. DEVPAIR)
> For a **random** paired `pairId` you do NOT create the stream by hand — the
> Bridge creates `EVT_{pairId}` itself once JetStream replay is enabled
> (Phase 1 step 1.8). Manual creation below is only for a fixed dev pairId.
```bash
nats --token devpairingtoken stream add EVT_DEVPAIR \
  --subjects 'p.DEVPAIR.evt.>' \
  --storage file --retention limits --discard old \
  --max-age 30m --max-bytes 64MB --max-msg-size 1MB --dupe-window 10m \
  --defaults
nats --token devpairingtoken stream ls                     # should show EVT_DEVPAIR
nats --token devpairingtoken stream info EVT_DEVPAIR
```

## Quick manual pub/sub test (optional, verify the pipeline)
```bash
# Terminal A:
nats --token devpairingtoken sub 'p.DEVPAIR.evt.>'
# Terminal B:
nats --token devpairingtoken pub p.DEVPAIR.evt.s1 'hello'
# Terminal A should receive 'hello'
```

## Stop / Cleanup
```bash
docker compose down          # stop containers (keep JetStream data volume)
docker compose down -v       # stop containers + delete data volume (full reset)
```
