import { gunzipSync } from "fflate";

const GZIP_MAGIC = [0x1f, 0x8b] as const;
// Command replies are already bounded below the NATS payload cap. Enforce the
// same ceiling before allocating decompressed output from the wire.
const MAX_REMOTE_JSON_BYTES = 1024 * 1024;
const decoder = new TextDecoder();

function isGzip(data: Uint8Array): boolean {
  if (data[0] !== GZIP_MAGIC[0] || data[1] !== GZIP_MAGIC[1]) {
    return false;
  }
  if (data.length < 4) throw new Error("remote_json_gzip_invalid");
  // gzip's final ISIZE field is the uncompressed byte length modulo 2^32.
  // Our protocol caps JSON far below 4 GiB, so it is a cheap pre-allocation
  // bomb guard before fflate creates the output buffer.
  const footer = data.length - 4;
  const jsonBytes =
    ((data[footer] ?? 0) |
      ((data[footer + 1] ?? 0) << 8) |
      ((data[footer + 2] ?? 0) << 16) |
      ((data[footer + 3] ?? 0) << 24)) >>>
    0;
  if (jsonBytes > MAX_REMOTE_JSON_BYTES) throw new Error("remote_json_gzip_too_large");
  return true;
}

/** Return bounded plaintext without changing the JSON payload. */
export function remoteJsonBytes(data: Uint8Array): Uint8Array {
  const jsonBytes = isGzip(data) ? gunzipSync(data) : data;
  if (jsonBytes.length > MAX_REMOTE_JSON_BYTES) throw new Error("remote_json_too_large");
  return jsonBytes;
}

/** Decode an automatically selected plain JSON or standard gzip JSON reply. */
export function decodeRemoteJson<T>(data: Uint8Array): T {
  const jsonBytes = remoteJsonBytes(data);
  return JSON.parse(decoder.decode(jsonBytes)) as T;
}

/**
 * The magic that marks a reply whose payload travels outside its JSON.
 *
 * Must stay byte-identical to the Rust side (`future_remote_protocol::json`):
 * four ASCII bytes that cannot begin a JSON document and are not the gzip
 * magic, checked *after* inflating.
 */
export const BINARY_FRAME_MAGIC = new Uint8Array([0x46, 0x42, 0x52, 0x31]);

/** Magic plus the big-endian `u32` header length, as Rust writes it. */
const BINARY_FRAME_PREFIX = BINARY_FRAME_MAGIC.length + 4;

export interface RemoteBinaryFrame {
  /** The reply envelope: `{"type","success","data","error"}`, still JSON. */
  header: Uint8Array;
  /** The bytes that travelled behind that JSON. */
  body: Uint8Array;
}

/** Whether a reply body is a binary frame rather than a JSON document. */
export function looksLikeBinaryFrame(data: Uint8Array): boolean {
  if (data.length < BINARY_FRAME_MAGIC.length) return false;
  for (let index = 0; index < BINARY_FRAME_MAGIC.length; index += 1) {
    if (data[index] !== BINARY_FRAME_MAGIC[index]) return false;
  }
  return true;
}

/**
 * Split an inflated reply body into header and carried bytes.
 *
 * `null` for an ordinary JSON reply, which is what an older Desktop still sends
 * for a paged read: the carrier is detected by its magic, so a client never has
 * to know which one the host chose. A frame whose length prefix does not fit its
 * body is refused instead of split at a guessed offset.
 */
export function splitBinaryFrame(data: Uint8Array): RemoteBinaryFrame | null {
  if (!looksLikeBinaryFrame(data)) return null;
  if (data.length < BINARY_FRAME_PREFIX) throw new Error("remote_binary_frame_invalid");
  const length = ((data[4]! << 24) | (data[5]! << 16) | (data[6]! << 8) | data[7]!) >>> 0;
  const end = BINARY_FRAME_PREFIX + length;
  if (end > data.length) throw new Error("remote_binary_frame_invalid");
  return { header: data.subarray(BINARY_FRAME_PREFIX, end), body: data.subarray(end) };
}

/** One inflated reply: its JSON document, and the bytes a frame carried. */
export interface RemoteReplyBody {
  json: Uint8Array;
  body?: Uint8Array;
}

/**
 * Inflate a reply body and separate its JSON from any binary payload.
 *
 * Both steps live together because the order matters: the magic is only visible
 * once the gzip layer is gone, and a caller that split first would classify every
 * compressed reply as a document.
 */
export function remoteReplyBody(data: Uint8Array): RemoteReplyBody {
  const plain = remoteJsonBytes(data);
  const frame = splitBinaryFrame(plain);
  return frame ? { json: frame.header, body: frame.body } : { json: plain };
}
