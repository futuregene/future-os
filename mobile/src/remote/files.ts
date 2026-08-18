import { File, FileMode, Directory, Paths } from "expo-file-system";
import * as ImageManipulator from "expo-image-manipulator";
import * as ImagePicker from "expo-image-picker";
import * as Crypto from "expo-crypto";
import { Image } from "react-native";
import type { RemoteClient } from "./client";
import type { DownloadInfo, HistoryAttachment, MobileAttachment } from "./types";
import { mobileFileType } from "./fileTypes";
import { basename } from "./localPath";

export const MAX_FILE_BYTES = 10 * 1024 * 1024;
export const MAX_MESSAGE_BYTES = 20 * 1024 * 1024;
export const MAX_ATTACHMENTS = 10;
export const MAX_IMAGES = 4;
// Longest-edge cap for image attachments. Images over this are downsampled
// (never rejected) so every source — camera, album, screenshot — behaves the
// same. 1600px keeps documents/screenshots readable and the transfer small
// (report 06 decision D1).
export const MAX_IMAGE_EDGE = 1600;
const MAX_PREVIEW_CACHE_BYTES = 100 * 1024 * 1024;
const preparedDownloadIndex = new Map<string, DownloadInfo>();

const IMAGE_EXTENSIONS = new Set(["jpg", "jpeg", "png", "gif", "webp", "bmp", "heic", "heif"]);
const JPEG_OUTPUT_INPUTS = new Set(["jpg", "jpeg", "bmp", "heic", "heif"]);
const SUPPORTED_IMAGE_EXTENSIONS = new Set(IMAGE_EXTENSIONS);
const UNSUPPORTED_IMAGE_EXTENSIONS = new Set([
  "avif",
  "tif",
  "tiff",
  "jxl",
  "ico",
  "dng",
  "cr2",
  "cr3",
  "nef",
  "arw",
  "orf",
  "rw2",
]);

function extension(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function withoutExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

function imageSize(uri: string): Promise<{ width: number; height: number }> {
  return new Promise((resolve, reject) => {
    Image.getSize(uri, (width, height) => resolve({ width, height }), reject);
  });
}

export function mimeFor(name: string, fallback = "application/octet-stream"): string {
  return mobileFileType(name)?.mimeType ?? fallback;
}

function isImage(file: File, mimeType?: string | null): boolean {
  const ext = extension(file.name);
  const mime = mimeType?.toLowerCase();
  if (ext === "svg" || mime === "image/svg+xml") return false;
  return (
    !!mime?.startsWith("image/") ||
    IMAGE_EXTENSIONS.has(ext) ||
    UNSUPPORTED_IMAGE_EXTENSIONS.has(ext)
  );
}

function imageFormat(file: File, mimeType?: string | null): string | null {
  const ext = extension(file.name);
  if (SUPPORTED_IMAGE_EXTENSIONS.has(ext)) return ext;
  switch (mimeType?.toLowerCase()) {
    case "image/jpeg":
      return "jpeg";
    case "image/png":
      return "png";
    case "image/gif":
      return "gif";
    case "image/webp":
      return "webp";
    case "image/bmp":
      return "bmp";
    case "image/heic":
    case "image/heic-sequence":
      return "heic";
    case "image/heif":
    case "image/heif-sequence":
      return "heif";
    default:
      return null;
  }
}

function ascii(bytes: Uint8Array, offset: number, length: number): string {
  return String.fromCharCode(...bytes.slice(offset, offset + length));
}

function isAnimatedPng(file: File): boolean {
  const handle = file.open(FileMode.ReadOnly);
  try {
    const signature = handle.readBytes(8);
    if (
      signature.byteLength !== 8 ||
      ![137, 80, 78, 71, 13, 10, 26, 10].every((byte, index) => signature[index] === byte)
    ) {
      return false;
    }
    while ((handle.offset ?? file.size) + 8 <= file.size) {
      const header = handle.readBytes(8);
      if (header.byteLength !== 8) return false;
      const length =
        ((header[0]! << 24) | (header[1]! << 16) | (header[2]! << 8) | header[3]!) >>> 0;
      const kind = ascii(header, 4, 4);
      if (kind === "acTL") return true;
      if (kind === "IDAT" || kind === "IEND") return false;
      const nextOffset = (handle.offset ?? 0) + length + 4;
      if (nextOffset > file.size) return false;
      handle.offset = nextOffset;
    }
    return false;
  } finally {
    handle.close();
  }
}

function isAnimatedWebp(file: File): boolean {
  const handle = file.open(FileMode.ReadOnly);
  try {
    const header = handle.readBytes(Math.min(21, file.size));
    return (
      header.byteLength >= 21 &&
      ascii(header, 0, 4) === "RIFF" &&
      ascii(header, 8, 4) === "WEBP" &&
      ascii(header, 12, 4) === "VP8X" &&
      (header[20]! & 0x02) !== 0
    );
  } finally {
    handle.close();
  }
}

function mobilePreviewUnsupported(file: File, format: string): boolean {
  if (format === "gif") return true;
  if (format === "png") return isAnimatedPng(file);
  if (format === "webp") return isAnimatedWebp(file);
  return false;
}

function validateRawSelection(
  existing: MobileAttachment[],
  files: { file: File; mimeType?: string | null }[],
): void {
  if (existing.length + files.length > MAX_ATTACHMENTS) throw new Error("attachment_count");
  const imageCount =
    existing.filter(item => item.kind === "image").length +
    files.filter(item => isImage(item.file, item.mimeType)).length;
  if (imageCount > MAX_IMAGES) throw new Error("attachment_image_count");
  let total = existing.reduce((sum, item) => sum + item.originalSize, 0);
  for (const { file } of files) {
    if (file.size <= 0 || file.size > MAX_FILE_BYTES) {
      throw new Error("attachment_file_too_large");
    }
    total += file.size;
  }
  if (total > MAX_MESSAGE_BYTES) throw new Error("attachment_total_size");
}

async function prepareFile(
  file: File,
  mimeType?: string | null,
  forceJpeg = false,
): Promise<MobileAttachment> {
  // Both callers (pickAttachments/takePhoto) run validateRawSelection first,
  // which rejects empty/oversized files, so no size re-check is needed here.
  const originalSize = file.size;
  const kind = isImage(file, mimeType) ? "image" : "file";
  if (kind === "file") {
    return {
      localUri: file.uri,
      name: file.name,
      mimeType: mimeType || mimeFor(file.name),
      kind,
      originalSize,
      transferSize: originalSize,
    };
  }

  const format = imageFormat(file, mimeType);
  if (!format) throw new Error("attachment_image_format");

  let dimensions: { width: number; height: number };
  try {
    dimensions = await imageSize(file.uri);
  } catch {
    throw new Error("attachment_image_decode");
  }
  const oversized = Math.max(dimensions.width, dimensions.height) > MAX_IMAGE_EDGE;
  const previewUnsupported = mobilePreviewUnsupported(file, format);
  // A small source in a non-JPEG format passes through untouched (animated GIF
  // must stay GIF); anything that needs re-encoding — oversized (downsampled to
  // the 1600px cap), or a JPEG-adjacent input — goes through the converter.
  if (!oversized && !forceJpeg && !JPEG_OUTPUT_INPUTS.has(format)) {
    return {
      localUri: file.uri,
      name: file.name,
      mimeType: mimeType || mimeFor(file.name),
      kind,
      originalSize,
      transferSize: originalSize,
      mobilePreviewUnsupported: previewUnsupported,
    };
  }

  const resizeActions: ImageManipulator.Action[] = oversized
    ? [
        {
          // Preserve aspect ratio: only the longer edge is capped, the other
          // edge scales proportionally.
          resize: {
            width:
              dimensions.width >= dimensions.height
                ? MAX_IMAGE_EDGE
                : Math.round((dimensions.width / dimensions.height) * MAX_IMAGE_EDGE),
            height:
              dimensions.height > dimensions.width
                ? MAX_IMAGE_EDGE
                : Math.round((dimensions.height / dimensions.width) * MAX_IMAGE_EDGE),
          },
        },
      ]
    : [];
  let converted: ImageManipulator.ImageResult;
  try {
    converted = await ImageManipulator.manipulateAsync(file.uri, resizeActions, {
      compress: 0.65,
      format: ImageManipulator.SaveFormat.JPEG,
    });
  } catch {
    throw new Error("attachment_image_decode");
  }
  const transfer = new File(converted.uri);
  if (transfer.size <= 0 || transfer.size > MAX_FILE_BYTES) {
    throw new Error("attachment_compressed_too_large");
  }
  return {
    localUri: transfer.uri,
    name: file.name,
    transferName: `${withoutExtension(file.name) || "image"}.jpg`,
    mimeType: "image/jpeg",
    kind,
    originalSize,
    transferSize: transfer.size,
    temporary: true,
  };
}

export function deleteTemporaryAttachment(attachment: MobileAttachment): void {
  if (!attachment.temporary) return;
  const file = new File(attachment.localUri);
  if (file.exists) file.delete();
}

function validateBatch(items: MobileAttachment[]): void {
  if (items.length > MAX_ATTACHMENTS) throw new Error("attachment_count");
  if (items.filter(item => item.kind === "image").length > MAX_IMAGES) {
    throw new Error("attachment_image_count");
  }
  const total = items.reduce((sum, item) => sum + item.originalSize, 0);
  if (total > MAX_MESSAGE_BYTES) throw new Error("attachment_total_size");
}

export async function pickAttachments(existing: MobileAttachment[]): Promise<MobileAttachment[]> {
  // The iOS native module expects an array here. Passing a scalar is accepted
  // by the TypeScript surface but can be marshalled as an invalid Record on
  // some Expo native builds, which makes the picker dismiss immediately.
  const result = await File.pickFileAsync({ multipleFiles: true, mimeTypes: ["*/*"] });
  if (result.canceled) return existing;
  const selected = result.result.map(file => ({ file, mimeType: file.type }));
  // Quotas are intentionally checked against original bytes before any image
  // decode/re-encode. This keeps the product rule visible and avoids burning
  // phone CPU on a batch that cannot be sent.
  validateRawSelection(existing, selected);
  const prepared = await Promise.all(
    selected.map(({ file, mimeType }) => prepareFile(file, mimeType)),
  );
  const combined = [...existing, ...prepared];
  validateBatch(combined);
  return combined;
}

export async function takePhoto(existing: MobileAttachment[]): Promise<MobileAttachment[]> {
  const permission = await ImagePicker.requestCameraPermissionsAsync();
  if (!permission.granted) throw new Error("attachment_camera_permission");
  const result = await ImagePicker.launchCameraAsync({
    mediaTypes: ["images"],
    quality: 1,
    exif: false,
  });
  if (result.canceled || !result.assets[0]) return existing;
  const asset = result.assets[0];
  const selected = { file: new File(asset.uri), mimeType: asset.mimeType ?? "image/jpeg" };
  validateRawSelection(existing, [selected]);
  const prepared = await prepareFile(selected.file, selected.mimeType, true);
  const cameraAttachment = prepared.transferName
    ? { ...prepared, name: prepared.transferName }
    : prepared;
  const combined = [...existing, cameraAttachment];
  validateBatch(combined);
  return combined;
}

async function prepareImagePickerAssets(
  existing: MobileAttachment[],
  assets: ImagePicker.ImagePickerAsset[],
): Promise<MobileAttachment[]> {
  const selected = assets.map(asset => ({
    file: new File(asset.uri),
    mimeType: asset.mimeType ?? "image/jpeg",
  }));
  validateRawSelection(existing, selected);
  const prepared = await Promise.all(
    selected.map(({ file, mimeType }) => prepareFile(file, mimeType)),
  );
  const combined = [...existing, ...prepared];
  validateBatch(combined);
  return combined;
}

export async function pickFromAlbum(existing: MobileAttachment[]): Promise<MobileAttachment[]> {
  const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
  if (!permission.granted) throw new Error("attachment_album_permission");
  const result = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: ["images"],
    allowsMultipleSelection: true,
    quality: 1,
    exif: false,
  });
  if (result.canceled || result.assets.length === 0) return existing;
  return prepareImagePickerAssets(existing, result.assets);
}

/** Recover a camera/library result after Android reconstructs MainActivity. */
export async function recoverPendingImagePickerAttachments(
  existing: MobileAttachment[],
): Promise<MobileAttachment[]> {
  const result = await ImagePicker.getPendingResultAsync();
  if (!result) return existing;
  if ("code" in result) throw new Error("attachment_failed");
  if (result.canceled || result.assets.length === 0) return existing;
  return prepareImagePickerAssets(existing, result.assets);
}

interface UploadInit {
  uploadId: string;
  chunkBytes: number;
}

interface UploadComplete {
  uploadId: string;
  contentHash: string;
}

function cancelDownload(client: RemoteClient, transferId: string): void {
  // Cancellation is best-effort cleanup on the desktop. Never await it: when
  // the network is down, waiting for this command's timeout would keep the
  // phone's progress dialog and download lock stuck after the user cancelled.
  void client.request({ type: "download_cancel", transferId }, "transfer").catch(() => {});
}

const TRANSFER_RETRY_DELAYS_MS = [500, 1_000, 2_000, 4_000, 8_000, 15_000, 15_000];

export class TransferCancelledError extends Error {
  constructor() {
    super("transfer_cancelled");
    this.name = "TransferCancelledError";
  }
}

function throwIfCancelled(signal?: AbortSignal): void {
  if (signal?.aborted) throw new TransferCancelledError();
}

/** Race an in-flight NATS request with cancellation. NATS requests themselves
 * don't accept AbortSignal, so their transport timeout may finish later; this
 * wrapper releases the UI path immediately while still observing that late
 * result/rejection. */
function abortable<T>(operation: Promise<T>, signal?: AbortSignal): Promise<T> {
  throwIfCancelled(signal);
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

export async function uploadAttachments(
  client: RemoteClient,
  attachments: MobileAttachment[],
  onProgress?: (completedBytes: number, totalBytes: number) => void,
): Promise<MobileAttachment[]> {
  validateBatch(attachments);
  const totalBytes = attachments.reduce((sum, item) => sum + item.transferSize, 0);
  let completedBytes = 0;
  const uploaded: MobileAttachment[] = [];
  for (const attachment of attachments) {
    const init = await client.request<UploadInit>(
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
    );
    const file = new File(attachment.localUri);
    const handle = file.open(FileMode.ReadOnly);
    try {
      let index = 0;
      while ((handle.offset ?? 0) < attachment.transferSize) {
        const remaining = attachment.transferSize - (handle.offset ?? 0);
        const bytes = handle.readBytes(Math.min(init.data.chunkBytes, remaining));
        await withTransferRetry(
          () => client.uploadChunk(init.data.uploadId, index, bytes),
          undefined,
          undefined,
          [250, 500],
        );
        completedBytes += bytes.byteLength;
        onProgress?.(completedBytes, totalBytes);
        index += 1;
      }
    } catch (error) {
      await client
        .request({ type: "upload_cancel", transferId: init.data.uploadId }, "transfer")
        .catch(() => {});
      throw error;
    } finally {
      handle.close();
    }
    const complete = await client.requestRetry<UploadComplete>(
      { type: "upload_complete", transferId: init.data.uploadId },
      "transfer",
    );
    uploaded.push({
      ...attachment,
      uploadId: complete.data.uploadId,
      contentHash: complete.data.contentHash,
    });
  }
  return uploaded;
}

export async function prepareDownload(
  client: RemoteClient,
  sessionId: string,
  attachment: HistoryAttachment,
  variant: "preview" | "original" = "preview",
  signal?: AbortSignal,
  onWaiting?: () => void,
): Promise<DownloadInfo> {
  const command = {
    id: `download_prepare_${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`,
    type: "download_prepare",
    sessionId,
    filePath: attachment.path,
    mode: variant,
  };
  const response = await withTransferRetry(
    () => client.request<DownloadInfo>(command, sessionId),
    signal,
    onWaiting,
  );
  // Older desktops do not return `variant`; their preview behavior remains
  // compatible, so normalize the response to the variant we requested.
  const info = response.data.variant ? response.data : { ...response.data, variant };
  if (signal?.aborted) {
    cancelDownload(client, info.transferId);
    throw new TransferCancelledError();
  }
  try {
    if (await verifiedCachedDownload(info, signal)) {
      cancelDownload(client, info.transferId);
    }
  } catch (error) {
    cancelDownload(client, info.transferId);
    throw error;
  }
  return info;
}

function downloadSourceKey(attachment: HistoryAttachment, variant: "preview" | "original"): string {
  // The same desktop attachment path always produces the same prepared
  // preview while it remains in the session. Its content hash is only known
  // after desktop has resized/re-encoded the preview, so retain that resolved
  // key locally and avoid the prepare RPC on subsequent opens.
  return `${attachment.path}\u0000${attachment.name}\u0000${variant}`;
}

export interface CachedAttachmentPreview {
  info: DownloadInfo;
  file: File;
}

export function rememberPreparedPreview(attachment: HistoryAttachment, info: DownloadInfo): void {
  preparedDownloadIndex.set(downloadSourceKey(attachment, info.variant), info);
}

function cacheFile(info: DownloadInfo): File {
  const directory = new Directory(Paths.cache, "futureos-previews");
  if (!directory.exists) directory.create({ intermediates: true, idempotent: true });
  const ext = extension(info.name);
  return new File(directory, `${info.contentHash}${ext ? `.${ext}` : ""}`);
}

/**
 * The content-addressed cache deliberately uses its SHA-256 as the filename.
 * Before handing a file to the OS (share sheet / another app), materialize a
 * named copy so the receiving app and the user's Files destination retain the
 * original attachment name rather than seeing the cache key.
 */
export async function namedExternalFile(file: File, name: string): Promise<File> {
  const directory = new Directory(Paths.cache, "futureos-exports");
  if (!directory.exists) directory.create({ intermediates: true, idempotent: true });
  const target = new File(directory, basename(name));
  await file.copy(target, { overwrite: true });
  return target;
}

function prunePreviewCache(requiredBytes: number): void {
  const directory = new Directory(Paths.cache, "futureos-previews");
  if (!directory.exists) return;
  const files = directory
    .list()
    .filter((entry): entry is File => entry instanceof File)
    .sort((a, b) => (a.modificationTime ?? 0) - (b.modificationTime ?? 0));
  let total = files.reduce((sum, file) => sum + file.size, 0);
  for (const file of files) {
    if (total + requiredBytes <= MAX_PREVIEW_CACHE_BYTES) break;
    total -= file.size;
    file.delete();
  }
}

export function cachedDownload(info: DownloadInfo): File | null {
  const file = cacheFile(info);
  return file.exists && file.size === info.size ? file : null;
}

async function fileSha256(file: File): Promise<string> {
  const digest = new Uint8Array(
    await Crypto.digest(Crypto.CryptoDigestAlgorithm.SHA256, await file.bytes()),
  );
  return Array.from(digest, byte => byte.toString(16).padStart(2, "0")).join("");
}

async function verifiedCachedDownload(
  info: DownloadInfo,
  signal?: AbortSignal,
): Promise<File | null> {
  throwIfCancelled(signal);
  const file = cachedDownload(info);
  if (!file) return null;
  try {
    const hash = await fileSha256(file);
    throwIfCancelled(signal);
    if (hash === info.contentHash) return file;
  } catch (error) {
    if (error instanceof TransferCancelledError) throw error;
    // Treat unreadable cache entries like a miss and replace them below.
  }
  if (file.exists) file.delete();
  return null;
}

export function cachedPreviewForAttachment(
  attachment: HistoryAttachment,
  variant: "preview" | "original" = "preview",
): CachedAttachmentPreview | null {
  const key = downloadSourceKey(attachment, variant);
  const info = preparedDownloadIndex.get(key);
  if (!info) return null;
  const file = cachedDownload(info);
  if (file) return { info, file };
  preparedDownloadIndex.delete(key);
  return null;
}

export async function downloadPrepared(
  client: RemoteClient,
  info: DownloadInfo,
  onProgress?: (completedBytes: number, totalBytes: number) => void,
  signal?: AbortSignal,
  onWaiting?: () => void,
): Promise<File> {
  throwIfCancelled(signal);
  let cached: File | null;
  try {
    cached = await verifiedCachedDownload(info, signal);
  } catch (error) {
    cancelDownload(client, info.transferId);
    throw error;
  }
  if (cached) {
    cancelDownload(client, info.transferId);
    return cached;
  }
  prunePreviewCache(info.size);
  const file = cacheFile(info);
  if (file.exists) file.delete();
  file.create({ intermediates: true, overwrite: true });
  const handle = file.open(FileMode.Truncate);
  try {
    const chunks = Math.ceil(info.size / info.chunkBytes);
    let completed = 0;
    for (let index = 0; index < chunks; index += 1) {
      throwIfCancelled(signal);
      const bytes = await withTransferRetry(
        () => client.downloadChunk(info.transferId, index),
        signal,
        onWaiting,
      );
      throwIfCancelled(signal);
      handle.writeBytes(bytes);
      completed += bytes.byteLength;
      onProgress?.(completed, info.size);
    }
  } catch (error) {
    handle.close();
    if (file.exists) file.delete();
    cancelDownload(client, info.transferId);
    throw error;
  }
  handle.close();
  if (signal?.aborted) {
    file.delete();
    cancelDownload(client, info.transferId);
    throw new TransferCancelledError();
  }
  if (file.size !== info.size) {
    file.delete();
    cancelDownload(client, info.transferId);
    throw new Error("download_size_mismatch");
  }
  let hash: string;
  try {
    hash = await fileSha256(file);
    throwIfCancelled(signal);
  } catch (error) {
    file.delete();
    cancelDownload(client, info.transferId);
    throw error;
  }
  if (hash !== info.contentHash) {
    file.delete();
    cancelDownload(client, info.transferId);
    throw new Error("download_hash_mismatch");
  }
  cancelDownload(client, info.transferId);
  return file;
}
