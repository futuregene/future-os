import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import { AppState, Platform, type AppStateStatus } from "react-native";
import { AppAlert as Alert } from "../../../components/appAlerts";
import * as Sharing from "expo-sharing";
import * as Network from "expo-network";
import * as downloadUtils from "../utils";
import * as LegacyFileSystem from "expo-file-system/legacy";
import { File } from "expo-file-system";
import { findSupportedMimeType, openFile, saveFile, shareFile, supportsNativeFileActions } from "future-file-handler";
import { nativePresentationInFlight } from "../../../remote/nativePresentation";
import { useFileDownload } from "../useFileDownload";
import { PREPARE_REVEAL_DELAY_MS } from "../utils";
import { namedExternalFile, TransferCancelledError } from "../../../remote/files";
import type { DownloadInfo } from "../../../remote/types";
import { readPreviewText } from "../readPreviewText";
import * as downloadPolicy from "../downloadPolicy";
import * as fileHandler from "../../../remote/fileHandler";

/** The exact warning key union `downloadWarning` resolves to. */
type DownloadWarning = Awaited<ReturnType<typeof downloadPolicy.downloadWarning>>;

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

async function mount(remote: unknown, onTransferProgress = jest.fn()) {
  let api!: ReturnType<typeof useFileDownload>;
  let tree!: ReactTestRenderer;
  const t = ((key: string) => key) as TFunction;
  function Harness() {
    api = useFileDownload(remote as Parameters<typeof useFileDownload>[0], t, onTransferProgress);
    return null;
  }
  await act(async () => { tree = create(createElement(Harness)); });
  return { get api() { return api; }, tree, onTransferProgress };
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

  test("a handle that is not the active transfer cannot resurrect the progress dialog", async () => {
    // `openOrShare` takes an optional existing handle, and that handle need not
    // be the one this hook is showing — the dialog must never be re-pointed at a
    // transfer nobody is running. No download is started here, so the hook's own
    // handle slot stays empty and the passed one is foreign by construction.
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => info),
      downloadAttachment: jest.fn(async () => file),
    };
    const h = await mount(remote);
    const foreign = {
      id: "foreign-handle",
      fileName: "old.txt",
      visible: false,
      controller: new AbortController(),
      handoffPending: false,
      revealTimer: null,
    };
    await act(async () => { await h.api.openOrShare(info, null, "save", foreign); });
    // Observable consequence: the progress surface still reports nothing,
    // because that handle owns no transfer. Dropping the check would publish the
    // foreign handle's id as the active download instead.
    expect(h.api.activeDownload).toBeNull();
    act(() => h.tree.unmount());
  });

  test("a foreign transfer's own progress cannot publish itself into the dialog", async () => {
    // The progress and waiting callbacks belong to the transport that was asked
    // for bytes. Here that transport reports for a handle the hook is not
    // showing, so the callbacks take their `else` arm — and the dialog must stay
    // empty rather than adopt a transfer nobody owns. Both callbacks are driven
    // explicitly, which is the only way those arms are reached at all.
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => info),
      downloadAttachment: jest.fn(async (
        _info: unknown, onProgress?: (d: number, t: number) => void, _signal?: AbortSignal,
        onWaiting?: () => void,
      ) => {
        onProgress?.(3, 3);
        onWaiting?.();
        return file;
      }),
    };
    const h = await mount(remote);
    const foreign = {
      id: "foreign-handle-2",
      fileName: "old.txt",
      visible: false,
      controller: new AbortController(),
      handoffPending: false,
      revealTimer: null,
    };
    await act(async () => { await h.api.openOrShare(info, null, "save", foreign); });
    // Progress and the waiting state were both reported by the foreign transfer;
    // neither may appear, because that handle does not own the lane.
    expect(h.api.activeDownload).toBeNull();
    expect(remote.downloadAttachment).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
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

describe("progress while preparing", () => {
  const platform = Platform.OS;
  beforeEach(() => { Platform.OS = "android"; });
  afterEach(() => {
    Platform.OS = platform;
    jest.useRealTimers();
  });

  test("a prepare slower than the delay shows the dialog instead of a dead screen", async () => {
    jest.useFakeTimers();
    const prepare = deferred<DownloadInfo>();
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(() => prepare.promise),
      downloadAttachment: jest.fn(async () => file as File),
    };
    const h = await mount(remote);
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openAttachment({ path: "/notes.txt", name: "notes.txt" }); });
    // Still invisible: an instant answer must not flash a dialog.
    expect(h.api.activeDownload).toBeNull();

    await act(async () => { jest.advanceTimersByTime(PREPARE_REVEAL_DELAY_MS); });
    expect(h.api.activeDownload).toMatchObject({
      fileName: "notes.txt",
      phase: "preparing",
      completedBytes: 0,
      totalBytes: 0,
    });

    // The reply then drives the same dialog the fast path uses, and the
    // deferred handoff (setTimeout 0 off iOS) closes it onto the preview.
    await act(async () => {
      prepare.resolve({ ...info, size: 2048 });
      await opening;
    });
    act(() => jest.runOnlyPendingTimers());
    expect(h.api.activeDownload).toBeNull();
    expect(h.api.preview).toMatchObject({ text: "new" });
    act(() => h.tree.unmount());
  });

  test("a prepare that finishes in time never reveals the dialog", async () => {
    jest.useFakeTimers();
    const remote = {
      cachedAttachment: jest.fn(() => ({ info, file })),
      prepareAttachment: jest.fn(async () => info),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openFileLink("/notes.txt"); });
    expect(h.api.preview).toMatchObject({ text: "new" });
    // A leaked reveal timer would surface here.
    act(() => jest.runOnlyPendingTimers());
    expect(h.api.activeDownload).toBeNull();
    act(() => h.tree.unmount());
  });
});

test("a revealed dialog adopts the freshly known size instead of being rebuilt", async () => {
  const platform = Platform.OS;
  Object.defineProperty(Platform, "OS", { configurable: true, value: "android" });
  jest.useFakeTimers();
  const prepare = deferred<DownloadInfo>();
  const download = deferred<typeof file>();
  const progress: Array<(done: number, total: number) => void> = [];
  const remote = {
    cachedAttachment: jest.fn(() => null),
    prepareAttachment: jest.fn(() => prepare.promise),
    downloadAttachment: jest.fn((_info: unknown, onProgress: (done: number, total: number) => void) => {
      progress.push(onProgress);
      return download.promise;
    }),
  };
  const h = await mount(remote);
  try {
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openFileLink("/notes.txt"); });
    await act(async () => { jest.advanceTimersByTime(PREPARE_REVEAL_DELAY_MS); });
    // The reveal fired before the size was known.
    expect(h.api.activeDownload).toMatchObject({ phase: "preparing", totalBytes: 0 });

    await act(async () => { prepare.resolve({ ...info, size: 2048 }); });
    // The dialog is already on screen, so the size the desktop just reported
    // has to reach it before the first byte — otherwise it claims 0 bytes
    // forever and the "preparing" phase never ends.
    expect(h.api.activeDownload).toMatchObject({ phase: "downloading", totalBytes: 2048, completedBytes: 0 });

    // The test's own mock recorded this transfer's progress callback, so it is there.
    await act(async () => { progress[0]!(1024, 2048); });
    expect(h.api.activeDownload).toMatchObject({ phase: "downloading", completedBytes: 1024 });

    await act(async () => { download.resolve(file as typeof file); await opening; });
    act(() => jest.runOnlyPendingTimers());
    // The visible dialog is dismissed into the preview, not left behind it.
    expect(h.api.activeDownload).toBeNull();
    expect(h.api.preview).toMatchObject({ text: "new" });
  } finally {
    act(() => h.tree.unmount());
    jest.useRealTimers();
    Object.defineProperty(Platform, "OS", { configurable: true, value: platform });
  }
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

describe("the download lane's invariants hold across randomized operation sequences", () => {
  // The lane's states are reachable only through interleavings of the public
  // operations, so a seeded random walk is the only way to explore combinations
  // no hand-written scenario covers. The seed is fixed, so a failure is
  // reproducible; the assertions are the lane's own invariants, not a snapshot.
  const invariants = (api: ReturnType<typeof useFileDownload>) => {
    // A preview stack is either empty or ends in the preview that is shown.
    expect(api.previews.length === 0).toBe(api.preview === null);
    expect(api.preview).toBe(api.previews[api.previews.length - 1] ?? null);
    for (const layer of api.previews) expect(layer.uri).toBeTruthy();
    // The fraction is a real ratio, never NaN and never out of range.
    expect(Number.isFinite(api.activeDownloadFraction)).toBe(true);
    expect(api.activeDownloadFraction).toBeGreaterThanOrEqual(0);
    expect(api.activeDownloadFraction).toBeLessThanOrEqual(1);
    if (api.activeDownload) {
      expect(["preparing", "downloading", "verifying", "waiting_network", "opening", "saving", "sharing"])
        .toContain(api.activeDownload.phase);
      expect(api.activeDownload.id).toBeTruthy();
      expect(api.activeDownload.fileName).toBeTruthy();
      expect(api.activeDownload.completedBytes).toBeLessThanOrEqual(
        Math.max(api.activeDownload.totalBytes, api.activeDownload.completedBytes));
      // The fraction always agrees with the dialog it describes.
      expect(api.activeDownloadFraction).toBe(
        api.activeDownload.totalBytes
          ? Math.min(1, api.activeDownload.completedBytes / api.activeDownload.totalBytes)
          : 0);
    } else {
      expect(api.activeDownloadFraction).toBe(0);
    }
    if (api.fileAction) expect(api.fileAction.info.name).toBeTruthy();
  };

  test.each([11, 4242, 987654]) ("seed %i keeps the lane consistent through 120 operations", async seedValue => {
    jest.useFakeTimers();
    let seed = seedValue as number;
    const rand = (n: number) => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return Math.floor((seed / 0x7fffffff) * n);
    };
    // Each transfer's callbacks and completion are held until the walk releases
    // them, so the operations interleave at arbitrary points.
    type Pending = { kind: "prepare" | "download"; settle: (reject: boolean) => void };
    const pending: Pending[] = [];
    let progressCallbacks: Array<(done: number, total: number) => void> = [];
    let waitingCallbacks: Array<() => void> = [];
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(() => new Promise<DownloadInfo>((resolve, reject) => {
        pending.push({ kind: "prepare", settle: (bad) => bad ? reject(new Error("prepare_failed")) : resolve({ ...info, size: 2048 }) });
      })),
      downloadAttachment: jest.fn((_i: unknown, onProgress: (d: number, t: number) => void, _s: unknown, onWaiting: () => void) => {
        progressCallbacks.push(onProgress);
        waitingCallbacks.push(onWaiting);
        return new Promise<unknown>((resolve, reject) => {
          pending.push({ kind: "download", settle: (bad) => bad ? reject(new Error("download_failed")) : resolve(file) });
        });
      }),
    };
    const t = ((key: string) => key) as TFunction;
    let api!: ReturnType<typeof useFileDownload>;
    let renderer!: ReactTestRenderer;
    // The walk hands the hook a deliberately partial remote — only the three
    // members above are ever reached — so the double crosses the boundary
    // exactly as `mount` (whose parameter is `unknown`) does for the other tests.
    const partialRemote = remote as unknown as Parameters<typeof useFileDownload>[0];
    function Harness() {
      api = useFileDownload(partialRemote, t, jest.fn());
      return null;
    }
    await act(async () => { renderer = create(createElement(Harness)); });

    try {
      for (let step = 0; step < 120; step += 1) {
        const op = rand(11);
        await act(async () => {
          if (op === 0) void api.openFileLink("/walk.txt").catch(() => undefined);
          else if (op === 1) void api.openAttachment({ path: "/walk.txt", name: "walk.txt" }).catch(() => undefined);
          else if (op === 2) void api.downloadOriginal({ path: "/walk.pdf", name: "walk.pdf" }, "share").catch(() => undefined);
          else if (op === 3) void api.openOrShare({ ...pdfInfo, name: `walk${step}.pdf` }, null, "share").catch(() => undefined);
          else if (op === 4) api.cancelActiveDownload();
          else if (op === 5) api.onDownloadModalShow();
          else if (op === 6) api.flushPendingDownloadModal();
          else if (op === 7) api.closePreview();
          else if (op === 8) api.popPreview();
          else if (op === 9 && progressCallbacks.length) {
            progressCallbacks[rand(progressCallbacks.length)]!(rand(4096), 2048);
          } else if (op === 10 && waitingCallbacks.length) {
            waitingCallbacks[rand(waitingCallbacks.length)]!();
          } else if (pending.length) {
            const index = rand(pending.length);
            pending.splice(index, 1)[0]!.settle(rand(5) === 0);
          }
          await Promise.resolve();
        });
        // Timers fire between operations, like a real frame boundary.
        act(() => { jest.advanceTimersByTime(rand(400) === 0 ? PREPARE_REVEAL_DELAY_MS + 1 : rand(50)); });
        // Settle whatever the operations started, then fire the deferred work
        // (the non-iOS presentation timer, the reveal timer) again.
        await act(async () => {
          while (pending.length) pending.splice(rand(pending.length), 1)[0]!.settle(false);
          await Promise.resolve();
        });
        act(() => { jest.runOnlyPendingTimers(); });
        await act(async () => { await Promise.resolve(); });
        invariants(api);
      }
      // A cancelled lane is fully released: nothing to cancel, nothing shown.
      await act(async () => { api.cancelActiveDownload(); });
      expect(api.activeDownload).toBeNull();
      expect(api.activeDownloadFraction).toBe(0);
      invariants(api);
      // Closing the stack leaves no layer behind.
      await act(async () => { api.closePreview(); });
      expect(api.previews).toHaveLength(0);
      expect(api.preview).toBeNull();
      invariants(api);
      // With nothing open a dismissal runs its action immediately — once — and a
      // later flush cannot make it run a second time.
      await act(async () => {
        const after = jest.fn();
        api.dismissPreviewThen(after);
        expect(after).toHaveBeenCalledTimes(1);
        api.flushPendingPreviewAction();
      });
      act(() => { jest.runOnlyPendingTimers(); });
      await act(async () => { await Promise.resolve(); });
      invariants(api);
    } finally {
      act(() => renderer.unmount());
      jest.useRealTimers();
    }
  });
});

describe("nested previews", () => {
  const cachedRemote = () => ({
    cachedAttachment: jest.fn(() => ({ info, file })),
    prepareAttachment: jest.fn(async () => info),
    downloadAttachment: jest.fn(),
  });
  const paths = (h: { api: { previews: { attachment: { path: string } }[] } }) =>
    h.api.previews.map(entry => entry.attachment.path);

  test("a link followed from a preview keeps the document that linked it", async () => {
    const h = await mount(cachedRemote());
    await act(async () => { await h.api.openFileLink("/notes.txt"); });
    expect(paths(h)).toEqual(["/notes.txt"]);

    await act(async () => { await h.api.openLinkedFile("/other.txt"); });
    expect(paths(h)).toEqual(["/notes.txt", "/other.txt"]);
    expect(h.api.preview?.attachment.path).toBe("/other.txt");

    // Back is reversible, and the outermost document is not popped away by it.
    act(() => h.api.popPreview());
    expect(paths(h)).toEqual(["/notes.txt"]);
    act(() => h.api.popPreview());
    expect(paths(h)).toEqual(["/notes.txt"]);
    act(() => h.api.closePreview());
    expect(h.api.previews).toEqual([]);
    act(() => h.tree.unmount());
  });

  test("opening from the conversation starts a new stack instead of stacking", async () => {
    const h = await mount(cachedRemote());
    await act(async () => { await h.api.openFileLink("/notes.txt"); });
    await act(async () => { await h.api.openLinkedFile("/other.txt"); });
    await act(async () => { await h.api.openFileLink("/third.txt"); });
    expect(paths(h)).toEqual(["/third.txt"]);
    act(() => h.tree.unmount());
  });

  test("an action that needs the whole surface gone clears every layer first", async () => {
    const platform = Platform.OS;
    Platform.OS = "android";
    jest.useFakeTimers();
    const h = await mount(cachedRemote());
    try {
      await act(async () => { await h.api.openFileLink("/notes.txt"); });
      await act(async () => { await h.api.openLinkedFile("/other.txt"); });
      const action = jest.fn();
      act(() => h.api.dismissPreviewThen(action));
      expect(h.api.previews).toEqual([]);
      // The action sheet is a second Modal: it waits for this one to be gone.
      expect(action).not.toHaveBeenCalled();
      act(() => { jest.runOnlyPendingTimers(); });
      expect(action).toHaveBeenCalledTimes(1);
    } finally {
      act(() => h.tree.unmount());
      jest.useRealTimers();
      Platform.OS = platform;
    }
  });
});

/** The mocked `expo-file-system` File. Real readers are out of scope here. */
function localFileMock(bytes: Uint8Array) {
  return ((uri: string) => ({
    uri,
    size: bytes.length,
    bytes: async () => bytes,
  })) as never;
}

describe("local file:// attachments are read on the phone, never transferred", () => {
  const platform = Platform.OS;
  const localBytes = new TextEncoder().encode("hello 世界");

  beforeEach(() => {
    jest.clearAllMocks();
    Platform.OS = "android";
    jest.mocked(File).mockImplementation(localFileMock(localBytes));
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    Platform.OS = platform;
  });

  test("a file type the phone cannot read is refused at every entry point", async () => {
    const h = await mount({ prepareAttachment: jest.fn(), downloadAttachment: jest.fn() });
    await act(async () => { await h.api.openAttachment({ path: "/scan.bin", name: "scan.bin" }); });
    await act(async () => { await h.api.openFileLink("/scan.bin"); });
    await act(async () => { await h.api.downloadOriginal({ path: "/scan.bin", name: "scan.bin" }); });
    expect(jest.mocked(Alert.alert).mock.calls).toEqual([
      ["attachment.title", "attachment.unsupportedType"],
      ["attachment.title", "attachment.unsupportedType"],
      ["attachment.title", "attachment.unsupportedType"],
    ]);
    expect(h.api.preview).toBeNull();
    expect(h.api.fileAction).toBeNull();
    act(() => h.tree.unmount());
  });

  test("local text, markdown, JSON and image files preview without a transfer", async () => {
    const remote = { prepareAttachment: jest.fn(), cachedAttachment: jest.fn(), downloadAttachment: jest.fn() };
    for (const [name, kind] of [
      ["notes.txt", "text"],
      ["notes.md", "markdown"],
      ["data.json", "json"],
      ["photo.png", "image"],
    ] as const) {
      const h = await mount(remote);
      await act(async () => { await h.api.openAttachment({ path: `file:///cache/${name}`, name }); });
      expect(remote.prepareAttachment).not.toHaveBeenCalled();
      expect(remote.downloadAttachment).not.toHaveBeenCalled();
      expect(h.api.preview).toMatchObject({ uri: `file:///cache/${name}`, info: { previewKind: kind } });
      if (kind === "image") expect(h.api.preview?.text).toBeUndefined();
      else if (kind === "markdown") expect(h.api.preview).toMatchObject({ markdown: "hello 世界" });
      else expect(h.api.preview).toMatchObject({ text: "hello 世界" });
      act(() => h.tree.unmount());
    }
    expect(Alert.alert).not.toHaveBeenCalled();
  });

  test("a local text file the reader cannot decode points at the desktop instead", async () => {
    jest.mocked(readPreviewText).mockResolvedValueOnce(null as never);
    const h = await mount({});
    await act(async () => { await h.api.openAttachment({ path: "file:///cache/notes.txt", name: "notes.txt" }); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.previewOnDesktop");
    expect(h.api.preview).toBeNull();
    act(() => h.tree.unmount());
  });

  test("a malformed local URI is reported as a download failure rather than thrown", async () => {
    jest.mocked(File).mockImplementation((() => { throw new Error("invalid file uri"); }) as never);
    const h = await mount({});
    await act(async () => { await h.api.openAttachment({ path: "file:///bad", name: "notes.txt" }); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.downloadFailed");
    expect(h.api.preview).toBeNull();
    act(() => h.tree.unmount());
  });
});

describe("one transfer at a time", () => {
  const platform = Platform.OS;

  beforeEach(() => {
    jest.clearAllMocks();
    // iOS routes the refusal through the shared dialog, which is observable
    // here; Android uses a native toast with no jest surface.
    Platform.OS = "ios";
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    Platform.OS = platform;
  });

  test("every entry point refuses a second transfer while one is in flight", async () => {
    const prepare = deferred<DownloadInfo>();
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(() => prepare.promise),
      downloadAttachment: jest.fn(async () => file as File),
    };
    const h = await mount(remote);
    let first!: Promise<void>;
    await act(async () => { first = h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    await act(async () => { await h.api.openAttachment({ path: "/b.txt", name: "b.txt" }); });
    await act(async () => { await h.api.openFileLink("/c.txt"); });
    await act(async () => { await h.api.downloadOriginal({ path: "/d.txt", name: "d.txt" }, "save"); });
    await act(async () => { await h.api.openOrShare({ ...info }, null, "save"); });
    const refusals = jest.mocked(Alert.alert).mock.calls
      .filter(call => call[0] === "attachment.downloadInProgress");
    expect(refusals).toHaveLength(4);
    // The live transfer is untouched by the four refusals.
    await act(async () => { prepare.resolve(info); await first; });
    act(() => { h.api.onDownloadModalShow(); h.api.flushPendingDownloadModal(); });
    expect(h.api.preview).toMatchObject({ text: "new" });
    act(() => h.tree.unmount());
  });

  test("cancelling with nothing in flight is a no-op", async () => {
    const h = await mount({});
    act(() => h.api.cancelActiveDownload());
    expect(h.api.activeDownload).toBeNull();
    act(() => h.tree.unmount());
  });
});

describe("progress, waiting and oversized guards", () => {
  const platform = Platform.OS;

  beforeEach(() => {
    jest.clearAllMocks();
    Platform.OS = "android";
    jest.mocked(File).mockImplementation(localFileMock(new TextEncoder().encode("new")));
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    Platform.OS = platform;
  });

  test("a live transfer reports chunks and a stalled transport to the dialog", async () => {
    const download = deferred<File>();
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => ({ ...info, size: 2048 })),
      downloadAttachment: jest.fn((_info, onProgress: (d: number, t: number) => void, _signal, onWaiting: () => void) => {
        onProgress(512, 2048);
        onWaiting();
        return download.promise;
      }),
    };
    const h = await mount(remote);
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    expect(h.api.activeDownload).toMatchObject({
      phase: "waiting_network", completedBytes: 512, totalBytes: 2048,
    });
    expect(h.api.activeDownloadFraction).toBeCloseTo(0.25);
    await act(async () => { download.resolve(file as File); await opening; });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(h.api.preview).toMatchObject({ text: "new" });
    act(() => h.tree.unmount());
  });

  test("a download that finishes under the progress dialog still hands off", async () => {
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => ({ ...info, size: 4 })),
      downloadAttachment: jest.fn(async (_info, onProgress: (d: number, t: number) => void, _signal, onWaiting: () => void) => {
        onWaiting();
        onProgress(4, 4);
        return file as File;
      }),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(h.api.preview).toMatchObject({ text: "new" });
    expect(h.api.activeDownload).toBeNull();
    act(() => h.tree.unmount());
  });

  test("an action sheet download drives the same progress callbacks", async () => {
    // The Android share ends in a native handoff: the app stays "active" until
    // the chooser comes back, bounded by the grace window. Capture that
    // lifecycle listener so this real-timer test can close the window the way
    // the platform does — leaving it open would outlive the test as a pending
    // 60 s timer, which is exactly the kind of handle Jest refuses to exit on.
    let onStateChanged!: (state: AppStateStatus) => void;
    jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
      onStateChanged = listener;
      return { remove: jest.fn() };
    });
    const download = deferred<File>();
    const remote = {
      downloadAttachment: jest.fn((_info, onProgress: (d: number, t: number) => void, _signal, onWaiting: () => void) => {
        onProgress(1, 4);
        onWaiting();
        return download.promise;
      }),
    };
    const h = await mount(remote);
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare({ ...pdfInfo, size: 4 }, null, "share"); });
    expect(h.api.activeDownload).toMatchObject({ phase: "waiting_network", totalBytes: 4 });
    await act(async () => { download.resolve(file as File); await sharing; });
    expect(shareFile).toHaveBeenCalledTimes(1);
    expect(h.api.activeDownload).toBeNull();
    // The handoff is still holding the connection open — only the chooser
    // returning (or the grace expiring) may release it.
    expect(nativePresentationInFlight()).toBe(true);
    onStateChanged("background");
    onStateChanged("active");
    expect(nativePresentationInFlight()).toBe(false);
    act(() => h.tree.unmount());
  });

  test("an oversized download is refused before a byte is requested", async () => {
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => ({ ...info, size: 10 * 1024 * 1024 + 1 })),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openAttachment({ path: "/big.txt", name: "big.txt" }); });
    await act(async () => { await h.api.openFileLink("/big.txt"); });
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.tooLarge");
    expect(h.api.preview).toBeNull();
    act(() => h.tree.unmount());
  });

  test("an image transferred from the desktop opens as a preview, not text", async () => {
    const imageInfo: DownloadInfo = { ...info, name: "photo.png", mimeType: "image/png", previewKind: "image" };
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => imageInfo),
      downloadAttachment: jest.fn(async () => file as File),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openAttachment({ path: "/photo.png", name: "photo.png" }); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(h.api.preview).toMatchObject({ uri: "file:///cache/notes.txt", info: { previewKind: "image" } });
    expect(h.api.preview?.text).toBeUndefined();
    act(() => h.tree.unmount());
  });

  test("a downloaded document that cannot be decoded fails loudly", async () => {
    jest.mocked(readPreviewText).mockResolvedValueOnce(null as never);
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => ({ ...info, size: 4 })),
      downloadAttachment: jest.fn(async () => file as File),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.downloadFailed");
    expect(h.api.preview).toBeNull();
    act(() => h.tree.unmount());
  });

  test("a failed preparation reports the failure instead of leaving a dead screen", async () => {
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => { throw new Error("desktop unreachable"); }),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.downloadFailed");
    await act(async () => { await h.api.downloadOriginal({ path: "/a.txt", name: "a.txt" }, "save"); });
    expect(Alert.alert).toHaveBeenCalledTimes(2);
    act(() => h.tree.unmount());
  });

  test.each(["attachment", "link", "original"])(
    "%s reports a stalled prepare as waiting for the network",
    async entry => {
      const waiting: (() => void)[] = [];
      const remote = {
        cachedAttachment: jest.fn(() => null),
        prepareAttachment: jest.fn((_a: unknown, _v: unknown, _s: unknown, onWaiting: () => void) => {
          waiting.push(onWaiting);
          return new Promise<DownloadInfo>(() => {});
        }),
        downloadAttachment: jest.fn(),
      };
      const h = await mount(remote);
      await act(async () => {
        if (entry === "attachment") void h.api.openAttachment({ path: "/a.txt", name: "a.txt" });
        else if (entry === "link") void h.api.openFileLink("/a.txt");
        else void h.api.downloadOriginal({ path: "/a.txt", name: "a.txt" }, "save");
        await Promise.resolve();
      });
      expect(waiting).toHaveLength(1);
      act(() => waiting[0]!());
      // The handle is still invisible, so the hook stays quiet rather than
      // flashing a dialog for a prepare that may still return instantly.
      expect(h.api.activeDownload).toBeNull();
      act(() => h.api.cancelActiveDownload());
      act(() => h.tree.unmount());
      expect(Alert.alert).not.toHaveBeenCalled();
    },
  );

  test("a desktop text preview may still open as plain text after the local path changed", async () => {
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => ({ ...info, size: 4 })),
      downloadAttachment: jest.fn(async () => file as File),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openFileLink("/a.txt"); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(h.api.preview).toMatchObject({ text: "new" });
    act(() => h.tree.unmount());
  });
});

describe("the invisible prepare reveal", () => {
  const platform = Platform.OS;

  beforeEach(() => { jest.clearAllMocks(); Platform.OS = "android"; jest.useFakeTimers(); });
  afterEach(() => { act(() => jest.runOnlyPendingTimers()); jest.useRealTimers(); Platform.OS = platform; });

  test("a reveal timer that fires after cancellation cannot resurrect the dialog", async () => {
    const prepare = deferred<DownloadInfo>();
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(() => prepare.promise),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    expect(h.api.activeDownload).toBeNull();
    act(() => h.api.cancelActiveDownload());
    act(() => { jest.advanceTimersByTime(PREPARE_REVEAL_DELAY_MS + 10); });
    expect(h.api.activeDownload).toBeNull();
    await act(async () => { prepare.resolve(info); await opening; });
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });
});

describe("cancelling at every await point", () => {
  const platform = Platform.OS;

  beforeEach(() => {
    jest.clearAllMocks();
    Platform.OS = "android";
    jest.mocked(File).mockImplementation(localFileMock(new TextEncoder().encode("new")));
    jest.mocked(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync)
      .mockResolvedValue({ granted: true, directoryUri: "content://documents/tree/downloads" });
    jest.mocked(LegacyFileSystem.StorageAccessFramework.createFileAsync)
      .mockResolvedValue("content://documents/report.pdf");
    jest.mocked(LegacyFileSystem.readAsStringAsync).mockResolvedValue("cGRm");
    jest.mocked(Sharing.isAvailableAsync).mockResolvedValue(true);
    jest.mocked(shareFile).mockResolvedValue();
    jest.mocked(openFile).mockResolvedValue();
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    Platform.OS = platform;
  });

  const never = () => new Promise<never>(() => {});

  test("cancelling while the download warning is pending abandons the transfer", async () => {
    const warning = deferred<DownloadWarning>();
    jest.spyOn(downloadPolicy, "downloadWarning").mockReturnValue(warning.promise);
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => info),
      downloadAttachment: jest.fn(async () => file as File),
    };
    const h = await mount(remote);
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { warning.resolve(null); await opening; });
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(h.api.preview).toBeNull();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelling while the large-file confirmation is up abandons the transfer", async () => {
    const confirm = deferred<boolean>();
    jest.spyOn(downloadPolicy, "downloadWarning").mockResolvedValue("attachment.cellularWarning");
    jest.spyOn(downloadUtils, "confirmDownload").mockReturnValue(confirm.promise);
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => info),
      downloadAttachment: jest.fn(async () => file as File),
    };
    const h = await mount(remote);
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openAttachment({ path: "/a.txt", name: "a.txt" }); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { confirm.resolve(true); await opening; });
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("an action sheet transfer cancelled while the warning is pending never reaches the file", async () => {
    const warning = deferred<DownloadWarning>();
    const confirm = deferred<boolean>();
    jest.spyOn(downloadPolicy, "downloadWarning").mockReturnValue(warning.promise);
    jest.spyOn(downloadUtils, "confirmDownload").mockReturnValue(confirm.promise);
    const remote = { prepareAttachment: jest.fn(), downloadAttachment: jest.fn(async () => file as File) };
    const h = await mount(remote);
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare({ ...pdfInfo, size: 5 * 1024 * 1024 }, null, "share"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { warning.resolve("attachment.cellularWarning"); await sharing; });
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(shareFile).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("an action sheet transfer cancelled while the confirmation is up never reaches the file", async () => {
    const warning = deferred<DownloadWarning>();
    const confirm = deferred<boolean>();
    jest.spyOn(downloadPolicy, "downloadWarning").mockReturnValue(warning.promise);
    jest.spyOn(downloadUtils, "confirmDownload").mockReturnValue(confirm.promise);
    const remote = { prepareAttachment: jest.fn(), downloadAttachment: jest.fn(async () => file as File) };
    const h = await mount(remote);
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare({ ...pdfInfo, size: 5 * 1024 * 1024 }, null, "share"); });
    // The warning has already resolved, so the transfer is past its first
    // abort check and is sitting in the user's confirmation dialog.
    await act(async () => { warning.resolve("attachment.cellularWarning"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { confirm.resolve(true); await sharing; });
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(shareFile).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelling a save while the desktop prepares it never opens the SAF picker", async () => {
    const prepare = deferred<DownloadInfo>();
    const remote = {
      prepareAttachment: jest.fn(() => prepare.promise),
      cachedAttachment: jest.fn(() => null),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    let saving!: Promise<void>;
    await act(async () => { saving = h.api.downloadOriginal({ path: "/报告.pdf", name: "报告.pdf" }, "save"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { prepare.resolve(pdfInfo); await saving; });
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelling while the system MIME lookup is pending keeps the file unopened", async () => {
    const mime = deferred<string | null>();
    jest.spyOn(fileHandler, "supportedExternalMime").mockReturnValue(mime.promise);
    const h = await mount({});
    let opening!: Promise<void>;
    await act(async () => { opening = h.api.openOrShare(pdfInfo, localFile, "open"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { mime.resolve("application/pdf"); await opening; });
    expect(openFile).not.toHaveBeenCalled();
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelling while the share-sheet probe is pending keeps the sheet closed", async () => {
    Platform.OS = "ios";
    const availability = deferred<boolean>();
    jest.mocked(Sharing.isAvailableAsync).mockReturnValue(availability.promise);
    const h = await mount({ downloadAttachment: jest.fn() });
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare(pdfInfo, null, "share"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { availability.resolve(true); await sharing; });
    expect(Sharing.shareAsync).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelling during the bytes stops before the native handoff", async () => {
    const download = deferred<File>();
    const h = await mount({ downloadAttachment: () => download.promise });
    let sharing!: Promise<void>;
    await act(async () => { sharing = h.api.openOrShare(pdfInfo, null, "share"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { download.resolve(localFile); await sharing; });
    expect(namedExternalFile).not.toHaveBeenCalled();
    expect(shareFile).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelling an Android save releases the SAF permission wait", async () => {
    const permission = deferred<{ granted: boolean; directoryUri: string }>();
    jest.mocked(LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync)
      .mockReturnValue(permission.promise);
    const h = await mount({});
    let saving!: Promise<void>;
    await act(async () => { saving = h.api.openOrShare(pdfInfo, localFile, "save"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { permission.resolve({ granted: true, directoryUri: "content://d" }); await saving; });
    expect(LegacyFileSystem.StorageAccessFramework.createFileAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("cancelling while the export file is being named never reaches the system", async () => {
    const named = deferred<{ uri: string }>();
    jest.mocked(namedExternalFile).mockReturnValueOnce(named.promise as never);
    const remote = {
      cachedAttachment: jest.fn(() => ({ info: pdfInfo, file: localFile })),
      prepareAttachment: jest.fn(async () => pdfInfo),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    let sharing!: Promise<void>;
    await act(async () => {
      sharing = h.api.downloadOriginal({ path: "/report.pdf", name: "report.pdf" }, "share");
    });
    expect(namedExternalFile).toHaveBeenCalledTimes(1);
    act(() => h.api.cancelActiveDownload());
    await act(async () => { named.resolve({ uri: "file:///cache/named/report.pdf" }); await sharing; });
    // Naming the file is the last await before the share sheet, and the user
    // revoked consent while it was in flight: handing it over anyway would open
    // a system sheet for a transfer the UI already cancelled.
    expect(shareFile).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    expect(h.api.activeDownload).toBeNull();
    act(() => h.tree.unmount());
  });

  test("cancelling an Android save releases the base64 read", async () => {
    const read = deferred<string>();
    jest.mocked(LegacyFileSystem.readAsStringAsync).mockReturnValue(read.promise);
    const h = await mount({});
    let saving!: Promise<void>;
    await act(async () => { saving = h.api.openOrShare(pdfInfo, localFile, "save"); });
    act(() => h.api.cancelActiveDownload());
    await act(async () => { read.resolve("cGRm"); await saving; });
    expect(LegacyFileSystem.StorageAccessFramework.writeAsStringAsync).not.toHaveBeenCalled();
    act(() => h.tree.unmount());
  });

  test("a cancelled prepare that never resolves still releases the lane", async () => {
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(() => never()),
      downloadAttachment: jest.fn(),
    };
    const h = await mount(remote);
    void h.api.openAttachment({ path: "/a.txt", name: "a.txt" });
    await act(async () => { await Promise.resolve(); });
    act(() => h.api.cancelActiveDownload());
    expect(h.api.activeDownload).toBeNull();
    act(() => h.tree.unmount());
  });
});

describe("platform presentation fallbacks", () => {
  const platform = Platform.OS;

  beforeEach(() => {
    jest.clearAllMocks();
    jest.mocked(namedExternalFile).mockResolvedValue({ uri: "file:///cache/named/report.pdf" } as never);
    jest.mocked(Sharing.isAvailableAsync).mockResolvedValue(true);
    jest.mocked(Sharing.shareAsync).mockResolvedValue();
    jest.mocked(saveFile).mockResolvedValue();
    jest.mocked(supportsNativeFileActions).mockReturnValue(false);
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    Platform.OS = platform;
  });

  test("a desktop platform without a native action falls back to the share sheet", async () => {
    Platform.OS = "windows";
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "open"); });
    expect(Sharing.shareAsync).toHaveBeenCalledWith("file:///cache/named/report.pdf", {
      mimeType: "application/pdf", dialogTitle: "attachment.open",
    });
    act(() => h.tree.unmount());
  });

  test("iOS reports a failed handoff after the modal has gone", async () => {
    Platform.OS = "ios";
    jest.mocked(supportsNativeFileActions).mockReturnValue(true);
    jest.mocked(saveFile).mockRejectedValue(new Error("save failed"));
    const h = await mount({});
    await act(async () => { await h.api.openOrShare(pdfInfo, localFile, "save"); });
    // The handoff is queued behind the progress modal; releasing it runs the
    // native action, whose failure must be surfaced rather than swallowed.
    await act(async () => { h.api.flushPendingDownloadModal(); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.saveFailed");
    act(() => h.tree.unmount());
  });

  test("a later onShow still releases a handoff that was never presented", async () => {
    Platform.OS = "ios";
    const download = deferred<File>();
    const h = await mount({ downloadAttachment: () => download.promise });
    let saving!: Promise<void>;
    await act(async () => { saving = h.api.openOrShare(pdfInfo, null, "save"); });
    await act(async () => { download.resolve(localFile); await saving; });
    expect(h.api.activeDownload).not.toBeNull();
    act(() => h.api.onDownloadModalShow());
    expect(h.api.activeDownload).toBeNull();
    act(() => h.tree.unmount());
  });

  test("following a non-previewable link from a document closes the stack first", async () => {
    Platform.OS = "android";
    jest.useFakeTimers();
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async ({ name }: { name: string }) =>
        name.endsWith(".pdf") ? pdfInfo : info),
      downloadAttachment: jest.fn(async () => localFile),
    };
    const h = await mount(remote);
    try {
      await act(async () => { await h.api.openFileLink("/notes.txt"); });
      act(() => { jest.runOnlyPendingTimers(); });
      expect(h.api.previews.map(p => p.attachment.path)).toEqual(["/notes.txt"]);
      await act(async () => { await h.api.openLinkedFile("/报告.pdf"); });
      expect(h.api.fileAction).toBeNull();
      act(() => { jest.runOnlyPendingTimers(); });
      expect(h.api.previews).toEqual([]);
      expect(h.api.fileAction?.info.name).toBe("报告.pdf");
    } finally {
      act(() => h.tree.unmount());
      jest.useRealTimers();
    }
  });

  test("an image link followed from a preview keeps the document underneath", async () => {
    Platform.OS = "android";
    const imageInfo: DownloadInfo = { ...pdfInfo, name: "photo.png", mimeType: "image/png", previewKind: "image" };
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async ({ name }: { name: string }) =>
        name.endsWith(".png") ? imageInfo : info),
      downloadAttachment: jest.fn(async () => localFile),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openFileLink("/notes.txt"); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(h.api.previews.map(p => p.attachment.path)).toEqual(["/notes.txt"]);
    await act(async () => { await h.api.openLinkedFile("/photo.png"); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(h.api.previews.map(p => p.attachment.path)).toEqual(["/notes.txt", "/photo.png"]);
    act(() => h.tree.unmount());
  });

  test("a text link that cannot be decoded after download fails loudly", async () => {
    Platform.OS = "android";
    jest.mocked(readPreviewText).mockResolvedValueOnce(null as never);
    const remote = {
      cachedAttachment: jest.fn(() => null),
      prepareAttachment: jest.fn(async () => info),
      downloadAttachment: jest.fn(async () => localFile),
    };
    const h = await mount(remote);
    await act(async () => { await h.api.openFileLink("/notes.txt"); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(Alert.alert).toHaveBeenCalledWith("attachment.title", "attachment.downloadFailed");
    act(() => h.tree.unmount());
  });
});

describe("preview dismissal handoff", () => {
  const platform = Platform.OS;

  beforeEach(() => { jest.clearAllMocks(); Platform.OS = "android"; });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    Platform.OS = platform;
  });

  const cached = () => ({
    cachedAttachment: jest.fn(() => ({ info, file })),
    prepareAttachment: jest.fn(async () => info),
    downloadAttachment: jest.fn(),
  });

  test("dismissing with nothing open runs the action immediately", async () => {
    const h = await mount(cached());
    const action = jest.fn();
    act(() => h.api.dismissPreviewThen(action));
    expect(action).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
  });

  test("a superseded dismissal cannot fire the action it replaced", async () => {
    jest.useFakeTimers();
    const h = await mount(cached());
    try {
      await act(async () => { await h.api.openFileLink("/notes.txt"); });
      const first = jest.fn();
      const second = jest.fn();
      act(() => {
        h.api.dismissPreviewThen(first);
        // Still inside the same commit: the preview list has not re-rendered
        // yet, so the second call queues behind the first instead of running.
        h.api.dismissPreviewThen(second);
      });
      act(() => { jest.runOnlyPendingTimers(); });
      expect(first).not.toHaveBeenCalled();
      expect(second).toHaveBeenCalledTimes(1);
    } finally {
      act(() => h.tree.unmount());
      jest.useRealTimers();
    }
  });

  test("Android flushes the queued action on its own tick, not when asked", async () => {
    jest.useFakeTimers();
    const h = await mount(cached());
    try {
      await act(async () => { await h.api.openFileLink("/notes.txt"); });
      expect(h.api.preview).not.toBeNull();
      const action = jest.fn();
      act(() => {
        h.api.dismissPreviewThen(action);
        // The sheet dismissal is not the flush point off iOS, so this must not
        // run the action early (it would race the still-closing native sheet).
        h.api.flushPendingPreviewAction();
      });
      expect(action).not.toHaveBeenCalled();
      act(() => { jest.runOnlyPendingTimers(); });
      expect(action).toHaveBeenCalledTimes(1);
    } finally {
      act(() => h.tree.unmount());
      jest.useRealTimers();
    }
  });

  test("iOS releases the queued action on the modal's dismissal callback", async () => {
    Platform.OS = "ios";
    const h = await mount(cached());
    await act(async () => { await h.api.openFileLink("/notes.txt"); });
    const action = jest.fn();
    act(() => h.api.dismissPreviewThen(action));
    expect(action).not.toHaveBeenCalled();
    act(() => h.api.flushPendingPreviewAction());
    expect(action).toHaveBeenCalledTimes(1);
    act(() => h.api.flushPendingPreviewAction());
    expect(action).toHaveBeenCalledTimes(1);
    act(() => h.tree.unmount());
  });
});
