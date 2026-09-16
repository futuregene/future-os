import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import { Platform } from "react-native";
import { AppAlert as Alert } from "../../../components/appAlerts";
import * as Sharing from "expo-sharing";
import * as Network from "expo-network";
import * as downloadUtils from "../utils";
import * as LegacyFileSystem from "expo-file-system/legacy";
import { File } from "expo-file-system";
import { findSupportedMimeType, openFile, saveFile, shareFile, supportsNativeFileActions } from "future-file-handler";
import { nativePresentationInFlight } from "../../../remote/nativePresentation";
import { useFileDownload } from "../useFileDownload";
import { namedExternalFile, TransferCancelledError } from "../../../remote/files";
import type { DownloadInfo } from "../../../remote/types";

jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
// Keep these tests focused on async modal/cancellation ordering. Native bounded
// reads, byte limits, UTF-8 boundaries and handle cleanup have dedicated tests.
jest.mock("../readPreviewText", () => ({
  readPreviewText: jest.fn(async (file: { bytes(): Promise<Uint8Array> }) => ({
    text: new TextDecoder().decode(await file.bytes()), truncated: false,
  })),
}));
jest.mock("future-file-handler", () => ({
  openFile: jest.fn(), saveFile: jest.fn(), shareFile: jest.fn(), findSupportedMimeType: jest.fn(), supportsNativeFileActions: jest.fn(),
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
  NetworkStateType: { WIFI: "WIFI", ETHERNET: "ETHERNET", CELLULAR: "CELLULAR", UNKNOWN: "UNKNOWN" },
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

describe("download confirmation policy across entry points", () => {
  const platform = Platform.OS;
  beforeEach(() => {
    jest.clearAllMocks();
    jest.useFakeTimers();
    Platform.OS = "android";
    jest.mocked(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync)
      .mockResolvedValue({ granted: false });
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    jest.useRealTimers();
    Platform.OS = platform;
    jest.mocked(Network.getNetworkStateAsync).mockResolvedValue({ type: Network.NetworkStateType.WIFI });
  });

  test.each(["attachment", "link", "original", "save"])("%s skips small-file prompts but respects a large-file cancellation", async entry => {
    const confirm = jest.spyOn(downloadUtils, "confirmDownload").mockResolvedValue(false);
    for (const type of [Network.NetworkStateType.CELLULAR, Network.NetworkStateType.UNKNOWN]) {
      for (const size of [5 * 1024, 1024 * 1024]) {
        confirm.mockClear();
        jest.mocked(Network.getNetworkStateAsync).mockResolvedValue({ type });
        const metadata = { ...info, size };
        const remote = {
          cachedAttachment: jest.fn(() => null),
          prepareAttachment: jest.fn(async () => metadata),
          downloadAttachment: jest.fn(async () => localFile),
        };
        const h = await mount(remote);
        await act(async () => {
          if (entry === "attachment") await h.api.openAttachment({ path: "/notes.txt", name: "notes.txt" });
          else if (entry === "link") await h.api.openFileLink("/notes.txt");
          else if (entry === "original") await h.api.downloadOriginal({ path: "/notes.txt", name: "notes.txt" });
          else await h.api.openOrShare(metadata, null, "save");
        });
        if (size < 1024 * 1024) {
          expect(confirm).not.toHaveBeenCalled();
          expect(remote.downloadAttachment).toHaveBeenCalledTimes(1);
        } else {
          expect(confirm).toHaveBeenCalledWith("attachment.downloadTitle",
            type === Network.NetworkStateType.CELLULAR ? "attachment.cellularWarning" : "attachment.unknownNetworkWarning",
            "chat.cancel", "attachment.download");
          expect(remote.downloadAttachment).not.toHaveBeenCalled();
          expect(h.api.activeDownload).toBeNull();
        }
        act(() => jest.runOnlyPendingTimers());
        act(() => h.tree.unmount());
      }
    }
  });
});

describe("independent file operations", () => {
  const platform = Platform.OS;
  beforeEach(() => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    Platform.OS = "android";
    jest.mocked(findSupportedMimeType).mockResolvedValue(null);
    jest.mocked(Sharing.isAvailableAsync).mockResolvedValue(true);
    jest.mocked(Sharing.shareAsync).mockResolvedValue();
    jest.mocked(openFile).mockResolvedValue();
    jest.mocked(saveFile).mockResolvedValue();
    jest.mocked(shareFile).mockResolvedValue();
    jest.mocked(supportsNativeFileActions).mockReturnValue(false);
    jest.mocked(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync)
      .mockResolvedValue({ granted: true, directoryUri: "content://documents/tree/downloads" });
    jest.mocked(LegacyFileSystem.StorageAccessFramework.createFileAsync)
      .mockResolvedValue("content://documents/report.pdf");
    jest.mocked(LegacyFileSystem.readAsStringAsync).mockResolvedValue("cGRm");
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    jest.useRealTimers();
    Platform.OS = platform;
  });

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

  test.each([
    ["报告.doc", "application/msword"],
    ["报告.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"],
    ["示例销售数据.xls", "application/vnd.ms-excel"],
    ["示例销售数据.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"],
  ])("generated Office file %s offers actions and can save/share without an installed reader", async (name, mimeType) => {
    const metadata: DownloadInfo = { ...pdfInfo, name, mimeType };
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => metadata),
      downloadAttachment: jest.fn(async () => localFile),
    };
    const h = await mount(remote);
    const path = `C:\\work\\${name}`;
    await act(async () => { await h.api.openFileLink(path); });
    expect(remote.prepareAttachment).toHaveBeenLastCalledWith(
      { path, name }, "original", expect.any(AbortSignal), expect.any(Function),
    );
    expect(h.api.fileAction?.info).toEqual(metadata);
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    await act(async () => { await h.api.openOrShare(metadata, null, "save"); });
    expect(LegacyFileSystem.StorageAccessFramework.createFileAsync).toHaveBeenLastCalledWith(
      "content://documents/tree/downloads", name, mimeType,
    );
    expect(LegacyFileSystem.StorageAccessFramework.writeAsStringAsync).toHaveBeenCalled();
    await act(async () => { await h.api.openOrShare(metadata, null, "share"); });
    expect(shareFile).toHaveBeenLastCalledWith(`file:///cache/named/${name}`, mimeType, "attachment.share");
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    jest.mocked(findSupportedMimeType).mockResolvedValueOnce(mimeType);
    await act(async () => { await h.api.openOrShare(metadata, null, "open"); });
    expect(openFile).toHaveBeenLastCalledWith(`file:///cache/named/${name}`, mimeType);
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("sharing uses the actual named file and MIME without a VIEW query", async () => {
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(shareFile).toHaveBeenCalledWith("file:///cache/named/报告.pdf", "application/pdf", "attachment.share");
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    expect(Sharing.isAvailableAsync).not.toHaveBeenCalled();
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    expect(openFile).not.toHaveBeenCalled();
    expect(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test.each(["open", "share"] as const)("Android %s preserves native failure details", async operation => {
    jest.mocked(findSupportedMimeType).mockResolvedValue("application/pdf");
    const action = operation === "open" ? openFile : shareFile;
    jest.mocked(action).mockRejectedValueOnce(new Error("Permission Denial: URI grant rejected"));
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, operation); });
    expect(Alert.alert).toHaveBeenCalledWith(
      "attachment.title", `attachment.${operation}Failed\n\nPermission Denial: URI grant rejected`,
    );
    expect(h.api.activeDownload).toBeNull();
    expect(nativePresentationInFlight()).toBe(false);
    act(() => h.tree.unmount());
  });

  test("Android reports export failures before launching the system share sheet", async () => {
    jest.mocked(namedExternalFile).mockRejectedValueOnce(new Error("attachment_export_cache_full"));
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(Alert.alert).toHaveBeenCalledWith(
      "attachment.title", "attachment.shareFailed\n\nattachment_export_cache_full",
    );
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test.each([new Error(""), "unknown failure"])("Android retains the fallback for errors without a message", async error => {
    jest.mocked(shareFile).mockRejectedValueOnce(error);
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.shareFailed");
    act(() => h.tree.unmount());
  });

  test("Android can share repeatedly without touching ExpoSharing's stale request lock", async () => {
    jest.mocked(Sharing.shareAsync).mockRejectedValue(new Error("Another share request is being processed now"));
    const h = await mount({});
    for (let attempt = 0; attempt < 3; attempt++) {
      await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
      expect(h.api.activeDownload).toBeNull();
    }
    expect(shareFile).toHaveBeenCalledTimes(3);
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("a rejected Android handoff does not block the next share", async () => {
    jest.mocked(shareFile).mockRejectedValueOnce(new Error("No activity"));
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(shareFile).toHaveBeenCalledTimes(2);
    expect(Alert.alert).toHaveBeenCalledTimes(1);
    act(() => jest.advanceTimersByTime(60_000));
    expect(nativePresentationInFlight()).toBe(false);
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
    expect(shareFile).toHaveBeenCalledTimes(1);
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
    expect(shareFile).toHaveBeenCalledWith("file:///cache/named/notes.txt", "text/plain", "attachment.share");
    expect(findSupportedMimeType).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("sharing a just-picked local file does not request a desktop transfer", async () => {
    const remote = { prepareAttachment: jest.fn(), downloadAttachment: jest.fn() };
    const h = await mount(remote);
    await act(async () => { await h.api.downloadOriginal({ path: "file:///cache/报告.pdf", name: "报告.pdf" }, "share"); });
    expect(remote.prepareAttachment).not.toHaveBeenCalled();
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(shareFile).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
  });

  test("unavailable iOS sharing is reported before downloading", async () => {
    Platform.OS = "ios";
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
    expect(shareFile).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("native sharing releases the action lane while keeping bounded connection grace", async () => {
    const sheet = deferred<void>();
    jest.mocked(shareFile).mockReturnValueOnce(sheet.promise);
    const h = await mount({});
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare(pdfInfo, localFile, "share"); });
    expect(nativePresentationInFlight()).toBe(true);
    await act(async () => { sheet.resolve(); await sharing; });
    expect(h.api.activeDownload).toBeNull();
    expect(nativePresentationInFlight()).toBe(true);
    act(() => jest.advanceTimersByTime(60_000));
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
