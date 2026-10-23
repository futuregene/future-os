import type { PaletteCommand, PaletteTarget } from "../../features/palette/paletteItems";
import type { DeviceFilter, MergedConversation, RemoteCatalog, RemoteWorkspaceTarget } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer, RemoteWorkspaceRow } from "../../features/remote-peer/remotePeerClient";
import type { SettingsTab } from "../../features/settings/SettingsDialog";
import type { VibeThreadViewProps } from "../../features/vibe/VibeStage";
import type { FutureAuthState, ProvidersView } from "../../integrations/agent/providers";
import type { StoredApprovalRequest, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ActivitySection } from "./ActivityRail";
import type { ContextTab } from "./ContextPanel";
import { Monitor } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentThread } from "../../features/agent/AgentThread";
import { saveComposerDraft } from "../../features/agent/composerDraft";
import { NewConversation } from "../../features/agent/NewConversation";
import { sessionMentionOptions } from "../../features/agent/sessionMention";
import { useSkillRecommendation } from "../../features/agent/useSkillRecommendation";
import { COMMANDS, isCommandId } from "../../features/keyboard/catalog";
import { runCommand, useCommand } from "../../features/keyboard/commands";
import { useShortcutDispatcher } from "../../features/keyboard/useDispatcher";
import { CommandPalette } from "../../features/palette/CommandPalette";
import { ShortcutCheatSheet } from "../../features/palette/ShortcutCheatSheet";
import { DeviceChoiceDialog } from "../../features/remote-peer/DeviceChoiceDialog";
import { peerDrivable, peerStatus } from "../../features/remote-peer/peerStatus";
import { RemoteComposer } from "../../features/remote-peer/RemoteComposer";
import { RemoteConversationSettings } from "../../features/remote-peer/RemoteConversationSettings";
import { RemoteConversationView } from "../../features/remote-peer/RemoteConversationView";
import { RemoteDraftTargetChooser } from "../../features/remote-peer/RemoteDraftTargetChooser";
import { RemoteFilesPanel } from "../../features/remote-peer/RemoteFilesPanel";
import { compactRemoteConversation, deleteRemoteConversation, forkRemoteConversation, installRemoteSkill, pinRemoteConversation, takeoverRemotePeer } from "../../features/remote-peer/remotePeerClient";
import { sendRemoteContinuation } from "../../features/remote-peer/remotePromptOutbox";
import { RemoteRenameDialog } from "../../features/remote-peer/RemoteRenameDialog";
import { remoteSkillRecoSource } from "../../features/remote-peer/remoteSkillRecoSource";
import { RemoteUsageDialog } from "../../features/remote-peer/RemoteUsageDialog";
import { RemoteWorkspaceCreateDialog } from "../../features/remote-peer/RemoteWorkspaceCreateDialog";
import { RemoteWorkspaceRenameDialog } from "../../features/remote-peer/RemoteWorkspaceRenameDialog";
import { useRemoteApprovals } from "../../features/remote-peer/useRemoteApprovals";
import { useRemoteConversationSettings } from "../../features/remote-peer/useRemoteConversationSettings";
import { useRemoteSkillCatalog } from "../../features/remote-peer/useRemoteSkillCatalog";
import { useRemoteSkillRecommend } from "../../features/remote-peer/useRemoteSkillRecommend";
import { useRemoteTimeline } from "../../features/remote-peer/useRemoteTimeline";
import { useRemoteWorkspaceRows } from "../../features/remote-peer/useRemoteWorkspaces";
import { WorkspaceDeviceDialog } from "../../features/remote-peer/WorkspaceDeviceDialog";
import { startRemote, stopRemote } from "../../features/remote/remoteClient";
import { RemoteHubView } from "../../features/remote/RemoteHubView";
import { SettingsDialog } from "../../features/settings/SettingsDialog";
import { SkillsView } from "../../features/skills/SkillsView";
import { TasksView } from "../../features/tasks/TasksView";
import { terminalTarget } from "../../features/terminal/panelTarget";
import { TerminalPanel } from "../../features/terminal/TerminalPanel";
import { TerminalToggleButton } from "../../features/terminal/TerminalToggleButton";
import { useTerminalPanel } from "../../features/terminal/useTerminalPanel";
import { exitNativeFullscreen } from "../../features/vibe/fullscreen";
import { VibeStage } from "../../features/vibe/VibeStage";
import { defaultAgentModelId, modelOption, readLastUsedModel } from "../../integrations/agent/agentClient";
import { installAgentEventListener, revalidateAgentState } from "../../integrations/agent/agentStateCache";
import { getFutureEnvironment } from "../../integrations/agent/providers";
import { refreshSkills } from "../../integrations/skills/skillsClient";
import { openExternalUrl } from "../../integrations/storage/files";
import {
  createWorkspace,
  markThreadOpened,
  pinThread,
  pinWorkspace,
  restoreThread,
} from "../../integrations/storage/threadStore";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { ToastHost } from "../ui/ToastHost";
import { ActivityRail } from "./ActivityRail";
import { AgentStatusGate } from "./AgentStatusGate";
import { AppShellDialogs } from "./AppShellDialogs";
import { ContextPanel } from "./ContextPanel";
import {
  defaultNewConversationTarget,
  readNewConversationTarget,
} from "./hooks/newConversationTarget";
import { canShowLeftPanel, canShowRightPanel, MIN_LEFT_PANEL_WIDTH } from "./hooks/panelGeometry";
import { useAgentConnection } from "./hooks/useAgentConnection";
import { useAgentDoneBell } from "./hooks/useAgentDoneBell";
import { useAgentStatus } from "./hooks/useAgentStatus";
import { useApprovals } from "./hooks/useApprovals";
import { useAppSettings } from "./hooks/useAppSettings";
import { useAppStartup } from "./hooks/useAppStartup";
import { useAutoUpgradeSkills } from "./hooks/useAutoUpgradeSkills";
import { useFutureAccount } from "./hooks/useFutureAccount";
import { useHasProviders } from "./hooks/useHasProviders";
import { useLeftPanelWidth } from "./hooks/useLeftPanelWidth";
import { useModelSelection } from "./hooks/useModelSelection";
import { useNewConversation } from "./hooks/useNewConversation";
import { useRemoteDoneBell } from "./hooks/useRemoteDoneBell";
import { useRemotePeers } from "./hooks/useRemotePeers";
import { useRemoteStatus } from "./hooks/useRemoteStatus";
import { useRightPanelWidth } from "./hooks/useRightPanelWidth";
import { useThreadDialogs } from "./hooks/useThreadDialogs";
import { useThreadStore } from "./hooks/useThreadStore";
import { useUnreadThreads } from "./hooks/useUnreadThreads";
import { useUpdateChecker } from "./hooks/useUpdateChecker";
import { useWindowWidth } from "./hooks/useWindowWidth";
import { useWorkspaceDialogs } from "./hooks/useWorkspaceDialogs";
import { MachineBadge } from "./MachineBadge";
import { OnboardingGate } from "./OnboardingGate";
import { WorkspaceBadge } from "./WorkspaceBadge";
import { WorkspaceDialogs } from "./WorkspaceDialogs";

export type { AgentConnectionState } from "./hooks/useAgentConnection";

interface WorkspaceCreateRequest {
  name?: string | null;
  path: string;
  createDirectory: boolean;
}

/**
 * What the open remote conversation's header says about it, from the catalogue
 * the rows are built from.
 *
 * The title falls back to the session id rather than to an empty header: a
 * conversation the catalogue has not caught up with yet is still *open*, and a
 * blank title would read as a broken view instead of a pending read. The
 * workspace is the one the row was grouped under in the list — a name, not an
 * id, so a host that does not report it leaves the qualifier off rather than
 * drawing something the reader cannot recognise. A chat conversation has no
 * workspace of its own, so it gets none.
 */
function contextOfRemote(
  catalogs: RemoteCatalog[],
  target: { desktopId: string; sessionId: string },
): { title: string; workspaceName: string | null } {
  const session = catalogs
    .find(catalog => catalog.desktopId === target.desktopId)
    ?.sessions
    .find(session => session.sessionId === target.sessionId);
  return {
    title: session?.title ?? target.sessionId,
    workspaceName: session?.mode === "workspace" ? (session.workspaceName ?? null) : null,
  };
}

export function AppShell() {
  const agentStatus = useAgentStatus();
  if ((!agentStatus.admitted && agentStatus.phase !== "ready")
    || agentStatus.phase === "stopping"
    || agentStatus.phase === "restart_required"
    || agentStatus.phase === "incompatible") {
    return <AgentStatusGate showWait={agentStatus.showWait} status={agentStatus} />;
  }
  return <StartingAppShell agentStatus={agentStatus} />;
}

function StartingAppShell({ agentStatus }: { agentStatus: ReturnType<typeof useAgentStatus> }) {
  const startup = useAppStartup(true);
  if (startup.phase === "pending")
    return <AgentStatusGate showWait={agentStatus.showWait} status={{ ...agentStatus, phase: "starting" }} />;
  if (startup.phase === "failed")
    return <AgentStatusGate status={{ ...agentStatus, phase: "unavailable" }} />;
  return <ReadyAppShell initialAuth={startup.auth} initialProviders={startup.providers} />;
}

function ReadyAppShell({
  initialAuth,
  initialProviders,
}: {
  initialAuth: FutureAuthState;
  initialProviders: ProvidersView;
}) {
  const { t } = useTranslation("layout");
  // The command names live in the shortcuts namespace — the same strings the
  // settings page shows, so the palette cannot rename a command the settings page
  // calls something else.
  const { t: tShortcuts } = useTranslation("shortcuts");
  // The remote peer's own namespace: its strings describe another machine, and
  // folding them into `layout` would put them where no other remote string is.
  const { t: tRemotePeer } = useTranslation("remotePeer");
  // The local new-conversation screen's words, for its remote twin: a draft on
  // another machine is titled and described exactly as this machine's is, and
  // its header reads "新对话" for the same reason.
  const { t: tAgent } = useTranslation("agent");
  const [section, setSection] = useState<ActivitySection>("chat");
  const [centerMode, setCenterMode] = useState<"thread" | "new-chat">("thread");
  /**
   * Whether the Vibe panel has the screen.
   *
   * Shell state rather than a route: the panel replaces the shell's columns but
   * not the shell, so the store, the run statuses, the approval queue and the
   * remote polling keep running behind it — that shared state is the whole reason
   * the panel is a view here instead of a window of its own.
   */
  const [vibeOpen, setVibeOpen] = useState(false);
  /**
   * Close the panel — and leave native fullscreen with it.
   *
   * Every way out of the panel goes through here (its own button and `Esc`, the
   * `vibe.toggle` accelerator, and the three "open it in the shell" routes), because
   * the panel is the only surface that offers fullscreen at all: its bar's icon, its
   * menu row and `vibe.fullscreen` are registered with it, so a panel closed while
   * fullscreen used to strand the reader in a window with no way back.
   * `exitNativeFullscreen` holds the rest of the reasoning.
   */
  const closeVibe = useCallback(() => {
    setVibeOpen(false);
    void exitNativeFullscreen();
  }, []);
  /**
   * Which machines the conversation list shows. Kept at the shell rather than
   * in the rail so the choice survives the rail remounting when the sidebar
   * collapses.
   */
  const [deviceFilter, setDeviceFilter] = useState<DeviceFilter>({ kind: "all" });
  /**
   * The remote conversation on screen, if any.
   *
   * Its own state rather than a third `centerMode`: the two are different kinds
   * of thing (a local thread id vs a `(desktopId, sessionId)` pair), and the
   * new-chat flows only know how to leave the thread view — giving them a mode
   * they never set is how a stale remote view reappears.
   */
  const [activeRemote, setActiveRemote] = useState<{
    desktopId: string;
    sessionId: string;
    /**
     * The workspace a *draft* belongs in, on that host.
     *
     * Set when the draft was started from a workspace heading; the host resolves
     * the id against its own store when the first prompt creates the session.
     * Null for a draft started with no workspace in mind.
     */
    workspaceId: string | null;
  } | null>(null);
  /**
   * A draft on another machine, if that is what is on screen.
   *
   * An empty session id is the host's own signal for "this conversation does not
   * exist yet", so it is also the answer to "may the reader still choose where
   * it lives": a conversation that already exists has a workspace and cannot be
   * moved, while a draft can be pointed at any of that host's workspaces (or at
   * none) until its first prompt creates it.
   */
  const remoteDraft = activeRemote?.sessionId === "" ? activeRemote : null;
  /**
   * That host's workspaces, read while a draft is open.
   *
   * Read here rather than inside the chooser because two things need the names:
   * the chooser's menu, and the draft's header badge — which otherwise has no
   * catalogue row to read a workspace name from, since the conversation does not
   * exist on the host yet.
   */
  const draftWorkspaces = useRemoteWorkspaceRows(remoteDraft?.desktopId ?? null);
  /** The remote conversation whose rename dialog is open, if any. */
  const [remoteRename, setRemoteRename] = useState<MergedConversation | null>(null);
  /** The remote *workspace* whose rename dialog is open, if any. */
  const [remoteWorkspaceRename, setRemoteWorkspaceRename] = useState<RemoteWorkspaceTarget | null>(null);
  /** Whether the open remote conversation's account book is showing. */
  const [remoteUsageOpen, setRemoteUsageOpen] = useState(false);
  const [leftExpanded, setLeftExpanded] = useState(true);
  const [leftOverlayOpen, setLeftOverlayOpen] = useState(false);
  const [rightExpanded, setRightExpanded] = useState(false);
  // Panels open on the content tab (Files/Review), not Runs; ContextPanel seeds
  // the exact tab each time the panel opens (not per thread — a mid-open thread
  // switch keeps the current tab). Files is the default in both modes while the
  // Artifacts tab is hidden (see `fileTabs` in ContextPanel).
  const [contextTab, setContextTab] = useState<ContextTab>("files");
  const [newChatWorkspaceId, setNewChatWorkspaceId] = useState<string | null>(null);
  const [newConversationMode, setNewConversationMode] = useState<"workspace" | "chat">("chat");
  const [newWorkspaceForm, setNewWorkspaceForm] = useState<"open" | null>(null);
  // Bumped on every workspace-header "+" click so the new-conversation view
  // remounts and re-opens the create dialog even when we're already on it.
  const [newWorkspaceNonce, setNewWorkspaceNonce] = useState(0);
  /**
   * The workspace "+" flow's machine step.
   *
   * The form behind it only knows this computer's folder picker, so with
   * another machine paired the machine is asked for first — the same first
   * question the phone's new-conversation dialog opens on.
   */
  const [workspaceDeviceChoice, setWorkspaceDeviceChoice] = useState(false);
  /**
   * The machine step in front of a new conversation.
   *
   * A conversation lives on one machine and runs there, so "new conversation"
   * cannot create anything until it knows which machine — and the rail's list
   * can be showing one machine or all of them. The create controls used to
   * assume this computer: with a host's conversations on screen, the button
   * and ⌘N made a local conversation, which is a different disk, a different
   * agent and a different place from the one the reader was looking at.
   *
   * The device picker above the list answers the question when it names one
   * machine, and that case asks nothing (`handleNewConversationRequest`).
   */
  const [newChatDeviceChoice, setNewChatDeviceChoice] = useState(false);
  /** The host whose create-workspace form is open, if any. */
  const [remoteWorkspaceHost, setRemoteWorkspaceHost] = useState<RemotePeer | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [cheatSheetOpen, setCheatSheetOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");

  const { hasUpdate, cachedStatus, markSeen: markUpdateSeen } = useUpdateChecker();
  // Drives the onboarding gate below. Kept with the other top-level hooks so
  // the early returns further down stay after every hook call (rules of hooks).
  const {
    balance: futureBalance,
    balanceStatus: futureBalanceStatus,
    email: futureEmail,
    refreshAuth: refreshFutureAuth,
    refreshBalance: refreshFutureBalance,
    status: futureSessionStatus,
  } = useFutureAccount(initialAuth);
  const { appSettings, appSettingsReady, changeSettings } = useAppSettings(futureSessionStatus);
  // Startup defaults must never briefly expose commercial entries before the
  // persisted community preference has arrived.
  const communityEdition = !appSettingsReady || appSettings.communityEdition;
  // Remote desktops (client role). Enabled for the whole session: the merged
  // conversation list is the chat section's own list, so a peer appearing must
  // update it without the user having visited the Remote Desktops screen first.
  //
  // The same list is what the Remote page's client section draws, and that is
  // deliberate: it is the one that is live. That section used to read its own
  // copy once, when the page opened, so a machine taken over from another device
  // kept saying "Connected" — until the user left the page and came back.
  const {
    peers: remotePeers,
    catalogs: remoteCatalogs,
    error: remotePeersError,
    loading: remotePeersLoading,
    refresh: refreshRemotePeers,
    reconnectAll: reconnectRemotePeers,
  } = useRemotePeers(true);
  useAutoUpgradeSkills(appSettings.autoUpgradeSkills);
  useAgentDoneBell(appSettings.bellOnComplete);
  useRemoteDoneBell(appSettings.bellOnComplete);
  const { showGate, byokMode, enableBYOK, finishInit, cancelLogin, hasAnyProvider, forceOnboarding, initPending } = useHasProviders(futureSessionStatus, initialProviders);

  const windowWidth = useWindowWidth();
  // Side panels yield to the conversation: when the window is too narrow to
  // keep the center at its floor alongside a panel, the panel folds instead of
  // squeezing (or overflowing) the center. The user's own collapsed/expanded
  // choice is kept, so the panel comes back on its own when the window grows.
  const rightPanelAvailable = canShowRightPanel(windowWidth);
  const showLeftPanel = leftExpanded && canShowLeftPanel(windowWidth);
  const leftPanel = useLeftPanelWidth(rightExpanded && rightPanelAvailable);
  const centerRef = useRef<HTMLElement>(null);
  const {
    width: rightPanelWidth,
    resizing: rightPanelResizing,
    startResize: startRightPanelResize,
    reclamp: reclampRightPanel,
    nudge: nudgeRightPanel,
  } = useRightPanelWidth(centerRef);

  // The left rail's width changes the center's left edge, so a collapse/expand
  // can shrink the space available to the center — re-clamp the right panel.
  useEffect(() => {
    reclampRightPanel();
  }, [showLeftPanel, leftPanel.width, reclampRightPanel]);

  // Install the Tauri event listener for real-time agent state updates
  // (settings changes from other clients).  Only runs once.
  useEffect(() => {
    installAgentEventListener();
  }, []);

  // On startup, tell the agent to re-scan skills so any skills installed
  // outside the GUI (e.g. via CLI) are visible without a restart.
  useEffect(() => {
    void refreshSkills();
  }, []);

  const {
    threads,
    workspaces,
    activeThread,
    activeWorkspace,
    activeThreadId,
    setActiveThreadId,
    threadRunStatuses,
    threadStreamingStatuses,
    loadingStore,
    storeError,
    refreshStore,
  } = useThreadStore();

  // Start observing the active thread's agent session for real-time
  // settings-change events (model, thinking, name, cwd, etc.).
  useEffect(() => {
    const threadId = activeThread?.id;
    const sessionId = activeThread?.agentSessionId;
    if (threadId && sessionId) {
      invokeCommand("observe_session", { threadId, sessionId }).catch(() => {});
    }
  }, [activeThread?.id, activeThread?.agentSessionId]);

  // Conversations the composer's `#` menu offers: the rail's order, minus the
  // conversation being composed in (referencing yourself would be a no-op). On
  // the new-chat screen nothing is being composed *in* — the thread does not
  // exist until the first message is sent — so nothing is excluded there.
  // `activeThread` is only the conversation the user came from, and excluding it
  // would hide the very conversation a first message most often references:
  // opening a fresh chat to ask about what you were just doing found "no
  // matches" for the one title you could be sure of. The id is still dropped on
  // a real thread, where the composer does have a conversation of its own.
  const sessionMentions = useMemo(
    () => sessionMentionOptions(
      threads,
      workspaces,
      centerMode === "new-chat" ? null : activeThread?.agentSessionId,
    ),
    [centerMode, threads, workspaces, activeThread?.agentSessionId],
  );

  // Refresh the store when the agent session's cwd changes (e.g. TUI /cwd),
  // so the thread moves to the correct workspace in the sidebar.
  useEffect(() => {
    const handler = () => {
      refreshStore().catch(() => {});
    };
    window.addEventListener("future:cwd-changed", handler);
    return () => window.removeEventListener("future:cwd-changed", handler);
  }, [refreshStore]);

  // Keep the startup/default thread and every user-selected thread eligible
  // for next-launch restoration. This timestamp is intentionally excluded
  // from the sidebar's recency sort.
  useEffect(() => {
    if (activeThreadId)
      void markThreadOpened(activeThreadId);
  }, [activeThreadId]);

  const { activeApproval, decideApproval } = useApprovals(activeThread?.id ?? null);
  const {
    agentConnection,
    modelOptions,
    visibleModelOptions,
    selectedModelId,
    setSelectedModelId,
    refreshAgentModels,
  } = useAgentConnection(appSettings.hiddenModels);

  // When the onboarding gate finalizes a model choice it writes the pick to the
  // composer's last-used slot and emits `future-models-synced`. Apply that pick
  // to the live selection explicitly so the first composer after the gate closes
  // shows the chosen model — not whatever `resolveInitialModelId` would default
  // to. Harmless on the init-phase emit (the last-used slot is empty/old then, so
  // the guard no-ops and normal reconciliation owns the value).
  useEffect(
    () => onFutureEvent("future-models-synced", () => {
      const lastUsed = readLastUsedModel();
      if (lastUsed && modelOption(lastUsed, visibleModelOptions))
        setSelectedModelId(lastUsed);
    }),
    [visibleModelOptions, setSelectedModelId],
  );
  // Bridge the backend's silent 24-hour Future catalogue refresh into the
  // existing frontend invalidation path used by onboarding and manual sync.
  useTauriEvent<{ synced: boolean }>("scheduler-future-models", (result) => {
    if (result.synced)
      emitFutureEvent("future-models-synced", undefined);
  });

  // Poll the remote bridge status (for the sidebar indicator dot) at the app
  // level. Returns { status, indicator, refresh } — the Remote page reads
  // `status` and the rail reads `indicator`, from this one source.
  // directly so its blue dot always matches the sidebar indicator.
  const { status: remoteStatus, indicator: remoteIndicator, refresh: refreshRemote } = useRemoteStatus(true);

  // Remote credential renewal can be the first place a revoked account key is
  // observed. Recheck the account through the same authoritative profile path.
  // A freshly issued bridge credential rejected by NATS remains a separate
  // service-side failure and must not be presented as an account logout.
  useEffect(() => {
    if (remoteStatus?.phase === "failed" && remoteStatus.reason === "account_authorization")
      refreshFutureAuth();
  }, [remoteStatus?.phase, remoteStatus?.reason, refreshFutureAuth]);

  useEffect(() => {
    if (futureSessionStatus === "signed_out" || futureSessionStatus === "invalid")
      void stopRemote().then(refreshRemote).catch(() => {});
  }, [futureSessionStatus, refreshRemote]);

  // Account reauthentication replaces the rejected FutureOS key. If Remote
  // stopped specifically because that key was invalid, immediately retry the
  // persisted pairing with the new key instead of leaving a stale terminal
  // error for the user to clear manually.
  const previousFutureSessionStatusRef = useRef(futureSessionStatus);
  useEffect(() => {
    const previousStatus = previousFutureSessionStatusRef.current;
    previousFutureSessionStatusRef.current = futureSessionStatus;
    if (
      futureSessionStatus === "authenticated"
      && previousStatus === "checking"
      && remoteStatus?.phase === "failed"
      && remoteStatus.reason === "account_authorization"
      && Boolean(remoteStatus.pairId)
    ) {
      void startRemote({}).then(refreshRemote).catch(() => {});
    }
  }, [futureSessionStatus, remoteStatus?.phase, remoteStatus?.reason, remoteStatus?.pairId, refreshRemote]);

  const handleRecharge = () => {
    getFutureEnvironment().then(env => openExternalUrl(`${env.platformUrl}/platform/#recharge`)).catch(() => {});
  };

  const handleOpenUpdate = () => {
    setSettingsTab("update");
    setSettingsOpen(true);
  };

  // When the agent becomes available (startup, restart, or recovery after
  // a disconnect), re-trigger the skills scan.  The agent has a 5 s rate
  // limit so rapid repeats are harmless.
  useEffect(() => {
    if (agentConnection.status === "connected") {
      void refreshSkills();
      // Cached per-thread agent state may predate the restart/reconnect —
      // possibly by less than the TTL, which would short-circuit a plain
      // prefetch — so force-revalidate the viewed thread. Other threads
      // revalidate on their next activation.
      // A new thread has no authoritative Agent state until its first prompt
      // creates a session. Preserve its explicit draft model/thinking cache;
      // forcing get_state now would return nulls and replace the user's pick
      // with the selected model's default.
      if (activeThread?.agentSessionId)
        revalidateAgentState(activeThread.id);
    }
  }, [activeThread?.id, activeThread?.agentSessionId, agentConnection.status]);

  // BYOK (bring your own key): the user chose to skip FutureOS sign-in and add
  // their own provider. Open Settings → Providers so they can configure it.
  useEffect(() => {
    if (byokMode) {
      setSettingsTab("providers");
      setSettingsOpen(true);
    }
  }, [byokMode]);

  const {
    selectedThinkingLevel,
    modelsEmptyReason,
    activeThreadModelId,
    activeThinkingLevel,
    changeModel,
    changeDraftModel,
    changeDraftThinkingLevel,
    changeThinkingLevel,
    syncSelection,
  } = useModelSelection({
    activeThread,
    selectedModelId,
    setSelectedModelId,
    modelOptions,
    visibleModelOptions,
    refreshStore,
  });
  /**
   * Set by the Vibe panel's add tile right before it opens the new-conversation screen.
   *
   * Read (and cleared) once, at the moment a thread is created, so "add a conversation" and
   * "pin what you create" stay one promise without the panel having to watch the store for
   * a thread it did not create.
   */
  const pinNextCreatedRef = useRef(false);
  const {
    pendingPrompt,
    startNewConversation,
    consumePendingPrompt,
  } = useNewConversation({
    refreshStore,
    syncSelection,
    setSection,
    setCenterMode,
    pinCreated: () => {
      const armed = pinNextCreatedRef.current;
      pinNextCreatedRef.current = false;
      return armed;
    },
  });
  const {
    renameDialog,
    deleteDialog,
    batchDeleteDialog,
    setRenameDialog,
    setDeleteDialog,
    setBatchDeleteDialog,
    openRename,
    confirmRename,
    generateTitle,
    openDelete,
    confirmDelete,
    openBatchDelete,
    confirmBatchDelete,
  } = useThreadDialogs({ activeThreadId, refreshStore });
  const {
    renameDialog: workspaceRenameDialog,
    deleteDialog: workspaceDeleteDialog,
    setRenameDialog: setWorkspaceRenameDialog,
    setDeleteDialog: setWorkspaceDeleteDialog,
    openRename: openWorkspaceRename,
    confirmRename: confirmWorkspaceRename,
    openDelete: openWorkspaceDelete,
    confirmDelete: confirmWorkspaceDelete,
  } = useWorkspaceDialogs({ refreshStore });
  const unreadThreadIds = useUnreadThreads(threadRunStatuses, activeThreadId);
  // Stable identity: an inline `.filter()` would hand NewConversation a fresh
  // array every render (and this component re-renders on every poll tick),
  // re-firing its workspace-adoption effect.
  const userWorkspaces = useMemo(
    () => workspaces.filter(workspace => workspace.kind === "user"),
    [workspaces],
  );
  const hideRightPanel
    = centerMode === "new-chat"
      || section === "skill"
      || section === "remote"
      // Tasks are their own two-pane view (list + detail); the context panel
      // beside them would describe whichever conversation happened to be active
      // before, which is not what the tasks view is about.
      || section === "tasks"
      || !rightPanelAvailable;

  // The terminal belongs to a conversation: it is offered only while a real
  // thread is on screen, so the shortcut and the panel state never apply to the
  // new-chat / skills / phone-control views (see `terminalTarget`).
  const terminalThreadId = terminalTarget({ section, centerMode, threadId: activeThread?.id });
  const terminalPanel = useTerminalPanel(terminalThreadId);
  const terminalHeaderAction = terminalThreadId
    ? (
        <TerminalToggleButton
          onToggle={terminalPanel.toggle}
          open={terminalPanel.open}
          shortcut={terminalPanel.shortcut}
        />
      )
    : null;

  /**
   * Which machine a local conversation is on — this one.
   *
   * Said only when there is another machine in the list: the header's job is to
   * answer "where am I", and with nothing paired there is one answer and no
   * question. The word and the icon are the device selector's own (`本机`, the
   * monitor glyph), so the same machine is named the same way in both places.
   */
  const localMachineBadge = remotePeers.length > 0
    ? (
        <MachineBadge
          glyph={<Monitor aria-hidden className="size-3.5" />}
          label={tRemotePeer("selector.local")}
        />
      )
    : null;

  /**
   * Which workspace the open conversation works in, named the way the rail's
   * heading names it.
   *
   * Read from the thread, not from `activeWorkspace`: that one falls back to the
   * first workspace when the thread's own is gone, and a header naming somebody
   * else's folder is worse than one that says nothing. The rail's own rule is
   * the mode — a chat conversation is listed under the chat section however the
   * agent's cwd has filed it, and a workspace conversation under its workspace —
   * so a chat names nothing here either, and the two lists keep saying the same
   * thing about the same conversation.
   */
  const activeThreadWorkspace = activeThread?.mode === "workspace"
    ? workspaces.find(workspace => workspace.id === activeThread.workspaceId)
    : undefined;
  const localWorkspaceBadge = activeThreadWorkspace
    ? <WorkspaceBadge name={activeThreadWorkspace.name} />
    : null;

  // Bridge the backend's deferred shadow-review notification (C1) onto the
  // typed event bus so the Review panel refreshes when the changeset lands.
  useTauriEvent<string>("review-updated", (threadId) => {
    emitFutureEvent("review-updated", { threadId });
  });

  // macOS app menu "About FutureOS" opens the in-app About page (there is no
  // native About dialog). The backend emits this event from the menu handler.
  useTauriEvent("open-settings", () => {
    setSettingsTab("about");
    setSettingsOpen(true);
  });

  // The rest of the macOS menu (New Conversation, Settings…) names a **command**,
  // not an action or a key: the item carries no accelerator, so this is where the
  // user's current binding, the palette and the menu converge on one handler. An
  // id this build does not know is ignored rather than guessed at.
  useTauriEvent<string>("menu-command", (id) => {
    if (isCommandId(id))
      runCommand(id);
  });

  // Remote (phone) activity: a phone client created or drove a thread. Refresh
  // the thread list + runs so it appears and updates live in the GUI.
  useTauriEvent("remote-activity", () => {
    void refreshStore();
  });

  // Sessions created outside the GUI (TUI/CLI/channels) were imported into the
  // store by the backend — refresh the thread list so they appear in the
  // sidebar without a user action.
  useTauriEvent("skills_changed", () => {
    emitFutureEvent("skills-changed", undefined);
  });

  useTauriEvent("threads-updated", () => {
    void refreshStore();
  });

  function handleSectionChange(nextSection: ActivitySection) {
    if (nextSection === "settings") {
      setSettingsTab("general");
      setSettingsOpen(true);
      return;
    }
    setSection(nextSection);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
  }

  // Stable by construction (they only set two pieces of shell state), which is
  // what lets a panel cell's prop object keep its identity across a re-render:
  // without this, every streaming push would hand every cell a new "open models"
  // and re-render the whole grid.
  const handleOpenModels = useCallback(() => {
    setSettingsTab("models");
    setSettingsOpen(true);
  }, []);

  const handleOpenAccount = useCallback(() => {
    setSettingsTab("account");
    setSettingsOpen(true);
  }, []);

  const handleOpenProviders = useCallback(() => {
    setSettingsTab("providers");
    setSettingsOpen(true);
  }, []);

  function handleSelectThread(thread: StoredThread) {
    setSection(thread.mode === "workspace" ? "workspace" : "chat");
    setActiveThreadId(thread.id);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
    // Opening a conversation of *this* machine leaves the remote one. Without
    // this the remote conversation stays selected and the column keeps rendering
    // it (the remote branch is checked first), so clicking a local row appeared
    // to open the other machine's conversation — with the remote transcript.
    setActiveRemote(null);
  }

  /// Open the conversation a task run produced (from the Tasks panel).
  function handleOpenTaskThread(threadId: string) {
    const thread = threads.find(candidate => candidate.id === threadId);
    if (thread) {
      handleSelectThread(thread);
      return;
    }
    // A run's conversation may not be in the local list yet (it was created by
    // the tick loop); refresh the catalog and open it by id.
    setSection("chat");
    setActiveThreadId(threadId);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
    // As in `handleSelectThread`: this is a local conversation.
    setActiveRemote(null);
    void refreshStore();
  }

  function handleSelectWorkspace(_workspace: StoredWorkspace, workspaceThreads: StoredThread[]) {
    const latestThread = workspaceThreads[0];
    setSection("workspace");
    setNewChatWorkspaceId(null);
    if (latestThread) {
      setActiveThreadId(latestThread.id);
      setCenterMode("thread");
    }
    else {
      setActiveThreadId(null);
      setCenterMode("thread");
    }
  }

  function handleOpenNewChat(workspaceId?: string) {
    // A workspace named by the caller (a workspace heading's "+") is an explicit
    // target. Anything else is the plain "new conversation", which opens on the
    // target the user created last: a workspace they keep working in should not
    // have to be picked again on every visit (see `newConversationTarget.ts`).
    const target = workspaceId
      ? { mode: "workspace" as const, workspaceId }
      : defaultNewConversationTarget(
          readNewConversationTarget(),
          id => userWorkspaces.some(workspace => workspace.id === id),
        );
    // The section follows the target so the rail highlights where the composer
    // is pointing, the same way the workspace "+" flow does.
    setSection(target.mode === "workspace" ? "workspace" : "chat");
    setNewChatWorkspaceId(target.workspaceId);
    setNewConversationMode(target.mode);
    setNewWorkspaceForm(null);
    setCenterMode("new-chat");
  }

  // Skills page 「试试」: pre-fill the new-chat composer with the `/skill` token
  // (restored as a pill), then open the new-chat screen.
  function handleTrySkill(skillName: string) {
    saveComposerDraft("new", { text: `/${skillName} ` });
    handleOpenNewChat();
  }

  /**
   * The rail's plain "new conversation": device first, then the conversation.
   *
   * Three answers, in the order they cost the reader:
   *
   * - **One machine on screen**: the picker above the list already names it, and
   *   that is the answer — asking again would be a dialog whose only content is
   *   what the button beside it says. This is the ordinary case and it creates
   *   straight away, on that machine.
   * - **One machine paired**: nothing to choose between, so nothing is asked.
   * - **The merged list with machines to choose from**: the machine *is* the
   *   question, so it is asked — in a dialog, because this is also what ⌘N does
   *   and a shortcut has no button to hang a menu under.
   *
   * A conversation started inside a workspace skips all of this
   * (`handleOpenNewChat(workspaceId)`): that workspace is a folder on this
   * machine's disk, and there is no question left to ask.
   */
  function handleNewConversationRequest() {
    if (deviceFilter.kind === "device") {
      if (deviceFilter.desktopId === null) {
        handleOpenNewChat();
        return;
      }
      startRemoteConversation(deviceFilter.desktopId);
      return;
    }
    // Only a machine that can take the work is something to choose between: a
    // machine that is offline, restarting, or driven by another paired device
    // would answer the tap and then refuse the first prompt.
    if (!remotePeers.some(peerDrivable)) {
      handleOpenNewChat();
      return;
    }
    setNewChatDeviceChoice(true);
  }

  // Skills page 「教我使用技能」: create the coach conversation (same flow as
  // the banner) with the platform coach prompt as the first message.
  function handleStartCoachConversation(content: string) {
    return startNewConversation({
      content,
      mode: "chat",
      modelId: selectedModelId || defaultAgentModelId,
      thinkingLevel: selectedThinkingLevel,
    });
  }

  // Workspace header "+" → always (re)open the create-workspace dialog, even if
  // we're already on the new-conversation view. The nonce forces a remount so a
  // previously-cancelled dialog reopens.
  function handleOpenNewWorkspace() {
    // A machine that can take work makes "which disk is this folder on?" the
    // first question: the form behind this button only browses *this*
    // computer. With nothing else able to answer there is nothing to choose,
    // and the form opens straight away as it always did.
    if (remotePeers.some(peer => peerStatus(peer) === "connected")) {
      setWorkspaceDeviceChoice(true);
      return;
    }
    openLocalWorkspaceForm();
  }

  /** The local half of the workspace "+": the form's picker browses this disk. */
  function openLocalWorkspaceForm() {
    setSection("workspace");
    setNewChatWorkspaceId(null);
    setNewConversationMode("workspace");
    setNewWorkspaceForm("open");
    setNewWorkspaceNonce(nonce => nonce + 1);
    setCenterMode("new-chat");
  }

  /**
   * A workspace was just created on a host.
   *
   * The host's catalogue is re-read so the row shows under that machine, and
   * the user lands where the local flow leaves them: a composer in the new
   * workspace — a draft on the host, which creates the session when the first
   * prompt is sent.
   */
  function handleRemoteWorkspaceCreated(desktopId: string, workspace: RemoteWorkspaceRow) {
    setRemoteWorkspaceHost(null);
    void refreshRemotePeers();
    // The draft's chooser lists that host's workspaces, and the one just created
    // is not in the list it read — a workspace picked from a menu that does not
    // contain it would read as unnamed.
    void draftWorkspaces.reload();
    startRemoteConversation(desktopId, workspace.id);
  }

  async function handleAddWorkspace(input: WorkspaceCreateRequest) {
    const workspace = await createWorkspace(input);
    await refreshStore(activeThread?.id ?? undefined);
    return workspace;
  }

  /**
   * Delete a batch of conversations, each on the host that owns it.
   *
   * One command per conversation, because that is the host's own unit: its
   * `delete_session` takes a thread id and is idempotent, so a parent deleted
   * before its child (or a row another client already removed) is answered
   * rather than refused. A failure is reported with the host's reason and does
   * not stop the rest — a batch that half-deleted is still a batch to be honest
   * about, and the catalogue is re-read either way so the list shows what is
   * actually there.
   */
  async function handleDeleteRemoteConversations(rows: MergedConversation[]) {
    const failures: string[] = [];
    for (const row of rows) {
      if (!row.desktopId)
        continue;
      try {
        await deleteRemoteConversation(row.desktopId, { sessionId: row.id, threadId: row.threadId });
      }
      catch (error) {
        failures.push(`${row.title}: ${errorMessage(error)}`);
      }
    }
    await refreshRemotePeers();
    if (failures.length > 0) {
      emitFutureEvent("toast", {
        message: tRemotePeer("deleteFailed", { message: failures.join("; ") }),
        tone: "error",
      });
    }
    for (const row of rows) {
      if (row.desktopId)
        forgetRemoteConversation(row.desktopId, row.id);
    }
  }

  /**
   * Stop drawing a conversation that no longer exists.
   *
   * Both ways of deleting one land here. Only the batch did, which is why the
   * same delete behaved differently depending on how it was asked for: from a
   * row's own menu the conversation was really deleted on the host, its row went
   * out of the list, and the column went on drawing its history — nothing on
   * screen said it was gone.
   *
   * Decided here rather than inferred from the catalogue, because a host that
   * does not answer drops out of the catalogue too (`useRemotePeers` keeps only
   * the hosts that replied), and closing what the reader is reading because the
   * network blinked would be worse than the bug.
   */
  function forgetRemoteConversation(desktopId: string, sessionId: string) {
    if (activeRemote?.desktopId === desktopId && activeRemote.sessionId === sessionId) {
      setActiveRemote(null);
      setCenterMode("thread");
    }
  }

  async function handleRemoteConversationDeleted(conversation: MergedConversation) {
    if (conversation.desktopId)
      forgetRemoteConversation(conversation.desktopId, conversation.id);
    await refreshRemotePeers();
  }

  /**
   * Pin or unpin this machine's conversation **by id**.
   *
   * The panel addresses a cell by its key, and the cell it removes is one it may have
   * just taken out of the list it is drawing: the id is the stable handle, and the
   * store is what knows the current flag.
   */
  async function pinLocalThread(threadId: string, pinned: boolean) {
    await pinThread({ pinned, threadId });
    await refreshStore(activeThread?.id ?? undefined);
  }

  /**
   * Pin or unpin a host's conversation.
   *
   * The host's own route takes a **thread id** and refuses a session id, so the
   * catalogue is what turns the panel's key into an address; a host too old to report
   * one is reported rather than guessed at (`pinRemoteConversation` refuses).
   */
  async function pinRemoteSession(desktopId: string, sessionId: string, pinned: boolean) {
    const row = remoteCatalogs
      .find(catalog => catalog.desktopId === desktopId)
      ?.sessions
      .find(session => session.sessionId === sessionId);
    try {
      await pinRemoteConversation(desktopId, { sessionId, threadId: row?.threadId ?? null }, pinned);
    }
    catch (error) {
      emitFutureEvent("toast", { message: errorMessage(error), tone: "error" });
    }
    // Either way the catalogue is re-read: a pin the host refused leaves the row as it
    // was, and the panel should show that instead of assuming it worked.
    await refreshRemotePeers();
  }

  async function handleTogglePinThread(thread: StoredThread) {
    await pinThread({ threadId: thread.id, pinned: !thread.pinned });
    await refreshStore(thread.id);
  }

  // Workspace pinning is the same ordering flag as a conversation's: the
  // group menu toggles it and the rail re-sorts on the refreshed store.
  async function handleTogglePinWorkspace(workspace: StoredWorkspace) {
    await pinWorkspace({ workspaceId: workspace.id, pinned: !workspace.pinned });
    await refreshStore(activeThread?.id ?? undefined);
  }

  async function handleApprovalDecision(
    approval: StoredApprovalRequest,
    status: "approved" | "rejected",
  ) {
    await decideApproval(approval, status);
    await refreshStore(activeThread?.id ?? undefined);
  }

  async function handleRestoreThread(thread: StoredThread) {
    const restoredThread = await restoreThread(thread.id);
    await refreshStore(restoredThread.id);
    setSection(restoredThread.mode === "workspace" ? "workspace" : "chat");
    setCenterMode("thread");
  }

  function handleToggleLeftPanel() {
    // Below the two-column floor the rail has nowhere to open into, so the
    // toggle must not commit "collapsed" as the user's preference — the rail
    // would then stay hidden after the window grows back.
    if (!canShowLeftPanel(windowWidth))
      return;
    setLeftOverlayOpen(false);
    setLeftExpanded(expanded => !expanded);
  }

  function handlePreviewLeftPanel(open: boolean) {
    if (showLeftPanel)
      return;
    setLeftOverlayOpen(open);
  }

  const remoteTimeline = useRemoteTimeline(
    activeRemote?.desktopId ?? null,
    activeRemote?.sessionId ?? null,
    activeRemote !== null,
  );
  const remoteApprovals = useRemoteApprovals(
    activeRemote?.desktopId ?? null,
    activeRemote?.sessionId ?? null,
  );
  // The conversation's model and thinking level, which the *host* owns.
  // Enabled whenever a remote conversation is on screen — *including* a draft.
  // A draft has no session to read, but it still needs the host's catalogue (the
  // model picker was empty, and so disabled) and the level a new conversation
  // will run with, and both travel with the first prompt. This used to require a
  // session id, which is exactly why a draft offered neither.
  const remoteSettings = useRemoteConversationSettings(
    activeRemote?.desktopId ?? null,
    activeRemote?.sessionId ?? null,
    activeRemote !== null,
    // The timeline has already read this conversation's `get_state` — it must,
    // for the run in flight — and the model and thinking level are the same
    // reply. Handing it over is what keeps an open from spending two of them,
    // which is one more than the phone spends on the same conversation.
    remoteTimeline.sessionState,
  );

  /**
   * Skill recommendation for a remote conversation, from that host.
   *
   * The same hook this app's own composer uses, with the data pointed at the
   * host and the catalogue read from there — so the trigger rules are the ones
   * that were tuned, not a second set written for this path.
   *
   * `sessionStatus: "unavailable"` / `balance: null` is this hook's own way of
   * saying "the account state is not knowable here": it is the paired
   * computer's account that matters, this machine cannot read it, and the host
   * answers "no recommendation" when its own account cannot produce one.
   */
  const remoteSkillRecommend = useRemoteSkillRecommend(activeRemote?.desktopId ?? "");
  const remoteCatalog = useRemoteSkillCatalog(
    activeRemote?.desktopId ?? "",
    remoteSkillRecommend,
  );
  const remoteSkillSource = useMemo(
    () => remoteSkillRecoSource(activeRemote?.desktopId ?? ""),
    [activeRemote?.desktopId],
  );
  const remoteSkillReco = useSkillRecommendation({
    balance: null,
    catalog: remoteCatalog,
    enabled: remoteSkillRecommend,
    sessionStatus: "unavailable",
    source: remoteSkillSource,
  });
  /**
   * The card's install, on the machine the card is about.
   *
   * The recommender names a skill but never a version, so the version comes
   * from that host's own catalogue — and a catalogue that does not publish one
   * cannot be installed from here.
   */
  const installRecommended = useCallback((card: { name: string }) => {
    const desktopId = activeRemote?.desktopId;
    const version = desktopId
      ? remoteCatalog.catalogue.find(entry => entry.id === card.name)?.version
      : undefined;
    if (!desktopId || !version)
      return Promise.resolve(false);
    return installRemoteSkill(desktopId, card.name, version)
      .then(() => {
        // The card was chosen from a candidate set that no longer exists once it
        // is installed, so the catalogue is re-read.
        remoteCatalog.reload();
        return true;
      })
      .catch(() => {
        // The card stays up and the draft is not sent: the message was written
        // for a skill that is not there.
        return false;
      });
  }, [activeRemote, remoteCatalog]);
  const remoteSkillRecommendation = useMemo(() => ({
    card: remoteSkillReco.state.recommendation,
    onDismiss: remoteSkillReco.dismiss,
    onEvaluate: remoteSkillReco.evaluate,
    onInstall: installRecommended,
  }), [
    remoteSkillReco.state.recommendation,
    remoteSkillReco.dismiss,
    remoteSkillReco.evaluate,
    installRecommended,
  ]);
  const activeRemotePeer = activeRemote
    ? remotePeers.find(peer => peer.desktopId === activeRemote.desktopId)
    : undefined;
  // Read once per render rather than at each use: the header and the usage
  // dialog must name the same conversation, and a draft (an empty session id)
  // simply has no catalogue row to read.
  const activeRemoteContext = activeRemote
    ? contextOfRemote(remoteCatalogs, activeRemote)
    : null;
  /**
   * The workspace a remote draft is in, named as the host names it.
   *
   * A draft has no catalogue row, so `activeRemoteContext` cannot supply this —
   * the chooser's own read of that host's workspaces does, which is why the read
   * lives here at the shell rather than inside the menu. `null` while the list
   * has not arrived (or when the draft has no workspace), and the header then
   * leaves the badge off rather than drawing an id.
   */
  const draftWorkspace = remoteDraft?.workspaceId
    ? (draftWorkspaces.rows.find(row => row.id === remoteDraft.workspaceId) ?? null)
    : null;
  /**
   * That host's conversations, for the remote composer's `#` menu.
   *
   * The same snapshot the list is drawn from, so a reference can point at any
   * conversation the reader can see, including one this client never opened —
   * and without a second request to the host while a draft is being typed.
   */
  const remoteConversations = useMemo(
    () => (activeRemote
      ? remoteCatalogs.find(catalog => catalog.desktopId === activeRemote.desktopId)?.sessions ?? []
      : []),
    [remoteCatalogs, activeRemote],
  );

  /**
   * Ask a host to compact a conversation's context.
   *
   * Resolving here means the host *accepted* it. What follows — commit, failure,
   * or nothing to do — arrives as the host's own events, which is why the
   * transcript is re-read afterwards: the checkpoint that comes out of a
   * committed compaction is content, and waiting for a later open to show it
   * would make the action look like it did nothing.
   */
  async function compactRemoteConversationOn(desktopId: string, sessionId: string) {
    if (!sessionId)
      return;
    try {
      await compactRemoteConversation(desktopId, sessionId);
      await remoteTimeline.refresh();
    }
    catch (error) {
      emitFutureEvent("toast", {
        message: tRemotePeer("compactFailed", { message: errorMessage(error) }),
        tone: "error",
      });
    }
  }

  /**
   * Resume a failed run on a host.
   *
   * A continuation re-runs the same turn with what it already had — including any
   * attachment it carried — which is why nothing is re-sent here. The transcript
   * is re-read afterwards so the resumed turn appears as it did before.
   */
  async function continueRemoteRunOn(desktopId: string, sessionId: string, runId: string) {
    try {
      // Queue the continuation through the outbox so an unreachable host does
      // not lose the user's request; it will be retried automatically.
      await sendRemoteContinuation(desktopId, sessionId, runId);
      await remoteTimeline.refresh();
      await refreshRemotePeers();
    }
    catch (error) {
      emitFutureEvent("toast", {
        message: tRemotePeer("continueFailed", { message: errorMessage(error) }),
        tone: "error",
      });
    }
  }

  function openRemoteConversation(conversation: MergedConversation) {
    if (conversation.desktopId === null)
      return;
    setActiveRemote({ desktopId: conversation.desktopId, sessionId: conversation.id, workspaceId: null });
    setSection("chat");
    // The mirror of `handleSelectThread`: opening a remote conversation leaves
    // the local one, and it has to actually be *shown* — a draft left in the
    // column would otherwise keep the new-chat screen up over it.
    setActiveThreadId(null);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
  }

  /**
   * Open one of a paired computer's conversations from its ids.
   *
   * The same destination as opening a row, for callers that have the ids rather
   * than a row — a task run names its conversation by session id.
   */
  function openRemoteSession(desktopId: string, sessionId: string) {
    setActiveRemote({ desktopId, sessionId, workspaceId: null });
    setSection("chat");
    setActiveThreadId(null);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
  }

  /**
   * Branch a remote conversation at a settled turn and open the child.
   *
   * The host owns the fork; the ack's ids are the only way to know the child, so
   * the child is opened from them (not guessed). `forkable` is false when the
   * turn is not persisted on that host yet — reported instead of sent, because
   * the host would resolve the id against its store and refuse, and its refusal
   * ("Fork source thread could not be loaded") describes a bug rather than the
   * situation the user is in.
   */
  async function forkRemoteConversationAt(desktopId: string, sessionId: string, sourceEntryId: string, forkable: boolean) {
    if (!forkable) {
      emitFutureEvent("toast", { message: tRemotePeer("forkNotSaved"), tone: "error" });
      return;
    }
    try {
      const child = await forkRemoteConversation(desktopId, sessionId, sourceEntryId);
      await refreshRemotePeers();
      setActiveRemote({ desktopId, sessionId: child.sessionId, workspaceId: null });
    }
    catch (error) {
      emitFutureEvent("toast", {
        message: tRemotePeer("forkFailed", { message: errorMessage(error) }),
        tone: "error",
      });
    }
  }

  /**
   * A conversation that does not exist yet on a host.
   *
   * The empty session id *is* the request: the host creates the thread and
   * answers with the ids it chose. A draft is therefore not an error state to
   * guard against — it is the only way to start a conversation remotely, since
   * the client cannot mint a host-side session id.
   */
  function startRemoteConversation(desktopId: string, workspaceId: string | null = null) {
    setActiveRemote({ desktopId, sessionId: "", workspaceId });
    setSection("chat");
    // A draft on a machine is still "a conversation in the column": without
    // this, asking for one from the new-chat screen left that screen up and the
    // machine's composer was never shown.
    setActiveThreadId(null);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
  }

  // ---- Keyboard shortcuts ---------------------------------------------
  // One listener for the whole window, and the commands that are the *shell's*
  // own: navigation, panels, settings. Everything that belongs to a view — the
  // transcript's find bar, the composer's caret, the rail's row order, a run's
  // interrupt — is registered by that view while it is mounted (see
  // `features/keyboard`), which is also what makes a command unavailable
  // instead of silently inert.
  useShortcutDispatcher();
  // Nothing on the onboarding gate: there is no conversation, no rail and no
  // settings page to reach yet.
  const shortcutsLive = !showGate;
  // ⌘N is the rail's "new conversation" button under a different name, so it
  // asks the same question — a shortcut that made something on a machine other
  // than the one on screen was wrong for the same reason the button was.
  useCommand("chat.new", () => handleNewConversationRequest(), shortcutsLive);
  useCommand("vibe.toggle", () => {
    if (vibeOpen)
      closeVibe();
    else
      setVibeOpen(true);
  }, shortcutsLive);
  useCommand("panel.left", handleToggleLeftPanel, shortcutsLive);
  useCommand("panel.right", () => setRightExpanded(value => !value), shortcutsLive && !hideRightPanel);
  useCommand("settings.open", () => {
    setSettingsTab("general");
    setSettingsOpen(true);
  }, shortcutsLive);
  // The same dialog on the page that edits the binds — what `⌘/` points at. A
  // separate command rather than a flag on `settings.open`: two intents, two
  // rebindable keys, and `⌘,` keeps opening the page it always did.
  useCommand("shortcuts.settings", () => {
    setSettingsTab("shortcuts");
    setSettingsOpen(true);
  }, shortcutsLive);
  useCommand("section.chat", () => handleSectionChange("chat"), shortcutsLive);
  useCommand("section.workspace", () => handleSectionChange("workspace"), shortcutsLive);
  useCommand("section.skill", () => handleSectionChange("skill"), shortcutsLive);
  useCommand("section.tasks", () => handleSectionChange("tasks"), shortcutsLive);
  useCommand("section.remote", () => handleSectionChange("remote"), shortcutsLive);
  useCommand("palette.open", () => setPaletteOpen(true), shortcutsLive);
  useCommand("shortcuts.help", () => setCheatSheetOpen(true), shortcutsLive);

  /**
   * What the command palette offers.
   *
   * Built here rather than inside the dialog because only the shell knows which
   * conversations, workspaces and machines exist. Two rules shape it:
   *
   * - **A command with no handler is not listed**, but that question is answered
   *   by the palette at the moment it opens (`hasCommandHandler`), not here: the
   *   handlers this shell registers are installed by an effect, so a list built
   *   during the first render would be a list of nothing.
   * - **The palette's own two commands are left out.** Nothing is more confusing
   *   than a "command palette" entry inside the command palette.
   */
  const paletteCommands = useMemo<PaletteCommand[]>(
    () => COMMANDS
      .filter(spec => spec.category !== "palette")
      .map(spec => ({
        aliases: tShortcuts(`keywords.${spec.id}`, { defaultValue: [], returnObjects: true }) as unknown as string[],
        category: spec.category,
        id: spec.id,
        label: tShortcuts(`commands.${spec.id}`),
      })),
    [tShortcuts],
  );

  const paletteTargets = useMemo<PaletteTarget[]>(() => {
    const workspaceNames = new Map(workspaces.map(workspace => [workspace.id, workspace.name]));
    const peerNames = new Map(remotePeers.map(peer => [peer.desktopId, peer.name]));
    const local: PaletteTarget[] = threads
      .filter(thread => thread.status !== "deleted")
      .map(thread => ({
        key: `local:${thread.id}`,
        kind: "conversation",
        run: () => handleSelectThread(thread),
        subtitle: thread.mode === "chat"
          ? t("activityRail.chat")
          : workspaceNames.get(thread.workspaceId) ?? undefined,
        title: thread.title,
      }));
    const remote: PaletteTarget[] = remoteCatalogs.flatMap(catalog => catalog.sessions.map(session => ({
      key: `remote:${catalog.desktopId}:${session.sessionId}`,
      kind: "conversation" as const,
      run: () => openRemoteSession(catalog.desktopId, session.sessionId),
      subtitle: [peerNames.get(catalog.desktopId), session.workspaceName ?? undefined]
        .filter(Boolean)
        .join(" · ") || undefined,
      title: session.title,
    })));
    const workspaceTargets: PaletteTarget[] = workspaces
      .filter(workspace => workspace.kind === "user")
      .map(workspace => ({
        key: `workspace:${workspace.id}`,
        kind: "workspace",
        run: () => handleSelectWorkspace(
          workspace,
          threads
            .filter(thread => thread.workspaceId === workspace.id && thread.status === "active")
            .sort((left, right) => right.updatedAt - left.updatedAt),
        ),
        subtitle: workspace.path,
        title: workspace.name,
      }));
    return [...local, ...remote, ...workspaceTargets];
    // The handlers are stable in what they do (they read current state through
    // setters and refs), and listing them would rebuild this list — and with it
    // every row's identity — on every render of a shell that re-renders on every
    // poll tick and every streaming push.
    // eslint-disable-next-line react/exhaustive-deps
  }, [
    remoteCatalogs,
    remotePeers,
    threads,
    workspaces,
  ]);

  const activityRailProps = {
    active: section,
    activeThreadId,
    remotePeers,
    remoteCatalogs,
    deviceFilter,
    onChangeDeviceFilter: setDeviceFilter,
    onOpenRemoteConversation: openRemoteConversation,
    onRenameRemoteConversation: setRemoteRename,
    onRenameRemoteWorkspace: setRemoteWorkspaceRename,
    onNewChatOnRemote: (desktopId: string, workspaceId: string | null) => startRemoteConversation(desktopId, workspaceId),
    onRemoteConversationsChanged: () => void refreshRemotePeers(),
    // The device picker's own takeover: the same `takeover` command the Remote
    // page sends, then a re-read so the machine's rows appear in the merged list
    // the picker sits above (the host's `control` push would get there a poll
    // later at the latest, but the list is right there under the button).
    onTakeControlRemote: async (desktopId: string) => {
      await takeoverRemotePeer(desktopId);
      await refreshRemotePeers();
    },
    onManageDesktops: () => handleSectionChange("remote"),
    activeRemoteKey: activeRemote
      ? `${activeRemote.desktopId}::${activeRemote.sessionId}`
      : null,
    hasUpdate,
    threads,
    threadRunStatuses,
    threadStreamingStatuses,
    unreadThreadIds,
    workspaces,
    onChange: handleSectionChange,
    onOpenModels: handleOpenModels,
    // The rail's create controls: a plain new conversation asks which machine
    // first (`handleNewConversationRequest`); one started inside a workspace
    // names its machine already — that folder is on this disk.
    onNewChat: (workspaceId?: string) => {
      if (workspaceId) {
        handleOpenNewChat(workspaceId);
        return;
      }
      handleNewConversationRequest();
    },
    onNewWorkspace: handleOpenNewWorkspace,
    onBatchDeleteThreads: openBatchDelete,
    onDeleteRemoteConversations: handleDeleteRemoteConversations,
    onRemoteConversationDeleted: handleRemoteConversationDeleted,
    onDeleteThread: openDelete,
    onRenameThread: openRename,
    onDeleteWorkspace: openWorkspaceDelete,
    onRenameWorkspace: openWorkspaceRename,
    onTogglePinWorkspace: handleTogglePinWorkspace,
    onRestoreThread: handleRestoreThread,
    onSelectWorkspace: handleSelectWorkspace,
    onSelectThread: handleSelectThread,
    onTogglePinThread: handleTogglePinThread,
    onToggleExpanded: handleToggleLeftPanel,
    remoteIndicator,
    futureBalance,
    futureSessionStatus,
    userEmail: futureEmail,
    communityEdition,
    onRecharge: handleRecharge,
    onOpenUpdate: handleOpenUpdate,
    skillIntroDismissed: appSettings.skillIntroDismissed,
    onDismissSkillIntro: () => void changeSettings({ skillIntroDismissed: true }),
  };

  /**
   * What a panel cell's conversation view needs from the shell.
   *
   * Built here because only the shell has it: the agent connection, the model
   * catalog, the approval tier, the composer's mention list. Memoized on those,
   * not on `activeThread*`, so a cell never remounts — or re-reads its history —
   * because the *main window's* selection moved.
   */
  const vibeView: VibeThreadViewProps = useMemo(() => ({
    agentConnection,
    approvalTier: appSettings.approvalTier,
    futureBalance,
    futureSessionStatus,
    loadingStore,
    modelId: activeThreadModelId,
    modelOptions: visibleModelOptions,
    onChangeApprovalTier: value => void changeSettings({ approvalTier: value }),
    // A decision is persisted by the panel's own queue; this only brings the
    // shell's view back in step with it.
    onApprovalDecided: () => void refreshStore(activeThread?.id ?? undefined),
    onForked: (threadId: string) => void refreshStore(threadId),
    onModelChange: changeModel,
    onOpenAccount: handleOpenAccount,
    onOpenModels: handleOpenModels,
    onOpenProviders: handleOpenProviders,
    onRetryAgentConnection: () => void refreshAgentModels(),
    onThinkingLevelChange: changeThinkingLevel,
    sessionMentions,
    skillRecommend: appSettings.skillRecommend,
    thinkingLevel: activeThinkingLevel,
    // A cell's own activity refresh: no thread id, so the store reloads whatever
    // it considers current instead of pinning the shell's selection.
    onThreadActivity: () => void refreshStore(activeThread?.id ?? undefined),
  }), [
    activeThinkingLevel,
    activeThreadModelId,
    activeThread?.id,
    agentConnection,
    appSettings.approvalTier,
    appSettings.skillRecommend,
    changeModel,
    changeSettings,
    changeThinkingLevel,
    futureBalance,
    futureSessionStatus,
    handleOpenAccount,
    handleOpenModels,
    handleOpenProviders,
    loadingStore,
    refreshAgentModels,
    refreshStore,
    sessionMentions,
    visibleModelOptions,
  ]);

  // The startup coordinator has already settled the first account/provider
  // snapshot, so this branch can choose its first visible page atomically.
  if (showGate)
    return <OnboardingGate communityEdition={communityEdition} autoLogin={forceOnboarding} hasAnyProvider={hasAnyProvider} initPending={initPending} modelsReady={modelOptions.length > 0} onEnableBYOK={enableBYOK} onInitComplete={finishInit} onCancelLogin={cancelLogin} />;

  return (
    <div className="relative flex h-full min-h-0 overflow-hidden bg-canvas text-ink">
      {/* The panel replaces the shell's columns instead of covering them: an editor
        left mounted underneath would be a second place a dictation could land, and
        the panel's whole design rests on there being exactly one. Everything the
        shell owns above this return — the store, run statuses, approvals, remote
        polling — stays mounted either way, which is why the panel is drawn from
        here and not from a second window. */}
      {vibeOpen
        ? (
            <VibeStage
              catalogs={remoteCatalogs}
              onClose={closeVibe}
              onOpenInShell={(thread) => {
                handleSelectThread(thread);
                closeVibe();
              }}
              onOpenRemoteInShell={(desktopId, sessionId) => {
                openRemoteSession(desktopId, sessionId);
                closeVibe();
              }}
              onNewConversation={() => {
                pinNextCreatedRef.current = true;
                closeVibe();
                handleOpenNewChat();
              }}
              onPinLocal={(threadId, pinned) => void pinLocalThread(threadId, pinned)}
              onPinRemote={(desktopId, sessionId, pinned) => void pinRemoteSession(desktopId, sessionId, pinned)}
              peers={remotePeers}
              runStatuses={threadRunStatuses}
              threads={threads}
              unreadThreadIds={unreadThreadIds}
              view={vibeView}
              workspaces={workspaces}
            />
          )
        : (
            <>
              {showLeftPanel
                ? (
                    <div className="relative h-full shrink-0" style={{ width: leftPanel.width }}>
                      <ActivityRail expanded {...activityRailProps} />
                      <div
                        role="separator"
                        aria-label={t("activityRail.resizeSidebar")}
                        aria-orientation="vertical"
                        aria-valuemin={MIN_LEFT_PANEL_WIDTH}
                        aria-valuemax={leftPanel.maxWidth}
                        aria-valuenow={leftPanel.width}
                        className="absolute inset-y-0 -right-1 z-40 w-2 cursor-col-resize touch-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
                        tabIndex={0}
                        onPointerDown={leftPanel.startResize}
                        onKeyDown={(event) => {
                          if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
                            event.preventDefault();
                            leftPanel.nudge(event.key === "ArrowRight" ? 16 : -16);
                          }
                        }}
                      />
                    </div>
                  )
                : null}
              {leftPanel.resizing ? <div className="fixed inset-0 z-50 cursor-col-resize select-none" /> : null}
              {!showLeftPanel
                ? (
                    <div
                      aria-hidden="true"
                      className="absolute left-0 top-0 z-30 h-full w-2 cursor-col-resize"
                      onMouseEnter={() => handlePreviewLeftPanel(true)}
                    />
                  )
                : null}
              {!showLeftPanel && leftOverlayOpen
                ? (
                    <div
                      className="absolute left-0 top-0 z-40 h-full"
                      style={{ width: leftPanel.width }}
                      onMouseEnter={() => handlePreviewLeftPanel(true)}
                      onMouseLeave={() => handlePreviewLeftPanel(false)}
                    >
                      <ActivityRail expanded floating {...activityRailProps} />
                    </div>
                  )
                : null}
              <div className="flex min-h-0 min-w-0 flex-1 flex-col">
                <div className="flex min-h-0 min-w-0 flex-1">
                  <main ref={centerRef} className="min-w-0 flex-1 bg-surface">
                    {centerMode === "new-chat"
                      ? (
                          <NewConversation
                            key={`${newConversationMode}:${newWorkspaceForm ?? ""}:${newChatWorkspaceId ?? ""}:${newWorkspaceNonce}`}
                            initialWorkspaceForm={newWorkspaceForm}
                            initialMode={newConversationMode}
                            initialWorkspaceId={newChatWorkspaceId}
                            leftPanelExpanded={showLeftPanel}
                            modelId={selectedModelId}
                            modelOptions={visibleModelOptions}
                            modelsEmptyReason={modelsEmptyReason}
                            onAddWorkspace={handleAddWorkspace}
                            onModelChange={changeDraftModel}
                            thinkingLevel={selectedThinkingLevel}
                            onThinkingLevelChange={changeDraftThinkingLevel}
                            approvalTier={appSettings.approvalTier}
                            onChangeApprovalTier={value => void changeSettings({ approvalTier: value })}
                            onStart={startNewConversation}
                            onToggleLeftPanel={handleToggleLeftPanel}
                            skillGuideDismissed={appSettings.skillGuideDismissed}
                            onDismissSkillGuide={() => void changeSettings({ skillGuideDismissed: true })}
                            workspaces={userWorkspaces}
                            skillRecommend={appSettings.skillRecommend}
                            sessionMentions={sessionMentions}
                            futureSessionStatus={futureSessionStatus}
                            futureBalance={futureBalance}
                          />
                        )
                      : section === "skill"
                        ? (
                            <SkillsView leftPanelExpanded={showLeftPanel} onToggleLeftPanel={handleToggleLeftPanel} onStartCoachConversation={handleStartCoachConversation} onTrySkill={handleTrySkill} />
                          )
                        : section === "tasks"
                          ? (
                              <TasksView
                                leftPanelExpanded={showLeftPanel}
                                modelOptions={visibleModelOptions}
                                onOpenThread={handleOpenTaskThread}
                                onToggleLeftPanel={handleToggleLeftPanel}
                              />
                            )
                          : section === "remote"
                            ? (
                                <RemoteHubView
                                  leftPanelExpanded={showLeftPanel}
                                  onOpenRemoteSession={openRemoteSession}
                                  onReconnectRemoteAll={reconnectRemotePeers}
                                  onRefreshPeers={refreshRemotePeers}
                                  onRefreshRemote={refreshRemote}
                                  onToggleLeftPanel={handleToggleLeftPanel}
                                  peers={remotePeers}
                                  peersError={remotePeersError}
                                  peersLoading={remotePeersLoading}
                                  remoteStatus={remoteStatus}
                                />
                              )
                            : section === "chat" && activeRemote
                              ? (
                                  <RemoteConversationView
                                    approvals={remoteApprovals.approvals}
                                    approvalErrors={remoteApprovals.errors}
                                    approvalPending={remoteApprovals.pending}
                                    desktopId={activeRemote.desktopId}
                                    composer={(
                                      // The composer and, for a draft, the row that
                                      // decides where the conversation will live. One
                                      // card, as on this app's own new-conversation
                                      // screen: the footer belongs to the box it sits
                                      // under, so the two are wrapped together and the
                                      // box's bottom corners are handed to the footer.
                                      <div className="pointer-events-auto mx-auto w-full max-w-3xl">
                                        <RemoteComposer
                                          key={`remote-composer:${activeRemote.desktopId}:${activeRemote.sessionId}`}
                                          canCompact={remoteTimeline.canCompact}
                                          // The conversation frame places the composer: the
                                          // band over the transcript's foot, at the same
                                          // width the local composer is given.
                                          className={remoteDraft ? "w-full rounded-b-none" : "w-full"}
                                          conversations={remoteConversations}
                                          desktopId={activeRemote.desktopId}
                                          // The host-owned model and thinking level live in
                                          // the composer's control row, beside the send
                                          // button they apply to — the same place this
                                          // app's own model and thinking controls are.
                                          controls={<RemoteConversationSettings settings={remoteSettings} />}
                                          freshModelId={remoteSettings.model}
                                          freshThinkingLevel={remoteSettings.thinkingLevel}
                                          onCompact={() => void compactRemoteConversationOn(activeRemote.desktopId, activeRemote.sessionId)}
                                          onCreated={(createdSessionId) => {
                                            // Adopt the conversation the host just made, so
                                            // the next prompt goes to it by name instead
                                            // of creating a second one.
                                            setActiveRemote({ desktopId: activeRemote.desktopId, sessionId: createdSessionId, workspaceId: null });
                                            void refreshRemotePeers();
                                          }}
                                          onSent={() => {
                                            void remoteTimeline.refresh();
                                            void refreshRemotePeers();
                                          }}
                                          peer={activeRemotePeer}
                                          sessionId={activeRemote.sessionId}
                                          skillRecommendation={remoteSkillRecommendation}
                                          streaming={remoteTimeline.streaming}
                                          workspaceId={activeRemote.workspaceId}
                                        />
                                        {remoteDraft
                                          ? (
                                              <RemoteDraftTargetChooser
                                                error={draftWorkspaces.error}
                                                loading={draftWorkspaces.loading}
                                                onOpenWorkspace={activeRemotePeer
                                                  ? () => setRemoteWorkspaceHost(activeRemotePeer)
                                                  : undefined}
                                                onSelect={workspaceId => setActiveRemote(current =>
                                                  current ? { ...current, workspaceId } : current)}
                                                rows={draftWorkspaces.rows}
                                                workspaceId={remoteDraft.workspaceId}
                                              />
                                            )
                                          : null}
                                      </div>
                                    )}
                                    leftPanelExpanded={showLeftPanel}
                                    liveReplies={remoteTimeline.liveReplies}
                                    messages={remoteTimeline.messages}
                                    error={remoteTimeline.error}
                                    hasMore={remoteTimeline.hasMore}
                                    loading={remoteTimeline.loading}
                                    loadingOlder={remoteTimeline.loadingOlder}
                                    onToggleLeftPanel={handleToggleLeftPanel}
                                    onContinueRun={runId => void continueRemoteRunOn(activeRemote.desktopId, activeRemote.sessionId, runId)}
                                    onDecideApproval={(approval, decision) => void remoteApprovals.decide(approval, decision)}
                                    onFork={(sourceEntryId, forkable) =>
                                      void forkRemoteConversationAt(
                                        activeRemote.desktopId,
                                        activeRemote.sessionId,
                                        sourceEntryId,
                                        forkable,
                                      )}
                                    onLoadOlder={() => remoteTimeline.loadOlder()}
                                    olderHistoryFailed={remoteTimeline.olderHistoryFailed}
                                    onOpenUsage={() => setRemoteUsageOpen(true)}
                                    onRetry={() => void remoteTimeline.refresh()}
                                    peer={activeRemotePeer}
                                    persistedEntryIds={remoteTimeline.persistedEntryIds}
                                    sessionId={activeRemote.sessionId}
                                    streaming={remoteTimeline.streaming}
                                    title={remoteDraft
                                      // A draft is named as this machine's own new
                                      // conversation is, rather than by the empty
                                      // session id that is about to exist.
                                      ? tAgent("newConversation.headerTitle")
                                      : (activeRemoteContext?.title ?? activeRemote.sessionId)}
                                    workspaceName={activeRemoteContext?.workspaceName ?? draftWorkspace?.name ?? null}
                                  />
                                )
                              : storeError
                                ? (
                                    <div className="flex h-full items-center justify-center p-8 text-sm text-ink-soft">
                                      {t("appShell.storeInitFailed")}
                                      {storeError}
                                    </div>
                                  )
                                : (
                                    <AgentThread
                                      // One instance per conversation: switching threads
                                      // remounts, so a conversation's messages, listeners and
                                      // in-flight writes can never bleed into another.
                                      key={activeThread?.id ?? "__none"}
                                      activeApproval={activeApproval}
                                      agentConnection={agentConnection}
                                      approvalTier={appSettings.approvalTier}
                                      loadingStore={loadingStore}
                                      modelId={activeThreadModelId}
                                      modelOptions={visibleModelOptions}
                                      onModelChange={changeModel}
                                      onChangeApprovalTier={value => void changeSettings({ approvalTier: value })}
                                      thinkingLevel={activeThinkingLevel}
                                      onThinkingLevelChange={changeThinkingLevel}
                                      pendingPrompt={pendingPrompt}
                                      thread={activeThread}
                                      sessionMentions={sessionMentions}
                                      workspacePath={activeWorkspace?.path ?? null}
                                      onApprovalDecision={handleApprovalDecision}
                                      leftPanelExpanded={showLeftPanel}
                                      onRetryAgentConnection={() => void refreshAgentModels()}
                                      onOpenAccount={handleOpenAccount}
                                      onOpenModels={handleOpenModels}
                                      onOpenProviders={handleOpenProviders}
                                      onToggleLeftPanel={handleToggleLeftPanel}
                                      skillRecommend={appSettings.skillRecommend}
                                      futureSessionStatus={futureSessionStatus}
                                      futureBalance={futureBalance}
                                      headerAction={terminalHeaderAction}
                                      headerMachine={localMachineBadge}
                                      headerWorkspace={localWorkspaceBadge}
                                      onPromptConsumed={consumePendingPrompt}
                                      onForked={(forkedThreadId: string) => {
                                        void refreshStore(forkedThreadId);
                                      }}
                                      onThreadActivity={() => {
                                        void refreshStore(activeThread?.id ?? undefined);
                                      }}
                                    />
                                  )}
                  </main>
                  {/* Views without thread context hide the right panel entirely, including
          the collapsed expand affordance. */}
                  {/* A remote conversation's right panel is that machine's files — and
          nothing else. The panel's other tabs (review, runs, artifacts)
          describe *this* machine, so they have no remote counterpart to show;
          the panel is hidden entirely while a remote conversation is open and
          this one is drawn instead. */}
                  {hideRightPanel || activeRemote
                    ? null
                    : (
                        <ContextPanel
                          activeThread={activeThread}
                          activeWorkspace={activeWorkspace}
                          activeTab={contextTab}
                          expanded={rightExpanded}
                          width={rightPanelWidth}
                          onResizeStart={startRightPanelResize}
                          onResizeNudge={nudgeRightPanel}
                          onTabChange={setContextTab}
                          onToggleExpanded={() => setRightExpanded(value => !value)}
                        />
                      )}
                  {activeRemote && activeRemote.sessionId !== "" && !hideRightPanel
                    ? (
                        <RemoteFilesPanel
                          desktopId={activeRemote.desktopId}
                          expanded={rightExpanded}
                          onToggleExpanded={() => setRightExpanded(value => !value)}
                          peer={activeRemotePeer}
                          sessionId={activeRemote.sessionId}
                        />
                      )
                    : null}
                  {/* While dragging the divider, a full-window overlay keeps the cursor and
          captures mouse events even over embedded iframes (PDF preview). */}
                  {rightPanelResizing && !hideRightPanel
                    ? <div className="fixed inset-0 z-50 cursor-col-resize select-none" />
                    : null}
                </div>
                {/* The terminal spans the conversation *and* the right panel: it is a
            sibling of that row, not of the left rail. Collapsing it removes its
            height entirely, so the conversation keeps the full space. */}
                {terminalThreadId && terminalPanel.open
                  ? (
                      <TerminalPanel
                        panel={terminalPanel}
                        threadId={terminalThreadId}
                      />
                    )
                  : null}
              </div>
            </>
          )}
      <AppShellDialogs
        batchDeleteDialog={batchDeleteDialog}
        deleteDialog={deleteDialog}
        renameDialog={renameDialog}
        setBatchDeleteDialog={setBatchDeleteDialog}
        setDeleteDialog={setDeleteDialog}
        setRenameDialog={setRenameDialog}
        onConfirmBatchDeleteThread={() => void confirmBatchDelete()}
        onConfirmDeleteThread={() => void confirmDelete()}
        onConfirmRenameThread={() => void confirmRename()}
        onGenerateTitle={() => void generateTitle()}
      />
      <WorkspaceDialogs
        deleteDialog={workspaceDeleteDialog}
        renameDialog={workspaceRenameDialog}
        setDeleteDialog={setWorkspaceDeleteDialog}
        setRenameDialog={setWorkspaceRenameDialog}
        onConfirmDeleteWorkspace={() => void confirmWorkspaceDelete()}
        onConfirmRenameWorkspace={() => void confirmWorkspaceRename()}
      />
      {remoteRename
        ? (
            <RemoteRenameDialog
              conversation={remoteRename}
              onClose={() => setRemoteRename(null)}
              onRenamed={() => void refreshRemotePeers()}
            />
          )
        : null}
      {workspaceDeviceChoice
        ? (
            <WorkspaceDeviceDialog
              onClose={() => setWorkspaceDeviceChoice(false)}
              onPickLocal={() => {
                setWorkspaceDeviceChoice(false);
                openLocalWorkspaceForm();
              }}
              onPickPeer={(peer) => {
                setWorkspaceDeviceChoice(false);
                setRemoteWorkspaceHost(peer);
              }}
              peers={remotePeers}
            />
          )
        : null}
      {newChatDeviceChoice
        ? (
            <DeviceChoiceDialog
              description={tRemotePeer("chooseChatDeviceHint")}
              onClose={() => setNewChatDeviceChoice(false)}
              onPickLocal={() => {
                setNewChatDeviceChoice(false);
                handleOpenNewChat();
                // The dialog had the caret, and this is the screen the question
                // is about to be typed on: give it back. A composer that mounts
                // in this same commit focuses itself; the event is for the one
                // already on screen (⌘N pressed again on the new-chat screen),
                // which no remount would have reached.
                emitFutureEvent("focus-composer", undefined);
              }}
              onPickPeer={(peer) => {
                setNewChatDeviceChoice(false);
                startRemoteConversation(peer.desktopId);
                // Same reason as above, on the machine that was picked: the
                // draft this opens may already be the one on screen.
                emitFutureEvent("focus-composer", undefined);
              }}
              peers={remotePeers}
              title={tRemotePeer("chooseChatDevice")}
            />
          )
        : null}
      {remoteWorkspaceHost
        ? (
            <RemoteWorkspaceCreateDialog
              onClose={() => setRemoteWorkspaceHost(null)}
              onCreated={workspace => handleRemoteWorkspaceCreated(remoteWorkspaceHost.desktopId, workspace)}
              peer={remoteWorkspaceHost}
            />
          )
        : null}
      {/* A workspace on another machine is renamed on that machine, so this uses
          the shared dialog shell (as this app's own workspace rename does) and the
          remote namespace for its words: which computer it belongs to is the one
          thing the reader needs that the local dialog never says. */}
      {remoteWorkspaceRename
        ? (
            <RemoteWorkspaceRenameDialog
              onClose={() => setRemoteWorkspaceRename(null)}
              onRenamed={() => void refreshRemotePeers()}
              target={remoteWorkspaceRename}
            />
          )
        : null}
      {/* The conversation's account book, as the local header's is: token usage
          and amount for the *host's* session, read from the host's own state. */}
      {remoteUsageOpen && activeRemote
        ? (
            <RemoteUsageDialog
              desktopId={activeRemote.desktopId}
              onClose={() => setRemoteUsageOpen(false)}
              sessionId={activeRemote.sessionId}
              title={activeRemoteContext?.title ?? activeRemote.sessionId}
            />
          )
        : null}
      <CommandPalette
        commands={paletteCommands}
        onClose={() => setPaletteOpen(false)}
        open={paletteOpen}
        targets={paletteTargets}
      />
      <ShortcutCheatSheet
        onClose={() => setCheatSheetOpen(false)}
        open={cheatSheetOpen}
      />
      <SettingsDialog
        appSettings={appSettings}
        cachedUpdateStatus={cachedStatus}
        futureBalance={futureBalance}
        futureBalanceStatus={futureBalanceStatus}
        futureEmail={futureEmail}
        futureSessionStatus={futureSessionStatus}
        onRefreshFutureAuth={refreshFutureAuth}
        onRefreshFutureBalance={refreshFutureBalance}
        hasUpdate={hasUpdate}
        initialTab={settingsTab}
        modelOptions={modelOptions}
        onChangeSettings={changeSettings}
        onClose={() => setSettingsOpen(false)}
        onProvidersChanged={() => void refreshAgentModels()}
        onUpdateSeen={markUpdateSeen}
        open={settingsOpen}
      />
      <ToastHost />
    </div>
  );
}
