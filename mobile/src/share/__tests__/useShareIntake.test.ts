import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState } from "react-native";
import { getPendingShare } from "future-share-intent";
import { showToast } from "../../features/chat/utils";
import { useRemoteControls as useRemote } from "../../remote/RemoteContext";
import {
  loadSessionDraft,
  saveSessionDraft,
} from "../../remote/draftStorage";
import { prepareSharedAttachments } from "../../remote/files";
import { markShareLanded } from "../shareInbox";
import type { MobileAttachment } from "../../remote/types";
import { useShareIntake } from "../useShareIntake";
const NEW_CONVERSATION_DRAFT_KEY = "desktop:draft:new";

jest.mock("future-share-intent", () => ({ getPendingShare: jest.fn() }));
jest.mock("../../remote/draftStorage", () => ({
  NEW_CONVERSATION_DRAFT_KEY: "draft:new",
  loadSessionDraft: jest.fn(),
  saveSessionDraft: jest.fn(),
}));
jest.mock("../../remote/files", () => ({ prepareSharedAttachments: jest.fn() }));
jest.mock("../../features/chat/utils", () => ({ showToast: jest.fn() }));
jest.mock("../../share/shareInbox", () => ({
  markShareLanded: jest.fn(),
}));
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: jest.fn() }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const mockedGetPendingShare = getPendingShare as jest.MockedFunction<typeof getPendingShare>;
const mockedLoadDraft = loadSessionDraft as jest.MockedFunction<typeof loadSessionDraft>;
const mockedSaveDraft = saveSessionDraft as jest.MockedFunction<typeof saveSessionDraft>;
const mockedPrepare = prepareSharedAttachments as jest.MockedFunction<
  typeof prepareSharedAttachments
>;
const mockedToast = showToast as jest.MockedFunction<typeof showToast>;
const mockedMarkLanded = markShareLanded as jest.MockedFunction<typeof markShareLanded>;
const mockedUseRemote = useRemote as jest.MockedFunction<typeof useRemote>;

function attachment(name: string): MobileAttachment {
  return {
    localUri: `file:///cache/share/${name}`,
    name,
    mimeType: "image/jpeg",
    kind: "image",
    originalSize: 10,
    transferSize: 10,
  };
}

let renderer: ReactTestRenderer | null = null;
const newConversation = jest.fn(async () => {});
const selectSession = jest.fn(async (_sessionId: string) => {});
let intake: ReturnType<typeof useShareIntake>;
async function choose(mode: "chat" | "workspace" | "session" = "chat", workspaceId?: string) {
  const start = intake.chooseDestination;
  act(() => intake.dismiss());
  await act(async () => { await start(mode, workspaceId); });
}

function Harness(): null {
  const result = useShareIntake();
  useEffect(() => { intake = result; });
  return null;
}

function render(credentials: unknown = { pairId: "pair", expectedDesktopId: "desktop" }): void {
  mockedUseRemote.mockReturnValue({
    credentials,
    workspaces: [{ id: "w1", name: "Project" }],
    newConversation,
    selectSession,
    sessions: [
      { sessionId: "s1", threadId: "t1", title: "Chat", mode: "chat", streaming: false },
      { sessionId: "s2", threadId: "t2", title: "Work", mode: "workspace", workspaceId: "w1", streaming: false },
    ],
  } as unknown as ReturnType<typeof useRemote>);
  act(() => {
    renderer = create(createElement(Harness));
  });
}

async function flush(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 10; i += 1) await Promise.resolve();
  });
}

function appStateListener(): (state: string) => void {
  const calls = (AppState.addEventListener as jest.Mock).mock.calls;
  return calls[calls.length - 1]![1];
}

beforeEach(() => {
  jest.clearAllMocks();
  mockedGetPendingShare.mockResolvedValue(null);
  mockedLoadDraft.mockResolvedValue(null);
  mockedSaveDraft.mockResolvedValue();
  mockedPrepare.mockResolvedValue([]);
  newConversation.mockResolvedValue();
  selectSession.mockResolvedValue();
});

afterEach(() => {
  if (renderer) {
    act(() => renderer!.unmount());
    renderer = null;
  }
});

test("stages a shared payload into the new-conversation draft", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({
    text: "look at this",
    tooLarge: false,
    files: [{ uri: "file:///cache/share/a.jpg", name: "a.jpg", mimeType: "image/jpeg" }],
  });
  mockedPrepare.mockResolvedValue([attachment("a.jpg")]);
  mockedLoadDraft.mockResolvedValue({ version: 1, text: "  typing…  ", attachments: [] });

  render();
  await flush();

  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(newConversation).not.toHaveBeenCalled();
  expect(intake.pending).not.toBeNull();
  await choose();
  expect(mockedPrepare).toHaveBeenCalledWith([
    { uri: "file:///cache/share/a.jpg", name: "a.jpg", mimeType: "image/jpeg" },
  ], []);
  // The staged draft keeps what the user had typed and appends the share.
  expect(mockedSaveDraft).toHaveBeenCalledWith(NEW_CONVERSATION_DRAFT_KEY, {
    text: "typing…\n\nlook at this",
    attachments: [attachment("a.jpg")],
  });
  expect(newConversation).toHaveBeenCalledWith("chat", undefined);
  expect(mockedMarkLanded).toHaveBeenCalled();
});

test.each(["s1", "s2"])("appends to existing session %s without creating or sending a conversation", async sessionId => {
  const previous = attachment("previous.jpg");
  const shared = attachment("shared.jpg");
  mockedGetPendingShare.mockResolvedValueOnce({
    text: "shared text",
    tooLarge: false,
    files: [{ uri: shared.localUri, name: shared.name, mimeType: shared.mimeType }],
  });
  mockedLoadDraft.mockResolvedValue({ version: 1, text: "unsent text", attachments: [previous] });
  mockedPrepare.mockResolvedValue([previous, shared]);
  render();
  await flush();
  await choose("session", sessionId);

  expect(mockedLoadDraft).toHaveBeenCalledWith(`desktop:${sessionId}`);
  expect(mockedPrepare).toHaveBeenCalledWith(expect.any(Array), [previous]);
  expect(mockedSaveDraft).toHaveBeenCalledWith(`desktop:${sessionId}`, {
    text: "unsent text\n\nshared text",
    attachments: [previous, shared],
  });
  expect(selectSession).toHaveBeenCalledWith(sessionId);
  expect(newConversation).not.toHaveBeenCalled();
  expect(mockedMarkLanded).toHaveBeenCalledTimes(1);
});

test.each([undefined, "", "deleted"])("ignores a missing or deleted session: %s", async sessionId => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  render();
  await flush();
  await choose("session", sessionId);
  expect(mockedLoadDraft).not.toHaveBeenCalled();
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(selectSession).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
});

test("attachment validation failure leaves an existing session draft untouched", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  mockedPrepare.mockRejectedValueOnce(new Error("attachment_file_too_large"));
  render();
  await flush();
  await choose("session", "s1");
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(selectSession).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_file_too_large");
});

test("a share with no text keeps the files-only draft", async () => {
  mockedGetPendingShare.mockResolvedValue({
    text: "   ",
    tooLarge: false,
    files: [{ uri: "file:///cache/share/a.jpg", name: "a.jpg", mimeType: "image/jpeg" }],
  });
  mockedPrepare.mockResolvedValue([attachment("a.jpg")]);

  render();
  await flush();

  await choose("workspace", "w1");
  expect(newConversation).toHaveBeenCalledWith("workspace", "w1");
  expect(mockedSaveDraft).toHaveBeenCalledWith(NEW_CONVERSATION_DRAFT_KEY, {
    text: "",
    attachments: [attachment("a.jpg")],
  });
});

test("surfaces a dropped oversized file without opening a conversation", async () => {
  mockedGetPendingShare.mockResolvedValue({ text: "", tooLarge: true, files: [] });

  render();
  await flush();

  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_file_too_large");
  expect(newConversation).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
});

test("reports the oversized file but still stages the files that fit", async () => {
  mockedGetPendingShare.mockResolvedValue({
    text: "caption",
    tooLarge: true,
    files: [{ uri: "file:///cache/share/a.jpg", name: "a.jpg", mimeType: "image/jpeg" }],
  });
  mockedPrepare.mockResolvedValue([attachment("a.jpg")]);

  render();
  await flush();

  await choose();
  expect(mockedSaveDraft).toHaveBeenCalled();
  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_file_too_large");
});

test("surfaces an attachment failure and leaves no half-staged draft", async () => {
  mockedGetPendingShare.mockResolvedValue({
    text: "",
    tooLarge: false,
    files: [{ uri: "file:///cache/share/big.bin", name: "big.bin", mimeType: "application/pdf" }],
  });
  mockedPrepare.mockRejectedValue(new Error("attachment_file_too_large"));

  render();
  await flush();

  await choose();
  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_file_too_large");
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(newConversation).not.toHaveBeenCalled();
});

test("cancelling the destination does not change a draft or start a conversation", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "caption", tooLarge: false, files: [] });
  render();
  await flush();
  act(() => intake.dismiss());
  await flush();
  expect(newConversation).not.toHaveBeenCalled();
  expect(mockedSaveDraft).not.toHaveBeenCalled();
});

test("does not open a workspace that disappeared while choosing", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "caption", tooLarge: false, files: [] });
  render();
  await flush();
  await choose("workspace", "deleted");
  expect(newConversation).not.toHaveBeenCalled();
  expect(mockedSaveDraft).not.toHaveBeenCalled();
});

test("does not stage into an existing session after the desktop changes during preparation", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  let release!: (attachments: MobileAttachment[]) => void;
  mockedPrepare.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  render();
  await flush();
  const start = intake.chooseDestination;
  let importing!: Promise<void>;
  act(() => {
    intake.dismiss();
    importing = start("session", "s1");
  });
  await flush();
  mockedUseRemote.mockReturnValue({
    ...mockedUseRemote(),
    credentials: { pairId: "other-pair", expectedDesktopId: "other" },
  } as ReturnType<typeof useRemote>);
  act(() => renderer!.update(createElement(Harness)));
  await act(async () => {
    release([]);
    await importing;
  });
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(selectSession).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
});

test("only imports once while an existing session is opening", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  let release!: () => void;
  selectSession.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; }));
  render();
  await flush();
  const start = intake.chooseDestination;
  let importing!: Promise<void>;
  act(() => {
    intake.dismiss();
    importing = start("session", "s1");
  });
  await flush();
  await act(async () => { await start("session", "s1"); });
  await act(async () => {
    release();
    await importing;
  });
  expect(mockedSaveDraft).toHaveBeenCalledTimes(1);
  expect(selectSession).toHaveBeenCalledTimes(1);
  expect(mockedMarkLanded).toHaveBeenCalledTimes(1);
});

test("does not read the share inbox until the device is paired", async () => {
  render(null);
  await flush();
  expect(mockedGetPendingShare).not.toHaveBeenCalled();
});

test("re-reads the inbox on every return to the foreground", async () => {
  render();
  await flush();
  expect(mockedGetPendingShare).toHaveBeenCalledTimes(1);

  act(() => appStateListener()("background"));
  await flush();
  expect(mockedGetPendingShare).toHaveBeenCalledTimes(1);

  act(() => appStateListener()("active"));
  await flush();
  expect(mockedGetPendingShare).toHaveBeenCalledTimes(2);
});

test("only one intake runs at a time", async () => {
  let release!: (value: null) => void;
  mockedGetPendingShare.mockImplementation(
    () =>
      new Promise(resolve => {
        release = resolve as (value: null) => void;
      }),
  );

  render();
  await flush();
  expect(mockedGetPendingShare).toHaveBeenCalledTimes(1);

  // A foreground transition while the first read is still in flight must not
  // start a second read (the native side would hand the payload to whichever
  // call it saw first).
  act(() => appStateListener()("active"));
  await flush();
  expect(mockedGetPendingShare).toHaveBeenCalledTimes(1);

  await act(async () => {
    release(null);
    await Promise.resolve();
  });
});
