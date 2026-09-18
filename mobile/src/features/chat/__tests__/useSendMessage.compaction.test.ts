import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useSendMessage, type SendMessageApi } from "../useSendMessage";
import { emptyTimeline } from "../../../remote/timeline";
import type { TimelineItem } from "../../../remote/types";

jest.mock("../../../remote/RemoteContext", () => ({}));
jest.mock("../../../remote/files", () => ({ mimeFor: jest.fn() }));
jest.mock("../utils", () => ({ showToast: jest.fn() }));

test("pending manual compaction blocks send, retry and continue without consuming drafts", async () => {
  const assistant: TimelineItem = { kind: "message", role: "assistant", id: "a", runId: "r", text: "answer" };
  const remote = {
    compacting: false, selectedSessionId: "s",
    timeline: { ...emptyTimeline(), items: [{ kind: "message", role: "user", id: "u", text: "question" }, assistant] },
    sendMessage: jest.fn(async () => {}), continueRun: jest.fn(async () => {}),
  } as unknown as Parameters<typeof useSendMessage>[0];
  const setMessage = jest.fn(), setAttachments = jest.fn();
  let api!: SendMessageApi;
  function Harness({ pending }: { pending: boolean }) {
    api = useSendMessage(remote, ((key: string) => key) as Parameters<typeof useSendMessage>[1], "draft", [], setMessage, setAttachments, jest.fn(), pending);
    return null;
  }
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(Harness, { pending: true })); });
  try {
    await act(async () => { await api.send(); api.retryMessage(assistant); api.continueMessage(assistant); });
    expect(remote.sendMessage).not.toHaveBeenCalled();
    expect(remote.continueRun).not.toHaveBeenCalled();
    expect(setMessage).not.toHaveBeenCalled();
    expect(setAttachments).not.toHaveBeenCalled();
    act(() => tree.update(createElement(Harness, { pending: false })));
    await act(async () => { await api.send(); });
    expect(remote.sendMessage).toHaveBeenCalledTimes(1);
  } finally { act(() => tree.unmount()); }
});
