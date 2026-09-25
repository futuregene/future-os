import { createHash } from "crypto";
import * as Crypto from "expo-crypto";
import * as FS from "expo-file-system";
import { hashFile as nativeFileSha256, listAlbumImages, resolveImagePickRoutes, supportsAlbumGrid } from "future-file-handler";
import type { ImagePickRoutes } from "future-file-handler";
import * as ImageManipulator from "expo-image-manipulator";
import * as ImagePicker from "expo-image-picker";
import * as IntentLauncher from "expo-intent-launcher";
import { Image, Platform } from "react-native";
import type { RemoteClient } from "../client";
import type { DownloadInfo, HistoryAttachment, MobileAttachment } from "../types";
import {
  albumSource,
  cachedDownload,
  cachedPreviewForAttachment,
  deleteTemporaryAttachment,
  downloadPrepared,
  fileSha256,
  namedExternalFile,
  loadAlbumImages,
  pickAttachments,
  pickFromAlbum,
  prepareAlbumImages,
  prepareDownload,
  prepareSharedAttachments,
  recoverPendingImagePickerAttachments,
  rememberPreparedPreview,
  remainingImageSlots,
  takePhoto,
  uploadAttachments,
} from "../files";

jest.mock("future-file-handler", () => ({
  hashFile: jest.fn(async () => null),
  resolveImagePickRoutes: jest.fn(async () => null),
  listAlbumImages: jest.fn(async () => []),
  supportsAlbumGrid: jest.fn(() => true),
}));

jest.mock("expo-file-system", () => {
  const store = new Map<
    string,
    { bytes?: Uint8Array; size?: number; type?: string; modTime?: number }
  >();
  const dirs = new Set<string>();
  let readHook: ((size: number) => void) | undefined;

  class MockFile {
    uri: string;
    static pickFileAsync = jest.fn();
    constructor(uriOrDir: string | { uri: string }, name?: string) {
      const base = typeof uriOrDir === "string" ? uriOrDir : uriOrDir.uri;
      this.uri = name ? `${base}/${name}` : base;
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
    async copy(destination: MockFile, _options?: { overwrite?: boolean }): Promise<void> {
      const source = store.get(this.uri);
      if (!source) throw new Error("source_missing");
      store.set(destination.uri, { ...source });
    }
  }

  class MockHandle {
    uri: string;
    offset = 0;
    constructor(uri: string) {
      this.uri = uri;
    }
    readBytes(n: number): Uint8Array {
      readHook?.(n);
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
    get name(): string { return this.uri.split("/").pop()!; }
    list(): (MockFile | MockDirectory)[] {
      const child = (uri: string) => uri.startsWith(`${this.uri}/`) && !uri.slice(this.uri.length + 1).includes("/");
      return [
        ...Array.from(store.keys()).filter(child).map(uri => new MockFile(uri)),
        ...Array.from(dirs).filter(child).map(uri => new MockDirectory(uri)),
      ];
    }
    delete(): void {
      for (const uri of store.keys()) if (uri.startsWith(`${this.uri}/`)) store.delete(uri);
      for (const uri of dirs) if (uri === this.uri || uri.startsWith(`${this.uri}/`)) dirs.delete(uri);
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
    __onRead: (hook: (size: number) => void) => { readHook = hook; },
    __reset: () => {
      readHook = undefined;
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
  getPendingResultAsync: jest.fn(),
}));


jest.mock("expo-intent-launcher", () => ({
  __esModule: true,
  ResultCode: { Success: -1, Canceled: 0 },
  startActivityAsync: jest.fn(),
}));

jest.mock("expo-crypto", () => ({
  __esModule: true,
  CryptoDigestAlgorithm: { SHA256: "SHA-256" },
  digest: jest.fn(),
  randomUUID: jest.fn(() => "test-id"),
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
  __onRead: (hook: (size: number) => void) => void;
};

const mockedManipulate = ImageManipulator.manipulateAsync as jest.Mock;
const mockedRequestCamera = ImagePicker.requestCameraPermissionsAsync as jest.Mock;
const mockedLaunchCamera = ImagePicker.launchCameraAsync as jest.Mock;
const mockedRequestLibrary = ImagePicker.requestMediaLibraryPermissionsAsync as jest.Mock;
const mockedLaunchLibrary = ImagePicker.launchImageLibraryAsync as jest.Mock;
const mockedPendingResult = ImagePicker.getPendingResultAsync as jest.Mock;
const mockedStartActivity = IntentLauncher.startActivityAsync as jest.Mock;
const mockedResolveRoutes = resolveImagePickRoutes as jest.Mock;
const mockedListAlbumImages = listAlbumImages as jest.Mock;
const mockedSupportsAlbumGrid = supportsAlbumGrid as jest.Mock;
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
  recoverAfterTransientRequest: jest.Mock;
  uploadChunk: jest.Mock;
  downloadChunk: jest.Mock;
} {
  return {
    request: jest.fn(),
    requestRetry: jest.fn(),
    recoverAfterTransientRequest: jest.fn().mockResolvedValue(true),
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

/** An image as the native album listing reports it. */
function albumImage(index: number, name: string, mimeType: string) {
  return {
    uri: `content://media/external/images/media/${index}`,
    name,
    mimeType,
    size: 10,
    modified: 1_700_000_000_000 + index,
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
  let exportId = 0;
  (Crypto.randomUUID as jest.Mock).mockImplementation(() => `export-${++exportId}`);
  setImageSize();
  mockedDigest.mockImplementation(async (_alg: unknown, data: Uint8Array) => {
    return new Uint8Array(createHash("sha256").update(Buffer.from(data)).digest());
  });
});

describe("failed attachment batches", () => {
  test.each(["files", "album", "share"])("cleans successful conversions after another %s item fails, including late results", async source => {
    const good = fsFile("file:///good.jpg", { size: 100 });
    const bad = fsFile("file:///bad.jpg", { size: 100 });
    const converted = fsFile("file:///converted.jpg", { size: 50 });
    const existing = attachment({ localUri: "file:///existing.jpg", temporary: true });
    fsFile(existing.localUri, { size: 8 });
    let finish!: (value: { uri: string; width: number; height: number }) => void;
    mockedManipulate.mockImplementation((uri: string) => uri === bad.uri
      ? Promise.reject(new Error("bad image"))
      : new Promise(resolve => { finish = resolve; }));
    mockFS.File.pickFileAsync.mockResolvedValue({ canceled: false, result: [good, bad] });
    mockedLaunchLibrary.mockResolvedValue({ canceled: false, assets: [{ uri: good.uri }, { uri: bad.uri }] });
    const pending = source === "files" ? pickAttachments([existing])
      : source === "album" ? pickFromAlbum([existing])
      : prepareSharedAttachments([good, bad].map(file => ({ uri: file.uri, name: file.name, mimeType: "image/jpeg" })), [existing]);
    const rejection = expect(pending).rejects.toThrow("attachment_image_decode");
    for (let index = 0; index < 20; index++) await Promise.resolve();
    finish({ uri: converted.uri, width: 100, height: 100 });
    await rejection;
    expect(converted.exists).toBe(false);
    expect(good.exists).toBe(true);
    expect(bad.exists).toBe(true);
    expect(new mockFS.File(existing.localUri as never).exists).toBe(true);
  });

  test("removes an oversized converted output but not the original", async () => {
    const source = fsFile("file:///photo.jpg", { size: 100 });
    const converted = fsFile("file:///oversized.jpg", { size: 11 * 1024 * 1024 });
    mockedManipulate.mockResolvedValue({ uri: converted.uri, width: 100, height: 100 });
    await expect(prepareOne(source)).rejects.toThrow("attachment_compressed_too_large");
    expect(source.exists).toBe(true);
    expect(converted.exists).toBe(false);
  });
});

describe("named export lifecycle", () => {
  test("same-name handoffs keep separate URIs and do not overwrite a file held by another app", async () => {
    const first = fsFile("file:///first.txt", { bytes: new Uint8Array([1]), modTime: 1 });
    const second = fsFile("file:///second.txt", { bytes: new Uint8Array([2]), modTime: 1 });
    const a = await namedExternalFile(first, "notes.txt");
    const b = await namedExternalFile(second, "notes.txt");
    expect(a.uri).not.toBe(b.uri);
    expect(await a.bytes()).toEqual(new Uint8Array([1]));
    expect(await b.bytes()).toEqual(new Uint8Array([2]));
  });

  test("prunes expired handoff copies, not their sources or a recent handoff", async () => {
    const now = jest.spyOn(Date, "now").mockReturnValue(100_000);
    try {
      const source = fsFile("file:///original.txt", { bytes: new Uint8Array([1]) });
      const old = await namedExternalFile(source, "notes.txt");
      now.mockReturnValue(100_000 + 24 * 60 * 60 * 1000);
      const fresh = await namedExternalFile(source, "notes.txt");
      expect(old.exists).toBe(false);
      expect(fresh.exists).toBe(true);
      expect(source.exists).toBe(true);
    } finally { now.mockRestore(); }
  });

  test("refuses capacity overflow rather than removing a recent OS handoff", async () => {
    const source = fsFile("file:///large.txt", { size: 60 * 1024 * 1024 });
    const exported = await namedExternalFile(source, "notes.txt");
    await expect(namedExternalFile(source, "notes.txt")).rejects.toThrow("attachment_export_cache_full");
    expect(exported.exists).toBe(true);
    expect(source.exists).toBe(true);
  });

  test("also bounds tiny or empty exports by entry count", async () => {
    const source = fsFile("file:///empty.txt", { size: 0 });
    for (let index = 0; index < 128; index++) await namedExternalFile(source, "empty.txt");
    await expect(namedExternalFile(source, "empty.txt")).rejects.toThrow("attachment_export_cache_full");
    expect(new FS.Directory("/mock/cache/futureos-exports").list()).toHaveLength(128);
  });

  test("cleans a failed copy and the queue remains usable", async () => {
    const source = fsFile("file:///a.txt", { bytes: new Uint8Array([1]) });
    const copy = jest.spyOn(source, "copy").mockImplementation(async target => {
      mockFS.__set((target as FS.File).uri, { size: 1 });
      throw new Error("disk full");
    });
    await expect(namedExternalFile(source, "a.txt")).rejects.toThrow("disk full");
    expect(new FS.Directory("/mock/cache/futureos-exports").list()).toEqual([]);
    copy.mockRestore();
    expect((await namedExternalFile(source, "a.txt")).exists).toBe(true);
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
  afterEach(() => {
    Platform.OS = "ios";
    Object.assign(Platform, { Version: 0 });
    mockedResolveRoutes.mockResolvedValue(null);
    mockedSupportsAlbumGrid.mockReturnValue(true);
  });

  /** API 33+ always has the system photo picker; older versions may not. */
  function useSystemPhotoPickerDevice(): void {
    Platform.OS = "android";
    Object.assign(Platform, { Version: 33 });
  }

  /** A phone that cannot host a real photo picker (the Huawei report). */
  function useAlbumOnlyDevice(): void {
    Platform.OS = "android";
    Object.assign(Platform, { Version: 31 });
  }

  /** What the native probe reports for this device's pick handlers. */
  function deviceResolves(overrides: Partial<ImagePickRoutes> = {}): void {
    mockedResolveRoutes.mockResolvedValue({
      sdkInt: Number(Platform.Version),
      album: [],
      imageContent: [],
      photoPicker: [],
      photoPickerFallback: [],
      photoPickerPlayServices: [],
      document: [],
      ...overrides,
    });
  }

  function handler(name: string): { package: string; activity: string } {
    return { package: name, activity: `${name}.PickerActivity` };
  }

  test.each(["ios", "android"] as const)(
    "%s opens the system album without requesting full-library access",
    async os => {
      if (os === "android") useSystemPhotoPickerDevice();
      else Platform.OS = os;
      mockedRequestLibrary.mockResolvedValue({ granted: false });
      mockedLaunchLibrary.mockResolvedValue({ canceled: true, assets: [] });
      await pickFromAlbum([]);
      expect(mockedRequestLibrary).not.toHaveBeenCalled();
      expect(mockedLaunchLibrary).toHaveBeenCalledWith(
        expect.objectContaining({ legacy: false, defaultTab: "albums", selectionLimit: 4 }),
      );
    },
  );

  test("Android delegates backport/fallback selection to the native photo contract, never an app resolver", async () => {
    useSystemPhotoPickerDevice();
    mockedLaunchLibrary.mockResolvedValue({ canceled: false, assets: [
      { uri: "file:///album/one.png", mimeType: "image/png" },
      { uri: "file:///album/two.png", mimeType: "image/png" },
    ] });
    for (const name of ["one", "two"]) mockFS.__set(`file:///album/${name}.png`, { bytes: new Uint8Array(10), type: "image/png" });
    const result = await pickFromAlbum([]);
    expect(result.map(item => item.name)).toEqual(["one.png", "two.png"]);
    expect(mockedLaunchLibrary).toHaveBeenCalledWith(expect.objectContaining({ allowsMultipleSelection: true, allowsEditing: false, legacy: false }));
    expect(mockFS.File.pickFileAsync).not.toHaveBeenCalled();
    expect(mockedRequestLibrary).not.toHaveBeenCalled();
  });

  test("Android below 33 opens the phone's gallery instead of the document picker", async () => {
    useAlbumOnlyDevice();
    mockedStartActivity.mockResolvedValue({
      resultCode: -1,
      data: "content://media/external/images/media/42",
    });
    mockFS.__set("content://media/external/images/media/42", {
      bytes: new Uint8Array(10),
      type: "image/png",
    });

    const result = await pickFromAlbum([]);

    expect(mockedStartActivity).toHaveBeenCalledWith("android.intent.action.PICK", {
      data: "content://media/external/images/media",
      type: "image/*",
    });
    expect(mockedLaunchLibrary).not.toHaveBeenCalled();
    expect(mockedRequestLibrary).not.toHaveBeenCalled();
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({
      kind: "image",
      mimeType: "image/png",
      temporary: true,
      name: "42",
    });
  });

  test("resolved routes target the gallery so a file manager cannot answer for it", async () => {
    useAlbumOnlyDevice();
    deviceResolves({
      album: [handler("com.huawei.hidisk"), handler("com.huawei.photos")],
      document: [handler("com.huawei.hidisk")],
    });
    mockedStartActivity.mockResolvedValue({
      resultCode: -1,
      data: "content://media/external/images/media/7",
    });
    mockFS.__set("content://media/external/images/media/7", {
      bytes: new Uint8Array(10),
      type: "image/png",
    });

    await pickFromAlbum([]);

    expect(mockedStartActivity).toHaveBeenCalledWith("android.intent.action.PICK", {
      data: "content://media/external/images/media",
      type: "image/*",
      packageName: "com.huawei.photos",
      className: "com.huawei.photos.PickerActivity",
    });
    expect(mockedLaunchLibrary).not.toHaveBeenCalled();
  });

  test.each([
    ["a file manager that advertises the pick", ["com.huawei.hidisk"]],
    ["no handler at all", []],
  ])("reports the album as unavailable when only %s answers on Android", async (_label, album) => {
    useAlbumOnlyDevice();
    mockedSupportsAlbumGrid.mockReturnValue(false);
    deviceResolves({ album: album.map(handler), document: [handler("com.huawei.hidisk")] });

    await expect(pickFromAlbum([])).rejects.toThrow("attachment_album_unavailable");

    // Never a document picker under the album label — that is "Choose files".
    expect(mockedStartActivity).not.toHaveBeenCalled();
    expect(mockedLaunchLibrary).not.toHaveBeenCalled();
  });

  test("hands an app-drawn album back to the caller when nothing can present one", async () => {
    useAlbumOnlyDevice();
    deviceResolves({ album: [handler("com.huawei.hidisk")], document: [handler("com.huawei.hidisk")] });

    expect(await albumSource()).toBe("inApp");
    // The grid is the caller's surface; a pick cannot start it.
    await expect(pickFromAlbum([])).rejects.toThrow("attachment_album_unavailable");
    expect(mockedStartActivity).not.toHaveBeenCalled();
    expect(mockedLaunchLibrary).not.toHaveBeenCalled();
  });

  test.each([
    ["a gallery", { album: [handler("com.huawei.photos")] }, "system"],
    ["a photo picker", { photoPicker: [handler("com.android.providers.media.module")] }, "system"],
    ["the Play-services photo picker", { photoPickerPlayServices: [handler("com.google.android.gms")] }, "system"],
    ["an image GET_CONTENT gallery", { imageContent: [handler("com.miui.gallery")] }, "system"],
    ["only a file manager", { album: [handler("com.android.documentsui")], imageContent: [handler("com.android.documentsui")] }, "inApp"],
  ])("%s makes the album source %s", async (_label, routes, expected) => {
    useAlbumOnlyDevice();
    deviceResolves(routes);
    expect(await albumSource()).toBe(expected);
  });

  test("an older app binary keeps the API-level default", async () => {
    useAlbumOnlyDevice();
    mockedResolveRoutes.mockResolvedValue(null);
    expect(await albumSource()).toBe("system");
  });

  test("iOS never needs the probe", async () => {
    Platform.OS = "ios";
    expect(await albumSource()).toBe("system");
    expect(mockedResolveRoutes).not.toHaveBeenCalled();
  });

  test("a gallery that answers GET_CONTENT instead of the pick still opens", async () => {
    useAlbumOnlyDevice();
    deviceResolves({ imageContent: [handler("com.miui.gallery")] });
    mockedStartActivity.mockResolvedValue({
      resultCode: -1,
      data: "content://media/external/images/media/9",
    });
    mockFS.__set("content://media/external/images/media/9", {
      bytes: new Uint8Array(10),
      type: "image/png",
    });

    const result = await pickFromAlbum([]);

    expect(mockedStartActivity).toHaveBeenCalledWith("android.intent.action.GET_CONTENT", {
      type: "image/*",
      packageName: "com.miui.gallery",
      className: "com.miui.gallery.PickerActivity",
    });
    expect(result).toHaveLength(1);
  });

  test.each([
    ["com.huawei.photos", true],
    ["com.android.gallery3d", true],
    ["com.miui.gallery", true],
    ["com.google.android.apps.photos", true],
    ["com.sec.android.gallery3d", true],
    ["com.android.documentsui", false],
    ["com.google.android.documentsui", false],
    ["com.coloros.filemanager", false],
    ["com.android.fileexplorer", false],
    ["com.huawei.hidisk", false],
  ])("classifies %s as an album app: %s", async (app, isAlbum) => {
    useAlbumOnlyDevice();
    deviceResolves({ album: [handler(app)] });
    mockedStartActivity.mockResolvedValue({ resultCode: 0 });

    const attempt = pickFromAlbum([]);

    if (isAlbum) {
      await attempt;
      expect(mockedStartActivity).toHaveBeenCalledWith(
        "android.intent.action.PICK",
        expect.objectContaining({ packageName: app }),
      );
    } else {
      await expect(attempt).rejects.toThrow("attachment_album_unavailable");
      expect(mockedStartActivity).not.toHaveBeenCalled();
    }
  });
  test("Android 13 keeps the system photo picker when the device has it", async () => {
    useSystemPhotoPickerDevice();
    deviceResolves({
      photoPicker: [handler("com.google.android.providers.media.module")],
      album: [handler("com.google.android.apps.photos")],
    });
    mockedLaunchLibrary.mockResolvedValue({ canceled: true, assets: [] });

    await pickFromAlbum([]);

    expect(mockedLaunchLibrary).toHaveBeenCalledWith(
      expect.objectContaining({ allowsMultipleSelection: true, legacy: false }),
    );
    expect(mockedStartActivity).not.toHaveBeenCalled();
  });

  test("falls back to the photo-picker backport, not the document picker", async () => {
    useAlbumOnlyDevice();
    deviceResolves({
      photoPickerFallback: [handler("com.android.providers.media.module")],
      album: [handler("com.huawei.hidisk")],
      document: [handler("com.huawei.hidisk")],
    });
    mockedLaunchLibrary.mockResolvedValue({ canceled: true, assets: [] });

    await pickFromAlbum([]);

    expect(mockedLaunchLibrary).toHaveBeenCalledWith(
      expect.objectContaining({ legacy: false, defaultTab: "albums" }),
    );
    expect(mockedStartActivity).not.toHaveBeenCalled();
  });

  test("Android below 33 returns existing attachments when the gallery is cancelled", async () => {
    useAlbumOnlyDevice();
    mockedStartActivity.mockResolvedValue({ resultCode: 0 });
    const existing = [attachment()];
    expect(await pickFromAlbum(existing)).toBe(existing);
    expect(mockedLaunchLibrary).not.toHaveBeenCalled();
  });

  test("reports the album as unavailable when nothing can present it", async () => {
    useAlbumOnlyDevice();
    mockedSupportsAlbumGrid.mockReturnValue(false);
    mockedStartActivity.mockRejectedValue(new Error("No activity found to handle Intent"));

    await expect(pickFromAlbum([])).rejects.toThrow("attachment_album_unavailable");
    expect(mockedLaunchLibrary).not.toHaveBeenCalled();
  });

  test("propagates native picker errors without trying a different app", async () => {
    mockedLaunchLibrary.mockRejectedValueOnce(new Error("No activity found"));
    await expect(pickFromAlbum([])).rejects.toThrow("No activity found");
  });

  test("does not open a picker when the image quota is full", async () => {
    await expect(
      pickFromAlbum(Array.from({ length: 4 }, () => attachment({ kind: "image" }))),
    ).rejects.toThrow("attachment_image_count");
    expect(mockedLaunchLibrary).not.toHaveBeenCalled();
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

describe("loadAlbumImages", () => {
  test("asks for the media permission before reading the library", async () => {
    mockedRequestLibrary.mockResolvedValue({ granted: false });
    await expect(loadAlbumImages()).rejects.toThrow("attachment_album_permission");
    expect(mockedListAlbumImages).not.toHaveBeenCalled();
  });

  test("returns what the phone reports, newest first", async () => {
    mockedRequestLibrary.mockResolvedValue({ granted: true });
    mockedListAlbumImages.mockResolvedValue([
      { uri: "content://media/external/images/media/2", name: "b.jpg", mimeType: "image/jpeg", size: 20, modified: 2 },
      { uri: "content://media/external/images/media/1", name: "a.jpg", mimeType: "image/jpeg", size: 10, modified: 1 },
    ]);

    expect((await loadAlbumImages(5)).map(image => image.name)).toEqual(["b.jpg", "a.jpg"]);
    expect(mockedListAlbumImages).toHaveBeenCalledWith(5);
  });
});

describe("prepareAlbumImages", () => {
  test("copies the chosen images into the cache and keeps their reported names", async () => {
    mockFS.__set("content://media/external/images/media/7", {
      bytes: new Uint8Array(10),
      type: "image/jpeg",
    });
    // A JPEG is re-encoded by the shared pipeline; the album name still wins.
    mockedManipulate.mockResolvedValue({ uri: "file:///cache/re-encoded.jpg" });
    mockFS.__set("file:///cache/re-encoded.jpg", { bytes: new Uint8Array(5) });

    const result = await prepareAlbumImages([], [
      albumImage(7, "IMG_20250925_203012.jpg", "image/jpeg"),
    ]);
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({
      kind: "image",
      mimeType: "image/jpeg",
      name: "IMG_20250925_203012.jpg",
      temporary: true,
    });
  });

  test("keeps every image the user batched in the album", async () => {
    for (const index of [1, 2]) {
      mockFS.__set(`content://media/external/images/media/${index}`, {
        bytes: new Uint8Array(10),
        type: "image/png",
      });
    }

    const result = await prepareAlbumImages(
      [],
      [1, 2].map(index => albumImage(index, `shot-${index}.png`, "image/png")),
    );

    expect(result.map(item => item.name)).toEqual(["shot-1.png", "shot-2.png"]);
  });

  test("rejects a batch past the image quota before copying anything", async () => {
    const existing = Array.from({ length: 4 }, () => attachment({ kind: "image" }));
    await expect(
      prepareAlbumImages(existing, [albumImage(1, "one.jpg", "image/jpeg")]),
    ).rejects.toThrow("attachment_image_count");
  });
});

describe("remainingImageSlots", () => {
  test("counts both the image cap and the attachment cap", () => {
    expect(remainingImageSlots([])).toBe(4);
    expect(remainingImageSlots([attachment({ kind: "image" })])).toBe(3);
    expect(
      remainingImageSlots([
        ...Array.from({ length: 4 }, () => attachment({ kind: "image" })),
        ...Array.from({ length: 6 }, () => attachment()),
      ]),
    ).toBe(0);
  });
});

describe("recoverPendingImagePickerAttachments", () => {
  test("returns existing attachments when Android has no pending result", async () => {
    mockedPendingResult.mockResolvedValue(null);
    const existing = [attachment()];
    expect(await recoverPendingImagePickerAttachments(existing)).toBe(existing);
  });

  test("prepares a pending image after MainActivity reconstruction", async () => {
    mockedPendingResult.mockResolvedValue({
      canceled: false,
      assets: [{ uri: "file:///pending/photo.png", mimeType: "image/png" }],
    });
    mockFS.__set("file:///pending/photo.png", { bytes: new Uint8Array(10), type: "image/png" });

    const result = await recoverPendingImagePickerAttachments([]);
    expect(result[0]).toMatchObject({
      kind: "image",
      mimeType: "image/png",
      name: "photo.png",
    });
  });

  test("surfaces a pending native picker error", async () => {
    mockedPendingResult.mockResolvedValue({ code: "E_PICKER", message: "picker failed" });
    await expect(recoverPendingImagePickerAttachments([])).rejects.toThrow("attachment_failed");
  });
});

describe("prepareSharedAttachments", () => {
  test("counts existing draft images before importing a share", async () => {
    mockFS.__set("file:///share/new.png", { bytes: new Uint8Array(10), type: "image/png" });
    await expect(prepareSharedAttachments(
      [{ uri: "file:///share/new.png", name: "new.png", mimeType: "image/png" }],
      Array.from({ length: 4 }, () => attachment({ kind: "image" })),
    )).rejects.toThrow("attachment_image_count");
    expect(mockedManipulate).not.toHaveBeenCalled();
  });

  test("keeps the name the sending app reported and marks the copy temporary", async () => {
    const file = fsFile("file:///cache/share/0f1-a.png", {
      bytes: new Uint8Array(10),
      type: "image/png",
    });
    // A small non-JPEG image passes through untouched — only the name is
    // re-labelled from the cache copy's generated prefix.
    const result = await prepareSharedAttachments([
      { uri: file.uri, name: "holiday.png", mimeType: "image/png" },
    ]);
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({
      localUri: "file:///cache/share/0f1-a.png",
      name: "holiday.png",
      kind: "image",
      temporary: true,
    });
    expect(result[0]!.transferName).toBeUndefined();
  });

  test("carries the shared name onto a downsampled transfer copy", async () => {
    const file = fsFile("file:///cache/share/0f1-photo.png", {
      bytes: new Uint8Array(10),
      type: "image/png",
    });
    setImageSize({ "file:///cache/share/0f1-photo.png": { width: 4000, height: 2000 } });
    mockedManipulate.mockResolvedValue({ uri: "file:///converted/out.jpg" });
    mockFS.__set("file:///converted/out.jpg", { bytes: new Uint8Array(5) });

    const result = await prepareSharedAttachments([
      { uri: file.uri, name: "screenshot.png", mimeType: "image/png" },
    ]);
    expect(mockedManipulate).toHaveBeenCalled();
    expect(result[0]).toMatchObject({
      name: "screenshot.png",
      transferName: "screenshot.jpg",
      mimeType: "image/jpeg",
      temporary: true,
    });
  });

  test("rejects a file over the size ceiling before anything is staged", async () => {
    const file = fsFile("file:///cache/share/big.bin", {
      bytes: new Uint8Array(1),
      size: 11 * 1024 * 1024,
      type: "application/pdf",
    });
    await expect(
      prepareSharedAttachments([{ uri: file.uri, name: "big.bin", mimeType: "application/pdf" }]),
    ).rejects.toThrow("attachment_file_too_large");
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
  test("cancels instead of sending endless empty chunks if the source shrinks", async () => {
    mockFS.__set("file:///docs/a.txt", { bytes: new Uint8Array([1, 2, 3, 4]) });
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: { uploadId: "u1", chunkBytes: 4 } });
    client.uploadChunk.mockResolvedValue(undefined);
    await expect(uploadAttachments(client as unknown as RemoteClient, [attachment()])).rejects.toThrow("attachment_read_stalled");
    expect(client.uploadChunk).toHaveBeenCalledTimes(1);
    expect(client.request).toHaveBeenCalledWith({ type: "upload_cancel", transferId: "u1" }, "transfer");
    expect(client.requestRetry).not.toHaveBeenCalled();
  });

  test.each([0, -1, NaN, Infinity])("rejects invalid chunk size %s", async chunkBytes => {
    mockFS.__set("file:///docs/a.txt", { bytes: new Uint8Array(8) });
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: { uploadId: "u1", chunkBytes } });
    await expect(uploadAttachments(client as unknown as RemoteClient, [attachment()])).rejects.toThrow("invalid_upload_chunk_size");
    expect(client.uploadChunk).not.toHaveBeenCalled();
  });

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

  const cacheClient = mockClient() as unknown as RemoteClient;

  function cacheUri(i: DownloadInfo): string {
    return `/mock/cache/futureos-previews/${i.contentHash}.jpg`;
  }

  test("10 MiB hashing is incremental, yields, and matches the independent SHA-256 oracle", async () => {
    jest.useFakeTimers();
    const bytes = new Uint8Array(10 * 1024 * 1024).fill(37);
    mockFS.__set("/large.bin", { bytes });
    const sizes: number[] = [];
    mockFS.__onRead(size => sizes.push(size));
    const source = new FS.File("/large.bin");
    const whole = jest.spyOn(source, "bytes").mockRejectedValue(new Error("whole-file read"));
    try {
      const result = fileSha256(source);
      await Promise.resolve();
      expect(sizes).toEqual(Array(4).fill(64 * 1024));
      await jest.runAllTimersAsync();
      expect(await result).toBe(sha256Hex(bytes));
      expect(Math.max(...sizes)).toBe(64 * 1024);
      expect(whole).not.toHaveBeenCalled();
    } finally { whole.mockRestore(); jest.useRealTimers(); }
  });

  test("native hashing never copies file bytes into JS and cancellation fences its result", async () => {
    const bytes = new Uint8Array([1, 2, 3]);
    const source = new FS.File("/native.bin");
    const read = jest.spyOn(source, "open");
    jest.mocked(nativeFileSha256).mockResolvedValueOnce(sha256Hex(bytes));
    expect(await fileSha256(source)).toBe(sha256Hex(bytes));
    expect(read).not.toHaveBeenCalled();
    const controller = new AbortController();
    jest.mocked(nativeFileSha256).mockImplementationOnce(async () => { controller.abort(); return sha256Hex(bytes); });
    await expect(fileSha256(source, controller.signal)).rejects.toThrow("transfer_cancelled");
    read.mockRestore();
  });

  test("downloads four independent chunks at a time but writes out-of-order replies in order", async () => {
    const bytes = new Uint8Array(Array.from({ length: 20 }, (_, i) => i));
    const metadata = { ...info, size: bytes.length, contentHash: sha256Hex(bytes) };
    const client = mockClient();
    const finish = new Map<number, (bytes: Uint8Array) => void>();
    client.downloadChunk.mockImplementation((_id: string, index: number) => new Promise<Uint8Array>(resolve => finish.set(index, resolve)));
    client.request.mockResolvedValue({ success: true, data: {} });
    const result = downloadPrepared(client as unknown as RemoteClient, metadata);
    for (let n = 0; n < 20; n++) await Promise.resolve();
    expect(client.downloadChunk).toHaveBeenCalledTimes(4);
    for (const index of [3, 2, 1, 0]) finish.get(index)!(bytes.slice(index * 4, index * 4 + 4));
    for (let n = 0; n < 30; n++) await Promise.resolve();
    expect(client.downloadChunk).toHaveBeenCalledTimes(5);
    finish.get(4)!(bytes.slice(16));
    expect(await (await result).bytes()).toEqual(bytes);
  });

  test("prepareDownload returns the prepared info when not cached", async () => {
    const client = mockClient();
    client.request.mockResolvedValue({ success: true, data: info });
    const history: HistoryAttachment = { path: "/tmp/a.jpg", name: "a.jpg" };
    expect(await prepareDownload(client as unknown as RemoteClient, "s1", history)).toBe(info);
    expect(client.request).toHaveBeenCalledWith(
      // The name travels with the request: the desktop would otherwise read the
      // session's whole history to look the attachment up, per file open (and
      // per inline image), to re-derive a string this phone already displays.
      expect.objectContaining({
        type: "download_prepare",
        sessionId: "s1",
        filePath: "/tmp/a.jpg",
        mode: "preview",
        name: "a.jpg",
      }),
      "s1",
      10_000,
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
      10_000,
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
    expect(client.request.mock.calls[0]![2]).toBe(10_000);
    expect(client.request.mock.calls[1]![2]).toBe(20_000);
    expect(client.recoverAfterTransientRequest).toHaveBeenCalledTimes(1);
  });

  test("prepareDownload does not retry a non-transient failure", async () => {
    const client = mockClient();
    client.request.mockRejectedValue(new Error("not an attachment"));
    client.recoverAfterTransientRequest.mockResolvedValue(false);

    await expect(
      prepareDownload(client as unknown as RemoteClient, "s1", {
        path: "/tmp/a.jpg",
        name: "a.jpg",
      }),
    ).rejects.toThrow("not an attachment");
    expect(client.request).toHaveBeenCalledTimes(1);
  });

  test("cachedDownload returns only a present size-matching cache candidate", () => {
    expect(cachedDownload(info)).toBeNull();
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    expect(cachedDownload(info)).not.toBeNull();
    // Wrong size → treated as not cached.
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(7) });
    expect(cachedDownload(info)).toBeNull();
  });

  test("uses the original name when materializing a file for the system share sheet", async () => {
    const source = fsFile(cacheUri(info), { bytes: new Uint8Array([1, 2, 3]) });
    const named = await namedExternalFile(source, "/workspace/results/experiment.csv");
    expect(named.uri).toMatch(/\/futureos-exports\/\d+-export-1\/experiment\.csv$/);
    expect(await named.bytes()).toEqual(new Uint8Array([1, 2, 3]));
  });

  test("rememberPreparedPreview + cachedPreviewForAttachment round-trip", () => {
    const attachment: HistoryAttachment = { path: "/tmp/a.jpg", name: "a.jpg" };
    expect(cachedPreviewForAttachment(cacheClient, "s1", attachment)).toBeNull();

    // No backing file yet — the bounded metadata survives until download.
    rememberPreparedPreview(cacheClient, "s1", attachment, info);
    expect(cachedPreviewForAttachment(cacheClient, "s1", attachment)).toBeNull();

    // The completed download becomes reusable without another prepare RPC.
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    const preview = cachedPreviewForAttachment(cacheClient, "s1", attachment);
    expect(preview?.info).toBe(info);

    // A pruned backing file is still a cache miss.
    mockFS.__reset();
    expect(cachedPreviewForAttachment(cacheClient, "s1", attachment)).toBeNull();
  });

  test("keeps preview and original cache entries independent", () => {
    const attachment: HistoryAttachment = { path: "/tmp/a.jpg", name: "a.jpg" };
    const original: DownloadInfo = {
      ...info,
      contentHash: "original123",
      previewKind: "file",
      variant: "original",
    };
    rememberPreparedPreview(cacheClient, "s1", attachment, info);
    rememberPreparedPreview(cacheClient, "s1", attachment, original);
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    mockFS.__set("/mock/cache/futureos-previews/original123.jpg", {
      bytes: new Uint8Array(8),
    });
    expect(cachedPreviewForAttachment(cacheClient, "s1", attachment, "preview")?.info).toBe(info);
    expect(cachedPreviewForAttachment(cacheClient, "s1", attachment, "original")?.info).toBe(original);
  });

  test("identical paths cannot reuse another desktop or session's prepared metadata", () => {
    const attachment = { path: "results/chart.jpg", name: "chart.jpg" };
    const otherClient = mockClient() as unknown as RemoteClient;
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    rememberPreparedPreview(cacheClient, "s1", attachment, info);
    expect(cachedPreviewForAttachment(cacheClient, "s1", attachment)?.info).toBe(info);
    expect(cachedPreviewForAttachment(otherClient, "s1", attachment)).toBeNull();
    expect(cachedPreviewForAttachment(cacheClient, "s2", attachment)).toBeNull();
  });

  test("bounds prepared metadata even while all files remain cached", () => {
    const client = mockClient() as unknown as RemoteClient;
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    for (let index = 0; index < 129; index++) {
      rememberPreparedPreview(client, "s1", { path: `${index}.jpg`, name: "a.jpg" }, info);
    }
    expect(cachedPreviewForAttachment(client, "s1", { path: "0.jpg", name: "a.jpg" })).toBeNull();
    expect(cachedPreviewForAttachment(client, "s1", { path: "128.jpg", name: "a.jpg" })).not.toBeNull();
  });

  test("an already-cancelled prepare does not start a desktop request", async () => {
    const client = mockClient();
    const controller = new AbortController();
    controller.abort();
    await expect(prepareDownload(client as unknown as RemoteClient, "s1", {
      path: "a.jpg", name: "a.jpg",
    }, "preview", controller.signal)).rejects.toThrow("transfer_cancelled");
    expect(client.request).not.toHaveBeenCalled();
  });

  test("cancels a transfer created after the prepare caller has already cancelled", async () => {
    const client = mockClient();
    const controller = new AbortController();
    let resolve!: (result: { data: DownloadInfo }) => void;
    client.request.mockReturnValueOnce(new Promise(yes => { resolve = yes; }))
      .mockResolvedValue({ data: {} });
    const pending = prepareDownload(client as unknown as RemoteClient, "s1", {
      path: "a.jpg", name: "a.jpg",
    }, "preview", controller.signal);
    controller.abort();
    await expect(pending).rejects.toThrow("transfer_cancelled");
    resolve({ data: info });
    await Promise.resolve();
    expect(client.request).toHaveBeenCalledWith({ type: "download_cancel", transferId: "t1" }, "transfer");
  });

  test.each([0, -1, NaN, Infinity, 0.5])("rejects invalid download chunk size %s before any disk write", async chunkBytes => {
    const client = mockClient();
    client.request.mockResolvedValue({ data: {} });
    await expect(downloadPrepared(client as unknown as RemoteClient, { ...info, chunkBytes }))
      .rejects.toThrow("invalid_download_size");
    expect(client.downloadChunk).not.toHaveBeenCalled();
    expect(cachedDownload(info)).toBeNull();
    expect(client.request).toHaveBeenCalledWith({ type: "download_cancel", transferId: "t1" }, "transfer");
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
    expect(client.downloadChunk).toHaveBeenCalledTimes(2); // Initial bounded wave, no retry.
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared cancels an in-flight chunk request immediately", async () => {
    const i: DownloadInfo = { ...info, size: 4, chunkBytes: 4, contentHash: "x" };
    const client = mockClient();
    const controller = new AbortController();
    let resolveChunk: ((value: Uint8Array) => void) | undefined;
    client.downloadChunk.mockImplementation(
      () =>
        new Promise<Uint8Array>(resolve => {
          resolveChunk = resolve;
        }),
    );
    client.request.mockResolvedValue({ success: true, data: {} });

    const download = downloadPrepared(
      client as unknown as RemoteClient,
      i,
      undefined,
      controller.signal,
    );
    await Promise.resolve();
    controller.abort();

    await expect(download).rejects.toThrow("transfer_cancelled");
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
    resolveChunk?.(new Uint8Array([1, 2, 3, 4]));
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
    mockFS.__onRead(() => { throw new Error("digest boom"); });

    await expect(downloadPrepared(client as unknown as RemoteClient, i)).rejects.toThrow(
      "digest boom",
    );
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared aborts during the retry backoff timer", async () => {
    const i: DownloadInfo = { ...info, size: 8, contentHash: "x" };
    const client = mockClient();
    const controller = new AbortController();
    client.downloadChunk.mockRejectedValue(new Error("not_connected"));
    client.request.mockResolvedValue({ success: true, data: {} });

    await expect(
      downloadPrepared(client as unknown as RemoteClient, i, undefined, controller.signal, () =>
        setTimeout(() => controller.abort(), 0),
      ),
    ).rejects.toThrow("transfer_cancelled");
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("prepareDownload cancels when aborted just after the response arrives", async () => {
    const client = mockClient();
    const controller = new AbortController();
    let resolveRequest: ((value: unknown) => void) | undefined;
    client.request.mockImplementation(
      () =>
        new Promise(resolve => {
          resolveRequest = resolve;
        }),
    );

    const pending = prepareDownload(
      client as unknown as RemoteClient,
      "s1",
      { path: "/tmp/a.jpg", name: "a.jpg" },
      "preview",
      controller.signal,
    );
    // Resolve the prepare RPC, then abort in a microtask that runs after
    // abortable's success handler but before prepareDownload resumes, so the
    // defensive post-response abort check observes the cancellation.
    resolveRequest!({ success: true, data: info });
    queueMicrotask(() => controller.abort());

    await expect(pending).rejects.toThrow("transfer_cancelled");
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("prepareDownload cancels when aborted during cache verification", async () => {
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    const client = mockClient();
    const controller = new AbortController();
    client.request.mockResolvedValue({ success: true, data: info });
    mockFS.__onRead(() => controller.abort());

    await expect(
      prepareDownload(
        client as unknown as RemoteClient,
        "s1",
        { path: "/tmp/a.jpg", name: "a.jpg" },
        "preview",
        controller.signal,
      ),
    ).rejects.toThrow("transfer_cancelled");
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared cancels when aborted during cache verification", async () => {
    mockFS.__set(cacheUri(info), { bytes: new Uint8Array(8) });
    const client = mockClient();
    const controller = new AbortController();
    client.request.mockResolvedValue({ success: true, data: {} });
    mockFS.__onRead(() => controller.abort());

    await expect(
      downloadPrepared(client as unknown as RemoteClient, info, undefined, controller.signal),
    ).rejects.toThrow("transfer_cancelled");
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });

  test("downloadPrepared cancels when aborted on the final progress callback", async () => {
    const fileBytes = new Uint8Array([1, 2, 3, 4]);
    const i: DownloadInfo = { ...info, size: 4, chunkBytes: 4, contentHash: sha256Hex(fileBytes) };
    const client = mockClient();
    client.downloadChunk.mockResolvedValue(fileBytes);
    client.request.mockResolvedValue({ success: true, data: {} });
    const controller = new AbortController();

    await expect(
      downloadPrepared(
        client as unknown as RemoteClient,
        i,
        () => controller.abort(),
        controller.signal,
      ),
    ).rejects.toThrow("transfer_cancelled");
    expect(client.request).toHaveBeenCalledWith(
      { type: "download_cancel", transferId: "t1" },
      "transfer",
    );
  });
});
