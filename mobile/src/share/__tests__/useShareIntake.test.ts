import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState } from "react-native";
import { addPendingShareListener, getPendingShare } from "future-share-intent";
import { showToast } from "../../features/chat/utils";
import { useRemoteControls as useRemote } from "../../remote/RemoteContext";
import {
  loadSessionDraft,
  saveSessionDraft,
} from "../../remote/draftStorage";
import { prepareSharedAttachments } from "../../remote/files";
import { markShareLanded } from "../shareInbox";
import type { MobileAttachment, RemoteSession } from "../../remote/types";
import { useShareIntake } from "../useShareIntake";
const NEW_CONVERSATION_DRAFT_KEY = "desktop:draft:new";

jest.mock("future-share-intent", () => ({
  getPendingShare: jest.fn(),
  addPendingShareListener: jest.fn(() => ({ remove: jest.fn() })),
}));
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
const newConversationOn = jest.fn(async (_desktopId: string, _mode?: string, _workspaceId?: string) => {});
const openSessionOnDesktop = jest.fn(async (_desktopId: string, _sessionId: string) => {});
/** Whether a machine answers a command right now; the sheet reads it live. */
const commandsFor = jest.fn((_desktopId: string) => ({}) as unknown);
/** The other paired machines, as the provider's catalogue reports them. */
const mockCatalogs = {
  commandsFor,
  catalogs: [] as {
    desktopId: string;
    label: string;
    status: string;
    sessions: RemoteSession[];
  }[],
};
let intake: ReturnType<typeof useShareIntake>;
async function chooseOn(
  desktopId: string,
  mode: "chat" | "workspace" | "session" = "chat",
  destinationId?: string,
) {
  const start = intake.chooseDestination;
  act(() => intake.dismiss());
  await act(async () => { await start(desktopId, mode, destinationId); });
}
/** The ordinary case: the machine the phone is reading. */
const choose = (mode: "chat" | "workspace" | "session" = "chat", destinationId?: string) =>
  chooseOn("desktop", mode, destinationId);

function Harness(): null {
  const result = useShareIntake();
  useEffect(() => { intake = result; });
  return null;
}

function render(credentials: unknown = { pairId: "pair", expectedDesktopId: "desktop" }): void {
  mockedUseRemote.mockReturnValue({
    credentials,
    workspaces: [{ id: "w1", name: "Project" }],
    newConversationOn,
    openSessionOnDesktop,
    catalogs: mockCatalogs,
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
  newConversationOn.mockResolvedValue(undefined);
  openSessionOnDesktop.mockResolvedValue(undefined);
  commandsFor.mockReturnValue({});
  mockCatalogs.catalogs = [];
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
  expect(newConversationOn).not.toHaveBeenCalled();
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
  expect(newConversationOn).toHaveBeenCalledWith("desktop", "chat", undefined);
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
  expect(openSessionOnDesktop).toHaveBeenCalledWith("desktop", sessionId);
  expect(newConversationOn).not.toHaveBeenCalled();
  expect(mockedMarkLanded).toHaveBeenCalledTimes(1);
});

test.each([undefined, "", "deleted"])("ignores a missing or deleted session: %s", async sessionId => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  render();
  await flush();
  await choose("session", sessionId);
  expect(mockedLoadDraft).not.toHaveBeenCalled();
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(openSessionOnDesktop).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
});

test("lands a share onto a machine the phone is not reading", async () => {
  mockCatalogs.catalogs = [{
    desktopId: "other",
    label: "Office",
    status: "connected",
    sessions: [{ sessionId: "s9", threadId: "t9", title: "Office chat", streaming: false }],
  }];
  mockedGetPendingShare.mockResolvedValueOnce({ text: "hi", tooLarge: false, files: [] });
  render();
  await flush();
  await chooseOn("other", "session", "s9");
  expect(mockedSaveDraft).toHaveBeenCalledWith("other:s9", { text: "hi", attachments: [] });
  expect(openSessionOnDesktop).toHaveBeenCalledWith("other", "s9");
  expect(mockedMarkLanded).toHaveBeenCalled();
});

test("a new conversation on another machine is built there", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "hi", tooLarge: false, files: [] });
  render();
  await flush();
  // The sheet read that machine's workspaces itself, so the id it offers is the
  // one its own desktop knows.
  await chooseOn("other", "workspace", "ow");
  expect(mockedSaveDraft).toHaveBeenCalledWith("other:draft:new", { text: "hi", attachments: [] });
  expect(newConversationOn).toHaveBeenCalledWith("other", "workspace", "ow");
});

test("a session another machine no longer lists is not opened", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  render();
  await flush();
  await chooseOn("other", "session", "gone");
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(openSessionOnDesktop).not.toHaveBeenCalled();
});

test("attachment validation failure leaves an existing session draft untouched", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  mockedPrepare.mockRejectedValueOnce(new Error("attachment_file_too_large"));
  render();
  await flush();
  await choose("session", "s1");
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(openSessionOnDesktop).not.toHaveBeenCalled();
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
  expect(newConversationOn).toHaveBeenCalledWith("desktop", "workspace", "w1");
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
  expect(newConversationOn).not.toHaveBeenCalled();
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

test("a share the sending app marked as failed is reported before it is staged", async () => {
  // `failed` means the native share intent could not read part of the payload
  // (a permission or export failure). The payload that did survive is still
  // usable, but the user has to be told something was dropped.
  mockedGetPendingShare.mockResolvedValue({
    text: "surviving caption",
    failed: true,
    tooLarge: false,
    files: [],
  });

  render();
  await flush();

  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_failed");
  await choose();
  expect(mockedSaveDraft).toHaveBeenCalledWith(NEW_CONVERSATION_DRAFT_KEY, {
    text: "surviving caption",
    attachments: [],
  });
});

test("a failed share with nothing left in it is reported and never opens a conversation", async () => {
  mockedGetPendingShare.mockResolvedValue({ text: "", failed: true, tooLarge: false, files: [] });

  render();
  await flush();

  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_failed");
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(newConversationOn).not.toHaveBeenCalled();
});

// ── the machine the share lands on ──────────────────────────────────────────

test("a share read that throws is reported instead of silently dropped", async () => {
  mockedGetPendingShare.mockRejectedValue(new Error("native bridge gone"));

  render();
  await flush();

  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_failed");
  expect(newConversationOn).not.toHaveBeenCalled();
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
  expect(newConversationOn).not.toHaveBeenCalled();
});

test("an attachment failure that is not an error at all is still reported", async () => {
  mockedGetPendingShare.mockResolvedValue({ text: "share", tooLarge: false, files: [] });
  mockedPrepare.mockRejectedValue("no room left");

  render();
  await flush();

  await choose();
  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_failed");
  expect(mockedSaveDraft).not.toHaveBeenCalled();
});

test("cancelling the destination does not change a draft or start a conversation", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "caption", tooLarge: false, files: [] });
  render();
  await flush();
  act(() => intake.dismiss());
  await flush();
  expect(newConversationOn).not.toHaveBeenCalled();
  expect(mockedSaveDraft).not.toHaveBeenCalled();
});

test("does not open a workspace that disappeared while choosing", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "caption", tooLarge: false, files: [] });
  render();
  await flush();
  await choose("workspace", "deleted");
  expect(newConversationOn).not.toHaveBeenCalled();
  expect(mockedSaveDraft).not.toHaveBeenCalled();
});

test("a machine that stopped taking commands while the files were prepared writes nothing", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  let release!: (attachments: MobileAttachment[]) => void;
  mockedPrepare.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  render();
  await flush();
  const start = intake.chooseDestination;
  let importing!: Promise<void>;
  act(() => {
    intake.dismiss();
    importing = start("desktop", "session", "s1");
  });
  await flush();
  commandsFor.mockReturnValue(null);
  await act(async () => {
    release([]);
    await importing;
  });
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(openSessionOnDesktop).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
});

test("a destination on a machine that stopped answering writes nothing", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "caption", tooLarge: false, files: [] });
  render();
  await flush();
  expect(intake.pending).not.toBeNull();
  commandsFor.mockReturnValue(null);
  await choose();
  // The machine cannot take the share any more, and a draft written for it
  // would be content addressed to nobody.
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(newConversationOn).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
});

test("a machine that stops taking commands while the draft is saved does not open a conversation", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "caption", tooLarge: false, files: [] });
  let release!: () => void;
  mockedSaveDraft.mockImplementationOnce(() => new Promise<void>(resolve => { release = () => resolve(); }));
  render();
  await flush();
  const start = intake.chooseDestination;
  let importing!: Promise<void>;
  act(() => {
    intake.dismiss();
    importing = start("desktop", "chat");
  });
  await flush();
  commandsFor.mockReturnValue(null);
  await act(async () => { release(); await importing; });
  // The draft write already happened for that machine; opening a conversation
  // now would start one there with content it can no longer be told about.
  expect(newConversationOn).not.toHaveBeenCalled();
  expect(mockedMarkLanded).not.toHaveBeenCalled();
});

test("an opening the machine refuses leaves the share unladen and says why", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "caption", tooLarge: true, files: [] });
  openSessionOnDesktop.mockRejectedValueOnce(new Error("desktop_unavailable"));
  render();
  await flush();
  await choose("session", "s1");
  // Marking it landed would clear the inbox entry for a share that was never
  // delivered, and the oversize notice belongs to that delivery too.
  expect(mockedMarkLanded).not.toHaveBeenCalled();
  expect(mockedToast).toHaveBeenCalledWith("desktops.switchFailed");
  expect(mockedToast).toHaveBeenCalledTimes(1);
});

test("only imports once while an existing session is opening", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "share", tooLarge: false, files: [] });
  let release!: () => void;
  openSessionOnDesktop.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; }));
  render();
  await flush();
  const start = intake.chooseDestination;
  let importing!: Promise<void>;
  act(() => {
    intake.dismiss();
    importing = start("desktop", "session", "s1");
  });
  await flush();
  await act(async () => { await start("desktop", "session", "s1"); });
  await act(async () => {
    release();
    await importing;
  });
  expect(mockedSaveDraft).toHaveBeenCalledTimes(1);
  expect(openSessionOnDesktop).toHaveBeenCalledTimes(1);
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

test("receives an Open In document while already active, without uploading or changing drafts", async () => {
  render();
  await flush();
  const share = {
    text: "", tooLarge: false,
    files: [{ uri: "file:///cache/report.pdf", name: "report.pdf", mimeType: "application/pdf" }],
  };
  mockedGetPendingShare.mockResolvedValueOnce(share);
  act(() => (addPendingShareListener as jest.Mock).mock.calls.at(-1)[0]());
  await flush();
  expect(intake.pending).toEqual(share);
  expect(mockedPrepare).not.toHaveBeenCalled();
  expect(mockedSaveDraft).not.toHaveBeenCalled();
  expect(newConversationOn).not.toHaveBeenCalled();
});

test("retries a native receipt arriving during an empty inbox read", async () => {
  let release!: (value: null) => void;
  mockedGetPendingShare.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  render();
  await flush();
  mockedGetPendingShare.mockResolvedValueOnce({ text: "opened", files: [], tooLarge: false });
  act(() => (addPendingShareListener as jest.Mock).mock.calls.at(-1)[0]());
  await act(async () => { release(null); });
  await flush();
  expect(intake.pending?.text).toBe("opened");
});

test("reports an unreadable Open In document", async () => {
  mockedGetPendingShare.mockResolvedValueOnce({ text: "", files: [], tooLarge: false, failed: true });
  render();
  await flush();
  expect(mockedToast).toHaveBeenCalledWith("attachment.errors.attachment_failed");
  expect(intake.pending).toBeNull();
});

test("removes the native receipt listener on unmount", async () => {
  render();
  await flush();
  const subscription = (addPendingShareListener as jest.Mock).mock.results.at(-1)!.value;
  act(() => renderer!.unmount());
  renderer = null;
  expect(subscription.remove).toHaveBeenCalledTimes(1);
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
