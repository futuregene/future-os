import { createElement } from "react";
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
import { modelReference, type DownloadInfo, type HistoryAttachment, type RemoteModel, type RemoteSessionState } from "../types";
import type { SyncEngine } from "../syncEngine";
import { useConversationController } from "../useConversationController";

jest.mock("../files", () => ({
  prepareDownload: jest.fn(),
  cachedPreviewForAttachment: jest.fn(),
  downloadPrepared: jest.fn(),
  rememberPreparedPreview: jest.fn(),
  TransferCancelledError: class extends Error { constructor() { super("transfer_cancelled"); } },
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
    open: jest.fn(),
    reconcile: jest.fn(),
    mutate: jest.fn(
      (_sessionId: string, apply: (tl: ReturnType<typeof emptyTimeline>) => unknown) => {
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

interface MountOpts {
  client?: RemoteClient | null;
  selected?: string;
  models?: RemoteModel[];
  engine?: SyncEngine | null;
  requestRetry?: jest.Mock;
  request?: jest.Mock;
  removeSession?: jest.Mock;
  removeWorkspace?: jest.Mock;
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
  if (syncEngineRef.current && jest.isMockFunction(syncEngineRef.current.open)) {
    (syncEngineRef.current.open as jest.Mock).mockImplementation(async (sessionId: string) =>
      (await requestRetry({ type: "get_state", sessionId }, sessionId)).data,
    );
  }
  const conversationEpochRef = { current: 0 };
  const setSelectedSessionId = jest.fn();
  const setDraft = jest.fn();
  const setDraftMode = jest.fn();
  const setDraftWorkspaceId = jest.fn();
  const setUnreadSessions = jest.fn();
  const setApprovalTierState = jest.fn();
  const ensureDraftTimeline = jest.fn();
  const prepareTimelineOpen = jest.fn();
  const recordError = jest.fn();
  const removeSession = opts.removeSession ?? jest.fn(async () => true);
  const removeWorkspace = opts.removeWorkspace ?? jest.fn(async () => true);
  const closeConversation = opts.closeConversation ?? jest.fn();
  const result: { current: ControllerResult | null } = { current: null };
  let renderer!: ReactTestRenderer;

  function Harness() {
    result.current = useConversationController({
      clientRef,
      selectedRef,
      syncEngineRef,
      conversationEpochRef,
      models: opts.models ?? [],
      setSelectedSessionId,
      setDraft,
      setDraftMode,
      setDraftWorkspaceId,
      setUnreadSessions,
      setApprovalTierState,
      ensureDraftTimeline,
      prepareTimelineOpen,
      recordError,
      removeSession,
      removeWorkspace,
      closeConversation,
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
    request,
    clientRef,
    selectedRef,
    syncEngineRef,
    conversationEpochRef,
    setSelectedSessionId,
    setDraft,
    setDraftMode,
    setDraftWorkspaceId,
    setUnreadSessions,
    setApprovalTierState,
    ensureDraftTimeline,
    prepareTimelineOpen,
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

describe("installed skills", () => {
  it("loads the connected Agent catalogue without requiring a saved session", async () => {
    const skills = [{ name: "web", description: "Search" }];
    const h = await mountController({ request: jest.fn(async () => ({ data: { skills } })) });
    await expect(h.result.current!.listSkills()).resolves.toEqual(skills);
    expect(h.request).toHaveBeenCalledWith({ type: "list_skills" });
    act(() => h.renderer.unmount());
  });

  it("rejects late results after switching conversation or desktop", async () => {
    for (const change of ["conversation", "desktop"]) {
      let resolve!: (response: unknown) => void;
      const h = await mountController({ request: jest.fn(() => new Promise(yes => { resolve = yes; })) });
      const pending = h.result.current!.listSkills();
      const rejected = expect(pending).rejects.toThrow("skills_context_changed");
      if (change === "conversation") h.conversationEpochRef.current += 1;
      else h.clientRef.current = null;
      resolve({ data: { skills: [] } });
      await rejected;
      act(() => h.renderer.unmount());
    }
  });

  it("does not mistake a malformed response for an empty skill list", async () => {
    const h = await mountController({ request: jest.fn(async () => ({ data: {} })) });
    await expect(h.result.current!.listSkills()).rejects.toThrow("skills_invalid_response");
    act(() => h.renderer.unmount());
  });
});

describe("session file browsing", () => {
  it("scopes chunked directory reads to the selected session", async () => {
    const listing = { rootPath: "/work", path: "/work/sub", entries: [] };
    const h = await mountController({ selected: "session-a", requestRetry: jest.fn().mockResolvedValue({ data: listing }) });
    await expect(current(h).listSessionFiles("/work/sub")).resolves.toEqual(listing);
    expect(h.requestRetry).toHaveBeenCalledWith({
      type: "list_session_files", sessionId: "session-a", filePath: "/work/sub", chunkedRead: true,
    }, "session-a");
    act(() => h.renderer.unmount());
  });

  it("rejects a listing completed after switching sessions", async () => {
    const pending = deferred<{ data: unknown }>();
    const h = await mountController({ selected: "a", requestRetry: jest.fn().mockReturnValue(pending.promise) });
    const result = current(h).listSessionFiles();
    h.selectedRef.current = "b";
    pending.resolve({ data: { rootPath: "/a", path: "/a", entries: [] } });
    await expect(result).rejects.toThrow("stale_sync_lane");
    act(() => h.renderer.unmount());
  });

  it("does not request a directory for an unsent draft", async () => {
    const h = await mountController();
    await expect(current(h).listSessionFiles()).rejects.toThrow("attachment_no_session");
    expect(h.requestRetry).not.toHaveBeenCalled();
    act(() => h.renderer.unmount());
  });
});

describe("navigation races", () => {
  it.each(["resolve", "reject"] as const)("an old A response (%s) cannot clear B's opening state", async outcome => {
    const a = deferred<{ data: RemoteSessionState }>();
    const b = deferred<{ data: RemoteSessionState }>();
    const h = await mountController({ requestRetry: jest.fn().mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise) });
    let openA!: Promise<void>;
    let openB!: Promise<void>;
    await act(async () => { openA = current(h).selectSession("a"); });
    await act(async () => { openB = current(h).selectSession("b"); });
    await act(async () => {
      if (outcome === "resolve") a.resolve({ data: { model: "old", thinkingLevel: "high" } });
      else a.reject(new Error("old connection"));
      await openA;
    });
    expect(current(h).openingSession).toBe(true);
    expect(current(h).modelId).toBe("");
    expect(h.recordError).not.toHaveBeenCalled();
    await act(async () => { b.resolve({ data: { model: "new", thinkingLevel: "off" } }); await openB; });
    expect(current(h).modelId).toBe("new");
    expect(current(h).openingSession).toBe(false);
    act(() => h.renderer.unmount());
  });

  it("a late A state cannot overwrite an already-open B", async () => {
    const a = deferred<{ data: RemoteSessionState }>();
    const h = await mountController({ requestRetry: jest.fn().mockReturnValueOnce(a.promise).mockResolvedValueOnce({ data: { model: "b", thinkingLevel: "low" } }) });
    let opening!: Promise<void>;
    await act(async () => { opening = current(h).selectSession("a"); });
    await act(async () => { await current(h).selectSession("b"); });
    await act(async () => { a.resolve({ data: { model: "a", thinkingLevel: "high" } }); await opening; });
    expect(current(h).modelId).toBe("b");
    expect(current(h).thinkingLevel).toBe("low");
    act(() => h.renderer.unmount());
  });

  it("slow draft preferences cannot navigate away from a subsequently opened session", async () => {
    const preference = deferred<string | null>();
    mockedLoadLastModel.mockReturnValueOnce(preference.promise);
    const h = await mountController({ requestRetry: jest.fn().mockResolvedValue({ data: { model: "b" } }) });
    let draft!: Promise<void>;
    await act(async () => { draft = current(h).newConversation(); });
    expect(h.selectedRef.current).toBe("");
    await act(async () => { await current(h).selectSession("b"); });
    await act(async () => { preference.resolve("old"); await draft; });
    expect(h.selectedRef.current).toBe("b");
    expect(current(h).modelId).toBe("b");
    act(() => h.renderer.unmount());
  });

  it.each(["model", "thinking"] as const)("a slow %s preference write cannot send the command to another session", async setting => {
    const saved = deferred<void>();
    if (setting === "model") mockedSaveLastModel.mockReturnValueOnce(saved.promise);
    else mockedSaveLastThinking.mockReturnValueOnce(saved.promise);
    const h = await mountController({ selected: "a" });
    let changing!: Promise<void>;
    await act(async () => { changing = setting === "model" ? current(h).setModel("p/m") : current(h).setThinkingLevel("high"); });
    h.selectedRef.current = "b";
    await act(async () => { saved.resolve(); await changing; });
    expect(h.request).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "a" }), "a");
    act(() => h.renderer.unmount());
  });
});

describe("desktop session setting synchronization", () => {
  test("live model/thinking changes update only the active conversation, without sending commands back", async () => {
    const h = await mountController({ selected: "s1" });
    await act(async () => {
      current(h).handleSessionSettingsEvent({ type: "model_changed", data: '{"model":"p/org/new"}' }, "s1");
      current(h).handleSessionSettingsEvent({ type: "thinking_level_changed", data: '{"level":"high"}' }, "s1");
    });
    expect(current(h).modelId).toBe("p/org/new");
    expect(current(h).thinkingLevel).toBe("high");
    await act(async () => {
      current(h).handleSessionSettingsEvent({ type: "model_changed", data: '{"model":"background"}' }, "other");
      current(h).handleSessionSettingsEvent({ type: "model_changed", data: 'not-json' }, "s1");
      current(h).handleSessionSettingsEvent({ type: "thinking_level_changed", data: '{"level":"invalid"}' }, "s1");
    });
    expect(current(h).modelId).toBe("p/org/new");
    expect(current(h).thinkingLevel).toBe("high");
    expect(h.request).not.toHaveBeenCalled();
    act(() => h.renderer.unmount());
  });

  test("reconnect state restores missed model changes without closing the conversation", async () => {
    const h = await mountController({ selected: "s1" });
    act(() => current(h).applySessionSettings("s1", { model: "p/restored", thinkingLevel: "medium" }));
    expect(current(h).modelId).toBe("p/restored");
    expect(current(h).thinkingLevel).toBe("medium");
    act(() => current(h).applySessionSettings("other", { model: "wrong" }));
    expect(current(h).modelId).toBe("p/restored");
    act(() => h.renderer.unmount());
  });

  test("an old open response cannot overwrite a newer desktop model notification", async () => {
    let resolve!: (value: unknown) => void;
    const requestRetry = jest.fn(() => new Promise(yes => { resolve = yes; }));
    const h = await mountController({ requestRetry });
    let opening!: Promise<void>;
    act(() => { opening = current(h).selectSession("s1"); });
    act(() => current(h).handleSessionSettingsEvent({ type: "model_changed", data: '{"model":"p/new"}' }, "s1"));
    await act(async () => { resolve({ data: { model: "p/old" } }); await opening; });
    expect(current(h).modelId).toBe("p/new");
    expect(current(h).openingSession).toBe(false);
    act(() => h.renderer.unmount());
  });
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
    expect(h.prepareTimelineOpen).toHaveBeenCalledWith("s1");
    expect(h.setDraft).toHaveBeenCalledWith(false);
    expect(current(h).modelId).toBe("openai/gpt-4");
    expect(current(h).thinkingLevel).toBe("high");
    const engine = h.syncEngineRef.current as unknown as { open: jest.Mock };
    expect(engine.open).toHaveBeenCalledWith("s1");
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
    const updater = h.setUnreadSessions.mock.calls[0][0] as (previous: Set<string>) => Set<string>;
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
    const engine = h.syncEngineRef.current as unknown as { open: jest.Mock };
    expect(engine.open).toHaveBeenCalledWith("s1");
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
    expect(mockedRememberPrepared).toHaveBeenCalledWith(h.clientRef.current, "s1", historyAttachment, downloadInfo);
  });

  it("cachedAttachment delegates to the cache", async () => {
    const cached = { info: downloadInfo, file: {} as never };
    mockedCachedPreview.mockReturnValue(cached as never);
    const h = await mountController({ selected: "s1" });
    expect(current(h).cachedAttachment(historyAttachment)).toBe(cached);
    expect(mockedCachedPreview).toHaveBeenCalledWith(h.clientRef.current, "s1", historyAttachment, "preview");
  });

  it.each(["desktop", "session", "epoch"])("discards a prepared attachment after changing %s", async change => {
    const deferredInfo = deferred<DownloadInfo>();
    mockedPrepareDownload.mockReturnValueOnce(deferredInfo.promise);
    const h = await mountController({ selected: "s1" });
    const pending = current(h).prepareAttachment(historyAttachment);
    if (change === "desktop") h.clientRef.current = null;
    else if (change === "session") h.selectedRef.current = "s2";
    else h.conversationEpochRef.current += 1;
    deferredInfo.resolve(downloadInfo);
    await expect(pending).rejects.toThrow("transfer_cancelled");
    expect(mockedRememberPrepared).not.toHaveBeenCalled();
    expect(h.request).toHaveBeenCalledWith({ type: "download_cancel", transferId: downloadInfo.transferId }, "transfer");
    act(() => h.renderer.unmount());
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
  it("sends distinct qualified identities for two providers sharing a slash-containing id", async () => {
    const catalog = [
      model("deepseek/deepseek-v4-flash", "deepseek"),
      model("deepseek/deepseek-v4-flash", "ambient"),
    ];
    const h = await mountController({ selected: "s1", models: catalog });
    for (const selected of catalog) {
      await act(async () => current(h).setModel(modelReference(selected)));
    }
    expect(h.request.mock.calls.map(([command]) => command.modelId)).toEqual([
      "deepseek/deepseek/deepseek-v4-flash",
      "ambient/deepseek/deepseek-v4-flash",
    ]);
    expect(h.request.mock.calls.map(([command]) => command.providerId)).toEqual(["deepseek", "ambient"]);
  });

  it("abort is a no-op without a session", async () => {
    const h = await mountController({});
    await act(async () => {
      await current(h).abort();
    });
    expect(h.request).not.toHaveBeenCalled();
  });

  it("abort rejects instead of falsely acknowledging when the client is disconnected", async () => {
    const h = await mountController({ client: null, selected: "s1" });
    await expect(current(h).abort()).rejects.toThrow("not_connected");
    expect(h.request).not.toHaveBeenCalled();
  });

  it("abort sends the abort command for the selected session", async () => {
    const h = await mountController({ selected: "s1" });
    await act(async () => {
      await current(h).abort();
    });
    expect(h.request).toHaveBeenCalledWith({ type: "abort", sessionId: "s1" }, "s1");
  });

  it("compactContext sends the manual compaction for the selected session", async () => {
    const request = jest.fn(async () => ({ data: { accepted: true, operationId: "cmp-1" } }));
    const h = await mountController({ selected: "s1", request });
    await expect(current(h).compactContext()).resolves.toEqual({
      sessionId: "s1",
      operationId: "cmp-1",
    });
    expect(request).toHaveBeenCalledWith({ type: "compact_context", sessionId: "s1" }, "s1");
  });

  it("compactContext never invents an operation for a missing acknowledgement", async () => {
    const request = jest.fn(async () => ({ data: { accepted: true } }));
    const h = await mountController({ selected: "s1", request });
    await expect(current(h).compactContext()).rejects.toThrow("compaction_invalid_ack");
    const disconnected = await mountController({ client: null, selected: "s1" });
    await expect(current(disconnected).compactContext()).rejects.toThrow("not_connected");
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

  it("does not apply a previous desktop's approval tier to the new desktop", async () => {
    const response = deferred<{ data: { approvalTier: string } }>();
    const h = await mountController({ request: jest.fn().mockReturnValue(response.promise) });
    const pending = current(h).setApprovalTier("off");
    h.clientRef.current = null;
    response.resolve({ data: { approvalTier: "off" } });
    await pending;
    expect(h.setApprovalTierState).not.toHaveBeenCalled();
    act(() => h.renderer.unmount());
  });

  it("does not apply an old desktop's approval decision to a replacement timeline", async () => {
    const response = deferred<{ data: object }>();
    const h = await mountController({ selected: "s1", engine: fakeEngine(), request: jest.fn().mockReturnValue(response.promise) });
    const pending = current(h).decideApproval("approval", "approved");
    h.clientRef.current = null;
    const replacement = fakeEngine();
    h.syncEngineRef.current = replacement;
    response.resolve({ data: {} });
    await pending;
    expect(replacement.mutate).not.toHaveBeenCalled();
    act(() => h.renderer.unmount());
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
