import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import { Alert, Platform } from "react-native";
import * as Sharing from "expo-sharing";
import * as LegacyFileSystem from "expo-file-system/legacy";
import { File } from "expo-file-system";
import { findSupportedMimeType, openFile, saveFile, supportsNativeFileActions } from "future-file-handler";
import { nativePresentationInFlight } from "../../../remote/nativePresentation";
import { useFileDownload } from "../useFileDownload";
import { TransferCancelledError } from "../../../remote/files";
import type { DownloadInfo } from "../../../remote/types";

jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("future-file-handler", () => ({
  openFile: jest.fn(), saveFile: jest.fn(), findSupportedMimeType: jest.fn(), supportsNativeFileActions: jest.fn(),
}));
jest.mock("expo-sharing", () => ({ shareAsync: jest.fn(), isAvailableAsync: jest.fn() }));
jest.mock("expo-file-system", () => ({
  File: jest.fn().mockImplementation((uri: string) => ({ uri, size: 3 })),
}));
jest.mock("expo-file-system/legacy", () => ({
  StorageAccessFramework: {
    requestDirectoryPermissionsAsync: jest.fn(),
    createFileAsync: jest.fn(),
    writeAsStringAsync: jest.fn(),
  },
  readAsStringAsync: jest.fn(),
  EncodingType: { Base64: "base64" },
}));
jest.mock("../../../remote/files", () => ({
  MAX_FILE_BYTES: 10 * 1024 * 1024,
  namedExternalFile: jest.fn(async (_file: unknown, name: string) => ({ uri: `file:///cache/named/${name}` })),
  mimeFor: jest.fn(() => "application/pdf"),
  TransferCancelledError: class extends Error { constructor() { super("transfer_cancelled"); } },
}));
jest.mock("expo-network", () => ({
  getNetworkStateAsync: jest.fn(async () => ({ type: "WIFI" })),
  NetworkStateType: { CELLULAR: "CELLULAR", UNKNOWN: "UNKNOWN" },
}));

const info: DownloadInfo = {
  transferId: "transfer", name: "notes.txt", mimeType: "text/plain", size: 3,
  contentHash: "new-content", previewKind: "text", variant: "preview", chunkBytes: 1024,
};
const file = { uri: "file:///cache/notes.txt", bytes: async () => new TextEncoder().encode("new") };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}

async function mount(remote: unknown) {
  let api!: ReturnType<typeof useFileDownload>;
  let tree!: ReactTestRenderer;
  const t = ((key: string) => key) as TFunction;
  function Harness() {
    api = useFileDownload(remote as Parameters<typeof useFileDownload>[0], t, jest.fn());
    return null;
  }
  await act(async () => { tree = create(createElement(Harness)); });
  return { get api() { return api; }, tree };
}

beforeEach(() => {
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
});
afterEach(() => jest.restoreAllMocks());

const pdfInfo: DownloadInfo = {
  ...info, name: "报告.pdf", mimeType: "application/pdf", previewKind: "file", variant: "original",
};
const localFile = file as File;

describe("independent file operations", () => {
  const platform = Platform.OS;
  beforeEach(() => {
    jest.clearAllMocks();
    Platform.OS = "android";
    jest.mocked(findSupportedMimeType).mockResolvedValue(null);
    jest.mocked(Sharing.isAvailableAsync).mockResolvedValue(true);
    jest.mocked(Sharing.shareAsync).mockResolvedValue();
    jest.mocked(openFile).mockResolvedValue();
    jest.mocked(saveFile).mockResolvedValue();
    jest.mocked(supportsNativeFileActions).mockReturnValue(false);
    jest.mocked(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync)
      .mockResolvedValue({ granted: true, directoryUri: "content://documents/tree/downloads" });
    jest.mocked(LegacyFileSystem.StorageAccessFramework.createFileAsync)
      .mockResolvedValue("content://documents/report.pdf");
    jest.mocked(LegacyFileSystem.readAsStringAsync).mockResolvedValue("cGRm");
  });
  afterEach(() => { Platform.OS = platform; });

  test.each(["attachment", "link", "local"])("%s exposes PDF actions without requiring a reader", async source => {
    const remote = {
      cachedAttachment: jest.fn(() => ({ info: pdfInfo, file })),
      prepareAttachment: jest.fn(async () => pdfInfo),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    await act(async () => {
      if (source === "link") await h.api.openFileLink("/报告.pdf");
      else await h.api.openAttachment({ path: source === "local" ? "file:///cache/报告.pdf" : "/报告.pdf", name: "报告.pdf" });
    });
    expect(h.api.fileAction?.info.name).toBe("报告.pdf");
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("sharing uses the actual named file and MIME without a VIEW query", async () => {
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(Sharing.shareAsync).toHaveBeenCalledWith("file:///cache/named/报告.pdf", {
      mimeType: "application/pdf", dialogTitle: "attachment.share",
    });
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    expect(openFile).not.toHaveBeenCalled();
    expect(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("saving uses SAF without a VIEW query or share sheet", async () => {
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "save"); });
    expect(LegacyFileSystem.StorageAccessFramework.createFileAsync).toHaveBeenCalledWith(
      "content://documents/tree/downloads", "报告.pdf", "application/pdf",
    );
    expect(LegacyFileSystem.StorageAccessFramework.writeAsStringAsync).toHaveBeenCalledWith(
      "content://documents/report.pdf", "cGRm", { encoding: "base64" },
    );
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("missing reader only blocks open; sharing afterward still works", async () => {
    const remote = { downloadAttachment: jest.fn() };
    const h = await mount(remote);
    await act(async () => { await h.api.openOrShare(pdfInfo, null, "open"); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.noHandler");
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(openFile).not.toHaveBeenCalled();
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(Sharing.shareAsync).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
  });

  test("open hands a PDF to the VIEW chooser, not SEND", async () => {
    jest.mocked(findSupportedMimeType).mockResolvedValue("application/pdf");
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "open"); });
    expect(openFile).toHaveBeenCalledWith("file:///cache/named/报告.pdf", "application/pdf");
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("sharing from a preview requests the original, not rendered/truncated bytes", async () => {
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => ({ ...info, variant: "original" })),
      downloadAttachment: jest.fn(async () => localFile),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.downloadOriginal({ path: "/notes.txt", name: "notes.txt" }, "share"); });
    expect(remote.prepareAttachment).toHaveBeenCalledWith(
      { path: "/notes.txt", name: "notes.txt" }, "original", expect.anything(), expect.any(Function),
    );
    expect(Sharing.shareAsync).toHaveBeenCalledWith("file:///cache/named/notes.txt", {
      mimeType: "text/plain", dialogTitle: "attachment.share",
    });
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("sharing a just-picked local file does not request a desktop transfer", async () => {
    const remote = { prepareAttachment: jest.fn(), downloadAttachment: jest.fn() };
    const h = await mount(remote);
    await act(async () => { await h.api.downloadOriginal({ path: "file:///cache/报告.pdf", name: "报告.pdf" }, "share"); });
    expect(remote.prepareAttachment).not.toHaveBeenCalled();
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(Sharing.shareAsync).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
  });

  test("unavailable sharing is reported before downloading", async () => {
    jest.mocked(Sharing.isAvailableAsync).mockResolvedValue(false);
    const remote = { downloadAttachment: jest.fn() };
    const h = await mount(remote);
    await act(async () => { await h.api.openOrShare(pdfInfo, null, "share"); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.shareUnavailable");
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("oversized originals cannot be shared", async () => {
    const remote = { downloadAttachment: jest.fn() };
    const h = await mount(remote);
    await act(async () => { await h.api.openOrShare({ ...pdfInfo, size: 10 * 1024 * 1024 + 1 }, null, "share"); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.tooLarge");
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelled downloads never launch a late share", async () => {
    const download = deferred<File>();
    const h = await mount({ downloadAttachment: () => download.promise });
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare(pdfInfo, null, "share"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { download.resolve(localFile); await sharing; });
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("native sharing gets the bounded foreground grace and releases it on return", async () => {
    const sheet = deferred<void>();
    jest.mocked(Sharing.shareAsync).mockReturnValueOnce(sheet.promise);
    const h = await mount({});
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(nativePresentationInFlight()).toBe(true);
    await act(async () => { sheet.resolve(); await sharing; });
    expect(nativePresentationInFlight()).toBe(false);
    act(() => h.tree.unmount());
  });

  test.each(["open", "save"] as const)("iOS %s uses its native action without depending on sharing", async operation => {
    Platform.OS = "ios";
    jest.mocked(supportsNativeFileActions).mockReturnValue(true);
    jest.mocked(Sharing.isAvailableAsync).mockResolvedValue(false);
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, operation); });
    if (operation === "open") {
      expect(openFile).toHaveBeenCalledWith("file:///cache/named/报告.pdf", "application/pdf");
      expect(saveFile).not.toHaveBeenCalled();
    } else {
      expect(saveFile).toHaveBeenCalledWith("file:///cache/named/报告.pdf");
      expect(openFile).not.toHaveBeenCalled();
    }
    expect(Sharing.isAvailableAsync).not.toHaveBeenCalled();
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test.each(["open", "save"] as const)("older iOS native builds retain the %s fallback", async operation => {
    Platform.OS = "ios";
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, operation); });
    expect(Sharing.shareAsync).toHaveBeenCalledTimes(1);
    expect(openFile).not.toHaveBeenCalled();
    expect(saveFile).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test.each(["open", "save", "share"] as const)("iOS %s waits for the download modal dismissal", async operation => {
    Platform.OS = "ios";
    jest.mocked(supportsNativeFileActions).mockReturnValue(true);
    const download = deferred<File>();
    const h = await mount({ downloadAttachment: () => download.promise });
    let transfer!: Promise<void>;
    await act(async () => { transfer = h.api.openOrShare(pdfInfo, null, operation); });
    act(() => h.api.onDownloadModalShow());
    await act(async () => { download.resolve(localFile); await transfer; });
    expect(h.api.activeDownload).toBeNull();
    const action = operation === "save" ? saveFile : operation === "open" ? openFile : Sharing.shareAsync;
    expect(action).not.toHaveBeenCalled();
    await act(async () => { h.api.flushPendingDownloadModal(); });
    expect(action).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
  });

  test("iOS sharing waits for the download modal dismissal", async () => {
    Platform.OS = "ios";
    const download = deferred<File>();
    const h = await mount({ downloadAttachment: () => download.promise });
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare(pdfInfo, null, "share"); });
    act(() => h.api.onDownloadModalShow());
    await act(async () => { download.resolve(localFile); await sharing; });
    expect(h.api.activeDownload).toBeNull();
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    await act(async () => { h.api.flushPendingDownloadModal(); });
    expect(Sharing.shareAsync).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
  });
});

test.each([true, undefined])("opening a mutable file revalidates metadata (explicit refresh: %s)", async refresh => {
  const remote = {
    cachedAttachment: jest.fn(() => ({ info, file })),
    prepareAttachment: jest.fn(async () => info),
    downloadAttachment: jest.fn(),
  };
  const h = await mount(remote);
  await act(async () => { await h.api.openFileLink("C:\\work\\notes.txt", refresh); });
  expect(remote.prepareAttachment).toHaveBeenCalledWith(
    { path: "C:\\work\\notes.txt", name: "notes.txt" }, "preview", expect.anything(), expect.any(Function),
  );
  expect(remote.cachedAttachment).toHaveBeenCalledTimes(1);
  expect(remote.downloadAttachment).not.toHaveBeenCalled();
  expect(h.api.preview).toMatchObject({ text: "new", info: { contentHash: "new-content" } });
  act(() => h.tree.unmount());
});

test.each(["preview", "original"])("a cancelled %s preparation releases the download lane", async variant => {
  const remote = {
    cachedAttachment: jest.fn(() => null),
    prepareAttachment: jest.fn().mockRejectedValue(new TransferCancelledError()),
  };
  const h = await mount(remote);
  const open = () => variant === "original"
    ? h.api.downloadOriginal({ path: "/notes.txt", name: "notes.txt" })
    : h.api.openFileLink("/notes.txt");
  await act(async () => { await open(); });
  await act(async () => { await open(); });
  expect(remote.prepareAttachment).toHaveBeenCalledTimes(2);
  expect(Alert.alert).not.toHaveBeenCalled();
  act(() => h.tree.unmount());
});

test("late file reads after cancellation cannot open a preview or steal a newer download", async () => {
  const bytes = deferred<Uint8Array>();
  const remote = {
    prepareAttachment: jest.fn(async () => info),
    cachedAttachment: jest.fn(() => ({ info, file: { ...file, bytes: () => bytes.promise } })),
  };
  const h = await mount(remote);
  let opening!: Promise<void>;
  await act(async () => { opening = h.api.openFileLink("/notes.txt"); });
  act(() => h.api.cancelActiveDownload());
  remote.cachedAttachment.mockReturnValue({ info, file });
  await act(async () => { await h.api.openFileLink("/other.txt"); });
  expect(h.api.preview?.attachment.path).toBe("/other.txt");
  await act(async () => { bytes.resolve(new TextEncoder().encode("old")); await opening; });
  expect(h.api.preview?.attachment.path).toBe("/other.txt");
  expect(h.api.preview?.text).toBe("new");
  act(() => h.tree.unmount());
});

test("late reads after unmount cannot display an error alert", async () => {
  const bytes = deferred<Uint8Array>();
  const remote = {
    prepareAttachment: jest.fn(async () => info),
    cachedAttachment: jest.fn(() => ({ info, file: { ...file, bytes: () => bytes.promise.then(() => { throw new Error("read failed"); }) } })),
  };
  const h = await mount(remote);
  let opening!: Promise<void>;
  await act(async () => { opening = h.api.openFileLink("/notes.txt"); });
  act(() => h.tree.unmount());
  await act(async () => { bytes.resolve(new Uint8Array()); await opening; });
  expect(Alert.alert).not.toHaveBeenCalled();
});

test("cancelling an iOS dismissal handoff prevents its queued preview", async () => {
  const platform = Platform.OS;
  Object.defineProperty(Platform, "OS", { configurable: true, value: "ios" });
  const downloaded = deferred<typeof file>();
  const remote = {
    cachedAttachment: jest.fn(() => null),
    prepareAttachment: jest.fn(async () => info),
    downloadAttachment: jest.fn(() => downloaded.promise),
  };
  const h = await mount(remote);
  try {
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openFileLink("/notes.txt"); });
    expect(h.api.activeDownload).not.toBeNull();
    act(() => h.api.onDownloadModalShow());
    await act(async () => { downloaded.resolve(file); await opening; });
    expect(h.api.activeDownload).toBeNull();
    expect(h.api.preview).toBeNull();
    act(() => h.api.cancelActiveDownload());
    act(() => h.api.flushPendingDownloadModal());
    expect(h.api.preview).toBeNull();
  } finally {
    act(() => h.tree.unmount());
    Object.defineProperty(Platform, "OS", { configurable: true, value: platform });
  }
});
