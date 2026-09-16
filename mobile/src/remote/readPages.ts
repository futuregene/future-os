import type { RemoteClient } from "./client";
import { decodeJsonBytes } from "./cooperativeJson";
import { decodeBase64Url } from "./codec";
import type { RemoteCommand, RpcResponse } from "./types";

const MAX_READ_BYTES = 16 * 1024 * 1024;
const CHUNK_BYTES = 192 * 1024;
// Bound in-flight replies/decodes while amortizing round-trip latency for large
// histories. Replay pages themselves still follow their fixed watermark.
const READ_CONCURRENCY = 4;
interface Chunk { id: string; offset: number; nextOffset: number; totalBytes: number; data: string }

function chunkOf(value: unknown): Chunk | null {
  if (!value || typeof value !== "object" || !("readChunk" in value)) return null;
  const chunk = value.readChunk as Partial<Chunk> | null;
  if (!chunk || typeof chunk !== "object"
    || typeof chunk.id !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(chunk.id)
    || typeof chunk.data !== "string" || chunk.data.length > CHUNK_BYTES * 4 / 3
    || !Number.isSafeInteger(chunk.offset) || !Number.isSafeInteger(chunk.nextOffset)
    || !Number.isSafeInteger(chunk.totalBytes) || chunk.totalBytes! <= 0 || chunk.totalBytes! > MAX_READ_BYTES)
    throw new Error("remote_read_invalid_chunk");
  return chunk as Chunk;
}

/** Reassemble negotiated history/replay/directory reads. Each NATS reply remains
 * small; the caller interprets data only after the immutable JSON snapshot is
 * complete. Old desktops ignore chunkedRead.
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
  const chunk = chunkOf(response.data);
  if (!chunk) return response as RpcResponse<T>;
  const id = chunk.id;
  const total = chunk.totalBytes;
  const bytes = new Uint8Array(total);
  const storeChunk = (chunk: Chunk, offset: number) => {
    if (chunk.id !== id || chunk.totalBytes !== total || chunk.offset !== offset)
      throw new Error("remote_read_chunk_changed");
    const part = decodeBase64Url(chunk.data);
    if (!part || part.length === 0 || part.length > CHUNK_BYTES
      || chunk.nextOffset !== offset + part.length || chunk.nextOffset > total
      || (chunk.nextOffset < total && part.length !== CHUNK_BYTES))
      throw new Error("remote_read_invalid_chunk");
    bytes.set(part, offset);
  };
  storeChunk(chunk, 0);
  // The initial reply pins an immutable snapshot and its size. Its remaining
  // offsets are independent: out-of-order replies write into disjoint ranges.
  // Use bounded batches so failure/cancellation cannot launch another wave.
  for (let start = chunk.nextOffset; start < total; start += READ_CONCURRENCY * CHUNK_BYTES) {
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
      const part = chunkOf(next.data);
      if (!part) throw new Error("remote_read_invalid_chunk");
      storeChunk(part, offset);
    }));
  }
  const data = await decodeJsonBytes<T>(bytes, isCurrent);
  return { ...response, data };
}
