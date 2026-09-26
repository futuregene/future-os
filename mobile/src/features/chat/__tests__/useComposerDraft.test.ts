import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState, Platform, type AppStateStatus } from "react-native";
import { useRemote } from "../../../remote/RemoteContext";
import {
  flushSessionDraft,
  loadSessionDraft,
  scheduleSessionDraft,
} from "../../../remote/draftStorage";
import { recoverPendingImagePickerAttachments } from "../../../remote/files";
import type { MobileAttachment } from "../../../remote/types";
import { showToast } from "../utils";
import { useComposerDraft, type ComposerDraftApi } from "../useComposerDraft";

jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("../../../remote/draftStorage", () => ({
  flushSessionDraft: jest.fn(async () => {}),
  loadSessionDraft: jest.fn(async () => null),
  scheduleSessionDraft: jest.fn(),
}));
jest.mock("../../../remote/files", () => ({
  recoverPendingImagePickerAttachments: jest.fn(async (attachments: unknown) => attachments),
}));
jest.mock("../utils", () => ({ showToast: jest.fn(), deferPresentation: jest.fn() }));

const useRemoteMock = useRemote as jest.MockedFunction<typeof useRemote>;
const load = loadSessionDraft as jest.Mock;
const schedule = scheduleSessionDraft as jest.Mock;
const flush = flushSessionDraft as jest.Mock;
const recover = recoverPendingImagePickerAttachments as jest.Mock;
const toast = showToast as jest.Mock;

/** `t` is passed in as a prop (the hook only ever calls it for toast copy). */
const t = (key: string) => `t:${key}`;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

let tree: ReactTestRenderer;
let current: ComposerDraftApi;
function Harness({ desktopId = "desk-1", sessionId = "s1" }: { desktopId?: string; sessionId?: string }) {
  useRemoteMock.mockReturnValue({
    credentials: { expectedDesktopId: desktopId },
    selectedSessionId: sessionId,
  } as unknown as ReturnType<typeof useRemote>);
  const value = useComposerDraft(useRemoteMock(), t as never);
  useEffect(() => { current = value; }, [value]);
  return null;
}
function render(props: Parameters<typeof Harness>[0] = {}) {
  act(() => {
    if (tree) tree.update(createElement(Harness, props));
    else tree = create(createElement(Harness, props));
  });
}
/** Let the restore effect's async body settle. */
const settle = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

const attachment: MobileAttachment = {
  kind: "image", localUri: "file:///tmp/a.png", mimeType: "image/png", name: "a.png",
  originalSize: 3, transferSize: 3,
};

beforeEach(() => {
  jest.clearAllMocks();
  load.mockResolvedValue(null);
  recover.mockImplementation(async (attachments: unknown) => attachments);
});
afterEach(() => {
  if (tree) act(() => tree.unmount());
  tree = undefined as never;
  jest.restoreAllMocks();
});

test("a stored draft comes back with its text and attachments", async () => {
  load.mockResolvedValue({ attachments: [attachment], text: "half-written 半成品" });
  render();
  await settle();
  expect(load).toHaveBeenCalledWith("desk-1:s1");
  expect(current.message).toBe("half-written 半成品");
  expect(current.attachments).toEqual([attachment]);
});

test("the empty composer is not persisted while the draft is still loading", async () => {
  // Without the restore guard the hook's initial "" state would be scheduled
  // and could overwrite the stored draft before the load came back.
  const pending = deferred<{ attachments: MobileAttachment[]; text: string } | null>();
  load.mockReturnValueOnce(pending.promise);
  render();
  await act(async () => { await Promise.resolve(); });
  expect(schedule).not.toHaveBeenCalled();
  await act(async () => {
    pending.resolve({ attachments: [], text: "restored" });
    await Promise.resolve();
    await Promise.resolve();
  });
  expect(current.message).toBe("restored");
  expect(schedule).toHaveBeenCalledWith("desk-1:s1", { attachments: [], text: "restored" });
});

test("an edit is persisted under the conversation's own key", async () => {
  render();
  await settle();
  act(() => current.setMessage("hello"));
  expect(schedule).toHaveBeenCalledWith("desk-1:s1", { attachments: [], text: "hello" });
  act(() => current.setAttachments([attachment]));
  expect(schedule).toHaveBeenLastCalledWith("desk-1:s1", { attachments: [attachment], text: "hello" });
});

test("the draft conversation uses the fixed new-conversation slot", async () => {
  render({ sessionId: "" });
  await settle();
  expect(load).toHaveBeenCalledWith("desk-1:draft:new");
});

test("leaving the screen flushes what was typed", async () => {
  render();
  await settle();
  act(() => tree.unmount());
  tree = undefined as never;
  expect(flush).toHaveBeenCalledWith("desk-1:s1");
});

test.each(["background", "inactive"] as const)(
  "a %s app flushes the draft without unmounting the composer",
  async state => {
    let onState!: (next: AppStateStatus) => void;
    jest.spyOn(AppState, "addEventListener").mockImplementation((_event, handler) => {
      onState = handler;
      return { remove: jest.fn() };
    });
    render();
    await settle();
    act(() => onState(state));
    expect(flush).toHaveBeenCalledWith("desk-1:s1");
    // Still mounted: the composer keeps what the user had typed.
    expect(current.message).toBe("");
  },
);

test("staying active does not flush", async () => {
  let onState!: (next: AppStateStatus) => void;
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, handler) => {
    onState = handler;
    return { remove: jest.fn() };
  });
  render();
  await settle();
  act(() => onState("active"));
  expect(flush).not.toHaveBeenCalled();
});

describe("Android pending image-picker results", () => {
  beforeEach(() => { Platform.OS = "android"; });
  afterEach(() => { Platform.OS = "ios"; });

  test("a pending capture is recovered into the restored draft", async () => {
    const recovered: MobileAttachment = { ...attachment, localUri: "file:///tmp/recovered.png" };
    load.mockResolvedValue({ attachments: [attachment], text: "with photo" });
    recover.mockResolvedValue([recovered]);
    render();
    await settle();
    expect(recover).toHaveBeenCalledWith([attachment]);
    expect(current.attachments).toEqual([recovered]);
  });

  test("a recovery failure keeps the draft and tells the user which attachment failed", async () => {
    load.mockResolvedValue({ attachments: [attachment], text: "with photo" });
    recover.mockRejectedValue(new Error("attachment_missing"));
    render();
    await settle();
    expect(toast).toHaveBeenCalledWith("t:attachment.errors.attachment_missing");
    expect(current.message).toBe("with photo");
    expect(current.attachments).toEqual([attachment]);
  });

  test("a non-Error rejection falls back to the generic attachment failure", async () => {
    load.mockResolvedValue({ attachments: [attachment], text: "x" });
    recover.mockRejectedValue("boom");
    render();
    await settle();
    expect(toast).toHaveBeenCalledWith("t:attachment.errors.attachment_failed");
  });

  test("iOS never asks for pending Android results", async () => {
    Platform.OS = "ios";
    load.mockResolvedValue({ attachments: [attachment], text: "x" });
    render();
    await settle();
    expect(recover).not.toHaveBeenCalled();
    expect(current.attachments).toEqual([attachment]);
  });
});

test("a switch to another conversation discards the late load of the old one", async () => {
  // The old conversation's read is still in flight when the user switches, so
  // it must not paint the new composer with the previous conversation's draft.
  const pending = deferred<{ attachments: MobileAttachment[]; text: string } | null>();
  load.mockReturnValueOnce(pending.promise);
  render({ sessionId: "old" });
  render({ sessionId: "new" });
  await act(async () => {
    pending.resolve({ attachments: [attachment], text: "from the old conversation" });
    await Promise.resolve();
    await Promise.resolve();
  });
  expect(current.message).toBe("");
  expect(current.attachments).toEqual([]);
  // The new conversation's own load did run, under its own key.
  expect(load).toHaveBeenCalledWith("desk-1:new");
});

test("switching desktops rekeys the draft instead of reusing the old slot", async () => {
  render({ desktopId: "desk-1", sessionId: "s1" });
  await settle();
  render({ desktopId: "desk-2", sessionId: "s1" });
  await settle();
  expect(load.mock.calls.map(([key]) => key)).toEqual(["desk-1:s1", "desk-2:s1"]);
  expect(flush).toHaveBeenCalledWith("desk-1:s1");
});
