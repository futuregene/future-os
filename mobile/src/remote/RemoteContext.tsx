import type { PropsWithChildren } from "react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { TimelineState } from "./timeline";
import { RemoteClient } from "./client";
import { useConversationController } from "./useConversationController";
import { useSessionCatalog } from "./useSessionCatalog";
import { usePromptOutbox } from "./usePromptOutbox";
import { useRemoteConnection } from "./useRemoteConnection";
import { useTimelineController } from "./useTimelineController";
import type { File } from "expo-file-system";
import type {
  ConnectionPhase,
  DownloadInfo,
  HistoryAttachment,
  MobileAttachment,
  Presence,
  RemoteCredentials,
  RemoteModel,
  RemoteSession,
  RemoteWorkspace,
  ThinkingLevel,
} from "./types";

interface RemoteContextValue {
  phase: ConnectionPhase;
  error: string | null;
  credentials: RemoteCredentials | null;
  presence: Presence | null;
  desktopOnline: boolean;
  sessions: RemoteSession[];
  workspaces: RemoteWorkspace[];
  unreadSessions: Set<string>;
  models: RemoteModel[];
  selectedSessionId: string;
  selectedTitle: string;
  draft: boolean;
  timeline: TimelineState;
  timelinePending: boolean;
  timelineError: "timeout" | null;
  canLoadOlderTimeline: boolean;
  loadingOlderTimeline: boolean;
  modelId: string;
  thinkingLevel: ThinkingLevel;
  approvalTier: string;
  sandboxAvailable: boolean;
  busy: boolean;
  fileTransferSupported: boolean;
  capabilities: Set<string>;
  pair(code: string): Promise<void>;
  reconnect(): Promise<void>;
  unpair(): Promise<void>;
  refreshSessions(): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  selectSession(sessionId: string): Promise<void>;
  retryTimeline(): Promise<void>;
  loadOlderTimeline(): Promise<void>;
  newConversation(mode?: "chat" | "workspace", workspaceId?: string): Promise<void>;
  closeConversation(): void;
  sendMessage(
    text: string,
    attachments?: MobileAttachment[],
    onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
  ): Promise<void>;
  prepareAttachment(
    attachment: HistoryAttachment,
    variant?: "preview" | "original",
    signal?: AbortSignal,
    onWaiting?: () => void,
  ): Promise<DownloadInfo>;
  cachedAttachment(
    attachment: HistoryAttachment,
    variant?: "preview" | "original",
  ): { info: DownloadInfo; file: File } | null;
  downloadAttachment(
    info: DownloadInfo,
    onProgress?: (completedBytes: number, totalBytes: number) => void,
    signal?: AbortSignal,
    onWaiting?: () => void,
  ): Promise<File>;
  abort(): Promise<void>;
  setModel(modelId: string): Promise<void>;
  setThinkingLevel(level: ThinkingLevel): Promise<void>;
  setApprovalTier(tier: string): Promise<void>;
  rename(sessionId: string, name: string): Promise<void>;
  deleteSession(sessionId: string, threadId: string): Promise<void>;
  setSessionPinned(sessionId: string, threadId: string, pinned: boolean): Promise<void>;
  decideApproval(id: string, decision: "approved" | "rejected"): Promise<void>;
  clearError(): void;
  continueRun(sessionId: string, runId: string): Promise<void>;
}

const RemoteContext = createContext<RemoteContextValue | null>(null);

/** Composes the remote domain controllers and preserves the public useRemote API. */
export function RemoteProvider({ children }: PropsWithChildren) {
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [draft, setDraft] = useState(false);
  const [draftMode, setDraftMode] = useState<"chat" | "workspace">("chat");
  const [draftWorkspaceId, setDraftWorkspaceId] = useState("");
  const clientRef = useRef<RemoteClient | null>(null);
  const credentialsRef = useRef<RemoteCredentials | null>(null);
  const selectedRef = useRef("");
  // The control-plane catalogue (sessions/workspaces/models/settings) lives in
  // its own hook; the provider keeps connection lifecycle + per-session
  // timeline and forwards the catalogue's state/setters into the context value.
  const {
    sessions,
    unreadSessions,
    setUnreadSessions,
    workspaces,
    setWorkspaces,
    models,
    approvalTier,
    setApprovalTier: setApprovalTierState,
    sandboxAvailable,
    setTitleOverrides,
    applySessionSnapshot,
    refreshSessions,
    refreshModels,
    refreshSettings,
    refreshWorkspaces,
    rename,
    deleteSession: removeSession,
    setSessionPinned,
    reset: resetCatalog,
  } = useSessionCatalog(clientRef, selectedRef);
  // Changes whenever the user navigates between conversations. Long uploads
  // capture the epoch so their eventual ack cannot pull the UI back to a
  // conversation the user has already left.
  const conversationEpochRef = useRef(0);
  useEffect(() => {
    selectedRef.current = selectedSessionId;
  }, [selectedSessionId]);
  const {
    timeline,
    timelinePending,
    timelineError,
    canLoadOlderTimeline,
    loadingOlderTimeline,
    loadOlderTimeline,
    syncEngineRef,
    streamingRef,
    hydrateAttachmentsRef,
    reconcileSession,
    handleEvent,
    applySessionStreaming,
    resetTimeline,
    ensureDraftTimeline,
    retryTimeline,
  } = useTimelineController({
    clientRef,
    selectedRef,
    selectedSessionId,
    draft,
    refreshModels,
    refreshSessions,
    setTitleOverrides,
  });

  const closeConversation = useCallback(() => {
    conversationEpochRef.current += 1;
    setSelectedSessionId("");
    selectedRef.current = "";
    setDraft(false);
    setDraftMode("chat");
    setDraftWorkspaceId("");
    // Keep per-session timeline caches so reopening renders live state.
    void refreshSessions();
    void refreshWorkspaces();
  }, [refreshSessions, refreshWorkspaces]);
  const resetConversation = useCallback(() => {
    selectedRef.current = "";
    setSelectedSessionId("");
    setDraft(false);
    setDraftMode("chat");
    setDraftWorkspaceId("");
  }, []);

  const recoverRemoteState = useCallback(
    async (sessionId?: string) => {
      await Promise.allSettled([
        refreshModels(),
        refreshSessions(),
        refreshWorkspaces(),
        refreshSettings(),
      ]);
      reconcileSession(sessionId, "reconnect");
    },
    [reconcileSession, refreshModels, refreshSessions, refreshSettings, refreshWorkspaces],
  );

  const {
    phase,
    error,
    credentials,
    presence,
    desktopOnline,
    capabilities,
    fileTransferSupported,
    promptReceiptSupported,
    recordError,
    pair,
    reconnect,
    unpair,
    clearError,
  } = useRemoteConnection({
    clientRef,
    credentialsRef,
    selectedRef,
    syncEngineRef,
    handleEvent,
    reconcileSession,
    recoverRemoteState,
    applySessionSnapshot,
    applySessionStreaming,
    setWorkspaces,
    refreshModels,
    refreshSessions,
    refreshSettings,
    refreshWorkspaces,
    closeConversation,
    resetConversation,
    resetCatalog,
    resetTimeline,
  });
  const {
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
  } = useConversationController({
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
  });

  const { sending, sendMessage, continueRun } = usePromptOutbox({
    clientRef,
    credentialsRef,
    selectedRef,
    streamingRef,
    conversationEpochRef,
    syncEngineRef,
    phase,
    draft,
    draftMode,
    draftWorkspaceId,
    modelId,
    thinkingLevel,
    fileTransferSupported,
    promptReceiptSupported,
    setSelectedSessionId,
    setDraft,
    setDraftMode,
    setDraftWorkspaceId,
    refreshSessions,
    reconcileSession,
    recordError,
  });
  const selectedTitle =
    sessions.find(session => session.sessionId === selectedSessionId)?.title ?? "";

  const value = useMemo<RemoteContextValue>(
    () => ({
      phase,
      error,
      credentials,
      presence,
      desktopOnline,
      sessions,
      workspaces,
      unreadSessions,
      models,
      selectedSessionId,
      selectedTitle,
      draft,
      timeline,
      timelinePending,
      timelineError,
      canLoadOlderTimeline,
      loadingOlderTimeline,
      modelId,
      thinkingLevel,
      approvalTier,
      sandboxAvailable,
      busy: sending || openingSession,
      fileTransferSupported,
      capabilities,
      pair,
      reconnect,
      unpair,
      refreshSessions,
      refreshWorkspaces,
      selectSession,
      retryTimeline,
      loadOlderTimeline,
      newConversation,
      closeConversation,
      sendMessage,
      prepareAttachment,
      cachedAttachment,
      downloadAttachment,
      abort,
      setModel,
      setThinkingLevel,
      setApprovalTier,
      rename,
      deleteSession,
      setSessionPinned,
      decideApproval,
      clearError,
      continueRun,
    }),
    [
      abort,
      openingSession,
      sending,
      capabilities,
      fileTransferSupported,
      credentials,
      closeConversation,
      clearError,
      continueRun,
      decideApproval,
      desktopOnline,
      deleteSession,
      draft,
      error,
      modelId,
      models,
      newConversation,
      pair,
      phase,
      presence,
      reconnect,
      refreshSessions,
      refreshWorkspaces,
      rename,
      selectSession,
      selectedSessionId,
      selectedTitle,
      sendMessage,
      setSessionPinned,
      prepareAttachment,
      cachedAttachment,
      downloadAttachment,
      sessions,
      timeline,
      timelineError,
      timelinePending,
      canLoadOlderTimeline,
      loadingOlderTimeline,
      unreadSessions,
      workspaces,
      setModel,
      setThinkingLevel,
      setApprovalTier,
      thinkingLevel,
      approvalTier,
      sandboxAvailable,
      unpair,
      retryTimeline,
      loadOlderTimeline,
    ],
  );

  return <RemoteContext.Provider value={value}>{children}</RemoteContext.Provider>;
}

export function useRemote(): RemoteContextValue {
  const value = useContext(RemoteContext);
  if (!value) throw new Error("useRemote must be used inside RemoteProvider");
  return value;
}
