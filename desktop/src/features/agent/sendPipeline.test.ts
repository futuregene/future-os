import type { AgentMessage } from "@future-os/thread-projection";
import type { SetStateAction } from "react";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { sendPromptToFutureAgent } from "../../integrations/agent/agentClient";
import { createRun, getRun, listRunEvents, updateRunStatus } from "../../integrations/storage/threadStore";
import { buildReferenceContext } from "./buildReferencePrompt";
import { runSendPipeline } from "./sendPipeline";
import { finalizeTemporaryAttachmentSources } from "./threadAttachments";
import { updatePendingMessageFromRunEvents } from "./threadRunProjection";

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
  persistImageAttachments: vi.fn(async () => ({
    attachments: [],
    temporarySources: [],
  })),
}));
vi.mock("./buildReferencePrompt", () => ({
  buildReferenceContext: vi.fn(async () => ""),
}));
vi.mock("./threadRunProjection", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./threadRunProjection")>();
  return {
    ...actual,
    // Delegate to the real implementation by default (the streaming tests below
    // exercise the live-preview projection through it); the race test overrides
    // this to control when the projection promise settles.
    updatePendingMessageFromRunEvents: vi.fn(actual.updatePendingMessageFromRunEvents),
  };
});

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
    });
    vi.mocked(buildReferenceContext).mockResolvedValue("");
  });

  it("keeps model-only reference context out of the user message", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "completed", endedAt: 2_000 }));
    vi.mocked(buildReferenceContext).mockResolvedValue("Referenced FutureOS objects:\n1. file:utils/a.py");
    const setMessages = vi.fn<(value: SetStateAction<AgentMessage[]>) => void>();

    await runSendPipeline(makeDeps(setMessages), {
      content: "what is a.py?",
      attachments: [],
    });

    expect(sendPromptToFutureAgent).toHaveBeenCalledWith(
      expect.objectContaining({
        message: "what is a.py?",
        modelContext: "Referenced FutureOS objects:\n1. file:utils/a.py",
      }),
    );
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

    await runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });

    expect(updateRunStatus).not.toHaveBeenCalled();
    expect(finalizeTemporaryAttachmentSources).toHaveBeenCalledWith([]);
    const assistant = foldMessages(setMessages)
      .filter(m => m.role === "assistant")
      .pop();
    expect(assistant?.status).toBe("complete");
    expect(assistant?.content).toBe("final answer");
  });

  it("writes completed itself when the run is still active when the response lands", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun());
    const setMessages = vi.fn<(value: SetStateAction<AgentMessage[]>) => void>();

    await runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });

    expect(updateRunStatus).toHaveBeenCalledWith(expect.objectContaining({ runId: "run-1", status: "completed" }));
    const assistant = foldMessages(setMessages)
      .filter(m => m.role === "assistant")
      .pop();
    expect(assistant?.status).toBe("complete");
    expect(assistant?.content).toBe("final answer");
  });

  it("keeps the failed row and still renders when the backend settled a truncated stream", async () => {
    // Backend settles `failed` for a stream that closed before `agent_end`;
    // the pipeline sees the terminal row and must not rewrite it, but still
    // leaves the view in a settled (non-streaming) state.
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "failed", errorType: "unknown", endedAt: 2_000 }));
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "partial answer",
      complete: false,
      sessionId: "session-1",
    });
    const setMessages = vi.fn<(value: SetStateAction<AgentMessage[]>) => void>();

    await runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });

    expect(updateRunStatus).not.toHaveBeenCalled();
    const assistant = foldMessages(setMessages)
      .filter(m => m.role === "assistant")
      .pop();
    expect(assistant?.status).toBe("failed");
    expect(assistant?.terminationTitle).toContain("cause is unconfirmed");
    expect(assistant?.terminationNotice).toContain("Generated content has been kept");
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
    const assistant = foldMessages(setMessages)
      .filter(m => m.role === "assistant")
      .pop();
    expect(assistant?.stopped).toBe(true);
    expect(assistant?.terminationNotice).toContain("stopped this response manually");
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

  it("marks the run failed when the stream closes incomplete and the row is still active", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun());
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "truncated",
      complete: false,
      sessionId: "session-1",
    });
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });
    expect(updateRunStatus).toHaveBeenCalledWith(expect.objectContaining({ runId: "run-1", status: "failed" }));
  });

  it("finalizes the bubble in place when cancelled before any text landed", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "cancelled", endedAt: 2_000 }));
    vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
      content: "",
      complete: false,
      sessionId: "session-1",
    });
    const setMessages = vi.fn();
    const deps = makeDeps(setMessages);
    await runSendPipeline(deps, { content: "hello", attachments: [] });
    const assistant = foldMessages(setMessages)
      .filter(m => m.role === "assistant")
      .pop();
    expect(assistant?.stopped).toBe(true);
    expect(assistant?.thinkingActive).toBe(false);
    expect(assistant?.terminationNotice).toContain("stopped this response manually");
    expect(deps.onThreadActivity).toHaveBeenCalled();
  });

  it("marks the run failed and renders the failure when the invoke throws", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun());
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("transport down"));
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });
    expect(finalizeTemporaryAttachmentSources).not.toHaveBeenCalled();
    expect(updateRunStatus).toHaveBeenCalledWith(expect.objectContaining({ runId: "run-1", status: "failed" }));
    const assistant = foldMessages(setMessages)
      .filter(m => m.role === "assistant")
      .pop();
    expect(assistant?.status).toBe("failed");
    expect(assistant?.content).toBe("");
    expect(assistant?.terminationTitle).toContain("Model service error");
    expect(assistant?.terminationNotice).toContain("Please try again later");
  });

  it.each(["completed", "cancelled"] as const)("honors a durable %s run after a late attach error", async (status) => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status, endedAt: 2_000 }));
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("Future Agent run ended before the stream attached"));
    vi.mocked(listRunEvents).mockResolvedValue([{
      id: "durable-text",
      runId: "run-1",
      sequence: 0,
      eventType: "text_chunk",
      payload: JSON.stringify({ text: "durable answer" }),
      createdAt: 1000,
    }]);
    const setMessages = vi.fn();
    const deps = makeDeps(setMessages);
    await runSendPipeline(deps, { content: "hello", attachments: [] });
    expect(updateRunStatus).not.toHaveBeenCalled();
    expect(deps.setRecentRun).toHaveBeenLastCalledWith(expect.objectContaining({ status }));
    expect(foldMessages(setMessages).slice(-1)[0]).toMatchObject({
      status: "complete",
      stopped: status === "cancelled",
      content: "durable answer",
      runError: undefined,
    });
  });

  it("skips the status write in the failure path when the run already settled", async () => {
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "failed", endedAt: 2_000 }));
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("late failure"));
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });
    expect(updateRunStatus).not.toHaveBeenCalled();
  });

  it("writes the failure when the run row is gone mid-flight", async () => {
    vi.mocked(getRun).mockResolvedValue(null);
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("gone"));
    const setMessages = vi.fn();
    await runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });
    expect(updateRunStatus).toHaveBeenCalledWith(expect.objectContaining({ runId: "run-1", status: "failed" }));
  });

  it("pushes stream updates into the pending bubble while the run streams", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    let handler: ((event: { payload: Record<string, unknown> }) => void) | null = null;
    vi.mocked(listen).mockImplementation(async (...args: unknown[]) => {
      handler = args[1] as typeof handler;
      return () => {};
    });
    let resolveReply!: (value: {
      content: string;
      complete: boolean;
      sessionId: string;
    }) => void;
    vi.mocked(sendPromptToFutureAgent).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveReply = resolve;
        }),
    );
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "completed", endedAt: 2_000 }));
    vi.mocked(listRunEvents).mockResolvedValue([]);
    const setMessages = vi.fn();
    const send = runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });
    await vi.waitFor(() => {
      expect(handler).not.toBeNull();
    });
    // Matching run: resetProjection variant + plain variant.
    handler!({
      payload: { updates: [{ runId: "run-1", resetProjection: true }] },
    });
    handler!({
      payload: { updates: [{ runId: "run-1", resetProjection: false }] },
    });
    // A different run's event is ignored.
    handler!({
      payload: { updates: [{ runId: "run-other", resetProjection: true }] },
    });
    const { listRunEventsSince } = await import("../../integrations/storage/threadStore");
    await vi.waitFor(() => {
      // One leading read plus one trailing read: burst notifications never run
      // journal IPCs concurrently, but the newest tail is still consumed.
      expect(listRunEventsSince).toHaveBeenCalledTimes(2);
    });
    resolveReply({
      content: "answer",
      complete: true,
      sessionId: "session-1",
    });
    await send;
  });

  it("re-queues a trailing update that lands between the loop exit and its finally", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    let handler: ((event: { payload: Record<string, unknown> }) => void) | null = null;
    vi.mocked(listen).mockImplementation(async (...args: unknown[]) => {
      handler = args[1] as typeof handler;
      return () => {};
    });

    let resolveReply!: (value: {
      content: string;
      complete: boolean;
      sessionId: string;
    }) => void;
    vi.mocked(sendPromptToFutureAgent).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveReply = resolve;
        }),
    );
    vi.mocked(getRun).mockResolvedValue(storedRun({ status: "completed", endedAt: 2_000 }));
    vi.mocked(listRunEvents).mockResolvedValue([]);

    // First projection settles on a controlled promise; later calls resolve
    // immediately so the re-queued iteration terminates cleanly.
    let firstCallPromise!: Promise<void>;
    let resolveFirst!: () => void;
    let calls = 0;
    vi.mocked(updatePendingMessageFromRunEvents).mockImplementation(() => {
      calls += 1;
      if (calls === 1) {
        firstCallPromise = new Promise<void>((resolve) => {
          resolveFirst = resolve;
        });
        return firstCallPromise;
      }
      return Promise.resolve();
    });

    const setMessages = vi.fn();
    const send = runSendPipeline(makeDeps(setMessages), {
      content: "hello",
      attachments: [],
    });
    await vi.waitFor(() => {
      expect(handler).not.toBeNull();
    });

    // First event starts the streaming loop (iteration 1, awaiting update #1).
    handler!({
      payload: { updates: [{ runId: "run-1", resetProjection: false }] },
    });
    await vi.waitFor(() => {
      expect(calls).toBe(1);
    });

    // Schedule a trailing event to fire in the microtask gap AFTER the loop's
    // await continuation (which exits the loop) but BEFORE its `.finally` — the
    // exact race the guard on the re-queue line exists to recover from.
    firstCallPromise.then(() => {
      handler!({
        payload: { updates: [{ runId: "run-1", resetProjection: false }] },
      });
    });
    resolveFirst();

    // The re-queued update spins up a fresh loop iteration (a second call).
    await vi.waitFor(() => {
      expect(calls).toBe(2);
    });

    resolveReply({
      content: "answer",
      complete: true,
      sessionId: "session-1",
    });
    await send;
  });
});

it("persists an interrupted reply but repaints nothing once the send is superseded", async () => {
  // concurrency: the stream closed incomplete and a thread switch landed while the
  // run row was being read. The reply must still be persisted (it is durable text
  // the returned-to thread reads back), but neither the sidebar refresh nor the
  // bubble repaint belongs to a view that no longer owns the send.
  let current = true;
  vi.mocked(getRun).mockImplementation(async () => {
    current = false;
    return storedRun();
  });
  vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
    content: "partial answer",
    complete: false,
    sessionId: "session-1",
  });
  const setMessages = vi.fn();
  const deps = { ...makeDeps(setMessages), isCurrentSend: () => current };

  await runSendPipeline(deps, { content: "hello", attachments: [] });

  expect(deps.refreshRecentRun).not.toHaveBeenCalled();
  const assistants = foldMessages(setMessages).filter(message => message.role === "assistant");
  expect(assistants.map(message => message.content)).not.toContain("partial answer");
});

it("does not repaint a cancelled exchange that was superseded mid-stream", async () => {
  // concurrency: the same guard on the CANCELLED outcome path. A user abort that
  // lands after this view stopped owning the send must not set `recentRun` or
  // patch the stopped bubble into whatever conversation is now on screen - the
  // durability half (the run row is already settled) is the backend's.
  let current = true;
  vi.mocked(getRun).mockImplementation(async () => {
    current = false;
    return storedRun({ status: "cancelled", endedAt: 2_000 });
  });
  vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
    content: "partial before the abort",
    complete: false,
    sessionId: "session-1",
  });
  const setMessages = vi.fn();
  const deps = { ...makeDeps(setMessages), isCurrentSend: () => current };

  await runSendPipeline(deps, { content: "hello", attachments: [] });

  // `setRecentRun` fires once, and that call is the pre-supersede one that pins the
  // RUNNING run. What the guard prevents is the cancelled row being pushed
  // afterwards - i.e. a second call, or any call carrying "cancelled".
  expect(deps.setRecentRun.mock.calls.map(call => (call[0] as { status?: string }).status)).toEqual(["running"]);
  const assistants = foldMessages(setMessages).filter(message => message.role === "assistant");
  expect(assistants.map(message => message.content)).not.toContain("partial before the abort");
  expect(assistants.some(message => message.stopped === true)).toBe(false);
});

it("leaves the new view alone when a cancelled empty run lands after the send was superseded", async () => {
  // concurrency: the sibling test above pins the same cancelled-before-text
  // finalization for a send that still owns the view. Here the abort is followed
  // by a thread switch, so `isCurrentSend()` flips false while the projection is
  // still resolving: the stopped bubble belongs to the abandoned conversation and
  // must not be painted into the one on screen.
  let current = true;
  vi.mocked(getRun).mockImplementation(async () => {
    current = false;
    return storedRun({ status: "cancelled", endedAt: 2_000 });
  });
  vi.mocked(sendPromptToFutureAgent).mockResolvedValue({
    content: "",
    complete: false,
    sessionId: "session-1",
  });
  const setMessages = vi.fn();
  const deps = { ...makeDeps(setMessages), isCurrentSend: () => current };

  await runSendPipeline(deps, { content: "hello", attachments: [] });

  const assistants = foldMessages(setMessages).filter(m => m.role === "assistant");
  expect(assistants.some(m => m.stopped === true)).toBe(false);
  expect(assistants.some(m => m.terminationNotice !== undefined)).toBe(false);
  // `createRun`'s resolution refreshes the sidebar unconditionally; the
  // finalization refresh inside the guard is what must not fire.
  expect(deps.onThreadActivity).toHaveBeenCalledTimes(1);
});

it("does not repaint a durable completion that arrived after the send was superseded", async () => {
  // concurrency: the attach/IPC failed, but the run row shows it actually
  // COMPLETED - the backend won a race. The recovery render exists so the user
  // sees the real answer rather than a bogus failure, but it too must stay off a
  // view that no longer owns this send.
  let current = true;
  vi.mocked(getRun).mockImplementation(async () => {
    current = false;
    return storedRun({ status: "completed", endedAt: 2_000 });
  });
  vi.mocked(listRunEvents).mockResolvedValue([{
    id: "durable-text",
    runId: "run-1",
    sequence: 0,
    eventType: "text_chunk",
    payload: JSON.stringify({ text: "durable answer" }),
    createdAt: 1000,
  }] as never);
  vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("Future Agent run ended before the stream attached"));
  const setMessages = vi.fn();
  const deps = { ...makeDeps(setMessages), isCurrentSend: () => current };

  await runSendPipeline(deps, { content: "hello", attachments: [] });

  expect(deps.refreshRecentRun).not.toHaveBeenCalled();
  const assistants = foldMessages(setMessages).filter(message => message.role === "assistant");
  expect(assistants.map(message => message.content)).not.toContain("durable answer");
});

it("does not refresh the sidebar when a failed send was superseded", async () => {
  // concurrency: the invoke throws after a thread switch. The failure bubble
  // belongs to the abandoned conversation, and refreshing THIS thread's
  // sidebar is the new view's business - but the failed run row is still
  // written, because the row is durable state the returned-to thread reads.
  let current = true;
  vi.mocked(getRun).mockImplementation(async () => {
    current = false;
    return storedRun();
  });
  vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("transport down"));
  const setMessages = vi.fn();
  const deps = { ...makeDeps(setMessages), isCurrentSend: () => current };

  await runSendPipeline(deps, { content: "hello", attachments: [] });

  expect(deps.refreshRecentRun).not.toHaveBeenCalled();
  expect(updateRunStatus).toHaveBeenCalledWith(expect.objectContaining({ runId: "run-1", status: "failed" }));
  const assistants = foldMessages(setMessages).filter(m => m.role === "assistant");
  expect(assistants.some(m => m.runError !== undefined)).toBe(false);
});

describe("send acceptance timing", () => {
  it("acknowledges before the response finishes and keeps the run active", async () => {
    vi.mocked(createRun).mockResolvedValue(storedRun());
    vi.mocked(getRun).mockResolvedValue(storedRun());
    let finish!: (value: Awaited<ReturnType<typeof sendPromptToFutureAgent>>) => void;
    vi.mocked(sendPromptToFutureAgent).mockImplementation(({ onAccepted }) => {
      onAccepted?.();
      return new Promise((resolve) => {
        finish = resolve;
      });
    });
    const onAccepted = vi.fn();
    let finished = false;
    const send = runSendPipeline({ ...makeDeps(vi.fn()), onAccepted }, { content: "hello", attachments: [] })
      .then(() => { finished = true; });
    await vi.waitFor(() => expect(onAccepted).toHaveBeenCalledTimes(1));
    expect(finished).toBe(false);
    finish({ content: "answer", complete: true, sessionId: "session-1" });
    await send;
    expect(onAccepted).toHaveBeenCalledTimes(1);
  });

  it("rejects delivery without acknowledging when the Agent rejects the prompt", async () => {
    vi.mocked(createRun).mockResolvedValue(storedRun());
    vi.mocked(getRun).mockResolvedValue(storedRun());
    vi.mocked(sendPromptToFutureAgent).mockRejectedValue(new Error("Agent unavailable"));
    const onAccepted = vi.fn();
    await expect(runSendPipeline({ ...makeDeps(vi.fn()), onAccepted }, { content: "hello", attachments: [] })).rejects.toThrow("Agent unavailable");
    expect(onAccepted).not.toHaveBeenCalled();
  });
});
