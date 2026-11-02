import { decodeJsonBytes } from "@future-os/thread-projection";
import { invokeCommand } from "../tauri/invoke";

const MAX_BYTES = 16 * 1024 * 1024;
const CHUNK_BYTES = 64 * 1024;
/**
 * One admission attempt for an opportunistic read: an older page, a search
 * crawl or a remote reply -- all asked for on top of a transcript that is
 * already on screen. Only the newest page gets a longer window (see
 * TAIL_WAIT_MS); this constant is not the transfer lane's global bound.
 */
const WAIT_MS = 5_000;
/**
 * The newest page is the whole screen: until it lands the conversation reads
 * as empty, so waiting out the transfers already running beats refusing. It is
 * the only read that may exceed WAIT_MS.
 */
export const TAIL_WAIT_MS = 15_000;
/** A refused admission opened no transfer; the retry only re-queues. */
const RETRY_DELAY_MS = 200;
interface Transfer { id: string; totalBytes: number; chunkBytes: number }
let active = 0;
let waiting = 0;
async function admit(isCurrent: () => boolean, waitMs: number): Promise<() => void> {
  if (waiting >= 32)
    throw new Error("history_transfer_queue_full");
  waiting += 1;
  const deadline = Date.now() + waitMs;
  try {
    while (true) {
      if (active < 2)
        break;
      if (!isCurrent())
        throw new Error("stale_history_transfer");
      if (Date.now() >= deadline)
        throw new Error("history_transfer_admission_timeout");
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    if (!isCurrent())
      throw new Error("stale_history_transfer");
    active += 1;
  }
  finally { waiting -= 1; }
  return () => {
    active -= 1;
  };
}

/**
 * A time-out is the one refusal that leaves nothing behind: no transfer was
 * opened, so asking again costs a wait and nothing else. The slot usually frees
 * within a few seconds of the two running reads finishing.
 */
async function admitWithRetry(isCurrent: () => boolean, waitMs: number): Promise<() => void> {
  try {
    return await admit(isCurrent, waitMs);
  }
  catch (error) {
    if (!(error instanceof Error) || error.message !== "history_transfer_admission_timeout")
      throw error;
    await new Promise(resolve => setTimeout(resolve, RETRY_DELAY_MS));
    return await admit(isCurrent, WAIT_MS);
  }
}

/**
 * A first-paint read that spent the short window and kept going left the user
 * looking at an empty conversation, so record how long it took. Frontend
 * console is the only sink the WebView has: frontend capture is outside the
 * Rust diagnostics migration (docs/guide/logging.md). `warn` is the highest
 * severity the frontend lint allows for a diagnostic line.
 */
function noteBlockingWait(waitMs: number, waitedMs: number) {
  if (waitMs <= WAIT_MS || waitedMs < WAIT_MS)
    return;
  console.warn(`[history] blocking load waited≈${Math.round(waitedMs / 1000)}s`);
}

/**
 * Native IPC, including its fallback, parses only a bounded chunk. The full
 * object is composed cooperatively after the immutable transfer is complete.
 * Every refusal still rejects with its own reason: a read that cannot be
 * admitted must reach the transcript as a failed load, never as no history.
 */
export async function readHistoryTransfer<T>(
  args: Record<string, unknown>,
  isCurrent: () => boolean = () => true,
  waitMs = WAIT_MS,
): Promise<T> {
  const startedAt = Date.now();
  try {
    const release = await admitWithRetry(isCurrent, waitMs);
    try {
      const transfer = await invokeCommand<Transfer>("begin_session_history_transfer", args);
      return await read<T>(transfer, isCurrent);
    }
    finally { release(); }
  }
  finally { noteBlockingWait(waitMs, Date.now() - startedAt); }
}

/**
 * Remote read commands negotiate an inline reply or the same bounded local
 * transfer. Admission covers the network read through final JSON composition.
 */
export async function requestNativeRead<T>(command: string, args: Record<string, unknown>, isCurrent: () => boolean = () => true): Promise<T> {
  const release = await admitWithRetry(isCurrent, WAIT_MS);
  try {
    const result = await invokeCommand<unknown>(command, args);
    if (result && typeof result === "object" && "nativeReadTransfer" in result)
      return await read<T>(result.nativeReadTransfer as Transfer, isCurrent);
    if (!isCurrent())
      throw new Error("stale_history_transfer");
    return result as T;
  }
  finally { release(); }
}

async function read<T>(transfer: Transfer, isCurrent: () => boolean): Promise<T> {
  if (!transfer || typeof transfer.id !== "string" || !/^[\w-]{1,128}$/.test(transfer.id))
    throw new Error("invalid_history_transfer");
  try {
    if (!Number.isSafeInteger(transfer.totalBytes) || transfer.totalBytes <= 0
      || transfer.totalBytes > MAX_BYTES || transfer.chunkBytes !== CHUNK_BYTES) {
      throw new Error("invalid_history_transfer");
    }
    if (!isCurrent())
      throw new Error("stale_history_transfer");
    const bytes = new Uint8Array(transfer.totalBytes);
    for (let offset = 0; offset < transfer.totalBytes; offset += CHUNK_BYTES) {
      if (!isCurrent())
        throw new Error("stale_history_transfer");
      const raw = await invokeCommand<ArrayBuffer | number[]>("read_session_history_transfer", { transferId: transfer.id, offset });
      if (!isCurrent())
        throw new Error("stale_history_transfer");
      const length = Math.min(CHUNK_BYTES, transfer.totalBytes - offset);
      if (raw instanceof ArrayBuffer) {
        if (raw.byteLength !== length)
          throw new Error("invalid_history_transfer_chunk");
        bytes.set(new Uint8Array(raw), offset);
      }
      else if (Array.isArray(raw) && raw.length === length && raw.every(value => Number.isInteger(value) && value >= 0 && value <= 255)) {
        bytes.set(raw, offset);
      }
      else {
        throw new Error("invalid_history_transfer_chunk");
      }
    }
    return await decodeJsonBytes<T>(bytes, { isCurrent });
  }
  finally {
    // Completion already retires the server copy; release also covers failures.
    await invokeCommand("release_session_history_transfer", { transferId: transfer.id }).catch(() => {});
  }
}
