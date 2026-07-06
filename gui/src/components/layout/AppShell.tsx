import type { MessageAttachment } from "../../features/agent/agentThreadTypes";
import type { NewConversationStart } from "../../features/agent/NewConversation";
import type { SettingsTab } from "../../features/settings/SettingsDialog";
import type { StoredApprovalRequest, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ActivitySection } from "./ActivityRail";
import type { ContextTab } from "./ContextPanel";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentThread } from "../../features/agent/AgentThread";
import { NewConversation } from "../../features/agent/NewConversation";
import { RemoteView } from "../../features/remote/RemoteView";
import { ResearchView } from "../../features/research/ResearchView";
import { SettingsDialog } from "../../features/settings/SettingsDialog";
import { SkillsView } from "../../features/skills/SkillsView";
import i18n from "../../i18n";
import { modelThinkingLevel, normalizeThinkingLevel } from "../../integrations/agent/agentClient";
import {
  createThread,
  createWorkspace,
  pinThread,
  restoreThread,
} from "../../integrations/storage/threadStore";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { ToastHost } from "../ui/ToastHost";
import { ActivityRail } from "./ActivityRail";
import { AppShellDialogs } from "./AppShellDialogs";
import { ContextPanel } from "./ContextPanel";
import { useAgentConnection } from "./hooks/useAgentConnection";
import { useApprovals } from "./hooks/useApprovals";
import { useAppSettings } from "./hooks/useAppSettings";
import { useModelSelection } from "./hooks/useModelSelection";
import { useThreadDialogs } from "./hooks/useThreadDialogs";
import { useThreadStore } from "./hooks/useThreadStore";
import { useUnreadThreads } from "./hooks/useUnreadThreads";
import { useWorkspaceDialogs } from "./hooks/useWorkspaceDialogs";
import { WorkspaceDialogs } from "./WorkspaceDialogs";

export type { AgentConnectionState } from "./hooks/useAgentConnection";

interface PendingPrompt {
  attachments?: MessageAttachment[];
  id: string;
  content: string;
}

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
  const [contextTab, setContextTab] = useState<ContextTab>("runs");
  const [newChatWorkspaceId, setNewChatWorkspaceId] = useState<string | null>(null);
  const [newConversationMode, setNewConversationMode] = useState<"workspace" | "chat">("chat");
  const [newWorkspaceForm, setNewWorkspaceForm] = useState<"open" | null>(null);
  const [selectedResearchResourceId, setSelectedResearchResourceId] = useState<string | null>(null);
  const [pendingPrompt, setPendingPrompt] = useState<PendingPrompt | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");

  const { appSettings, changeSettings } = useAppSettings();

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
    changeModel,
    changeDraftModel,
    changeThinkingLevel,
    syncSelection,
  } = useModelSelection({
    activeThread,
    selectedModelId,
    setSelectedModelId,
    visibleModelOptions,
    refreshStore,
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
  const activeThreadModelId = activeThread?.modelId ?? selectedModelId;
  const activeThinkingLevel = activeThread
    ? normalizeThinkingLevel(activeThread.thinkingLevel ?? modelThinkingLevel(activeThreadModelId, visibleModelOptions))
    : selectedThinkingLevel;

  useEffect(() => onFutureEvent("open-research-resource", (detail) => {
    setSelectedResearchResourceId(detail.resourceId);
    setSection("research");
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
  }), []);

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

  // macOS app menu "About FutureOS" opens the in-app Settings page (there is no
  // native About dialog). The backend emits this event from the menu handler.
  useEffect(() => {
    const unlisten = listen("open-settings", () => {
      setSettingsTab("general");
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

  // Workspace header "+" → start the new-conversation flow on the open-workspace step.
  function handleOpenNewWorkspace() {
    setSection("workspace");
    setNewChatWorkspaceId(null);
    setNewConversationMode("workspace");
    setNewWorkspaceForm("open");
    setCenterMode("new-chat");
  }

  async function handleStartNewConversation(input: NewConversationStart) {
    const title = deriveThreadTitle(input.content);
    const thread = await createThread({
      mode: input.mode,
      title,
      workspaceId: input.workspace?.id,
      workspaceName: input.workspace?.label,
      workspacePath: input.workspace?.path,
      modelId: input.modelId,
      thinkingLevel: input.thinkingLevel,
    });
    syncSelection(input.modelId, input.thinkingLevel);
    await refreshStore(thread.id);
    setSection(thread.mode === "workspace" ? "workspace" : "chat");
    setCenterMode("thread");
    setPendingPrompt({
      attachments: input.attachments,
      id: newPendingPromptId(thread.id),
      content: input.content,
    });
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
      <main className="min-w-0 flex-1 bg-surface">
        {centerMode === "new-chat"
          ? (
              <NewConversation
                key={`${newConversationMode}:${newWorkspaceForm ?? ""}:${newChatWorkspaceId ?? ""}`}
                initialWorkspaceForm={newWorkspaceForm}
                initialMode={newConversationMode}
                initialWorkspaceId={newChatWorkspaceId}
                leftPanelExpanded={leftExpanded}
                modelId={selectedModelId}
                modelOptions={visibleModelOptions}
                onAddWorkspace={handleAddWorkspace}
                onModelChange={changeDraftModel}
                thinkingLevel={selectedThinkingLevel}
                onThinkingLevelChange={changeThinkingLevel}
                approvalTier={appSettings.approvalTier}
                onChangeApprovalTier={value => void changeSettings({ approvalTier: value })}
                onStart={handleStartNewConversation}
                onToggleLeftPanel={handleToggleLeftPanel}
                workspaces={workspaces.filter(workspace => workspace.kind === "user")}
              />
            )
          : section === "research"
            ? (
                <ResearchView
                  selectedResourceId={selectedResearchResourceId}
                  workspaceId={activeWorkspace?.id ?? null}
                  workspaceName={activeWorkspace?.name ?? t("appShell.noWorkspaceSelected")}
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
                          modelId={activeThread?.modelId ?? selectedModelId}
                          modelOptions={visibleModelOptions}
                          onModelChange={changeModel}
                          onChangeApprovalTier={value => void changeSettings({ approvalTier: value })}
                          thinkingLevel={activeThinkingLevel}
                          onThinkingLevelChange={changeThinkingLevel}
                          pendingPrompt={pendingPrompt}
                          thread={activeThread}
                          onApprovalDecision={handleApprovalDecision}
                          leftPanelExpanded={leftExpanded}
                          onRetryAgentConnection={() => void refreshAgentModels()}
                          onOpenProviders={handleOpenProviders}
                          onOpenModels={handleOpenModels}
                          onToggleLeftPanel={handleToggleLeftPanel}
                          onPromptConsumed={(id) => {
                            setPendingPrompt(current => (current?.id === id ? null : current));
                          }}
                          onThreadActivity={() => {
                            void refreshStore(activeThread?.id ?? undefined);
                          }}
                        />
                      )}
      </main>
      {/* A new blank conversation has no thread context yet — hide the right
          panel entirely (no expand affordance) until a thread exists. */}
      {centerMode === "new-chat"
        ? null
        : (
            <ContextPanel
              activeThread={activeThread}
              activeWorkspace={activeWorkspace}
              activeTab={contextTab}
              expanded={rightExpanded}
              onTabChange={setContextTab}
              onToggleExpanded={() => setRightExpanded(value => !value)}
            />
          )}
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

function deriveThreadTitle(content: string) {
  const compact = content.replace(/\s+/g, " ").trim();
  if (!compact)
    return i18n.t("layout:appShell.newChat");
  return compact.length > 28 ? `${compact.slice(0, 28)}...` : compact;
}

let pendingPromptCounter = 0;

function newPendingPromptId(threadId: string) {
  pendingPromptCounter += 1;
  return `${threadId}:${Date.now()}:${pendingPromptCounter}`;
}
