import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import {
  cachedPreviewForAttachment,
  downloadPrepared,
  prepareDownload,
  rememberPreparedPreview,
} from "../files";
import { loadLastModel, loadLastThinking, saveLastModel, saveLastThinking } from "../storage";
import { emptyTimeline } from "../timeline";
import type { DownloadInfo, HistoryAttachment, RemoteModel, RemoteSessionState } from "../types";
import type { SyncEngine } from "../syncEngine";
import { useConversationController } from "../useConversationController";

jest.mock("../files", () => ({
  prepareDownload: jest.fn(),
  cachedPreviewForAttachment: jest.fn(),
  downloadPrepared: jest.fn(),
  rememberPreparedPreview: jest.fn(),
}));

jest.mock("../storage", () => ({
  loadLastModel: jest.fn(),
  loadLastThinking: jest.fn(),
  saveLastModel: jest.fn(),
  saveLastThinking: jest.fn(),
}));

const mockedPrepareDownload = prepareDownload as jest.MockedFunction<typeof prepareDownload>;
const mockedCachedPreview = cachedPreviewForAttachment as jest.MockedFunction<
  typeof cachedPreviewForAttachment
>;
const mockedDownloadPrepared = downloadPrepared as jest.MockedFunction<typeof downloadPrepared>;
const mockedRememberPrepared = rememberPreparedPreview as jest.MockedFunction<
  typeof rememberPreparedPreview
>;
const mockedLoadLastModel = loadLastModel as jest.MockedFunction<typeof loadLastModel>;
const mockedLoadLastThinking = loadLastThinking as jest.MockedFunction<typeof loadLastThinking>;
const mockedSaveLastModel = saveLastModel as jest.MockedFunction<typeof saveLastModel>;
const mockedSaveLastThinking = saveLastThinking as jest.MockedFunction<typeof saveLastThinking>;

const downloadInfo: DownloadInfo = {
  transferId: "transfer-1",
  name: "a.jpg",
  mimeType: "image/jpeg",
  size: 10,
  contentHash: "hash",
  previewKind: "image",
  variant: "preview",
  chunkBytes: 4,
};

const historyAttachment: HistoryAttachment = { path: "a.jpg", name: "a.jpg", kind: "image" };

function model(id: string, provider?: string, extra: Partial<RemoteModel> = {}): RemoteModel {
  return { id, ...(provider ? { provider } : {}), ...extra };
}

function fakeEngine(): SyncEngine {
  return {
    reconcile: jest.fn(),
    mutate: jest.fn((_sessionId: string, apply: (tl: ReturnType<typeof emptyTimeline>) => unknown) => {
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

interface MountOpts {
  client?: RemoteClient | null;
  selected?: string;
  models?: RemoteModel[];
  engine?: SyncEngine | null;
  requestRetry?: jest.Mock;
  request?: jest.Mock;
  removeSession?: jest.Mock;
  closeConversation?: jest.Mock;
}

type ControllerResult = ReturnType<typeof useConversationController>;

async function mountController(opts: MountOpts = {}) {
  const requestRetry =
    opts.requestRetry ?? jest.fn(async () => ({ data: {} as RemoteSessionState }));
  const request = opts.request ?? jest.fn(async () => ({ data: {} }));
  const client = { requestRetry, request } as unknown as RemoteClient;
  const clientRef = {
    current: (opts.client === undefined ? client : opts.client) as RemoteClient | null,
  };
  const selectedRef = { current: opts.selected ?? "" };
  const syncEngineRef = { current: (opts.engine ?? null) as SyncEngine | null };
  const hydrateAttachmentsRef = {
    current: jest.fn(async () => {}) as (sessionId: string) => Promise<void>,
  };
  const conversationEpochRef = { current: 0 };
  const setSelectedSessionId = jest.fn();
  const setDraft = jest.fn();
  const setDraftMode = jest.fn();
  const setDraftWorkspaceId = jest.fn();
  const setUnreadSessions = jest.fn();
  const setApprovalTierState = jest.fn();
  const ensureDraftTimeline = jest.fn();
  const recordError = jest.fn();
  const removeSession = opts.removeSession ?? jest.fn(async () => true);
  const closeConversation = opts.closeConversation ?? jest.fn();
  const result: { current: ControllerResult | null } = { current: null };
  let renderer!: ReactTestRenderer;

  function Harness() {
    result.current = useConversationController({
      clientRef,
      selectedRef,
      syncEngineRef,
      hydrateAttachmentsRef,
      conversationEpochRef,
      models: opts.models ?? [],
      setSelectedSessionId,
      setDraft,
      setDraftMode,
      setDraftWorkspaceId,
      setUnreadSessions,
      setApprovalTierState,
      ensureDraftTimeline,
      recordError,
      removeSession,
      closeConversation,
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
    request,
    clientRef,
    selectedRef,
    syncEngineRef,
    setSelectedSessionId,
    setDraft,
    setDraftMode,
    setDraftWorkspaceId,
    setUnreadSessions,
    setApprovalTierState,
    ensureDraftTimeline,
    recordError,
    removeSession,
    closeConversation,
  };
}

function current(h: Awaited<ReturnType<typeof mountController>>): ControllerResult {
  return h.result.current!;
}

beforeEach(() => {
  jest.clearAllMocks();
  mockedLoadLastModel.mockResolvedValue(null);
  mockedLoadLastThinking.mockResolvedValue(null);
});

describe("selectSession", () => {
  it("is a no-op when the client is absent", async () => {
    const h = await mountController({ client: null });
    await act(async () => {
      await current(h).selectSession("s1");
    });
    expect(h.setSelectedSessionId).not.toHaveBeenCalled();
  });

  it("opens a session and applies the remote state", async () => {
    const h = await mountController({
      selected: "s1",
      engine: fakeEngine(),
      models: [model("gpt-4", "openai")],
      requestRetry: jest.fn(async () => ({
        data: { model: "openai/gpt-4", thinkingLevel: "high" } as RemoteSessionState,
      })),
    });
    await act(async () => {
      await current(h).selectSession("s1");
    });
    expect(h.setSelectedSessionId).toHaveBeenCalledWith("s1");
    expect(h.setDraft).toHaveBeenCalledWith(false);
    expect(current(h).modelId).toBe("openai/gpt-4");
    expect(current(h).thinkingLevel).toBe("high");
    const engine = h.syncEngineRef.current as unknown as { reconcile: jest.Mock };
    expect(engine.reconcile).toHaveBeenCalledWith("s1", "open");
  });

  it("keeps the raw model reference when no catalogue model matches", async () => {
    const h = await mountController({
      selected: "s1",
      models: [model("gpt-4", "openai")],
      requestRetry: jest.fn(async () => ({
        data: { model: "custom/model", thinkingLevel: undefined } as RemoteSessionState,
      })),
    });
    await act(async () => {
      await current(h).selectSession("s1");
    });
    expect(current(h).modelId).toBe("custom/model");
    expect(current(h).thinkingLevel).toBe("off");
  });

  it("clears a session from the unread set only when present", async () => {
    const h = await mountController({ selected: "s1" });
    await act(async () => {
      await current(h).selectSession("s1");
    });
    const updater = h.setUnreadSessions.mock.calls[0][0] as (
      previous: Set<string>,
    ) => Set<string>;
    expect(updater(new Set(["s1", "s2"]))).toEqual(new Set(["s2"]));
    const untouched = new Set(["s2"]);
    expect(updater(untouched)).toBe(untouched);
  });

  it("records a state-fetch failure", async () => {
    const h = await mountController({
      selected: "s1",
      engine: fakeEngine(),
      requestRetry: jest.fn(async () => {
        throw new Error("offline");
      }),
    });
    await act(async () => {
      await current(h).selectSession("s1");
    });
    expect(h.recordError).toHaveBeenCalledWith(expect.objectContaining({ message: "offline" }));
    const engine = h.syncEngineRef.current as unknown as { reconcile: jest.Mock };
    expect(engine.reconcile).toHaveBeenCalledWith("s1", "open");
  });
});

describe("newConversation", () => {
  it("reuses the last model when it still exists", async () => {
    mockedLoadLastModel.mockResolvedValue("openai/gpt-4");
    const h = await mountController({ models: [model("gpt-4", "openai")] });
    await act(async () => {
      await current(h).newConversation();
    });
    expect(current(h).modelId).toBe("openai/gpt-4");
    expect(h.setDraft).toHaveBeenCalledWith(true);
    expect(h.ensureDraftTimeline).toHaveBeenCalled();
  });

  it("falls back to the default model when the last model is gone", async () => {
    mockedLoadLastModel.mockResolvedValue("gone/model");
    const h = await mountController({
      models: [model("gpt-4", "openai", { isDefault: true }), model("other", "x")],
    });
    await act(async () => {
      await current(h).newConversation();
    });
    expect(current(h).modelId).toBe("openai/gpt-4");
  });

  it("falls back to the first model when there is no default", async () => {
    const h = await mountController({ models: [model("m", "p")] });
    await act(async () => {
      await current(h).newConversation();
    });
    expect(current(h).modelId).toBe("p/m");
  });

  it("leaves the model empty when there are no models", async () => {
    const h = await mountController({ models: [] });
    await act(async () => {
      await current(h).newConversation();
    });
    expect(current(h).modelId).toBe("");
  });

  it("starts a workspace draft with the requested mode and workspace", async () => {
    const h = await mountController({ models: [model("m", "p")] });
    await act(async () => {
      await current(h).newConversation("workspace", "ws-1");
    });
    expect(h.setDraftMode).toHaveBeenCalledWith("workspace");
    expect(h.setDraftWorkspaceId).toHaveBeenCalledWith("ws-1");
    expect(h.setSelectedSessionId).toHaveBeenCalledWith("");
  });
});

describe("attachment helpers", () => {
  it("prepareAttachment throws without a client or session", async () => {
    const h = await mountController({ client: null });
    await expect(current(h).prepareAttachment(historyAttachment)).rejects.toThrow(
      "attachment_no_session",
    );
  });

  it("prepareAttachment prepares and remembers a preview", async () => {
    mockedPrepareDownload.mockResolvedValue(downloadInfo);
    const h = await mountController({ selected: "s1" });
    const info = await current(h).prepareAttachment(historyAttachment);
    expect(info).toBe(downloadInfo);
    expect(mockedPrepareDownload).toHaveBeenCalledWith(
      expect.anything(),
      "s1",
      historyAttachment,
      "preview",
      undefined,
      undefined,
    );
    expect(mockedRememberPrepared).toHaveBeenCalledWith(historyAttachment, downloadInfo);
  });

  it("cachedAttachment delegates to the cache", async () => {
    const cached = { info: downloadInfo, file: {} as never };
    mockedCachedPreview.mockReturnValue(cached as never);
    const h = await mountController({});
    expect(current(h).cachedAttachment(historyAttachment)).toBe(cached);
  });

  it("downloadAttachment throws without a client", async () => {
    const h = await mountController({ client: null });
    await expect(current(h).downloadAttachment(downloadInfo)).rejects.toThrow(
      "attachment_not_connected",
    );
  });

  it("downloadAttachment delegates to the downloader", async () => {
    mockedDownloadPrepared.mockResolvedValue({} as never);
    const h = await mountController({});
    await current(h).downloadAttachment(downloadInfo);
    expect(mockedDownloadPrepared).toHaveBeenCalledWith(
      expect.anything(),
      downloadInfo,
      undefined,
      undefined,
      undefined,
    );
  });
});

describe("command dispatchers", () => {
  it("abort is a no-op without a client or session", async () => {
    const h = await mountController({});
    await act(async () => {
      await current(h).abort();
    });
    expect(h.request).not.toHaveBeenCalled();
  });

  it("abort sends the abort command for the selected session", async () => {
    const h = await mountController({ selected: "s1" });
    await act(async () => {
      await current(h).abort();
    });
    expect(h.request).toHaveBeenCalledWith({ type: "abort", sessionId: "s1" }, "s1");
  });

  it("setModel persists the model and sends set_model when connected", async () => {
    const h = await mountController({ selected: "s1" });
    await act(async () => {
      await current(h).setModel("openai/gpt-5");
    });
    expect(mockedSaveLastModel).toHaveBeenCalledWith("openai/gpt-5");
    expect(h.request).toHaveBeenCalledWith(
      {
        type: "set_model",
        sessionId: "s1",
        modelId: "openai/gpt-5",
        providerId: "openai",
      },
      "s1",
    );
  });

  it("setModel skips the command when no session is selected", async () => {
    const h = await mountController({ selected: "" });
    await act(async () => {
      await current(h).setModel("openai/gpt-5");
    });
    expect(mockedSaveLastModel).toHaveBeenCalledWith("openai/gpt-5");
    expect(h.request).not.toHaveBeenCalled();
  });

  it("setThinkingLevel persists and sends set_thinking_level when connected", async () => {
    const h = await mountController({ selected: "s1" });
    await act(async () => {
      await current(h).setThinkingLevel("high");
    });
    expect(mockedSaveLastThinking).toHaveBeenCalledWith("high");
    expect(h.request).toHaveBeenCalledWith(
      { type: "set_thinking_level", sessionId: "s1", level: "high" },
      "s1",
    );
  });

  it("setThinkingLevel skips the command when no session is selected", async () => {
    const h = await mountController({ selected: "" });
    await act(async () => {
      await current(h).setThinkingLevel("high");
    });
    expect(mockedSaveLastThinking).toHaveBeenCalledWith("high");
    expect(h.request).not.toHaveBeenCalled();
  });

  it("setApprovalTier throws without a client", async () => {
    const h = await mountController({ client: null });
    await expect(current(h).setApprovalTier("auto")).rejects.toThrow("not_connected");
  });

  it("setApprovalTier sends the command and applies the response", async () => {
    const h = await mountController({
      request: jest.fn(async () => ({ data: { approvalTier: "manual" } })),
    });
    await act(async () => {
      await current(h).setApprovalTier("manual");
    });
    expect(h.setApprovalTierState).toHaveBeenCalledWith("manual");
  });

  it("deleteSession closes the conversation when removal succeeds", async () => {
    const closeConversation = jest.fn();
    const h = await mountController({ closeConversation });
    await act(async () => {
      await current(h).deleteSession("s1", "thread-1");
    });
    expect(closeConversation).toHaveBeenCalled();
  });

  it("deleteSession keeps the conversation open when removal fails", async () => {
    const closeConversation = jest.fn();
    const removeSession = jest.fn(async () => false);
    const h = await mountController({ closeConversation, removeSession });
    await act(async () => {
      await current(h).deleteSession("s1", "thread-1");
    });
    expect(closeConversation).not.toHaveBeenCalled();
  });

  it("decideApproval is a no-op without a client or session", async () => {
    const h = await mountController({ client: null });
    await act(async () => {
      await current(h).decideApproval("a1", "approved");
    });
    expect(h.request).not.toHaveBeenCalled();
  });

  it("decideApproval sends the decision and mutates the timeline", async () => {
    const h = await mountController({ selected: "s1", engine: fakeEngine() });
    await act(async () => {
      await current(h).decideApproval("a1", "rejected");
    });
    expect(h.request).toHaveBeenCalledWith(
      { type: "approval_decision", sessionId: "s1", entryId: "a1", mode: "rejected" },
      "s1",
    );
    const engine = h.syncEngineRef.current as unknown as { mutate: jest.Mock };
    expect(engine.mutate).toHaveBeenCalledWith("s1", expect.any(Function));
  });
});
