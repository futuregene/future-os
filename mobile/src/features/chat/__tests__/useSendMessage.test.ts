import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useSendMessage, type SendMessageApi } from "../useSendMessage";
import { showToast } from "../utils";
import type { HistoryAttachment, MobileAttachment, TimelineItem } from "../../../remote/types";

jest.mock("../../../remote/RemoteContext", () => ({}));
jest.mock("../../../remote/files", () => ({ mimeFor: jest.fn(() => "image/png") }));
jest.mock("../utils", () => ({ showToast: jest.fn(), deferPresentation: jest.fn() }));
jest.mock("expo-file-system", () => ({
  File: class MockFile {
    uri: string;
    size = 42;
    constructor(uri: string) { this.uri = uri; }
  },
}));

const toast = showToast as jest.Mock;
const t = (key: string) => `t:${key}`;

function remoteFor(overrides: Record<string, unknown> = {}) {
  return {
    cachedAttachment: jest.fn(() => null),
    compacting: false,
    continueRun: jest.fn(async () => {}),
    downloadAttachment: jest.fn(async () => ({ uri: "file:///cache/downloaded.bin" })),
    prepareAttachment: jest.fn(async (attachment: HistoryAttachment) => ({
      mimeType: "application/pdf", name: `${attachment.name}.transfer`, path: "/tmp/x", size: 99,
    })),
    selectedSessionId: "session-1",
    sendMessage: jest.fn(async () => {}),
    timeline: { items: [] as TimelineItem[] },
    ...overrides,
  } as unknown as Parameters<typeof useSendMessage>[0];
}

let tree: ReactTestRenderer;
let api: SendMessageApi;
function mount(options: {
  remote: Parameters<typeof useSendMessage>[0];
  message?: string;
  attachments?: MobileAttachment[];
  compactionPending?: boolean;
}) {
  const setMessage = jest.fn();
  const setAttachments = jest.fn();
  const setTransferProgress = jest.fn();
  function Harness() {
    api = useSendMessage(
      options.remote, t as never, options.message ?? "", options.attachments ?? [],
      setMessage, setAttachments, setTransferProgress, options.compactionPending ?? false,
    );
    return null;
  }
  act(() => {
    if (tree) tree.update(createElement(Harness));
    else tree = create(createElement(Harness));
  });
  return { setAttachments, setMessage, setTransferProgress };
}

const attachment: MobileAttachment = {
  kind: "image", localUri: "file:///tmp/a.png", mimeType: "image/png", name: "a.png",
  originalSize: 12, transferSize: 12,
};
const userMessage = (fields: Partial<Extract<TimelineItem, { kind: "message" }>>): TimelineItem => ({
  id: "u", kind: "message", role: "user", text: "question", ...fields,
});
const assistant = (fields: Partial<Extract<TimelineItem, { kind: "message" }>> = {}): TimelineItem => ({
  id: "a", kind: "message", role: "assistant", text: "answer", runId: "run-1", ...fields,
});

beforeEach(() => jest.clearAllMocks());
afterEach(() => {
  if (tree) act(() => tree.unmount());
  tree = undefined as never;
});

describe("sending", () => {
  test("an empty composer sends nothing at all", async () => {
    const remote = remoteFor();
    const { setTransferProgress } = mount({ remote, message: "   " });
    await act(async () => { await api.send(); });
    expect(remote.sendMessage).not.toHaveBeenCalled();
    expect(setTransferProgress).not.toHaveBeenCalled();
  });

  test("a text send clears the composer and reports progress as the transfer advances", async () => {
    let report!: (done: number, total: number) => void;
    const remote = remoteFor({
      sendMessage: jest.fn(async (_text: string, _files: unknown, onProgress: typeof report) => { report = onProgress; }),
    });
    const { setAttachments, setMessage, setTransferProgress } = mount({ remote, message: "  hello  " });
    await act(async () => { await api.send(); });
    expect(remote.sendMessage).toHaveBeenCalledWith("hello", [], expect.any(Function));
    expect(setMessage).toHaveBeenCalledWith("");
    expect(setAttachments).toHaveBeenCalledWith([]);
    // Nailed to the end, whatever happens next.
    expect(setTransferProgress).toHaveBeenLastCalledWith(null);
    // An attachment-less send has no transfer to size.
    expect(setTransferProgress.mock.calls[0]).toEqual([null]);
    report(3, 12);
    expect(setTransferProgress).toHaveBeenCalledWith(0.25);
  });

  test("a send with attachments starts the progress bar at zero", async () => {
    const remote = remoteFor();
    const { setTransferProgress } = mount({ remote, message: "hi", attachments: [attachment] });
    await act(async () => { await api.send(); });
    expect(setTransferProgress.mock.calls[0]).toEqual([0]);
    expect(setTransferProgress).toHaveBeenLastCalledWith(null);
  });

  test("an unknown total leaves the bar indeterminate instead of dividing by zero", async () => {
    let report!: (done: number, total: number) => void;
    const remote = remoteFor({
      sendMessage: jest.fn(async (_text: string, _files: unknown, onProgress: typeof report) => { report = onProgress; }),
    });
    const { setTransferProgress } = mount({ remote, attachments: [attachment] });
    await act(async () => { await api.send(); });
    report(0, 0);
    expect(setTransferProgress).toHaveBeenCalledWith(null);
  });

  test("an override sends the composed skill command, not the stale composer text", async () => {
    const remote = remoteFor();
    mount({ remote, message: "old draft" });
    await act(async () => { await api.send("old draft /future-web"); });
    expect(remote.sendMessage).toHaveBeenCalledWith("old draft /future-web", [], expect.any(Function));
  });

  test.each([
    ["send_compacting", "t:chat.compacting"],
    ["prompt_too_large", "t:chat.promptTooLarge"],
    ["send_disconnected", "t:chat.sendFailed"],
  ])("a failed send reports %s and hands the draft back", async (error, expectedToast) => {
    const remote = remoteFor({ sendMessage: jest.fn(async () => { throw new Error(error); }) });
    const { setAttachments, setMessage, setTransferProgress } = mount({ remote, message: "keep me", attachments: [attachment] });
    await act(async () => { await api.send(); });
    expect(toast).toHaveBeenCalledWith(expectedToast);
    expect(setMessage).toHaveBeenCalledWith("keep me");
    // Nothing vanishes: the attachments stay in the composer for a retry.
    expect(setAttachments).not.toHaveBeenCalled();
    expect(setTransferProgress).toHaveBeenLastCalledWith(null);
  });

  test("a rejection that is not an Error still restores the draft", async () => {
    const remote = remoteFor({ sendMessage: jest.fn(async () => { throw "boom"; }) });
    const { setMessage } = mount({ remote, message: "keep me" });
    await act(async () => { await api.send(); });
    expect(toast).toHaveBeenCalledWith("t:chat.sendFailed");
    expect(setMessage).toHaveBeenCalledWith("keep me");
  });

  test("a manual compaction in flight blocks the send without eating the draft", async () => {
    const remote = remoteFor();
    const { setMessage } = mount({ remote, message: "keep me", compactionPending: true });
    await act(async () => { await api.send(); });
    expect(toast).toHaveBeenCalledWith("t:chat.compacting");
    expect(remote.sendMessage).not.toHaveBeenCalled();
    expect(setMessage).not.toHaveBeenCalled();
  });
});

describe("retrying a failed answer", () => {
  test("a retry re-attaches the local file the user picked", async () => {
    const remote = remoteFor();
    remote.timeline.items = [
      userMessage({
        id: "u1",
        text: "look at this",
        attachments: [{ kind: "image", name: "shot.png", path: "file:///tmp/shot.png" }],
      }),
      assistant({ id: "a1" }),
    ];
    mount({ remote });
    await act(async () => { api.retryMessage(assistant({ id: "a1" })); await Promise.resolve(); });
    expect(remote.prepareAttachment).not.toHaveBeenCalled();
    expect(remote.sendMessage).toHaveBeenCalledWith("look at this", [
      expect.objectContaining({
        localUri: "file:///tmp/shot.png", name: "shot.png", kind: "image", mimeType: "image/png",
      }),
    ]);
  });

  test("a retry of a desktop-side file asks the desktop for it and reuses its cache", async () => {
    const remote = remoteFor({
      cachedAttachment: jest.fn(() => ({ file: { uri: "file:///cache/kept.pdf" } })),
    });
    remote.timeline.items = [
      userMessage({ id: "u1", text: "here", attachments: [{ kind: "file", name: "doc.pdf", path: "/desktop/doc.pdf" }] }),
      assistant({ id: "a1" }),
    ];
    mount({ remote });
    await act(async () => { api.retryMessage(assistant({ id: "a1" })); await Promise.resolve(); });
    expect(remote.prepareAttachment).toHaveBeenCalledWith(expect.objectContaining({ path: "/desktop/doc.pdf" }));
    expect(remote.downloadAttachment).not.toHaveBeenCalled();
    expect(remote.sendMessage).toHaveBeenCalledWith("here", [
      expect.objectContaining({
        localUri: "file:///cache/kept.pdf", name: "doc.pdf",
        transferName: "doc.pdf.transfer", mimeType: "application/pdf",
        originalSize: 99, transferSize: 99,
      }),
    ]);
  });

  test("a retry downloads a file the phone has not cached yet", async () => {
    const remote = remoteFor();
    remote.timeline.items = [
      userMessage({ id: "u1", text: "here", attachments: [{ name: "doc.pdf", path: "/desktop/doc.pdf" }] }),
      assistant({ id: "a1" }),
    ];
    mount({ remote });
    await act(async () => { api.retryMessage(assistant({ id: "a1" })); await Promise.resolve(); });
    expect(remote.downloadAttachment).toHaveBeenCalledTimes(1);
    expect(remote.sendMessage).toHaveBeenCalledWith("here", [
      expect.objectContaining({ localUri: "file:///cache/downloaded.bin" }),
    ]);
  });

  test("a retry that cannot re-attach the file says so instead of failing silently", async () => {
    const remote = remoteFor({
      prepareAttachment: jest.fn(async () => { throw new Error("desktop offline"); }),
    });
    remote.timeline.items = [
      userMessage({ id: "u1", text: "here", attachments: [{ name: "doc.pdf", path: "/desktop/doc.pdf" }] }),
      assistant({ id: "a1" }),
    ];
    mount({ remote });
    await act(async () => { api.retryMessage(assistant({ id: "a1" })); await Promise.resolve(); await Promise.resolve(); });
    expect(toast).toHaveBeenCalledWith("t:chat.sendFailed");
    expect(remote.sendMessage).not.toHaveBeenCalled();
  });

  test("a retry walks back to the nearest user message, not the first one", async () => {
    const remote = remoteFor();
    remote.timeline.items = [
      userMessage({ id: "u1", text: "first" }),
      assistant({ id: "a1" }),
      userMessage({ id: "u2", text: "second" }),
      assistant({ id: "a2" }),
    ];
    mount({ remote });
    await act(async () => { api.retryMessage(assistant({ id: "a2" })); await Promise.resolve(); });
    expect(remote.sendMessage).toHaveBeenCalledWith("second", []);
  });

  test("a retry with no user message above it does nothing", async () => {
    const remote = remoteFor();
    remote.timeline.items = [assistant({ id: "a1" })];
    mount({ remote });
    await act(async () => { api.retryMessage(assistant({ id: "a1" })); await Promise.resolve(); });
    expect(remote.sendMessage).not.toHaveBeenCalled();
  });

  test.each([
    ["a user message", userMessage({ id: "u1" })],
    ["a notice", { id: "n1", kind: "notice", tone: "warning", text: "careful" } as TimelineItem],
  ])("a retry of %s is ignored", async (_label, item) => {
    const remote = remoteFor();
    remote.timeline.items = [userMessage({ id: "u1" }), item];
    mount({ remote });
    await act(async () => { api.retryMessage(item); await Promise.resolve(); });
    expect(remote.sendMessage).not.toHaveBeenCalled();
  });

  test("a retry while a compaction is pending only tells the user why", async () => {
    const remote = remoteFor();
    remote.timeline.items = [userMessage({ id: "u1" }), assistant({ id: "a1" })];
    mount({ remote, compactionPending: true });
    await act(async () => { api.retryMessage(assistant({ id: "a1" })); await Promise.resolve(); });
    expect(toast).toHaveBeenCalledWith("t:chat.compacting");
    expect(remote.sendMessage).not.toHaveBeenCalled();
  });
});

describe("continuing a truncated answer", () => {
  test("a continue resumes the run on the selected session", async () => {
    const remote = remoteFor();
    mount({ remote });
    await act(async () => { api.continueMessage(assistant({ id: "a1", runId: "run-9" })); await Promise.resolve(); });
    expect(remote.continueRun).toHaveBeenCalledWith("session-1", "run-9");
  });

  test("a continue that fails tells the user", async () => {
    const remote = remoteFor({ continueRun: jest.fn(async () => { throw new Error("nope"); }) });
    mount({ remote });
    await act(async () => { api.continueMessage(assistant({ id: "a1" })); await Promise.resolve(); });
    expect(toast).toHaveBeenCalledWith("t:chat.sendFailed");
  });

  test("an answer with no run to resume is not continued", async () => {
    const remote = remoteFor();
    mount({ remote });
    await act(async () => { api.continueMessage(assistant({ id: "a1", runId: undefined })); await Promise.resolve(); });
    expect(remote.continueRun).not.toHaveBeenCalled();
  });

  test("a continue while a compaction is pending only tells the user why", async () => {
    const remote = remoteFor();
    mount({ remote, compactionPending: true });
    await act(async () => { api.continueMessage(assistant({ id: "a1" })); await Promise.resolve(); });
    expect(toast).toHaveBeenCalledWith("t:chat.compacting");
    expect(remote.continueRun).not.toHaveBeenCalled();
  });
});
