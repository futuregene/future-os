import type { SettingsTab } from "../../features/settings/SettingsDialog";
import type { StoredApprovalRequest, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ActivitySection } from "./ActivityRail";
import type { ContextTab } from "./ContextPanel";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentThread } from "../../features/agent/AgentThread";
import { NewConversation } from "../../features/agent/NewConversation";
import { RemoteView } from "../../features/remote/RemoteView";
import { SettingsDialog } from "../../features/settings/SettingsDialog";
import { SkillsView } from "../../features/skills/SkillsView";
import { modelOption, readLastUsedModel } from "../../integrations/agent/agentClient";
import { installAgentEventListener, prefetchAgentState } from "../../integrations/agent/agentStateCache";
import { getFutureEnvironment } from "../../integrations/agent/providers";
import { refreshSkills } from "../../integrations/skills/skillsClient";
import { openExternalUrl } from "../../integrations/storage/files";
import {
  createWorkspace,
  pinThread,
  restoreThread,
} from "../../integrations/storage/threadStore";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { ToastHost } from "../ui/ToastHost";
import { ActivityRail } from "./ActivityRail";
import { AppShellDialogs } from "./AppShellDialogs";
import { ContextPanel } from "./ContextPanel";
import { useAgentConnection } from "./hooks/useAgentConnection";
import { useApprovals } from "./hooks/useApprovals";
import { useAppSettings } from "./hooks/useAppSettings";
import { useAutoUpgradeSkills } from "./hooks/useAutoUpgradeSkills";
import { useFutureAccount } from "./hooks/useFutureAccount";
import { useHasProviders } from "./hooks/useHasProviders";
import { useModelSelection } from "./hooks/useModelSelection";
import { useNewConversation } from "./hooks/useNewConversation";
import { useRemoteStatus } from "./hooks/useRemoteStatus";
import { useRightPanelWidth } from "./hooks/useRightPanelWidth";
import { useThreadDialogs } from "./hooks/useThreadDialogs";
import { useThreadStore } from "./hooks/useThreadStore";
import { useUnreadThreads } from "./hooks/useUnreadThreads";
import { useUpdateChecker } from "./hooks/useUpdateChecker";
import { useWorkspaceDialogs } from "./hooks/useWorkspaceDialogs";
import { OnboardingGate } from "./OnboardingGate";
import { WorkspaceDialogs } from "./WorkspaceDialogs";

export type { AgentConnectionState } from "./hooks/useAgentConnection";

interface WorkspaceCreateRequest {
  name?: string | null;
  path: string;
  createDirectory: boolean;
}

export function AppShell() {
  const { t } = useTranslation("layout");
  const [section, setSection] = useState<ActivitySection>("chat");
  const [centerMode, setCenterMode] = useState<"thread" | "new-chat">("thread");
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
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");

  const { appSettings, changeSettings } = useAppSettings();
  useAutoUpgradeSkills(appSettings.autoUpgradeSkills);
  const { hasUpdate, cachedStatus, markSeen: markUpdateSeen } = useUpdateChecker();
  // Drives the onboarding gate below. Kept with the other top-level hooks so
  // the early returns further down stay after every hook call (rules of hooks).
  const { showGate, byokMode, enableBYOK, finishInit, cancelLogin, hasAnyProvider, forceOnboarding, initPending, initialLoading } = useHasProviders();

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
  }, [leftExpanded, reclampRightPanel]);

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

  // Refresh the store when the agent session's cwd changes (e.g. TUI /cwd),
  // so the thread moves to the correct workspace in the sidebar.
  useEffect(() => {
    const handler = () => {
      refreshStore().catch(() => {});
    };
    window.addEventListener("future:cwd-changed", handler);
    return () => window.removeEventListener("future:cwd-changed", handler);
  }, [refreshStore]);

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

  // Remote control is dev-only: poll its status (for the sidebar indicator dot)
  // only on non-release builds, and never while build info is still loading.
  // Returns { status, indicator, refresh } — RemoteView reads `status` directly
  // so its blue dot always matches the sidebar indicator.
  const build = useBuildInfo();
  const showRemote = Boolean(build.data && !build.data.isRelease);
  const { status: remoteStatus, indicator: remoteIndicator, refresh: refreshRemote } = useRemoteStatus(showRemote);
  const { balance: futureBalance, email: futureEmail } = useFutureAccount();

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
      // Cached per-thread agent state may predate the restart/reconnect; the
      // activation prefetch only runs on thread switches, so revalidate the
      // viewed thread here too (deduped by the in-flight map + TTL gate).
      prefetchAgentState(activeThreadId);
    }
  }, [activeThreadId, agentConnection.status]);

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
  const {
    pendingPrompt,
    startNewConversation,
    consumePendingPrompt,
  } = useNewConversation({
    refreshStore,
    syncSelection,
    setSection,
    setCenterMode,
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
  const hideRightPanel = centerMode === "new-chat" || section === "skill" || section === "remote";

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

  // Remote (phone) activity: a phone client created or drove a thread. Refresh
  // the thread list + runs so it appears and updates live in the GUI.
  useTauriEvent("remote-activity", () => {
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

  function handleOpenModels() {
    setSettingsTab("models");
    setSettingsOpen(true);
  }

  function handleOpenAccount() {
    setSettingsTab("account");
    setSettingsOpen(true);
  }

  function handleOpenProviders() {
    setSettingsTab("providers");
    setSettingsOpen(true);
  }

  function handleSelectThread(thread: StoredThread) {
    setSection(thread.mode === "workspace" ? "workspace" : "chat");
    setActiveThreadId(thread.id);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
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
    // Workspace "+" on a specific workspace → a chat inside it; otherwise a
    // plain chat (Chat header "+" / top New Chat).
    setSection(workspaceId ? "workspace" : "chat");
    setNewChatWorkspaceId(workspaceId ?? null);
    setNewConversationMode(workspaceId ? "workspace" : "chat");
    setNewWorkspaceForm(null);
    setCenterMode("new-chat");
  }

  // Workspace header "+" → always (re)open the create-workspace dialog, even if
  // we're already on the new-conversation view. The nonce forces a remount so a
  // previously-cancelled dialog reopens.
  function handleOpenNewWorkspace() {
    setSection("workspace");
    setNewChatWorkspaceId(null);
    setNewConversationMode("workspace");
    setNewWorkspaceForm("open");
    setNewWorkspaceNonce(nonce => nonce + 1);
    setCenterMode("new-chat");
  }

  async function handleAddWorkspace(input: WorkspaceCreateRequest) {
    const workspace = await createWorkspace(input);
    await refreshStore(activeThread?.id ?? undefined);
    return workspace;
  }

  async function handleTogglePinThread(thread: StoredThread) {
    await pinThread({ threadId: thread.id, pinned: !thread.pinned });
    await refreshStore(thread.id);
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
    setLeftExpanded((expanded) => {
      const nextExpanded = !expanded;
      setLeftOverlayOpen(false);
      return nextExpanded;
    });
  }

  function handlePreviewLeftPanel(open: boolean) {
    if (leftExpanded)
      return;
    setLeftOverlayOpen(open);
  }

  const activityRailProps = {
    active: section,
    activeThreadId,
    hasUpdate,
    threads,
    threadRunStatuses,
    threadStreamingStatuses,
    unreadThreadIds,
    workspaces,
    onChange: handleSectionChange,
    onOpenModels: handleOpenModels,
    onNewChat: handleOpenNewChat,
    onNewWorkspace: handleOpenNewWorkspace,
    onBatchDeleteThreads: openBatchDelete,
    onDeleteThread: openDelete,
    onRenameThread: openRename,
    onDeleteWorkspace: openWorkspaceDelete,
    onRenameWorkspace: openWorkspaceRename,
    onRestoreThread: handleRestoreThread,
    onSelectWorkspace: handleSelectWorkspace,
    onSelectThread: handleSelectThread,
    onTogglePinThread: handleTogglePinThread,
    onToggleExpanded: handleToggleLeftPanel,
    remoteIndicator,
    futureBalance,
    userEmail: futureEmail,
    onRecharge: handleRecharge,
    onOpenUpdate: handleOpenUpdate,
  };

  // Onboarding gate: show during the initial probe, when no provider is
  // usable yet, or during post-login initialization (models + skills + agent).
  if (initialLoading) {
    return (
      <div className="flex h-full items-center justify-center bg-canvas">
        <span className="size-6 animate-spin rounded-full border-2 border-accent-soft border-t-accent" />
      </div>
    );
  }
  if (showGate)
    return <OnboardingGate autoLogin={forceOnboarding} hasAnyProvider={hasAnyProvider} initPending={initPending} modelsReady={modelOptions.length > 0} onEnableBYOK={enableBYOK} onInitComplete={finishInit} onCancelLogin={cancelLogin} />;

  return (
    <div className="relative flex h-full min-h-0 overflow-hidden bg-canvas text-ink">
      {leftExpanded ? <ActivityRail expanded {...activityRailProps} /> : null}
      {!leftExpanded
        ? (
            <div
              aria-hidden="true"
              className="absolute left-0 top-0 z-30 h-full w-2 cursor-ew-resize"
              onMouseEnter={() => handlePreviewLeftPanel(true)}
            />
          )
        : null}
      {!leftExpanded && leftOverlayOpen
        ? (
            <div
              className="absolute left-0 top-0 z-40 h-full w-56 md:w-64 xl:w-72"
              onMouseEnter={() => handlePreviewLeftPanel(true)}
              onMouseLeave={() => handlePreviewLeftPanel(false)}
            >
              <ActivityRail expanded floating {...activityRailProps} />
            </div>
          )
        : null}
      <main ref={centerRef} className="min-w-0 flex-1 bg-surface">
        {centerMode === "new-chat"
          ? (
              <NewConversation
                key={`${newConversationMode}:${newWorkspaceForm ?? ""}:${newChatWorkspaceId ?? ""}:${newWorkspaceNonce}`}
                initialWorkspaceForm={newWorkspaceForm}
                initialMode={newConversationMode}
                initialWorkspaceId={newChatWorkspaceId}
                leftPanelExpanded={leftExpanded}
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
                workspaces={userWorkspaces}
              />
            )
          : section === "skill"
            ? (
                <SkillsView leftPanelExpanded={leftExpanded} onToggleLeftPanel={handleToggleLeftPanel} />
              )
            : section === "remote"
              ? (
                  <RemoteView appSettings={appSettings} leftPanelExpanded={leftExpanded} onChangeSettings={patch => void changeSettings(patch)} onToggleLeftPanel={handleToggleLeftPanel} remoteStatus={remoteStatus} onRefreshRemote={refreshRemote} />
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
                      showThinking={appSettings.showThinking}
                      loadingStore={loadingStore}
                      modelId={activeThreadModelId}
                      modelOptions={visibleModelOptions}
                      onModelChange={changeModel}
                      onChangeApprovalTier={value => void changeSettings({ approvalTier: value })}
                      thinkingLevel={activeThinkingLevel}
                      onThinkingLevelChange={changeThinkingLevel}
                      pendingPrompt={pendingPrompt}
                      thread={activeThread}
                      workspacePath={activeWorkspace?.path ?? null}
                      onApprovalDecision={handleApprovalDecision}
                      leftPanelExpanded={leftExpanded}
                      onRetryAgentConnection={() => void refreshAgentModels()}
                      onOpenAccount={handleOpenAccount}
                      onOpenModels={handleOpenModels}
                      onOpenProviders={handleOpenProviders}
                      onToggleLeftPanel={handleToggleLeftPanel}
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
      {hideRightPanel
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
      {/* While dragging the divider, a full-window overlay keeps the cursor and
          captures mouse events even over embedded iframes (PDF preview). */}
      {rightPanelResizing && !hideRightPanel
        ? <div className="fixed inset-0 z-50 cursor-ew-resize select-none" />
        : null}
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
      />
      <WorkspaceDialogs
        deleteDialog={workspaceDeleteDialog}
        renameDialog={workspaceRenameDialog}
        setDeleteDialog={setWorkspaceDeleteDialog}
        setRenameDialog={setWorkspaceRenameDialog}
        onConfirmDeleteWorkspace={() => void confirmWorkspaceDelete()}
        onConfirmRenameWorkspace={() => void confirmWorkspaceRename()}
      />
      <SettingsDialog
        appSettings={appSettings}
        cachedUpdateStatus={cachedStatus}
        hasUpdate={hasUpdate}
        initialTab={settingsTab}
        modelOptions={modelOptions}
        onChangeSettings={patch => void changeSettings(patch)}
        onClose={() => setSettingsOpen(false)}
        onProvidersChanged={() => void refreshAgentModels()}
        onUpdateSeen={markUpdateSeen}
        open={settingsOpen}
      />
      <ToastHost />
    </div>
  );
}
