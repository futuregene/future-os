import {
  downloadPrepared, prepareDownload, uploadAttachments, TransferCancelledError,
  TransferConnectionChangedError, type DownloadHost, type TransferConnection,
} from "../src/transfer";
import type { DownloadInfo } from "../src/types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
const metadata = (overrides: Partial<DownloadInfo> = {}): DownloadInfo => ({
  transferId: "download", name: "a.txt", mimeType: "text/plain", size: 6,
  contentHash: "expected", previewKind: "text", variant: "preview", chunkBytes: 2, ...overrides,
});
const attachment = () => ({
  name: "a.txt", mimeType: "text/plain", kind: "file" as const,
  originalSize: 6, transferSize: 6, localToken: "host-owned",
});

function fixture() {
  let current = true;
  const request = jest.fn().mockResolvedValue({ success: true, data: { uploadId: "upload", chunkBytes: 2 } });
  const requestRetry = jest.fn().mockResolvedValue({ success: true, data: { uploadId: "upload", contentHash: "hash" } });
  const recover = jest.fn().mockResolvedValue(false);
  const uploadChunk = jest.fn().mockResolvedValue(undefined);
  const downloadChunk = jest.fn(async (_id: string, index: number): Promise<Uint8Array> => new Uint8Array([index * 2, index * 2 + 1]));
  const check = () => { if (!current) throw new TransferConnectionChangedError(); };
  const connection: TransferConnection = {
    check,
    request: async (...args) => { check(); return request(...args); },
    requestRetry: async (...args) => { check(); return requestRetry(...args); },
    recoverAfterTransientRequest: async error => { check(); return recover(error); },
    uploadChunk: async (...args) => { check(); return uploadChunk(...args); },
    downloadChunk: async (...args) => { check(); return downloadChunk(...args); },
  };
  const client = {
    runTransfer: <T>(operation: (signal: AbortSignal) => Promise<T>, signal?: AbortSignal) =>
      operation(signal ?? new AbortController().signal),
    captureTransfer: jest.fn(() => connection),
  };
  const reader = {
    offset: 0,
    readBytes: jest.fn((count: number): Uint8Array => {
      const bytes = new Uint8Array(Array.from({ length: count }, (_, index) => reader.offset + index));
      reader.offset += count;
      return bytes;
    }),
    close: jest.fn(),
  };
  const file = { bytes: [] as number[], removed: false };
  const writer = {
    writeBytes: jest.fn((bytes: Uint8Array) => { file.bytes.push(...bytes); }), close: jest.fn(),
  };
  const verifiedCached = jest.fn().mockResolvedValue(null);
  const createDownload = jest.fn(() => ({ file, handle: writer }));
  const hash = jest.fn().mockResolvedValue("expected");
  const remove = jest.fn(() => { file.removed = true; file.bytes = []; });
  const host: DownloadHost<typeof file> = {
    maxFileBytes: 100, verifiedCached, createDownload,
    size: value => value.bytes.length, remove, hash,
  };
  return {
    client, connection, request, requestRetry, recover, uploadChunk, downloadChunk,
    reader, uploadHost: { openUpload: jest.fn(() => reader) },
    file, writer, host, verifiedCached, createDownload, hash, remove,
    retire: () => { current = false; },
  };
}

afterEach(() => { jest.useRealTimers(); });

test("upload preserves host metadata, ordered indexed bytes and cumulative progress", async () => {
  const f = fixture();
  const progress = jest.fn();
  const item = attachment();
  const uploaded = await uploadAttachments(f.client, [item], f.uploadHost, progress);
  expect(uploaded).toEqual([{ ...item, uploadId: "upload", contentHash: "hash" }]);
  expect(f.uploadChunk.mock.calls).toEqual([
    ["upload", 0, new Uint8Array([0, 1])], ["upload", 1, new Uint8Array([2, 3])],
    ["upload", 2, new Uint8Array([4, 5])],
  ]);
  expect(progress.mock.calls).toEqual([[2, 6], [4, 6], [6, 6]]);
  expect(f.reader.close).toHaveBeenCalledTimes(1);
  expect(f.requestRetry).toHaveBeenCalledWith({ type: "upload_complete", transferId: "upload" }, "transfer");
});

test.each([0, -1, NaN, 1.5])("invalid upload chunk size %s closes its host reader and cancels", async chunkBytes => {
  const f = fixture();
  f.request.mockResolvedValueOnce({ data: { uploadId: "upload", chunkBytes } });
  await expect(uploadAttachments(f.client, [attachment()], f.uploadHost)).rejects.toThrow("invalid_upload_chunk_size");
  expect(f.reader.close).toHaveBeenCalledTimes(1);
  expect(f.uploadChunk).not.toHaveBeenCalled();
  expect(f.request).toHaveBeenLastCalledWith({ type: "upload_cancel", transferId: "upload" }, "transfer");
  expect(f.requestRetry).not.toHaveBeenCalled();
});

test("a stalled host read cancels without completing the upload", async () => {
  const f = fixture();
  f.reader.readBytes.mockReturnValue(new Uint8Array());
  await expect(uploadAttachments(f.client, [attachment()], f.uploadHost)).rejects.toThrow("attachment_read_stalled");
  expect(f.reader.close).toHaveBeenCalledTimes(1);
  expect(f.request).toHaveBeenLastCalledWith({ type: "upload_cancel", transferId: "upload" }, "transfer");
  expect(f.requestRetry).not.toHaveBeenCalled();
});

test("opening a host reader can fail after init without leaving an unowned upload", async () => {
  const f = fixture();
  f.uploadHost.openUpload.mockImplementation(() => { throw new Error("permission_denied"); });
  await expect(uploadAttachments(f.client, [attachment()], f.uploadHost)).rejects.toThrow("permission_denied");
  expect(f.request).toHaveBeenLastCalledWith({ type: "upload_cancel", transferId: "upload" }, "transfer");
  expect(f.reader.close).not.toHaveBeenCalled();
});

test("upload chunk retries retain the same id, index and bytes", async () => {
  jest.useFakeTimers();
  const f = fixture();
  f.uploadChunk.mockRejectedValueOnce(new Error("timeout"));
  const operation = uploadAttachments(f.client, [attachment()], f.uploadHost);
  await jest.runAllTimersAsync();
  await operation;
  expect(f.uploadChunk.mock.calls[0]).toEqual(f.uploadChunk.mock.calls[1]);
  expect(f.uploadChunk).toHaveBeenCalledTimes(4);
});

test("retired upload connection is never replaced for retries or cleanup", async () => {
  const f = fixture();
  f.uploadChunk.mockImplementation(async () => { f.retire(); throw new TransferConnectionChangedError(); });
  await expect(uploadAttachments(f.client, [attachment()], f.uploadHost)).rejects.toThrow("transfer_connection_changed");
  expect(f.client.captureTransfer).toHaveBeenCalledTimes(1);
  expect(f.uploadChunk).toHaveBeenCalledTimes(1);
  expect(f.request).toHaveBeenCalledTimes(1); // init only; no cancel on a replacement
  expect(f.reader.close).toHaveBeenCalledTimes(1);
});

test("prepare retry uses a stable command id and the two legacy timeout budgets", async () => {
  jest.useFakeTimers();
  const f = fixture();
  const info = metadata();
  f.request.mockRejectedValueOnce(new Error("timeout")).mockResolvedValueOnce({ data: info });
  f.recover.mockResolvedValue(true);
  const operation = prepareDownload(f.client, "session", { path: "a.txt", name: "a.txt" }, f.host);
  await jest.runAllTimersAsync();
  expect(await operation).toBe(info);
  expect(f.request.mock.calls[0]?.[0]).toEqual(f.request.mock.calls[1]?.[0]);
  expect(f.request.mock.calls.map(call => call[2])).toEqual([10_000, 20_000]);
  expect(f.recover).toHaveBeenCalledTimes(1);
});

test("prepare preserves a business failure without another request", async () => {
  const f = fixture();
  f.request.mockRejectedValueOnce(new Error("not an attachment"));
  await expect(prepareDownload(f.client, "session", { path: "a", name: "a" }, f.host)).rejects.toThrow("not an attachment");
  expect(f.request).toHaveBeenCalledTimes(1);
});

test("cancelled prepare observes a late successful reply and releases its transfer", async () => {
  const f = fixture();
  const pending = deferred<{ data: DownloadInfo }>();
  f.request.mockReturnValueOnce(pending.promise);
  const controller = new AbortController();
  const operation = prepareDownload(f.client, "session", { path: "a", name: "a" }, f.host, "preview", controller.signal);
  controller.abort();
  await expect(operation).rejects.toBeInstanceOf(TransferCancelledError);
  pending.resolve({ data: metadata() });
  await settle();
  expect(f.request).toHaveBeenLastCalledWith({ type: "download_cancel", transferId: "download" }, "transfer");
  expect(f.verifiedCached).not.toHaveBeenCalled();
});

test("prepared transfer identity stays bound across separate prepare and download calls", async () => {
  const f = fixture();
  const info = metadata();
  f.request.mockResolvedValueOnce({ data: info });
  expect(await prepareDownload(f.client, "session", { path: "a", name: "a" }, f.host)).toBe(info);
  f.retire();
  await expect(downloadPrepared(f.client, info, f.host)).rejects.toThrow("transfer_connection_changed");
  expect(f.client.captureTransfer).toHaveBeenCalledTimes(1);
  expect(f.createDownload).not.toHaveBeenCalled();
  expect(f.downloadChunk).not.toHaveBeenCalled();
});

test("prepared descriptor cannot be consumed by another client instance", async () => {
  const f = fixture();
  const other = fixture();
  const info = metadata();
  f.request.mockResolvedValueOnce({ data: info });
  await prepareDownload(f.client, "session", { path: "a", name: "a" }, f.host);
  await expect(downloadPrepared(other.client, info, other.host)).rejects.toThrow("transfer_connection_changed");
  expect(other.client.captureTransfer).not.toHaveBeenCalled();
  expect(other.downloadChunk).not.toHaveBeenCalled();
});

test("download bounds requests to four and writes out-of-order replies in index order", async () => {
  const f = fixture();
  const pending = Array.from({ length: 5 }, () => deferred<Uint8Array>());
  f.downloadChunk.mockImplementation((_id, index) => pending[index]!.promise);
  const progress = jest.fn();
  const operation = downloadPrepared(f.client, metadata({ size: 10 }), f.host, progress);
  await settle();
  expect(f.downloadChunk.mock.calls.map(call => call[1])).toEqual([0, 1, 2, 3]);
  for (const index of [3, 2, 1, 0]) pending[index]!.resolve(new Uint8Array([index * 2, index * 2 + 1]));
  await settle();
  expect(f.file.bytes).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  expect(f.downloadChunk.mock.calls.map(call => call[1])).toEqual([0, 1, 2, 3, 4]);
  pending[4]!.resolve(new Uint8Array([8, 9]));
  expect(await operation).toBe(f.file);
  expect(f.file.bytes).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
  expect(progress).toHaveBeenLastCalledWith(10, 10);
  expect(f.writer.close).toHaveBeenCalledTimes(1);
  expect(f.request).toHaveBeenLastCalledWith({ type: "download_cancel", transferId: "download" }, "transfer");
});

test("download retained-byte budget reduces concurrency for large chunks", async () => {
  const f = fixture();
  const pending = deferred<Uint8Array>();
  f.host.maxFileBytes = 4 * 1024 * 1024;
  f.downloadChunk.mockReturnValue(pending.promise);
  const controller = new AbortController();
  const operation = downloadPrepared(f.client, metadata({ size: 4 * 1024 * 1024, chunkBytes: 2 * 1024 * 1024 }), f.host, undefined, controller.signal);
  await settle();
  expect(f.downloadChunk).toHaveBeenCalledTimes(1);
  controller.abort();
  await expect(operation).rejects.toBeInstanceOf(TransferCancelledError);
  expect(f.remove).toHaveBeenCalledTimes(1);
  expect(f.writer.writeBytes).not.toHaveBeenCalled();
  pending.resolve(new Uint8Array());
});

test.each([
  { size: -1 }, { size: 101 }, { size: 1.5 }, { chunkBytes: 0 }, { chunkBytes: NaN },
])("invalid download metadata %j never opens a local file", async invalid => {
  const f = fixture();
  await expect(downloadPrepared(f.client, metadata(invalid), f.host)).rejects.toThrow("invalid_download_size");
  expect(f.createDownload).not.toHaveBeenCalled();
  expect(f.downloadChunk).not.toHaveBeenCalled();
  expect(f.request).toHaveBeenLastCalledWith({ type: "download_cancel", transferId: "download" }, "transfer");
});

test("verified cache hit releases remote bytes without opening a writer", async () => {
  const f = fixture();
  f.verifiedCached.mockResolvedValue(f.file);
  expect(await downloadPrepared(f.client, metadata(), f.host)).toBe(f.file);
  expect(f.createDownload).not.toHaveBeenCalled();
  expect(f.downloadChunk).not.toHaveBeenCalled();
  expect(f.request).toHaveBeenLastCalledWith({ type: "download_cancel", transferId: "download" }, "transfer");
});

test("short indexed chunk removes the partial download without hashing", async () => {
  const f = fixture();
  f.downloadChunk.mockResolvedValueOnce(new Uint8Array([1]));
  await expect(downloadPrepared(f.client, metadata(), f.host)).rejects.toThrow("download_size_mismatch");
  expect(f.writer.close).toHaveBeenCalledTimes(1);
  expect(f.remove).toHaveBeenCalledTimes(1);
  expect(f.hash).not.toHaveBeenCalled();
});

test("digest mismatch removes bytes and releases the server transfer", async () => {
  const f = fixture();
  f.hash.mockResolvedValue("wrong");
  await expect(downloadPrepared(f.client, metadata(), f.host)).rejects.toThrow("download_hash_mismatch");
  expect(f.writer.close).toHaveBeenCalledTimes(1);
  expect(f.remove).toHaveBeenCalledTimes(1);
  expect(f.request).toHaveBeenLastCalledWith({ type: "download_cancel", transferId: "download" }, "transfer");
});

test("cancelled in-flight wave cannot write late chunks to a removed file", async () => {
  const f = fixture();
  const pending = deferred<Uint8Array>();
  f.downloadChunk.mockReturnValue(pending.promise);
  const controller = new AbortController();
  const operation = downloadPrepared(f.client, metadata(), f.host, undefined, controller.signal);
  await settle();
  controller.abort();
  await expect(operation).rejects.toBeInstanceOf(TransferCancelledError);
  pending.resolve(new Uint8Array([0, 1]));
  await settle();
  expect(f.writer.writeBytes).not.toHaveBeenCalled();
  expect(f.writer.close).toHaveBeenCalledTimes(1);
  expect(f.remove).toHaveBeenCalledTimes(1);
});

test("connection retired during an in-flight wave cannot publish bytes or cancel on a replacement", async () => {
  const f = fixture();
  const pending = deferred<Uint8Array>();
  f.downloadChunk.mockReturnValue(pending.promise);
  const operation = downloadPrepared(f.client, metadata(), f.host);
  await settle();
  f.retire();
  pending.resolve(new Uint8Array([0, 1]));
  await expect(operation).rejects.toThrow("transfer_connection_changed");
  expect(f.writer.writeBytes).not.toHaveBeenCalled();
  expect(f.remove).toHaveBeenCalledTimes(1);
  expect(f.request).not.toHaveBeenCalled();
});

test("empty upload batch does not require a serving connection", async () => {
  const f = fixture();
  expect(await uploadAttachments(f.client, [], f.uploadHost)).toEqual([]);
  expect(f.client.captureTransfer).not.toHaveBeenCalled();
  expect(f.request).not.toHaveBeenCalled();
});

test("verified cached bytes remain usable when there is no live connection to capture", async () => {
  const f = fixture();
  f.client.captureTransfer.mockImplementation(() => { throw new Error("communication_frozen"); });
  f.verifiedCached.mockResolvedValue(f.file);
  expect(await downloadPrepared(f.client, metadata(), f.host)).toBe(f.file);
  expect(f.downloadChunk).not.toHaveBeenCalled();
  expect(f.createDownload).not.toHaveBeenCalled();
});

test("cache miss preserves the connection error without opening a native writer", async () => {
  const f = fixture();
  f.client.captureTransfer.mockImplementation(() => { throw new Error("communication_frozen"); });
  await expect(downloadPrepared(f.client, metadata(), f.host)).rejects.toThrow("communication_frozen");
  expect(f.createDownload).not.toHaveBeenCalled();
  expect(f.request).not.toHaveBeenCalled();
});

test("failure to create host output still releases the prepared server transfer", async () => {
  const f = fixture();
  f.createDownload.mockImplementation(() => { throw new Error("disk_full"); });
  await expect(downloadPrepared(f.client, metadata(), f.host)).rejects.toThrow("disk_full");
  expect(f.request).toHaveBeenLastCalledWith({ type: "download_cancel", transferId: "download" }, "transfer");
});

test("native hash completing after retirement cannot finish an old download", async () => {
  const f = fixture();
  const hash = deferred<string>();
  f.hash.mockReturnValue(hash.promise);
  const operation = downloadPrepared(f.client, metadata(), f.host);
  await settle();
  expect(f.hash).toHaveBeenCalledTimes(1);
  f.retire();
  hash.resolve("expected");
  await expect(operation).rejects.toThrow("transfer_connection_changed");
  expect(f.remove).toHaveBeenCalledTimes(1);
  expect(f.request).not.toHaveBeenCalled();
});

test("upload completion failure still closes local IO and cancels the same server transfer", async () => {
  const f = fixture();
  f.requestRetry.mockRejectedValue(new Error("completion_failed"));
  await expect(uploadAttachments(f.client, [attachment()], f.uploadHost)).rejects.toThrow("completion_failed");
  expect(f.reader.close).toHaveBeenCalledTimes(1);
  expect(f.request).toHaveBeenLastCalledWith({ type: "upload_cancel", transferId: "upload" }, "transfer");
});
