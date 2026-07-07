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

## Create dev event stream (pairId=DEVPAIR)
```bash
nats stream add EVT_DEVPAIR \
  --subjects 'p.DEVPAIR.evt.>' \
  --storage file --retention limits --discard old \
  --max-age 30m --max-bytes 64MB --max-msg-size 1MB --dupe-window 10m \
  --defaults
nats stream ls                     # should show EVT_DEVPAIR
nats stream info EVT_DEVPAIR
```

## Quick manual pub/sub test (optional, verify the pipeline)
```bash
# Terminal A:
nats sub 'p.DEVPAIR.evt.>'
# Terminal B:
nats pub p.DEVPAIR.evt.s1 'hello'
# Terminal A should receive 'hello'
```

## Stop / Cleanup
```bash
docker compose down          # stop containers (keep JetStream data volume)
docker compose down -v       # stop containers + delete data volume (full reset)
```
