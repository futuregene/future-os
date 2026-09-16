# Remote v2 end-to-end channel

## Security boundary

Mobile and Desktop authenticate each other and encrypt application records before
publishing to NATS. A malicious broker can drop, delay, reorder, replay, route,
and fabricate messages, but must not learn application plaintext or manufacture
an accepted command, reply, file chunk, or presence/catalog event.

This does not protect a compromised endpoint, a stolen/unlocked credential store,
a leaked **unused** invitation, traffic timing/size, NATS subject metadata, or
availability. Cloud revocation is not an instantaneous endpoint revocation signal
when an attacker can suppress delivery. Local disable/unpair invalidates access
and clears traffic keys/candidates immediately. NATS JWT ACLs remain defense in
depth, not proof of message origin. Production Desktop requires verified TLS;
Mobile requires WSS. The in-process Rust test broker deliberately uses plaintext.

## Pairing and migration

Desktop generates an X25519 identity and an independent random 32-byte invitation
PSK locally. Neither private key nor PSK is sent to the platform API. The existing
platform claim nonce is **not** this PSK: the claim nonce still goes to the platform.

The same complete `futureos://remote/pair` invitation is used for QR scanning and
copy/paste. It contains `v=2`, the existing platform `code`, `desktopId`, the existing
NATS `desktopKey`, the X25519 `secureKey`, and the local `secret`. Treat the whole
invitation as a short-lived bearer credential. No 6-digit-code mode is added.
Do not log invitations, send them to analytics, or render QR codes with remote APIs.

Mobile generates its own X25519 identity and stores the bundle in Expo SecureStore
with `WHEN_UNLOCKED_THIS_DEVICE_ONLY`, using the existing two-slot credential
commit. Desktop stores the pairing identity with its existing owner-restricted,
atomic pairing credential file; this is not a hardware-keystore guarantee.

Initial pairing uses `Noise_XXpsk0_25519_ChaChaPoly_BLAKE2b`. Mobile checks the remote
static key against the QR/pasted key before sending the final handshake message.
Desktop verifies possession of the invitation PSK, binds the remote static key,
persists the binding, and removes the PSK from the active credential. The invitation
is then rejected even if someone copies it. This is authentication by possession
of a trusted invitation, not verification of a human/legal identity.

Subsequent handshakes use `Noise_IK_25519_ChaChaPoly_BLAKE2b`; both peers require the
bound remote identity. A lost first-pair confirmation can recover with IK using
the same phone identity, without accepting a server-supplied replacement key.
Every handshake uses fresh ephemeral keys. Traffic keys/counters are never saved.

Old Desktop pairings are replaced with new invitations. Old mobile credential
bundles cannot enter the v1 business channel. Existing users must pair again after
upgrading both endpoints. There is no production plaintext fallback. The old
`desktop/web` verification client is not a v2 client and cannot control this bridge.
Legacy Rust protocol/business fixtures are test-only, not a compatibility mode.

## Handshake ownership and readiness

The raw request/reply exchange uses only `secure_open` and `secure_finish` on the
existing pair command subjects. Raw input is capped at 16 KiB; Noise messages are
capped at 8 KiB. Pending handshakes/candidates are bounded and expire after 30s.
Only authenticated Noise output can produce a candidate traffic channel.

Desktop returns an encrypted `handshake-confirm` record with presence, bridge
identity and capabilities. This alone does not replace a serving channel. After
Mobile installs subscriptions and flushes them, it sends an encrypted
`secure_ready` command. Desktop commits that candidate as current and returns an
encrypted acknowledgement. A failed candidate does not deactivate the previous
channel, and a stopped access epoch cannot install a late candidate. Uncommitted
candidates cannot run ordinary commands or upload chunks.

JWT renewal and NATS socket replacement reuse the pinned identity, never reset a
traffic-key counter. Background/restart recovery creates a new handshake. A stale
heartbeat receipt triggers an authenticated probe/reconnect; no unsigned discovery
beacon is trusted to recover a restarted Desktop.

## Binary application records

The record layer uses ChaCha20-Poly1305 with the two directional keys from Noise
Split. Rust uses `snow` and `chacha20poly1305`; Mobile uses `noise-handshake` with
the pure-JS sodium backend and `@noble/ciphers` for records. The standalone record
layer supports NATS message sizes beyond Noise's 65,535-byte transport-message
limit. Do not use the JS handshake library's transport cipher for these records.

| Offset | Bytes | Field |
| --- | ---: | --- |
| 0 | 4 | ASCII `FRE2` |
| 4 | 16 | First 16 bytes of the Noise handshake hash |
| 20 | 8 | Little-endian sequence number |
| 28 | variable | Ciphertext followed by the 16-byte AEAD tag |

Nonce = four zero bytes followed by the 8-byte little-endian sequence. Each
sending direction has an independent key and monotonic counter. A new handshake
is mandatory before sequence `2^32-1`. The wire limit is 1 MiB, leaving 1 MiB - 44
bytes for plaintext. Requests and all unsolicited records authenticate the exact
NATS subject as additional data after the fixed header.

Reply context is `reply:<request subject>:<hex of request header bytes 4..28>`.
It binds the reply to the authenticated request channel and sequence, not the
broker-provided inbox. Substituting reply inboxes must not substitute responses.
Cached business replies remain plaintext **inside the endpoint** and are encrypted
again for each authenticated retry's reply context. Business operation IDs remain
stable across retries; record sequence numbers do not.

Receivers maintain a bounded 4096-record replay window per channel/direction.
Authentication succeeds before the window is mutated. Reflected, wrong-subject,
wrong-connection, tampered, duplicate and out-of-window records are rejected.
Receive authentication precedes decompression, JSON decoding and business dedup.
Compression, when enabled by the existing policy, happens before encryption.

All command/reply, file upload/download, presence, catalog, event and disconnect
paths use this boundary. Pairing controls are the only raw exception. NATS subjects
remain visible, so the protocol does not claim metadata confidentiality.

## Validation and limits

`packages/remote-crypto/tests/vectors.json` is checked by both Rust/snow and Mobile
using deterministic, public test keys, covering XXpsk0, IK, handshake hashes, record
bytes, AAD, and reply association. `generate-vectors.cjs` prints the JS vectors;
production has no fixed-ephemeral option exposed by our wrapper.

Tests also exercise bytewise tampering, wrong PSK/prologue/key, replay/reflection,
large records, lost confirmation, pinned-key reconnect, stale credential refresh,
failed persistence, invitation reuse/expiry, readiness handoff, local invalidation,
and an encrypted request through the actual Desktop NATS command loop. Existing
business/lifecycle fixtures explicitly mock their authenticated transport; those
fixtures alone are not evidence of E2EE.

The fixed record overhead is 44 bytes, with no Base64 expansion for business/file
traffic. Handshake fields are small Base64url strings. Performance and compatibility
must be measured on actual Android/iOS devices before making handset latency,
battery or hardware-backed-key claims. Metro/Hermes export and desktop Node tests
are useful checks but are not physical-device runtime or penetration testing.
