import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import { Alert, Platform } from "react-native";
import { useFileDownload } from "../useFileDownload";
import { TransferCancelledError } from "../../../remote/files";
import type { DownloadInfo } from "../../../remote/types";

jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("future-file-handler", () => ({ openFile: jest.fn() }));
jest.mock("../../../remote/files", () => ({
  MAX_FILE_BYTES: 10 * 1024 * 1024,
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
