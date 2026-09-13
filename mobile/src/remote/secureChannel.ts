import { chacha20poly1305 } from "@noble/ciphers/chacha";
import Noise from "noise-handshake";
import { decodeBase64Url, encodeBase64Url } from "./codec";

const encoder = new TextEncoder();
const MAGIC = new Uint8Array([70, 82, 69, 50]); // FRE2
const HEADER = 28;
const WINDOW = 4096;
const MAX_SEQUENCE = 0xffffffff;
export const MAX_SECURE_PLAINTEXT = 1024 * 1024 - HEADER - 16;
const invalid = () => new Error("remote_secure_channel_invalid");

function join(...parts: Uint8Array[]): Uint8Array {
  const result = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const part of parts) { result.set(part, offset); offset += part.length; }
  return result;
}
function equal(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a[i]! ^ b[i]!;
  return diff === 0;
}
export function keyBytes(value: string): Uint8Array {
  const bytes = decodeBase64Url(value);
  if (!bytes || bytes.length !== 32 || encodeBase64Url(bytes) !== value) throw invalid();
  return bytes;
}
export function securePrologue(pairId: string, desktopId: string): Uint8Array {
  if (![pairId, desktopId].every(s => /^[A-Za-z0-9_-]{1,128}$/.test(s))) throw invalid();
  return encoder.encode(`futureos-remote-v2\n${pairId}\n${desktopId}`);
}
function aad(header: Uint8Array, context: string): Uint8Array {
  if (!context || context.length > 1024 || /[^\x00-\x7f]/.test(context)) throw invalid();
  return join(header, encoder.encode(context));
}
function nonce(sequence: number): Uint8Array {
  const value = new Uint8Array(12);
  new DataView(value.buffer).setUint32(4, sequence, true);
  return value;
}
export function replyContext(subject: string, request: Uint8Array): string {
  if (request.length < HEADER || !equal(request.subarray(0, 4), MAGIC)) throw invalid();
  const header = Array.from(request.subarray(4, HEADER), b => b.toString(16).padStart(2, "0")).join("");
  const context = `reply:${subject}:${header}`;
  if (context.length > 1024) throw invalid();
  return context;
}

/** Per-connection traffic keys. Never persist or reset the counters. */
export class SecureChannel {
  private next = 0;
  private highest = 0;
  // Sequence+1 in a ring: O(1) replay checks, no scan of 4096 entries per token.
  private seen = new Uint32Array(WINDOW);
  private destroyed = false;
  constructor(private send: Uint8Array, private receive: Uint8Array, private id: Uint8Array) {
    if (send.length !== 32 || receive.length !== 32 || id.length !== 16) throw invalid();
    this.send = send.slice(); this.receive = receive.slice(); this.id = id.slice();
  }
  seal(context: string, plaintext: Uint8Array): Uint8Array {
    if (this.destroyed || this.next >= MAX_SEQUENCE || plaintext.length > MAX_SECURE_PLAINTEXT) throw invalid();
    const sequence = this.next++;
    const header = new Uint8Array(HEADER);
    header.set(MAGIC); header.set(this.id, 4);
    new DataView(header.buffer).setUint32(20, sequence, true);
    return join(header, chacha20poly1305(this.send, nonce(sequence), aad(header, context)).encrypt(plaintext));
  }
  open(context: string, wire: Uint8Array): Uint8Array {
    if (this.destroyed || wire.length < HEADER + 16 || wire.length > 1024 * 1024 ||
        !equal(wire.subarray(0, 4), MAGIC) || !equal(wire.subarray(4, 20), this.id)) throw invalid();
    const view = new DataView(wire.buffer, wire.byteOffset, wire.byteLength);
    const sequence = view.getUint32(20, true);
    if (view.getUint32(24, true) !== 0 || sequence >= MAX_SEQUENCE || this.seen[sequence % WINDOW] === sequence + 1 ||
        (this.highest >= WINDOW && sequence <= this.highest - WINDOW)) throw invalid();
    let plaintext: Uint8Array;
    try {
      plaintext = chacha20poly1305(this.receive, nonce(sequence), aad(wire.subarray(0, HEADER), context))
        .decrypt(wire.subarray(HEADER));
    } catch { throw invalid(); }
    this.highest = Math.max(this.highest, sequence);
    this.seen[sequence % WINDOW] = sequence + 1;
    return plaintext;
  }
  destroy(): void {
    this.destroyed = true;
    this.send.fill(0); this.receive.fill(0); this.seen.fill(0);
  }
}

export interface SecureIdentity { privateKey: string; publicKey: string }
export function createSecureIdentity(): SecureIdentity {
  const noise = new Noise("XX", true);
  const identity = { privateKey: encodeBase64Url(noise.s.secretKey), publicKey: encodeBase64Url(noise.s.publicKey) };
  noise.s.secretKey.fill(0);
  return identity;
}

/** Noise XXpsk0 for QR pairing; IK for already pinned peers. No TOFU fallback. */
export class SecureHandshake {
  private noise: Noise;
  constructor(identity: SecureIdentity, pairId: string, desktopId: string,
    private expectedDesktopKey: string, secret?: string) {
    this.noise = new Noise(secret ? "XXpsk0" : "IK", true, {
      secretKey: keyBytes(identity.privateKey), publicKey: keyBytes(identity.publicKey),
    }, secret ? { psk: keyBytes(secret) } : undefined);
    this.noise.initialise(securePrologue(pairId, desktopId), secret ? undefined : keyBytes(expectedDesktopKey));
  }
  write(): Uint8Array { return this.noise.send(); }
  read(message: Uint8Array): void {
    if (message.length > 8192) throw invalid();
    this.noise.recv(message);
    if (!this.noise.rs || !equal(this.noise.rs, keyBytes(this.expectedDesktopKey))) throw invalid();
  }
  finish(): SecureChannel {
    if (!this.noise.complete || !this.noise.hash || !this.noise.tx || !this.noise.rx) throw invalid();
    const channel = new SecureChannel(this.noise.tx, this.noise.rx, this.noise.hash.subarray(0, 16));
    this.destroy();
    return channel;
  }
  destroy(): void {
    // The library clears ephemeral secrets on completion. Also clear the
    // static-key copy, PSK and split keys when the caller abandons a handshake.
    this.noise.s.secretKey.fill(0);
    this.noise.e?.secretKey.fill(0);
    this.noise.psk?.fill(0);
    this.noise.tx?.fill(0); this.noise.rx?.fill(0);
  }
}
