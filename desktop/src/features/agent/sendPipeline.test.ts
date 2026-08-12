import type { AgentMessage } from "@future-os/thread-projection";
import type { SetStateAction } from "react";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { sendPromptToFutureAgent } from "../../integrations/agent/agentClient";
import { createRun, getRun, listRunEvents, updateRunStatus } from "../../integrations/storage/threadStore";
import { runSendPipeline } from "./sendPipeline";

vi.mock("../../integrations/storage/threadStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../integrations/storage/threadStore")>();
  return {
    ...actual,
    createRun: vi.fn(),
    getRun: vi.fn(),
    listRunEvents: vi.fn(),
    updateRunStatus: vi.fn(),
  };
});
vi.mock("../../integrations/agent/agentClient", () => ({
  sendPromptToFutureAgent: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("./threadAttachments", () => ({
  persistImageAttachments: vi.fn(async () => []),
}));

const thread = {
  id: "thread-1",
  workspaceId: "workspace-1",
  agentSessionId: "session-1",
} as unknown as StoredThread;

function storedRun(partial: Partial<StoredRun> = {}): StoredRun {
  return {
    id: "run-1",
    threadId: "thread-1",
    triggerMessageId: null,
    status: "running",
    modelProvider: null,
    modelId: "provider/model",
    startedAt: 1_000,
    endedAt: null,
    errorMessage: null,
    errorType: null,
    createdAt: 1_000,
    updatedAt: 1_000,
    ...partial,
  };
}

type MockedSetMessages = ReturnType<typeof vi.fn<(value: SetStateAction<AgentMessage[]>) => void>>;

/**
 * Fold every functional updater the pipeline handed to `setMessages` into a
 * final message list, so tests can assert the rendered bubble without knowing
 * the client-generated ids.
 */
function foldMessages(setMessages: MockedSetMessages): AgentMessage[] {
  return setMessages.mock.calls.reduce<AgentMessage[]>((messages, call) => {
    const updater = call[0] as (previous: AgentMessage[]) => AgentMessage[];
    return updater(messages);
  }, []);
}

function makeDeps(setMessages: MockedSetMessages) {
  return {
    thread,
    modelId: "provider/model",
    thinkingLevel: "off",
    setMessages,
    setRecentRun: vi.fn(),
    refreshRecentRun: vi.fn(async () => {}),
    onThreadActivity: vi.fn(),
    isCurrentSend: () => true,
  };
}

describe("runSendPipeline terminal-status handling", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(createRun).mockResolvedValue(storedRun());
    vi.mocked(listRunEvents).mockResolvedValue([]);
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "final answer",
      complete: true,
      sessionId: "session-1",
      sessionRecreated: false,
    });
  });

  it("renders the final bubble without a redundant status write when the backend already settled the run", async () => {
    // The backend CASes the run `completed` the instant the stream ends, so by
    // the time the invoke response resolves the row is already terminal. The
    // pipeline must still finalize the bubble — an early return here is the
    // bug that left the reply frozen as "streaming".
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "completed", endedAt: 2_000 }));
    const setMessages = vi.fn<(value: SetStateAction<AgentMessage[]>) => void>();

    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });

    expect(updateRunStatus).not.toHaveBeenCalled();
    const assistant = foldMessages(setMessages).filter(m => m.role === "assistant").pop();
    expect(assistant?.status).toBe("complete");
    expect(assistant?.content).toBe("final answer");
  });

  it("writes completed itself when the run is still active when the response lands", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun());
    const setMessages = vi.fn<(value: SetStateAction<AgentMessage[]>) => void>();

    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });

    expect(updateRunStatus).toHaveBeenCalledWith(
      expect.objectContaining({ runId: "run-1", status: "completed" }),
    );
    const assistant = foldMessages(setMessages).filter(m => m.role === "assistant").pop();
    expect(assistant?.status).toBe("complete");
    expect(assistant?.content).toBe("final answer");
  });

  it("keeps the failed row and still renders when the backend settled a truncated stream", async () => {
    // Backend settles `failed` for a stream that closed before `agent_end`;
    // the pipeline sees the terminal row and must not rewrite it, but still
    // leaves the view in a settled (non-streaming) state.
    vi.mocked(getRun).mockResolvedValue(
      storedRun({ status: "failed", errorType: "unknown", endedAt: 2_000 }),
    );
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "partial answer",
      complete: false,
      sessionId: "session-1",
      sessionRecreated: false,
    });
    const setMessages = vi.fn<(value: SetStateAction<AgentMessage[]>) => void>();

    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });

    expect(updateRunStatus).not.toHaveBeenCalled();
    const assistant = foldMessages(setMessages).filter(m => m.role === "assistant").pop();
    expect(assistant?.status).toBe("failed");
  });

  it("keeps the cancelled early return: stopped bubble, no fall-through render", async () => {
    // A user abort keeps its own finalization (partial text, `stopped`) and
    // returns early — it must not fall through to the completion render.
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "cancelled", endedAt: 2_000 }));
    const setMessages = vi.fn();
    const deps = makeDeps(setMessages);

    await runSendPipeline(deps, { content: "hello", attachments: [] });

    expect(updateRunStatus).not.toHaveBeenCalled();
    // Early return: exactly one run read (the settle check), no second
    // loadCurrentRun from the completion path.
    expect(getRun).toHaveBeenCalledTimes(1);
    const assistant = foldMessages(setMessages).filter(m => m.role === "assistant").pop();
    expect(assistant?.stopped).toBe(true);
    expect(assistant?.content).toBe("final answer");
    // Pipeline start + the cancelled finalization; a fall-through into the
    // completion render would add a third.
    expect(deps.onThreadActivity).toHaveBeenCalledTimes(2);
  });
});
