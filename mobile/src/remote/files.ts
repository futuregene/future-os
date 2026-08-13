import { File, FileMode, Directory, Paths } from "expo-file-system";
import * as ImageManipulator from "expo-image-manipulator";
import * as ImagePicker from "expo-image-picker";
import * as Crypto from "expo-crypto";
import { Image } from "react-native";
import type { RemoteClient } from "./client";
import type { DownloadInfo, HistoryAttachment, MobileAttachment } from "./types";

export const MAX_FILE_BYTES = 10 * 1024 * 1024;
export const MAX_MESSAGE_BYTES = 20 * 1024 * 1024;
export const MAX_ATTACHMENTS = 10;
export const MAX_IMAGES = 4;
// Longest-edge cap for image attachments. Images over this are downsampled
// (never rejected) so every source — camera, album, screenshot — behaves the
// same. 1600px keeps documents/screenshots readable and the transfer small
// (report 06 decision D1). The desktop receive-side 2000px check still passes.
export const MAX_IMAGE_EDGE = 1600;
const MAX_PREVIEW_CACHE_BYTES = 100 * 1024 * 1024;
const preparedPreviewIndex = new Map<string, DownloadInfo>();

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

function mimeFor(name: string, fallback = "application/octet-stream"): string {
  switch (extension(name)) {
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "png":
      return "image/png";
    case "gif":
      return "image/gif";
    case "webp":
      return "image/webp";
    case "bmp":
      return "image/bmp";
    case "heic":
      return "image/heic";
    case "heif":
      return "image/heif";
    case "md":
    case "markdown":
      return "text/markdown";
    default:
      return fallback;
  }
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
  const originalSize = file.size;
  if (originalSize <= 0 || originalSize > MAX_FILE_BYTES) {
    throw new Error("attachment_file_too_large");
  }
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

interface UploadInit {
  uploadId: string;
  chunkBytes: number;
}

interface UploadComplete {
  uploadId: string;
  contentHash: string;
}

async function cancelDownload(client: RemoteClient, transferId: string): Promise<void> {
  await client.request({ type: "download_cancel", transferId }, "transfer").catch(() => {});
}

async function withTransferRetry<T>(operation: () => Promise<T>): Promise<T> {
  let lastError: unknown;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      if (attempt < 2) {
        await new Promise(resolve => setTimeout(resolve, 250 * 2 ** attempt));
      }
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
        await withTransferRetry(() => client.uploadChunk(init.data.uploadId, index, bytes));
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
): Promise<DownloadInfo> {
  const response = await client.request<DownloadInfo>(
    { type: "download_prepare", sessionId, filePath: attachment.path },
    sessionId,
  );
  const info = response.data;
  if (cachedDownload(info)) {
    await cancelDownload(client, info.transferId);
  }
  return info;
}

function previewSourceKey(attachment: HistoryAttachment): string {
  // The same desktop attachment path always produces the same prepared
  // preview while it remains in the session. Its content hash is only known
  // after desktop has resized/re-encoded the preview, so retain that resolved
  // key locally and avoid the prepare RPC on subsequent opens.
  return `${attachment.path}\u0000${attachment.name}`;
}

export interface CachedAttachmentPreview {
  info: DownloadInfo;
  file: File;
}

export function rememberPreparedPreview(attachment: HistoryAttachment, info: DownloadInfo): void {
  preparedPreviewIndex.set(previewSourceKey(attachment), info);
}

function cacheFile(info: DownloadInfo): File {
  const directory = new Directory(Paths.cache, "futureos-previews");
  if (!directory.exists) directory.create({ intermediates: true, idempotent: true });
  const ext = extension(info.name);
  return new File(directory, `${info.contentHash}${ext ? `.${ext}` : ""}`);
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

export function cachedPreviewForAttachment(
  attachment: HistoryAttachment,
): CachedAttachmentPreview | null {
  const info = preparedPreviewIndex.get(previewSourceKey(attachment));
  if (!info) return null;
  const file = cachedDownload(info);
  if (file) return { info, file };
  preparedPreviewIndex.delete(previewSourceKey(attachment));
  return null;
}

export async function downloadPrepared(
  client: RemoteClient,
  info: DownloadInfo,
  onProgress?: (completedBytes: number, totalBytes: number) => void,
): Promise<File> {
  const cached = cachedDownload(info);
  if (cached) {
    await cancelDownload(client, info.transferId);
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
      const bytes = await withTransferRetry(() => client.downloadChunk(info.transferId, index));
      handle.writeBytes(bytes);
      completed += bytes.byteLength;
      onProgress?.(completed, info.size);
    }
  } catch (error) {
    handle.close();
    if (file.exists) file.delete();
    await cancelDownload(client, info.transferId);
    throw error;
  }
  handle.close();
  if (file.size !== info.size) {
    file.delete();
    await cancelDownload(client, info.transferId);
    throw new Error("download_size_mismatch");
  }
  let hash: string;
  try {
    const digest = new Uint8Array(
      await Crypto.digest(Crypto.CryptoDigestAlgorithm.SHA256, await file.bytes()),
    );
    hash = Array.from(digest, byte => byte.toString(16).padStart(2, "0")).join("");
  } catch (error) {
    file.delete();
    await cancelDownload(client, info.transferId);
    throw error;
  }
  if (hash !== info.contentHash) {
    file.delete();
    await cancelDownload(client, info.transferId);
    throw new Error("download_hash_mismatch");
  }
  await cancelDownload(client, info.transferId);
  return file;
}
