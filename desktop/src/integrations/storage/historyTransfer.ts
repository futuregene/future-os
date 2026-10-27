import { decodeJsonBytes } from "@future-os/thread-projection";
import { invokeCommand } from "../tauri/invoke";

const MAX_BYTES = 16 * 1024 * 1024;
const CHUNK_BYTES = 64 * 1024;
interface Transfer { id: string; totalBytes: number; chunkBytes: number }
let active = 0;
let waiting = 0;
async function admit(isCurrent: () => boolean): Promise<() => void> {
  if (waiting >= 32)
    throw new Error("history_transfer_queue_full");
  waiting += 1;
  const deadline = Date.now() + 5000;
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
 * Native IPC, including its fallback, parses only a bounded chunk. The full
 * object is composed cooperatively after the immutable transfer is complete.
 */
export async function readHistoryTransfer<T>(args: Record<string, unknown>, isCurrent: () => boolean = () => true): Promise<T> {
  const release = await admit(isCurrent);
  try {
    const transfer = await invokeCommand<Transfer>("begin_session_history_transfer", args);
    return await read<T>(transfer, isCurrent);
  }
  finally { release(); }
}

/**
 * Remote read commands negotiate an inline reply or the same bounded local
 * transfer. Admission covers the network read through final JSON composition.
 */
export async function requestNativeRead<T>(command: string, args: Record<string, unknown>, isCurrent: () => boolean = () => true): Promise<T> {
  const release = await admit(isCurrent);
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
