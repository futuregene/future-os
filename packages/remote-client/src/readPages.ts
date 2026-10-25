import type { RemoteClient } from "./client";
import { decodeJsonBytes } from "./cooperativeJson";
import { decodeBase64Url } from "./codec";
import type { RemoteCommand, RpcResponse } from "./types";

const MAX_READ_BYTES = 16 * 1024 * 1024;
const CHUNK_BYTES = 192 * 1024;
// Bound in-flight replies/decodes while amortizing round-trip latency for large
// histories. Replay pages themselves still follow their fixed watermark.
const READ_CONCURRENCY = 4;
// The address of one chunk, exactly as the host writes it. `data` is present
// only for the base64 carrier; a negotiated binary page carries the same bytes
// beside this object instead (`ClientRpcResponse.chunkBytes`).
interface Chunk { id: string; offset: number; nextOffset: number; totalBytes: number; data?: string }
interface CarriedChunk { meta: Chunk; bytes?: Uint8Array }

function chunkOf(value: unknown, carried: unknown): CarriedChunk | null {
  if (!value || typeof value !== "object" || !("readChunk" in value)) return null;
  const chunk = value.readChunk as Partial<Chunk> | null;
  if (!chunk || typeof chunk !== "object"
    || typeof chunk.id !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(chunk.id)
    || !Number.isSafeInteger(chunk.offset) || !Number.isSafeInteger(chunk.nextOffset)
    || !Number.isSafeInteger(chunk.totalBytes) || chunk.totalBytes! <= 0 || chunk.totalBytes! > MAX_READ_BYTES)
    throw new Error("remote_read_invalid_chunk");
  if (carried !== undefined) {
    // The frame carrier: the chunk's bytes came out of the reply body rather
    // than out of `data`, so the encoded-length bound that guards `data` (4/3 of
    // the chunk, base64 being the expansion factor) has no equivalent here —
    // the raw length is checked instead, and the "full chunk while the page
    // continues" rule stays in `storeChunk` where it applies to both carriers.
    if (!(carried instanceof Uint8Array) || carried.length === 0 || carried.length > CHUNK_BYTES)
      throw new Error("remote_read_invalid_chunk");
    return { meta: chunk as Chunk, bytes: carried };
  }
  if (typeof chunk.data !== "string" || chunk.data.length > CHUNK_BYTES * 4 / 3)
    throw new Error("remote_read_invalid_chunk");
  return { meta: chunk as Chunk };
}

/** Reassemble negotiated history/replay/directory reads. Each NATS reply remains
 * small; the caller interprets data only after the immutable JSON snapshot is
 * complete. Old desktops ignore chunkedRead.
 *
 * A chunk arrives on one of two carriers and this reader does not care which:
 * `data` as base64 (what an undeclared desktop keeps sending), or the reply's
 * own bytes when the desktop negotiated `read_binary_v1`. Everything the
 * snapshot pins — `id`, `totalBytes`, `offset`, `nextOffset` — is JSON on both,
 * so the checks below are written once.
 */
export async function requestReadPage<T>(
  client: RemoteClient,
  command: RemoteCommand,
  sessionId: string,
  isCurrent: () => boolean = () => true,
): Promise<RpcResponse<T>> {
  if (!isCurrent()) throw new Error("stale_sync_lane");
  const response = await client.requestRetry<unknown>({ ...command, chunkedRead: true }, sessionId);
  if (!isCurrent()) throw new Error("stale_sync_lane");
  const first = chunkOf(response.data, response.chunkBytes);
  if (!first) return response as RpcResponse<T>;
  const id = first.meta.id;
  const total = first.meta.totalBytes;
  const bytes = new Uint8Array(total);
  const storeChunk = (carried: CarriedChunk, offset: number) => {
    const chunk = carried.meta;
    if (chunk.id !== id || chunk.totalBytes !== total || chunk.offset !== offset)
      throw new Error("remote_read_chunk_changed");
    const part = carried.bytes ?? decodeBase64Url(chunk.data ?? "");
    if (!part || part.length === 0 || part.length > CHUNK_BYTES
      || chunk.nextOffset !== offset + part.length || chunk.nextOffset > total
      || (chunk.nextOffset < total && part.length !== CHUNK_BYTES))
      throw new Error("remote_read_invalid_chunk");
    bytes.set(part, offset);
  };
  storeChunk(first, 0);
  // The initial reply pins an immutable snapshot and its size. Its remaining
  // offsets are independent: out-of-order replies write into disjoint ranges.
  // Use bounded batches so failure/cancellation cannot launch another wave.
  for (let start = first.meta.nextOffset; start < total; start += READ_CONCURRENCY * CHUNK_BYTES) {
    if (!isCurrent()) throw new Error("stale_sync_lane");
    const offsets = Array.from({ length: Math.min(READ_CONCURRENCY, Math.ceil((total - start) / CHUNK_BYTES)) },
      (_, index) => start + index * CHUNK_BYTES);
    await Promise.all(offsets.map(async offset => {
      const next = await client.requestRetry<unknown>({
        type: "get_read_chunk", sessionId, replyId: id, offset,
        ...(command.runId ? { runId: command.runId } : {}),
        ...(command.bridgeInstanceId ? { bridgeInstanceId: command.bridgeInstanceId } : {}),
      }, sessionId);
      if (!isCurrent()) throw new Error("stale_sync_lane");
      const part = chunkOf(next.data, next.chunkBytes);
      if (!part) throw new Error("remote_read_invalid_chunk");
      storeChunk(part, offset);
    }));
  }
  const data = await decodeJsonBytes<T>(bytes, isCurrent);
  return { ...response, data };
}
