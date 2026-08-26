import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback, useState } from "react";
import type { RemoteClient } from "./client";
import {
  cachedPreviewForAttachment,
  downloadPrepared,
  prepareDownload,
  rememberPreparedPreview,
} from "./files";
import type { SyncEngine } from "./syncEngine";
import { loadLastModel, loadLastThinking, saveLastModel, saveLastThinking } from "./storage";
import { markApprovalDecision } from "./timeline";
import { modelProviderFromReference, modelReference } from "./types";
import type {
  DownloadInfo,
  HistoryAttachment,
  RemoteModel,
  RemoteSessionState,
  ThinkingLevel,
} from "./types";

interface ConversationControllerOptions {
  clientRef: MutableRefObject<RemoteClient | null>;
  selectedRef: MutableRefObject<string>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  hydrateAttachmentsRef: MutableRefObject<(sessionId: string) => Promise<void>>;
  conversationEpochRef: MutableRefObject<number>;
  models: RemoteModel[];
  setSelectedSessionId: Dispatch<SetStateAction<string>>;
  setDraft: Dispatch<SetStateAction<boolean>>;
  setDraftMode: Dispatch<SetStateAction<"chat" | "workspace">>;
  setDraftWorkspaceId: Dispatch<SetStateAction<string>>;
  setUnreadSessions: Dispatch<SetStateAction<Set<string>>>;
  setApprovalTierState: Dispatch<SetStateAction<string>>;
  ensureDraftTimeline(): void;
  recordError(error: unknown): void;
  removeSession(sessionId: string, threadId: string): Promise<boolean>;
  closeConversation(): void;
}

export function useConversationController({
  clientRef,
  selectedRef,
  syncEngineRef,
  hydrateAttachmentsRef,
  conversationEpochRef,
  models,
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
}: ConversationControllerOptions) {
  const [modelId, setModelId] = useState("");
  const [thinkingLevel, setThinkingLevelState] = useState<ThinkingLevel>("off");
  const [openingSession, setOpeningSession] = useState(false);

  const selectSession = useCallback(
    async (sessionId: string) => {
      const client = clientRef.current;
      if (!client) return;
      setOpeningSession(true);
      conversationEpochRef.current += 1;
      setSelectedSessionId(sessionId);
      selectedRef.current = sessionId;
      setDraft(false);
      setUnreadSessions(previous => {
        if (!previous.has(sessionId)) return previous;
        const next = new Set(previous);
        next.delete(sessionId);
        return next;
      });
      try {
        const state = await client.requestRetry<RemoteSessionState>(
          { type: "get_state", sessionId },
          sessionId,
        );
        const currentModel = state.data.model ?? "";
        const matchingModel = models.find(model => modelReference(model) === currentModel);
        setModelId(matchingModel ? modelReference(matchingModel) : currentModel);
        setThinkingLevelState(state.data.thinkingLevel ?? "off");
      } catch (nextError) {
        recordError(nextError);
      } finally {
        setOpeningSession(false);
      }
      syncEngineRef.current?.reconcile(sessionId, "open");
      void hydrateAttachmentsRef.current(sessionId);
    },
    [
      clientRef,
      conversationEpochRef,
      hydrateAttachmentsRef,
      models,
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
      const [lastModel, lastThinking] = await Promise.all([loadLastModel(), loadLastThinking()]);
      const defaultOption = models.find(model => model.isDefault);
      const defaultModel =
        (lastModel && models.some(model => modelReference(model) === lastModel)
          ? lastModel
          : null) ??
        (defaultOption ? modelReference(defaultOption) : null) ??
        (models[0] ? modelReference(models[0]) : "");
      conversationEpochRef.current += 1;
      setSelectedSessionId("");
      selectedRef.current = "";
      setDraft(true);
      setDraftMode(mode);
      setDraftWorkspaceId(workspaceId);
      ensureDraftTimeline();
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
      const info = await prepareDownload(client, sessionId, attachment, variant, signal, onWaiting);
      rememberPreparedPreview(attachment, info);
      return info;
    },
    [clientRef, selectedRef],
  );

  const cachedAttachment = useCallback(
    (attachment: HistoryAttachment, variant: "preview" | "original" = "preview") =>
      cachedPreviewForAttachment(attachment, variant),
    [],
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
    if (!client || !selectedRef.current) return;
    await client.request({ type: "abort", sessionId: selectedRef.current }, selectedRef.current);
  }, [clientRef, selectedRef]);

  const setModel = useCallback(
    async (nextModelId: string) => {
      setModelId(nextModelId);
      await saveLastModel(nextModelId);
      const client = clientRef.current;
      if (client && selectedRef.current) {
        await client.request(
          {
            type: "set_model",
            sessionId: selectedRef.current,
            modelId: nextModelId,
            providerId: modelProviderFromReference(nextModelId),
          },
          selectedRef.current,
        );
      }
    },
    [clientRef, selectedRef],
  );

  const setThinkingLevel = useCallback(
    async (level: ThinkingLevel) => {
      setThinkingLevelState(level);
      await saveLastThinking(level);
      const client = clientRef.current;
      if (client && selectedRef.current) {
        await client.request(
          { type: "set_thinking_level", sessionId: selectedRef.current, level },
          selectedRef.current,
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
      setApprovalTierState(response.data.approvalTier);
    },
    [clientRef, setApprovalTierState],
  );

  const deleteSession = useCallback(
    async (sessionId: string, threadId: string) => {
      if (await removeSession(sessionId, threadId)) closeConversation();
    },
    [closeConversation, removeSession],
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
      syncEngineRef.current?.mutate(sessionId, timeline =>
        markApprovalDecision(timeline, id, decision),
      );
    },
    [clientRef, selectedRef, syncEngineRef],
  );

  return {
    modelId,
    thinkingLevel,
    openingSession,
    selectSession,
    newConversation,
    prepareAttachment,
    cachedAttachment,
    downloadAttachment,
    abort,
    setModel,
    setThinkingLevel,
    setApprovalTier,
    deleteSession,
    decideApproval,
  };
}
