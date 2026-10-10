import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Platform } from "react-native";
import type { TFunction } from "i18next";
import { useAttachmentPicker } from "../useAttachmentPicker";
import { ActionMenu } from "../../../components/ActionMenu";
import { albumSource, pickAttachments, pickFromAlbum, takePhoto } from "../../../remote/files";
import { showToast } from "../utils";
import type { MobileAttachment } from "../../../remote/types";

jest.mock("../../../remote/files", () => ({
  albumSource: jest.fn(async () => "system"),
  loadAlbumImages: jest.fn(async () => []),
  pickAttachments: jest.fn(async () => []),
  pickFromAlbum: jest.fn(async () => []),
  prepareAlbumImages: jest.fn(async () => []),
  remainingImageSlots: jest.fn(() => 4),
  takePhoto: jest.fn(async () => []),
}));
jest.mock("../utils", () => ({ showToast: jest.fn() }));
jest.mock("lucide-react-native", () => ({ Camera: () => null, Images: () => null, File: () => null, X: () => null }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

const t = ((key: string) => key) as TFunction;
const pick = jest.mocked(pickAttachments);
const toast = showToast as jest.Mock;
const source = jest.mocked(albumSource);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, reject, resolve };
}

let tree: ReactTestRenderer;
let api: ReturnType<typeof useAttachmentPicker>;
const setAttachments = jest.fn();

function Harness() {
  const value = useAttachmentPicker([], setAttachments, t);
  useEffect(() => { api = value; });
  return value.attachmentMenu;
}

beforeEach(() => {
  jest.clearAllMocks();
  source.mockResolvedValue("system");
  pick.mockResolvedValue([]);
  Platform.OS = "ios";
  act(() => { tree = create(createElement(Harness)); });
});
afterEach(() => { act(() => tree.unmount()); });

const attachment: MobileAttachment = {
  kind: "image", localUri: "file:///a.png", mimeType: "image/png", name: "a.png",
  originalSize: 3, transferSize: 3,
};

test("a second pick while one is in flight is ignored, so the sheet cannot double-open", async () => {
  const pending = deferred<MobileAttachment[]>();
  pick.mockReturnValueOnce(pending.promise);
  act(() => { void api.chooseFiles(); });
  // The native picker is already up; a second tap must not open another one.
  act(() => { void api.chooseFiles(); });
  act(() => { void api.capturePhoto(); });
  act(() => { void api.chooseFromAlbum(); });
  // Nor may the source sheet reopen behind the running picker.
  act(() => { api.openAttachmentMenu(); });
  expect(pick).toHaveBeenCalledTimes(1);
  expect(takePhoto).not.toHaveBeenCalled();
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
  await act(async () => {
    pending.resolve([attachment]);
    await Promise.resolve();
  });
  expect(setAttachments).toHaveBeenCalledWith([attachment]);
  // The lock is released: the next pick starts a new request, and the sheet can
  // open again.
  await act(async () => { await api.chooseFiles(); });
  expect(pick).toHaveBeenCalledTimes(2);
  act(() => { api.openAttachmentMenu(); });
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(true);
});

test("a pick that fails reports the attachment error and keeps what was attached", async () => {
  pick.mockRejectedValueOnce(new Error("attachment_permission_denied"));
  await act(async () => { await api.chooseFiles(); });
  expect(toast).toHaveBeenCalledWith("attachment.errors.attachment_permission_denied");
  expect(setAttachments).not.toHaveBeenCalled();
});

test("a pick that fails with something other than an Error falls back to the generic key", async () => {
  pick.mockRejectedValueOnce("boom");
  await act(async () => { await api.chooseFiles(); });
  expect(toast).toHaveBeenCalledWith("attachment.errors.attachment_failed");
});

test("a failed pick releases the lock so the user can try again", async () => {
  pick.mockRejectedValueOnce(new Error("attachment_failed"));
  await act(async () => { await api.chooseFiles(); });
  await act(async () => { await api.chooseFiles(); });
  expect(pick).toHaveBeenCalledTimes(2);
});

test("an album source that cannot be probed is treated as unavailable, never as a crash", async () => {
  Platform.OS = "android";
  source.mockRejectedValueOnce(new Error("no gallery"));
  await act(async () => { await api.chooseFromAlbum(); });
  expect(toast).toHaveBeenCalledWith("attachment.errors.attachment_album_unavailable");
  expect(pick).not.toHaveBeenCalled();
});

test("a system album source opens the native picker instead of the in-app grid", async () => {
  Platform.OS = "android";
  source.mockResolvedValueOnce("system");
  await act(async () => { await api.chooseFromAlbum(); });
  // The OS picker is preferred whenever the device can present one.
  expect(pickFromAlbum).toHaveBeenCalledWith([]);
  expect(pick).not.toHaveBeenCalled();
  expect(toast).not.toHaveBeenCalled();
});
