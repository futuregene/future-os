import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import AsyncStorage from "@react-native-async-storage/async-storage";
import type { RemoteClient } from "../client";
import { uploadAttachments } from "../files";
import { loadPendingContinuation, savePendingContinuation } from "../pendingContinuationStorage";
import { loadPendingPrompt, savePendingPrompt } from "../pendingPromptStorage";
import { emptyTimeline } from "../timeline";
import { modelReference, type ConnectionPhase, type MobileAttachment, type PromptAck, type RemoteCredentials } from "../types";
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
    mutate: jest.fn(
      (_sessionId: string, apply: (timeline: ReturnType<typeof emptyTimeline>) => unknown) => {
        apply(emptyTimeline());
      },
    ),
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

  async function mountOutbox(opts: { client?: RemoteClient | null; requestRetry?: jest.Mock } = {}) {
    const requestRetry = opts.requestRetry ?? jest.fn(async (request: { promptId?: string }) => ({
      data:
        request.promptId === "prompt-1"
          ? { sessionId: "session-1", threadId: "thread-1", runId: "run-1" }
          : { sessionId: "session-2", threadId: "thread-2", runId: "run-2" },
    }));
    const reconcileSession = jest.fn();
    const refreshSessions = jest.fn(async () => {});
    const clientRef = {
      current: (opts.client === undefined ? { requestRetry } : opts.client) as RemoteClient | null,
    };
    const credentialsRef = { current: credentials as RemoteCredentials | null };
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
        compactingRef: { current: {} },
        conversationEpochRef,
        syncEngineRef,
        phase: "ready",
        businessReady: true,
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
      renderer = create(createElement(Harness));
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    return { requestRetry, reconcileSession, refreshSessions, renderer: renderer!, clientRef, credentialsRef, recordError };
  }

  it("reconciles a prompt while an existing conversation and draft are active", async () => {
    await savePendingPrompt({
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
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

  it.each([
    { pairId: "other-pair", expectedDesktopId: credentials.expectedDesktopId },
    { pairId: credentials.pairId, expectedDesktopId: "other-desktop" },
  ])("never queries or uploads a prompt from another pairing: %j", async (identity) => {
    await savePendingPrompt({
      version: 2,
      ...identity,
      commandId: "foreign-prompt",
      draftKey: "draft:new",
      sessionId: "",
      text: "private draft for another desktop",
      attachments: [attachment],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const { requestRetry, renderer } = await mountOutbox();
    expect(requestRetry).not.toHaveBeenCalled();
    expect(mockedUploadAttachments).not.toHaveBeenCalled();
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toBeNull();
    await act(async () => renderer.unmount());
  });

  it("automatically reconciles a durable continuation after reconnect", async () => {
    await savePendingContinuation({
      version: 2,
      commandId: "continue-1",
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      sessionId: "session-2",
      sourceRunId: "failed-run",
      createdAt: 2,
    });
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toMatchObject({ commandId: "continue-1" });
    const { requestRetry, reconcileSession, refreshSessions, renderer } = await mountOutbox();

    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: "continue-1" },
      "list",
    );
    expect(reconcileSession).toHaveBeenCalledWith("session-2", "reconnect", "run-2");
    expect(refreshSessions).toHaveBeenCalledTimes(1);

    await act(async () => renderer.unmount());
  });

  it("does nothing while the pairing has no client or credentials", async () => {
    await savePendingPrompt({
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      commandId: "prompt-offline",
      draftKey: "session-1",
      sessionId: "session-1",
      text: "queued while offline",
      attachments: [],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    await savePendingContinuation({
      version: 2,
      commandId: "continue-offline",
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      sessionId: "session-1",
      sourceRunId: "failed-run",
      createdAt: 2,
    });
    const { requestRetry, renderer } = await mountOutbox({ client: null });
    // Both recovery passes must bail out before probing: a `ready` phase with a
    // torn-down pairing must not ask an absent client for receipts, and the
    // durable records have to survive for the next real reconnect.
    expect(requestRetry).not.toHaveBeenCalled();
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toMatchObject({
      commandId: "prompt-offline",
    });
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toMatchObject({
      commandId: "continue-offline",
    });
    await act(async () => renderer.unmount());
  });

  it("a continuation that fails after the pairing moved on is left for its owner", async () => {
    await savePendingContinuation({
      version: 2,
      commandId: "continue-moved",
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      sessionId: "session-1",
      sourceRunId: "failed-run",
      createdAt: 2,
    });
    const probe = deferred<never>();
    const { requestRetry, clientRef, credentialsRef, recordError, renderer } = await mountOutbox({
      requestRetry: jest.fn(() => probe.promise),
    });
    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: "continue-moved" },
      "list",
    );
    // The sweep is superseded while its receipt probe is in flight.
    credentialsRef.current = { ...credentials, pairId: "other-pair" };
    clientRef.current = null;
    await act(async () => { probe.reject(new Error("socket closed")); });
    await flush();
    // The failure belongs to the old pairing: it must not be reported as a
    // user-visible error, and the record must not be discarded under the new
    // owner's feet.
    expect(recordError).not.toHaveBeenCalled();
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toMatchObject({
      commandId: "continue-moved",
    });
    await act(async () => renderer.unmount());
  });
});

describe("usePromptOutbox readiness edges", () => {
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
    mockedUploadAttachments.mockResolvedValue([]);
  });

  interface EdgeOpts {
    requestRetry: jest.Mock;
    client?: RemoteClient | null;
  }

  async function mountEdges(opts: EdgeOpts) {
    // The outbox reaches only `requestRetry`; the rest of the client interface
    // is unused here, so the double stops at that one member — the same boundary
    // `mountSend` draws with `{ requestRetry } as unknown as RemoteClient`.
    const stub = { requestRetry: opts.requestRetry } as unknown as RemoteClient;
    const clientRef = { current: (opts.client === undefined ? stub : opts.client) as RemoteClient | null };
    const credentialsRef = { current: credentials };
    const recordError = jest.fn();
    // Stable identities: an inline `jest.fn()` here would change the recovery
    // callbacks on every render and re-run the readiness effect mid-probe.
    const setSelectedSessionId = jest.fn();
    const setDraft = jest.fn();
    const setDraftMode = jest.fn();
    const setDraftWorkspaceId = jest.fn();
    const refreshSessions = jest.fn(async () => {});
    const reconcileSession = jest.fn();
    const selectedRef = { current: "session-1" };
    const streamingRef = { current: {} };
    const compactingRef = { current: {} };
    const conversationEpochRef = { current: 1 };
    const syncEngineRef = { current: null };
    let result!: ReturnType<typeof usePromptOutbox>;
    let renderer!: ReactTestRenderer;

    function Harness({ ready }: { ready: boolean }) {
      result = usePromptOutbox({
        clientRef,
        credentialsRef,
        selectedRef,
        streamingRef,
        compactingRef,
        conversationEpochRef,
        syncEngineRef,
        phase: "ready",
        businessReady: ready,
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

    await act(async () => { renderer = create(createElement(Harness, { ready: false })); });
    const setReady = async (ready: boolean) => {
      await act(async () => { renderer.update(createElement(Harness, { ready })); });
    };
    return { get result() { return result; }, renderer, clientRef, credentialsRef, recordError, setReady };
  }

  const savedPrompt = () => savePendingPrompt({
    version: 2,
    pairId: credentials.pairId,
    expectedDesktopId: credentials.expectedDesktopId,
    commandId: "prompt-pending",
    draftKey: "session-1",
    sessionId: "session-1",
    text: "queued",
    attachments: [],
    modelId: "provider/model",
    thinkingLevel: "medium",
    mode: "chat",
    workspaceId: "",
    createdAt: 1,
  });

  it("holds recovery back while a send owns the lane", async () => {
    await savedPrompt();
    const send = deferred<never>();
    const requestRetry = jest.fn(() => send.promise);
    const h = await mountEdges({ requestRetry });
    let sending!: Promise<void>;
    await act(async () => { sending = h.result.sendMessage("live message"); });
    // The send itself probes the queued slot once for a stale receipt.
    expect(requestRetry).toHaveBeenCalledTimes(1);
    // A readiness edge arrives while the user's prompt is still in flight.
    await h.setReady(true);
    await flush();
    // Recovery must not race the live send for the same slot: it must not add a
    // second receipt probe while that send is still unresolved.
    expect(requestRetry).toHaveBeenCalledTimes(1);
    await act(async () => { send.resolve({ data: ack() } as never); await sending; });
    await flush();
    expect(h.result.sending).toBe(false);
    await act(async () => h.renderer.unmount());
  });

  it("shares one recovery pass between overlapping readiness edges", async () => {
    await savedPrompt();
    const probe = deferred<{ data: unknown }>();
    const requestRetry = jest.fn(() => probe.promise);
    const h = await mountEdges({ requestRetry });
    await h.setReady(true);
    await flush();
    expect(requestRetry).toHaveBeenCalledTimes(1);
    // A second edge while the first recovery is still probing must join it
    // instead of asking the desktop for the same receipt twice.
    await h.setReady(false);
    await h.setReady(true);
    await flush();
    expect(requestRetry).toHaveBeenCalledTimes(1);
    await act(async () => { probe.resolve({ data: ack() }); });
    await flush();
    await act(async () => h.renderer.unmount());
  });

  it("a torn-down readiness edge stops before sweeping on its own", async () => {
    await savedPrompt();
    await savePendingContinuation({
      version: 2,
      commandId: "continue-after-teardown",
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      sessionId: "session-1",
      sourceRunId: "failed-run",
      createdAt: 2,
    });
    const probe = deferred<{ data: unknown }>();
    const requestRetry = jest.fn((request: { type: string }) =>
      request.type === "get_prompt_receipt" ? probe.promise : Promise.resolve({ data: ack("session-1") }),
    );
    const h = await mountEdges({ requestRetry });
    await h.setReady(true);
    await flush();
    expect(requestRetry).toHaveBeenCalledTimes(1);
    // A second edge mounts while the first pass is still probing, and that
    // second edge is torn down before the owner settles.
    await h.setReady(false);
    await h.setReady(true);
    await h.setReady(false);
    await act(async () => { probe.resolve({ data: ack() }); });
    await flush();
    // The continuation is left for the owner: a cancelled pass must not deliver
    // a stored continuation for a screen that is gone.
    expect(h.recordError).not.toHaveBeenCalled();
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toMatchObject({
      commandId: "continue-after-teardown",
    });
    await act(async () => h.renderer.unmount());
  });

  it("waits for an in-flight continuation before sweeping again", async () => {
    const sendContinue = deferred<{ data: unknown }>();
    const requestRetry = jest.fn((request: { type: string }) =>
      request.type === "continue_run" ? sendContinue.promise : Promise.resolve({ data: ack("session-2") }),
    );
    const h = await mountEdges({ requestRetry });
    await h.setReady(true);
    await flush();
    let retrying!: Promise<void>;
    await act(async () => {
      retrying = h.result.continueRun("session-2", "failed-run").catch(() => undefined);
    });
    expect(requestRetry).toHaveBeenCalledWith(
      expect.objectContaining({ type: "continue_run" }),
      "session-2",
    );
    // A readiness edge arrives while the user's retry is in flight: the new
    // pass joins that owner instead of delivering the continuation twice.
    await h.setReady(false);
    await h.setReady(true);
    await flush();
    expect(requestRetry).toHaveBeenCalledTimes(1);
    // The retry then fails while the pass is still waiting on it. The failure
    // belongs to the caller that started it, so the readiness pass must swallow
    // it rather than report it to the user a second time.
    await act(async () => {
      sendContinue.reject(new Error("not_connected"));
      await retrying;
    });
    await flush();
    expect(h.recordError).not.toHaveBeenCalled();
    await act(async () => h.renderer.unmount());
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
    modelId?: string;
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
    const compactingRef = { current: {} as Record<string, boolean> };
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
        compactingRef,
        conversationEpochRef,
        syncEngineRef,
        phase: opts.phase ?? "connecting",
        businessReady: true,
        draft: opts.draft ?? true,
        draftMode: opts.draftMode ?? "chat",
        draftWorkspaceId: opts.draftWorkspaceId ?? "",
        modelId: opts.modelId ?? "provider/model",
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
      renderer = create(createElement(Harness));
    });
    await flush();

    return {
      result,
      renderer,
      requestRetry,
      clientRef,
      credentialsRef,
      selectedRef,
      streamingRef,
      compactingRef,
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

  it.each(["deepseek", "ambient"])("preserves the selected %s provider on the first prompt", async provider => {
    const modelId = modelReference({ provider, id: "deepseek/deepseek-v4-flash" });
    const h = await mountSend({ modelId });
    await act(async () => h.result.sendMessage("hello"));
    expect(h.requestRetry).toHaveBeenCalledWith(
      expect.objectContaining({ type: "prompt", modelId: `${provider}/deepseek/deepseek-v4-flash`, providerId: provider }),
      "session-1",
    );
  });

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
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
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

  it("acquires the send lane before the first storage await", async () => {
    const promptResponse = deferred<{ data: ReturnType<typeof ack> }>();
    const requestRetry = jest.fn((request: { type: string }) => {
      if (request.type === "prompt") return promptResponse.promise;
      return Promise.resolve({ data: null });
    });
    const h = await mountSend({ requestRetry });

    let first!: Promise<void>;
    await act(async () => {
      first = h.result.sendMessage("first");
      await expect(h.result.sendMessage("second")).rejects.toThrow("send_busy");
    });
    await flush();
    expect(requestRetry.mock.calls.filter(([request]) => request.type === "prompt")).toHaveLength(
      1,
    );

    promptResponse.resolve({ data: ack() });
    await act(async () => {
      await first;
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

  it("rejects prompts and continuations during compaction before creating an outbox entry", async () => {
    const h = await mountSend();
    h.compactingRef.current["session-1"] = true;
    await expect(h.result.sendMessage("keep this draft")).rejects.toThrow("send_compacting");
    await expect(h.result.continueRun("session-1", "run-1")).rejects.toThrow("send_compacting");
    expect(mockedUploadAttachments).not.toHaveBeenCalled();
  });

  it("rejects a send while the target session is streaming", async () => {
    const h = await mountSend();
    h.streamingRef.current["session-1"] = true;
    await expect(h.result.sendMessage("hi")).rejects.toThrow("send_streaming");
  });

  it("stops delivery when pairing changes during an attachment upload", async () => {
    const upload = deferred<MobileAttachment[]>();
    mockedUploadAttachments.mockReturnValue(upload.promise);
    const h = await mountSend();
    let sending!: Promise<void>;
    act(() => {
      sending = h.result.sendMessage("private", [attachment]);
    });
    const rejected = expect(sending).rejects.toThrow("pairing_changed");
    await flush();
    h.credentialsRef.current = { ...credentials, pairId: "new-pair" };
    upload.resolve([]);
    await act(async () => {
      await rejected;
    });
    expect(h.requestRetry).not.toHaveBeenCalled();
  });

  it("does not apply an old desktop's late prompt acknowledgement to the new timeline", async () => {
    const response = deferred<{ data: ReturnType<typeof ack> }>();
    const requestRetry = jest.fn(() => response.promise);
    const engine = fakeEngine();
    const h = await mountSend({ requestRetry, engine });
    let sending!: Promise<void>;
    act(() => { sending = h.result.sendMessage("for the first desktop"); });
    const rejected = expect(sending).rejects.toThrow("pairing_changed");
    await flush();
    h.clientRef.current = null;
    h.credentialsRef.current = { ...credentials, pairId: "second-pair", expectedDesktopId: "second-desktop" };
    response.resolve({ data: ack() });
    await act(async () => { await rejected; });
    expect(engine.mutate).not.toHaveBeenCalled();
    expect(h.setSelectedSessionId).not.toHaveBeenCalled();
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
    const prompt = h.requestRetry.mock.calls.find(([command]) => command.type === "prompt")![0];
    expect(prompt.images).toBeUndefined();
    expect(prompt.attachments).toEqual([{ uploadId: "u1" }]);
    const engine = h.syncEngineRef.current as unknown as { mutate: jest.Mock };
    expect(engine.mutate).toHaveBeenCalled();
  });

  it("acknowledges a matching pending prompt via its receipt", async () => {
    await savePendingPrompt({
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      commandId: "prompt-match",
      draftKey: "desktop:session-1",
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
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toBeNull();
    const engine = h.syncEngineRef.current as unknown as { mutate: jest.Mock };
    expect(engine.mutate).toHaveBeenCalled();
  });

  it("keeps an acknowledged command id when local cleanup briefly fails", async () => {
    const removeItem = AsyncStorage.removeItem as jest.MockedFunction<
      typeof AsyncStorage.removeItem
    >;
    removeItem.mockRejectedValueOnce(new Error("storage unavailable"));
    const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
    const requestRetry = jest.fn(async (request: { type: string }) => ({
      data: request.type === "get_prompt_receipt" ? ack() : ack(),
    }));
    const h = await mountSend({ engine: fakeEngine(), requestRetry });

    await act(async () => {
      await h.result.sendMessage("hello");
    });
    const retained = await loadPendingPrompt(credentials.pairId);
    expect(retained?.commandId).toBeTruthy();
    expect(warning).toHaveBeenCalledWith(
      "[remote] acknowledged prompt cleanup deferred",
      expect.objectContaining({ commandId: retained?.commandId }),
    );

    await act(async () => {
      await h.result.sendMessage("hello");
    });
    expect(requestRetry.mock.calls.filter(([request]) => request.type === "prompt")).toHaveLength(1);
    expect(requestRetry).toHaveBeenCalledWith(
      { type: "get_prompt_receipt", promptId: retained?.commandId },
      "list",
    );
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toBeNull();
    warning.mockRestore();
  });

  it("treats the same attachment set in a different order as the same queued prompt", async () => {
    const other: MobileAttachment = { ...attachment, localUri: "file:///tmp/b.png", name: "b.png" };
    await savePendingPrompt({
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      commandId: "prompt-reuse",
      draftKey: "desktop:session-1",
      sessionId: "session-1",
      text: "hello",
      attachments: [attachment, other],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    const requestRetry = jest.fn(async (request: { type: string }) =>
      request.type === "get_prompt_receipt" ? { data: null } : { data: ack() },
    );
    const h = await mountSend({ requestRetry });
    await act(async () => { await h.result.sendMessage("hello", [other, attachment]); });
    // Same text, same session, same attachments in a different order: the
    // queued prompt is reused, so the wire id is the one already on the desktop.
    expect(requestRetry).toHaveBeenCalledWith(
      expect.objectContaining({ type: "prompt", id: "prompt-reuse" }),
      "session-1",
    );
  });

  it("does not reuse a queued prompt whose attachment bytes changed", async () => {
    const other: MobileAttachment = { ...attachment, localUri: "file:///tmp/b.png", name: "b.png" };
    await savePendingPrompt({
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
      commandId: "prompt-stale",
      draftKey: "desktop:session-1",
      sessionId: "session-1",
      text: "hello",
      attachments: [attachment, other],
      modelId: "provider/model",
      thinkingLevel: "medium",
      mode: "chat",
      workspaceId: "",
      createdAt: 1,
    });
    // The test's own mock records every call, so the prompt it sent is there; the
    // `id` the outbox stamps on it is what this test reads back.
    const requestRetry = jest.fn(async (request: { type: string; id?: string }) =>
      request.type === "get_prompt_receipt" ? { data: null } : { data: ack() },
    );
    const h = await mountSend({ requestRetry });
    // Same name and URI, different byte size: a different payload, so the
    // queued prompt must be replaced rather than resent under its old id.
    const resized: MobileAttachment = { ...attachment, transferSize: 99 };
    await act(async () => { await h.result.sendMessage("hello", [resized, other]); });
    const sent = requestRetry.mock.calls.find(([request]) => request.type === "prompt")?.[0];
    expect(sent).toMatchObject({ type: "prompt", sessionId: "session-1" });
    expect(sent!.id).not.toBe("prompt-stale");
  });

  it("does not receipt-check a foreign prompt when the user sends a new message", async () => {
    await savePendingPrompt({
      version: 2,
      pairId: "other-pair",
      expectedDesktopId: "other-desktop",
      commandId: "foreign",
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
    const h = await mountSend();
    await act(async () => {
      await h.result.sendMessage("hello");
    });
    expect(h.requestRetry).toHaveBeenCalledTimes(1);
    expect(h.requestRetry).toHaveBeenCalledWith(
      expect.objectContaining({ type: "prompt", message: "hello" }),
      "session-1",
    );
  });

  it("replaces a stale pending prompt before delivering a new one", async () => {
    await savePendingPrompt({
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
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
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toBeNull();
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
      version: 2,
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
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
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toBeNull();
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
        compactingRef: { current: {} },
        conversationEpochRef,
        syncEngineRef,
        phase: "connecting",
        businessReady: true,
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
      renderer = create(createElement(Harness));
    });
    await flush();

    return { result, renderer, requestRetry, recordError, reconcileSession, refreshSessions, clientRef, credentialsRef };
  }

  it("delivers a fresh continuation and clears it", async () => {
    const requestRetry = jest.fn(async () => ({ data: ack() }));
    const h = await mountContinue(requestRetry);
    await act(async () => {
      await h.result.continueRun("session-1", "run-1");
    });
    expect(requestRetry).toHaveBeenCalledWith(
      {
        id: expect.stringMatching(/^continue_/),
        type: "continue_run",
        sessionId: "session-1",
        runId: "run-1",
      },
      "session-1",
    );
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toBeNull();
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

  it("rejects a continuation without a connected desktop", async () => {
    const h = await mountContinue(jest.fn(async () => ({ data: ack() })));
    h.clientRef.current = null;
    // A continuation resumes a stored run: with no channel it has to fail with
    // the named error instead of silently dropping the user's retry.
    await expect(h.result.continueRun("session-1", "failed-run")).rejects.toThrow("not_connected");
    expect(h.requestRetry).not.toHaveBeenCalled();
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
      version: 2,
      commandId: "continue-old",
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
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
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toBeNull();
  });

  it("clears the continuation and rethrows a non-transient failure", async () => {
    const requestRetry = jest.fn(async () => {
      throw new Error("boom");
    });
    const h = await mountContinue(requestRetry);
    await expect(h.result.continueRun("session-1", "run-1")).rejects.toThrow("boom");
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toBeNull();
  });

  it("preserves a continuation from a different pairing without sending it", async () => {
    await savePendingContinuation({
      version: 2,
      commandId: "continue-foreign",
      pairId: "another-pair",
      expectedDesktopId: "another-desktop",
      sessionId: "session-2",
      sourceRunId: "run-2",
      createdAt: 2,
    });
    const requestRetry = jest.fn(async () => ({ data: ack() }));
    const h = await mountContinue(requestRetry);
    await act(async () => {
      await h.result.continueRun("session-1", "run-1");
    });
    expect(requestRetry).not.toHaveBeenCalledWith(
      expect.objectContaining({ id: "continue-foreign" }),
      expect.anything(),
    );
    await expect(loadPendingContinuation("another-pair")).resolves.toMatchObject({ commandId: "continue-foreign" });
  });
});

describe("usePromptOutbox recovery error handling", () => {
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
    mockedUploadAttachments.mockResolvedValue([]);
  });

  async function mountRecovery(requestRetry: jest.Mock, initiallyReady = true) {
    const client = { requestRetry, accessIdentity: "active" } as unknown as RemoteClient;
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

    function Harness({ businessReady }: { businessReady: boolean }) {
      usePromptOutbox({
        clientRef,
        credentialsRef,
        selectedRef,
        streamingRef,
        compactingRef: { current: {} },
        conversationEpochRef,
        syncEngineRef,
        phase: "ready",
        businessReady,
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
      renderer = create(createElement(Harness, { businessReady: initiallyReady }));
    });
    await flush();

    return { renderer, requestRetry, recordError, reconcileSession, refreshSessions,
      setBusinessReady: async (businessReady: boolean) => {
        await act(async () => renderer.update(createElement(Harness, { businessReady })));
        await flush();
      },
    };
  }

  it.each(["prompt", "continuation"])("retains a frozen %s and checks its receipt when business readiness returns", async kind => {
    const common = {
      version: 2 as const, bridgeInstanceId: "active", pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId, commandId: "uncertain-operation",
      sessionId: "session-1", createdAt: 1,
    };
    if (kind === "prompt") {
      await savePendingPrompt({ ...common, draftKey: "session-1", text: "hello", attachments: [],
        modelId: "provider/model", thinkingLevel: "medium", mode: "chat", workspaceId: "" });
    } else {
      await savePendingContinuation({ ...common, sourceRunId: "failed-run" });
    }
    const load = () => kind === "prompt"
      ? loadPendingPrompt(credentials.pairId) : loadPendingContinuation(credentials.pairId);
    const requestRetry = jest.fn().mockRejectedValue(new Error("communication_frozen"));
    const h = await mountRecovery(requestRetry, false);
    try {
      expect(requestRetry).not.toHaveBeenCalled();
      await h.setBusinessReady(true);
      expect(requestRetry).toHaveBeenCalledTimes(1);
      await expect(load()).resolves.toMatchObject({ commandId: "uncertain-operation" });
      expect(h.recordError).not.toHaveBeenCalled();
      await h.setBusinessReady(false);
      requestRetry.mockResolvedValue({ data: ack() });
      await h.setBusinessReady(true);
      await expect(load()).resolves.toBeNull();
      expect(requestRetry).toHaveBeenCalledTimes(2);
      for (const [request] of requestRetry.mock.calls) {
        expect(request).toEqual({ type: "get_prompt_receipt", promptId: "uncertain-operation" });
      }
    } finally { await act(async () => h.renderer.unmount()); }
  });

  it("rechecks a receipt if readiness returns before the previous frozen probe settles", async () => {
    await savePendingPrompt({
      version: 2, bridgeInstanceId: "active", pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId, commandId: "pending-probe",
      draftKey: "session-1", sessionId: "session-1", text: "hello", attachments: [],
      modelId: "provider/model", thinkingLevel: "medium", mode: "chat",
      workspaceId: "", createdAt: 1,
    });
    const oldProbe = deferred();
    const requestRetry = jest.fn().mockReturnValueOnce(oldProbe.promise).mockResolvedValue({ data: ack() });
    const h = await mountRecovery(requestRetry);
    try {
      expect(requestRetry).toHaveBeenCalledTimes(1);
      await h.setBusinessReady(false);
      await h.setBusinessReady(true);
      expect(requestRetry).toHaveBeenCalledTimes(1);
      oldProbe.reject(new Error("communication_frozen"));
      await flush();
      expect(requestRetry).toHaveBeenCalledTimes(2);
      await expect(loadPendingPrompt(credentials.pairId)).resolves.toBeNull();
      expect(h.recordError).not.toHaveBeenCalled();
    } finally { await act(async () => h.renderer.unmount()); }
  });

  it("clears and records a non-transient prompt recovery failure", async () => {
    await savePendingPrompt({
      version: 2,
      bridgeInstanceId: "active",
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
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
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toBeNull();
    await act(async () => h.renderer.unmount());
  });

  it("clears and records a non-transient continuation recovery failure", async () => {
    await savePendingContinuation({
      version: 2,
      bridgeInstanceId: "active",
      commandId: "continue-err",
      pairId: credentials.pairId,
      expectedDesktopId: credentials.expectedDesktopId,
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
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toBeNull();
    await act(async () => h.renderer.unmount());
  });
  it.each(["retired", undefined])(
    "does not automatically execute unaccepted operations from access %s",
    async (bridgeInstanceId) => {
      await savePendingPrompt({
        version: 2,
        commandId: "stale-prompt",
        bridgeInstanceId,
        pairId: credentials.pairId,
        expectedDesktopId: credentials.expectedDesktopId,
        draftKey: "session-1",
        sessionId: "session-1",
        text: "keep my draft",
        attachments: [],
        modelId: "provider/model",
        thinkingLevel: "medium",
        mode: "chat",
        workspaceId: "",
        createdAt: 1,
      });
      await savePendingContinuation({
        version: 2,
        commandId: "stale-continue",
        bridgeInstanceId,
        pairId: credentials.pairId,
        expectedDesktopId: credentials.expectedDesktopId,
        sessionId: "session-1",
        sourceRunId: "failed",
        createdAt: 1,
      });
      const requestRetry = jest.fn(async () => ({ data: null }));
      const h = await mountRecovery(requestRetry);
      expect(requestRetry.mock.calls).toHaveLength(2);
      for (const call of requestRetry.mock.calls as unknown as [{ type: string }][]) {
        expect(call[0].type).toBe("get_prompt_receipt");
      }
      expect(h.recordError).not.toHaveBeenCalled();
      expect(h.reconcileSession).not.toHaveBeenCalled();
      await act(async () => h.renderer.unmount());
    },
  );
});

describe("usePromptOutbox delivery edges", () => {
  let warnSpy: jest.SpyInstance | undefined;
  const spyWarn = () => (warnSpy = jest.spyOn(console, "warn").mockImplementation(() => {}));

  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
    mockedUploadAttachments.mockResolvedValue([]);
  });

  function makeClient(
    impl: (request: { type: string; [key: string]: unknown }) => unknown,
    accessIdentity = "bridge-1",
  ): RemoteClient {
    return { accessIdentity, requestRetry: jest.fn(impl) } as unknown as RemoteClient;
  }

  /** Mount with the connection already usable, so the recovery effect runs.
   * `phase: "connecting"` mounts *without* running recovery, so a test can
   * stage a durable record and then drive the readiness edge itself. */
  async function mountEdge(opts: { client?: RemoteClient; engine?: SyncEngine | null; phase?: ConnectionPhase } = {}) {
    const client = opts.client ?? makeClient(async () => ({ data: ack() }));
    const requestRetry = (client as unknown as { requestRetry: jest.Mock }).requestRetry;
    const clientRef = { current: client as RemoteClient | null };
    const credentialsRef = { current: credentials as RemoteCredentials | null };
    const selectedRef = { current: "session-1" };
    const syncEngineRef = { current: (opts.engine ?? null) as SyncEngine | null };
    const refreshSessions = jest.fn(async () => {});
    const reconcileSession = jest.fn();
    const recordError = jest.fn();
    const setSelectedSessionId = jest.fn();
    let phase = opts.phase ?? "ready";
    let result!: ReturnType<typeof usePromptOutbox>;
    let renderer!: ReactTestRenderer;

    function Harness() {
      result = usePromptOutbox({
        clientRef,
        credentialsRef,
        selectedRef,
        streamingRef: { current: {} },
        compactingRef: { current: {} },
        conversationEpochRef: { current: 1 },
        syncEngineRef,
        phase,
        businessReady: true,
        draft: true,
        draftMode: "chat",
        draftWorkspaceId: "",
        modelId: "provider/model",
        thinkingLevel: "medium",
        fileTransferSupported: true,
        promptReceiptSupported: true,
        setSelectedSessionId,
        setDraft: jest.fn(),
        setDraftMode: jest.fn(),
        setDraftWorkspaceId: jest.fn(),
        refreshSessions,
        reconcileSession,
        recordError,
      });
      return null;
    }

    await act(async () => { renderer = create(createElement(Harness)); });
    await flush(60);
    const setPhase = async (next: ConnectionPhase) => {
      phase = next;
      await act(async () => { renderer.update(createElement(Harness)); });
      await flush(60);
    };
    return {
      result, renderer, client, requestRetry, clientRef, credentialsRef, selectedRef,
      refreshSessions, reconcileSession, recordError, setSelectedSessionId, setPhase,
    };
  }

  const stored = (overrides: Partial<Parameters<typeof savePendingPrompt>[0]>) => ({
    version: 2 as const,
    pairId: credentials.pairId,
    expectedDesktopId: credentials.expectedDesktopId,
    commandId: "prompt-reuse",
    draftKey: "desktop:session-1",
    sessionId: "session-1",
    text: "hello",
    attachments: [],
    modelId: "provider/model",
    thinkingLevel: "medium" as const,
    mode: "chat" as const,
    workspaceId: "",
    createdAt: 1,
    ...overrides,
  });

  const promptRequest = (h: Awaited<ReturnType<typeof mountEdge>>) => {
    const call = h.requestRetry.mock.calls.find(
      ([request]: [{ type: string }]) => request.type === "prompt",
    );
    return call?.[0] as { id: string; message: string; bridgeInstanceId?: string };
  };

  afterEach(() => {
    // NOTE: do NOT call `jest.restoreAllMocks()` here. It runs `.mockRestore()`
    // over every mock in the registry, which strips the implementations the
    // AsyncStorage / `../files` factories installed and silently turns later
    // tests' storage reads into `undefined`.
    warnSpy?.mockRestore();
    warnSpy = undefined;
    mockedUploadAttachments.mockReset();
    mockedUploadAttachments.mockResolvedValue([]);
  });

  it("keeps the durable record when the post-ack cleanup fails, so recovery checks the receipt instead of re-running the prompt", async () => {
    const warn = spyWarn();
    const h = await mountEdge();
    // The desktop accepted the command; only the phone-side cleanup fails.
    // Dropping the record here would lose the proof that this command already
    // ran, so a later recovery would mint a second one and run it twice.
    (AsyncStorage.removeItem as jest.Mock).mockRejectedValueOnce(new Error("disk full"));
    await act(async () => { await h.result.sendMessage("hello"); });
    expect(warn).toHaveBeenCalledWith(
      "[remote] acknowledged prompt cleanup deferred",
      expect.objectContaining({ commandId: expect.any(String) }),
    );
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toMatchObject({
      text: "hello",
    });
    await act(async () => h.renderer.unmount());
  });

  it("replaces a stored prompt that belongs to another desktop instead of delivering it", async () => {
    // Staged after mount: the mount-time recovery has its own handling of a
    // foreign record (covered above), so this exercises the send path only.
    const h = await mountEdge();
    await savePendingPrompt(stored({ expectedDesktopId: "other-desktop", text: "stale text", commandId: "stale-prompt" }), credentials.pairId);
    await act(async () => { await h.result.sendMessage("hello"); });
    // Same pair, different desktop: the record cannot be this send's receipt,
    // so it is dropped and the draft the user just wrote goes out instead.
    const request = promptRequest(h);
    expect(request.message).toBe("hello");
    expect(request.id).not.toBe("stale-prompt");
    await act(async () => h.renderer.unmount());
  });

  it("keeps the durable continuation when the post-ack cleanup fails, so it is not resumed twice", async () => {
    const warn = spyWarn();
    const h = await mountEdge();
    await savePendingContinuation({
      version: 2, commandId: "continue-1", bridgeInstanceId: "bridge-1",
      pairId: credentials.pairId, expectedDesktopId: credentials.expectedDesktopId,
      sessionId: "session-1", sourceRunId: "run-1", createdAt: 1,
    }, credentials.pairId);
    // The desktop accepted the resume; only the phone-side bookkeeping fails.
    const rm = AsyncStorage.removeItem as jest.Mock;
    rm.mockRejectedValueOnce(new Error("disk full"));
    await act(async () => { await h.result.continueRun("session-1", "run-1"); });
    expect(warn).toHaveBeenCalledWith(
      "[remote] acknowledged continuation cleanup deferred",
      expect.objectContaining({ commandId: "continue-1" }),
    );
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toMatchObject({
      commandId: "continue-1",
    });
    await act(async () => h.renderer.unmount());
  });

  it("abandons a mount-time recovery whose pairing changed while probing, keeping the record", async () => {
    const probe = deferred<{ data: PromptAck | null }>();
    let probes = 0;
    const client = makeClient((request) => {
      if (request.type === "get_prompt_receipt") { probes += 1; return probe.promise; }
      return { data: ack() };
    }, "bridge-1");
    const h = await mountEdge({ client, phase: "connecting" });
    await savePendingPrompt(stored({ commandId: "prompt-recover" }), credentials.pairId);
    // `setPhase` is awaited in full: starting another `act` while this one is
    // still open leaves React's act queue unbalanced and later renders in the
    // same file silently stop flushing.
    await h.setPhase("ready");
    expect(probes).toBe(1);
    // The phone is re-paired while the receipt probe is open. The answer would
    // arrive on a connection that no longer exists, so the recovery stops
    // without reconciling anything — and without erasing the record, which the
    // new pairing still has to resolve.
    h.clientRef.current = makeClient(async () => ({ data: ack() }));
    probe.resolve({ data: null });
    await act(async () => { await flush(30); });
    expect(h.reconcileSession).not.toHaveBeenCalled();
    expect(h.recordError).not.toHaveBeenCalled();
    await expect(loadPendingPrompt(credentials.pairId)).resolves.not.toBeNull();
    await act(async () => h.renderer.unmount());
  });

  it("rewrites a stored prompt's bridge instance before delivering it", async () => {
    const gate = deferred<{ data: PromptAck }>();
    const client = makeClient(
      (request) => request.type === "get_prompt_receipt" ? { data: null } : gate.promise,
      "bridge-new",
    );
    // Staged after mount: recovery owns the mount path (it is covered by the
    // "foreign pairing" tests), so this test owns the send path only.
    const h = await mountEdge({ client });
    await savePendingPrompt(stored({ bridgeInstanceId: "bridge-old" }), credentials.pairId);
    const send = h.result.sendMessage("hello");
    await flush(30);
    // The record is reused (same command id) so the receipt probe still guards
    // against a double run, but its bridge identity must follow the live
    // connection before the desktop sees it.
    await expect(loadPendingPrompt(credentials.pairId)).resolves.toMatchObject({
      commandId: "prompt-reuse",
      bridgeInstanceId: "bridge-new",
    });
    expect(promptRequest(h).bridgeInstanceId).toBe("bridge-new");
    gate.resolve({ data: ack() });
    await act(async () => { await send; });
    await act(async () => h.renderer.unmount());
  });

  it("aborts a send whose pairing changed mid-delivery, leaving the record for recovery", async () => {
    const h = await mountEdge();
    const replacement = makeClient(async () => ({ data: ack() }));
    mockedUploadAttachments.mockImplementation(async () => {
      // The user re-pairs (or switches desktop) while the upload is running.
      h.clientRef.current = replacement;
      return [];
    });
    await expect(h.result.sendMessage("hello")).rejects.toThrow("pairing_changed");
    // The command may already have reached the old desktop, so it is not
    // erased: recovery on the new pairing decides with a receipt.
    await expect(loadPendingPrompt(credentials.pairId)).resolves.not.toBeNull();
    await act(async () => h.renderer.unmount());
  });

  it("discards a continuation stored for another desktop when recovery runs", async () => {
    await savePendingContinuation({
      version: 2, commandId: "foreign-continue", pairId: credentials.pairId,
      expectedDesktopId: "other-desktop", sessionId: "session-9",
      sourceRunId: "run-9", createdAt: 1,
    }, credentials.pairId);
    const h = await mountEdge();
    expect(h.requestRetry).not.toHaveBeenCalled();
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toBeNull();
    expect(h.reconcileSession).not.toHaveBeenCalled();
    await act(async () => h.renderer.unmount());
  });

  it("drops a continuation stored for another desktop before continuing the run", async () => {
    const h = await mountEdge();
    await savePendingContinuation({
      version: 2, commandId: "stale-continue", pairId: credentials.pairId,
      expectedDesktopId: "other-desktop", sessionId: "session-1",
      sourceRunId: "run-1", createdAt: 1,
    }, credentials.pairId);
    await act(async () => { await h.result.continueRun("session-1", "run-1"); });
    const request = h.requestRetry.mock.calls.find(
      ([r]: [{ type: string }]) => r.type === "continue_run",
    )?.[0] as { id: string };
    // The stale record's command id would resume work on a desktop this phone
    // is no longer paired with, so a fresh one is minted for this pairing.
    expect(request.id).not.toBe("stale-continue");
    await act(async () => h.renderer.unmount());
  });

  it("rewrites a stored continuation's bridge instance before continuing", async () => {
    const gate = deferred<{ data: PromptAck }>();
    const client = makeClient(
      (request) => request.type === "get_prompt_receipt" ? { data: null } : gate.promise,
      "bridge-new",
    );
    const h = await mountEdge({ client });
    await savePendingContinuation({
      version: 2, commandId: "continue-reuse", bridgeInstanceId: "bridge-old",
      pairId: credentials.pairId, expectedDesktopId: credentials.expectedDesktopId,
      sessionId: "session-1", sourceRunId: "run-1", createdAt: 1,
    }, credentials.pairId);
    const continuing = h.result.continueRun("session-1", "run-1");
    await flush(30);
    await expect(loadPendingContinuation(credentials.pairId)).resolves.toMatchObject({
      commandId: "continue-reuse",
      bridgeInstanceId: "bridge-new",
    });
    gate.resolve({ data: ack() });
    await act(async () => { await continuing; });
    await act(async () => h.renderer.unmount());
  });

  it("abandons a queued continuation when the pairing changed while it waited", async () => {
    const gate = deferred<{ data: PromptAck }>();
    const client = makeClient(
      (request) => request.type === "get_prompt_receipt" ? { data: null } : gate.promise,
      "bridge-1",
    );
    const h = await mountEdge({ client });
    const first = h.result.continueRun("session-1", "run-1");
    first.catch(() => undefined);
    await flush(20);
    expect(h.requestRetry).toHaveBeenCalledWith(
      expect.objectContaining({ type: "continue_run" }), "session-1",
    );
    // A second, different continuation queues behind the first rather than
    // starting a parallel delivery.
    expect(h.requestRetry.mock.calls.filter(
      ([r]: [{ type: string }]) => r.type === "continue_run",
    )).toHaveLength(1);
    const second = h.result.continueRun("session-2", "run-2");
    second.catch(() => undefined);
    await flush(2);
    // The phone is re-paired while the second one is still waiting its turn.
    h.clientRef.current = makeClient(async () => ({ data: ack() }));
    gate.resolve({ data: ack() });
    await act(async () => { await flush(30); });
    await expect(second).rejects.toThrow("pairing_changed");
    await act(async () => h.renderer.unmount());
  });

  it("does not report a recovered continuation as done once the pairing is gone", async () => {
    const gate = deferred<{ data: PromptAck }>();
    const client = makeClient(
      (request) => request.type === "get_prompt_receipt" ? { data: null } : gate.promise,
      "bridge-1",
    );
    // Mount unready so recovery does not fire yet; the readiness edge below is
    // the path under test.
    const h = await mountEdge({ client, phase: "connecting" });
    await savePendingContinuation({
      version: 2, commandId: "continue-1", bridgeInstanceId: "bridge-1",
      pairId: credentials.pairId, expectedDesktopId: credentials.expectedDesktopId,
      sessionId: "session-2", sourceRunId: "run-2", createdAt: 1,
    }, credentials.pairId);
    const readiness = h.setPhase("ready");
    await readiness;
    expect(h.requestRetry).toHaveBeenCalledWith(
      expect.objectContaining({ type: "continue_run" }), "session-2",
    );
    // The phone is re-paired while the delivery is still open. The result
    // belongs to a pairing that no longer exists: it must not refresh the
    // catalogue or navigate to a conversation on the new desktop, and the
    // record must survive for the new pairing to resolve.
    h.clientRef.current = makeClient(async () => ({ data: ack() }));
    gate.resolve({ data: ack() });
    await act(async () => { await flush(30); });
    expect(h.reconcileSession).not.toHaveBeenCalled();
    expect(h.refreshSessions).not.toHaveBeenCalled();
    await expect(loadPendingContinuation(credentials.pairId)).resolves.not.toBeNull();
    await act(async () => h.renderer.unmount());
  });
});
