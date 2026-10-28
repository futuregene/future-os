import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { RecoveryReason, RemoteClient } from "./client";
import { captureConversationLifetime } from "./conversationLifetime";
import { notifyTaskFinished } from "../notifications/taskNotifications";
import { useConversationController } from "./useConversationController";
import { useDesktopConnection } from "./useDesktopConnection";
import type { DesktopConnection } from "./useDesktopConnection";
import { useDesktopManagement } from "./useDesktopManagement";
import { usePromptOutbox } from "./usePromptOutbox";
import { useSessionCatalog } from "./useSessionCatalog";
import { useTimelineController } from "./useTimelineController";
import { DEFAULT_THINKING_LEVEL } from "./types";
import type {
  RemoteCredentials,
  RemoteSession,
  RemoteSessionState,
  StreamEvent,
} from "./types";

/**
 * Everything the phone knows about *one* paired desktop: its connection, its
 * catalogue, its conversations, its timelines and its outbox.
 *
 * The phone used to hold a single runtime and hop it onto whichever desktop was
 * selected, which meant a tap on a row from another machine closed the working
 * connection, handshook a new one, and threw away every mirrored catalogue and
 * timeline on the way — so the conversation the user asked for appeared only
 * after the connection did, and going back cost the same again. A runtime per
 * desktop removes the hop: each paired machine keeps its own connection open,
 * its own catalogue current and its own open conversation warm, and "switching"
 * is choosing which of them the screens read from.
 *
 * Per desktop, not per connection: the state here survives a transport fault.
 * The connection inside it is one generation of one socket, and everything
 * about a *conversation* — what is open, what was typed into it, what has been
 * read — belongs to the machine that owns the conversation, which outlives any
 * particular socket.
 *
 * This hook is called once per paired desktop, by a host component per desktop
 * (`RemoteContext` renders them). It is the same hook in the same order as the
 * old single-runtime provider, which is what makes N of them a pure widening:
 * the per-conversation state inside was always keyed by session id, and a
 * session id is a desktop's own.
 */
export interface DesktopRuntimeOptions {
  desktopId: string;
  /** Whether the screens are reading this desktop. */
  active: boolean;
  /** Whether the app is in front. A backgrounded phone reads nothing. */
  foreground: boolean;
  /** The user's own name for the machine, when they have given it one. */
  label?: string;
  /**
   * The desktop ended this pairing (see `DesktopConnectionOptions.onUnpaired`).
   * The provider removes the pairing; nothing in this runtime can survive it,
   * because being paired is what mounts it.
   */
  onUnpaired?(): void;
  /** See `DesktopConnectionOptions.verifyPairing`. */
  verifyPairing?(credentials: RemoteCredentials): Promise<unknown>;
}

export interface DesktopRuntime {
  desktopId: string;
  label: string;
  connection: DesktopConnection;
  catalog: ReturnType<typeof useSessionCatalog>;
  timeline: ReturnType<typeof useTimelineController>;
  conversation: ReturnType<typeof useConversationController>;
  outbox: ReturnType<typeof usePromptOutbox>;
  management: ReturnType<typeof useDesktopManagement>;
  selectedSessionId: string;
  selectedTitle: string;
  draft: boolean;
  draftMode: "chat" | "workspace";
  draftWorkspaceId: string;
  streaming: boolean;
  compacting: boolean;
  busy: boolean;
  /** Snapshot navigation/connection ownership before starting async UI work. */
  captureConversationOperation(): () => boolean;
  closeConversation(): void;
  setVisibleSession(sessionId: string): void;
  setAppActive(active: boolean): void;
  /** Best-effort "this phone is leaving" notice, bounded by the caller. */
  notifyUnpair(): Promise<void>;
  /** Bumped by the desktop's own settings/skills change notices. */
  desktopSettingsRevision: number;
  skillsRevision: number;
}

/** Per-desktop, per-render state the catalogue needs to know about a run. */
export function useDesktopRuntime({
  desktopId,
  active,
  foreground,
  label,
  onUnpaired,
  verifyPairing,
}: DesktopRuntimeOptions): DesktopRuntime {
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [draft, setDraft] = useState(false);
  const [draftMode, setDraftMode] = useState<"chat" | "workspace">("chat");
  const [draftWorkspaceId, setDraftWorkspaceId] = useState("");
  const clientRef = useRef<RemoteClient | null>(null);
  const credentialsRef = useRef<RemoteCredentials | null>(null);
  const selectedRef = useRef("");
  const conversationEpochRef = useRef(0);
  const onTaskFinished = useCallback((session: RemoteSession) => {
    const pairing = credentialsRef.current?.pairId;
    if (!pairing) return;
    // The machine is named in the notification because two desktops can each
    // have a conversation with the same title, and it is named by id because a
    // tap on the reminder has to open the conversation on *this* machine: the
    // phone may be reading another one by then.
    void notifyTaskFinished(session, () => credentialsRef.current?.pairId === pairing, {
      machine: label,
      desktopId,
    });
  }, [desktopId, label]);

  const catalog = useSessionCatalog(clientRef, selectedRef, onTaskFinished);
  const {
    sessions,
    setTitleOverrides,
    refreshModels,
    refreshSessions,
    refreshWorkspaces,
    applySessionSnapshot,
    applySessionActivity,
    forgetCatalogue,
    setWorkspaces,
    setCatalogEpoch,
    noteCatalogRevisions,
    observeRunEvent,
    refreshSettings,
    reset: resetCatalog,
    setApprovalTier: setApprovalTierState,
    models,
  } = catalog;

  // Connection callbacks are created before the conversation controller. Keep
  // their settings sink current without rebuilding the connection/sync engine.
  const settingsSink = useRef<Pick<ReturnType<typeof useConversationController>,
    "applySessionSettings" | "handleSessionSettingsEvent"> | null>(null);
  const desktopSettingsRevisionSink = useRef<() => void>(() => undefined);
  const skillsRevisionSink = useRef<() => void>(() => undefined);
  const onSessionState = useCallback((sessionId: string, state: RemoteSessionState) => {
    settingsSink.current?.applySessionSettings(sessionId, {
      model: state.model ?? "",
      thinkingLevel: state.thinkingLevel ?? DEFAULT_THINKING_LEVEL,
      usage: state.usage,
    });
  }, []);

  // Set after the connection exists, read by the timeline controller's retry.
  const recoverConnectionRef = useRef<((reason: RecoveryReason) => Promise<void>) | null>(null);
  const timelineController = useTimelineController({
    clientRef,
    selectedRef,
    selectedSessionId,
    draft,
    refreshModels,
    refreshSessions,
    setTitleOverrides,
    onSessionState,
    // Built before the connection (the controller owns the sync engine the
    // connection drives), so the runtime's recovery reaches it through a ref:
    // a retry on a fatal client must be able to replace that client, which
    // `client.recoverNow` alone can never do.
    recoverConnection: (reason) => recoverConnectionRef.current?.(reason) ?? Promise.resolve(),
  });
  const {
    timeline,
    prepareTimelineOpen,
    syncEngineRef,
    compactingRef,
    reconcileSession,
    handleEvent,
    resetTimeline,
    ensureDraftTimeline,
    applySessionStreaming,
  } = timelineController;

  const handleLiveEvent = useCallback((event: StreamEvent, sessionId: string) => {
    if (event.type === "app_settings_changed") {
      desktopSettingsRevisionSink.current();
      void refreshSettings();
      return;
    }
    if (event.type === "skills_changed") {
      skillsRevisionSink.current();
      return;
    }
    if (event.type === "provider_config_changed" || event.type === "model_visibility_changed")
      desktopSettingsRevisionSink.current();
    settingsSink.current?.handleSessionSettingsEvent(event, sessionId);
    observeRunEvent(event, sessionId);
    handleEvent(event, sessionId);
  }, [handleEvent, observeRunEvent, refreshSettings]);

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
    conversationEpochRef.current += 1;
    selectedRef.current = "";
    setSelectedSessionId("");
    setDraft(false);
    setDraftMode("chat");
    setDraftWorkspaceId("");
  }, []);

  /**
   * Control of this machine moved, so its catalogue is not this phone's any
   * more — or is again.
   *
   * A machine another device drives refuses this client every read and is sent
   * no snapshot, so the conversations mirrored here are the ones of a machine
   * somebody else is using: they come out of the merged list at once instead of
   * lingering as rows that cannot be opened. When control returns, the same
   * emptiness would read as "no conversations": the host sends no snapshot to a
   * standby device, so this side reads the catalogue itself rather than waiting
   * for a push that will not come.
   */
  const controlChanged = useCallback((control: "active" | "standby") => {
    if (control === "standby") {
      forgetCatalogue();
      return;
    }
    void refreshSessions();
    void refreshWorkspaces();
  }, [forgetCatalogue, refreshSessions, refreshWorkspaces]);

  const connection = useDesktopConnection({
    desktopId,
    clientRef,
    credentialsRef,
    selectedRef,
    syncEngineRef,
    handleEvent: handleLiveEvent,
    reconcileSession,
    setCatalogEpoch,
    applySessionSnapshot,
    applySessionActivity,
    applySessionStreaming,
    setWorkspaces,
    refreshModels,
    refreshSessions,
    refreshSettings,
    refreshWorkspaces,
    noteCatalogRevisions,
    controlChanged,
    onUnpaired,
    verifyPairing,
    closeConversation,
    resetConversation,
    resetCatalog,
    resetTimeline,
  });
  const { phase, presence, desktopOnline } = connection;
  useEffect(() => {
    recoverConnectionRef.current = connection.recover;
  }, [connection.recover]);

  const conversation = useConversationController({
    clientRef,
    selectedRef,
    syncEngineRef,
    conversationEpochRef,
    models,
    setSelectedSessionId,
    setDraft,
    setDraftMode,
    setDraftWorkspaceId,
    setUnreadSessions: catalog.setUnreadSessions,
    setApprovalTierState,
    ensureDraftTimeline,
    prepareTimelineOpen,
    recordError: connection.recordError,
    removeSession: catalog.deleteSession,
    removeWorkspace: catalog.deleteWorkspace,
    refreshSessions,
    closeConversation,
  });

  useEffect(() => {
    settingsSink.current = {
      applySessionSettings: conversation.applySessionSettings,
      handleSessionSettingsEvent: conversation.handleSessionSettingsEvent,
    };
    return () => { settingsSink.current = null; };
  }, [conversation.applySessionSettings, conversation.handleSessionSettingsEvent]);

  const outbox = usePromptOutbox({
    clientRef,
    credentialsRef,
    selectedRef,
    compactingRef,
    conversationEpochRef,
    syncEngineRef,
    phase,
    businessReady: desktopOnline && presence?.agentAvailable !== false && !presence?.disconnected,
    draft,
    draftMode,
    draftWorkspaceId,
    modelId: conversation.modelId,
    thinkingLevel: conversation.thinkingLevel,
    fileTransferSupported: connection.fileTransferSupported,
    promptReceiptSupported: connection.promptReceiptSupported,
    setSelectedSessionId,
    setDraft,
    setDraftMode,
    setDraftWorkspaceId,
    refreshSessions,
    reconcileSession,
    recordError: connection.recordError,
  });

  const management = useDesktopManagement(clientRef);

  const captureConversationOperation = useCallback(
    () => captureConversationLifetime({ conversationEpochRef, clientRef, credentialsRef }),
    [],
  );

  /**
   * What this desktop is asked to detail, and whether its socket is held.
   *
   * Only the desktop the user is reading declares a visible session: the
   * others keep the low-rate notices, which is what makes N open connections
   * affordable. Their timelines are not stale for it — the phone retains each
   * conversation's timeline, and the desktop replays it against durable history
   * when the session becomes visible again.
   */
  const selectedForVisibility = useMemo(
    () => (active && foreground ? selectedSessionId : ""),
    [active, foreground, selectedSessionId],
  );
  const setVisibleSession = connection.setVisibleSession;
  useEffect(() => {
    setVisibleSession(selectedForVisibility);
  }, [selectedForVisibility, setVisibleSession]);

  /**
   * The open conversation's title, which outlives the row it came from.
   *
   * A machine taken over drops its catalogue (see `controlChanged`), and the
   * conversation stays on screen — it has its own standby notice, and its
   * transcript is still the user's to read. Losing the row must not rename the
   * conversation to "unnamed" under them, so the last title seen for the open
   * session is kept until the catalogue comes back or another session opens.
   */
  const [lastTitle, setLastTitle] = useState<{ sessionId: string; title: string }>({
    sessionId: "",
    title: "",
  });
  const selectedTitleRow = sessions.find(session => session.sessionId === selectedSessionId);
  // Adjusted during render rather than in an effect (the same pattern the
  // sessions screen uses for state derived from props): the title has to be
  // remembered before the row that carried it goes, or the conversation is
  // renamed for one frame.
  if (
    selectedSessionId
    && selectedTitleRow?.title
    && (lastTitle.sessionId !== selectedSessionId || lastTitle.title !== selectedTitleRow.title)
  )
    setLastTitle({ sessionId: selectedSessionId, title: selectedTitleRow.title });
  const selectedTitle = selectedTitleRow?.title
    || (lastTitle.sessionId === selectedSessionId ? lastTitle.title : "");

  const [desktopSettingsRevision, setDesktopSettingsRevision] = useState(0);
  const [skillsRevision, setSkillsRevision] = useState(0);
  useEffect(() => {
    desktopSettingsRevisionSink.current = () => setDesktopSettingsRevision(value => value + 1);
    skillsRevisionSink.current = () => setSkillsRevision(value => value + 1);
  }, []);

  const streaming = timeline.streaming;
  const compacting = timeline.compacting === true;
  return useMemo<DesktopRuntime>(() => ({
    desktopId,
    label: label ?? desktopId,
    connection,
    catalog,
    timeline: timelineController,
    conversation,
    outbox,
    management,
    selectedSessionId,
    selectedTitle,
    draft,
    draftMode,
    draftWorkspaceId,
    streaming,
    compacting,
    busy: outbox.sending || conversation.openingSession,
    captureConversationOperation,
    closeConversation,
    setVisibleSession: connection.setVisibleSession,
    setAppActive: connection.setAppActive,
    notifyUnpair: connection.notifyUnpair,
    desktopSettingsRevision,
    skillsRevision,
  }), [
    captureConversationOperation,
    catalog,
    closeConversation,
    compacting,
    connection,
    conversation,
    desktopId,
    desktopSettingsRevision,
    draft,
    draftMode,
    draftWorkspaceId,
    label,
    management,
    outbox,
    selectedSessionId,
    selectedTitle,
    skillsRevision,
    streaming,
    timelineController,
  ]);
}
