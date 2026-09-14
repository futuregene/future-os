import { createElement, useLayoutEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import * as Network from "expo-network";
import { useMarkdownImageLoader } from "../useMarkdownImageLoader";
import { confirmDownload } from "../utils";
import type { MarkdownImageLoader } from "../../../components/MarkdownImage";

jest.mock("expo-network", () => ({ NetworkStateType: { WIFI: "wifi", CELLULAR: "cellular", UNKNOWN: "unknown" }, getNetworkStateAsync: jest.fn(async () => ({ type: "wifi" })) }));
jest.mock("../utils", () => ({ formatBytes: (n: number) => String(n), confirmDownload: jest.fn(async () => true) }));
jest.mock("../../../remote/files", () => ({ MAX_FILE_BYTES: 10 * 1024 * 1024, TransferCancelledError: class extends Error {} }));
const t = ((key: string) => key) as TFunction;
let tree: ReactTestRenderer;
let loader: MarkdownImageLoader;
const info = { previewKind: "image", mimeType: "image/png", size: 100, transferId: "t" };
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

test("cache hits avoid network requests and transfer", async () => {
  const remote = mount();
  remote.cachedAttachment.mockReturnValue({ info, file: { uri: "file:///cached.png" } });
  expect(loader.cached("a.png")).toBe("file:///cached.png");
  await expect(loader.load("a.png", new AbortController().signal)).resolves.toBe("file:///cached.png");
  expect(remote.prepareAttachment).not.toHaveBeenCalled();
  expect(Network.getNetworkStateAsync).not.toHaveBeenCalled();
});

test.each([Network.NetworkStateType.CELLULAR, Network.NetworkStateType.UNKNOWN])("asks before downloading on %s; declining downloads no bytes", async type => {
  const remote = mount();
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
