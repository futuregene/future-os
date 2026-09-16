import { FileMode, type File } from "expo-file-system";
import { TransferCancelledError } from "../../remote/files";
import { MARKDOWN_RENDER_BYTES, plainText } from "./utils";

const READ_CHUNK_BYTES = 64 * 1024;
const YIELD_BYTES = 256 * 1024;

/** File.slice() currently calls bytesSync() in Expo, so it is not a bounded
 * read. Use a read-only handle and yield between small groups of native reads.
 * Never bring a whole 10-MiB attachment into JS just to show its first 2 MiB. */
export async function readPreviewText(file: File, signal?: AbortSignal): Promise<{
  text: string;
  truncated: boolean;
} | null> {
  const check = () => { if (signal?.aborted) throw new TransferCancelledError(); };
  check();
  const size = file.size;
  if (!Number.isSafeInteger(size) || size < 0) throw new Error("invalid_preview_size");
  const length = Math.min(size, MARKDOWN_RENDER_BYTES);
  const bytes = new Uint8Array(length);
  const handle = file.open(FileMode.ReadOnly);
  try {
    let offset = 0;
    let sinceYield = 0;
    while (offset < length) {
      check();
      const requested = Math.min(READ_CHUNK_BYTES, length - offset);
      const part = handle.readBytes(requested);
      if (part.length === 0 || part.length > requested) throw new Error("preview_read_size_mismatch");
      bytes.set(part, offset);
      offset += part.length;
      sinceYield += part.length;
      if (offset < length && sinceYield >= YIELD_BYTES) {
        await new Promise<void>(resolve => setTimeout(resolve, 0));
        sinceYield = 0;
      }
    }
    check();
    const truncated = size > length;
    // A byte limit can split a valid UTF-8 code point. Validation drops only
    // that incomplete trailing code point; invalid interior data fails.
    const text = plainText(bytes, truncated);
    return text === null ? null : { text, truncated };
  } finally { handle.close(); }
}
