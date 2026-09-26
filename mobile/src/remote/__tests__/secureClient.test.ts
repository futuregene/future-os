import Noise from "noise-handshake";
import type { Msg, NatsConnection } from "@nats-io/nats-core";
import { MsgImpl, QueuedIteratorImpl } from "@nats-io/nats-core/internal";
import { ConnectionGeneration } from "../connectionGeneration";
import { RemoteClient, type RemoteClientCallbacks } from "../client";
import { createSecureIdentity, keyBytes, replyContext, SecureChannel, securePrologue } from "../secureChannel";
import { decodeBase64Url, encodeBase64Url } from "../codec";
import type { RemoteCredentials } from "../types";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const encode = (value: unknown) => encoder.encode(JSON.stringify(value));
const natsMessage = (subject: string, data: Uint8Array) => new MsgImpl(
  { subject: encoder.encode(subject), reply: new Uint8Array(), sid: 1, hdr: -1, size: data.length },
  data, { publish: jest.fn() },
);
const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };

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
  let omitConfirmation = false;
  let corruptReply = false;
  let breakConfirmation = false;
  const commands: unknown[] = [];
  const wires: Uint8Array[] = [];
  const confirmation = () => {
    if (!pending?.complete || !pending.hash || !pending.tx || !pending.rx) throw new Error("incomplete");
    serverChannel = new SecureChannel(pending.tx, pending.rx, pending.hash.subarray(0, 16));
    pinned = encodeBase64Url(pending.rs!);
    return encodeBase64Url(serverChannel.seal("handshake-confirm", encode(breakConfirmation
      ? { confirmed: false, pairId: "pair_1", bridgeInstanceId: "bridge_1",
          presence: { bridgeInstanceId: "bridge_1", online: true }, features: ["e2ee_v2"] }
      : { confirmed: true, pairId: "pair_1",
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
      return natsMessage("untrusted-inbox", reply);
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
      // The desktop identity is pinned by a real confirmation even when the
      // reply under test omits it, so the client's refusal is about the missing
      // field and not about an unknown peer.
      const confirmed = body.pairing ? undefined : confirmation();
      return { data: encode({ success: true, data: { id: "challenge", message,
        ...(confirmed && !omitConfirmation ? { confirmation: confirmed } : {}) } }) } as Msg;
    }
    if (body.type === "secure_finish") {
      pending!.recv(decodeBase64Url(body.message)!);
      const confirmed = confirmation();
      if (loseConfirmation) { loseConfirmation = false; throw new Error("lost response"); }
      return { data: encode({ success: true, data: { ...(omitConfirmation ? {} : { confirmation: confirmed }) } }) } as Msg;
    }
    throw new Error("plaintext business command");
  });
  const subscriptions = new Map<string, QueuedIteratorImpl<Msg>>();
  const connection = {
    request,
    subscribe: (subject: string) => {
      const queue = new QueuedIteratorImpl<Msg>();
      Object.assign(queue, { unsubscribe: () => queue.stop() });
      subscriptions.set(subject, queue);
      return queue;
    },
    close: async () => { for (const queue of subscriptions.values()) queue.stop(); },
    flush: jest.fn(async () => {}), isClosed: () => false,
  } as unknown as NatsConnection;
  const internal = client as unknown as {
    connection: NatsConnection; credentials: RemoteCredentials;
    performHandshake(connection: NatsConnection): Promise<unknown>;
    secureChannels: WeakMap<NatsConnection, SecureChannel>;
    subscribeEvents(connection: NatsConnection, generation: number, selective?: boolean): void;
    subscribeState(connection: NatsConnection, generation: number): void;
    subscribeLiveness(connection: NatsConnection, generation: number): void;
    subscribeTransfers(connection: NatsConnection, generation: number): void;
    desktopAvailable: boolean;
    state: string;
  };
  internal.connection = connection;
  return { client, internal, connection, subscriptions, request, commands, wires, callbacks, creds, desktop,
    pair: async () => {
      const result = await internal.performHandshake(connection);
      internal.desktopAvailable = true;
      internal.state = "ready";
      return result;
    },
    loseConfirmation: () => { loseConfirmation = true; },
    omitConfirmation: () => { omitConfirmation = true; },
    corruptReply: () => { corruptReply = true; },
    mismatchConfirmation: () => { breakConfirmation = true; },
    serverChannel: () => serverChannel!,
    restartDesktop: () => { serverChannel?.destroy(); serverChannel = undefined; },
  };
}

test("real NATS messages retain routing after decryption across repeated catalog pushes", async () => {
  const f = fixture();
  jest.spyOn(f.client, "open").mockResolvedValue(undefined);
  try {
    await f.pair();
    const owner = new ConnectionGeneration(1);
    owner.activate();
    Object.assign(f.internal, { generation: 1, activeGeneration: owner, state: "ready", confirmedBridgeInstanceId: "bridge_1" });
    f.internal.subscribeEvents(f.connection, 1);
    f.internal.subscribeState(f.connection, 1);
    f.internal.subscribeLiveness(f.connection, 1);
    f.internal.subscribeTransfers(f.connection, 1);
    const deliver = async (subscription: string, subject: string, plaintext: Uint8Array) => {
      const message = natsMessage(subject, f.serverChannel().seal(subject, plaintext));
      expect(message.subject).toBe(subject);
      // This is the production SDK object, not a plain-object imitation. The
      // encrypted-message spread introduced in v2 loses these prototype getters.
      expect(Object.keys(message)).not.toContain("subject");
      f.subscriptions.get(subscription)!.push(message);
      await settle();
    };
    for (let tick = 0; tick < 12; tick++) {
      await deliver("p.pair_1.presence", "p.pair_1.presence", encode({ online: true, pairId: "pair_1", bridgeInstanceId: "bridge_1", lastHeartbeatTs: Date.now() / 1000 }));
      await deliver("p.pair_1.state.>", "p.pair_1.state.sessions", encode({ sessions: [{ sessionId: "s1", title: "Session" }], version: { epoch: "epoch", revision: tick } }));
      await deliver("p.pair_1.state.>", "p.pair_1.state.workspaces", encode({ workspaces: [{ id: "w1", name: "Project" }] }));
      await deliver("p.pair_1.evt.>", "p.pair_1.evt.s1", encode({ type: "agent_start", runId: "r1", data: "{}" }));
      expect(f.callbacks.onSessions).toHaveBeenCalledTimes(tick + 1);
      expect(f.callbacks.onWorkspaces).toHaveBeenCalledTimes(tick + 1);
      expect(f.callbacks.onEvent).toHaveBeenLastCalledWith({ type: "agent_start", runId: "r1", data: "{}" }, "s1");
      expect(f.callbacks.onError).not.toHaveBeenCalled();
      expect(f.callbacks.onConnectionState).not.toHaveBeenCalledWith("reconnecting");
    }
    const download = f.client.downloadChunk("file_1", 0);
    const bytes = encoder.encode("file contents");
    await deliver("p.pair_1.xfer.down.>", "p.pair_1.xfer.down.file_1.chunk.0", bytes);
    await expect(download).resolves.toEqual(bytes);
    expect(f.client.open).not.toHaveBeenCalled();
  } finally { await f.client.close(); }
});

test("selective event subscriptions retain background approvals/end events and retire old session feeds safely", async () => {
  const f = fixture();
  try {
    await f.pair();
    const owner = new ConnectionGeneration(1); owner.activate();
    Object.assign(f.internal, { generation: 1, activeGeneration: owner, state: "ready" });
    f.client.setVisibleSession("s1");
    f.internal.subscribeEvents(f.connection, 1, true);
    f.internal.subscribeState(f.connection, 1);
    expect(f.subscriptions.has("p.pair_1.evt.>")).toBe(false);
    expect(f.subscriptions.has("p.pair_1.evt.s1")).toBe(true);
    for (const type of ["approval_request", "agent_end"]) {
      const subject = "p.pair_1.state.events";
      f.subscriptions.get("p.pair_1.state.>")!.push(natsMessage(subject,
        f.serverChannel().seal(subject, encode({ sessionId: "background", type, runId: "r", data: "{}" }))));
      await settle();
      expect(f.callbacks.onEvent).toHaveBeenLastCalledWith(expect.objectContaining({ type }), "background");
    }
    const old = f.subscriptions.get("p.pair_1.evt.s1")!;
    f.client.setVisibleSession("s2");
    await settle();
    expect(old.done).toBe(true);
    expect(f.subscriptions.has("p.pair_1.evt.s2")).toBe(true);
    f.client.setVisibleSession("");
    await settle();
    expect(f.subscriptions.get("p.pair_1.evt.s2")!.done).toBe(true);
    expect(f.callbacks.onError).not.toHaveBeenCalled();
    expect(f.callbacks.onConnectionState).not.toHaveBeenCalledWith("reconnecting");
  } finally { await f.client.close(); }
});

test("a queued encrypted event burst yields before all records have decoded", async () => {
  jest.useFakeTimers();
  const f = fixture();
  try {
    await f.pair();
    const owner = new ConnectionGeneration(1); owner.activate();
    Object.assign(f.internal, { generation: 1, activeGeneration: owner, state: "ready" });
    f.internal.subscribeEvents(f.connection, 1);
    const subject = "p.pair_1.evt.s1";
    for (let i = 0; i < 500; i++) f.subscriptions.get("p.pair_1.evt.>")!.push(natsMessage(subject,
      f.serverChannel().seal(subject, encode({ type: "text_chunk", idx: i, data: '{"text":"x"}' }))));
    let atInput = 500;
    setTimeout(() => { atInput = (f.callbacks.onEvent as jest.Mock).mock.calls.length; }, 0);
    await jest.runAllTimersAsync();
    expect(atInput).toBeLessThan(500);
    expect(f.callbacks.onEvent).toHaveBeenCalledTimes(500);
  } finally { await f.client.close(); jest.useRealTimers(); }
});

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

test("desktop traffic-key loss recovers through Noise IK even when the broker is still reachable", async () => {
  const f = fixture();
  await f.pair();
  f.restartDesktop();
  await f.connection.flush();
  await expect(f.client.request({ type: "get_presence" })).rejects.toThrow();
  // Only socket creation is replaced; handshake and record crypto are real.
  const open = jest.spyOn(f.client, "open").mockImplementation(async () => { await f.pair(); });
  await Promise.all([f.client.recoverNow("presence-stale"), f.client.recoverNow("presence-stale")]);
  expect(open).toHaveBeenCalledTimes(1);
  await expect(f.client.request({ type: "get_presence" })).resolves.toMatchObject({ success: true });
  expect(f.callbacks.onCredentials).toHaveBeenCalledTimes(1);
  await f.client.close();
});

test("tampered replies are rejected before RPC success can be observed", async () => {
  const f = fixture(); await f.pair(); f.corruptReply();
  await expect(f.client.request({ type: "get_presence" })).rejects.toThrow("remote_secure_channel_invalid");
  await f.client.close();
});

test("does not send any business command before a secure handshake", async () => {
  const f = fixture();
  f.internal.desktopAvailable = true;
  f.internal.state = "ready";
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

test("a refused handshake carries the desktop's own reason, not just the stable token", async () => {
  const f = fixture();
  const base = f.request.getMockImplementation()!;
  f.request.mockImplementation(async (subject: string, data: Uint8Array) => {
    const body = JSON.parse(decoder.decode(data)) as { type?: string };
    if (body.type === "secure_open") {
      // The phone holds a code the desktop will not accept. The stable
      // `pairing_signature_invalid` token is what the credential classifier and
      // the connection presentation match on; the detail is the only statement
      // of *why*, so it has to survive into the error.
      return natsMessage("untrusted-inbox", encode({ success: false, error: "invitation_expired" }));
    }
    return base(subject, data);
  });
  await expect(f.pair()).rejects.toThrow("pairing_signature_invalid:invitation_expired");
  expect(f.internal.secureChannels.get(f.connection)).toBeUndefined();
  await f.client.close();
});

test("a confirmation whose binding does not match the pairing is refused", async () => {
  const f = fixture();
  // Every other field is well-formed and the AEAD is genuine; only the
  // `confirmed` flag disagrees. A phone that accepted this would hand the
  // desktop keys to whoever produced the frame.
  f.mismatchConfirmation();
  await expect(f.pair()).rejects.toThrow("pairing_confirmation_mismatch");
  expect(f.internal.secureChannels.get(f.connection)).toBeUndefined();
  await f.client.close();
});

/** Handshake frames are plaintext JSON; the sealed business/secure frames start
 * with the FRE2 magic, so this counts only identity exchanges. */
const jsonRequests = (f: ReturnType<typeof fixture>) =>
  f.request.mock.calls.filter(([, data]) => decoder.decode(data.subarray(0, 1)) === "{").length;

test("a handshake reply with no message is refused instead of pairing on nothing", async () => {
  const f = fixture();
  // A successful first pairing drops the invitation secret, so this is the
  // reconnect path: one IK exchange, whose answer carries no key exchange.
  await f.pair();
  const base = f.request.getMockImplementation()!;
  f.request.mockImplementation(async (subject: string, data: Uint8Array) => {
    const body = JSON.parse(decoder.decode(data)) as { type?: string; pairing?: boolean };
    if (body.type === "secure_open" && !body.pairing) {
      return natsMessage("untrusted-inbox", encode({ success: true, data: { id: "challenge" } }));
    }
    return base(subject, data);
  });
  await expect(f.internal.performHandshake(f.connection)).rejects.toThrow("pairing_signature_invalid");
  await f.client.close();
});

test("a handshake reply with no confirmation is refused instead of trusting the socket", async () => {
  const f = fixture();
  // The desktop answers the key exchange but never states which identity it
  // bound: with nothing to verify, the phone must not keep the channel. The
  // second, secretless attempt gets the same silent answer.
  f.omitConfirmation();
  await expect(f.pair()).rejects.toThrow("pairing_confirmation_mismatch");
  expect(f.internal.secureChannels.get(f.connection)).toBeUndefined();
  await f.client.close();
});

test("a pairing that closes after the confirmation is not retried without the secret", async () => {
  const f = fixture();
  Object.assign(f.internal, { stopped: true });
  await expect(f.internal.performHandshake(f.connection)).rejects.toThrow("not_connected");
  // Exactly the two frames of one attempt (open + finish): a client the user
  // closed must not start the second, unauthenticated exchange.
  expect(jsonRequests(f)).toBe(2);
  expect(f.internal.secureChannels.get(f.connection)).toBeUndefined();
  await f.client.close();
});

test("a pairing that closes while writing the rotated identity is not retried", async () => {
  const f = fixture();
  jest.mocked(f.callbacks.onCredentials).mockImplementation(async () => {
    Object.assign(f.internal, { stopped: true });
  });
  await expect(f.internal.performHandshake(f.connection)).rejects.toThrow("not_connected");
  expect(jsonRequests(f)).toBe(2);
  expect(f.internal.secureChannels.get(f.connection)).toBeUndefined();
  await f.client.close();
});
