import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import { loadPendingContinuation, savePendingContinuation } from "../pendingContinuationStorage";
import { savePendingPrompt } from "../pendingPromptStorage";
import type { RemoteCredentials } from "../types";
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
