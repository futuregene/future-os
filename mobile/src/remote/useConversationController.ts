import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback, useRef, useState } from "react";
import type { RemoteClient } from "./client";
import {
  cachedPreviewForAttachment,
  downloadPrepared,
  prepareDownload,
  rememberPreparedPreview,
  TransferCancelledError,
} from "./files";
import type { SyncEngine } from "./syncEngine";
import { requestReadPage } from "./readPages";
import { loadLastModel, loadLastThinking, saveLastModel, saveLastThinking } from "./storage";
import { markApprovalDecision } from "./timeline";
import { modelProviderFromReference, modelReference } from "./types";
import type {
  DownloadInfo,
  HistoryAttachment,
  RemoteModel,
  RemoteSessionState,
  RemoteSkill,
  SessionFileListing,
  StreamEvent,
  ThinkingLevel,
} from "./types";

interface ConversationControllerOptions {
  clientRef: MutableRefObject<RemoteClient | null>;
  selectedRef: MutableRefObject<string>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  conversationEpochRef: MutableRefObject<number>;
  models: RemoteModel[];
  setSelectedSessionId: Dispatch<SetStateAction<string>>;
  setDraft: Dispatch<SetStateAction<boolean>>;
  setDraftMode: Dispatch<SetStateAction<"chat" | "workspace">>;
  setDraftWorkspaceId: Dispatch<SetStateAction<string>>;
  setUnreadSessions: Dispatch<SetStateAction<Set<string>>>;
  setApprovalTierState: Dispatch<SetStateAction<string>>;
  ensureDraftTimeline(): void;
  prepareTimelineOpen(sessionId: string): void;
  recordError(error: unknown): void;
  removeSession(sessionId: string, threadId: string): Promise<boolean>;
  removeWorkspace(workspaceId: string): Promise<boolean>;
  closeConversation(): void;
}

export function useConversationController({
  clientRef,
  selectedRef,
  syncEngineRef,
  conversationEpochRef,
  models,
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
}: ConversationControllerOptions) {
  const [modelId, setModelId] = useState("");
  const [thinkingLevel, setThinkingLevelState] = useState<ThinkingLevel>("off");
  const [openingSession, setOpeningSession] = useState(false);
  const settingsRevision = useRef(0);

  const applySessionSettings = useCallback((sessionId: string, state: Pick<RemoteSessionState, "model" | "thinkingLevel">) => {
    if (!sessionId || sessionId !== selectedRef.current) return;
    settingsRevision.current += 1;
    if (typeof state.model === "string") setModelId(state.model);
    if (state.thinkingLevel !== undefined) setThinkingLevelState(state.thinkingLevel);
  }, [selectedRef]);

  const handleSessionSettingsEvent = useCallback((event: StreamEvent, sessionId: string) => {
    if (event.type !== "model_changed" && event.type !== "thinking_level_changed") return;
    try {
      const data: unknown = JSON.parse(event.data);
      if (!data || typeof data !== "object") return;
      if (event.type === "model_changed" && "model" in data && typeof data.model === "string") {
        applySessionSettings(sessionId, { model: data.model });
      } else if ("level" in data && typeof data.level === "string"
        && ["off", "minimal", "low", "medium", "high", "xhigh"].includes(data.level)) {
        applySessionSettings(sessionId, { thinkingLevel: data.level as ThinkingLevel });
      }
    } catch { /* Ignore malformed notifications; the next state read recovers. */ }
  }, [applySessionSettings]);

  const selectSession = useCallback(
    async (sessionId: string) => {
      const client = clientRef.current;
      if (!client) return;
      setOpeningSession(true);
      const epoch = ++conversationEpochRef.current;
      const isCurrent = () => conversationEpochRef.current === epoch
        && selectedRef.current === sessionId && clientRef.current === client;
      prepareTimelineOpen(sessionId);
      setSelectedSessionId(sessionId);
      selectedRef.current = sessionId;
      client.setVisibleSession?.(sessionId);
      setDraft(false);
      setUnreadSessions(previous => {
        if (!previous.has(sessionId)) return previous;
        const next = new Set(previous);
        next.delete(sessionId);
        return next;
      });
      const revision = settingsRevision.current;
      try {
        const engine = syncEngineRef.current;
        const state = engine
          ? await engine.open(sessionId)
          : (await client.requestRetry<RemoteSessionState>({ type: "get_state", sessionId }, sessionId)).data;
        if (!isCurrent() || settingsRevision.current !== revision) return;
        const currentModel = state.model ?? "";
        const matchingModel = models.find(model => modelReference(model) === currentModel);
        setModelId(matchingModel ? modelReference(matchingModel) : currentModel);
        setThinkingLevelState(state.thinkingLevel ?? "off");
      } catch (nextError) {
        if (isCurrent()) recordError(nextError);
      } finally {
        if (isCurrent()) setOpeningSession(false);
      }
      // The engine's history request includes attachments. An extra hydration
      // here doubled history reads on warm opens and raced the paging cursor.
    },
    [
      clientRef,
      conversationEpochRef,
      models,
      prepareTimelineOpen,
      recordError,
      selectedRef,
      setDraft,
      setSelectedSessionId,
      setUnreadSessions,
      syncEngineRef,
    ],
  );

  const newConversation = useCallback(
    async (mode: "chat" | "workspace" = "chat", workspaceId = "") => {
      const epoch = ++conversationEpochRef.current;
      setOpeningSession(false);
      setSelectedSessionId("");
      selectedRef.current = "";
      setDraft(true);
      setDraftMode(mode);
      setDraftWorkspaceId(workspaceId);
      ensureDraftTimeline();
      const [lastModel, lastThinking] = await Promise.all([loadLastModel(), loadLastThinking()]);
      if (conversationEpochRef.current !== epoch || selectedRef.current !== "") return;
      const defaultOption = models.find(model => model.isDefault);
      const defaultModel =
        (lastModel && models.some(model => modelReference(model) === lastModel)
          ? lastModel
          : null) ??
        (defaultOption ? modelReference(defaultOption) : null) ??
        (models[0] ? modelReference(models[0]) : "");
      setModelId(defaultModel);
      setThinkingLevelState((lastThinking as ThinkingLevel | null) ?? "off");
    },
    [
      conversationEpochRef,
      ensureDraftTimeline,
      models,
      selectedRef,
      setDraft,
      setDraftMode,
      setDraftWorkspaceId,
      setSelectedSessionId,
    ],
  );

  const listSkills = useCallback(async () => {
    const client = clientRef.current;
    const epoch = conversationEpochRef.current;
    if (!client) throw new Error("skills_not_connected");
    const response = await client.request<{ skills: RemoteSkill[] }>({ type: "list_skills" });
    if (clientRef.current !== client || conversationEpochRef.current !== epoch) {
      throw new Error("skills_context_changed");
    }
    if (!Array.isArray(response.data.skills)) throw new Error("skills_invalid_response");
    return response.data.skills;
  }, [clientRef, conversationEpochRef]);

  const listSessionFiles = useCallback(async (path = "") => {
    const client = clientRef.current;
    const sessionId = selectedRef.current;
    const epoch = conversationEpochRef.current;
    if (!client || !sessionId) throw new Error("attachment_no_session");
    const response = await requestReadPage<SessionFileListing>(
      client,
      { type: "list_session_files", sessionId, filePath: path },
      sessionId,
      () => clientRef.current === client && selectedRef.current === sessionId
        && conversationEpochRef.current === epoch,
    );
    return response.data;
  }, [clientRef, selectedRef, conversationEpochRef]);

  const prepareAttachment = useCallback(
    async (
      attachment: HistoryAttachment,
      variant: "preview" | "original" = "preview",
      signal?: AbortSignal,
      onWaiting?: () => void,
    ) => {
      const client = clientRef.current;
      const sessionId = selectedRef.current;
      if (!client || !sessionId) throw new Error("attachment_no_session");
      const epoch = conversationEpochRef.current;
      const info = await prepareDownload(client, sessionId, attachment, variant, signal, onWaiting);
      if (clientRef.current !== client || selectedRef.current !== sessionId ||
          conversationEpochRef.current !== epoch) {
        void client.request({ type: "download_cancel", transferId: info.transferId }, "transfer").catch(() => undefined);
        throw new TransferCancelledError();
      }
      rememberPreparedPreview(client, sessionId, attachment, info);
      return info;
    },
    [clientRef, selectedRef, conversationEpochRef],
  );

  const cachedAttachment = useCallback(
    (attachment: HistoryAttachment, variant: "preview" | "original" = "preview") => {
      const client = clientRef.current;
      const sessionId = selectedRef.current;
      return client && sessionId ? cachedPreviewForAttachment(client, sessionId, attachment, variant) : null;
    },
    [clientRef, selectedRef],
  );

  const downloadAttachment = useCallback(
    async (
      info: DownloadInfo,
      onProgress?: (completedBytes: number, totalBytes: number) => void,
      signal?: AbortSignal,
      onWaiting?: () => void,
    ) => {
      const client = clientRef.current;
      if (!client) throw new Error("attachment_not_connected");
      return downloadPrepared(client, info, onProgress, signal, onWaiting);
    },
    [clientRef],
  );

  const abort = useCallback(async () => {
    const client = clientRef.current;
    const sessionId = selectedRef.current;
    if (!sessionId) return;
    // Do not acknowledge a stop that was never sent after a disconnect.
    if (!client) throw new Error("not_connected");
    await client.request({ type: "abort", sessionId }, sessionId);
  }, [clientRef, selectedRef]);

  const setModel = useCallback(
    async (nextModelId: string) => {
      const client = clientRef.current;
      const sessionId = selectedRef.current;
      settingsRevision.current += 1;
      setModelId(nextModelId);
      await saveLastModel(nextModelId);
      if (client && clientRef.current === client && sessionId) {
        await client.request(
          {
            type: "set_model",
            sessionId,
            modelId: nextModelId,
            providerId: modelProviderFromReference(nextModelId),
          },
          sessionId,
        );
      }
    },
    [clientRef, selectedRef],
  );

  const setThinkingLevel = useCallback(
    async (level: ThinkingLevel) => {
      const client = clientRef.current;
      const sessionId = selectedRef.current;
      settingsRevision.current += 1;
      setThinkingLevelState(level);
      await saveLastThinking(level);
      if (client && clientRef.current === client && sessionId) {
        await client.request(
          { type: "set_thinking_level", sessionId, level },
          sessionId,
        );
      }
    },
    [clientRef, selectedRef],
  );

  const setApprovalTier = useCallback(
    async (tier: string) => {
      const client = clientRef.current;
      if (!client) throw new Error("not_connected");
      const response = await client.request<{ approvalTier: string }>(
        { type: "set_approval_tier", tier },
        "list",
      );
      if (clientRef.current === client) setApprovalTierState(response.data.approvalTier);
    },
    [clientRef, setApprovalTierState],
  );

  const deleteSession = useCallback(
    async (sessionId: string, threadId: string) => {
      if (await removeSession(sessionId, threadId)) closeConversation();
    },
    [closeConversation, removeSession],
  );

  const deleteWorkspace = useCallback(
    async (workspaceId: string) => {
      // Deleting the workspace removes every thread inside it, so a
      // conversation the user is reading may be gone with it.
      if (await removeWorkspace(workspaceId)) closeConversation();
    },
    [closeConversation, removeWorkspace],
  );

  const decideApproval = useCallback(
    async (id: string, decision: "approved" | "rejected") => {
      const client = clientRef.current;
      const sessionId = selectedRef.current;
      if (!client || !sessionId) return;
      await client.request(
        { type: "approval_decision", sessionId, entryId: id, mode: decision },
        sessionId,
      );
      if (clientRef.current !== client) return;
      syncEngineRef.current?.mutate(sessionId, timeline =>
        markApprovalDecision(timeline, id, decision),
      );
    },
    [clientRef, selectedRef, syncEngineRef],
  );

  return {
    modelId,
    thinkingLevel,
    applySessionSettings,
    handleSessionSettingsEvent,
    openingSession,
    selectSession,
    newConversation,
    listSessionFiles,
    listSkills,
    prepareAttachment,
    cachedAttachment,
    downloadAttachment,
    abort,
    setModel,
    setThinkingLevel,
    setApprovalTier,
    deleteSession,
    deleteWorkspace,
    decideApproval,
  };
}
