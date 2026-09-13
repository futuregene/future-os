import Noise from "noise-handshake";
import type { Msg, NatsConnection } from "@nats-io/nats-core";
import { RemoteClient, type RemoteClientCallbacks } from "../client";
import { createSecureIdentity, keyBytes, replyContext, SecureChannel, securePrologue } from "../secureChannel";
import { decodeBase64Url, encodeBase64Url } from "../codec";
import type { RemoteCredentials } from "../types";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const encode = (value: unknown) => encoder.encode(JSON.stringify(value));

// Real Noise + real AEAD, with only NATS delivery replaced. Rust independently
// verifies these algorithms/vectors and its actual NATS command boundary.
function fixture() {
  const desktop = createSecureIdentity();
  const mobile = createSecureIdentity();
  const secret = encodeBase64Url(new Uint8Array(32).fill(19));
  const creds: RemoteCredentials = { pairId: "pair_1", deviceId: "dev_1", seed: "unused", userJwt: "unused",
    refreshToken: "unused", natsWsUrl: "wss://example.test", tokenUrl: "https://example.test/token",
    expectedDesktopId: "desktop_1", expectedDesktopPublicKey: "UNKEY",
    secureBundle: JSON.stringify({ identity: mobile, desktopKey: desktop.publicKey, secret }) };
  const callbacks: RemoteClientCallbacks = {
    onCredentials: jest.fn(async () => {}), onEvent: jest.fn(), onEventDecodeFailure: jest.fn(), onPresence: jest.fn(),
    onSessions: jest.fn(), onWorkspaces: jest.fn(), onFeatures: jest.fn(), onConnectionState: jest.fn(),
    onReconnected: jest.fn(), onError: jest.fn(),
  };
  const client = new RemoteClient(creds, callbacks);
  let pending: Noise | undefined;
  let serverChannel: SecureChannel | undefined;
  let pinned: string | undefined;
  let loseConfirmation = false;
  let corruptReply = false;
  const commands: unknown[] = [];
  const wires: Uint8Array[] = [];
  const confirmation = () => {
    if (!pending?.complete || !pending.hash || !pending.tx || !pending.rx) throw new Error("incomplete");
    serverChannel = new SecureChannel(pending.tx, pending.rx, pending.hash.subarray(0, 16));
    pinned = encodeBase64Url(pending.rs!);
    return encodeBase64Url(serverChannel.seal("handshake-confirm", encode({ confirmed: true, pairId: "pair_1",
      bridgeInstanceId: "bridge_1", presence: { bridgeInstanceId: "bridge_1", online: true }, features: ["e2ee_v2"] })));
  };
  const request = jest.fn(async (subject: string, data: Uint8Array) => {
    wires.push(data.slice());
    if (decoder.decode(data.subarray(0, 4)) === "FRE2") {
      const plain = serverChannel!.open(subject, data);
      commands.push(subject.includes(".cmd.") ? JSON.parse(decoder.decode(plain)) : plain);
      const body = encode({ success: true, data: { ok: true } });
      const reply = serverChannel!.seal(replyContext(subject, data), body);
      if (corruptReply) reply[reply.length - 1] = reply[reply.length - 1]! ^ 1;
      return { subject: "untrusted-inbox", data: reply } as Msg;
    }
    const body = JSON.parse(decoder.decode(data));
    if (body.type === "secure_open") {
      if (body.pairing && pinned) return { data: encode({ success: false }) } as Msg;
      pending = new Noise(body.pairing ? "XXpsk0" : "IK", false,
        { secretKey: keyBytes(desktop.privateKey), publicKey: keyBytes(desktop.publicKey) },
        body.pairing ? { psk: keyBytes(secret) } : undefined);
      pending.initialise(securePrologue("pair_1", "desktop_1"));
      pending.recv(decodeBase64Url(body.message)!);
      if (!body.pairing && (!pinned || encodeBase64Url(pending.rs!) !== pinned)) throw new Error("wrong peer");
      const message = encodeBase64Url(pending.send());
      return { data: encode({ success: true, data: { id: "challenge", message,
        ...(!body.pairing ? { confirmation: confirmation() } : {}) } }) } as Msg;
    }
    if (body.type === "secure_finish") {
      pending!.recv(decodeBase64Url(body.message)!);
      const confirmed = confirmation();
      if (loseConfirmation) { loseConfirmation = false; throw new Error("lost response"); }
      return { data: encode({ success: true, data: { confirmation: confirmed } }) } as Msg;
    }
    throw new Error("plaintext business command");
  });
  const connection = { request, close: async () => {} } as unknown as NatsConnection;
  const internal = client as unknown as {
    connection: NatsConnection; credentials: RemoteCredentials;
    performHandshake(connection: NatsConnection): Promise<unknown>;
    secureChannels: WeakMap<NatsConnection, SecureChannel>;
  };
  internal.connection = connection;
  return { client, internal, connection, request, commands, wires, callbacks, creds, desktop,
    pair: () => internal.performHandshake(connection),
    loseConfirmation: () => { loseConfirmation = true; },
    corruptReply: () => { corruptReply = true; },
    serverChannel: () => serverChannel!,
  };
}

test("pairs, persists the bound identity without the invitation secret, encrypts commands and file chunks", async () => {
  const f = fixture();
  await f.pair();
  expect(JSON.parse(f.internal.credentials.secureBundle!).secret).toBeUndefined();
  expect(f.callbacks.onCredentials).toHaveBeenCalledTimes(1);
  await expect(f.client.request({ type: "list_sessions" })).resolves.toMatchObject({ data: { ok: true } });
  await f.client.uploadChunk("file_1", 0, encoder.encode("private attachment"));
  expect(f.commands).toHaveLength(2);
  expect(f.commands[0]).toMatchObject({ type: "list_sessions" });
  expect(f.wires.every(wire => !decoder.decode(wire).includes("private attachment"))).toBe(true);
  expect(f.wires.every(wire => !decoder.decode(wire).includes("list_sessions"))).toBe(true);
  expect(f.wires.every(wire => !decoder.decode(wire).includes(JSON.parse(f.creds.secureBundle!).secret))).toBe(true);
  await f.client.close();
});

test("a lost pairing confirmation recovers using IK, without another scan or accepting a new identity", async () => {
  const f = fixture(); f.loseConfirmation();
  await f.pair();
  await expect(f.client.request({ type: "get_presence" })).resolves.toMatchObject({ success: true });
  expect(f.request).toHaveBeenCalledTimes(4); // XX messages, IK retry, command
  await f.client.close();
});

test("reconnecting rotates traffic keys and rejects prior-channel replay", async () => {
  const f = fixture(); await f.pair();
  const subject = "p.pair_1.cmd.list";
  const old = f.internal.secureChannels.get(f.connection)!.seal(subject, encode({ type: "get_presence" }));
  await f.pair();
  expect(() => f.serverChannel().open(subject, old)).toThrow("remote_secure_channel_invalid");
  await f.client.close();
});

test("tampered replies are rejected before RPC success can be observed", async () => {
  const f = fixture(); await f.pair(); f.corruptReply();
  await expect(f.client.request({ type: "get_presence" })).rejects.toThrow("remote_secure_channel_invalid");
  await f.client.close();
});

test("does not send any business command before a secure handshake", async () => {
  const f = fixture();
  await expect(f.client.request({ type: "approval_decision" })).rejects.toThrow("pairing_handshake_required");
  expect(f.request).not.toHaveBeenCalled();
  await f.client.close();
});

test("rejects old credentials rather than silently using v1", async () => {
  const f = fixture(); delete f.internal.credentials.secureBundle;
  await expect(f.pair()).rejects.toThrow("pairing_identity_mismatch");
  expect(f.request).not.toHaveBeenCalled();
  await f.client.close();
});

test("rejects a substituted desktop key even when the attacker relays the real handshake", async () => {
  const f = fixture();
  const bundle = JSON.parse(f.internal.credentials.secureBundle!);
  bundle.desktopKey = createSecureIdentity().publicKey;
  f.internal.credentials.secureBundle = JSON.stringify(bundle);
  await expect(f.pair()).rejects.toThrow();
  expect(f.internal.secureChannels.get(f.connection)).toBeUndefined();
  expect(f.commands).toHaveLength(0);
  await f.client.close();
});
