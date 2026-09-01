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

/** Decode an automatically selected plain JSON or standard gzip JSON reply. */
export function decodeRemoteJson<T>(data: Uint8Array): T {
  const jsonBytes = isGzip(data) ? gunzipSync(data) : data;
  if (jsonBytes.length > MAX_REMOTE_JSON_BYTES) throw new Error("remote_json_too_large");
  return JSON.parse(decoder.decode(jsonBytes)) as T;
}
