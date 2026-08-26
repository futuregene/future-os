import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import { uploadAttachments } from "../files";
import { loadPendingContinuation, savePendingContinuation } from "../pendingContinuationStorage";
import { loadPendingPrompt, savePendingPrompt } from "../pendingPromptStorage";
import { emptyTimeline } from "../timeline";
import type { ConnectionPhase, MobileAttachment, RemoteCredentials } from "../types";
import type { SyncEngine } from "../syncEngine";
import { usePromptOutbox } from "../usePromptOutbox";

const mockData = new Map<string, string>();

jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(async (key: string) => mockData.get(key) ?? null),
    setItem: jest.fn(async (key: string, value: string) => {
      mockData.set(key, value);
    }),
    removeItem: jest.fn(async (key: string) => {
      mockData.delete(key);
    }),
  },
}));

jest.mock("../files", () => ({
  uploadAttachments: jest.fn(),
}));

const mockedUploadAttachments = uploadAttachments as jest.MockedFunction<typeof uploadAttachments>;

const credentials: RemoteCredentials = {
  pairId: "pair",
  deviceId: "device",
  seed: "seed",
  userJwt: "jwt",
  refreshToken: "refresh",
  natsWsUrl: "wss://example.test",
  tokenUrl: "https://example.test/token",
  expectedDesktopId: "desktop",
  expectedDesktopPublicKey: "public-key",
};

const attachment: MobileAttachment = {
  localUri: "file:///tmp/a.jpg",
  name: "a.jpg",
  mimeType: "image/jpeg",
  kind: "image",
  originalSize: 10,
  transferSize: 10,
};

function ack(sessionId = "session-1"): { sessionId: string; threadId: string; runId: string } {
  return { sessionId, threadId: `thread-${sessionId}`, runId: `run-${sessionId}` };
}

function deferred<T = unknown>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeEngine(): SyncEngine {
  return {
    mutate: jest.fn((_sessionId: string, apply: (timeline: ReturnType<typeof emptyTimeline>) => unknown) => {
      apply(emptyTimeline());
    }),
  } as unknown as SyncEngine;
}

async function flush(times = 20): Promise<void> {
  await act(async () => {
    for (let i = 0; i < times; i += 1) {
      await Promise.resolve();
    }
  });
}

describe("usePromptOutbox recovery", () => {
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
  });

  async function mountOutbox() {
    const requestRetry = jest.fn(async (request: { promptId?: string }) => ({
      data:
        request.promptId === "prompt-1"
          ? { sessionId: "session-1", threadId: "thread-1", runId: "run-1" }
          : { sessionId: "session-2", threadId: "thread-2", runId: "run-2" },
    }));
    const reconcileSession = jest.fn();
    const refreshSessions = jest.fn(async () => {});
    const clientRef = {
      current: { requestRetry } as unknown as RemoteClient,
    };
    const credentialsRef = { current: credentials };
    const selectedRef = { current: "session-1" };
    const streamingRef = { current: {} };
    const conversationEpochRef = { current: 1 };
    const syncEngineRef = { current: null };
    const setSelectedSessionId = jest.fn();
    const setDraft = jest.fn();
    const setDraftMode = jest.fn();
    const setDraftWorkspaceId = jest.fn();
    const recordError = jest.fn();
    let renderer: ReactTestRenderer | null = null;

    function Harness() {
      usePromptOutbox({
        clientRef,
        credentialsRef,
        selectedRef,
        streamingRef,
        conversationEpochRef,
        syncEngineRef,
        phase: "ready",
        draft: true,
        draftMode: "chat",
        draftWorkspaceId: "",
        modelId: "provider/model",
        thinkingLevel: "medium",
        fileTransferSupported: true,
        promptReceiptSupported: true,
        setSelectedSessionId,
        setDraft,
        setDraftMode,
        setDraftWorkspaceId,
        refreshSessions,
        reconcileSession,
        recordError,
      });
      return null;
    }

    await act(async () => {
      renderer = create(React.createElement(Harness));
    });
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 50));
    });
    return { requestRetry, reconcileSession, refreshSessions, renderer: renderer! };
  }

  it("reconciles a prompt while an existing conversation and draft are active", async () => {
    await savePendingPrompt({
      version: 1,
      commandId: "prompt-1",
      draftKey: "session-1",
      sessionId: "session-1",
      text: "hello",
      attachments: [],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const { requestRetry, reconcileSession, refreshSessions, renderer } = await mountOutbox();

    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: "prompt-1" },
      "list",
    );
    expect(reconcileSession).toHaveBeenCalledWith("session-1", "reconnect");
    expect(refreshSessions).toHaveBeenCalledTimes(1);

    await act(async () => renderer.unmount());
  });

  it("automatically reconciles a durable continuation after reconnect", async () => {
    await savePendingContinuation({
      version: 1,
      commandId: "continue-1",
      sessionId: "session-2",
      sourceRunId: "failed-run",
      createdAt: 2,
    });
    await expect(loadPendingContinuation()).resolves.toMatchObject({ commandId: "continue-1" });
    const { requestRetry, reconcileSession, refreshSessions, renderer } = await mountOutbox();

    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: "continue-1" },
      "list",
    );
    expect(reconcileSession).toHaveBeenCalledWith("session-2", "reconnect", "run-2");
    expect(refreshSessions).toHaveBeenCalledTimes(1);

    await act(async () => renderer.unmount());
  });
});

describe("usePromptOutbox sendMessage", () => {
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
    mockedUploadAttachments.mockResolvedValue([]);
  });

  interface MountOpts {
    phase?: ConnectionPhase;
    draft?: boolean;
    draftMode?: "chat" | "workspace";
    draftWorkspaceId?: string;
    fileTransferSupported?: boolean;
    promptReceiptSupported?: boolean;
    engine?: SyncEngine | null;
    requestRetry?: jest.Mock;
  }

  async function mountSend(opts: MountOpts = {}) {
    const requestRetry = opts.requestRetry ?? jest.fn(async () => ({ data: ack() }));
    const client = { requestRetry } as unknown as RemoteClient;
    const clientRef = { current: client as RemoteClient | null };
    const credentialsRef = { current: credentials };
    const selectedRef = { current: "session-1" };
    const streamingRef = { current: {} as Record<string, boolean> };
    const conversationEpochRef = { current: 1 };
    const syncEngineRef = { current: (opts.engine ?? null) as SyncEngine | null };
    const setSelectedSessionId = jest.fn();
    const setDraft = jest.fn();
    const setDraftMode = jest.fn();
    const setDraftWorkspaceId = jest.fn();
    const refreshSessions = jest.fn(async () => {});
    const reconcileSession = jest.fn();
    const recordError = jest.fn();
    let result!: ReturnType<typeof usePromptOutbox>;
    let renderer!: ReactTestRenderer;

    function Harness() {
      result = usePromptOutbox({
        clientRef,
        credentialsRef,
        selectedRef,
        streamingRef,
        conversationEpochRef,
        syncEngineRef,
        phase: opts.phase ?? "connecting",
        draft: opts.draft ?? true,
        draftMode: opts.draftMode ?? "chat",
        draftWorkspaceId: opts.draftWorkspaceId ?? "",
        modelId: "provider/model",
        thinkingLevel: "medium",
        fileTransferSupported: opts.fileTransferSupported ?? true,
        promptReceiptSupported: opts.promptReceiptSupported ?? true,
        setSelectedSessionId,
        setDraft,
        setDraftMode,
        setDraftWorkspaceId,
        refreshSessions,
        reconcileSession,
        recordError,
      });
      return null;
    }

    await act(async () => {
      renderer = create(React.createElement(Harness));
    });
    await flush();

    return {
      result,
      renderer,
      requestRetry,
      clientRef,
      selectedRef,
      streamingRef,
      conversationEpochRef,
      syncEngineRef,
      setSelectedSessionId,
      setDraft,
      setDraftMode,
      setDraftWorkspaceId,
      refreshSessions,
      reconcileSession,
      recordError,
    };
  }

  it("ignores an empty send with no attachments", async () => {
    const h = await mountSend();
    await act(async () => {
      await h.result.sendMessage("", []);
    });
    expect(h.requestRetry).not.toHaveBeenCalled();
  });

  it("throws not_connected when the client is absent", async () => {
    const h = await mountSend();
    h.clientRef.current = null;
    await expect(h.result.sendMessage("hi")).rejects.toThrow("not_connected");
  });

  it("rejects a concurrent send while one is already in flight", async () => {
    await savePendingPrompt({
      version: 1,
      commandId: "prompt-busy",
      draftKey: "session-1",
      sessionId: "session-1",
      text: "hello",
      attachments: [],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const receipt = deferred();
    const requestRetry = jest.fn(async (request: { type: string }) => {
      if (request.type === "get_prompt_receipt") return receipt.promise;
      return { data: ack() };
    });
    const h = await mountSend({ phase: "ready", requestRetry });
    await flush();
    await expect(h.result.sendMessage("hi")).rejects.toThrow("send_busy");
    await act(async () => {
      receipt.resolve({ data: ack() });
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => h.renderer.unmount());
  });

  it("rejects attachments when the client does not support file transfer", async () => {
    const h = await mountSend({ fileTransferSupported: false });
    await expect(h.result.sendMessage("hi", [attachment])).rejects.toThrow(
      "attachment_unsupported_desktop",
    );
  });

  it("rejects a prompt larger than the wire budget", async () => {
    const h = await mountSend();
    await expect(h.result.sendMessage("x".repeat(512 * 1024 + 1))).rejects.toThrow(
      "prompt_too_large",
    );
  });

  it("rejects a send while the target session is streaming", async () => {
    const h = await mountSend();
    h.streamingRef.current["session-1"] = true;
    await expect(h.result.sendMessage("hi")).rejects.toThrow("send_streaming");
  });

  it("delivers a prompt with attachments and mutates the target timeline", async () => {
    mockedUploadAttachments.mockResolvedValue([{ ...attachment, uploadId: "u1" }]);
    const h = await mountSend({ engine: fakeEngine() });
    await act(async () => {
      await h.result.sendMessage("hello", [attachment]);
    });
    expect(mockedUploadAttachments).toHaveBeenCalled();
    expect(h.requestRetry).toHaveBeenCalledWith(
      expect.objectContaining({ type: "prompt", attachments: [{ uploadId: "u1" }] }),
      "session-1",
    );
    const engine = h.syncEngineRef.current as unknown as { mutate: jest.Mock };
    expect(engine.mutate).toHaveBeenCalled();
  });

  it("acknowledges a matching pending prompt via its receipt", async () => {
    await savePendingPrompt({
      version: 1,
      commandId: "prompt-match",
      draftKey: "session-1",
      sessionId: "session-1",
      text: "hello",
      attachments: [],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const requestRetry = jest.fn(async () => ({ data: ack() }));
    const h = await mountSend({ engine: fakeEngine(), requestRetry });
    await act(async () => {
      await h.result.sendMessage("hello");
    });
    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: "prompt-match" },
      "list",
    );
    await expect(loadPendingPrompt()).resolves.toBeNull();
    const engine = h.syncEngineRef.current as unknown as { mutate: jest.Mock };
    expect(engine.mutate).toHaveBeenCalled();
  });

  it("replaces a stale pending prompt before delivering a new one", async () => {
    await savePendingPrompt({
      version: 1,
      commandId: "prompt-old",
      draftKey: "session-1",
      sessionId: "session-1",
      text: "other",
      attachments: [],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const requestRetry = jest.fn(async () => ({ data: ack() }));
    const h = await mountSend({ engine: fakeEngine(), requestRetry });
    await act(async () => {
      await h.result.sendMessage("hello");
    });
    // The stale record is receipt-checked and cleared before the new prompt.
    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: "prompt-old" },
      "list",
    );
    await expect(loadPendingPrompt()).resolves.toBeNull();
  });

  it("switches to the new session when the ack returns a different id", async () => {
    const requestRetry = jest.fn(async () => ({ data: ack("session-2") }));
    const h = await mountSend({ engine: fakeEngine(), requestRetry });
    await act(async () => {
      await h.result.sendMessage("hello");
    });
    expect(h.setSelectedSessionId).toHaveBeenCalledWith("session-2");
    expect(h.setDraft).toHaveBeenCalledWith(false);
    expect(h.setDraftMode).toHaveBeenCalledWith("chat");
    expect(h.setDraftWorkspaceId).toHaveBeenCalledWith("");
    expect(h.refreshSessions).toHaveBeenCalled();
    const engine = h.syncEngineRef.current as unknown as { mutate: jest.Mock };
    expect(engine.mutate).toHaveBeenCalledWith("session-2", expect.any(Function));
    expect(engine.mutate).toHaveBeenCalledWith("session-1", expect.any(Function));
  });

  it("clears the pending prompt and rethrows a non-transient send failure", async () => {
    await savePendingPrompt({
      version: 1,
      commandId: "prompt-fail",
      draftKey: "session-1",
      sessionId: "session-1",
      text: "hello",
      attachments: [],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const requestRetry = jest.fn(async (request: { type: string }) => {
      if (request.type === "get_prompt_receipt") return { data: null };
      throw new Error("boom");
    });
    const h = await mountSend({ requestRetry });
    await act(async () => {
      await expect(h.result.sendMessage("hello")).rejects.toThrow("boom");
    });
    await expect(loadPendingPrompt()).resolves.toBeNull();
  });
});

describe("usePromptOutbox continueRun", () => {
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
    mockedUploadAttachments.mockResolvedValue([]);
  });

  async function mountContinue(requestRetry: jest.Mock) {
    const client = { requestRetry } as unknown as RemoteClient;
    const clientRef = { current: client as RemoteClient | null };
    const credentialsRef = { current: credentials };
    const selectedRef = { current: "session-1" };
    const streamingRef = { current: {} as Record<string, boolean> };
    const conversationEpochRef = { current: 1 };
    const syncEngineRef = { current: null as SyncEngine | null };
    const setSelectedSessionId = jest.fn();
    const setDraft = jest.fn();
    const setDraftMode = jest.fn();
    const setDraftWorkspaceId = jest.fn();
    const refreshSessions = jest.fn(async () => {});
    const reconcileSession = jest.fn();
    const recordError = jest.fn();
    let result!: ReturnType<typeof usePromptOutbox>;
    let renderer!: ReactTestRenderer;

    function Harness() {
      result = usePromptOutbox({
        clientRef,
        credentialsRef,
        selectedRef,
        streamingRef,
        conversationEpochRef,
        syncEngineRef,
        phase: "connecting",
        draft: true,
        draftMode: "chat",
        draftWorkspaceId: "",
        modelId: "provider/model",
        thinkingLevel: "medium",
        fileTransferSupported: true,
        promptReceiptSupported: true,
        setSelectedSessionId,
        setDraft,
        setDraftMode,
        setDraftWorkspaceId,
        refreshSessions,
        reconcileSession,
        recordError,
      });
      return null;
    }

    await act(async () => {
      renderer = create(React.createElement(Harness));
    });
    await flush();

    return { result, renderer, requestRetry, recordError, reconcileSession, refreshSessions };
  }

  it("delivers a fresh continuation and clears it", async () => {
    const requestRetry = jest.fn(async () => ({ data: ack() }));
    const h = await mountContinue(requestRetry);
    await act(async () => {
      await h.result.continueRun("session-1", "run-1");
    });
    expect(requestRetry).toHaveBeenCalledWith(
      { id: expect.stringMatching(/^continue_/), type: "continue_run", sessionId: "session-1", runId: "run-1" },
      "session-1",
    );
    await expect(loadPendingContinuation()).resolves.toBeNull();
  });

  it("returns the in-flight promise for a matching retry", async () => {
    const d = deferred();
    const requestRetry = jest.fn(async (request: { type: string }) => {
      if (request.type === "continue_run") return d.promise;
      return { data: ack() };
    });
    const h = await mountContinue(requestRetry);
    const p1 = h.result.continueRun("session-1", "run-1");
    const p2 = h.result.continueRun("session-1", "run-1");
    await flush();
    expect(requestRetry).toHaveBeenCalledTimes(1);
    d.resolve({ data: ack() });
    await p1;
    await p2;
  });

  it("waits for a different in-flight continuation before starting its own", async () => {
    const d = deferred();
    const requestRetry = jest.fn(async (request: { type: string }) => {
      if (request.type === "continue_run") return d.promise;
      return { data: ack() };
    });
    const h = await mountContinue(requestRetry);
    const p1 = h.result.continueRun("session-1", "run-1");
    const p2 = h.result.continueRun("session-2", "run-2");
    await flush();
    expect(requestRetry).toHaveBeenCalledTimes(1);
    d.resolve({ data: ack() });
    await p1;
    await p2;
    expect(requestRetry).toHaveBeenCalledTimes(2);
  });

  it("clears a stale continuation that targets a different run", async () => {
    await savePendingContinuation({
      version: 1,
      commandId: "continue-old",
      sessionId: "other-session",
      sourceRunId: "other-run",
      createdAt: 2,
    });
    const requestRetry = jest.fn(async () => ({ data: ack() }));
    const h = await mountContinue(requestRetry);
    await act(async () => {
      await h.result.continueRun("session-1", "run-1");
    });
    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: "continue-old" },
      "list",
    );
    await expect(loadPendingContinuation()).resolves.toBeNull();
  });

  it("clears the continuation and rethrows a non-transient failure", async () => {
    const requestRetry = jest.fn(async () => {
      throw new Error("boom");
    });
    const h = await mountContinue(requestRetry);
    await expect(h.result.continueRun("session-1", "run-1")).rejects.toThrow("boom");
    await expect(loadPendingContinuation()).resolves.toBeNull();
  });
});

describe("usePromptOutbox recovery error handling", () => {
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
    mockedUploadAttachments.mockResolvedValue([]);
  });

  async function mountRecovery(requestRetry: jest.Mock) {
    const client = { requestRetry } as unknown as RemoteClient;
    const clientRef = { current: client as RemoteClient | null };
    const credentialsRef = { current: credentials };
    const selectedRef = { current: "session-1" };
    const streamingRef = { current: {} as Record<string, boolean> };
    const conversationEpochRef = { current: 1 };
    const syncEngineRef = { current: null as SyncEngine | null };
    const setSelectedSessionId = jest.fn();
    const setDraft = jest.fn();
    const setDraftMode = jest.fn();
    const setDraftWorkspaceId = jest.fn();
    const refreshSessions = jest.fn(async () => {});
    const reconcileSession = jest.fn();
    const recordError = jest.fn();
    let renderer!: ReactTestRenderer;

    function Harness() {
      usePromptOutbox({
        clientRef,
        credentialsRef,
        selectedRef,
        streamingRef,
        conversationEpochRef,
        syncEngineRef,
        phase: "ready",
        draft: true,
        draftMode: "chat",
        draftWorkspaceId: "",
        modelId: "provider/model",
        thinkingLevel: "medium",
        fileTransferSupported: true,
        promptReceiptSupported: true,
        setSelectedSessionId,
        setDraft,
        setDraftMode,
        setDraftWorkspaceId,
        refreshSessions,
        reconcileSession,
        recordError,
      });
      return null;
    }

    await act(async () => {
      renderer = create(React.createElement(Harness));
    });
    await flush();

    return { renderer, requestRetry, recordError, reconcileSession, refreshSessions };
  }

  it("clears and records a non-transient prompt recovery failure", async () => {
    await savePendingPrompt({
      version: 1,
      commandId: "prompt-err",
      draftKey: "session-1",
      sessionId: "session-1",
      text: "hello",
      attachments: [],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const requestRetry = jest.fn(async (request: { type: string }) => {
      if (request.type === "get_prompt_receipt") return { data: null };
      throw new Error("boom");
    });
    const h = await mountRecovery(requestRetry);
    expect(h.recordError).toHaveBeenCalledWith(expect.objectContaining({ message: "boom" }));
    await expect(loadPendingPrompt()).resolves.toBeNull();
    await act(async () => h.renderer.unmount());
  });

  it("clears and records a non-transient continuation recovery failure", async () => {
    await savePendingContinuation({
      version: 1,
      commandId: "continue-err",
      sessionId: "session-2",
      sourceRunId: "failed-run",
      createdAt: 2,
    });
    const requestRetry = jest.fn(async (request: { type: string }) => {
      if (request.type === "get_prompt_receipt") return { data: null };
      throw new Error("boom");
    });
    const h = await mountRecovery(requestRetry);
    expect(h.recordError).toHaveBeenCalledWith(expect.objectContaining({ message: "boom" }));
    await expect(loadPendingContinuation()).resolves.toBeNull();
    await act(async () => h.renderer.unmount());
  });
});
