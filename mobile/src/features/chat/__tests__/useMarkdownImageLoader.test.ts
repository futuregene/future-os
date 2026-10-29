import { createElement, useLayoutEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import * as Network from "expo-network";
import { useMarkdownImageLoader } from "../useMarkdownImageLoader";
import { confirmDownload } from "../utils";
import type { MarkdownImageLoader } from "../../../components/MarkdownImage";
import {
  cachedPreviewFile,
  forgetPreviewFile,
  previewAspect,
  rememberPreviewAspect,
  rememberPreviewFile,
} from "../../../remote/files";

jest.mock("expo-network", () => ({ NetworkStateType: { WIFI: "wifi", CELLULAR: "cellular", UNKNOWN: "unknown" }, getNetworkStateAsync: jest.fn(async () => ({ type: "wifi" })) }));
jest.mock("../utils", () => ({ formatBytes: (n: number) => String(n), confirmDownload: jest.fn(async () => true) }));
jest.mock("../../../remote/files", () => ({
  MAX_FILE_BYTES: 10 * 1024 * 1024,
  TransferCancelledError: class extends Error {},
  cachedPreviewFile: jest.fn(() => null),
  forgetPreviewFile: jest.fn(),
  previewAspect: jest.fn(),
  rememberPreviewAspect: jest.fn(),
  rememberPreviewFile: jest.fn(),
}));
const t = ((key: string) => key) as TFunction;
let tree: ReactTestRenderer;
let loader: MarkdownImageLoader;
const info = { previewKind: "image", mimeType: "image/png", size: 100, transferId: "t", contentHash: "held" };
function mockRemote() {
  return {
    credentials: { pairId: "pair", expectedDesktopId: "desktop" }, selectedSessionId: "one",
    cachedAttachment: jest.fn(), prepareAttachment: jest.fn(async () => info),
    downloadAttachment: jest.fn(async () => ({ uri: "file:///verified.png" })),
  };
}
function Harness({ remote }: { remote: ReturnType<typeof mockRemote> }) {
  const result = useMarkdownImageLoader(remote as unknown as Parameters<typeof useMarkdownImageLoader>[0], t);
  useLayoutEffect(() => { loader = result; });
  return null;
}
beforeEach(() => {
  jest.clearAllMocks();
  jest.mocked(cachedPreviewFile).mockReturnValue(null);
  jest.mocked(Network.getNetworkStateAsync).mockResolvedValue({ type: Network.NetworkStateType.WIFI });
  jest.mocked(confirmDownload).mockResolvedValue(true);
});
afterEach(() => { act(() => tree?.unmount()); });
function mount(remote = mockRemote()) {
  act(() => { tree = create(createElement(Harness, { remote })); });
  return remote;
}

test("mount does not read files; explicit load uses the verified preview flow", async () => {
  const remote = mount();
  expect(remote.prepareAttachment).not.toHaveBeenCalled();
  const signal = new AbortController().signal;
  await expect(loader.load("docs/chart.png", signal)).resolves.toBe("file:///verified.png");
  expect(remote.prepareAttachment).toHaveBeenCalledWith({ path: "docs/chart.png", name: "chart.png" }, "preview", signal);
  expect(remote.downloadAttachment).toHaveBeenCalledWith(info, undefined, signal);
});

test("cache hits are used only after metadata is revalidated", async () => {
  const remote = mount();
  remote.cachedAttachment.mockReturnValue({ info, file: { uri: "file:///cached.png" } });
  expect(loader.cached("a.png")).toBeNull();
  await expect(loader.load("a.png", new AbortController().signal)).resolves.toBe("file:///cached.png");
  expect(remote.prepareAttachment).toHaveBeenCalledWith(
    { path: "a.png", name: "a.png" }, "preview", expect.anything(),
  );
  expect(Network.getNetworkStateAsync).not.toHaveBeenCalled();
});

test.each([Network.NetworkStateType.CELLULAR, Network.NetworkStateType.UNKNOWN])("small images do not prompt on %s", async type => {
  const remote = mount();
  jest.mocked(Network.getNetworkStateAsync).mockResolvedValue({ type });
  await expect(loader.load("a.png", new AbortController().signal)).resolves.toBe("file:///verified.png");
  expect(confirmDownload).not.toHaveBeenCalled();
  expect(remote.downloadAttachment).toHaveBeenCalledTimes(1);
});

test.each([Network.NetworkStateType.CELLULAR, Network.NetworkStateType.UNKNOWN])("asks before large downloads on %s; declining downloads no bytes", async type => {
  const remote = mount();
  remote.prepareAttachment.mockResolvedValue({ ...info, size: 1024 * 1024 });
  jest.mocked(Network.getNetworkStateAsync).mockResolvedValue({ type });
  jest.mocked(confirmDownload).mockResolvedValue(false);
  await expect(loader.load("a.png", new AbortController().signal)).resolves.toBeNull();
  expect(confirmDownload).toHaveBeenCalledTimes(1);
  expect(remote.downloadAttachment).not.toHaveBeenCalled();
});

test.each([
  { ...info, size: 11 * 1024 * 1024 }, { ...info, size: 0 },
  { ...info, previewKind: "file" }, { ...info, mimeType: "image/svg+xml" },
])("does not download oversized/non-native image previews: %j", async rejected => {
  const remote = mount();
  remote.prepareAttachment.mockResolvedValue(rejected);
  await expect(loader.load("a.png", new AbortController().signal)).rejects.toThrow();
  expect(remote.downloadAttachment).not.toHaveBeenCalled();
});

test("session changes between metadata and download reject stale work", async () => {
  const remote = mount();
  const request = loader.load("a.png", new AbortController().signal);
  const oldLoader = loader;
  act(() => tree.update(createElement(Harness, { remote: { ...remote, selectedSessionId: "two" } })));
  await expect(request).rejects.toThrow();
  expect(oldLoader.cached("a.png")).toBeNull();
  expect(remote.downloadAttachment).not.toHaveBeenCalled();
});

test("an aborted request never starts preparing", async () => {
  const remote = mount();
  const controller = new AbortController();
  controller.abort();
  await expect(loader.load("a.png", controller.signal)).rejects.toThrow();
  expect(remote.prepareAttachment).not.toHaveBeenCalled();
});

/** A preview file this phone already holds. */
function held(contentHash = "held", stale = false) {
  return { file: { uri: "file:///held.png" } as never, contentHash, size: 8, stale };
}

async function settle() {
  // Let the unqueued background re-check finish.
  for (let tick = 0; tick < 5; tick++) await Promise.resolve();
}

test("a picture this phone already holds paints without asking the desktop", async () => {
  const remote = mount();
  jest.mocked(cachedPreviewFile).mockReturnValue(held());
  expect(loader.cached("a.png")).toBe("file:///held.png");
  await expect(loader.load("a.png", new AbortController().signal)).resolves.toBe("file:///held.png");
  // No `download_prepare`, so no round trip between the tap and the picture.
  expect(remote.prepareAttachment).not.toHaveBeenCalled();
  expect(remote.downloadAttachment).not.toHaveBeenCalled();
});

test("a held picture still honours an aborted request", async () => {
  mount();
  jest.mocked(cachedPreviewFile).mockReturnValue(held());
  const controller = new AbortController();
  controller.abort();
  await expect(loader.load("a.png", controller.signal)).rejects.toThrow();
});

test("a held picture is re-checked behind the paint, never in front of it", async () => {
  const remote = mount();
  jest.mocked(cachedPreviewFile).mockReturnValue(held("held", true));
  remote.prepareAttachment.mockResolvedValue({ ...info, contentHash: "held" });
  await expect(loader.load("a.png", new AbortController().signal)).resolves.toBe("file:///held.png");
  await settle();
  expect(remote.prepareAttachment).toHaveBeenCalledWith({ path: "a.png", name: "a.png" }, "preview");
  // Unchanged content only refreshes the entry's age.
  expect(rememberPreviewFile).toHaveBeenCalledWith(expect.any(String), "a.png", expect.objectContaining({ contentHash: "held" }));
  expect(forgetPreviewFile).not.toHaveBeenCalled();
});

test("content that changed on the desktop drops the entry instead of showing it again", async () => {
  const remote = mount();
  jest.mocked(cachedPreviewFile).mockReturnValue(held("old", true));
  remote.prepareAttachment.mockResolvedValue({ ...info, contentHash: "new" });
  await expect(loader.load("a.png", new AbortController().signal)).resolves.toBe("file:///held.png");
  await settle();
  expect(forgetPreviewFile).toHaveBeenCalledWith(expect.any(String), "a.png");
});

test("a fresh entry is not re-checked at all", async () => {
  const remote = mount();
  jest.mocked(cachedPreviewFile).mockReturnValue(held("held", false));
  await loader.load("a.png", new AbortController().signal);
  await settle();
  expect(remote.prepareAttachment).not.toHaveBeenCalled();
});

test("the measured ratio is kept per path, so the next paint reserves its height", () => {
  mount();
  jest.mocked(previewAspect).mockReturnValue(2);
  expect(loader.aspect?.("a.png")).toBe(2);
  loader.rememberAspect?.("a.png", 2.5);
  expect(rememberPreviewAspect).toHaveBeenCalledWith(expect.any(String), "a.png", 2.5);
});
