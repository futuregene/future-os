import type { DownloadInfo, HistoryAttachment, RemoteCommand, RpcResponse } from "./types";

/** A transfer never migrates between serving connections or bridge instances. */
export class TransferConnectionChangedError extends Error {
  constructor() {
    super("transfer_connection_changed");
    this.name = "TransferConnectionChangedError";
  }
}

export interface TransferConnection {
  check(): void;
  request<T>(command: RemoteCommand, sessionId?: string, timeoutMs?: number): Promise<RpcResponse<T>>;
  requestRetry<T>(command: RemoteCommand, sessionId?: string): Promise<RpcResponse<T>>;
  recoverAfterTransientRequest(error: unknown): Promise<boolean>;
  uploadChunk(transferId: string, index: number, bytes: Uint8Array): Promise<void>;
  downloadChunk(transferId: string, index: number): Promise<Uint8Array>;
}

export interface TransferClient {
  runTransfer<T>(operation: (signal: AbortSignal) => Promise<T>, signal?: AbortSignal): Promise<T>;
  captureTransfer(): TransferConnection;
}

export interface UploadAttachment {
  name: string;
  transferName?: string;
  mimeType: string;
  kind: "image" | "file";
  originalSize: number;
  transferSize: number;
}

export interface UploadReader {
  readonly offset: number | null;
  readBytes(count: number): Uint8Array;
  close(): void;
}

export interface UploadHost<Attachment extends UploadAttachment> {
  openUpload(attachment: Attachment): UploadReader;
}

/** File/cache/hash operations are supplied by the application or CLI host. */
export interface DownloadHost<File> {
  maxFileBytes: number;
  verifiedCached(info: DownloadInfo, signal?: AbortSignal): Promise<File | null>;
  createDownload(info: DownloadInfo): { file: File; handle: { writeBytes(bytes: Uint8Array): void; close(): void } };
  size(file: File): number;
  remove(file: File): void;
  hash(file: File, signal?: AbortSignal): Promise<string>;
}

// Preserve descriptor identity and wire DTOs while remembering the connection
// that prepared them. Weak keys do not keep application caches alive.
const preparedConnections = new WeakMap<DownloadInfo, { client: TransferClient; connection: TransferConnection }>();

interface UploadInit {
  uploadId: string;
  chunkBytes: number;
}

interface UploadComplete {
  uploadId: string;
  contentHash: string;
}

function cancelDownload(client: TransferConnection, transferId: string): void {
  // Cancellation is best-effort cleanup on the desktop. Never await it: when
  // the network is down, waiting for this command's timeout would keep the
  // phone's progress dialog and download lock stuck after the user cancelled.
  void client.request({ type: "download_cancel", transferId }, "transfer").catch(() => {});
}

const TRANSFER_RETRY_DELAYS_MS = [500, 1_000, 2_000, 4_000, 8_000, 15_000, 15_000];
// Metadata preparation has a different failure profile from byte chunks. One
// retry with the same command id is enough to retrieve a desktop singleflight
// result or recover a half-open socket; the second attempt gets extra time for
// an uncached image pipeline without multiplying silence for minutes.
const PREPARE_RPC_TIMEOUTS_MS = [10_000, 20_000] as const;
const PREPARE_RETRY_DELAY_MS = 500;

export class TransferCancelledError extends Error {
  constructor() {
    super("transfer_cancelled");
    this.name = "TransferCancelledError";
  }
}

export function throwIfCancelled(signal?: AbortSignal): void {
  if (signal?.aborted) throw new TransferCancelledError();
}

/** Race an in-flight NATS request with cancellation. NATS requests themselves
 * don't accept AbortSignal, so their transport timeout may finish later; this
 * wrapper releases the UI path immediately while still observing that late
 * result/rejection. */
function abortable<T>(operation: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (signal?.aborted) {
    void operation.catch(() => undefined);
    return Promise.reject(new TransferCancelledError());
  }
  if (!signal) return operation;
  return new Promise((resolve, reject) => {
    const onAbort = () => {
      cleanup();
      reject(new TransferCancelledError());
    };
    const cleanup = () => signal.removeEventListener("abort", onAbort);
    signal.addEventListener("abort", onAbort, { once: true });
    operation.then(
      value => {
        cleanup();
        resolve(value);
      },
      error => {
        cleanup();
        reject(error);
      },
    );
  });
}

function isPermanentTransferError(error: unknown): boolean {
  const message =
    error instanceof Error ? error.message.toLowerCase() : String(error).toLowerCase();
  return [
    "expired or does not exist",
    "outside the file",
    "unsupported download variant",
    "not an attachment",
    "no longer available",
    "larger than 10 mib",
    "transfer_connection_changed",
  ].some(fragment => message.includes(fragment));
}

function retryDelay(delayMs: number, signal?: AbortSignal): Promise<void> {
  throwIfCancelled(signal);
  return new Promise((resolve, reject) => {
    const jittered = Math.round(delayMs * (0.85 + Math.random() * 0.3));
    const onAbort = () => {
      clearTimeout(timer);
      reject(new TransferCancelledError());
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, jittered);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

async function withTransferRetry<T>(
  operation: () => Promise<T>,
  signal?: AbortSignal,
  onWaiting?: () => void,
  retryDelays = TRANSFER_RETRY_DELAYS_MS,
): Promise<T> {
  let lastError: unknown;
  for (let attempt = 0; attempt <= retryDelays.length; attempt += 1) {
    throwIfCancelled(signal);
    try {
      return await abortable(operation(), signal);
    } catch (error) {
      lastError = error;
      if (error instanceof TransferCancelledError || isPermanentTransferError(error)) throw error;
      const delay = retryDelays[attempt];
      if (delay === undefined) break;
      onWaiting?.();
      await retryDelay(delay, signal);
    }
  }
  throw lastError;
}

export function uploadAttachments<Attachment extends UploadAttachment>(
  client: TransferClient,
  attachments: Attachment[],
  host: UploadHost<Attachment>,
  onProgress?: (completedBytes: number, totalBytes: number) => void,
): Promise<Array<Attachment & UploadComplete>> {
  if (attachments.length === 0) return Promise.resolve([]);
  return client.runTransfer(signal => runUploadAttachments(client, attachments, host, signal, onProgress));
}

async function runUploadAttachments<Attachment extends UploadAttachment>(
  client: TransferClient,
  attachments: Attachment[],
  host: UploadHost<Attachment>,
  signal: AbortSignal,
  onProgress?: (completedBytes: number, totalBytes: number) => void,
): Promise<Array<Attachment & UploadComplete>> {
  if (attachments.length === 0) return [];
  throwIfCancelled(signal);
  const connection = client.captureTransfer();
  const totalBytes = attachments.reduce((sum, item) => sum + item.transferSize, 0);
  let completedBytes = 0;
  const uploaded: Array<Attachment & UploadComplete> = [];
  for (const attachment of attachments) {
    throwIfCancelled(signal);
    const init = await abortable(connection.request<UploadInit>(
      {
        type: "upload_init",
        name: attachment.name,
        transferName: attachment.transferName ?? attachment.name,
        mimeType: attachment.mimeType,
        kind: attachment.kind,
        originalSize: attachment.originalSize,
        transferSize: attachment.transferSize,
      },
      "transfer",
    ), signal);
    let handle: UploadReader | undefined;
    try {
      handle = host.openUpload(attachment);
      if (!Number.isSafeInteger(init.data.chunkBytes) || init.data.chunkBytes <= 0)
        throw new Error("invalid_upload_chunk_size");
      let index = 0;
      while ((handle.offset ?? 0) < attachment.transferSize) {
        throwIfCancelled(signal);
        const remaining = attachment.transferSize - (handle.offset ?? 0);
        const bytes = handle.readBytes(Math.min(init.data.chunkBytes, remaining));
        if (bytes.byteLength === 0)
          throw new Error("attachment_read_stalled");
        await withTransferRetry(
          () => connection.uploadChunk(init.data.uploadId, index, bytes),
          signal,
          undefined,
          [250, 500],
        );
        completedBytes += bytes.byteLength;
        onProgress?.(completedBytes, totalBytes);
        index += 1;
      }
      const completedHandle = handle;
      handle = undefined;
      completedHandle.close();
      throwIfCancelled(signal);
      const complete = await abortable(connection.requestRetry<UploadComplete>(
        { type: "upload_complete", transferId: init.data.uploadId },
        "transfer",
      ), signal);
      uploaded.push({
        ...attachment,
        uploadId: complete.data.uploadId,
        contentHash: complete.data.contentHash,
      });
    } catch (error) {
      await connection
        .request({ type: "upload_cancel", transferId: init.data.uploadId }, "transfer")
        .catch(() => {});
      throw error;
    } finally {
      handle?.close();
    }
  }
  return uploaded;
}

export function prepareDownload<File>(
  client: TransferClient,
  sessionId: string,
  attachment: HistoryAttachment,
  host: Pick<DownloadHost<File>, "verifiedCached">,
  variant: "preview" | "original" = "preview",
  signal?: AbortSignal,
  onWaiting?: () => void,
): Promise<DownloadInfo> {
  return client.runTransfer(signal => runPrepareDownload(client, sessionId, attachment, host, variant, signal, onWaiting), signal);
}

async function runPrepareDownload<File>(
  client: TransferClient,
  sessionId: string,
  attachment: HistoryAttachment,
  host: Pick<DownloadHost<File>, "verifiedCached">,
  variant: "preview" | "original" = "preview",
  signal?: AbortSignal,
  onWaiting?: () => void,
): Promise<DownloadInfo> {
  throwIfCancelled(signal);
  const connection = client.captureTransfer();
  const command = {
    id: `download_prepare_${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`,
    type: "download_prepare",
    sessionId,
    filePath: attachment.path,
    mode: variant,
    // The name this phone is already displaying (from the message metadata).
    // Without it the desktop has to read the session's whole history to look
    // the attachment up — megabytes of JSON per open, and once per inline
    // image in a Markdown preview — to re-derive a string we are sending here.
    name: attachment.name,
  };
  let attempt = 0;
  let response: RpcResponse<DownloadInfo> | null = null;
  for (const timeoutMs of PREPARE_RPC_TIMEOUTS_MS) {
    attempt += 1;
    try {
      const request = connection.request<DownloadInfo>(command, sessionId, timeoutMs);
      // Cancellation releases the UI immediately, but the desktop may finish
      // preparing later. Observe that reply and release its transfer as well.
      // This observer is owned separately: cancellation may finish the caller
      // before the SDK reply, whose late cleanup must still be awaited.
      void client.runTransfer(async () => {
        try {
          const result = await request;
          if (signal?.aborted) cancelDownload(connection, result.data.transferId);
        } catch { /* request failure is reported by the caller */ }
      });
      response = await abortable(request, signal);
      break;
    } catch (error) {
      if (error instanceof TransferCancelledError) throw error;
      if (attempt >= PREPARE_RPC_TIMEOUTS_MS.length) throw error;
      const retryable = await abortable(connection.recoverAfterTransientRequest(error), signal);
      if (!retryable) throw error;
      onWaiting?.();
      await retryDelay(PREPARE_RETRY_DELAY_MS, signal);
    }
  }
  if (!response) throw new Error("download_prepare_failed");
  // Older desktops do not return `variant`; their preview behavior remains
  // compatible, so normalize the response to the variant we requested.
  const info = response.data.variant ? response.data : { ...response.data, variant };
  if (signal?.aborted) {
    cancelDownload(connection, info.transferId);
    throw new TransferCancelledError();
  }
  try {
    const cached = await host.verifiedCached(info, signal);
    if (cached) {
      cancelDownload(connection, info.transferId);
    }
  } catch (error) {
    cancelDownload(connection, info.transferId);
    throw error;
  }
  preparedConnections.set(info, { client, connection });
  return info;
}

export function downloadPrepared<File>(
  client: TransferClient,
  info: DownloadInfo,
  host: DownloadHost<File>,
  onProgress?: (completedBytes: number, totalBytes: number) => void,
  signal?: AbortSignal,
  onWaiting?: () => void,
): Promise<File> {
  return client.runTransfer(signal => runDownloadPrepared(client, info, host, onProgress, signal, onWaiting), signal);
}

async function runDownloadPrepared<File>(
  client: TransferClient,
  info: DownloadInfo,
  host: DownloadHost<File>,
  onProgress?: (completedBytes: number, totalBytes: number) => void,
  signal?: AbortSignal,
  onWaiting?: () => void,
): Promise<File> {
  const prepared = preparedConnections.get(info);
  if (prepared && prepared.client !== client) throw new TransferConnectionChangedError();
  let connection = prepared?.connection;
  let captureError: unknown;
  if (!connection) {
    try { connection = client.captureTransfer(); }
    catch (error) { captureError = error; }
  }
  let file: File | undefined;
  let handle: ReturnType<DownloadHost<File>["createDownload"]>["handle"] | undefined;
  try {
    throwIfCancelled(signal);
    if (!Number.isSafeInteger(info.size) || info.size < 0 || info.size > host.maxFileBytes ||
        !Number.isSafeInteger(info.chunkBytes) || info.chunkBytes <= 0) {
      throw new Error("invalid_download_size");
    }
    const cached = await host.verifiedCached(info, signal);
    throwIfCancelled(signal);
    if (cached) return cached;
    if (!connection) throw captureError;
    connection.check();
    const owner = connection;
    ({ file, handle } = host.createDownload(info));
    const chunks = Math.ceil(info.size / info.chunkBytes);
    let completed = 0;
    // Bound both requests and retained bytes. Each full wave settles before
    // ordered writes, so cancellation cannot race writes into a removed file.
    const concurrency = Math.max(1, Math.min(4, Math.floor(2 * 1024 * 1024 / info.chunkBytes)));
    for (let start = 0; start < chunks; start += concurrency) {
      throwIfCancelled(signal);
      const wave = await Promise.all(Array.from({ length: Math.min(concurrency, chunks - start) }, (_, offset) =>
        withTransferRetry(() => owner.downloadChunk(info.transferId, start + offset), signal, onWaiting)
          .then(bytes => ({ bytes }), error => ({ error }))));
      throwIfCancelled(signal);
      connection.check();
      for (const result of wave) {
        if ("error" in result) throw result.error;
        const bytes = result.bytes;
        if (bytes.byteLength !== Math.min(info.chunkBytes, info.size - completed)) throw new Error("download_size_mismatch");
        handle.writeBytes(bytes);
        completed += bytes.byteLength;
        onProgress?.(completed, info.size);
      }
    }
    const completedHandle = handle;
    handle = undefined;
    completedHandle.close();
    throwIfCancelled(signal);
    if (host.size(file) !== info.size) throw new Error("download_size_mismatch");
    const hash = await host.hash(file, signal);
    throwIfCancelled(signal);
    connection.check();
    if (hash !== info.contentHash) throw new Error("download_hash_mismatch");
    return file;
  } catch (error) {
    try { handle?.close(); }
    finally { if (file !== undefined) host.remove(file); }
    throw error;
  } finally {
    if (connection) cancelDownload(connection, info.transferId);
  }
}
