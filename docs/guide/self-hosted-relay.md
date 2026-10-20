# Self-hosted relay (account-free remote control)

FutureOS remote control is a NATS relay plus an end-to-end encrypted channel on
top of it. The official relay is `future-os.cn`: a NATS deployment plus a
platform service that mints a pair-scoped NATS user JWT for a signed-in
account. `future relay` is the same relay without the account part, so a
desktop and a phone can pair over a machine you run, with no FutureOS account
anywhere in the path.

The wire protocol is unchanged. Only how a client obtains its NATS credential
differs, which is why the official relay and a self-hosted one can run side by
side and a single desktop can serve both at once.

## What it is (and is not)

| | official relay | `future relay` |
| --- | --- | --- |
| who issues the NATS credential | the platform service, for an account | the relay, for a room |
| account required | yes | no |
| pair id minted by | the platform | the desktop |
| what authenticates the pair | invitation + `Noise XXpsk0` | invitation + `Noise XXpsk0` (identical) |
| records, subjects, commands | `FRE2` + JSON, per-pair subjects | identical |
| revocation | cloud status + short JWT | local unpair + relay room delete |
| storage | Postgres | one JSON file, hashes only |

The relay is **not** a trust anchor. It never sees the Noise static keys, the
invitation PSK, or a single plaintext byte, and it cannot manufacture a key: the
invitation in the QR code is the only thing that binds the two endpoints.

## 1. Create the trust setup

```bash
future relay init --dir /opt/future-relay \
  --listen 127.0.0.1:8099 \
  --public-url https://relay.example.com \
  --nats-url tls://relay.example.com:4222 \
  --nats-ws-url wss://relay.example.com:9090 \
  --tls-cert /etc/letsencrypt/live/relay.example.com/fullchain.pem \
  --tls-key  /etc/letsencrypt/live/relay.example.com/privkey.pem
```

That writes, into `--dir`:

| file | what it is |
| --- | --- |
| `operator.seed` | operator NKey. Keep private. |
| `account.seed` | account NKey. Signs every credential this relay issues. Keep private. |
| `operator.jwt` | the operator JWT nats-server trusts |
| `account.jwt` | the account JWT, preloaded by the resolver |
| `nats.conf` | a ready `nats-server` configuration: operator mode, memory resolver, no JetStream, TLS when a certificate was given |
| `relay.json` | the relay's own settings |
| `rooms.json` | created at the first pairing; the room registry |

`--tls-cert`/`--tls-key` go together. Without them the generated config listens
in plaintext: HTTP, `ws://` and `nats://`. That path is for a relay on a local
network — see below.

Both listeners take their port from `--nats-url`/`--nats-ws-url`, so the
address the relay hands to clients is the address it actually listens on.

### Plaintext or TLS

TLS and Noise do not protect the same thing. The Noise static key and the
invitation PSK in the QR code decide *which desktop* is on the other end, and
that layer is independent of the transport. TLS protects **credentials and
metadata**: the join token, the `refresh_token`, the NATS JWT, and the pair id,
device names and traffic shape.

So the trade is:

| | with `--tls-cert/--tls-key` | without a certificate |
| --- | --- | --- |
| Is content encrypted | yes (Noise) | yes (Noise) |
| Credentials and metadata | protected by TLS | **readable and replayable by anyone on that network** |
| Whose certificate | a CA the client trusts | none needed |
| Phone | must trust that certificate | works as-is |

Plaintext exists because a self-signed certificate on a home LAN cannot be made
trustworthy to the apps: Android ignores user-installed CAs for apps targeting
API 24+, so a private CA either never reaches the trust chain or requires
changing the whole app's network policy. The result of insisting on TLS there
is not safety — it is a relay nobody can pair with. A self-hosted relay may
therefore be plaintext: the phone (`http://` + `ws://`) and the desktop both
follow the origin the invitation declared, and anything else is still refused.

The allowance covers exactly the scheme the relay declared: a relay that
declares `https://` must still answer `wss://`, and the Future platform always
requires `https://` and `wss://`. A public deployment with a public CA is
unaffected.

## 2. Run it

```bash
nats-server -c /opt/future-relay/nats.conf      # or a systemd/docker unit
future relay serve --config /opt/future-relay/relay.json
```

`future relay serve` needs nothing else: no database, no account, no network
access of its own. It listens on `--listen` and answers four routes:

| route | auth | purpose |
| --- | --- | --- |
| `GET  /v1/health` | none | liveness + room count |
| `POST /v1/rooms` | host NKey signature | create/refresh a pairing's room |
| `GET  /v1/rooms/{pair}` | host NKey signature | who is in this room |
| `DELETE /v1/rooms/{pair}` | host NKey signature | delete the room |
| `POST /client/v1/remote/pair/claim` | one-time join token | a phone becomes a device of a room |
| `POST /client/v1/remote/auth/token` | host signature (bridge) or refresh credential (client) | rotate a short-lived NATS JWT |

The `/client/v1/...` paths are deliberately the official platform's, with its
JSON shapes, so the phone and the desktop's client role need no special case:
going through a self-hosted relay changes *where* the request goes, not what it
says.

## 3. Point a desktop at it

```bash
futureos --relay https://relay.example.com           # GUI
future headless --relay https://relay.example.com  # server, no GUI
```

The address applies to **this launch only**. Nothing is written down, so the
next launch without the flag is back on the Future platform — a debugging run
against a self-hosted relay cannot silently become this machine's permanent
choice. `--relay ""` is the same as leaving the flag out. A private CA is added
with `--relay-ca /path/to/ca.pem` (or `FUTURE_RELAY_CA`). A container or a test
can also set `FUTURE_RELAY_URL`, which wins over the flag (an empty value means
the official relay).

In relay mode:

* no FutureOS login is required for remote control, and `future headless`
  skips the login gate entirely;
* the desktop mints its own pair id, Noise identity, invitation PSK and join
  token;
* the invitation is a `futureos://remote/pair?v=3` link. Its payload keeps the
  official v2 shape (so the claim path is the same) and adds `relay`, the
  origin the claim endpoint belongs to;
* the QR still contains `desktopKey`, `secureKey` and `secret`. The relay
  receives only the *hash* of the join token.

Devices paired through the official platform keep working: the choice is
recorded **per pairing** in `remote_pairings.json` (`controlUrl`), so one
desktop can hold an official device and a self-hosted one at the same time,
each talking to the relay that issued it.

Managing those pairings from a terminal is the same command in both modes:

```bash
future remote devices                  # each device's endpoint is named
future remote devices add              # mints a v3 relay invitation here
future remote devices remove <id|#>    # deletes that pairing's room, signed by its key
future remote devices remove --all     # every device, each at its own issuer
```

No FutureOS login is involved: the relay has no accounts, and `add` only asks the
running Desktop. A relay behind a private CA needs no flag for these commands —
name it with `FUTURE_RELAY_CA` and it is applied to that relay's requests.
Nothing is remembered between runs: a self-hosted relay is chosen per launch
with `--relay`, and a launch without it uses the Future platform.

## 4. Run the official relay and a self-hosted one together

Nothing special is required. A machine can hold several pairings, each with its
own relay, its own NATS connection, heartbeat and credential rotation.

* Dev builds point at `test.future-os.cn` by default; the environment switch in
  the settings page does not affect a pairing that has a `controlUrl`.
* A phone can pair with several desktops, including a mix of official and
  self-hosted ones; the picker names each machine, not its relay.
* Losing a relay affects only its own pairings: the others keep serving.

## 5. What the relay can and cannot do

Content is protected by `Noise XXpsk0`/`IK` and the `FRE2` record layer, both
unchanged. A malicious relay operator gets:

* **no plaintext**, no ability to forge a command, reply, file chunk or
  presence/catalog event (AEAD with the NATS subject as additional data, plus
  the reply context binding a reply to its request);
* **no ability to substitute a key** — the desktop's Noise static key comes from
  the QR, and a client that asks the relay for one is not following the
  protocol;
* **metadata**: pair id, device public keys, device names, subject names,
  message sizes and timing, and the traffic pattern. TLS hides it from the
  network but not from the relay;
* **availability**: it can drop, delay, reorder or refuse. NATS Core is
  at-most-once by design; recovery is the clients' reconnect and replay, never
  the relay's promise.

What it cannot do, because of how the rooms and credentials are scoped:

* read or write another pairing's subjects. Every credential carries an allow
  list for exactly one `p.<pair>.>` namespace, and the broker enforces it —
  `SUB >`, a cross-pair subscription and a queue-group grab are all refused;
* publish as the bridge from a client connection, or read a client's reply
  inbox;
* reuse a join token for a second device. It is single-use per device (the same
  device may retry a lost response), expires after five minutes, and the relay
  keeps only its SHA-256.

Residual risks you should decide about before exposing one publicly:

| risk | mitigation |
| --- | --- |
| the relay sees pair ids, device names and traffic shape | run your own; keep the relay off shared infrastructure |
| the relay can refuse service | that is true of any relay; it is not a confidentiality issue |
| a QR code is a bearer credential | five-minute expiry, one use, and the desktop deletes the PSK the moment pairing succeeds |
| anyone who can reach the relay can open rooms | rate-limit at the network edge and treat a public relay as a shared resource |
| losing a phone's credential | unpair on the desktop: it deletes the room, and a self-hosted relay has no cloud revocation to rely on |

## 6. Operating notes

* **No JetStream.** Remote control is NATS Core only, and a store would be a
  place for ciphertext and metadata to accumulate on a machine that is not
  supposed to keep either.
* **The room file is not a secret store.** `rooms.json` holds pair ids, public
  NKeys, device names, timestamps and two SHA-256 hashes per device. A stolen
  copy cannot be used to connect: every credential is bound to a public key
  whose seed never leaves the endpoint. Deleting the file invalidates every
  pairing at once, which is a valid "start over" and the reason a corrupt file
  is an error rather than an empty registry.
* **Restarts are free.** The desktop re-registers its own room on the next
  credential rotation (signed with its pairing NKey), and its already-paired
  phone reconnects. The pairing lives in the endpoints' credentials, not in the
  relay.
* **Credentials are short-lived.** The default JWT lifetime is five minutes and
  rotation is a signed request; there is no long-lived bearer token at the
  relay.
* **One invitation is one device**, the same rule the official platform applies
  per pairing. "Add device" on the desktop creates another pairing (its own pair
  id, its own room) rather than a second phone in one room.
* **TLS is a deployment decision.** A public relay takes a public CA (or
  `--relay-ca` with your own); a plaintext relay on a local network is accepted
  only on the origin it declared, and never loosens the official platform or an
  `https://` relay.

## 7. Verifying a deployment

Broker-level (needs a `nats-server` binary):

```bash
FUTURE_RELAY_NATS_SERVER=/usr/local/bin/nats-server \
  cargo test -p future-cli --test relay_nats_auth -- --ignored --nocapture
```

It starts a real broker in operator mode with the generated JWTs, runs the
relay's HTTP API in-process, and asserts the allow lists by *delivery*: a
forbidden publish or subscription must arrive nowhere, including `SUB >`, a
cross-pair subscription, a reply-inbox read and a queue-group grab.

End to end (needs a live relay):

```bash
future relay init --dir /tmp/relay --nats-url nats://127.0.0.1:14222
nats-server -c /tmp/relay/nats.conf
future relay serve --config /tmp/relay/relay.json

RELAY_E2E_RELAY_URL=http://127.0.0.1:8099 \
  cargo test -p futureos --lib remote::relay_e2e -- --ignored --nocapture
```

That test is the real thing: `remote::start()` mints a v3 invitation through the
relay, the client role claims it and connects with `Noise XXpsk0`, presence and
`list_sessions` cross the encrypted channel, the relay's room roster matches
what happened, and unpairing deletes the room.
