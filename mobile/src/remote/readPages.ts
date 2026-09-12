import type { RemoteClient } from "./client";
import { decodeBase64Url } from "./codec";
import type { RemoteCommand, RpcResponse } from "./types";

const MAX_READ_BYTES = 16 * 1024 * 1024;
const CHUNK_BYTES = 192 * 1024;
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

/** Reassemble only negotiated history/replay reads. Each NATS reply remains
 * small; message grouping and projection cursors are interpreted only after the
 * immutable JSON snapshot is complete. Old desktops ignore chunkedRead.
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
  let chunk = chunkOf(response.data);
  if (!chunk) return response as RpcResponse<T>;
  const id = chunk.id;
  const total = chunk.totalBytes;
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (;;) {
    if (chunk.id !== id || chunk.totalBytes !== total || chunk.offset !== offset)
      throw new Error("remote_read_chunk_changed");
    const part = decodeBase64Url(chunk.data);
    if (!part || part.length === 0 || part.length > CHUNK_BYTES
      || chunk.nextOffset !== offset + part.length || chunk.nextOffset > total
      || (chunk.nextOffset < total && part.length !== CHUNK_BYTES))
      throw new Error("remote_read_invalid_chunk");
    bytes.set(part, offset);
    offset = chunk.nextOffset;
    if (offset === total) break;
    if (!isCurrent()) throw new Error("stale_sync_lane");
    const next = await client.requestRetry<unknown>({
      type: "get_read_chunk", sessionId, replyId: id, offset,
      ...(command.runId ? { runId: command.runId } : {}),
      ...(command.bridgeInstanceId ? { bridgeInstanceId: command.bridgeInstanceId } : {}),
    }, sessionId);
    if (!isCurrent()) throw new Error("stale_sync_lane");
    chunk = chunkOf(next.data);
    if (!chunk) throw new Error("remote_read_invalid_chunk");
  }
  const data = JSON.parse(new TextDecoder().decode(bytes)) as T;
  return { ...response, data };
}
