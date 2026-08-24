import type { AgentMessage } from "@future-os/thread-projection";
import type { SetStateAction } from "react";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { sendPromptToFutureAgent } from "../../integrations/agent/agentClient";
import { createRun, getRun, listRunEvents, updateRunStatus } from "../../integrations/storage/threadStore";
import { buildReferenceContext } from "./buildReferencePrompt";
import { runSendPipeline } from "./sendPipeline";
import { finalizeTemporaryAttachmentSources } from "./threadAttachments";

vi.mock("../../integrations/storage/threadStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../integrations/storage/threadStore")>();
  return {
    ...actual,
    createRun: vi.fn(),
    getRun: vi.fn(),
    listRunEvents: vi.fn(),
    listRunEventsSince: vi.fn(async () => []),
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
  finalizeTemporaryAttachmentSources: vi.fn(async () => {}),
  persistImageAttachments: vi.fn(async () => ({ attachments: [], temporarySources: [] })),
}));
vi.mock("./buildReferencePrompt", () => ({
  buildReferenceContext: vi.fn(async () => ""),
}));

const emitFutureEvent = vi.fn();
vi.mock("../../lib/futureEvents", () => ({
  emitFutureEvent: (...args: unknown[]) => emitFutureEvent(...args),
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
    vi.mocked(buildReferenceContext).mockResolvedValue("");
  });

  it("keeps model-only reference context out of the user message", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "completed", endedAt: 2_000 }));
    vi.mocked(buildReferenceContext).mockResolvedValue("Referenced FutureOS objects:\n1. file:utils/a.py");
    const setMessages = vi.fn<(value: SetStateAction<AgentMessage[]>) => void>();

    await runSendPipeline(makeDeps(setMessages), { content: "what is a.py?", attachments: [] });

    expect(sendPromptToFutureAgent).toHaveBeenCalledWith(expect.objectContaining({
      message: "what is a.py?",
      modelContext: "Referenced FutureOS objects:\n1. file:utils/a.py",
    }));
    const user = foldMessages(setMessages).find(message => message.role === "user");
    expect(user?.content).toBe("what is a.py?");
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
    expect(finalizeTemporaryAttachmentSources).toHaveBeenCalledWith([]);
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

describe("runSendPipeline stream/failure edges", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(buildReferenceContext).mockResolvedValue("");
    vi.mocked(createRun).mockResolvedValue(storedRun());
    vi.mocked(listRunEvents).mockResolvedValue([]);
  });

  it("toasts when the agent recreated the session", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "completed", endedAt: 2_000 }));
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "answer",
      complete: true,
      sessionId: "session-2",
      sessionRecreated: true,
    });
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });
    expect(emitFutureEvent).toHaveBeenCalledWith("toast", expect.objectContaining({ tone: "info" }));
  });

  it("marks the run failed when the stream closes incomplete and the row is still active", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun());
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "truncated",
      complete: false,
      sessionId: "session-1",
      sessionRecreated: false,
    });
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });
    expect(updateRunStatus).toHaveBeenCalledWith(
      expect.objectContaining({ runId: "run-1", status: "failed" }),
    );
  });

  it("finalizes the bubble in place when cancelled before any text landed", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "cancelled", endedAt: 2_000 }));
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "",
      complete: false,
      sessionId: "session-1",
      sessionRecreated: false,
    });
    const setMessages = vi.fn();
    const deps = makeDeps(setMessages);
    await runSendPipeline(deps, { content: "hello", attachments: [] });
    const assistant = foldMessages(setMessages).filter(m => m.role === "assistant").pop();
    expect(assistant?.stopped).toBe(true);
    expect(assistant?.thinkingActive).toBe(false);
    expect(deps.onThreadActivity).toHaveBeenCalled();
  });

  it("marks the run failed and renders the failure when the invoke throws", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun());
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("transport down"));
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });
    expect(finalizeTemporaryAttachmentSources).not.toHaveBeenCalled();
    expect(updateRunStatus).toHaveBeenCalledWith(
      expect.objectContaining({ runId: "run-1", status: "failed" }),
    );
    const assistant = foldMessages(setMessages).filter(m => m.role === "assistant").pop();
    expect(assistant?.status).toBe("failed");
    expect(assistant?.content).toContain("transport down");
  });

  it("skips the status write in the failure path when the run already settled", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "failed", endedAt: 2_000 }));
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("late failure"));
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });
    expect(updateRunStatus).not.toHaveBeenCalled();
  });

  it("writes the failure when the run row is gone mid-flight", async () => {
    vi.mocked(getRun).mockResolvedValue(null);
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("gone"));
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });
    expect(updateRunStatus).toHaveBeenCalledWith(
      expect.objectContaining({ runId: "run-1", status: "failed" }),
    );
  });

  it("pushes stream updates into the pending bubble while the run streams", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    let handler: ((event: { payload: Record<string, unknown> }) => void) | null = null;
    vi.mocked(listen).mockImplementation(async (...args: unknown[]) => {
      handler = args[1] as typeof handler;
      return () => {};
    });
    let resolveReply!: (value: { content: string; complete: boolean; sessionId: string; sessionRecreated: boolean }) => void;
    vi.mocked(sendPromptToFutureAgent).mockImplementation(
      () => new Promise((resolve) => {
        resolveReply = resolve;
      }),
    );
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "completed", endedAt: 2_000 }));
    vi.mocked(listRunEvents).mockResolvedValue([]);
    const setMessages = vi.fn();
    const send = runSendPipeline(makeDeps(setMessages), { content: "hello", attachments: [] });
    await vi.waitFor(() => {
      expect(handler).not.toBeNull();
    });
    // Matching run: resetProjection variant + plain variant.
    handler!({ payload: { runId: "run-1", resetProjection: true } });
    handler!({ payload: { runId: "run-1", resetProjection: false } });
    // A different run's event is ignored.
    handler!({ payload: { runId: "run-other", resetProjection: true } });
    const { listRunEventsSince } = await import("../../integrations/storage/threadStore");
    await vi.waitFor(() => {
      expect(listRunEventsSince).toHaveBeenCalled();
    });
    resolveReply({
      content: "answer",
      complete: true,
      sessionId: "session-1",
      sessionRecreated: false,
    });
    await send;
  });
});
