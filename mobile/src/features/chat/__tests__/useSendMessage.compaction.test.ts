import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useSendMessage, type SendMessageApi } from "../useSendMessage";
import { emptyTimeline } from "@future-os/remote-client/timeline";
import type { TimelineItem } from "../../../remote/types";

jest.mock("../../../remote/RemoteContext", () => ({}));
jest.mock("../../../remote/files", () => ({ mimeFor: jest.fn() }));
jest.mock("../utils", () => ({ showToast: jest.fn() }));

test("a compaction no longer blocks send or retry; continue still waits for it", async () => {
  // The Agent accepts a prompt while it compacts and holds it in the session's
  // queue, so neither a send nor a retry is refused here. Continuing the
  // assistant's turn is a different command and still needs the session.
  const assistant: TimelineItem = { kind: "message", role: "assistant", id: "a", runId: "r", text: "answer" };
  const remote = {
    compacting: false, selectedSessionId: "s",
    timeline: { ...emptyTimeline(), items: [{ kind: "message", role: "user", id: "u", text: "question" }, assistant] },
    sendMessage: jest.fn(async () => {}), continueRun: jest.fn(async () => {}),
  } as unknown as Parameters<typeof useSendMessage>[0]["remote"];
  const setMessage = jest.fn(), setAttachments = jest.fn();
  let api!: SendMessageApi;
  function Harness({ pending }: { pending: boolean }) {
    api = useSendMessage({ remote, t: ((key: string) => key) as Parameters<typeof useSendMessage>[0]["t"],
      message: "draft", attachments: [], setMessage, setAttachments,
      setTransferProgress: jest.fn(), compactionPending: pending, streaming: false });
    return null;
  }
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(Harness, { pending: true })); });
  try {
    await act(async () => { await api.send(); });
    expect(remote.sendMessage).toHaveBeenCalledTimes(1);
    expect(remote.sendMessage).toHaveBeenCalledWith("draft", [], expect.any(Function), undefined);
    await act(async () => { api.retryMessage(assistant); });
    expect(remote.sendMessage).toHaveBeenCalledTimes(2);
    await act(async () => { api.continueMessage(assistant); });
    expect(remote.continueRun).not.toHaveBeenCalled();
  } finally { act(() => tree.unmount()); }
});
