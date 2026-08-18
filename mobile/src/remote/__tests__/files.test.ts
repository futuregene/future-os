import { createHash } from "crypto";
import * as Crypto from "expo-crypto";
import * as FS from "expo-file-system";
import * as ImageManipulator from "expo-image-manipulator";
import * as ImagePicker from "expo-image-picker";
import { Image } from "react-native";
import type { RemoteClient } from "../client";
import type { DownloadInfo, HistoryAttachment, MobileAttachment } from "../types";
import {
  cachedDownload,
  cachedPreviewForAttachment,
  deleteTemporaryAttachment,
  downloadPrepared,
  pickAttachments,
  pickFromAlbum,
  prepareDownload,
  rememberPreparedPreview,
  takePhoto,
  uploadAttachments,
} from "../files";

jest.mock("expo-file-system", () => {
  const store = new Map<
    string,
    { bytes?: Uint8Array; size?: number; type?: string; modTime?: number }
  >();
  const dirs = new Set<string>();

  class MockFile {
    uri: string;
    static pickFileAsync = jest.fn();
    constructor(uriOrDir: string | { uri: string }, name?: string) {
      this.uri = typeof uriOrDir === "string" ? uriOrDir : `${uriOrDir.uri}/${name}`;
    }
    get name(): string {
      const parts = this.uri.split("/");
      return parts[parts.length - 1] ?? "";
    }
    get exists(): boolean {
      return store.has(this.uri);
    }
    get size(): number {
      const rec = store.get(this.uri);
      if (!rec) return 0;
      return rec.size ?? rec.bytes?.length ?? 0;
    }
    get type(): string {
      return store.get(this.uri)?.type ?? "";
    }
    get modificationTime(): number {
      return store.get(this.uri)?.modTime ?? 0;
    }
    open(_mode: string): MockHandle {
      return new MockHandle(this.uri);
    }
    create(): void {
      if (!store.has(this.uri)) store.set(this.uri, { bytes: new Uint8Array(0) });
    }
    delete(): void {
      store.delete(this.uri);
    }
    async bytes(): Promise<Uint8Array> {
      return store.get(this.uri)?.bytes ?? new Uint8Array(0);
    }
  }

  class MockHandle {
    uri: string;
    offset = 0;
    constructor(uri: string) {
      this.uri = uri;
    }
    readBytes(n: number): Uint8Array {
      const bytes = store.get(this.uri)?.bytes ?? new Uint8Array(0);
      const slice = bytes.slice(this.offset, this.offset + n);
      this.offset += slice.byteLength;
      return slice;
    }
    writeBytes(bytes: Uint8Array): void {
      const rec = store.get(this.uri) ?? {};
      const existing = rec.bytes ?? new Uint8Array(0);
      const total = Math.max(existing.length, this.offset + bytes.length);
      const next = new Uint8Array(total);
      next.set(existing);
      next.set(bytes, this.offset);
      store.set(this.uri, { ...rec, bytes: next });
      this.offset += bytes.length;
    }
    close(): void {}
  }

  class MockDirectory {
    uri: string;
    constructor(...segments: string[]) {
      this.uri = segments.join("/");
    }
    get exists(): boolean {
      return dirs.has(this.uri);
    }
    create(): void {
      dirs.add(this.uri);
    }
    list(): MockFile[] {
      return Array.from(store.keys())
        .filter(uri => uri.startsWith(`${this.uri}/`))
        .map(uri => new MockFile(uri));
    }
  }

  return {
    __esModule: true,
    File: MockFile,
    Directory: MockDirectory,
    FileMode: { ReadOnly: "read", Truncate: "truncate" },
    Paths: { cache: "/mock/cache" },
    __set: (
      uri: string,
      opts: { bytes?: Uint8Array; size?: number; type?: string; modTime?: number } = {},
    ) => {
      store.set(uri, opts);
    },
    __reset: () => {
      store.clear();
      dirs.clear();
    },
    __dirs: dirs,
  };
});

jest.mock("expo-image-manipulator", () => ({
  __esModule: true,
  SaveFormat: { JPEG: "jpeg" },
  manipulateAsync: jest.fn(),
}));

jest.mock("expo-image-picker", () => ({
  __esModule: true,
  requestCameraPermissionsAsync: jest.fn(),
  launchCameraAsync: jest.fn(),
  requestMediaLibraryPermissionsAsync: jest.fn(),
  launchImageLibraryAsync: jest.fn(),
}));

jest.mock("expo-crypto", () => ({
  __esModule: true,
  CryptoDigestAlgorithm: { SHA256: "SHA-256" },
  digest: jest.fn(),
}));

// Only `Image.getSize` needs stubbing, but a bare `{ Image }` mock strips the
// `Platform`/`TurboModuleRegistry`/`NativeEventEmitter` exports that
// `expo-modules-core` reads from react-native. Expo installs a lazy global
// `fetch` polyfill (jest-expo setup); its first access loads `expo-modules-core`,
// whose `Platform.ts` evaluates `ReactNativePlatform.select` at module scope and
// crashes with "Cannot read properties of undefined (reading 'select')" when the
// real surface is stripped. `jest.requireActual("react-native")` can't be spread
// here because that eagerly evaluates the whole index (including the `DevMenu`
// getter -> `TurboModuleRegistry.getEnforcing('DevMenu')` -> invariant), so
// provide only the exports expo-modules-core actually touches at load time.
jest.mock("react-native", () => ({
  __esModule: true,
  Platform: {
    OS: "ios",
    select: (specifics: Record<string, unknown>) =>
      specifics?.ios ?? specifics?.native ?? specifics?.default,
  },
  TurboModuleRegistry: {
    get: () => null,
    getEnforcing: () => {
      throw new Error("native module not found");
    },
  },
  NativeEventEmitter: class {},
  Image: { getSize: jest.fn() },
}));

const mockFS = FS as unknown as {
  File: (new (...args: never[]) => FS.File) & { pickFileAsync: jest.Mock };
  Directory: new (...segments: string[]) => { uri: string };
  __set: (
    uri: string,
    opts?: { bytes?: Uint8Array; size?: number; type?: string; modTime?: number },
  ) => void;
  __reset: () => void;
};

const mockedManipulate = ImageManipulator.manipulateAsync as jest.Mock;
const mockedRequestCamera = ImagePicker.requestCameraPermissionsAsync as jest.Mock;
const mockedLaunchCamera = ImagePicker.launchCameraAsync as jest.Mock;
const mockedRequestLibrary = ImagePicker.requestMediaLibraryPermissionsAsync as jest.Mock;
const mockedLaunchLibrary = ImagePicker.launchImageLibraryAsync as jest.Mock;
const mockedDigest = Crypto.digest as jest.Mock;
const mockedGetSize = Image.getSize as jest.Mock;

function fsFile(
  uri: string,
  opts: { bytes?: Uint8Array; size?: number; type?: string; modTime?: number } = {},
): FS.File {
  mockFS.__set(uri, opts);
  return new mockFS.File(uri as never);
}

function mockClient(): {
  request: jest.Mock;
  requestRetry: jest.Mock;
  uploadChunk: jest.Mock;
  downloadChunk: jest.Mock;
} {
  return {
    request: jest.fn(),
    requestRetry: jest.fn(),
    uploadChunk: jest.fn(),
    downloadChunk: jest.fn(),
  };
}

function attachment(overrides: Partial<MobileAttachment> = {}): MobileAttachment {
  return {
    localUri: "file:///docs/a.txt",
    name: "a.txt",
    mimeType: "text/plain",
    kind: "file",
    originalSize: 8,
    transferSize: 8,
    ...overrides,
  };
}

function setImageSize(
  dimensions: Record<string, { width: number; height: number }> = {},
  errors: string[] = [],
): void {
  mockedGetSize.mockImplementation(
    (uri: string, onSuccess: (w: number, h: number) => void, onError: (e: Error) => void) => {
      if (errors.includes(uri)) {
        onError(new Error("decode"));
        return;
      }
      const dim = dimensions[uri] ?? { width: 100, height: 100 };
      onSuccess(dim.width, dim.height);
    },
  );
}

/** Drive prepareFile through pickAttachments with a single picker result. */
async function prepareOne(
  file: FS.File,
  existing: MobileAttachment[] = [],
): Promise<MobileAttachment[]> {
  mockFS.File.pickFileAsync.mockResolvedValue({ canceled: false, result: [file] });
  return pickAttachments(existing);
}

function sha256Hex(bytes: Uint8Array): string {
  return createHash("sha256").update(Buffer.from(bytes)).digest("hex");
}

const PNG_SIGNATURE = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
const PNG_ACTL = new Uint8Array([0, 0, 0, 0, 97, 99, 84, 76]); // length 0 + "acTL"
const PNG_IDAT = new Uint8Array([0, 0, 0, 0, 73, 68, 65, 84]); // length 0 + "IDAT"
const PNG_IHDR = new Uint8Array([0, 0, 0, 0, 73, 72, 68, 82]); // length 0 + "IHDR"

function concat(...arrays: Uint8Array[]): Uint8Array {
  const total = arrays.reduce((sum, a) => sum + a.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const a of arrays) {
    out.set(a, offset);
    offset += a.length;
  }
  return out;
}

function animatedWebpBytes(): Uint8Array {
  const bytes = new Uint8Array(21);
  bytes.set([0x52, 0x49, 0x46, 0x46], 0); // "RIFF"
  bytes.set([0x57, 0x45, 0x42, 0x50], 8); // "WEBP"
  bytes.set([0x56, 0x50, 0x38, 0x58], 12); // "VP8X"
  bytes[20] = 0x02; // animation flag
  return bytes;
}

beforeEach(() => {
  mockFS.__reset();
  jest.clearAllMocks();
  setImageSize();
  mockedDigest.mockImplementation(async (_alg: unknown, data: Uint8Array) => {
    return new Uint8Array(createHash("sha256").update(Buffer.from(data)).digest());
  });
});

describe("pickAttachments", () => {
  test("returns the existing attachments when the picker is cancelled", async () => {
    const existing = [attachment()];
    mockFS.File.pickFileAsync.mockResolvedValue({ canceled: true });
    expect(await pickAttachments(existing)).toBe(existing);
  });

  test("prepares a non-image file with its extension mime", async () => {
    const file = fsFile("file:///docs/notes.md", { bytes: new Uint8Array(3) });
    const result = await prepareOne(file);
    expect(result).toEqual([
      {
        localUri: "file:///docs/notes.md",
        name: "notes.md",
        mimeType: "text/markdown",
        kind: "file",
        originalSize: 3,
        transferSize: 3,
      },
    ]);
  });

  test("prepares a non-image file with the octet-stream fallback", async () => {
    const file = fsFile("file:///docs/data.bin", { bytes: new Uint8Array(3) });
    const result = await prepareOne(file);
    expect(result[0]!.mimeType).toBe("application/octet-stream");
  });

  test("preserves the Word mime type for external open/save", async () => {
    const file = fsFile("file:///docs/report.docx", { bytes: new Uint8Array(3) });
    const result = await prepareOne(file);
    expect(result[0]!.mimeType).toBe(
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );
  });

  test("rejects a batch over the attachment count quota", async () => {
    const files = Array.from({ length: 11 }, (_, i) =>
      fsFile(`file:///docs/f${i}.txt`, { bytes: new Uint8Array(1) }),
    );
    mockFS.File.pickFileAsync.mockResolvedValue({ canceled: false, result: files });
    await expect(pickAttachments([])).rejects.toThrow("attachment_count");
  });

  test("rejects a batch over the image count quota", async () => {
    const files = Array.from({ length: 5 }, (_, i) =>
      fsFile(`file:///docs/f${i}.png`, { bytes: new Uint8Array(1), type: "image/png" }),
    );
    mockFS.File.pickFileAsync.mockResolvedValue({ canceled: false, result: files });
    await expect(pickAttachments([])).rejects.toThrow("attachment_image_count");
  });

  test("rejects an empty or oversized file", async () => {
    const empty = fsFile("file:///docs/empty.txt", { bytes: new Uint8Array(0) });
    await expect(prepareOne(empty)).rejects.toThrow("attachment_file_too_large");

    const huge = fsFile("file:///docs/huge.bin", { size: 10 * 1024 * 1024 + 1 });
    await expect(prepareOne(huge)).rejects.toThrow("attachment_file_too_large");
  });

  test("rejects a batch over the total byte quota", async () => {
    const existing = [attachment({ originalSize: 20 * 1024 * 1024 - 5, transferSize: 1 })];
    const file = fsFile("file:///docs/x.txt", { bytes: new Uint8Array(10) });
    mockFS.File.pickFileAsync.mockResolvedValue({ canceled: false, result: [file] });
    await expect(pickAttachments(existing)).rejects.toThrow("attachment_total_size");
  });

  test("passes a small non-JPEG image through untouched with a preview flag", async () => {
    const file = fsFile("file:///docs/photo.png", {
      bytes: new Uint8Array(10),
      type: "image/png",
    });
    const result = await prepareOne(file);
    expect(result[0]).toMatchObject({
      localUri: "file:///docs/photo.png",
      name: "photo.png",
      mimeType: "image/png",
      kind: "image",
      mobilePreviewUnsupported: false,
    });
  });

  test("marks an animated gif as preview-unsupported", async () => {
    const file = fsFile("file:///docs/anim.gif", { bytes: new Uint8Array(10), type: "image/gif" });
    const result = await prepareOne(file);
    expect(result[0]!.mobilePreviewUnsupported).toBe(true);
  });

  test("rejects an image with no resolvable format", async () => {
    const file = fsFile("file:///docs/photo", { bytes: new Uint8Array(10), type: "image/tiff" });
    await expect(prepareOne(file)).rejects.toThrow("attachment_image_format");
  });

  test("rejects an image that fails to decode", async () => {
    setImageSize({}, ["file:///docs/photo.png"]);
    const file = fsFile("file:///docs/photo.png", { bytes: new Uint8Array(10), type: "image/png" });
    await expect(prepareOne(file)).rejects.toThrow("attachment_image_decode");
  });

  test("re-encodes a JPEG-adjacent input through the converter", async () => {
    mockedManipulate.mockResolvedValue({ uri: "file:///converted/out.jpg" });
    mockFS.__set("file:///converted/out.jpg", { bytes: new Uint8Array(7) });
    const file = fsFile("file:///docs/photo.jpg", {
      bytes: new Uint8Array(10),
      type: "image/jpeg",
    });
    const result = await prepareOne(file);
    expect(mockedManipulate).toHaveBeenCalled();
    expect(result[0]).toMatchObject({
      localUri: "file:///converted/out.jpg",
      transferName: "photo.jpg",
      mimeType: "image/jpeg",
      temporary: true,
    });
  });

  test("downsamples an oversized image and derives a jpeg transfer name", async () => {
    setImageSize({ "file:///docs/big.png": { width: 3200, height: 1600 } });
    mockedManipulate.mockResolvedValue({ uri: "file:///converted/out.jpg" });
    mockFS.__set("file:///converted/out.jpg", { bytes: new Uint8Array(5) });
    const file = fsFile("file:///docs/big.png", { bytes: new Uint8Array(10), type: "image/png" });
    const result = await prepareOne(file);
    expect(result[0]!.transferName).toBe("big.jpg");
    // Resize was requested (not the no-op []) and the longer edge was capped.
    expect(mockedManipulate).toHaveBeenCalledWith(
      "file:///docs/big.png",
      expect.any(Array),
      expect.anything(),
    );
  });

  test("rejects a converted file whose compressed bytes exceed the cap", async () => {
    mockedManipulate.mockResolvedValue({ uri: "file:///converted/out.jpg" });
    mockFS.__set("file:///converted/out.jpg", { size: 10 * 1024 * 1024 + 1 });
    const file = fsFile("file:///docs/photo.jpg", {
      bytes: new Uint8Array(10),
      type: "image/jpeg",
    });
    await expect(prepareOne(file)).rejects.toThrow("attachment_compressed_too_large");
  });

  test("rejects when the converter itself fails", async () => {
    mockedManipulate.mockRejectedValue(new Error("converter boom"));
    const file = fsFile("file:///docs/photo.jpg", {
      bytes: new Uint8Array(10),
      type: "image/jpeg",
    });
    await expect(prepareOne(file)).rejects.toThrow("attachment_image_decode");
  });

  test("flags an animated png as preview-unsupported", async () => {
    const file = fsFile("file:///docs/anim.png", {
      bytes: concat(PNG_SIGNATURE, PNG_ACTL),
      type: "image/png",
    });
    const result = await prepareOne(file);
    expect(result[0]!.mobilePreviewUnsupported).toBe(true);
  });

  test("treats a static png as preview-supported", async () => {
    const file = fsFile("file:///docs/static.png", {
      bytes: concat(PNG_SIGNATURE, PNG_IDAT),
      type: "image/png",
    });
    const result = await prepareOne(file);
    expect(result[0]!.mobilePreviewUnsupported).toBe(false);
  });

  test("flags an animated webp as preview-unsupported", async () => {
    const file = fsFile("file:///docs/anim.webp", {
      bytes: animatedWebpBytes(),
      type: "image/webp",
    });
    const result = await prepareOne(file);
    expect(result[0]!.mobilePreviewUnsupported).toBe(true);
  });

  it.each([
    ["png", "image/png"],
    ["gif", "image/gif"],
    ["webp", "image/webp"],
  ])(
    "derives the %s mime from the extension when the picker reports no type",
    async (ext, mime) => {
      const file = fsFile(`file:///docs/photo.${ext}`, { bytes: new Uint8Array(10) });
      const result = await prepareOne(file);
      expect(result[0]!.mimeType).toBe(mime);
    },
  );

  it.each([
    ["image/png", "image/png"],
    ["image/gif", "image/gif"],
    ["image/webp", "image/webp"],
  ])("passes through a no-extension %s input", async (mimeType, expected) => {
    const file = fsFile("file:///docs/photo", { bytes: new Uint8Array(10), type: mimeType });
    const result = await prepareOne(file);
    expect(result[0]).toMatchObject({ mimeType: expected, kind: "image" });
  });

  it.each(["image/jpeg", "image/bmp", "image/heic", "image/heif"])(
    "re-encodes a no-extension %s input to jpeg",
    async mimeType => {
      mockedManipulate.mockResolvedValue({ uri: "file:///converted/out.jpg" });
      mockFS.__set("file:///converted/out.jpg", { bytes: new Uint8Array(5) });
      const file = fsFile("file:///docs/photo", { bytes: new Uint8Array(10), type: mimeType });
      const result = await prepareOne(file);
      expect(mockedManipulate).toHaveBeenCalled();
      expect(result[0]).toMatchObject({ mimeType: "image/jpeg", temporary: true });
    },
  );

  test("skips a non-terminal png chunk and stops at the end", async () => {
    const file = fsFile("file:///docs/meta.png", {
      bytes: concat(PNG_SIGNATURE, PNG_IHDR, new Uint8Array(4)),
      type: "image/png",
    });
    const result = await prepareOne(file);
    expect(result[0]!.mobilePreviewUnsupported).toBe(false);
  });

  test("treats a png whose chunk length runs past the file as static", async () => {
    const file = fsFile("file:///docs/bad.png", {
      bytes: concat(PNG_SIGNATURE, PNG_IHDR),
      type: "image/png",
    });
    const result = await prepareOne(file);
    expect(result[0]!.mobilePreviewUnsupported).toBe(false);
  });
});

describe("takePhoto", () => {
  test("rejects when camera permission is denied", async () => {
    mockedRequestCamera.mockResolvedValue({ granted: false });
    await expect(takePhoto([])).rejects.toThrow("attachment_camera_permission");
  });

  test("returns existing attachments when the camera is cancelled", async () => {
    mockedRequestCamera.mockResolvedValue({ granted: true });
    mockedLaunchCamera.mockResolvedValue({ canceled: true, assets: [] });
    const existing = [attachment()];
    expect(await takePhoto(existing)).toBe(existing);
  });

  test("prepares a captured photo as a forced-jpeg attachment", async () => {
    mockedRequestCamera.mockResolvedValue({ granted: true });
    mockedLaunchCamera.mockResolvedValue({
      canceled: false,
      assets: [{ uri: "file:///camera/photo.jpg", mimeType: "image/jpeg" }],
    });
    mockFS.__set("file:///camera/photo.jpg", { bytes: new Uint8Array(10), type: "image/jpeg" });
    mockedManipulate.mockResolvedValue({ uri: "file:///converted/out.jpg" });
    mockFS.__set("file:///converted/out.jpg", { bytes: new Uint8Array(5) });

    const result = await takePhoto([]);
    expect(mockedManipulate).toHaveBeenCalled();
    expect(result[0]).toMatchObject({
      kind: "image",
      mimeType: "image/jpeg",
      name: expect.stringMatching(/\.jpg$/),
      temporary: true,
    });
  });
});

describe("pickFromAlbum", () => {
  test("rejects when media library permission is denied", async () => {
    mockedRequestLibrary.mockResolvedValue({ granted: false });
    await expect(pickFromAlbum([])).rejects.toThrow("attachment_album_permission");
  });

  test("returns existing attachments when the library is cancelled", async () => {
    mockedRequestLibrary.mockResolvedValue({ granted: true });
    mockedLaunchLibrary.mockResolvedValue({ canceled: true, assets: [] });
    const existing = [attachment()];
    expect(await pickFromAlbum(existing)).toBe(existing);
  });

  test("prepares a selected image without forcing jpeg", async () => {
    mockedRequestLibrary.mockResolvedValue({ granted: true });
    mockedLaunchLibrary.mockResolvedValue({
      canceled: false,
      assets: [{ uri: "file:///album/photo.png", mimeType: "image/png" }],
    });
    mockFS.__set("file:///album/photo.png", { bytes: new Uint8Array(10), type: "image/png" });

    const result = await pickFromAlbum([]);
    expect(mockedManipulate).not.toHaveBeenCalled();
    expect(result[0]).toMatchObject({
      kind: "image",
      mimeType: "image/png",
      name: "photo.png",
    });
  });
});

describe("deleteTemporaryAttachment", () => {
  test("does nothing for a non-temporary attachment", () => {
    const a = attachment({ temporary: false });
    expect(() => deleteTemporaryAttachment(a)).not.toThrow();
  });

  test("deletes the backing file of a temporary attachment", () => {
    mockFS.__set("file:///cache/tmp.jpg", { bytes: new Uint8Array(3) });
    const a = attachment({ localUri: "file:///cache/tmp.jpg", temporary: true });
    deleteTemporaryAttachment(a);
    expect(new mockFS.File("file:///cache/tmp.jpg" as never).exists).toBe(false);
  });
});

describe("uploadAttachments", () => {
  test("uploads every chunk and completes with server content identity", async () => {
    mockFS.__set("file:///docs/a.txt", { bytes: new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]) });
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: { uploadId: "u1", chunkBytes: 4 } });
    client.requestRetry.mockResolvedValue({
      success: true,
      data: { uploadId: "u1", contentHash: "hash" },
    });
    client.uploadChunk.mockResolvedValue(undefined);

    const progress: number[][] = [];
    const result = await uploadAttachments(
      client as unknown as RemoteClient,
      [attachment()],
      (done, total) => progress.push([done, total]),
    );

    expect(client.uploadChunk).toHaveBeenCalledTimes(2);
    expect(result[0]).toMatchObject({ uploadId: "u1", contentHash: "hash" });
    expect(progress).toEqual([
      [4, 8],
      [8, 8],
    ]);
  });

  test("retries a failed chunk before giving up", async () => {
    mockFS.__set("file:///docs/a.txt", {
      bytes: new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]),
    });
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: { uploadId: "u1", chunkBytes: 8 } });
    client.requestRetry.mockResolvedValue({
      success: true,
      data: { uploadId: "u1", contentHash: "hash" },
    });
    client.uploadChunk.mockRejectedValueOnce(new Error("transient")).mockResolvedValue(undefined);

    const result = await uploadAttachments(client as unknown as RemoteClient, [attachment()]);
    expect(client.uploadChunk).toHaveBeenCalledTimes(2);
    expect(result).toHaveLength(1);
  });

  test("cancels the transfer and rethrows when a chunk keeps failing", async () => {
    mockFS.__set("file:///docs/a.txt", {
      bytes: new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]),
    });
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: { uploadId: "u1", chunkBytes: 8 } });
    client.uploadChunk.mockRejectedValue(new Error("dead"));

    await expect(
      uploadAttachments(client as unknown as RemoteClient, [attachment()]),
    ).rejects.toThrow("dead");
    expect(client.request).toHaveBeenCalledWith(
      { type: "upload_cancel", transferId: "u1" },
      "transfer",
    );
  });

  test("rejects a batch over the image count quota", async () => {
    const client = mockClient();
    const images = Array.from({ length: 5 }, (_, i) =>
      attachment({ kind: "image", localUri: `file:///img/${i}.png` }),
    );
    await expect(uploadAttachments(client as unknown as RemoteClient, images)).rejects.toThrow(
      "attachment_image_count",
    );
  });
});

describe("download & preview cache", () => {
  const info: DownloadInfo = {
    transferId: "t1",
    name: "result.jpg",
    mimeType: "image/jpeg",
    size: 8,
    contentHash: sha256Hex(new Uint8Array(8)),
    previewKind: "image",
    variant: "preview",
    chunkBytes: 4,
  };

  function cacheUri(i: DownloadInfo): string {
    return `/mock/cache/futureos-previews/${i.contentHash}.jpg`;
  }

  test("prepareDownload returns the prepared info when not cached", async () => {
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: info });
    const history: HistoryAttachment = { path: "/tmp/a.jpg", name: "a.jpg" };
    expect(await prepareDownload(client as unknown as RemoteClient, "s1", history)).toBe(info);
    expect(client.request).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "download_prepare",
        sessionId: "s1",
        filePath: "/tmp/a.jpg",
        mode: "preview",
      }),
      "s1",
    );
  });

  test("requests the untouched original independently from the preview", async () => {
    const original = { ...info, previewKind: "file" as const, variant: "original" as const };
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: original });
    const history: HistoryAttachment = { path: "/tmp/a.jpg", name: "a.jpg" };
    expect(
      await prepareDownload(client as unknown as RemoteClient, "s1", history, "original"),
    ).toBe(original);
    expect(client.request).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "download_prepare",
        sessionId: "s1",
        filePath: "/tmp/a.jpg",
        mode: "original",
      }),
      "s1",
    );
  });

  test("prepareDownload cancels the transfer when the file is already cached", async () => {
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: info });
    await prepareDownload(client as unknown as RemoteClient, "s1", {
      path: "/tmp/a.jpg",
      name: "a.jpg",
    });
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("prepareDownload retries with the same command id after a transient failure", async () => {
    const client = mockClient();
    client.request.mockRejectedValueOnce(new Error("timeout")).mockResolvedValue({
      success: true,
      data: info,
    });
    const waiting = jest.fn();
    await prepareDownload(
      client as unknown as RemoteClient,
      "s1",
      { path: "/tmp/a.jpg", name: "a.jpg" },
      "preview",
      undefined,
      waiting,
    );
    expect(waiting).toHaveBeenCalledTimes(1);
    expect(client.request).toHaveBeenCalledTimes(2);
    expect(client.request.mock.calls[0]![0].id).toBe(client.request.mock.calls[1]![0].id);
  });

  test("cachedDownload returns only a present size-matching cache candidate", () => {
    expect(cachedDownload(info)).toBeNull();
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    expect(cachedDownload(info)).not.toBeNull();
    // Wrong size → treated as not cached.
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(7) });
    expect(cachedDownload(info)).toBeNull();
  });

  test("rememberPreparedPreview + cachedPreviewForAttachment round-trip", () => {
    const attachment: HistoryAttachment = { path: "/tmp/a.jpg", name: "a.jpg" };
    expect(cachedPreviewForAttachment(attachment)).toBeNull();

    // No backing file yet — the remembered preview is pruned on first look.
    rememberPreparedPreview(attachment, info);
    expect(cachedPreviewForAttachment(attachment)).toBeNull();

    // Re-remember once the backing file exists; the preview is then served.
    rememberPreparedPreview(attachment, info);
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    const preview = cachedPreviewForAttachment(attachment);
    expect(preview?.info).toBe(info);

    // A pruned backing file drops the remembered preview again.
    mockFS.__reset();
    expect(cachedPreviewForAttachment(attachment)).toBeNull();
  });

  test("keeps preview and original cache entries independent", () => {
    const attachment: HistoryAttachment = { path: "/tmp/a.jpg", name: "a.jpg" };
    const original: DownloadInfo = {
      ...info,
      contentHash: "original123",
      previewKind: "file",
      variant: "original",
    };
    rememberPreparedPreview(attachment, info);
    rememberPreparedPreview(attachment, original);
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    mockFS.__set("/mock/cache/futureos-previews/original123.jpg", {
      bytes: new Uint8Array(8),
    });
    expect(cachedPreviewForAttachment(attachment, "preview")?.info).toBe(info);
    expect(cachedPreviewForAttachment(attachment, "original")?.info).toBe(original);
  });

  test("downloadPrepared downloads, verifies size and hash, and cancels", async () => {
    const fileBytes = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
    const i: DownloadInfo = { ...info, size: 8, contentHash: sha256Hex(fileBytes) };
    const client = mockClient();
    client.downloadChunk.mockImplementation(async (_tid: string, index: number) =>
      fileBytes.slice(index * 4, index * 4 + 4),
    );
    client.request.mockResolvedValue({ success: true, data: {} });

    const progress: number[][] = [];
    const file = await downloadPrepared(client as unknown as RemoteClient, i, (done, total) =>
      progress.push([done, total]),
    );

    expect(file.exists).toBe(true);
    expect(client.downloadChunk).toHaveBeenCalledTimes(2);
    expect(progress).toEqual([
      [4, 8],
      [8, 8],
    ]);
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared returns the cached file without re-downloading", async () => {
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: {} });
    const file = await downloadPrepared(client as unknown as RemoteClient, info);
    expect(file.exists).toBe(true);
    expect(client.downloadChunk).not.toHaveBeenCalled();
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared replaces a same-size cache entry whose hash is wrong", async () => {
    const fileBytes = new Uint8Array([1, 2, 3, 4]);
    const i: DownloadInfo = { ...info, size: 4, chunkBytes: 4, contentHash: sha256Hex(fileBytes) };
    mockFS.__set(cacheUri(i), { bytes: new Uint8Array([9, 9, 9, 9]) });
    const client = mockClient();
    client.downloadChunk.mockResolvedValue(fileBytes);
    client.request.mockResolvedValue({ success: true, data: {} });
    const file = await downloadPrepared(client as unknown as RemoteClient, i);
    expect(client.downloadChunk).toHaveBeenCalledTimes(1);
    expect(await file.bytes()).toEqual(fileBytes);
  });

  test("downloadPrepared retries a transient chunk failure", async () => {
    const fileBytes = new Uint8Array([1, 2, 3, 4]);
    const i: DownloadInfo = { ...info, size: 4, chunkBytes: 4, contentHash: sha256Hex(fileBytes) };
    const client = mockClient();
    client.downloadChunk.mockRejectedValueOnce(new Error("transient")).mockResolvedValue(fileBytes);
    client.request.mockResolvedValue({ success: true, data: {} });
    const file = await downloadPrepared(client as unknown as RemoteClient, i);
    expect(file.exists).toBe(true);
    expect(client.downloadChunk).toHaveBeenCalledTimes(2);
  });

  test("downloadPrepared rejects a size mismatch", async () => {
    const i: DownloadInfo = { ...info, size: 8, contentHash: "x" };
    const client = mockClient();
    client.downloadChunk.mockResolvedValue(new Uint8Array(0)); // writes no bytes
    client.request.mockResolvedValue({ success: true, data: {} });
    await expect(downloadPrepared(client as unknown as RemoteClient, i)).rejects.toThrow(
      "download_size_mismatch",
    );
  });

  test("downloadPrepared rejects a hash mismatch", async () => {
    const fileBytes = new Uint8Array([1, 2, 3, 4]);
    const i: DownloadInfo = { ...info, size: 4, chunkBytes: 4, contentHash: "wrong" };
    const client = mockClient();
    client.downloadChunk.mockResolvedValue(fileBytes);
    client.request.mockResolvedValue({ success: true, data: {} });
    await expect(downloadPrepared(client as unknown as RemoteClient, i)).rejects.toThrow(
      "download_hash_mismatch",
    );
  });

  test("downloadPrepared cleans up and rethrows on a chunk failure", async () => {
    const i: DownloadInfo = { ...info, size: 8, contentHash: "x" };
    const client = mockClient();
    client.downloadChunk.mockRejectedValue(new Error("Download expired or does not exist"));
    client.request.mockResolvedValue({ success: true, data: {} });
    await expect(downloadPrepared(client as unknown as RemoteClient, i)).rejects.toThrow(
      "Download expired or does not exist",
    );
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared cancels a weak-network retry without waiting for the next attempt", async () => {
    const i: DownloadInfo = { ...info, size: 8, contentHash: "x" };
    const client = mockClient();
    const controller = new AbortController();
    client.downloadChunk.mockRejectedValue(new Error("not_connected"));
    client.request.mockResolvedValue({ success: true, data: {} });
    await expect(
      downloadPrepared(client as unknown as RemoteClient, i, undefined, controller.signal, () =>
        controller.abort(),
      ),
    ).rejects.toThrow("transfer_cancelled");
    expect(client.downloadChunk).toHaveBeenCalledTimes(1);
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared prunes the preview cache when over budget", async () => {
    const fileBytes = new Uint8Array([1, 2, 3, 4]);
    const i: DownloadInfo = { ...info, size: 4, chunkBytes: 4, contentHash: sha256Hex(fileBytes) };
    mockFS.__set("/mock/cache/futureos-previews/old1.jpg", { size: 60 * 1024 * 1024, modTime: 1 });
    mockFS.__set("/mock/cache/futureos-previews/old2.jpg", { size: 60 * 1024 * 1024, modTime: 2 });
    const client = mockClient();
    client.downloadChunk.mockResolvedValue(fileBytes);
    client.request.mockResolvedValue({ success: true, data: {} });

    const file = await downloadPrepared(client as unknown as RemoteClient, i);
    expect(file.exists).toBe(true);
    // The oldest preview is evicted first; the second stays under budget.
    expect(new mockFS.File("/mock/cache/futureos-previews/old1.jpg" as never).exists).toBe(false);
    expect(new mockFS.File("/mock/cache/futureos-previews/old2.jpg" as never).exists).toBe(true);
  });

  test("downloadPrepared cleans up and rethrows when hashing fails", async () => {
    const fileBytes = new Uint8Array([1, 2, 3, 4]);
    const i: DownloadInfo = { ...info, size: 4, chunkBytes: 4, contentHash: "x" };
    const client = mockClient();
    client.downloadChunk.mockResolvedValue(fileBytes);
    client.request.mockResolvedValue({ success: true, data: {} });
    mockedDigest.mockRejectedValue(new Error("digest boom"));

    await expect(downloadPrepared(client as unknown as RemoteClient, i)).rejects.toThrow(
      "digest boom",
    );
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });
});
