import type { SettingsTab } from "../../features/settings/SettingsDialog";
import type { StoredApprovalRequest, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ActivitySection } from "./ActivityRail";
import type { ContextTab } from "./ContextPanel";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentThread } from "../../features/agent/AgentThread";
import { NewConversation } from "../../features/agent/NewConversation";
import { RemoteView } from "../../features/remote/RemoteView";
import { SettingsDialog } from "../../features/settings/SettingsDialog";
import { SkillsView } from "../../features/skills/SkillsView";
import {
  createWorkspace,
  pinThread,
  restoreThread,
} from "../../integrations/storage/threadStore";
import { emitFutureEvent } from "../../lib/futureEvents";
import { ToastHost } from "../ui/ToastHost";
import { ActivityRail } from "./ActivityRail";
import { AppShellDialogs } from "./AppShellDialogs";
import { ContextPanel } from "./ContextPanel";
import { useAgentConnection } from "./hooks/useAgentConnection";
import { useApprovals } from "./hooks/useApprovals";
import { useAppSettings } from "./hooks/useAppSettings";
import { useModelSelection } from "./hooks/useModelSelection";
import { useNewConversation } from "./hooks/useNewConversation";
import { useRightPanelWidth } from "./hooks/useRightPanelWidth";
import { useThreadDialogs } from "./hooks/useThreadDialogs";
import { useThreadStore } from "./hooks/useThreadStore";
import { useUnreadThreads } from "./hooks/useUnreadThreads";
import { useWorkspaceDialogs } from "./hooks/useWorkspaceDialogs";
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

  const {
    threads,
    workspaces,
    activeThread,
    activeWorkspace,
    activeThreadId,
    setActiveThreadId,
    threadRunStatuses,
    loadingStore,
    storeError,
    refreshStore,
  } = useThreadStore();
  const { activeApproval, decideApproval } = useApprovals(activeThread?.id ?? null);
  const {
    agentConnection,
    modelOptions,
    visibleModelOptions,
    selectedModelId,
    setSelectedModelId,
    refreshAgentModels,
  } = useAgentConnection(appSettings.hiddenModels);
  const {
    selectedThinkingLevel,
    modelsEmptyReason,
    activeThreadModelId,
    activeThinkingLevel,
    changeModel,
    changeDraftModel,
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
    setRenameDialog,
    setDeleteDialog,
    openRename,
    confirmRename,
    openDelete,
    confirmDelete,
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
  useEffect(() => {
    const unlisten = listen<string>("review-updated", (event) => {
      emitFutureEvent("review-updated", { threadId: event.payload });
    });
    return () => {
      void unlisten.then(stop => stop());
    };
  }, []);

  // macOS app menu "About FutureOS" opens the in-app About page (there is no
  // native About dialog). The backend emits this event from the menu handler.
  useEffect(() => {
    const unlisten = listen("open-settings", () => {
      setSettingsTab("about");
      setSettingsOpen(true);
    });
    return () => {
      void unlisten.then(stop => stop());
    };
  }, []);

  // Remote (phone) activity: a phone client created or drove a thread. Refresh
  // the thread list + runs so it appears and updates live in the GUI.
  useEffect(() => {
    const unlisten = listen("remote-activity", () => {
      void refreshStore();
    });
    return () => {
      void unlisten.then(stop => stop());
    };
  }, [refreshStore]);

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
    threads,
    threadRunStatuses,
    unreadThreadIds,
    workspaces,
    onChange: handleSectionChange,
    onOpenModels: handleOpenModels,
    onNewChat: handleOpenNewChat,
    onNewWorkspace: handleOpenNewWorkspace,
    onDeleteThread: openDelete,
    onRenameThread: openRename,
    onDeleteWorkspace: openWorkspaceDelete,
    onRenameWorkspace: openWorkspaceRename,
    onRestoreThread: handleRestoreThread,
    onSelectWorkspace: handleSelectWorkspace,
    onSelectThread: handleSelectThread,
    onTogglePinThread: handleTogglePinThread,
    onToggleExpanded: handleToggleLeftPanel,
  };

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
                onThinkingLevelChange={changeThinkingLevel}
                approvalTier={appSettings.approvalTier}
                onChangeApprovalTier={value => void changeSettings({ approvalTier: value })}
                onStart={startNewConversation}
                onToggleLeftPanel={handleToggleLeftPanel}
                workspaces={userWorkspaces}
              />
            )
          : section === "skill"
            ? (
                <SkillsView />
              )
            : section === "remote"
              ? (
                  <RemoteView appSettings={appSettings} onChangeSettings={patch => void changeSettings(patch)} />
                )
              : section === "data"
                ? (
                    <ModulePlaceholder section="data" />
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
        deleteDialog={deleteDialog}
        renameDialog={renameDialog}
        setDeleteDialog={setDeleteDialog}
        setRenameDialog={setRenameDialog}
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
        initialTab={settingsTab}
        modelOptions={modelOptions}
        onChangeSettings={patch => void changeSettings(patch)}
        onClose={() => setSettingsOpen(false)}
        onProvidersChanged={() => void refreshAgentModels()}
        open={settingsOpen}
      />
      <ToastHost />
    </div>
  );
}

function ModulePlaceholder({ section }: { section: "data" | "skill" }) {
  const { t } = useTranslation("layout");
  const title = section === "data" ? t("appShell.data") : t("appShell.skill");
  const detail = section === "data"
    ? t("appShell.dataPlaceholder")
    : t("appShell.skillPlaceholder");

  return (
    <section className="flex h-full min-h-0 items-center justify-center bg-surface p-8">
      <div className="max-w-md rounded-lg border border-line-soft bg-surface-subtle p-6 text-center">
        <div className="text-base font-semibold text-ink">{title}</div>
        <p className="mt-2 text-sm leading-6 text-ink-muted">{detail}</p>
      </div>
    </section>
  );
}
