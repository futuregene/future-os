import Noise from "noise-handshake";
import { createSecureIdentity, keyBytes, MAX_SECURE_PLAINTEXT, replyContext, SecureChannel, SecureHandshake, securePrologue } from "../secureChannel";
import { encodeBase64Url } from "../codec";

function pair() {
  const desktop = createSecureIdentity();
  const mobile = createSecureIdentity();
  const secret = new Uint8Array(32).fill(7);
  const initiator = new SecureHandshake(mobile, "pair_1", "desktop_1", desktop.publicKey, encodeBase64Url(secret));
  const responder = new Noise("XXpsk0", false, {
    publicKey: keyBytes(desktop.publicKey), secretKey: keyBytes(desktop.privateKey),
  }, { psk: secret });
  responder.initialise(securePrologue("pair_1", "desktop_1"));
  responder.recv(initiator.write());
  initiator.read(responder.send());
  responder.recv(initiator.write());
  expect(encodeBase64Url(responder.rs!)).toBe(mobile.publicKey);
  return { i: initiator.finish(), r: new SecureChannel(responder.tx!, responder.rx!, responder.hash!.subarray(0, 16)) };
}

// The same deterministic vectors are checked independently by Rust/snow.
const vectors = require("../../../../packages/remote-crypto/tests/vectors.json") as {
  pattern: "XXpsk0" | "IK"; desktopPublicKey: string; messages: string[]; hash: string;
  context: string; plaintext: string; request: string; replyContext: string; reply: string;
}[];
const unhex = (value: string) => Uint8Array.from(value.match(/../g)!, byte => parseInt(byte, 16));
const hex = (value: Uint8Array) => Array.from(value, b => b.toString(16).padStart(2, "0")).join("");

describe("remote v2 secure records", () => {
  test.each(vectors)("matches Rust/snow for $pattern, including record AAD and replies", vector => {
    const responder = new Noise(vector.pattern, false, { secretKey: new Uint8Array(32).fill(2), publicKey: unhex(vector.desktopPublicKey) },
      vector.pattern === "XXpsk0" ? { psk: new Uint8Array(32).fill(7) } : undefined);
    responder.e = { secretKey: new Uint8Array(32).fill(4), publicKey: unhex(vector.messages[1]!.slice(0, 64)) };
    responder.initialise(securePrologue("pair_1", "desktop_1"));
    responder.recv(unhex(vector.messages[0]!));
    expect(hex(responder.send())).toBe(vector.messages[1]);
    if (vector.messages[2]) responder.recv(unhex(vector.messages[2]));
    expect(hex(responder.hash!)).toBe(vector.hash);
    const channel = new SecureChannel(responder.tx!, responder.rx!, responder.hash!.subarray(0, 16));
    expect(new TextDecoder().decode(channel.open(vector.context, unhex(vector.request)))).toBe(vector.plaintext);
    expect(replyContext(vector.context, unhex(vector.request))).toBe(vector.replyContext);
    expect(hex(channel.seal(vector.replyContext, new TextEncoder().encode("private reply")))).toBe(vector.reply);
  });
  test("authenticates a QR-paired channel and binds each reply to its request", () => {
    const { i, r } = pair();
    const wire = i.seal("p.pair_1.cmd.list", new TextEncoder().encode("private prompt"));
    expect(new TextDecoder().decode(wire)).not.toContain("private prompt");
    expect(new TextDecoder().decode(r.open("p.pair_1.cmd.list", wire))).toBe("private prompt");
    const context = replyContext("p.pair_1.cmd.list", wire);
    const reply = r.seal(context, new Uint8Array([1, 2]));
    expect(() => i.open(`${context}0`, reply)).toThrow("remote_secure_channel_invalid");
    expect(i.open(context, reply)).toEqual(new Uint8Array([1, 2]));
  });
  test("rejects every tampered byte, wrong subject, reflected messages and plaintext", () => {
    const { i, r } = pair();
    const wire = i.seal("approval", new Uint8Array([1, 2, 3]));
    for (let n = 0; n < wire.length; n++) {
      const tampered = wire.slice(); tampered[n] = tampered[n]! ^ 1;
      expect(() => r.open("approval", tampered)).toThrow();
    }
    expect(() => r.open("prompt", wire)).toThrow();
    expect(() => i.open("approval", wire)).toThrow();
    expect(() => r.open("approval", new TextEncoder().encode('{"type":"approval_decision"}'))).toThrow();
    expect(r.open("approval", wire)).toEqual(new Uint8Array([1, 2, 3]));
    expect(() => r.open("approval", wire)).toThrow();
  });
  test("accepts reordered records and enforces the wire budget", () => {
    const { i, r } = pair();
    const first = i.seal("file", new Uint8Array([1]));
    const second = i.seal("file", new Uint8Array(MAX_SECURE_PLAINTEXT).fill(42));
    expect(second.length).toBe(1024 * 1024);
    expect(r.open("file", second).length).toBe(MAX_SECURE_PLAINTEXT);
    expect(r.open("file", first)).toEqual(new Uint8Array([1]));
    expect(() => i.seal("file", new Uint8Array(MAX_SECURE_PLAINTEXT + 1))).toThrow();
  });
  test("enforces replay-window boundaries, ring wrap and nonce exhaustion", () => {
    const { i, r } = pair();
    const zero = i.seal("event", new Uint8Array([0]));
    const one = i.seal("event", new Uint8Array([1]));
    (i as unknown as { next: number }).next = 4096;
    r.open("event", i.seal("event", new Uint8Array([2])));
    expect(() => r.open("event", zero)).toThrow();
    expect(r.open("event", one)).toEqual(new Uint8Array([1]));
    r.open("event", i.seal("event", new Uint8Array([3])));
    expect(() => r.open("event", one)).toThrow();
    (i as unknown as { next: number }).next = 0xfffffffe;
    r.open("event", i.seal("event", new Uint8Array([4])));
    expect(() => i.seal("event", new Uint8Array())).toThrow();
  });
  test("destroyed channels cannot send or receive", () => {
    const { i, r } = pair();
    const wire = i.seal("command", new Uint8Array());
    r.destroy();
    expect(() => r.open("command", wire)).toThrow();
    expect(() => r.seal("command", new Uint8Array())).toThrow();
  });
  test("a new handshake rejects records from the previous connection", () => {
    const old = pair(); const fresh = pair();
    const wire = old.i.seal("command", new Uint8Array([1]));
    expect(() => fresh.r.open("command", wire)).toThrow();
  });
  test("refuses keys and identifiers that are not canonical v2 material", () => {
    const key = encodeBase64Url(new Uint8Array(32).fill(9));
    // A short or non-canonical key would seed Noise with bytes the desktop
    // never bound; the failure has to be this stable code, not a library error.
    expect(() => keyBytes("AAAA")).toThrow("remote_secure_channel_invalid");
    expect(() => keyBytes(`${key}=`)).toThrow("remote_secure_channel_invalid");
    expect(keyBytes(key).length).toBe(32);
    // Identifiers go into the Noise prologue verbatim.
    expect(() => securePrologue("pair 1", "desktop_1")).toThrow("remote_secure_channel_invalid");
    expect(() => securePrologue("pair_1", "x".repeat(129))).toThrow("remote_secure_channel_invalid");
  });
  test("refuses a channel whose split keys or id are the wrong size", () => {
    const bytes = keyBytes(createSecureIdentity().publicKey);
    expect(() => new SecureChannel(new Uint8Array(31), bytes, new Uint8Array(16)))
      .toThrow("remote_secure_channel_invalid");
    expect(() => new SecureChannel(bytes, bytes, new Uint8Array(15)))
      .toThrow("remote_secure_channel_invalid");
  });
  test("refuses contexts that cannot be authenticated and requests too broken to answer", () => {
    const { i } = pair();
    // The context is authenticated as AAD, so it must stay ASCII and bounded
    // for both peers to derive the same bytes.
    expect(() => i.seal("", new Uint8Array([1]))).toThrow("remote_secure_channel_invalid");
    expect(() => i.seal("caf\u00e9", new Uint8Array([1]))).toThrow("remote_secure_channel_invalid");
    expect(() => i.seal("x".repeat(1025), new Uint8Array([1]))).toThrow("remote_secure_channel_invalid");
    // A reply binds to the request it answers; a truncated record has no header
    // to hash, and an oversized subject cannot fit the AAD budget.
    expect(() => replyContext("p.pair_1.cmd.list", new Uint8Array(8)))
      .toThrow("remote_secure_channel_invalid");
    const wire = i.seal("p.pair_1.cmd.list", new Uint8Array([1]));
    expect(() => replyContext("x".repeat(1100), wire)).toThrow("remote_secure_channel_invalid");
  });
  test("refuses an oversized handshake message and an unfinished handshake", () => {
    const identity = createSecureIdentity();
    const handshake = new SecureHandshake(
      identity, "pair_1", "desktop_1", identity.publicKey,
      encodeBase64Url(new Uint8Array(32).fill(1)),
    );
    // No reply has been read yet: there is no shared secret to hand over.
    expect(() => handshake.finish()).toThrow("remote_secure_channel_invalid");
    // A message beyond the protocol's frame budget is refused before the Noise
    // state machine ever sees it.
    expect(() => handshake.read(new Uint8Array(8193))).toThrow("remote_secure_channel_invalid");
    handshake.destroy();
  });
});
